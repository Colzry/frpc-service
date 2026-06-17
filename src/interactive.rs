//! 处理用户交互模式下的服务管理逻辑（注册、删除、停止）

use anyhow::{Context, Result};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::dialog;

const SERVICE_NAME: &str = "FrpcService";
const DISPLAY_NAME: &str = "FRP Client Service";
pub const SERVICE_ARG: &str = "--service";

/// 服务预检查结果
#[derive(Clone, Debug)]
pub(crate) enum PreCheckResult {
    /// 服务正在运行
    Running,
    /// 服务已停止
    Stopped,
    /// 服务未注册
    NotRegistered,
}

/// 检查服务状态
pub(crate) fn check_service_status() -> Result<PreCheckResult> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;

    if let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        let status = service.query_status()?;
        match status.current_state {
            ServiceState::Running => {
                log::info!("服务 FrpcService 正在运行。");
                Ok(PreCheckResult::Running)
            }
            ServiceState::Stopped => {
                log::info!("服务 FrpcService 已停止。");
                Ok(PreCheckResult::Stopped)
            }
            _ => {
                log::warn!(
                    "服务 FrpcService 处于未知状态：{:?}",
                    status.current_state
                );
                Err(anyhow::anyhow!(
                    "服务处于非预期状态：{:?}",
                    status.current_state
                ))
            }
        }
    } else {
        log::info!("服务 FrpcService 未注册。");
        Ok(PreCheckResult::NotRegistered)
    }
}

/// 运行交互模式的入口函数
pub fn run() -> Result<()> {
    let pre_check = check_service_status()?;
    dialog::run_service_dialog(pre_check);
    Ok(())
}

/// 操作：删除服务并停止
pub(crate) fn op_delete_and_stop() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::all())?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    )?;
    stop_service_and_wait(&service, SERVICE_NAME)?;
    log::info!("尝试删除服务 {}", SERVICE_NAME);
    service
        .delete()
        .context(format!("无法删除服务 {}", SERVICE_NAME))?;
    log::info!("服务 {} 已删除", SERVICE_NAME);
    Ok(())
}

/// 操作：仅停止服务
pub(crate) fn op_stop_only() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::all())?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
    )?;
    stop_service_and_wait(&service, SERVICE_NAME)?;
    Ok(())
}

/// 操作：启动服务
pub(crate) fn op_start() -> Result<()> {
    start_registered_service()
}

/// 操作：删除服务
pub(crate) fn op_delete() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::all())?;
    let service = manager.open_service(SERVICE_NAME, ServiceAccess::DELETE)?;
    service
        .delete()
        .context(format!("无法删除服务 {}", SERVICE_NAME))?;
    log::info!("服务 {} 已删除", SERVICE_NAME);
    Ok(())
}

/// 操作：安装并启动服务
pub(crate) fn op_install_and_start() -> Result<()> {
    install_service()?;
    start_registered_service()?;
    Ok(())
}

/// 注册 Windows 服务
fn install_service() -> Result<()> {
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;
    let exe_path = env::current_exe()?;
    manager.create_service(
        &ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from(DISPLAY_NAME),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: PathBuf::from(exe_path),
            launch_arguments: vec![OsString::from(SERVICE_ARG)],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        },
        ServiceAccess::all(),
    )?;
    log::info!("服务 {} 已成功注册", SERVICE_NAME);
    Ok(())
}

/// 启动已注册的 Windows 服务
fn start_registered_service() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::START | ServiceAccess::QUERY_STATUS,
    )?;
    log::info!("尝试启动服务 {}", SERVICE_NAME);
    service
        .start(&[] as &[&str])
        .context(format!("无法启动服务 {}", SERVICE_NAME))?;
    // 等待服务运行
    let max_wait = Duration::from_secs(10);
    let start = std::time::Instant::now();
    loop {
        let status = service.query_status()?;
        if status.current_state == ServiceState::Running {
            log::info!("服务 {} 启动成功", SERVICE_NAME);
            break;
        }
        if start.elapsed() > max_wait {
            return Err(anyhow::anyhow!("服务 {} 启动超时", SERVICE_NAME));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// 停止服务并等待其完成
fn stop_service_and_wait(
    service: &windows_service::service::Service,
    service_name: &str,
) -> Result<()> {
    let status = service.query_status()?;
    if status.current_state != ServiceState::Stopped {
        log::info!("尝试停止服务 {}", service_name);
        service
            .stop()
            .context(format!("无法停止服务 {}", service_name))?;
        let max_wait = Duration::from_secs(10);
        let start = std::time::Instant::now();
        loop {
            let status = service.query_status()?;
            if status.current_state == ServiceState::Stopped {
                log::info!("服务 {} 已停止", service_name);
                break;
            }
            if start.elapsed() > max_wait {
                return Err(anyhow::anyhow!(
                    "服务 {} 停止超时，请在系统服务管理器中手动处理。",
                    service_name
                ));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    } else {
        log::info!("服务 {} 已经处于停止状态。", service_name);
    }
    Ok(())
}
