//! 主题管理模块：加载自定义主题、切换主题、保存偏好

use gpui::App;
use gpui_component::theme::{Theme, ThemeMode, ThemeRegistry};
use std::sync::RwLock;

/// 当前主题名称（全局状态）
static CURRENT_THEME: RwLock<String> = RwLock::new(String::new());

/// 获取当前主题名称
pub fn current_theme_name() -> String {
    CURRENT_THEME
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 当前主题是否为暗色模式（基于主题名称判断）
pub fn is_dark_mode(_cx: &gpui::App) -> bool {
    let name = current_theme_name();
    // "Default Light" 是唯一的亮色主题，其余均为暗色
    !name.contains("Light")
}

/// 当前主题的 primary_foreground 是否为深色（需要黑色图标）
/// - Default Dark、Custom Dark 的 primary.foreground 是 neutral-900（深色）
/// - 其他主题的 primary.foreground 是白色或浅色
pub fn is_primary_foreground_dark() -> bool {
    let name = current_theme_name();
    name == "Default Dark" || name == "Custom Dark"
}

/// 设置当前主题名称
fn set_current_theme(name: &str) {
    if let Ok(mut current) = CURRENT_THEME.write() {
        *current = name.to_string();
    }
}

/// 主题信息
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeInfo {
    pub name: String,
    pub mode: ThemeMode,
}

/// 可用主题列表
pub fn available_themes() -> Vec<ThemeInfo> {
    vec![
        ThemeInfo {
            name: "Default Light".to_string(),
            mode: ThemeMode::Light,
        },
        ThemeInfo {
            name: "Default Dark".to_string(),
            mode: ThemeMode::Dark,
        },
        ThemeInfo {
            name: "Custom Dark".to_string(),
            mode: ThemeMode::Dark,
        },
        ThemeInfo {
            name: "Ocean Blue".to_string(),
            mode: ThemeMode::Dark,
        },
        ThemeInfo {
            name: "Warm Sunset".to_string(),
            mode: ThemeMode::Dark,
        },
    ]
}

/// 内嵌的主题 JSON 文件
const THEME_JSONS: &[&str] = &[
    include_str!("../themes/light.json"),
    include_str!("../themes/dark.json"),
    include_str!("../themes/custom-dark.json"),
    include_str!("../themes/ocean-blue.json"),
    include_str!("../themes/sunset.json"),
];

/// 加载所有自定义主题到注册表
pub fn load_all_themes(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for json in THEME_JSONS {
        if let Err(e) = registry.load_themes_from_str(json) {
            log::error!("加载主题失败: {:?}", e);
        }
    }
}

/// 应用指定主题
pub fn apply_theme(theme_name: &str, cx: &mut gpui::App) {
    let themes = available_themes();
    let info = themes.iter().find(|t| t.name == theme_name);

    if let Some(info) = info {
        // 先确保 Theme 全局已初始化
        Theme::change(info.mode, None, cx);

        // 从注册表获取主题配置
        let registry = ThemeRegistry::global(cx);
        if let Some(theme_config) = registry.themes().get(theme_name).cloned() {
            let theme = Theme::global_mut(cx);
            match info.mode {
                ThemeMode::Light => theme.light_theme = theme_config,
                ThemeMode::Dark => theme.dark_theme = theme_config,
            }
            Theme::change(info.mode, None, cx);
            // 刷新所有窗口以应用新主题
            cx.refresh_windows();
            set_current_theme(theme_name);
            log::info!("主题已切换为: {}", theme_name);
        }
    }
}

/// 获取主题偏好文件路径
fn theme_pref_path() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("conf").join("theme.json")))
}

/// 保存主题偏好
pub fn save_theme_preference(theme_name: &str) {
    if let Some(path) = theme_pref_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = format!("{{\"theme\": \"{}\"}}", theme_name);
        let _ = std::fs::write(&path, content);
    }
}

/// 加载主题偏好，返回主题名称
pub fn load_theme_preference() -> String {
    let default_theme = "Default Light".to_string();

    if let Some(path) = theme_pref_path() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(name) = json["theme"].as_str() {
                    set_current_theme(name);
                    return name.to_string();
                }
            }
        }
    }

    set_current_theme(&default_theme);
    default_theme
}
