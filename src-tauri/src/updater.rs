//! In-app update checker and installer launcher via GitHub Releases.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::process::Command;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const GITHUB_LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/programinglive/dbpinball/releases/latest";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches('v');
    let core = trimmed.split('-').next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

fn pick_installer_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
    if cfg!(target_os = "windows") {
        assets
            .iter()
            .find(|a| a.name.ends_with("-setup.exe"))
            .or_else(|| assets.iter().find(|a| a.name.ends_with(".msi")))
    } else if cfg!(target_os = "macos") {
        assets.iter().find(|a| a.name.ends_with(".dmg"))
    } else {
        assets.iter().find(|a| a.name.ends_with(".AppImage"))
    }
}

fn curl_bin() -> &'static str {
    if cfg!(target_os = "windows") {
        "curl.exe"
    } else {
        "curl"
    }
}

fn apply_no_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

pub async fn check_for_update() -> Result<UpdateCheckResult, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    let mut cmd = Command::new(curl_bin());
    cmd.args([
        "-fsSL",
        "--max-time",
        "10",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "User-Agent: dbpinball-desktop",
        GITHUB_LATEST_RELEASE_API,
    ]);
    apply_no_window(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run update check: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("404") {
            return Ok(UpdateCheckResult {
                current_version: current.clone(),
                latest_version: current,
                update_available: false,
                release_url: "https://github.com/programinglive/dbpinball/releases".to_string(),
                download_url: None,
                asset_name: None,
            });
        }
        return Err(format!("Update check failed: {}", stderr.trim()));
    }

    let release: GitHubRelease = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid release metadata: {e}"))?;

    let latest = release.tag_name.trim_start_matches('v').to_string();
    let update_available = is_newer(&latest, &current);
    let asset = pick_installer_asset(&release.assets);

    Ok(UpdateCheckResult {
        current_version: current,
        latest_version: latest,
        update_available,
        release_url: release.html_url,
        download_url: asset.map(|a| a.browser_download_url.clone()),
        asset_name: asset.map(|a| a.name.clone()),
    })
}

pub async fn install_update(
    app: tauri::AppHandle,
    download_url: String,
    asset_name: String,
) -> Result<(), String> {
    if !download_url.starts_with("https://github.com/programinglive/dbpinball/releases/download/")
        && !download_url.starts_with("https://objects.githubusercontent.com/")
    {
        return Err("Refusing untrusted update download URL".to_string());
    }

    let safe_name = PathBuf::from(&asset_name)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid installer filename".to_string())?
        .to_string();

    let update_dir = std::env::temp_dir().join("dbpinball-update");
    tokio::fs::create_dir_all(&update_dir)
        .await
        .map_err(|e| format!("Failed to create update directory: {e}"))?;
    let installer_path = update_dir.join(&safe_name);

    let mut cmd = Command::new(curl_bin());
    cmd.args([
        "-fL",
        "--max-time",
        "120",
        "-H",
        "User-Agent: dbpinball-desktop",
        "-o",
        installer_path.to_str().unwrap_or_default(),
        &download_url,
    ]);
    apply_no_window(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to download update: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Failed to download update: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    std::process::Command::new(&installer_path)
        .spawn()
        .map_err(|e| format!("Failed to launch installer: {e}"))?;

    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_detects_newer_versions() {
        assert!(is_newer("0.1.8", "0.1.7"));
        assert!(is_newer("v0.2.0", "0.1.7"));
        assert!(!is_newer("0.1.7", "0.1.7"));
        assert!(!is_newer("0.1.6", "0.1.7"));
    }
}
