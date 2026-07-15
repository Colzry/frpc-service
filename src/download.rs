//! frpc 下载与解压模块

use anyhow::{Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// GitHub 代理地址列表，按优先级排序
const PROXY_URLS: &[&str] = &[
    "https://gitpy.223327.xyz/",
    "https://gh-proxy.com/",
    "https://github.catvod.com/",
    "https://gh.xxooo.cf/",
    "https://gh.llkk.cc/",
    "https://github.ednovas.xyz/",
    "https://gitdl.cn/",
    "https://cf.ghproxy.cc/",
    "https://ghproxy.net/",
    "https://gh.aptv.app/",
    "https://gitpr.xmcom.us.kg/",
    "https://ghproxy.cn/",
];

/// 检查目录下是否存在 frpc.exe 或 frpc@*.exe
pub fn has_frpc_executable(exe_dir: &Path) -> bool {
    if exe_dir.join("frpc.exe").exists() {
        return true;
    }
    if let Ok(entries) = fs::read_dir(exe_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("frpc@") && name.ends_with(".exe") {
                    return true;
                }
            }
        }
    }
    false
}

/// 获取最新 release 版本号（如 "v0.70.0"）
fn get_latest_release_tag(client: &reqwest::blocking::Client) -> Result<String> {
    let url = "https://api.github.com/repos/fatedier/frp/releases/latest";
    let resp = client
        .get(url)
        .header("User-Agent", "frpc-service")
        .send()
        .context("无法获取最新版本信息")?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "获取最新版本信息失败: HTTP {}",
            resp.status()
        ));
    }

    let json: serde_json::Value = resp.json().context("解析版本信息失败")?;
    let tag = json["tag_name"]
        .as_str()
        .context("无法从 API 响应中获取 tag_name")?;

    Ok(tag.to_string())
}

/// 带进度回调的下载
fn download_with_progress(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    on_progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<()> {
    let resp = client
        .get(url)
        .header("User-Agent", "frpc-service")
        .send()
        .context("无法发起下载请求")?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("下载失败: HTTP {}", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(0);
    let mut file = fs::File::create(dest).context("无法创建临时文件")?;
    let mut downloaded: u64 = 0;
    let mut buffer = [0u8; 8192];
    let mut reader = resp;

    on_progress(0, total_size);

    loop {
        let bytes_read = reader.read(&mut buffer).context("读取下载数据失败")?;
        if bytes_read == 0 {
            break;
        }
        file.write_all(&buffer[..bytes_read])
            .context("写入文件失败")?;
        downloaded += bytes_read as u64;
        on_progress(downloaded, total_size);
    }

    file.flush()?;
    Ok(())
}

/// 需要从 zip 中提取的文件名
const EXTRACT_FILES: &[&str] = &["frpc.exe", "frpc.toml"];

/// 从 zip 文件中提取 frpc.exe 和 frpc.toml 到目标目录
fn extract_frpc_from_zip(zip_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = fs::File::open(zip_path).context("无法打开下载的 zip 文件")?;
    let mut archive = zip::ZipArchive::new(file).context("无法解析 zip 文件")?;

    let mut found_exe = false;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).context("无法读取 zip 条目")?;
        let entry_name = entry.mangled_name();

        let file_name = match entry_name.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        // 只提取 frpc.exe 和 frpc.toml（可能在子目录中，如 frp_0.70.0_windows_amd64/）
        if EXTRACT_FILES.contains(&file_name.as_str()) {
            let out_path = dest_dir.join(&*file_name);
            let mut out_file =
                fs::File::create(&out_path).context(format!("无法创建 {}", file_name))?;
            std::io::copy(&mut entry, &mut out_file).context(format!("解压 {} 失败", file_name))?;
            log::info!("已将 {} 解压到 {:?}", file_name, out_path);
            if file_name == "frpc.exe" {
                found_exe = true;
            }
        }
    }

    if found_exe {
        Ok(())
    } else {
        Err(anyhow::anyhow!("下载的 zip 文件中未找到 frpc.exe"))
    }
}

/// 主入口：下载并解压 frpc.exe 到目标目录
///
/// 会依次尝试：原始 GitHub 地址 → 各代理地址
/// 进度通过 `on_progress(downloaded_bytes, total_bytes)` 回调报告
pub fn download_and_extract_frpc(
    dest_dir: &Path,
    on_progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("创建 HTTP 客户端失败")?;

    // 1. 获取最新版本号
    let tag = get_latest_release_tag(&client)?;
    log::info!("获取到最新 frp 版本: {}", tag);

    let file_name = format!("frp_{}_windows_amd64.zip", tag.trim_start_matches('v'));
    let github_url = format!(
        "https://github.com/fatedier/frp/releases/download/{}/{}",
        tag, file_name
    );

    // 2. 构建候选 URL 列表（原始地址 + 代理地址）
    let mut candidate_urls = Vec::new();
    candidate_urls.push(github_url.clone());
    for proxy in PROXY_URLS {
        candidate_urls.push(format!("{}{}", proxy, github_url));
    }

    // 3. 依次尝试下载
    let zip_path = dest_dir.join("__frpc_download_temp.zip");
    let mut last_error = String::new();

    for url in &candidate_urls {
        log::info!("尝试下载: {}", url);
        match download_with_progress(&client, url, &zip_path, on_progress) {
            Ok(()) => {
                log::info!("下载成功: {}", url);
                // 解压
                match extract_frpc_from_zip(&zip_path, dest_dir) {
                    Ok(()) => {
                        let _ = fs::remove_file(&zip_path);
                        return Ok(());
                    }
                    Err(e) => {
                        let _ = fs::remove_file(&zip_path);
                        last_error = format!("解压失败: {}", e);
                        log::warn!("解压失败 ({}): {}", url, e);
                        continue;
                    }
                }
            }
            Err(e) => {
                last_error = format!("下载失败: {}", e);
                log::warn!("下载失败 ({}): {}", url, e);
                let _ = fs::remove_file(&zip_path);
                continue;
            }
        }
    }

    Err(anyhow::anyhow!(
        "所有下载地址均失败，最后的错误: {}",
        last_error
    ))
}
