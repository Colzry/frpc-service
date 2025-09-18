//! frpc 进程管理，负责启动和停止 frpc 进程

use std::path::PathBuf;
use std::process::{Child, Command, Stdio, ExitStatus};
use std::io::{BufReader, BufRead};
use anyhow::{Result, Context};
use strip_ansi_escapes::strip;

pub struct FrpcProcess {
    child: Child,
    pub identifier: String, // 用于日志和重启
    pub exe_path: PathBuf,      // 用于重启
    pub config_path: PathBuf,   // 用于重启
}

impl FrpcProcess {
    /// 启动一个 frpc 进程实例，并将其标准输出和错误输出重定向到日志
    pub fn start(
        identifier: String,
        exe_path: PathBuf,
        config_path: PathBuf,
    ) -> Result<Self> {
        // 验证文件存在
        if !exe_path.exists() {
            log::error!("[{}] 未找到可执行文件: {:?}", identifier, exe_path);
            return Err(anyhow::anyhow!("[{}] 未找到可执行文件: {:?}", identifier, exe_path));
        }
        if !config_path.exists() {
            log::error!("[{}] 未找到配置文件: {:?}", identifier, config_path);
            return Err(anyhow::anyhow!("[{}] 未找到配置文件: {:?}", identifier, config_path));
        }
        log::info!("[{}] 找到 frpc.exe: {:?}", identifier, exe_path);
        log::info!("[{}] 找到 frpc.toml: {:?}", identifier, config_path);

        // 启动 frpc 进程，并捕获标准输出和标准错误
        let mut child = Command::new(&exe_path)
            .arg("-c")
            .arg(&config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(format!("[{}] 无法启动 frpc 进程: {:?}", identifier, exe_path))?;
        log::info!("[{}] frpc 进程启动成功，PID: {}", identifier, child.id());

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
            child,
            identifier,
            exe_path,
            config_path,
        })
    }

    /// 停止 frpc 进程
    pub fn stop(&mut self) -> Result<()> {
        log::info!("[{}] 尝试终止 frpc 进程，PID: {}", self.identifier, self.child.id());
        self.child.kill().context(format!("[{}] 无法终止 frpc 进程", self.identifier))?;
        self.child.wait().context(format!("[{}] 无法等待 frpc 进程终止", self.identifier))?;
        log::info!("[{}] frpc 进程已停止", self.identifier);
        Ok(())
    }

    // 检查 frpc 进程是否已退出
    pub fn check_status(&mut self) -> Result<Option<ExitStatus>> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                log::warn!("[{}] frpc 子进程已退出，退出状态: {}", self.identifier, status);
                Ok(Some(status))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                log::error!("[{}] 无法检查 frpc 子进程状态: {}", self.identifier, e);
                Err(e.into())
            }
        }
    }
}