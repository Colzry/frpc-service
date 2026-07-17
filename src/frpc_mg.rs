//! frpc 进程管理，负责启动和停止 frpc 进程

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::Sender;
use strip_ansi_escapes::strip;

pub struct FrpcProcess {
    child: Option<Child>,
    pub identifier: String, // 用于日志和重启
    #[allow(dead_code)]
    pub exe_path: PathBuf, // 用于重启
    #[allow(dead_code)]
    pub config_path: PathBuf, // 用于重启
    pid: u32,               // 进程 ID
}

impl FrpcProcess {
    /// 从已有的 PID 恢复进程跟踪（用于重启后恢复状态）
    pub fn from_pid(pid: u32, identifier: String, exe_path: PathBuf, config_path: PathBuf) -> Self {
        FrpcProcess {
            child: None,
            identifier,
            exe_path,
            config_path,
            pid,
        }
    }

    /// 获取进程 ID
    #[allow(dead_code)]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// 检查是否有子进程句柄
    #[allow(dead_code)]
    pub fn has_child_handle(&self) -> bool {
        self.child.is_some()
    }

    /// 检查指定 PID 是否仍在运行
    pub fn is_pid_running(pid: u32) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let output = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            if let Ok(out) = output {
                let stdout = String::from_utf8_lossy(&out.stdout);
                return stdout.contains(&pid.to_string());
            }
            false
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            false
        }
    }

    /// 通过 PID 终止进程
    pub fn kill_pid(pid: u32) -> Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .context(format!("无法终止进程 PID: {}", pid))?;
            log::info!("已终止进程 PID: {}", pid);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            Err(anyhow::anyhow!("当前平台不支持按 PID 终止进程"))
        }
    }
}

impl FrpcProcess {
    /// 启动一个 frpc 进程实例，并将其标准输出和错误输出重定向到日志
    ///
    /// `on_connected` 回调在检测到 "login to server success" 时触发（仅一次）
    pub fn start(
        identifier: String,
        exe_path: PathBuf,
        config_path: PathBuf,
        on_connected: Option<Sender<()>>,
    ) -> Result<Self> {
        // 验证文件存在
        if !exe_path.exists() {
            log::error!("[{}] 未找到可执行文件: {:?}", identifier, exe_path);
            return Err(anyhow::anyhow!(
                "[{}] 未找到可执行文件: {:?}",
                identifier,
                exe_path
            ));
        }
        if !config_path.exists() {
            log::error!("[{}] 未找到配置文件: {:?}", identifier, config_path);
            return Err(anyhow::anyhow!(
                "[{}] 未找到配置文件: {:?}",
                identifier,
                config_path
            ));
        }
        log::info!("[{}] 找到 frpc.exe: {:?}", identifier, exe_path);
        log::info!("[{}] 找到 frpc.toml: {:?}", identifier, config_path);

        // 启动 frpc 进程，并捕获标准输出和标准错误
        let mut cmd = Command::new(&exe_path);
        cmd.arg("-c")
            .arg(&config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Windows: 隐藏控制台窗口
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().context(format!(
            "[{}] 无法启动 frpc 进程: {:?}",
            identifier, exe_path
        ))?;
        log::info!("[{}] frpc 进程启动成功，PID: {}", identifier, child.id());
        let pid = child.id();

        // 为日志捕获克隆标识符
        let log_identifier_stdout = identifier.clone();
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        let cleaned_bytes = strip(line);
                        let cleaned_line = String::from_utf8_lossy(&cleaned_bytes).into_owned();
                        log::info!("FRPC STDOUT [{}]: {}", log_identifier_stdout, cleaned_line);
                        if cleaned_line.contains("login to server success") {
                            if let Some(ref tx) = on_connected {
                                let _ = tx.send(());
                            }
                        }
                    }
                }
            });
        }

        let log_identifier_stderr = identifier.clone();
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        let cleaned_bytes = strip(line);
                        let cleaned_line = String::from_utf8_lossy(&cleaned_bytes).into_owned();
                        log::error!("FRPC STDERR [{}]: {}", log_identifier_stderr, cleaned_line);
                    }
                }
            });
        }

        Ok(FrpcProcess {
            child: Some(child),
            identifier,
            exe_path,
            config_path,
            pid,
        })
    }

    /// 停止 frpc 进程
    pub fn stop(&mut self) -> Result<()> {
        log::info!(
            "[{}] 尝试终止 frpc 进程，PID: {}",
            self.identifier,
            self.pid
        );
        if let Some(ref mut child) = self.child {
            child
                .kill()
                .context(format!("[{}] 无法终止 frpc 进程", self.identifier))?;
            child
                .wait()
                .context(format!("[{}] 无法等待 frpc 进程终止", self.identifier))?;
        } else {
            // 只有 PID，通过 taskkill 终止
            Self::kill_pid(self.pid)?;
        }
        log::info!("[{}] frpc 进程已停止", self.identifier);
        Ok(())
    }

    /// 检查 frpc 进程是否仍在运行
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    log::warn!(
                        "[{}] frpc 进程已退出，退出状态: {}",
                        self.identifier,
                        status
                    );
                    false
                }
                Ok(None) => true,
                Err(e) => {
                    log::error!("[{}] 无法检查 frpc 进程状态: {}", self.identifier, e);
                    false
                }
            }
        } else {
            // 只有 PID，通过 tasklist 检查
            Self::is_pid_running(self.pid)
        }
    }

    /// 检查 frpc 进程是否已退出（返回退出状态）
    pub fn check_exit_status(&mut self) -> Option<std::process::ExitStatus> {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => Some(status),
                _ => None,
            }
        } else {
            // 只有 PID，检查是否还在运行
            if !Self::is_pid_running(self.pid) {
                // 进程已退出，返回一个模拟的退出状态
                Some(std::process::ExitStatus::default())
            } else {
                None
            }
        }
    }

    // 检查 frpc 进程是否已退出
    #[allow(dead_code)]
    pub fn check_status(&mut self) -> Result<Option<ExitStatus>> {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    log::warn!(
                        "[{}] frpc 子进程已退出，退出状态: {}",
                        self.identifier,
                        status
                    );
                    Ok(Some(status))
                }
                Ok(None) => Ok(None),
                Err(e) => {
                    log::error!("[{}] 无法检查 frpc 子进程状态: {}", self.identifier, e);
                    Err(e.into())
                }
            }
        } else {
            // 只有 PID
            if !Self::is_pid_running(self.pid) {
                log::warn!("[{}] frpc 进程已退出（PID: {}）", self.identifier, self.pid);
                Ok(Some(std::process::ExitStatus::default()))
            } else {
                Ok(None)
            }
        }
    }
}
