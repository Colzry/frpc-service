//! Windows 服务逻辑，管理服务的生命周期（使用windows-service库）

use anyhow::{Result, Context};
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use std::time::Duration;
use windows_service::{
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};
use crate::frpc::FrpcProcess;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

const SERVICE_NAME: &str = "FrpcService";
const MAX_RESTART_ATTEMPTS: u32 = 3;
const CHECK_INTERVAL: Duration = Duration::from_secs(5);

extern "system" fn service_main(_arguments: u32, _argv: *mut *mut u16) {
    log::info!("服务主函数被调用");
    if let Err(e) = run_service() {
        log::error!("服务运行失败: {:?}", e);
    }
}

pub fn run_service_dispatcher() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, service_main)?;
    Ok(())
}

/// 发现所有需要启动的 frpc 实例
fn discover_frpc_instances() -> Result<Vec<(String, PathBuf, PathBuf)>> {
    let mut instances = Vec::new();
    let exe_path = env::current_exe().context("无法获取可执行文件路径")?;
    let exe_dir = exe_path.parent().context("无法获取可执行文件目录")?;

    // 1. 查找默认实例: frpc.exe 和 frpc.toml
    let default_exe = exe_dir.join("frpc.exe");
    let default_config = exe_dir.join("frpc.toml");
    if default_exe.exists() && default_config.exists() {
        instances.push(("default".to_string(), default_exe, default_config));
    }

    // 2. 查找命名实例: frpc@<name>.exe 和 <name>.toml
    for entry in std::fs::read_dir(exe_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with("frpc@") && file_name.ends_with(".exe") {
                    let name_part = &file_name["frpc@".len()..file_name.len() - ".exe".len()];
                    if !name_part.is_empty() {
                        let config_file = exe_dir.join(format!("{}.toml", name_part));
                        if config_file.exists() {
                            instances.push((name_part.to_string(), path.clone(), config_file));
                        } else {
                            log::warn!("找到可执行文件 {:?}，但未找到对应的配置文件 {:?}", path, config_file);
                        }
                    }
                }
            }
        }
    }

    if instances.is_empty() {
        log::warn!("未发现任何有效的 frpc 实例可供启动。");
    }

    Ok(instances)
}

fn run_service() -> Result<()> {
    log::info!("进入 run_service");

    let (shutdown_tx, shutdown_rx): (Sender<()>, Receiver<()>) = channel();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                log::info!("收到来自 SCM 的停止或关闭信号");
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .context("无法注册服务控制处理程序")?;

    set_service_status(&status_handle, ServiceState::StartPending)?;
    log::info!("服务状态设置为 START_PENDING");

    // 发现并启动所有 frpc 实例
    let instance_configs = discover_frpc_instances()?;
    let mut frpc_processes: Vec<FrpcProcess> = Vec::new();
    for (id, exe, conf) in instance_configs {
        match FrpcProcess::start(id, exe, conf) {
            Ok(process) => frpc_processes.push(process),
            Err(e) => log::error!("启动 frpc 实例失败: {:?}", e),
        }
    }

    if frpc_processes.is_empty() {
        log::error!("没有任何 frpc 进程成功启动，服务将停止。");
        set_service_status(&status_handle, ServiceState::Stopped)?;
        return Err(anyhow::anyhow!("没有任何 frpc 进程成功启动"));
    }

    set_service_status(&status_handle, ServiceState::Running)?;
    log::info!("服务 FrpcService 启动成功，进入监控循环");

    let mut restart_attempts: HashMap<String, u32> = HashMap::new();

    loop {
        // 检查停止信号
        match shutdown_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                log::info!("收到停止信号或通道已断开，准备停止服务。");
                break;
            }
            Err(TryRecvError::Empty) => {}
        }

        // 检查所有子进程的状态
        for i in 0..frpc_processes.len() {
            let process = &mut frpc_processes[i];
            if let Some(_exit_status) = process.check_status()? {
                let identifier = process.identifier.clone();
                log::warn!("检测到 frpc 进程 [{}] 已退出。", identifier);

                let attempts = restart_attempts.entry(identifier.clone()).or_insert(0);
                *attempts += 1;

                if *attempts > MAX_RESTART_ATTEMPTS {
                    log::error!(
                        "frpc 进程 [{}] 重启次数已达上限 ({}/{})，将放弃重启此进程。",
                        identifier,
                        *attempts -1,
                        MAX_RESTART_ATTEMPTS
                    );
                    // 可以选择在这里停止整个服务，或仅放弃此进程
                    // 当前选择放弃此进程，让服务继续为其他进程运行
                    continue;
                }

                log::info!(
                    "尝试第 {} 次重启 frpc 进程 [{}]...",
                    *attempts,
                    identifier
                );

                // 使用存储的路径和标识符尝试重启
                match FrpcProcess::start(
                    process.identifier.clone(),
                    process.exe_path.clone(),
                    process.config_path.clone(),
                ) {
                    Ok(new_process) => {
                        // 替换掉旧的、已退出的进程实例
                        frpc_processes[i] = new_process;
                        log::info!("[{}] frpc 进程重启成功。", identifier);
                    }
                    Err(e) => {
                        log::error!("[{}] frpc 进程重启失败: {:?}", identifier, e);
                        // 如果重启失败，可以考虑停止整个服务或稍后重试
                    }
                }
            }
        }

        std::thread::sleep(CHECK_INTERVAL);
    }

    log::info!("正在停止所有 frpc 进程...");
    for process in &mut frpc_processes {
        if let Err(e) = process.stop() {
            log::error!("停止进程 [{}] 时出错: {:?}", process.identifier, e);
        }
    }

    set_service_status(&status_handle, ServiceState::Stopped)?;
    log::info!("服务状态设置为 STOPPED，正常退出。");

    Ok(())
}

fn set_service_status(
    status_handle: &ServiceStatusHandle,
    current_state: ServiceState,
) -> Result<()> {
    let mut controls_accepted = ServiceControlAccept::empty();
    if current_state == ServiceState::Running {
        controls_accepted = ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN;
    }

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state,
        controls_accepted,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    })?;
    Ok(())
}