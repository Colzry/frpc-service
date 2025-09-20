//! 程序入口，根据命令行参数分发到服务模式或交互模式

#![windows_subsystem = "windows"]
mod service;
mod frpc;
mod logger;
mod interactive;

use std::env;
use anyhow::{Result, Context};
use crate::logger::init_logging;

fn main() -> Result<()> {
    // 提前初始化日志，确保所有模式都能记录日志
    init_logging().context("无法初始化日志")?;

    // 检查命令行参数，判断运行模式
    let args: Vec<String> = env::args().collect();
    if args.contains(&interactive::SERVICE_ARG.to_string()) {
        // 服务模式：由 SCM (服务控制管理器) 启动
        log::info!("在服务模式下启动，即将进入服务调度器");
        service::run_service_dispatcher().context("服务调度器启动失败")
    } else {
        // 交互模式：用户手动运行
        log::info!("在交互模式下启动");
        interactive::run().context("交互模式运行失败")
    }
}