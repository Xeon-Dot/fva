//! Self-upgrade: fetch and install the latest FVA release.
//!
//! Reuses the official install scripts (`scripts/install.sh` /
//! `scripts/install.ps1`) so platform-specific download, checksum
//! verification and extraction logic stay in a single place.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{FvaError, Result};
use crate::util;

const REPO: &str = "Xeon-Dot/fva";
const API_LATEST: &str = "https://api.github.com/repos/Xeon-Dot/fva/releases/latest";

#[cfg(not(windows))]
const INSTALL_SHELL: (&str, &[&str], &str, &str) = ("sh", &["-c"], "curl -fsSL", "INSTALL_DIR");
#[cfg(windows)]
const INSTALL_SHELL: (&str, &[&str], &str, &str) = (
    "powershell",
    &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"],
    "irm",
    "FVA_INSTALL_DIR",
);

/// Run the upgrade flow.
///
/// * `target_version` — explicit release tag (e.g. `v0.2.0`); when `None`
///   the latest published release is used.
/// * `force` — reinstall even if the current version already matches.
pub fn run(target_version: Option<&str>, force: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("==> current version: v{current}");

    let target = match target_version {
        Some(v) => normalize_tag(v),
        None => {
            println!("==> checking latest release ({REPO})...");
            let latest = fetch_latest_tag()?;
            println!("==> latest version:  {latest}");
            if !force && strip_v(&latest) == strip_v(current) {
                println!("Already on the latest version (v{current}).");
                return Ok(());
            }
            latest
        }
    };

    let exe = current_exe()?;
    let install_dir = exe
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| FvaError::Upgrade("cannot determine install directory".into()))?;

    println!("==> upgrading to {target} in {}", install_dir.display());

    // On Windows a running executable cannot be overwritten, but it can be
    // renamed. Move it aside so the install script can write the new binary.
    let backup = stage_self_replace(&exe)?;

    let result = run_install_script(&target, &install_dir);

    match result {
        Ok(()) => {
            if let Some(b) = backup {
                let _ = std::fs::remove_file(&b);
            }
            println!("==> upgrade complete — run `fva version` to confirm.");
            Ok(())
        }
        Err(e) => {
            if let Some(b) = backup {
                restore_backup(&b, &exe);
            }
            Err(e)
        }
    }
}

fn normalize_tag(v: &str) -> String {
    let v = v.trim();
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

fn strip_v(v: &str) -> &str {
    v.trim().trim_start_matches('v')
}

fn current_exe() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| FvaError::Upgrade(format!("cannot locate current executable: {e}")))
}

fn fetch_latest_tag() -> Result<String> {
    let client = util::http_client(concat!("fva/", env!("CARGO_PKG_VERSION")))?;

    let resp = client
        .get(API_LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| FvaError::Upgrade(format!("fetch latest release: {e}")))?;

    if !resp.status().is_success() {
        return Err(FvaError::Upgrade(format!(
            "GitHub API returned status {}",
            resp.status()
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .map_err(|e| FvaError::Upgrade(format!("parse release response: {e}")))?;

    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| FvaError::Upgrade("release response missing tag_name".into()))
}

#[cfg(windows)]
fn stage_self_replace(exe: &Path) -> Result<Option<PathBuf>> {
    let backup = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&backup);
    std::fs::rename(exe, &backup)
        .map_err(|e| FvaError::Upgrade(format!("cannot move running executable aside: {e}")))?;
    Ok(Some(backup))
}

#[cfg(not(windows))]
fn stage_self_replace(_exe: &Path) -> Result<Option<PathBuf>> {
    // On Unix the running binary's inode can be replaced in place.
    Ok(None)
}

fn restore_backup(backup: &Path, exe: &Path) {
    if !exe.exists() {
        let _ = std::fs::rename(backup, exe);
    }
}

fn run_install_script(version: &str, install_dir: &Path) -> Result<()> {
    let (shell, args, fetch_cmd, dir_env) = INSTALL_SHELL;
    let script_url = if cfg!(windows) {
        "https://raw.githubusercontent.com/Xeon-Dot/fva/main/scripts/install.ps1"
    } else {
        "https://raw.githubusercontent.com/Xeon-Dot/fva/main/scripts/install.sh"
    };
    let pipe_cmd = if cfg!(windows) { "iex" } else { "bash" };
    let command = format!("{fetch_cmd} {script_url} | {pipe_cmd}");

    let status = Command::new(shell)
        .args(args)
        .arg(&command)
        .env("FVA_VERSION", version)
        .env(dir_env, install_dir)
        .env("FVA_REPO", REPO)
        .status()
        .map_err(|e| FvaError::Upgrade(format!("failed to launch installer: {e}")))?;

    if status.success() {
        Ok(())
    } else {
        Err(FvaError::Upgrade(format!(
            "installer exited with status {status}"
        )))
    }
}
