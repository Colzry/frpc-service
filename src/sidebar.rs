//! 侧边栏渲染：导航菜单 + 服务状态

use gpui::prelude::*;
use gpui::{div, img, px, FontWeight};
use gpui_component::{ActiveTheme, IconNamed};

use crate::app::{AppView, Page};
use crate::icons::AppIcon;

pub fn render(view: &AppView, cx: &mut Context<AppView>) -> gpui::AnyElement {
    let is_config = matches!(view.page, Page::ConfigList | Page::ConfigEditor { .. });
    let is_settings = view.page == Page::Settings;
    let is_dark = crate::theme::is_dark_mode(cx);
    let primary_is_dark = crate::theme::is_primary_foreground_dark();

    // 根据主题和选中状态选择图标：
    // - 选中 + primary 前景深色：黑色图标（如 Default Dark、Custom Dark）
    // - 选中 + primary 前景浅色：白色图标（如其他主题）
    // - 未选中 + 暗色主题：浅灰色图标
    // - 未选中 + 亮色主题：深色图标（currentColor 默认黑色）
    let menu_icon = |active: bool,
                     dark_icon: AppIcon,
                     light_icon: AppIcon,
                     black_icon: AppIcon,
                     white_icon: AppIcon|
     -> AppIcon {
        if active {
            if primary_is_dark {
                black_icon
            } else {
                white_icon
            }
        } else if is_dark {
            light_icon
        } else {
            dark_icon
        }
    };

    div()
        .id("sidebar")
        .w(px(120.0))
        .h_full()
        .bg(cx.theme().sidebar)
        .flex()
        .flex_col()
        .p(px(0.0))
        // 标题区域
        .child(
            div()
                .px(px(16.0))
                .py(px(16.0))
                .flex()
                .flex_col()
                .gap_y(px(4.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(cx.theme().sidebar_foreground)
                        .child("FrpcDesk"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                ),
        )
        // 分割线
        .child(div().h(px(1.0)).bg(cx.theme().sidebar_border))
        // 菜单项
        .child(
            div()
                .flex()
                .flex_col()
                .gap_y(px(8.0))
                .px(px(12.0))
                .py(px(16.0))
                .child(
                    div()
                        .id("menu-config")
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .gap_x(px(8.0))
                        .cursor_pointer()
                        .when_else(
                            is_config,
                            |d| {
                                d.bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                            },
                            |d| {
                                d.text_color(cx.theme().sidebar_foreground)
                                    .hover(|d| d.bg(cx.theme().accent))
                            },
                        )
                        .on_click(cx.listener(move |view, _event, _window, cx| {
                            view.switch_page(Page::ConfigList, cx);
                        }))
                        .child(
                            img(menu_icon(
                                is_config,
                                AppIcon::FileSliders,
                                AppIcon::FileSlidersLight,
                                AppIcon::FileSlidersBlack,
                                AppIcon::FileSlidersWhite,
                            )
                            .path()
                            .as_ref())
                            .w(px(16.0))
                            .h(px(16.0)),
                        )
                        .child(div().text_sm().child("配置")),
                )
                .child(
                    div()
                        .id("menu-settings")
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .gap_x(px(8.0))
                        .cursor_pointer()
                        .when_else(
                            is_settings,
                            |d| {
                                d.bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                            },
                            |d| {
                                d.text_color(cx.theme().sidebar_foreground)
                                    .hover(|d| d.bg(cx.theme().accent))
                            },
                        )
                        .on_click(cx.listener(move |view, _event, _window, cx| {
                            view.switch_page(Page::Settings, cx);
                            view.detect_frpc_version(cx);
                        }))
                        .child(
                            img(menu_icon(
                                is_settings,
                                AppIcon::Settings,
                                AppIcon::SettingsLight,
                                AppIcon::SettingsBlack,
                                AppIcon::SettingsWhite,
                            )
                            .path()
                            .as_ref())
                            .w(px(16.0))
                            .h(px(16.0)),
                        )
                        .child(div().text_sm().child("设置")),
                ),
        )
        // 底部服务状态
        .child(div().flex_1())
        .child(div().h(px(1.0)).bg(cx.theme().sidebar_border))
        .child(
            div()
                .px(px(16.0))
                .py(px(12.0))
                .flex()
                .flex_col()
                .gap_y(px(4.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Windows 服务"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_x(px(6.0))
                        .child(div().w(px(8.0)).h(px(8.0)).rounded(px(4.0)).bg(
                            if view.service_registered {
                                cx.theme().success
                            } else {
                                cx.theme().danger
                            },
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().sidebar_foreground)
                                .child(if view.service_registered {
                                    "已注册"
                                } else {
                                    "未注册"
                                }),
                        ),
                ),
        )
        .into_any_element()
}
