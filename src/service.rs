//! Windows 服务逻辑，管理服务的生命周期（使用windows-service库）

use crate::frpc::FrpcProcess;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::env;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::Duration;
use windows_service::{
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};

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

/// 清理可能残留的孤儿 frpc 进程
///
/// 当 frpc_service 进程被异常终止（如任务管理器强杀）时，
/// 其管理的 frpc 子进程会变成孤儿进程继续运行。
/// 再次启动服务时，如果不清理这些孤儿进程，会导致重复的 frpc 实例。
fn cleanup_orphan_frpc_processes(exe_dir: &std::path::Path) {
    log::info!("检查并清理可能残留的孤儿 frpc 进程...");

    // 1. 扫描目录，收集需要清理的 frpc 可执行文件名
    let mut exe_names: Vec<String> = Vec::new();
    if exe_dir.join("frpc.exe").exists() {
        exe_names.push("frpc.exe".to_string());
    }
    if let Ok(entries) = std::fs::read_dir(exe_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("frpc@") && name.ends_with(".exe") {
                    exe_names.push(name.to_string());
                }
            }
        }
    }

    if exe_names.is_empty() {
        log::info!("目录中未找到 frpc 可执行文件，跳过清理");
        return;
    }

    let mut cleaned_count = 0;

    // 2. 对每个已知的 frpc 可执行文件名，查找并终止对应的运行进程
    for exe_name in &exe_names {
        let output = match std::process::Command::new("tasklist")
            .args([
                "/FI",
                &format!("IMAGENAME eq {}", exe_name),
                "/FO",
                "CSV",
                "/NH",
            ])
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                log::warn!("tasklist 执行失败: {}", e);
                continue;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("INFO:") {
                continue;
            }
            // CSV 格式: "frpc.exe","12345","Console","1","50,000 K"
            // 用引号分割来安全提取 PID
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let pid = parts[1].trim().trim_matches('"');
                if !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()) {
                    log::info!("清理孤儿进程: {} PID={}", exe_name, pid);
                    match std::process::Command::new("taskkill")
                        .args(["/PID", pid, "/F"])
                        .output()
                    {
                        Ok(o) if o.status.success() => {
                            cleaned_count += 1;
                            log::info!("已终止孤儿进程 PID={}", pid);
                        }
                        Ok(o) => {
                            log::warn!(
                                "终止进程 PID={} 失败: {}",
                                pid,
                                String::from_utf8_lossy(&o.stderr).trim()
                            );
                        }
                        Err(e) => {
                            log::warn!("执行 taskkill 失败 (PID={}): {}", pid, e);
                        }
                    }
                }
            }
        }
    }

    if cleaned_count > 0 {
        log::info!("共清理了 {} 个孤儿 frpc 进程", cleaned_count);
        // 等待一小段时间确保进程完全退出，释放端口等资源
        std::thread::sleep(std::time::Duration::from_secs(2));
    } else {
        log::info!("未发现需要清理的孤儿 frpc 进程");
    }
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
                            log::warn!(
                                "找到可执行文件 {:?}，但未找到对应的配置文件 {:?}",
                                path,
                                config_file
                            );
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

/// 检查网络是否连接可用
fn is_network_connected() -> bool {
    // 尝试连接公共 DNS 端口 (阿里 DNS 和 Google DNS) 验证外网连通性
    let addrs = ["223.5.5.5:53", "8.8.8.8:53"];
    for addr in addrs {
        if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
            // 设置 2 秒超时时间
            if TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2)).is_ok() {
                return true;
            }
        }
    }
    false
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

    // 清理可能残留的孤儿 frpc 进程，防止重复实例
    let exe_path = env::current_exe().context("无法获取可执行文件路径")?;
    let exe_dir = exe_path.parent().context("无法获取可执行文件目录")?;
    cleanup_orphan_frpc_processes(exe_dir);

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
    let mut abandoned: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        // 1. 检查停止信号
        match shutdown_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                log::info!("收到停止信号或通道已断开，准备停止服务。");
                break;
            }
            Err(TryRecvError::Empty) => {}
        }

        // ================= 缓存当前这一轮循环的网络状态 =================
        // 初始为 None，只有在真正需要时才去检测并赋值
        let mut current_network_status: Option<bool> = None;
        // ====================================================================

        // 2. 检查所有子进程的状态
        for i in 0..frpc_processes.len() {
            let process = &mut frpc_processes[i];
            if abandoned.contains(&process.identifier) {
                continue;
            }
            if let Some(_exit_status) = process.check_status()? {
                let identifier = process.identifier.clone();

                // ================= 使用缓存的网络状态 =================
                // get_or_insert_with 会在值为 None 时执行闭包去检测网络，
                // 如果已经检测过了（变成了 Some(true/false)），就直接复用结果。
                let is_connected =
                    *current_network_status.get_or_insert_with(|| is_network_connected());

                if !is_connected {
                    log::warn!("[{}] 网络未连接，暂不重启，等待网络恢复...", identifier);
                    // 不累加尝试次数，直接跳过当前进程的重启逻辑
                    continue;
                }
                // ============================================================

                log::warn!("检测到 frpc 进程 [{}] 已退出，准备尝试重启。", identifier);

                let attempts = restart_attempts.entry(identifier.clone()).or_insert(0);
                *attempts += 1;

                if *attempts > MAX_RESTART_ATTEMPTS {
                    log::error!(
                        "frpc 进程 [{}] 重启次数已达上限 ({}/{})，将放弃重启此进程。",
                        identifier,
                        *attempts - 1,
                        MAX_RESTART_ATTEMPTS
                    );
                    abandoned.insert(identifier);
                    continue;
                }

                log::info!("尝试第 {} 次重启 frpc 进程 [{}]...", *attempts, identifier);

                // 使用存储的路径和标识符尝试重启
                match FrpcProcess::start(
                    process.identifier.clone(),
                    process.exe_path.clone(),
                    process.config_path.clone(),
                ) {
                    Ok(new_process) => {
                        frpc_processes[i] = new_process;
                        log::info!("[{}] frpc 进程重启成功。", identifier);
                    }
                    Err(e) => {
                        log::error!("[{}] frpc 进程重启失败: {:?}", identifier, e);
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
