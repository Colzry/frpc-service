//! 主应用视图：AppView 结构体、事件处理、run_app 入口

use anyhow::Result;
use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, SharedString, Task, TitlebarOptions,
    Window, WindowBounds, WindowOptions,
};
use gpui_component::input::InputState;
use gpui_component::select::{SelectEvent, SelectState};
use gpui_component::{ActiveTheme, IndexPath, Root};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::{self, FrpcConfigMeta};
use crate::download;
use crate::frpc_mg::FrpcProcess;
use crate::message::MessageLevel;
use crate::pages;
use crate::service::{self, PreCheckResult};
use crate::sidebar;
use crate::theme;

/// 自定义暗色主题 JSON
/// 当前页面
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Page {
    ConfigList,
    ConfigEditor { original_name: Option<String> },
    Settings,
}

/// 运行中的进程信息
pub(crate) struct RunningProcess {
    pub process: FrpcProcess,
}

/// 主界面视图
pub(crate) struct AppView {
    pub page: Page,
    pub service_registered: bool,
    pub configs: Vec<FrpcConfigMeta>,
    pub running: HashMap<String, RunningProcess>,
    pub stopped_configs: std::collections::HashSet<String>, // 手动停止的配置，防止被健康检查重新拉起
    pub edit_name: String,
    pub edit_content: String,
    pub edit_auto_start: bool,
    pub name_input: Entity<InputState>,
    pub content_input: Entity<InputState>,
    pub frpc_version: Option<String>,
    pub is_checking_update: bool,
    pub is_downloading: bool,
    pub download_percent: u64,
    pub is_processing: bool,
    pub status_message: Option<String>,
    pub status_level: MessageLevel,
    pub config_page: usize,
    pub theme_select: Entity<SelectState<Vec<SharedString>>>,
    pub process_guard: bool,
}

impl AppView {
    pub fn new(
        pre_check: PreCheckResult,
        name_input: Entity<InputState>,
        content_input: Entity<InputState>,
        theme_select: Entity<SelectState<Vec<SharedString>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let configs = config::load_configs().unwrap_or_default();
        let service_registered = !matches!(pre_check, PreCheckResult::NotRegistered);

        // 恢复上次运行的 frpc 进程状态
        let mut running = HashMap::new();
        let frpc_exe = config::frpc_exe_path().ok().filter(|p| p.exists());
        if let Some(exe_path) = frpc_exe {
            for (name, pid) in service::discover_running_frpc_processes() {
                if FrpcProcess::is_pid_running(pid) {
                    let config_path = config::config_toml_path(&name).unwrap_or_default();
                    let process =
                        FrpcProcess::from_pid(pid, name.clone(), exe_path.clone(), config_path);
                    running.insert(name.clone(), RunningProcess { process });
                    log::info!("恢复 frpc 进程状态: {} (PID: {})", name, pid);
                }
            }
        }

        let s = Self {
            page: Page::ConfigList,
            service_registered,
            configs,
            running,
            stopped_configs: config::load_guard_stopped().into_iter().collect(),
            edit_name: String::new(),
            edit_content: String::new(),
            edit_auto_start: false,
            name_input,
            content_input,
            frpc_version: None,
            is_checking_update: false,
            is_downloading: false,
            download_percent: 0,
            is_processing: false,
            status_message: None,
            status_level: MessageLevel::Info,
            config_page: 0,
            theme_select: theme_select.clone(),
            process_guard: config::load_settings().process_guard,
        };

        // 订阅主题下拉选择事件
        cx.subscribe_in(&theme_select, window, |view, _entity, event, window, cx| {
            view.on_theme_selected(event, window, cx);
        })
        .detach();

        s
    }

    pub fn switch_page(&mut self, page: Page, _cx: &mut Context<Self>) {
        self.page = page;
        self.status_message = None;
        self.config_page = 0;
    }

