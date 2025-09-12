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

    let mut frpc_process = FrpcProcess::start().context("首次启动 frpc 进程失败")?;

    set_service_status(&status_handle, ServiceState::Running)?;
    log::info!("服务 FrpcService 启动成功，进入监控循环");

    let mut restart_attempts = 0;
    loop {
        match shutdown_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                log::info!("收到停止信号或通道已断开，准备停止服务。");
                break;
            }
            Err(TryRecvError::Empty) => {}
        }

        if let Some(_exit_status) = frpc_process.check_status()? {
            log::warn!("检测到 frpc 进程已退出。");
            restart_attempts += 1;

            if restart_attempts > MAX_RESTART_ATTEMPTS {
                log::error!(
                    "frpc 进程重启次数已达上限 ({}/{})，将停止服务。",
                    restart_attempts - 1,
                    MAX_RESTART_ATTEMPTS
                );
                status_handle.set_service_status(ServiceStatus {
                    service_type: ServiceType::OWN_PROCESS,
                    current_state: ServiceState::Stopped,
                    controls_accepted: ServiceControlAccept::empty(),
                    exit_code: ServiceExitCode::ServiceSpecific(1),
                    checkpoint: 0,
                    wait_hint: Duration::ZERO,
                    process_id: None,
                })?;
                return Err(anyhow::anyhow!("frpc 进程重启次数过多，服务停止"));
            }

            log::info!(
                "尝试第 {} 次重启 frpc 进程...",
                restart_attempts
            );
            match FrpcProcess::start() {
                Ok(new_process) => {
                    frpc_process = new_process;
                    log::info!("frpc 进程重启成功。");
                }
                Err(e) => {
                    log::error!("frpc 进程重启失败: {:?}，将停止服务。", e);
                    status_handle.set_service_status(ServiceStatus {
                        service_type: ServiceType::OWN_PROCESS,
                        current_state: ServiceState::Stopped,
                        controls_accepted: ServiceControlAccept::empty(),
                        exit_code: ServiceExitCode::ServiceSpecific(1),
                        checkpoint: 0,
                        wait_hint: Duration::ZERO,
                        process_id: None,
                    })?;
                    return Err(e);
                }
            }
        }

        std::thread::sleep(CHECK_INTERVAL);
    }

    log::info!("正在停止 frpc 进程...");
    frpc_process.stop()?;

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