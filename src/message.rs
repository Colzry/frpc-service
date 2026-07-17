//! 消息提示组件：悬浮在顶部中间，支持 info / success / warning / error 四种类型

use crate::icons::AppIcon;
use gpui::prelude::*;
use gpui::{div, img, px, SharedString};
use gpui_component::{IconNamed, ThemeColor};

/// 消息类型
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum MessageLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// 获取消息类型对应的图标路径
fn message_icon(level: &MessageLevel) -> SharedString {
    match level {
        MessageLevel::Info => AppIcon::InfoBlue.path(),
        MessageLevel::Success => AppIcon::CircleCheckGreen.path(),
        MessageLevel::Warning => AppIcon::InfoYellow.path(),
        MessageLevel::Error => AppIcon::CircleXRed.path(),
    }
}

/// 构建消息提示的 div，供各页面使用
pub fn toast(level: &MessageLevel, msg: &str, theme: &ThemeColor) -> gpui::Div {
    let icon_path = message_icon(level);

    div()
        .absolute()
        .top(px(16.0))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap_x(px(8.0))
                .px(px(16.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .shadow_sm()
                .child(img(icon_path.as_ref()).w(px(16.0)).h(px(16.0)))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.foreground)
                        .child(msg.to_string()),
                ),
        )
}
