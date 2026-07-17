//! 程序入口，根据命令行参数分发到服务模式或交互模式

#![windows_subsystem = "windows"]
mod app;
mod config;
mod download;
mod frpc_mg;
mod icons;
mod logger;
mod message;
mod pages;
mod service;
mod sidebar;
mod theme;

use crate::logger::init_logging;
use anyhow::{Context, Result};
use std::env;

/// 检查是否已有实例在运行，如果没有则创建互斥量
#[cfg(windows)]
fn ensure_single_instance() -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "FrpcService_SingleInstance_Mutex\0"
        .encode_utf16()
        .collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr());
        if handle == 0 as HANDLE {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return None;
        }
        Some(handle)
    }
}

fn main() -> Result<()> {
    // 交互模式下检查单实例
    let _mutex_guard = if !env::args().any(|a| a == service::SERVICE_ARG) {
        match ensure_single_instance() {
            Some(h) => Some(h),
            None => return Ok(()),
        }
    } else {
        None
    };

    init_logging().context("无法初始化日志")?;

    let args: Vec<String> = env::args().collect();
    if args.contains(&service::SERVICE_ARG.to_string()) {
        log::info!("在服务模式下启动，即将进入服务调度器");
        service::run_service_dispatcher().context("服务调度器启动失败")
    } else {
        log::info!("在交互模式下启动");
        service::check_and_run_app().context("交互模式运行失败")
    }
}
