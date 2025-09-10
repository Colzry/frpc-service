//! 日志配置与清理，按天存储日志并清理超过一个月的日志

use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use chrono::{Local, Duration};
use fs_extra::dir::{ls, DirEntryAttr, DirEntryValue};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::env;
use log::LevelFilter;
use anyhow::{Result, Context};

pub fn init_logging() -> Result<log4rs::Handle> {
    // 获取当前可执行文件所在目录
    let exe_path = env::current_exe().context("无法获取可执行文件路径")?;
    let exe_dir = exe_path
        .parent()
        .context("无法获取可执行文件目录")?;

    // 创建 logs 目录
    let logs_dir = exe_dir.join("logs");
    fs::create_dir_all(&logs_dir).context("无法创建日志目录")?;

    // 配置按天生成的日志文件
    let today = Local::now().format("%Y-%m-%d").to_string();
    let log_file = logs_dir.join(format!("{}.log", today));
    let logfile = FileAppender::builder()
        // 【已修改】: 调整日志格式
        .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%d %H:%M:%S)} [{l}] {m}{n}")))
        .build(log_file)
        .context("无法创建日志文件")?;

    // 配置日志
    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(Root::builder().appender("logfile").build(LevelFilter::Info))
        .context("无法构建日志配置")?;

    let handle = log4rs::init_config(config).context("无法初始化日志")?;

    // 清理超过一个月的日志
    clean_old_logs(&logs_dir)?;

    Ok(handle)
}

/// 清理超过一个月的日志文件
fn clean_old_logs(logs_dir: &PathBuf) -> Result<()> {
    let one_month_ago = Local::now() - Duration::days(30);

    // 获取所有日志文件
    let mut entries = HashSet::new();
    entries.insert(DirEntryAttr::Path);
    let log_files = ls(logs_dir, &entries)
        .context("无法列出日志文件")?
        .items
        .into_iter()
        .filter(|item| {
            item.get(&DirEntryAttr::Path)
                .and_then(|path| match path {
                    DirEntryValue::String(s) => {
                        let path = PathBuf::from(s);
                        Some(path.is_file())
                    }
                    _ => None,
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    for item in log_files {
        let path = item
            .get(&DirEntryAttr::Path)
            .and_then(|p| match p {
                DirEntryValue::String(s) => Some(PathBuf::from(s)),
                _ => None,
            })
            .context("无法获取日志文件路径")?;

        // 从文件名中提取日期（格式：YYYY-MM-DD.log）
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if let Some(date_str) = file_name.strip_suffix(".log") {
                if let Ok(file_date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    if file_date < one_month_ago.naive_local().date() {
                        fs::remove_file(&path).context(format!("无法删除旧日志: {:?}", path))?;
                        log::info!("已删除旧日志: {:?}", path);
                    }
                }
            }
        }
    }

    Ok(())
}
