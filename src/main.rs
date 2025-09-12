//! 程序入口，自动注册服务并启动，处理二次运行逻辑

#![windows_subsystem = "windows"]
mod service;
mod frpc;
mod logger;

use windows_service::{
    service::{ServiceAccess, ServiceState},
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows::{
    core::w,
    Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_YESNO, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION},
};
use std::env;
use std::path::PathBuf;
use std::ffi::OsString;
use std::time::Duration;
use anyhow::{Result, Context};
use crate::logger::init_logging;

const SERVICE_NAME: &str = "FrpcService";
const DISPLAY_NAME: &str = "FRP Client Service";
const SERVICE_ARG: &str = "--service";

fn main() -> Result<()> {
    // 检查命令行参数，判断是否为服务模式。
    let args: Vec<String> = env::args().collect();
    if args.contains(&SERVICE_ARG.to_string()) {
        // 服务模式：由 SCM 启动，直接进入服务调度器
        init_logging().context("无法初始化日志")?;
        log::info!("在服务模式下启动，即将进入服务调度器");
        return service::run_service_dispatcher().context("服务调度器启动失败");
    }

    // 手动运行模式
    // 初始化日志
    init_logging().context("无法初始化日志")?;

    // 检查服务是否已存在
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;
    let service_exists = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).is_ok();

    if service_exists {
        // 服务已存在，使用 Windows 弹窗提示用户是否删除
        let result = unsafe {
            MessageBoxW(
                None,
                w!("服务 FrpcService 已存在，是否停止并删除？此操作可能需要几秒钟。"),
                w!("确认"),
                MB_YESNO | MB_ICONQUESTION,
            )
        };

        if result == IDYES {
            let service = manager.open_service(
                SERVICE_NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
            )?;
            let status = service.query_status()?;
            if status.current_state != ServiceState::Stopped {
                log::info!("尝试停止服务 {}", SERVICE_NAME);
                service.stop().context(format!("无法停止服务 {}", SERVICE_NAME))?;
                // 等待服务停止，最多 10 秒
                let max_wait = Duration::from_secs(10);
                let start = std::time::Instant::now();
                loop {
                    let status = service.query_status()?;
                    if status.current_state == ServiceState::Stopped {
                        log::info!("服务 {} 已停止", SERVICE_NAME);
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
                        return Err(anyhow::anyhow!("服务 {} 停止超时", SERVICE_NAME));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }

            log::info!("尝试删除服务 {}", SERVICE_NAME);
            service.delete().context(format!("无法删除服务 {}", SERVICE_NAME))?;
            log::info!("服务 {} 已删除", SERVICE_NAME);

            // 显示最终删除成功提示窗口
            unsafe {
                MessageBoxW(
                    None,
                    w!("服务 FrpcService 已成功删除。"),
                    w!("操作完成"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
    } else {
        // 服务不存在，注册服务
        install_service()?;

        // 注册成功后，自动启动服务
        start_registered_service()?;

        // 显示注册成功提示窗口
        unsafe {
            MessageBoxW(
                None,
                w!("服务已成功注册为 FrpcService 并已启动。"),
                w!("提示"),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    Ok(())
}

/// 注册 Windows 服务
fn install_service() -> Result<()> {
    // 获取服务管理器
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;

    // 获取当前可执行文件路径
    let exe_path = env::current_exe()?;
    let exe_path_str = exe_path.to_string_lossy().into_owned();

    // 创建服务，并指定由SCM启动时需要传入的参数
    manager.create_service(
        &windows_service::service::ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from(DISPLAY_NAME),
            service_type: windows_service::service::ServiceType::OWN_PROCESS,
            start_type: windows_service::service::ServiceStartType::AutoStart,
            error_control: windows_service::service::ServiceErrorControl::Normal,
            executable_path: PathBuf::from(exe_path_str),
            launch_arguments: vec![OsString::from(SERVICE_ARG)], // 添加 --service 参数
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
    // 获取服务管理器
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::START | ServiceAccess::QUERY_STATUS,
    )?;

    log::info!("尝试启动服务 {}", SERVICE_NAME);
    service.start(&[] as &[&str]).context(format!("无法启动服务 {}", SERVICE_NAME))?;

    // 等待服务运行，最多 10 秒
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
