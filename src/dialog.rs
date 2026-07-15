//! 基于 GPUI 的服务管理对话框

use anyhow::Result;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, FontWeight, MouseButton,
    SharedString, Task, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::download;
use crate::interactive::{self, PreCheckResult};

/// 旋转 spinner 的 Unicode braille 字符帧
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 下载完成后的后续操作
#[derive(Clone, Debug)]
enum PostDoneAction {
    /// 无后续操作
    None,
    /// 启动服务
    StartService,
}

/// 对话框步骤
#[derive(Clone, Debug)]
enum Step {
    /// 询问用户操作，携带当前服务状态
    Question(PreCheckResult),
    /// 正在执行操作
    Processing,
    /// 正在下载 frpc（下载完成后自动进入 Done）
    Downloading,
    /// 操作完成，显示结果；点击"确定"后重新检查状态或执行后续操作
    Done(String, PostDoneAction),
}

/// 服务管理对话框视图
struct ServiceDialogView {
    step: Step,
    spinner_frame: usize,
    download_percent: u64,
}

impl ServiceDialogView {
    fn handle_action(&mut self, action_id: usize, cx: &mut Context<Self>) {
        // 用 take() 临时取出 step，避免 borrow 冲突
        let current_step = std::mem::replace(&mut self.step, Step::Processing);
        match current_step {
            Step::Question(pre_check) => {
                let op: Option<fn() -> Result<()>> = match (&pre_check, action_id) {
                    (PreCheckResult::Running, 0) => Some(interactive::op_delete_and_stop),
                    (PreCheckResult::Running, 1) => Some(interactive::op_stop_only),
                    (PreCheckResult::Stopped, 0) => {
                        // 启动前检查 frpc 是否存在
                        let exe_dir = std::env::current_exe()
                            .ok()
                            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                            .unwrap_or_else(|| std::path::PathBuf::from("."));
                        if !download::has_frpc_executable(&exe_dir) {
                            // frpc 不存在，进入下载流程，下载完成后启动服务
                            self.step = Step::Downloading;
                            self.download_percent = 0;
                            self.spinner_frame = 0;
                            cx.notify();
                            self.start_download(cx);
                            return;
                        }
                        Some(interactive::op_start)
                    }
                    (PreCheckResult::Stopped, 1) => Some(interactive::op_delete),
                    (_, 99) => {
                        cx.quit();
                        return;
                    }
                    _ => {
                        cx.quit();
                        return;
                    }
                };

                if let Some(op) = op {
                    self.step = Step::Processing;
                    self.spinner_frame = 0;
                    cx.notify();
                    self.start_spinner(cx);

                    let task: Task<Result<()>> = cx.background_spawn(async move { op() });
                    let mut async_cx = cx.to_async();
                    cx.spawn(
                        move |this: gpui::WeakEntity<ServiceDialogView>,
                              _cx: &mut gpui::AsyncApp| async move {
                            let result = task.await;
                            let msg = match result {
                                Ok(()) => "操作已完成。".to_string(),
                                Err(e) => format!("操作失败：{}", e),
                            };
                            this.update(&mut async_cx, |view, cx| {
                                view.step = Step::Done(msg, PostDoneAction::None);
                                cx.notify();
                            })
                            .ok();
                        },
                    )
                    .detach();
                } else {
                    self.step = Step::Question(pre_check);
                }
            }
            Step::Processing => {
                self.step = Step::Processing;
            }
            Step::Downloading => {
                self.step = Step::Downloading;
            }
            Step::Done(_msg, post_action) => {
                match post_action {
                    PostDoneAction::StartService => {
                        // 执行启动服务操作
                        self.step = Step::Processing;
                        self.spinner_frame = 0;
                        cx.notify();
                        self.start_spinner(cx);

                        let task: Task<Result<()>> =
                            cx.background_spawn(async move { interactive::op_start() });
                        let mut async_cx = cx.to_async();
                        cx.spawn(
                            move |this: gpui::WeakEntity<ServiceDialogView>,
                                  _cx: &mut gpui::AsyncApp| async move {
                                let result = task.await;
                                let msg = match result {
                                    Ok(()) => "操作已完成。".to_string(),
                                    Err(e) => format!("操作失败：{}", e),
                                };
                                this.update(&mut async_cx, |view, cx| {
                                    view.step = Step::Done(msg, PostDoneAction::None);
                                    cx.notify();
                                })
                                .ok();
                            },
                        )
                        .detach();
                    }
                    PostDoneAction::None => {
                        // 普通确定：重新检查服务状态
                        self.step = Step::Processing;
                        self.spinner_frame = 0;
                        cx.notify();
                        self.start_spinner(cx);

                        let mut async_cx = cx.to_async();
                        let task: Task<Result<PreCheckResult>> =
                            cx.background_spawn(async { interactive::check_service_status() });

                        cx.spawn(
                            move |this: gpui::WeakEntity<ServiceDialogView>,
                                  _cx: &mut gpui::AsyncApp| async move {
                                let result = task.await;
                                this.update(&mut async_cx, |view, cx| match result {
                                    Ok(
                                        status
                                        @ (PreCheckResult::Running | PreCheckResult::Stopped),
                                    ) => {
                                        view.step = Step::Question(status);
                                        cx.notify();
                                    }
                                    _ => {
                                        // 服务已删除或无法获取状态，退出
                                        cx.quit();
                                    }
                                })
                                .ok();
                            },
                        )
                        .detach();
                    }
                }
            }
        }
    }

