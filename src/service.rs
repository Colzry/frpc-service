//! Windows 服务逻辑，管理服务的生命周期（使用windows-service库）

use anyhow::{Result, Context};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::time::Duration;
use windows_service::{
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};
use crate::frpc::FrpcProcess;

const SERVICE_NAME: &str = "FrpcService";

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
                log::info!("收到停止或关闭信号");
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .context("无法注册服务控制处理程序")?;

    let next_status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        // 【FIX 4】: The correct variant for no error is `Win32(0)`.
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(1),
        process_id: None,
    };
    status_handle.set_service_status(next_status)?;
    log::info!("服务状态设置为 START_PENDING");

    let mut frpc_process = FrpcProcess::start().context("无法启动 frpc 进程")?;

    let next_status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    };
    status_handle.set_service_status(next_status)?;
    log::info!("服务 FrpcService 启动成功");

    log::info!("等待停止信号");
    shutdown_rx.recv().unwrap();
    log::info!("收到停止信号");

    log::info!("停止 frpc 进程");
    frpc_process.stop()?;

    let next_status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    };
    status_handle.set_service_status(next_status)?;
    log::info!("服务状态设置为 STOPPED");

    Ok(())
}