    pub fn toggle_process_guard(&mut self, cx: &mut Context<Self>) {
        self.process_guard = !self.process_guard;
        let settings = config::AppSettings {
            process_guard: self.process_guard,
        };
        match config::save_settings(&settings) {
            Ok(()) => {
                let msg = if self.process_guard {
                    "进程守护已开启".to_string()
                } else {
                    "进程守护已关闭".to_string()
                };
                log::info!("进程守护设置已变更: {}", self.process_guard);
                self.set_status_message(msg, MessageLevel::Success, cx);
            }
            Err(e) => {
                log::error!("保存进程守护设置失败: {}", e);
                self.set_status_message(format!("保存设置失败: {}", e), MessageLevel::Error, cx);
            }
        }
        cx.notify();
        cx.notify();
    }

    pub fn on_theme_selected(
        &mut self,
        event: &SelectEvent<Vec<SharedString>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(name)) = event {
            let name = name.to_string();
            theme::apply_theme(&name, cx);
            theme::save_theme_preference(&name);
            self.set_status_message(
                format!("主题已切换为 '{}'", name),
                MessageLevel::Success,
                cx,
            );
        }
    }

    pub fn switch_page_with_message(
        &mut self,
        page: Page,
        msg: String,
        level: MessageLevel,
        cx: &mut Context<Self>,
    ) {
        self.page = page;
        self.status_message = Some(msg);
        self.status_level = level;
        self.config_page = 0;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async { std::thread::sleep(std::time::Duration::from_secs(3)) })
                .await;
            this.update(cx, |v, cx| {
                v.status_message = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn set_status_message(&mut self, msg: String, level: MessageLevel, cx: &mut Context<Self>) {
        self.status_message = Some(msg);
        self.status_level = level;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async { std::thread::sleep(std::time::Duration::from_secs(3)) })
                .await;
            this.update(cx, |v, cx| {
                v.status_message = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn reload_configs(&mut self, cx: &mut Context<Self>) {
        self.configs = config::load_configs().unwrap_or_default();
        let total_pages = (self.configs.len() + 7) / 8;
        if self.config_page > 0 && self.config_page >= total_pages {
            self.config_page = total_pages.saturating_sub(1);
        }
        cx.notify();
    }

    pub fn open_add_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_name = String::new();
        self.edit_content = String::new();
        self.edit_auto_start = true;
        self.status_message = None;
        self.name_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.content_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.switch_page(
            Page::ConfigEditor {
                original_name: None,
            },
            cx,
        );
    }

    pub fn open_edit_config(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_name = name.to_string();
        self.edit_content = config::read_config_content(name).unwrap_or_default();
        self.edit_auto_start = self
            .configs
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.auto_start)
            .unwrap_or(false);
        self.status_message = None;
        self.name_input
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.content_input.update(cx, |s, cx| {
            s.set_value(&self.edit_content.clone(), window, cx)
        });
        self.switch_page(
            Page::ConfigEditor {
                original_name: Some(name.to_string()),
            },
            cx,
        );
    }

    pub fn save_config(&mut self, cx: &mut Context<Self>) {
        self.edit_name = self.name_input.read(cx).value().to_string();
        self.edit_content = self.content_input.read(cx).value().to_string();
        let name = self.edit_name.trim().to_string();
        if name.is_empty() {
            self.set_status_message("配置名称不能为空".to_string(), MessageLevel::Error, cx);
            return;
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            self.set_status_message(
                "配置名称只能包含字母、数字、下划线和连字符".to_string(),
                MessageLevel::Error,
                cx,
            );
            return;
        }
        if self.edit_content.trim().is_empty() {
            self.set_status_message("配置内容不能为空".to_string(), MessageLevel::Error, cx);
            return;
        }
        if let Page::ConfigEditor {
            original_name: ref orig,
        } = self.page
        {
            if orig.is_none() && config::config_exists(&name) {
                self.set_status_message(format!("配置 '{}' 已存在", name), MessageLevel::Error, cx);
                return;
            }
        }
        if let Page::ConfigEditor {
            original_name: Some(ref orig),
        } = self.page
        {
            if orig != &name {
                let _ = config::delete_config(orig);
            }
        }
        match config::save_config(&name, &self.edit_content, self.edit_auto_start) {
            Ok(()) => {
                self.reload_configs(cx);
                self.switch_page_with_message(
                    Page::ConfigList,
                    format!("配置 '{}' 保存成功", name),
                    MessageLevel::Success,
                    cx,
                );
            }
            Err(e) => {
                self.set_status_message(format!("保存失败：{}", e), MessageLevel::Error, cx);
            }
        }
    }

    pub fn delete_config(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(mut rp) = self.running.remove(name) {
            let _ = rp.process.stop();
        }
        match config::delete_config(name) {
            Ok(()) => {
                log::info!("配置 '{}' 已删除", name);
                self.reload_configs(cx);
                self.set_status_message(
                    format!("配置 '{}' 已删除", name),
                    MessageLevel::Success,
                    cx,
                );
            }
            Err(e) => {
                log::error!("删除配置 '{}' 失败: {}", name, e);
                self.set_status_message(format!("删除失败：{}", e), MessageLevel::Error, cx);
            }
        }
    }

    pub fn start_config(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.running.contains_key(name) {
            return;
        }
        // 从手动停止列表中移除，允许健康检查监控
        self.stopped_configs.remove(name);
        // 更新共享文件
        let _ =
            config::save_guard_stopped(&self.stopped_configs.iter().cloned().collect::<Vec<_>>());
        // 检查 frpc.exe 是否存在
        if !crate::download::has_frpc_executable(
            &std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
        ) {
            self.set_status_message(
                "请先在设置中下载 frpc 程序".to_string(),
                MessageLevel::Warning,
                cx,
            );
            return;
        }
        let n = name.to_string();
        self.is_processing = true;
        self.status_message = None;
        cx.notify();

        // 创建通道用于检测 frpc 连接成功
        let (tx, rx) = std::sync::mpsc::channel();

        let task: Task<Result<FrpcProcess>> = cx
            .background_spawn(async move { service::start_frpc_process_with_sender(&n, Some(tx)) });
        let nc = name.to_string();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.is_processing = false;
                match result {
                    Ok(p) => {
                        log::info!("[{}] frpc 进程已启动", nc);
                        view.running
                            .insert(nc.clone(), RunningProcess { process: p });
                        cx.notify();

                        // 启动后台任务监听连接成功
                        let name_for_toast = nc.clone();
                        cx.spawn(async move |this, cx| {
                            // 在后台线程等待连接成功信号
                            let connected = cx
                                .background_spawn(async move {
                                    rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok()
                                })
                                .await;
                            if connected {
                                this.update(cx, |view, cx| {
                                    if view.running.contains_key(&name_for_toast) {
                                        view.set_status_message(
                                            format!("'{}' 连接成功", name_for_toast),
                                            MessageLevel::Success,
                                            cx,
                                        );
                                    }
                                })
                                .ok();
                            }
                        })
                        .detach();

                        // 500ms 后检查进程是否立即退出（如配置解析错误）
                        let name_check = nc.clone();
                        cx.spawn(async move |this, cx| {
                            cx.background_spawn(async {
                                std::thread::sleep(Duration::from_millis(500));
                            })
                            .await;
                            this.update(cx, |view, cx| {
                                if let Some(rp) = view.running.get_mut(&name_check) {
                                    if let Some(status) = rp.process.check_exit_status() {
                                        log::error!(
                                            "[{}] frpc 启动后立即退出，退出码: {}",
                                            name_check,
                                            status
                                        );
                                        view.running.remove(&name_check);
                                        view.set_status_message(
                                            format!(
                                                "'{}' 启动失败，请检查配置是否正确 (退出码: {})",
                                                name_check, status
                                            ),
                                            MessageLevel::Error,
                                            cx,
                                        );
                                    }
                                }
                            })
                            .ok();
                        })
                        .detach();
                    }
                    Err(e) => {
                        log::error!("[{}] 启动失败: {}", nc, e);
                        view.set_status_message(
                            format!("启动失败：{}", e),
                            MessageLevel::Error,
                            cx,
                        );
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn stop_config(&mut self, name: &str, cx: &mut Context<Self>) {
        // 标记为手动停止，防止健康检查重新发现
        self.stopped_configs.insert(name.to_string());
        // 写入共享文件，通知服务守护不要重启
        let _ =
            config::save_guard_stopped(&self.stopped_configs.iter().cloned().collect::<Vec<_>>());
        if let Some(mut rp) = self.running.remove(name) {
            self.is_processing = true;
            cx.notify();
            let task: Task<Result<()>> = cx.background_spawn(async move {
                rp.process.stop()?;
                Ok(())
            });
            let nc = name.to_string();
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |view, cx| {
                    view.is_processing = false;
                    match result {
                        Ok(()) => {
                            log::info!("[{}] frpc 已停止", nc);
                            view.set_status_message(
                                format!("'{}'已停止", nc),
                                MessageLevel::Success,
                                cx,
                            );
                        }
                        Err(e) => {
                            log::error!("[{}] 停止失败: {}", nc, e);
                            view.set_status_message(
                                format!("停止失败：{}", e),
                                MessageLevel::Error,
                                cx,
                            );
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    pub fn restart_config(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(mut rp) = self.running.remove(name) {
            log::info!("[{}] 正在重启，先停止当前进程", name);
            let _ = rp.process.stop();
        }
        // 检查 frpc.exe 是否存在
        if !crate::download::has_frpc_executable(
            &std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
        ) {
            self.set_status_message(
                "请先在设置中下载 frpc 程序".to_string(),
                MessageLevel::Warning,
                cx,
            );
            return;
        }
        let n = name.to_string();
        self.is_processing = true;
        self.status_message = None;
        cx.notify();

        // 创建通道用于检测 frpc 连接成功
        let (tx, rx) = std::sync::mpsc::channel();

        let task: Task<Result<FrpcProcess>> = cx
            .background_spawn(async move { service::start_frpc_process_with_sender(&n, Some(tx)) });
        let nc = name.to_string();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.is_processing = false;
                match result {
                    Ok(p) => {
                        log::info!("[{}] frpc 重启成功", nc);
                        view.running
                            .insert(nc.clone(), RunningProcess { process: p });
                        view.set_status_message(format!("'{}'已重启", nc), MessageLevel::Info, cx);
                        cx.notify();

                        // 启动后台任务监听连接成功
                        let name_for_toast = nc.clone();
                        cx.spawn(async move |this, cx| {
                            let connected = cx
                                .background_spawn(async move {
                                    rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok()
                                })
                                .await;
                            if connected {
                                this.update(cx, |view, cx| {
                                    if view.running.contains_key(&name_for_toast) {
                                        view.set_status_message(
                                            format!("'{}' 连接成功", name_for_toast),
                                            MessageLevel::Success,
                                            cx,
                                        );
                                    }
                                })
                                .ok();
                            }
                        })
                        .detach();

                        // 500ms 后检查进程是否立即退出（如配置解析错误）
                        let name_check = nc.clone();
                        cx.spawn(async move |this, cx| {
                            cx.background_spawn(async {
                                std::thread::sleep(Duration::from_millis(500));
                            })
                            .await;
                            this.update(cx, |view, cx| {
                                if let Some(rp) = view.running.get_mut(&name_check) {
                                    if let Some(status) = rp.process.check_exit_status() {
                                        log::error!(
                                            "[{}] 重启后立即退出，退出码: {}",
                                            name_check,
                                            status
                                        );
                                        view.running.remove(&name_check);
                                        view.set_status_message(
                                            format!(
                                                "'{}' 重启失败，请检查配置是否正确 (退出码: {})",
                                                name_check, status
                                            ),
                                            MessageLevel::Error,
                                            cx,
                                        );
                                    }
                                }
                            })
                            .ok();
                        })
                        .detach();
                    }
                    Err(e) => {
                        log::error!("[{}] 重启失败: {}", nc, e);
                        view.set_status_message(
                            format!("重启失败：{}", e),
                            MessageLevel::Error,
                            cx,
                        );
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn start_download(&mut self, cx: &mut Context<Self>) {
        self.is_checking_update = true;
        self.is_downloading = false;
        self.download_percent = 0;
        self.status_message = None;
        cx.notify();

        // 第一步：在后台检查版本
        let check_task: Task<Result<Option<String>>> =
            cx.background_spawn(async move { download::check_update() });

        cx.spawn(async move |this, cx| {
            let result = check_task.await;
            let should_download = this
                .update(cx, |view, cx| {
                    view.is_checking_update = false;
                    match result {
                        Ok(Some(tag)) => {
                            log::info!("发现新版本: {}，开始下载", tag);
                            view.is_downloading = true;
                            cx.notify();
                            true
                        }
                        Ok(None) => {
                            view.set_status_message(
                                "已经是最新版本".to_string(),
                                MessageLevel::Success,
                                cx,
                            );
                            false
                        }
                        Err(e) => {
                            log::error!("检查版本失败: {}", e);
                            view.set_status_message(
                                format!("检查版本失败：{}", e),
                                MessageLevel::Error,
                                cx,
                            );
                            false
                        }
                    }
                })
                .unwrap_or(false);

            if !should_download {
                return;
            }

            // 第二步：启动下载
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let progress = Arc::new(AtomicU64::new(0));
            let pc = progress.clone();

            // 启动进度更新循环
            this.update(cx, |_, cx| {
                cx.spawn(async move |this, cx| loop {
                    cx.background_spawn(async {
                        std::thread::sleep(Duration::from_millis(200));
                    })
                    .await;
                    let ok = this
                        .update(cx, |v, cx| {
                            if v.is_downloading {
                                v.download_percent = pc.load(Ordering::Relaxed);
                                cx.notify();
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !ok {
                        break;
                    }
                })
                .detach();
            })
            .ok();

            // 在后台执行下载
            let download_result = cx
                .background_spawn(async move {
                    download::download_and_extract_frpc(&exe_dir, &move |d, t| {
                        progress.store(
                            if t > 0 { (d * 100 / t).min(100) } else { 0 },
                            Ordering::Relaxed,
                        );
                    })
                })
                .await;

            // 下载完成，更新 UI
            this.update(cx, |view, cx| {
                view.is_downloading = false;
                match download_result {
                    Ok(()) => {
                        view.set_status_message(
                            "下载成功！".to_string(),
                            MessageLevel::Success,
                            cx,
                        );
                        view.detect_frpc_version(cx);
                    }
                    Err(e) => {
                        view.set_status_message(
                            format!("下载失败：{}", e),
                            MessageLevel::Error,
                            cx,
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn detect_frpc_version(&mut self, cx: &mut Context<Self>) {
        let exe_path = match config::frpc_exe_path().ok().filter(|p| p.exists()) {
            Some(p) => p,
            None => {
                self.frpc_version = None;
                cx.notify();
                return;
            }
        };
        let task: Task<Result<String>> = cx.background_spawn(async move {
            let mut cmd = std::process::Command::new(&exe_path);
            cmd.arg("--version");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            let out = cmd.output().map_err(|e| anyhow::anyhow!("{}", e))?;
            let s = String::from_utf8_lossy(&out.stdout);
            let e = String::from_utf8_lossy(&out.stderr);
            Ok(if !s.trim().is_empty() {
                s.trim().to_string()
            } else if !e.trim().is_empty() {
                e.trim().to_string()
            } else {
                "未知版本".to_string()
            })
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            this.update(cx, |v, cx| {
                v.frpc_version = Some(match r {
                    Ok(v) => v,
                    Err(_) => "无法运行".to_string(),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn install_service(&mut self, cx: &mut Context<Self>) {
        self.is_processing = true;
        self.status_message = None;
        cx.notify();
        let task: Task<Result<()>> = cx.background_spawn(async move { service::install_service() });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            this.update(cx, |v, cx| {
                v.is_processing = false;
                match r {
                    Ok(()) => {
                        v.service_registered = true;
                        v.set_status_message("注册成功".to_string(), MessageLevel::Success, cx);
                    }
                    Err(e) => {
                        v.set_status_message(format!("注册失败：{}", e), MessageLevel::Error, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn uninstall_service(&mut self, cx: &mut Context<Self>) {
        self.is_processing = true;
        self.status_message = None;
        cx.notify();
        let task: Task<Result<()>> =
            cx.background_spawn(async move { service::uninstall_service() });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            this.update(cx, |v, cx| {
                v.is_processing = false;
                match r {
                    Ok(()) => {
                        v.service_registered = false;
                        v.set_status_message("已注销".to_string(), MessageLevel::Success, cx);
                    }
                    Err(e) => {
                        v.set_status_message(format!("注销失败：{}", e), MessageLevel::Error, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 启动周期性健康检查，每 3 秒检测所有运行中的 frpc 进程
    pub fn start_health_monitor(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_spawn(async {
                std::thread::sleep(Duration::from_secs(3));
            })
            .await;
            let alive = this
                .update(cx, |view, cx| {
                    // 检查已跟踪的进程是否仍然存活
                    let mut dead_names = Vec::new();
                    for (name, rp) in view.running.iter_mut() {
                        if !rp.process.is_running() {
                            log::warn!("[{}] 健康检查发现进程已退出", name);
                            dead_names.push(name.clone());
                        }
                    }
                    if !dead_names.is_empty() {
                        for name in &dead_names {
                            view.running.remove(name);

                            // 进程守护：非手动停止的进程自动重启
                            if view.process_guard && !view.stopped_configs.contains(name) {
                                log::info!("[{}] 进程守护：尝试重启", name);
                                match service::start_frpc_process(name) {
                                    Ok(p) => {
                                        view.running
                                            .insert(name.clone(), RunningProcess { process: p });
                                        log::info!("[{}] 进程守护：重启成功", name);
                                        view.set_status_message(
                                            format!("'{}' 进程异常退出，已自动重启", name),
                                            MessageLevel::Warning,
                                            cx,
                                        );
                                    }
                                    Err(e) => {
                                        log::error!("[{}] 进程守护：重启失败 - {}", name, e);
                                    }
                                }
                            } else {
                                log::info!("[{}] 已从运行列表移除", name);
                            }
                        }
                        cx.notify();
                    }

                    true
                })
                .unwrap_or(false);
            if !alive {
                break;
            }
        })
        .detach();
    }
}

impl gpui::Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sb = sidebar::render(self, cx);
        let content = match &self.page {
            Page::ConfigList => pages::config_list::render(self, cx),
            Page::ConfigEditor { .. } => pages::config_editor::render(self, cx),
            Page::Settings => pages::settings::render(self, cx),
        };
        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(cx.theme().background)
            .child(sb)
            .child(div().w(px(1.0)).h_full().bg(cx.theme().border))
            .child(content)
    }
}

pub fn run_app(pre_check: PreCheckResult) {
    let app = gpui_platform::application().with_assets(crate::icons::AppAssets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        theme::load_all_themes(cx);
        let saved_theme = theme::load_theme_preference();
        theme::apply_theme(&saved_theme, cx);
        let bounds = Bounds::centered(None, size(px(960.0), px(600.0)), cx);
        let init = pre_check.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(960.0), px(600.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("FrpDesk")),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let name_input = cx.new(|cx| InputState::new(window, cx));
                let content_input = cx.new(|cx| InputState::new(window, cx).code_editor("toml"));

                // 创建主题下拉选择
                let themes = theme::available_themes();
                let current = theme::current_theme_name();
                let theme_names: Vec<SharedString> =
                    themes.iter().map(|t| t.name.clone().into()).collect();
                let selected = themes.iter().position(|t| t.name == current);
                let selected_index = selected.map(|i| IndexPath::default().row(i));
                let theme_select =
                    cx.new(|cx| SelectState::new(theme_names, selected_index, window, cx));

                let app_view = cx.new(|cx| {
                    let mut v = AppView::new(
                        init,
                        name_input,
                        content_input,
                        theme_select.clone(),
                        window,
                        cx,
                    );
                    v.detect_frpc_version(cx);
                    AppView::start_health_monitor(cx);
                    v
                });

                cx.new(|cx| Root::new(app_view, window, cx))
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
