//! 程序入口，自动注册服务并启动，处理二次运行逻辑

mod service;
mod frpc;
mod logger;

use windows_service::{
    service::{ServiceAccess, ServiceInfo, ServiceState, ServiceStartType, ServiceType, ServiceErrorControl},
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows::{
    core::w,
    Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONINFORMATION},
};
use std::env;
use std::io::{self, Write};
use std::time::Duration;
use anyhow::{Result, Context};
use crate::logger::init_logging;

const SERVICE_NAME: &str = "FrpcService";
const DISPLAY_NAME: &str = "FRP Client Service";
const SERVICE_ARG: &str = "--service";

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.contains(&SERVICE_ARG.to_string()) {
        init_logging().context("无法初始化日志")?;
        log::info!("在服务模式下启动，即将进入服务调度器");
        return service::run_service_dispatcher().context("服务调度器启动失败");
    }

    init_logging().context("无法初始化日志")?;

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;

    match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
        Ok(service) => {
            println!("服务 {} 已存在，是否停止并删除[no]？(输入 'yes' 或 'y' 确认):", SERVICE_NAME);
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim().to_lowercase();

            if input == "yes" || input == "y" {
                let status = service.query_status()?;
                if status.current_state != ServiceState::Stopped {
                    log::info!("尝试停止服务 {}", SERVICE_NAME);
                    service.stop().context(format!("无法停止服务 {}", SERVICE_NAME))?;
                    let max_wait = Duration::from_secs(10);
                    let start = std::time::Instant::now();
                    loop {
                        if service.query_status()?.current_state == ServiceState::Stopped {
                            log::info!("服务 {} 已停止", SERVICE_NAME);
                            break;
                        }
                        if start.elapsed() > max_wait {
                            return Err(anyhow::anyhow!("服务 {} 停止超时", SERVICE_NAME));
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
                std::thread::sleep(Duration::from_secs(1));

                log::info!("尝试删除服务 {}", SERVICE_NAME);
                service.delete().context(format!("无法删除服务 {}", SERVICE_NAME))?;
                log::info!("服务 {} 已删除", SERVICE_NAME);

                unsafe {
                    MessageBoxW(
                        None,
                        w!("服务 FrpcService 已成功删除，请重新运行程序以注册服务。"),
                        w!("提示"),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
            } else {
                println!("保留现有服务，程序退出。");
            }
        }
        Err(_) => {
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
        }
    }

    Ok(())
}

/// 注册 Windows 服务
fn install_service() -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CREATE_SERVICE
    )?;

    let exe_path = env::current_exe()?;

    // Use `ServiceInfo` struct
    let service_info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe_path,
        launch_arguments: vec![SERVICE_ARG.into()],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    manager.create_service(&service_info, ServiceAccess::all()).context("无法创建服务")?;

    log::info!("服务 {} 已成功注册", SERVICE_NAME);
    Ok(())
}

/// 启动已注册的 Windows 服务
fn start_registered_service() -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT
    )?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::START | ServiceAccess::QUERY_STATUS,
    )?;

    log::info!("尝试启动服务 {}", SERVICE_NAME);
    service.start(&[] as &[&str]).context(format!("无法启动服务 {}", SERVICE_NAME))?;

    let max_wait = Duration::from_secs(10);
    let start = std::time::Instant::now();
    loop {
        if service.query_status()?.current_state == ServiceState::Running {
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