//! 日志配置与清理，按天存储日志并自动清理超过 30 天的日志文件

use anyhow::{Context, Result};
use chrono::Local;
use log::LevelFilter;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::thread;

const LOG_PATTERN: &str = "{d(%Y-%m-%d %H:%M:%S)} [{l}] {m}{n}";

/// 初始化日志系统，并启动后台线程在每天零点自动切换日志文件
pub fn init_logging() -> Result<()> {
    let exe_path = env::current_exe().context("无法获取可执行文件路径")?;
    let exe_dir = exe_path.parent().context("无法获取可执行文件目录")?;

    let logs_dir = exe_dir.join("logs");
    fs::create_dir_all(&logs_dir).context("无法创建日志目录")?;

    // 构建今天的日志配置
    let config = build_log_config(&logs_dir)?;

    let handle = log4rs::init_config(config).context("无法初始化日志")?;

    // 确认日志文件已创建并写入首条记录
    log::info!("日志系统初始化完成，日志目录: {:?}", logs_dir);

    // 首次启动时清理超过 30 天的旧日志
    let _ = clean_old_logs(&logs_dir);

    // 启动后台线程：在每天零点切换到新的日志文件并清理过期日志
    let handle_clone = handle.clone();
    thread::spawn(move || {
        log_rotation_loop(handle_clone, &logs_dir);
    });

    Ok(())
}

/// 构建指向当天日志文件的 Config
fn build_log_config(logs_dir: &Path) -> Result<Config> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let log_file = logs_dir.join(format!("{}.log", today));

    // 预创建日志文件，确保可写
    if !log_file.exists() {
        let mut f = fs::File::create(&log_file).context("无法创建日志文件")?;
        f.write_all(b"").context("无法写入日志文件")?;
        f.flush().ok();
    }

    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(LOG_PATTERN)))
        .build(&log_file)
        .context("无法打开日志文件")?;

    Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(Root::builder().appender("logfile").build(LevelFilter::Info))
        .context("无法构建日志配置")
}

/// 后台日志轮转循环：
/// - 每 30 秒检查一次当天日志文件是否存在，若被外部删除则重建
/// - 日期变化时切换到新的日志文件并清理过期日志
fn log_rotation_loop(handle: log4rs::Handle, logs_dir: &Path) {
    let mut last_date = Local::now().format("%Y-%m-%d").to_string();

    loop {
        thread::sleep(std::time::Duration::from_secs(30));

        let today = Local::now().format("%Y-%m-%d").to_string();
        let log_file = logs_dir.join(format!("{}.log", today));

        // 日期变化：轮转 + 清理旧日志
        let date_changed = today != last_date;
        // 日志文件被外部删除：需要重建
        let file_deleted = !log_file.exists();

        if date_changed || file_deleted {
            match build_log_config(logs_dir) {
                Ok(new_config) => {
                    handle.set_config(new_config);
                    if date_changed {
                        log::info!("日志文件已切换到 {}", today);
                        let _ = clean_old_logs(logs_dir);
                        last_date = today;
                    } else {
                        log::warn!("日志文件被外部删除，已重新创建: {:?}", log_file);
                    }
                }
                Err(e) => {
                    eprintln!("日志配置重建失败: {:?}", e);
                }
            }
        }
    }
}

/// 清理超过 30 天的日志文件（按文件名中的日期判断，格式 YYYY-MM-DD.log）
fn clean_old_logs(logs_dir: &Path) -> Result<()> {
    let cutoff = (Local::now() - chrono::Duration::days(30)).date_naive();

    let entries = fs::read_dir(logs_dir).context("无法列出日志目录")?;

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // 只处理 YYYY-MM-DD.log 格式的文件
        let date_str = match name.strip_suffix(".log") {
            Some(s) => s,
            None => continue,
        };

        let file_date = match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => continue,
        };

        if file_date < cutoff {
            if let Err(e) = fs::remove_file(entry.path()) {
                eprintln!("删除旧日志 {:?} 失败: {}", entry.path(), e);
            }
        }
    }

    Ok(())
}
