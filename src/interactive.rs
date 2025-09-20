//! 处理用户交互模式下的服务管理逻辑（注册、删除、停止）

use anyhow::{Result, Context};
use std::env;
use std::path::PathBuf;
use std::ffi::OsString;
use std::time::Duration;
use windows_service::{
    service::{ServiceAccess, ServiceState, Service, ServiceInfo, ServiceType, ServiceStartType, ServiceErrorControl},
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows::{
    core::w,
    Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_OK, MB_YESNOCANCEL, IDYES, IDNO, MB_ICONINFORMATION, MB_ICONQUESTION,
    },
};

const SERVICE_NAME: &str = "FrpcService";
const DISPLAY_NAME: &str = "FRP Client Service";
pub const SERVICE_ARG: &str = "--service";

/// 运行交互模式的入口函数
pub fn run() -> Result<()> {
    // 检查服务是否已存在
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::all())?;

    // 尝试打开服务
    if let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::DELETE) {
        // 服务已注册，检查其运行状态
        let status = service.query_status()?;
        match status.current_state {
            ServiceState::Running => {
                log::info!("服务 FrpcService 正在运行。");
                handle_running_service(&manager)?;
            }
            ServiceState::Stopped => {
                log::info!("服务 FrpcService 已停止。");
                handle_stopped_service(&manager)?;
            }
            _ => {
                log::warn!("服务 FrpcService 处于未知状态：{:?}", status.current_state);
                // 可以提供一个通用的处理选项，或者直接退出
                unsafe {
                    MessageBoxW(
                        None,
                        w!("服务处于非预期状态，请手动处理。"),
                        w!("警告"),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
            }
        }
    } else {
        // 服务不存在
        log::info!("服务 FrpcService 未注册。");
        handle_first_installation()?;
    }

    Ok(())
}

/// 处理服务正在运行的情况
fn handle_running_service(manager: &ServiceManager) -> Result<()> {
    let result = unsafe {
        MessageBoxW(
            None,
            w!("服务 FrpcService 已在运行中。\n\n\
            请选择您要执行的操作：\n\n\
            - 是 (Yes): 删除服务并停止所有实例。\n\
            - 否 (No): 仅停止所有实例，但不删除服务。\n\
            - 取消 (Cancel): 退出程序，不做任何更改。"),
            w!("服务管理"),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };

    match result {
        IDYES => {
            // 删除服务
            let service = manager.open_service(
                SERVICE_NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
            )?;
            stop_service_and_wait(&service, SERVICE_NAME)?;
            log::info!("尝试删除服务 {}", SERVICE_NAME);
            service.delete().context(format!("无法删除服务 {}", SERVICE_NAME))?;
            log::info!("服务 {} 已删除", SERVICE_NAME);
            unsafe {
                MessageBoxW(
                    None,
                    w!("服务 FrpcService 已成功删除。"),
                    w!("操作完成"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        IDNO => {
            // 仅停止服务
            let service = manager.open_service(
                SERVICE_NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
            )?;
            stop_service_and_wait(&service, SERVICE_NAME)?;
            unsafe {
                MessageBoxW(
                    None,
                    w!("服务 FrpcService 及其所有实例已成功停止。"),
                    w!("操作完成"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        _ => {
            log::info!("用户选择取消操作，程序退出。");
        }
    }
    Ok(())
}


/// 处理服务已停止的情况
fn handle_stopped_service(manager: &ServiceManager) -> Result<()> {
    let result = unsafe {
        MessageBoxW(
            None,
            w!("服务 FrpcService 已停止。\n\n\
            请选择您要执行的操作：\n\n\
            - 是 (Yes): 启动服务。\n\
            - 否 (No): 删除服务。\n\
            - 取消 (Cancel): 退出程序，不做任何更改。"),
            w!("服务管理"),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };

    match result {
        IDYES => {
            // 启动服务
            start_registered_service()?;
            unsafe {
                MessageBoxW(
                    None,
                    w!("服务 FrpcService 已成功启动。"),
                    w!("操作完成"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        IDNO => {
            // 删除服务
            let service = manager.open_service(SERVICE_NAME, ServiceAccess::DELETE)?;
            service.delete().context(format!("无法删除服务 {}", SERVICE_NAME))?;
            log::info!("服务 {} 已删除", SERVICE_NAME);
            unsafe {
                MessageBoxW(
                    None,
                    w!("服务 FrpcService 已成功删除。"),
                    w!("操作完成"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        _ => {
            log::info!("用户选择取消操作，程序退出。");
        }
    }
    Ok(())
}

/// 处理首次安装服务的情况
fn handle_first_installation() -> Result<()> {
    install_service()?;
    start_registered_service()?;
    unsafe {
        MessageBoxW(
            None,
            w!("服务已成功注册为 FrpcService 并已启动。"),
            w!("提示"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
    Ok(())
}

/// 注册 Windows 服务
fn install_service() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;
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
    service.start(&[] as &[&str]).context(format!("无法启动服务 {}", SERVICE_NAME))?;
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
fn stop_service_and_wait(service: &Service, service_name: &str) -> Result<()> {
    let status = service.query_status()?;
    if status.current_state != ServiceState::Stopped {
        log::info!("尝试停止服务 {}", service_name);
        service.stop().context(format!("无法停止服务 {}", service_name))?;
        let max_wait = Duration::from_secs(10);
        let start = std::time::Instant::now();
        loop {
            let status = service.query_status()?;
            if status.current_state == ServiceState::Stopped {
                log::info!("服务 {} 已停止", service_name);
                break;
            }
            if start.elapsed() > max_wait {
                unsafe {
                    MessageBoxW(
                        None,
                        w!("停止服务超时，请在系统服务管理器中手动处理。"),
                        w!("错误"),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
                return Err(anyhow::anyhow!("服务 {} 停止超时", service_name));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    } else {
        log::info!("服务 {} 已经处于停止状态。", service_name);
    }
    Ok(())
}