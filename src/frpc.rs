//! frpc 进程管理，负责启动和停止 frpc 进程

use std::env;
use std::process::{Child, Command, Stdio};
use std::io::{BufReader, BufRead};
use anyhow::{Result, Context};
use strip_ansi_escapes::strip;

pub struct FrpcProcess {
    child: Child,
}

impl FrpcProcess {
    /// 启动 frpc 进程，并将其标准输出和错误输出重定向到日志
    pub fn start() -> Result<Self> {
        // 获取当前可执行文件所在目录
        let exe_path = env::current_exe().context("无法获取可执行文件路径")?;
        let exe_dir = exe_path
            .parent()
            .context("无法获取可执行文件目录")?;
        log::info!("可执行文件目录: {:?}", exe_dir);

        // 构建 frpc.exe 和 frpc.toml 的路径
        let frpc_exe = exe_dir.join("frpc.exe");
        let frpc_config = exe_dir.join("frpc.toml");

        // 验证文件存在
        if !frpc_exe.exists() {
            log::error!("未找到 frpc.exe: {:?}", frpc_exe);
            return Err(anyhow::anyhow!("未找到 frpc.exe: {:?}", frpc_exe));
        }
        if !frpc_config.exists() {
            log::error!("未找到 frpc.toml: {:?}", frpc_config);
            return Err(anyhow::anyhow!("未找到 frpc.toml: {:?}", frpc_config));
        }
        log::info!("找到 frpc.exe: {:?}", frpc_exe);
        log::info!("找到 frpc.toml: {:?}", frpc_config);

        // 启动 frpc 进程，并捕获标准输出和标准错误
        let mut child = Command::new(&frpc_exe)
            .arg("-c")
            .arg(&frpc_config)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(format!("无法启动 frpc 进程: {:?}", frpc_exe))?;
        log::info!("frpc 进程启动成功，PID: {}", child.id());

        // 捕获标准输出并写入日志
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        // 在写入日志前，移除 ANSI 转义码
                        let cleaned_bytes = strip(line);
                        let cleaned_line = String::from_utf8_lossy(&cleaned_bytes).into_owned();
                        log::info!("FRPC STDOUT: {}", cleaned_line);
                    }
                }
            });
        }

        // 捕获标准错误并写入日志
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        // 在写入日志前，移除 ANSI 转义码
                        let cleaned_bytes = strip(line);
                        let cleaned_line = String::from_utf8_lossy(&cleaned_bytes).into_owned();
                        log::error!("FRPC STDERR: {}", cleaned_line);
                    }
                }
            });
        }

        Ok(FrpcProcess { child })
    }

    /// 停止 frpc 进程
    pub fn stop(&mut self) -> Result<()> {
        log::info!("尝试终止 frpc 进程，PID: {}", self.child.id());
        self.child.kill().context("无法终止 frpc 进程")?;
        self.child.wait().context("无法等待 frpc 进程终止")?;
        log::info!("frpc 进程已停止");
        Ok(())
    }
}