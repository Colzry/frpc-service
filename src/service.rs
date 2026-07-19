//! Windows 服务管理：注册/注销/状态检查 + 服务调度器
//!

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    InitializeSecurityDescriptor, SetSecurityDescriptorDacl, SECURITY_ATTRIBUTES,
    SECURITY_DESCRIPTOR,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, OpenEventW, SetEvent, WaitForMultipleObjects,
};

/// 服务停止信号，由 SCM 停止事件设置
static SERVICE_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

// Event access constants
const EVENT_MODIFY_STATE: u32 = 0x0002;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;

/// "Global\FrpcGuardChanged" as UTF-16 with null terminator
fn guard_event_name() -> Vec<u16> {
    "Global\\FrpcGuardChanged\0".encode_utf16().collect()
}

/// "Global\FrpcGuardStoppedChanged" as UTF-16 with null terminator
fn guard_stopped_event_name() -> Vec<u16> {
    "Global\\FrpcGuardStoppedChanged\0".encode_utf16().collect()
}

/// Create a named event with NULL DACL (allows cross-session access)
///
/// The service runs as SYSTEM (session 0) and the UI runs as the user (session 1+).
/// A NULL DACL grants everyone access to the named event.
fn create_named_event(name: &[u16], desc: &str) -> Result<HANDLE> {
    let mut sd: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };

    unsafe {
        if InitializeSecurityDescriptor(&mut sd as *mut _ as *mut _, 1) == 0 {
            return Err(anyhow::anyhow!("无法初始化安全描述符"));
        }
        // NULL DACL = everyone has access
        if SetSecurityDescriptorDacl(&mut sd as *mut _ as *mut _, 1, std::ptr::null(), 0) == 0 {
            return Err(anyhow::anyhow!("无法设置安全描述符 DACL"));
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: &mut sd as *mut _ as *mut _,
            bInheritHandle: 0,
        };
        let event = CreateEventW(&sa, 0, 0, name.as_ptr());
        if event == 0 {
            return Err(anyhow::anyhow!("无法创建{}事件", desc));
        }
        Ok(event)
    }
}

/// Signal a named event by name. Does nothing if the event doesn't exist.
fn signal_named_event(name: &[u16]) {
    unsafe {
        let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if event != 0 {
            SetEvent(event);
            CloseHandle(event);
        }
    }
}

/// Signal the guard change event to wake up the service.
/// Called by the UI after toggling process guard.
pub fn signal_guard_changed() {
    signal_named_event(&guard_event_name());
}

/// Signal the guard stopped list change event to wake up the service.
/// Called by the UI after modifying guard_stopped.json.
pub fn signal_guard_stopped_changed() {
    signal_named_event(&guard_stopped_event_name());
}
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

    // 立即启动服务
    start_service()?;

    Ok(())
}

/// 注销 Windows 服务（先停止再删除）
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

/// 启动 Windows 服务
pub(crate) fn start_service() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::START | ServiceAccess::QUERY_STATUS,
    )?;
    let status = service.query_status()?;
    if status.current_state == ServiceState::Running {
        log::info!("服务 {} 已在运行", SERVICE_NAME);
        return Ok(());
    }
    service.start(&[] as &[&str]).context("无法启动服务")?;
    log::info!("服务 {} 已启动", SERVICE_NAME);
    Ok(())
}

/// 停止 Windows 服务
#[allow(dead_code)]
pub(crate) fn stop_service() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
    )?;
    stop_service_if_running(&service)
}