    /// 启动下载流程
    fn start_download(&mut self, cx: &mut Context<Self>) {
        self.spinner_frame = 0;
        cx.notify();
        self.start_spinner(cx);

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let progress = Arc::new(AtomicU64::new(0));
        let progress_clone = progress.clone();

        // 进度更新定时器
        let this = cx.entity().downgrade();
        let mut async_cx_1 = cx.to_async();
        cx.spawn(
            move |_this: gpui::WeakEntity<ServiceDialogView>, _cx: &mut gpui::AsyncApp| async move {
                loop {
                    async_cx_1
                        .background_spawn(async {
                            std::thread::sleep(Duration::from_millis(200));
                        })
                        .await;

                    let should_continue = this
                        .update(&mut async_cx_1, |view, cx| {
                            if matches!(view.step, Step::Downloading) {
                                view.download_percent = progress_clone.load(Ordering::Relaxed);
                                cx.notify();
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !should_continue {
                        break;
                    }
                }
            },
        )
        .detach();

        // 后台下载任务
        let task: Task<Result<()>> = cx.background_spawn(async move {
            download::download_and_extract_frpc(&exe_dir, &move |downloaded, total| {
                let percent = if total > 0 {
                    (downloaded * 100 / total).min(100)
                } else {
                    0
                };
                progress.store(percent, Ordering::Relaxed);
            })
        });

        // 下载完成回调
        let post_action = PostDoneAction::StartService;
        let mut async_cx_2 = cx.to_async();
        cx.spawn(
            move |this: gpui::WeakEntity<ServiceDialogView>, _cx: &mut gpui::AsyncApp| async move {
                let result = task.await;
                this.update(&mut async_cx_2, |view, cx| {
                    let msg = match result {
                        Ok(()) => "frpc.exe 下载成功。".to_string(),
                        Err(e) => format!("frpc 下载失败：{}", e),
                    };
                    view.step = Step::Done(msg, post_action);
                    cx.notify();
                })
                .ok();
            },
        )
        .detach();
    }

    /// 启动 spinner 动画定时器
    fn start_spinner(&self, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let mut async_cx = cx.to_async();
        cx.spawn(
            move |_this: gpui::WeakEntity<ServiceDialogView>, _cx: &mut gpui::AsyncApp| async move {
                loop {
                    // 在后台线程休眠，避免阻塞 UI
                    async_cx
                        .background_spawn(async {
                            std::thread::sleep(Duration::from_millis(120));
                        })
                        .await;

                    let should_continue = this
                        .update(&mut async_cx, |view, cx| {
                            if matches!(view.step, Step::Processing | Step::Downloading) {
                                view.spinner_frame =
                                    (view.spinner_frame + 1) % SPINNER_FRAMES.len();
                                cx.notify();
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !should_continue {
                        break;
                    }
                }
            },
        )
        .detach();
    }

    fn question_buttons(pre_check: &PreCheckResult) -> Vec<(&str, usize)> {
        match pre_check {
            PreCheckResult::Running => {
                vec![("删除服务并停止", 0), ("仅停止", 1), ("取消", 99)]
            }
            PreCheckResult::Stopped => {
                vec![("启动", 0), ("删除", 1), ("取消", 99)]
            }
            PreCheckResult::NotRegistered => vec![],
        }
    }
}

impl Render for ServiceDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, message, buttons, is_busy) = match &self.step {
            Step::Question(pre_check) => {
                let title = "服务管理".to_string();
                let message = match pre_check {
                    PreCheckResult::Running => {
                        "服务 FrpcService 已在运行中。\n\n请选择您要执行的操作：".to_string()
                    }
                    PreCheckResult::Stopped => {
                        "服务 FrpcService 已停止。\n\n请选择您要执行的操作：".to_string()
                    }
                    PreCheckResult::NotRegistered => "服务未注册。".to_string(),
                };
                let buttons: Vec<(String, usize)> = Self::question_buttons(pre_check)
                    .into_iter()
                    .map(|(label, id)| (label.to_string(), id))
                    .collect();
                (title, message, buttons, false)
            }
            Step::Processing => (
                "服务管理".to_string(),
                "正在执行操作，请稍候...".to_string(),
                vec![],
                true,
            ),
            Step::Downloading => (
                "下载 frpc".to_string(),
                format!("正在从 GitHub 下载 frpc.exe ... {}%", self.download_percent),
                vec![],
                true,
            ),
            Step::Done(msg, _) => (
                "操作结果".to_string(),
                msg.clone(),
                vec![("确定".to_string(), 100)],
                false,
            ),
        };

        div()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .size_full()
            .p(px(32.0))
            .gap_y(px(20.0))
            // 标题区域：左侧蓝色竖条 + 标题文字
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_x(px(12.0))
                    .child(
                        div()
                            .w(px(4.0))
                            .h(px(24.0))
                            .bg(rgb(0x0078d4))
                            .rounded(px(2.0)),
                    )
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xffffff))
                            .child(title),
                    ),
            )
            // 消息内容
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xbbbbbb))
                    .line_height(px(22.0))
                    .child(message),
            )
            // 下载进度条（仅 Downloading 状态显示）
            .child(div().when(matches!(self.step, Step::Downloading), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_y(px(6.0))
                        .child(
                            div()
                                .w(px(400.0))
                                .h(px(10.0))
                                .bg(rgb(0x333333))
                                .rounded(px(5.0))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .h_full()
                                        .bg(rgb(0x0078d4))
                                        .rounded(px(5.0))
                                        .w(px(400.0 * self.download_percent as f32 / 100.0)),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x888888))
                                .child(format!("{}%", self.download_percent)),
                        ),
                )
            }))
            // 弹性空间，将按钮推到底部
            .child(div().flex_1())
            // 底部区域：进度指示器 + 按钮
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    // 左侧：进度提示（仅 Processing / Downloading 状态显示）
                    .child(div().when(is_busy, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_x(px(8.0))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(0x0078d4))
                                        .child(SPINNER_FRAMES[self.spinner_frame]),
                                )
                                .child(
                                    div().text_xs().text_color(rgb(0x888888)).child("处理中..."),
                                ),
                        )
                    }))
                    // 右侧：按钮组
                    .child(div().flex().flex_row().gap_x(px(10.0)).children(
                        buttons.into_iter().map(|(label, action_id)| {
                            let is_primary = action_id != 99;
                            let (bg, hover_bg, fg) = if is_primary {
                                (rgb(0x0078d4), rgb(0x1a8cff), rgb(0xffffff))
                            } else {
                                (rgb(0x2d2d2d), rgb(0x3a3a3a), rgb(0xcccccc))
                            };

                            div()
                                .px(px(20.0))
                                .py(px(8.0))
                                .rounded(px(6.0))
                                .bg(bg)
                                .text_color(fg)
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .cursor_pointer()
                                .hover(|s| s.bg(hover_bg))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(move |view, _event, _window, cx| {
                                        view.handle_action(action_id, cx);
                                    }),
                                )
                                .child(label)
                        }),
                    )),
            )
    }
}

