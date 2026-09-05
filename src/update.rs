//! Non-blocking update checker and updater for BUTION.

use crate::locale::text;
use anyhow::Result;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub html_url: String,
    pub download_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(serde::Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Parse semantic version numbers into (major, minor, patch).
pub fn parse_semver(version_str: &str) -> Option<(u64, u64, u64)> {
    let clean = version_str.trim().trim_start_matches(['v', 'V']);
    let parts: Vec<&str> = clean.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = parts[2]
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse::<u64>()
        .ok()?;
    Some((major, minor, patch))
}

/// Determine whether the remote version is strictly newer than current version.
pub fn is_newer(remote: &str, current: &str) -> bool {
    match (parse_semver(remote), parse_semver(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

/// Check the latest GitHub release. If offline or timed out, returns None immediately without failing.
pub async fn check_latest_release(repo: &str, timeout_secs: u64) -> Option<UpdateInfo> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent("bution-updater")
        .build()
        .ok()?;

    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let release: GitHubRelease = response.json().await.ok()?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    if is_newer(&release.tag_name, &current_version) {
        let expected_asset = target_asset_name();
        let download_url = release
            .assets
            .into_iter()
            .find(|asset| asset.name.contains(expected_asset))
            .map(|asset| asset.browser_download_url);

        Some(UpdateInfo {
            current_version,
            latest_version: release.tag_name,
            html_url: release.html_url,
            download_url,
        })
    } else {
        None
    }
}

/// Target asset name based on current OS and architecture.
pub fn target_asset_name() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "bution-macos-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "bution-macos-x64"
    }
    #[cfg(target_os = "windows")]
    {
        "bution-windows-x64"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "bution"
    }
}

/// Run an interactive CLI update check and trigger the installer if a new version is available.
pub async fn run_cli_update() -> Result<()> {
    println!(
        "[1/2] {}",
        text(
            "Checking for BUTION updates…",
            "Проверка обновлений BUTION…"
        )
    );
    let repo = "danytebyya/bution";
    match check_latest_release(repo, 6).await {
        Some(info) => {
            println!(
                "{}: {} ({}: v{})",
                text("New version", "Новая версия"),
                info.latest_version,
                text("installed", "установлена"),
                info.current_version
            );
            println!(
                "[2/2] {}",
                text(
                    "Updating BUTION components…",
                    "Обновление компонентов BUTION…"
                )
            );

            #[cfg(target_os = "windows")]
            {
                let status = std::process::Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-Command",
                        "$env:BUTION_FORCE_UPDATE='1'; irm https://raw.githubusercontent.com/danytebyya/bution/main/install.ps1 | iex",
                    ])
                    .status()?;
                if !status.success() {
                    anyhow::bail!(text(
                        "Windows update script failed.",
                        "Ошибка скрипта обновления Windows."
                    ));
                }
            }

            #[cfg(not(target_os = "windows"))]
            {
                let status = std::process::Command::new("bash")
                    .arg("-c")
                    .arg("curl -fsSL https://raw.githubusercontent.com/danytebyya/bution/main/install.sh | BUTION_FORCE_UPDATE=1 bash")
                    .status()?;
                if !status.success() {
                    anyhow::bail!(text(
                        "macOS update script failed.",
                        "Ошибка скрипта обновления macOS."
                    ));
                }
            }
            println!(
                "{}: {}",
                text("BUTION updated", "BUTION обновлён"),
                info.latest_version
            );
        }
        None => {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .user_agent("bution-updater")
                .build()?;
            let online = client.get("https://api.github.com").send().await.is_ok();
            if online {
                println!(
                    "{} (v{}).",
                    text("No update found", "Обновление не найдено"),
                    env!("CARGO_PKG_VERSION")
                );
            } else {
                println!(
                    "{}",
                    text(
                        "Offline. Update check skipped.",
                        "Нет интернета. Проверка обновлений пропущена."
                    )
                );
            }
        }
    }
    Ok(())
}

/// Automatically check and perform update on startup if online and a newer release exists.
/// Returns Ok(true) if updated and re-executed, or Ok(false) if no update needed / offline.
pub async fn auto_update_on_startup_if_needed() -> Result<bool> {
    let repo = "danytebyya/bution";
    // 5-second timeout ensures reliable check even over slower network connections
    let Some(info) = check_latest_release(repo, 5).await else {
        return Ok(false);
    };

    println!(
        "{}: {} ({}: v{})",
        text("New BUTION version", "Новая версия BUTION"),
        info.latest_version,
        text("installed", "установлена"),
        info.current_version
    );
    println!(
        "{}",
        text("Updating automatically…", "Автоматическое обновление…")
    );

    let update_success = {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    "$env:BUTION_FORCE_UPDATE='1'; irm https://raw.githubusercontent.com/danytebyya/bution/main/install.ps1 | iex",
                ])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }

        #[cfg(not(target_os = "windows"))]
        {
            std::process::Command::new("bash")
                .arg("-c")
                .arg("curl -fsSL https://raw.githubusercontent.com/danytebyya/bution/main/install.sh | BUTION_FORCE_UPDATE=1 bash")
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
    };

    if !update_success {
        println!(
            "{}",
            text(
                "Update failed. Starting the installed version…",
                "Не удалось обновить. Запуск установленной версии…"
            )
        );
        return Ok(false);
    }

    println!(
        "{}: {}. {}",
        text("BUTION updated", "BUTION обновлён"),
        info.latest_version,
        text("Starting…", "Запуск…")
    );

    // Collect original arguments, ensure --no-update-check is present to avoid loop
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--no-update-check") {
        args.push("--no-update-check".to_string());
    }

    let exe = std::env::current_exe()?;

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(exe).args(args).exec();
        eprintln!(
            "{}: {err}",
            text(
                "Could not restart BUTION",
                "Не удалось перезапустить BUTION"
            )
        );
        std::process::exit(1);
    }

    #[cfg(not(unix))]
    {
        let mut child = std::process::Command::new(exe).args(args).spawn()?;
        let status = child.wait()?;
        std::process::exit(status.code().unwrap_or(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_various_semver_strings() {
        assert_eq!(parse_semver("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_semver("v0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_semver("V1.20.3-beta"), Some((1, 20, 3)));
        assert_eq!(parse_semver("invalid"), None);
        assert_eq!(parse_semver("1.0"), None);
    }

    #[test]
    fn correctly_identifies_newer_versions() {
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.1"));
        assert!(!is_newer("invalid", "0.1.0"));
    }

    #[test]
    fn target_asset_name_is_non_empty() {
        assert!(!target_asset_name().is_empty());
    }
}
