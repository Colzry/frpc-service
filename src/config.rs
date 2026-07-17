//! frpc 配置管理模块，负责在 conf/ 目录下管理多个 frpc 配置

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 单个 frpc 配置的元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrpcConfigMeta {
    /// 配置名称（也是文件名前缀）
    pub name: String,
    /// 是否开机自启
    #[serde(default)]
    pub auto_start: bool,
}

/// 所有配置的元数据集合
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigStore {
    pub configs: Vec<FrpcConfigMeta>,
}

/// 获取程序目录下的 conf/ 目录路径
pub fn conf_dir() -> Result<PathBuf> {
    let exe_path = std::env::current_exe().context("无法获取可执行文件路径")?;
    let exe_dir = exe_path.parent().context("无法获取可执行文件目录")?;
    Ok(exe_dir.join("conf"))
}

/// 获取程序目录下的 bin/ 目录路径
pub fn bin_dir() -> Result<PathBuf> {
    let exe_path = std::env::current_exe().context("无法获取可执行文件路径")?;
    let exe_dir = exe_path.parent().context("无法获取可执行文件目录")?;
    Ok(exe_dir.join("bin"))
}

/// 元数据文件路径: conf/metadata.json
fn metadata_path() -> Result<PathBuf> {
    Ok(conf_dir()?.join("metadata.json"))
}

/// 获取指定配置的 toml 文件路径: conf/<name>.toml
pub fn config_toml_path(name: &str) -> Result<PathBuf> {
    Ok(conf_dir()?.join(format!("{}.toml", name)))
}

/// 获取 frpc.exe 路径: bin/frpc.exe
pub fn frpc_exe_path() -> Result<PathBuf> {
    Ok(bin_dir()?.join("frpc.exe"))
}

/// 加载所有配置元数据
pub fn load_configs() -> Result<Vec<FrpcConfigMeta>> {
    let path = metadata_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path).context("无法读取 metadata.json")?;
    let store: ConfigStore = serde_json::from_str(&content).context("无法解析 metadata.json")?;
    Ok(store.configs)
}

/// 保存所有配置元数据
fn save_configs(configs: &[FrpcConfigMeta]) -> Result<()> {
    let dir = conf_dir()?;
    fs::create_dir_all(&dir).context("无法创建 conf 目录")?;
    let path = dir.join("metadata.json");
    let store = ConfigStore {
        configs: configs.to_vec(),
    };
    let content = serde_json::to_string_pretty(&store).context("无法序列化配置")?;
    fs::write(&path, content).context("无法写入 metadata.json")?;
    Ok(())
}

/// 添加或更新一个配置
///
/// - `name`: 配置名称
/// - `toml_content`: frpc.toml 的内容
/// - `auto_start`: 是否开机自启
pub fn save_config(name: &str, toml_content: &str, auto_start: bool) -> Result<()> {
    // 1. 写入 toml 文件
    let dir = conf_dir()?;
    fs::create_dir_all(&dir).context("无法创建 conf 目录")?;
    let toml_path = dir.join(format!("{}.toml", name));
    fs::write(&toml_path, toml_content).context("无法写入配置文件")?;

    // 2. 更新元数据
    let mut configs = load_configs().unwrap_or_default();
    if let Some(existing) = configs.iter_mut().find(|c| c.name == name) {
        existing.auto_start = auto_start;
    } else {
        configs.push(FrpcConfigMeta {
            name: name.to_string(),
            auto_start,
        });
    }
    save_configs(&configs)?;
    log::info!("配置 '{}' 已保存", name);
    Ok(())
}

/// 删除一个配置
pub fn delete_config(name: &str) -> Result<()> {
    // 1. 删除 toml 文件
    let toml_path = config_toml_path(name)?;
    if toml_path.exists() {
        fs::remove_file(&toml_path).context("无法删除配置文件")?;
    }

    // 2. 从元数据中移除
    let mut configs = load_configs().unwrap_or_default();
    configs.retain(|c| c.name != name);
    save_configs(&configs)?;
    log::info!("配置 '{}' 已删除", name);
    Ok(())
}

/// 读取指定配置的 toml 内容
pub fn read_config_content(name: &str) -> Result<String> {
    let path = config_toml_path(name)?;
    fs::read_to_string(&path).context(format!("无法读取配置文件 '{}.toml'", name))
}

/// 获取所有标记为自启动的配置
pub fn get_auto_start_configs() -> Result<Vec<FrpcConfigMeta>> {
    let configs = load_configs()?;
    Ok(configs.into_iter().filter(|c| c.auto_start).collect())
}

/// 检查指定名称的配置是否存在
pub fn config_exists(name: &str) -> bool {
    let configs = load_configs().unwrap_or_default();
    configs.iter().any(|c| c.name == name)
}