/// 检查 frpc 是否存在，不存在则尝试下载
fn check_and_prompt_download(exe_dir: &std::path::Path) -> Step {
    if download::has_frpc_executable(exe_dir) {
        return Step::Done(
            "服务已成功注册为 FrpcService。".to_string(),
            PostDoneAction::None,
        );
    }

    // 直接进入下载流程（弹出确认提示属于用户体验细节，这里简化为直接下载）
    Step::Downloading
}

/// 运行服务管理对话框
pub fn run_service_dialog(pre_check: PreCheckResult) {
    // 对于未注册的服务，预先执行安装操作
    let initial_step = match &pre_check {
        PreCheckResult::NotRegistered => {
            let result = interactive::op_install();
            match result {
                Ok(()) => {
                    let exe_dir = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    check_and_prompt_download(&exe_dir)
                }
                Err(e) => Step::Done(format!("服务安装失败：{}", e), PostDoneAction::None),
            }
        }
        _ => Step::Question(pre_check),
    };

    let app = Application::new();
    app.run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(480.0), px(320.0)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("FRP 服务管理")),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|cx| {
                    if matches!(initial_step, Step::Downloading) {
                        let mut view = ServiceDialogView {
                            step: initial_step,
                            spinner_frame: 0,
                            download_percent: 0,
                        };
                        view.start_download(cx);
                        view
                    } else {
                        ServiceDialogView {
                            step: initial_step,
                            spinner_frame: 0,
                            download_percent: 0,
                        }
                    }
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
