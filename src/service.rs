//! Windows 服务管理：注册/注销/状态检查 + 服务调度器
//!

use anyhow::{Context, Result};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// 服务停止信号，由 SCM 停止事件设置
static SERVICE_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
use windows_service::service::{
    ServiceAccess, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo,
    ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::config;
use crate::frpc_mg::FrpcProcess;

pub const SERVICE_NAME: &str = "FrpcService";
pub const DISPLAY_NAME: &str = "FRP Client Service";
pub const SERVICE_ARG: &str = "--service";

// =========================================================================
//  交互模式入口
// =========================================================================

/// 服务预检查结果
#[derive(Clone, Debug)]
pub(crate) enum PreCheckResult {
    Running,
    Stopped,
    NotRegistered,
}

/// 检查服务状态并启动 GUI
pub fn check_and_run_app() -> Result<()> {
    let pre_check = check_service_status()?;
    crate::app::run_app(pre_check);
    Ok(())
}

/// 检查 Windows 服务当前状态
pub(crate) fn check_service_status() -> Result<PreCheckResult> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    if let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        let status = service.query_status()?;
        match status.current_state {
            ServiceState::Running => Ok(PreCheckResult::Running),
            ServiceState::Stopped => Ok(PreCheckResult::Stopped),
            _ => Err(anyhow::anyhow!(
                "服务处于非预期状态：{:?}",
                status.current_state
            )),
        }
    } else {
        Ok(PreCheckResult::NotRegistered)
    }
}

// =========================================================================
//  服务注册 / 注销
// =========================================================================

/// 注册 Windows 服务（如果已存在则先删除再重建）
pub(crate) fn install_service() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::all())?;

    // 如果服务已存在，先停止并删除
    if let Ok(service) = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        log::info!("服务 {} 已存在，尝试删除旧服务", SERVICE_NAME);
        stop_service_if_running(&service)?;
        service.delete().context("无法删除旧服务")?;
        std::thread::sleep(Duration::from_millis(500));
    }

    // 创建新服务
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;
    let exe_path = env::current_exe().context("无法获取当前可执行文件路径")?;
    manager
        .create_service(
            &ServiceInfo {
                name: OsString::from(SERVICE_NAME),
                display_name: OsString::from(DISPLAY_NAME),
                service_type: ServiceType::OWN_PROCESS,
                start_type: ServiceStartType::AutoStart,
                error_control: ServiceErrorControl::Normal,
                executable_path: PathBuf::from(&exe_path),
                launch_arguments: vec![OsString::from(SERVICE_ARG)],
                dependencies: vec![],
                account_name: None,
                account_password: None,
            },
            ServiceAccess::all(),
        )
        .context("创建服务失败，请确保以管理员身份运行")?;
    log::info!("服务 {} 已成功注册", SERVICE_NAME);
    Ok(())
}

/// 注销 Windows 服务（先停止服务、清理残留 frpc 进程，再删除）
pub(crate) fn uninstall_service() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::all())?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    )?;
    stop_service_if_running(&service)?;
    service.delete().context("无法删除服务")?;
    log::info!("服务 {} 已删除", SERVICE_NAME);
    Ok(())
}

/// 启动一个 frpc 配置进程（无连接回调）
#[allow(dead_code)]
pub fn start_frpc_process(name: &str) -> Result<FrpcProcess> {
    start_frpc_process_with_sender(name, None)
}

/// 启动一个 frpc 配置进程，可传入连接成功回调
pub fn start_frpc_process_with_sender(
    name: &str,
    on_connected: Option<Sender<()>>,
) -> Result<FrpcProcess> {
    let exe_path = config::frpc_exe_path().context("无法获取 frpc.exe 路径")?;
    let config_path = config::config_toml_path(name).context("无法获取配置文件路径")?;
    FrpcProcess::start(name.to_string(), exe_path, config_path, on_connected)
}

// =========================================================================
//  内部辅助
// =========================================================================

/// 如果服务正在运行则停止它
fn stop_service_if_running(service: &windows_service::service::Service) -> Result<()> {
    let status = service.query_status()?;
    if status.current_state == ServiceState::Stopped {
        return Ok(());
    }
    service.stop().context("无法停止服务")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let status = service.query_status()?;
        if status.current_state == ServiceState::Stopped {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err(anyhow::anyhow!("服务停止超时"));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

// =========================================================================
//  服务模式（由 SCM 启动）
// =========================================================================

extern "system" fn service_main(_arguments: u32, _argv: *mut *mut u16) {
    if let Err(e) = run_service() {
        log::error!("服务运行失败: {:?}", e);
    }
}

pub fn run_service_dispatcher() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, service_main)?;
    Ok(())
}

