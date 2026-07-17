//! 配置列表页面：卡片网格展示所有配置，支持分页（每页最多8个配置 + 1个添加卡 = 9个，3行×3列）

use crate::icons::AppIcon;
use crate::message;
use gpui::prelude::*;
use gpui::{div, px, FontWeight};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::spinner::Spinner;
use gpui_component::{ActiveTheme, Disableable};

use crate::app::AppView;

/// 每页最多显示的配置数量（不含添加卡）
const PAGE_SIZE: usize = 8;

pub fn render(view: &mut AppView, cx: &mut Context<AppView>) -> gpui::AnyElement {
    let configs = view.configs.clone();
    let running_keys: Vec<String> = view.running.keys().cloned().collect();
    let current_page = view.config_page;

    let total_pages = ((configs.len() + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
    let page_configs: Vec<_> = configs
        .into_iter()
        .skip(current_page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .collect();

    let mut content = div()
        .flex_1()
        .h_full()
        .bg(cx.theme().background)
        .flex()
        .flex_col();

    // 顶部标题栏
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(24.0))
        .py(px(16.0))
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(cx.theme().foreground)
                .child("配置列表"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_x(px(8.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("共 {} 个配置", view.configs.len())),
                )
                .child(
                    Button::new("btn-add-config")
                        .primary()
                        .icon(AppIcon::Plus)
                        .label("添加配置")
                        .on_click(cx.listener(|view, _event, window, cx| {
                            view.open_add_config(window, cx);
                        })),
                ),
        );

    // 配置卡片网格
    let mut grid = div()
        .flex()
        .flex_wrap()
        .gap_x(px(16.0))
        .gap_y(px(16.0))
        .px(px(24.0))
        .pb(px(16.0));

    for config_item in &page_configs {
        let name = config_item.name.clone();
        let is_running = running_keys.contains(&name);
        let card = render_config_card(&name, is_running, cx);
        grid = grid.child(card);
    }

    // 添加配置的虚线卡片（始终显示在最后）
    let add_card = div()
        .w(px(240.0))
        .h(px(140.0))
        .rounded(px(8.0))
        .border_2()
        .border_dashed()
        .border_color(cx.theme().border)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_y(px(8.0))
        .cursor_pointer()
        .hover(|s| s.border_color(cx.theme().primary))
        .id("add-card")
        .on_click(cx.listener(|view, _event, window, cx| {
            view.open_add_config(window, cx);
        }))
        .child(
            div()
                .text_xl()
                .text_color(cx.theme().muted_foreground)
                .child("+"),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("添加配置"),
        );

    grid = grid.child(add_card);

    content = content.child(header).child(grid);

    // 分页控件
    if total_pages > 1 {
        let has_prev = current_page > 0;
        let has_next = current_page + 1 < total_pages;

        let pagination = div()
            .flex()
            .items_center()
            .justify_center()
            .gap_x(px(12.0))
            .px(px(24.0))
            .pb(px(16.0))
            .child(
                Button::new("btn-prev-page")
                    .icon(AppIcon::ArrowLeft)
                    .label("上一页")
                    .when(!has_prev, |b| b.disabled(true))
                    .on_click(cx.listener(|view, _event, _window, cx| {
                        if view.config_page > 0 {
                            view.config_page -= 1;
                            cx.notify();
                        }
                    })),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{}/{}", current_page + 1, total_pages)),
            )
            .child(
                Button::new("btn-next-page")
                    .label("下一页")
                    .when(!has_next, |b| b.disabled(true))
                    .on_click(cx.listener(move |view, _event, _window, cx| {
                        let total = ((view.configs.len() + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
                        if view.config_page + 1 < total {
                            view.config_page += 1;
                            cx.notify();
                        }
                    })),
            );

        content = content.child(pagination);
    }

    // 处理中指示器
    if view.is_processing {
        content = content.child(
            div()
                .mx(px(24.0))
                .mb(px(16.0))
                .flex()
                .items_center()
                .gap_x(px(8.0))
                .child(Spinner::new())
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("正在处理..."),
                ),
        );
    }

    div()
        .relative()
        .flex_1()
        .h_full()
        .child(content)
        .when_some(view.status_message.clone(), |el, msg| {
            el.child(message::toast(&view.status_level, &msg, cx.theme()))
        })
        .into_any_element()
}

fn render_config_card(name: &str, is_running: bool, cx: &mut Context<AppView>) -> gpui::AnyElement {
    let name_for_action = name.to_string();
    let name_for_edit = name.to_string();
    let name_for_delete = name.to_string();

    div()
        .w(px(240.0))
        .h(px(140.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().sidebar)
        .p(px(16.0))
        .flex()
        .flex_col()
        .justify_between()
        // 第一行：名称 + 状态点
        .child(
            div()
                .flex()
                .items_center()
                .gap_x(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(if is_running {
                            cx.theme().success
                        } else {
                            cx.theme().danger
                        }),
                ),
        )
        // 第二行：启停/重启按钮
        .child(
            div()
                .flex()
                .items_center()
                .gap_x(px(6.0))
                .child(if is_running {
                    Button::new(format!("btn-stop-{}", name))
                        .danger()
                        .icon(AppIcon::Square)
                        .label("停止")
                        .on_click(cx.listener(move |view, _event, _window, cx| {
                            view.stop_config(&name_for_action, cx);
                        }))
                        .into_any_element()
                } else {
                    Button::new(format!("btn-start-{}", name))
                        .success()
                        .icon(AppIcon::Play)
                        .label("启动")
                        .on_click(cx.listener(move |view, _event, _window, cx| {
                            view.start_config(&name_for_action, cx);
                        }))
                        .into_any_element()
                })
                .when(is_running, |el| {
                    let name_for_restart = name.to_string();
                    el.child(
                        Button::new(format!("btn-restart-{}", name))
                            .icon(AppIcon::RotateCcw)
                            .label("重启")
                            .on_click(cx.listener(move |view, _event, _window, cx| {
                                view.restart_config(&name_for_restart, cx);
                            })),
                    )
                }),
        )
        // 第三行：编辑/删除
        .child(
            div()
                .flex()
                .items_center()
                .gap_x(px(4.0))
                .child(
                    Button::new(format!("btn-edit-{}", name))
                        .ghost()
                        .icon(AppIcon::SquarePen)
                        .label("编辑")
                        .on_click(cx.listener(move |view, _event, window, cx| {
                            view.open_edit_config(&name_for_edit, window, cx);
                        })),
                )
                .child(
                    Button::new(format!("btn-delete-{}", name))
                        .ghost()
                        .icon(AppIcon::Trash)
                        .label("删除")
                        .on_click(cx.listener(move |view, _event, _window, cx| {
                            view.delete_config(&name_for_delete, cx);
                        })),
                ),
        )
        .into_any_element()
}
