//! 基于 GPUI 的服务管理对话框
//!
//! 使用单一 GPUI 应用程序和状态机模式，
//! 替代原生 Windows MessageBoxW 和 rfd 对话框。

use anyhow::Result;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, FontWeight, MouseButton,
    SharedString, Task, TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use crate::interactive::{self, PreCheckResult};

/// 对话框步骤
#[derive(Clone, Debug)]
enum Step {
    /// 询问用户操作
    Question,
    /// 正在执行操作
    Processing,
    /// 显示操作结果
    Info(String),
}

/// 服务管理对话框视图
struct ServiceDialogView {
    pre_check: PreCheckResult,
    step: Step,
}

impl ServiceDialogView {
    fn handle_action(&mut self, action_id: usize, cx: &mut Context<Self>) {
        match &self.step {
            Step::Question => {
                let op: Option<fn() -> Result<()>> = match (&self.pre_check, action_id) {
                    (PreCheckResult::Running, 0) => Some(interactive::op_delete_and_stop),
                    (PreCheckResult::Running, 1) => Some(interactive::op_stop_only),
                    (PreCheckResult::Stopped, 0) => Some(interactive::op_start),
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
                    cx.notify();

                    // 在后台线程执行阻塞操作，不阻塞 UI
                    let task: Task<Result<()>> = cx.background_spawn(async move { op() });

                    // 使用 to_async() 获取拥有 'static 生命周期的 AsyncApp
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
                                view.step = Step::Info(msg);
                                cx.notify();
                            })
                            .ok();
                        },
                    )
                    .detach();
                }
            }
            Step::Info(_) => {
                cx.quit();
            }
            Step::Processing => {} // 操作进行中，忽略点击
        }
    }

    fn question_buttons(&self) -> Vec<(&str, usize)> {
        match self.pre_check {
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
            Step::Question => {
                let title = "服务管理".to_string();
                let message = match self.pre_check {
                    PreCheckResult::Running => {
                        "服务 FrpcService 已在运行中。\n\n请选择您要执行的操作：".to_string()
                    }
                    PreCheckResult::Stopped => {
                        "服务 FrpcService 已停止。\n\n请选择您要执行的操作：".to_string()
                    }
                    PreCheckResult::NotRegistered => unreachable!(),
                };
                let buttons: Vec<(String, usize)> = self
                    .question_buttons()
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
            Step::Info(msg) => (
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
            // 弹性空间，将按钮推到底部
            .child(div().flex_1())
            // 底部区域：进度指示器 + 按钮
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    // 左侧：进度提示（仅 Processing 状态显示）
                    .child(div().when(is_busy, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_x(px(8.0))
                                .child(
                                    div()
                                        .w(px(16.0))
                                        .h(px(16.0))
                                        .border_2()
                                        .border_color(rgb(0x0078d4))
                                        .rounded_full(),
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

/// 运行服务管理对话框
pub fn run_service_dialog(pre_check: PreCheckResult) {
    // 对于未注册的服务，预先执行安装操作
    let initial_step = match &pre_check {
        PreCheckResult::NotRegistered => {
            let result = interactive::op_install_and_start();
            let msg = match result {
                Ok(()) => "服务已成功注册为 FrpcService 并已启动。".to_string(),
                Err(e) => format!("服务安装失败：{}", e),
            };
            Step::Info(msg)
        }
        _ => Step::Question,
    };

    let app = Application::new();
    app.run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(480.0), px(300.0)), cx);

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
                cx.new(|_| ServiceDialogView {
                    pre_check,
                    step: initial_step,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