fn run_service() -> Result<()> {
    SERVICE_STOP_REQUESTED.store(false, Ordering::SeqCst);
    let status_handle =
        service_control_handler::register(SERVICE_NAME, |control_event| match control_event {
            windows_service::service::ServiceControl::Stop
            | windows_service::service::ServiceControl::Shutdown => {
                SERVICE_STOP_REQUESTED.store(true, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .context("无法注册服务控制处理程序")?;
    set_service_status(&status_handle, ServiceState::StartPending)?;

    let settings = config::load_settings();

    // 启动所有自启动配置
    let instances = discover_auto_start_instances()?;
    let mut processes: Vec<(String, FrpcProcess)> = Vec::new();
    for (id, exe, conf) in instances {
        match FrpcProcess::start(id.clone(), exe, conf, None) {
            Ok(p) => {
                log::info!("[{}] frpc 进程已启动", id);
                processes.push((id, p));
            }
            Err(e) => log::error!("启动 frpc 实例失败: {:?}", e),
        }
    }

    if processes.is_empty() {
        log::warn!("没有任何 frpc 进程成功启动");
    } else {
        log::info!("成功启动 {} 个 frpc 实例", processes.len());
    }

    if !settings.process_guard {
        // 未开启进程守护，服务直接退出，frpc 进程独立运行
        set_service_status(&status_handle, ServiceState::Stopped)?;
        return Ok(());
    }

    // 开启进程守护：保持服务运行，监控进程状态
    log::info!("进程守护已开启，服务将持续运行");
    set_service_status(&status_handle, ServiceState::Running)?;

    // 重新发现自启动配置，用于进程退出后重启
    let auto_start_map = discover_auto_start_map();

    let mut loop_count: u32 = 0;
    let mut guard_stopped: Vec<String> = Vec::new();

    loop {
        // 检查是否收到停止信号
        if SERVICE_STOP_REQUESTED.load(Ordering::SeqCst) {
            log::info!("收到服务停止信号");
            set_service_status(&status_handle, ServiceState::Stopped)?;
            return Ok(());
        }

        std::thread::sleep(std::time::Duration::from_secs(1));
        loop_count = (loop_count + 1) % 3;

        // 每 3 秒读取一次 UI 手动停止列表
        if loop_count == 0 {
            guard_stopped = config::load_guard_stopped();
        }

        // 检查是否有进程退出
        let mut restart_list = Vec::new();
        processes.retain(|(name, proc)| {
            if FrpcProcess::is_pid_running(proc.pid()) {
                true
            } else {
                if guard_stopped.contains(name) {
                    log::info!("[{}] 进程已退出（UI 手动停止，不重启）", name);
                } else {
                    log::warn!("[{}] 进程守护发现进程已退出，将重启", name);
                    restart_list.push(name.clone());
                }
                false
            }
        });

        // 重启退出的进程
        for name in restart_list {
            if let Some((exe, conf)) = auto_start_map.get(&name) {
                match FrpcProcess::start(name.clone(), exe.clone(), conf.clone(), None) {
                    Ok(p) => {
                        log::info!("[{}] 进程守护重启成功", name);
                        processes.push((name, p));
                    }
                    Err(e) => log::error!("[{}] 进程守护重启失败: {:?}", name, e),
                }
            }
        }
    }
}

fn set_service_status(
    handle: &windows_service::service_control_handler::ServiceStatusHandle,
    state: ServiceState,
) -> Result<()> {
    let mut controls = ServiceControlAccept::empty();
    if state == ServiceState::Running {
        controls = ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN;
    }
    handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: controls,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    })?;
    Ok(())
}

fn discover_auto_start_instances() -> Result<Vec<(String, PathBuf, PathBuf)>> {
    let frpc_exe = config::frpc_exe_path().context("无法获取 frpc.exe 路径")?;
    if !frpc_exe.exists() {
        return Ok(Vec::new());
    }
    let mut instances = Vec::new();
    for meta in config::get_auto_start_configs().unwrap_or_default() {
        let conf = config::config_toml_path(&meta.name)?;
        if conf.exists() {
            instances.push((meta.name.clone(), frpc_exe.clone(), conf));
        }
    }
    Ok(instances)
}

/// 发现自启动配置，返回 name -> (exe, conf) 的映射
fn discover_auto_start_map() -> std::collections::HashMap<String, (PathBuf, PathBuf)> {
    let mut map = std::collections::HashMap::new();
    let frpc_exe = match config::frpc_exe_path() {
        Ok(p) if p.exists() => p,
        _ => return map,
    };
    for meta in config::get_auto_start_configs().unwrap_or_default() {
        if let Ok(conf) = config::config_toml_path(&meta.name) {
            if conf.exists() {
                map.insert(meta.name.clone(), (frpc_exe.clone(), conf));
            }
        }
    }
    map
}

/// 发现当前正在运行的 frpc 进程，匹配到已有配置
///
/// 返回 (配置名, PID) 的列表
pub fn discover_running_frpc_processes() -> Vec<(String, u32)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // 获取 frpc.exe 路径
    let frpc_exe = match config::frpc_exe_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    if !frpc_exe.exists() {
        return Vec::new();
    }

    // 使用 wmic 获取 frpc.exe 进程的命令行
    let output = match std::process::Command::new("wmic")
        .args([
            "process",
            "where",
            "name='frpc.exe'",
            "get",
            "ProcessId,CommandLine",
            "/FORMAT:CSV",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("wmic 执行失败: {}", e);
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let configs = config::load_configs().unwrap_or_default();
    let conf_dir = config::conf_dir().unwrap_or_default();
    let mut result = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Node") || line.starts_with("CommandLine") {
            continue;
        }
        // CSV 格式: Node,CommandLine,ProcessId
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 3 {
            continue;
        }
        let cmd_line = parts[1];
        let pid_str = parts[2].trim();

        // 解析 PID
        let pid: u32 = match pid_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // 从命令行中提取配置文件名
        // 命令行格式: frpc.exe -c <config_path>
        for config_meta in &configs {
            let config_path = conf_dir.join(format!("{}.toml", config_meta.name));
            let config_path_str = config_path.to_string_lossy();
            if cmd_line.contains(&*config_path_str) {
                result.push((config_meta.name.clone(), pid));
                log::info!(
                    "发现运行中的 frpc 进程: {} (PID: {})",
                    config_meta.name,
                    pid
                );
                break;
            }
        }
    }

    result
}