/// 重启 Windows 服务（先停止再启动，不影响已运行的 frpc 进程）
#[allow(dead_code)]
pub(crate) fn restart_service() -> Result<()> {
    stop_service()?;
    std::thread::sleep(Duration::from_millis(500));
    start_service()
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

    let mut settings = config::load_settings();

    // 启动所有自启动配置（跳过已运行的）
    let running_frpc = discover_running_frpc_processes();
    let instances = discover_auto_start_instances()?;
    let mut processes: Vec<(String, FrpcProcess)> = Vec::new();
    for (id, exe, conf) in instances {
        if let Some((_, pid)) = running_frpc.iter().find(|(n, _)| n == &id) {
            // 已有进程在运行，直接跟踪
            if FrpcProcess::is_pid_running(*pid) {
                let process = FrpcProcess::from_pid(*pid, id.clone(), exe, conf);
                log::info!("[{}] 检测到已运行的进程 (PID: {})", id, pid);
                processes.push((id, process));
                continue;
            }
        }
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

    // 服务始终运行，进程守护由设置动态控制
    log::info!("服务已启动，进程守护: {}", settings.process_guard);
    set_service_status(&status_handle, ServiceState::Running)?;

    let auto_start_map = discover_auto_start_map();

    // 创建跨进程命名事件，UI 可通过信号通知服务
    let guard_event = create_named_event(&guard_event_name(), "进程守护")?;
    let guard_stopped_event = create_named_event(&guard_stopped_event_name(), "手动停止列表")?;

    // 维护内存中的手动停止列表，避免每次检测进程退出时读取文件
    let mut guard_stopped: HashSet<String> = config::load_guard_stopped().into_iter().collect();

    // 事件句柄数组，用于 WaitForMultipleObjects
    let handles = [guard_event, guard_stopped_event];

    loop {
        if SERVICE_STOP_REQUESTED.load(Ordering::SeqCst) {
            log::info!("收到服务停止信号");
            unsafe {
                CloseHandle(guard_event);
                CloseHandle(guard_stopped_event);
            }
            set_service_status(&status_handle, ServiceState::Stopped)?;
            return Ok(());
        }

        // 使用命名事件等待 1 秒，替代 thread::sleep
        // - WAIT_OBJECT_0: guard_event 信号化（进程守护开关切换）
        // - WAIT_OBJECT_0+1: guard_stopped_event 信号化（手动停止列表变更）
        // - WAIT_TIMEOUT: 超时，继续检查进程状态
        let wait_result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, 1000) };
        match wait_result {
            WAIT_OBJECT_0 => {
                // 进程守护开关切换，直接翻转即可，无需读取文件
                settings.process_guard = !settings.process_guard;
                log::info!(
                    "收到进程守护变更信号，process_guard={}",
                    settings.process_guard
                );
            }
            1 => {
                // 手动停止列表变更，重新读取文件更新内存缓存
                guard_stopped = config::load_guard_stopped().into_iter().collect();
                log::info!("收到手动停止列表变更信号，已更新缓存");
            }
            WAIT_TIMEOUT => {} // 超时，继续检查进程状态
            _ => {
                log::error!("WaitForMultipleObjects 返回未知状态: {}", wait_result);
            }
        }

        // 进程守护未开启时，不监控不重启
        if !settings.process_guard {
            continue;
        }

        // 进程守护开启：检查是否有进程退出并重启
        // 直接查询内存中的 guard_stopped HashSet，无需读取文件
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

        for name in restart_list {
            // 再次检查内存缓存，防止 UI 在 retain 与 restart 之间发送了信号
            if guard_stopped.contains(&name) {
                log::info!("[{}] 已在手动停止列表中，跳过重启", name);
                continue;
            }
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
/// 返回 (配置名, PID) 的列表。优先使用 wmic（快速），失败则回退到 PowerShell。
pub fn discover_running_frpc_processes() -> Vec<(String, u32)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let frpc_exe = match config::frpc_exe_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    if !frpc_exe.exists() {
        return Vec::new();
    }

    let configs = config::load_configs().unwrap_or_default();
    if configs.is_empty() {
        return Vec::new();
    }
    let conf_dir = config::conf_dir().unwrap_or_default();

    // 尝试 wmic（快速），失败或无输出则回退到 PowerShell
    let stdout = match std::process::Command::new("wmic")
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
        Ok(o) if !o.stdout.is_empty() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => {
            log::info!("wmic 不可用或无输出，尝试 PowerShell");
            match std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command",
                    "Get-CimInstance Win32_Process -Filter \"Name='frpc.exe'\" | Select-Object ProcessId,CommandLine | ConvertTo-CSV -NoTypeInformation"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
                _ => {
                    log::warn!("PowerShell 也无法获取进程信息");
                    return Vec::new();
                }
            }
        }
    };

    if stdout.trim().is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let fields = parse_csv_line(line);

        // 跳过标题行
        if fields
            .iter()
            .any(|f| f == "ProcessId" || f == "CommandLine" || f == "Node")
        {
            continue;
        }

        // 解析 PID 和命令行（wmic: Node,CommandLine,ProcessId；PowerShell: ProcessId,CommandLine）
        let (pid, cmd_line) = if fields.len() >= 3 {
            match fields[2].trim().parse::<u32>() {
                Ok(p) => (p, fields[1].as_str()),
                Err(_) => continue,
            }
        } else if fields.len() == 2 {
            if let Ok(p) = fields[0].trim().parse::<u32>() {
                (p, fields[1].as_str())
            } else if let Ok(p) = fields[1].trim().parse::<u32>() {
                (p, fields[0].as_str())
            } else {
                continue;
            }
        } else {
            continue;
        };

        // 匹配配置
        for config_meta in &configs {
            let config_path = conf_dir.join(format!("{}.toml", config_meta.name));
            let config_path_str = config_path.to_string_lossy();
            if cmd_line.contains(&*config_path_str) {
                result.push((config_meta.name.clone(), pid));
                break;
            }
        }
    }

    result
}

/// CSV 行解析，支持引号包裹的字段
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    fields.push(current.trim().to_string());
    fields
}
