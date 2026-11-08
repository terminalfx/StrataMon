//! The saved settings. The file format matches the earlier C# builds, so keys already saved keep working.

use crate::protect;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub id: String,
    pub name: String,

    /// Tried in order until one answers, for example a LAN address then a Tailscale one.
    pub addresses: Vec<String>,

    /// Strata's API key or read-only telemetry key. Blank for a server that has neither.
    pub key: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { id: new_id(), name: String::new(), addresses: Vec::new(), key: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlertThresholds {
    #[serde(rename = "gpuTemperatureC")]
    pub gpu_temperature_c: f64,
    pub ram_percent: f64,

    /// How long requests can sit in the queue before alerting.
    pub queue_wait_seconds: i32,

    /// How long a reading must stay over its limit before alerting, so brief spikes don't.
    pub sustain_seconds: i32,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self { gpu_temperature_c: 85.0, ram_percent: 95.0, queue_wait_seconds: 60, sustain_seconds: 60 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppConfig {
    pub servers: Vec<ServerConfig>,
    pub alerts: AlertThresholds,
    pub poll_seconds: u32,
    pub desktop_notifications: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { servers: Vec::new(), alerts: AlertThresholds::default(), poll_seconds: 3, desktop_notifications: true }
    }
}

fn settings_root() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };

    // Unusual, but when the environment names no folder at all, settings go next to the program.
    // Falling back to a bare relative path would instead follow whichever folder the process was
    // started from, so every run could read and write a different file.
    base.or_else(|| std::env::current_exe().ok().and_then(|exe| exe.parent().map(|folder| folder.to_path_buf())))
        .unwrap_or_else(std::env::temp_dir)
}

/// Where settings live unless `--config` says otherwise.
pub fn default_path() -> PathBuf {
    settings_root().join("StrataMon").join("config.json")
}

/// Where settings lived under the app's previous name.
pub fn old_default_path() -> PathBuf {
    settings_root().join("StrataWatch").join("config.json")
}

/// Copies settings saved under the app's old name, because they hold the user's server keys.
/// This only happens while there's no new file, and the old file stays where it is.
pub fn adopt_old_settings(old: &Path, new: &Path) -> std::io::Result<()> {
    if new.exists() || !old.exists() {
        return Ok(());
    }

    if let Some(folder) = new.parent() {
        std::fs::create_dir_all(folder)?;
    }

    // Copied as raw bytes, so a setting this build doesn't know about survives. Renamed into
    // place so that a failed copy can't look like a corrupt settings file on the next launch.
    let temp = PathBuf::from(format!("{}.tmp", new.display()));
    let copied = std::fs::copy(old, &temp).and_then(|_| restrict_to_owner(&temp)).and_then(|()| std::fs::rename(&temp, new));
    if copied.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    copied
}

/// Loads the config, or defaults if there isn't one. An unreadable file is moved aside rather
/// than overwritten, because it holds the user's server keys, and its new path is returned.
/// The same happens to a sealed file this build can't open, so those bytes are never lost.
pub fn load(path: &Path) -> (AppConfig, Option<PathBuf>) {
    let Ok(bytes) = std::fs::read(path) else {
        return (AppConfig::default(), None);
    };

    let text = match protect::unwrap(&bytes) {
        Err(_) => return (AppConfig::default(), move_aside(path)),
        Ok(Some(sealed)) => sealed,
        // Plain text, which is what older builds wrote and what Linux still reads.
        Ok(None) if std::str::from_utf8(&bytes).is_ok() => bytes,
        // Not text at all, which is treated as no file at all, as before.
        // Not text at all: still no settings, but renamed out of the way like any other
        // unreadable file, so the next save can't overwrite those bytes without a copy.
        Ok(None) => return (AppConfig::default(), move_aside(path)),
    };

    match serde_json::from_slice(&text) {
        Ok(config) => (config, None),
        Err(_) => (AppConfig::default(), move_aside(path)),
    }
}

fn move_aside(path: &Path) -> Option<PathBuf> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let base = format!("{}.corrupt-{seconds}", path.display());
    // Two unreadable settings files in the same second must not overwrite each other's kept copy.
    let mut counter = 0;
    let moved = loop {
        let candidate = if counter == 0 { PathBuf::from(&base) } else { PathBuf::from(format!("{base}.{counter}")) };
        if !candidate.exists() {
            break candidate;
        }

        counter += 1;
        if counter > 20 {
            return None;
        }
    };
    // A read-only folder can block the rename, which would leave the unreadable file exactly
    // where a later save would replace it. Copying keeps a readable copy of the user's keys.
    if std::fs::rename(path, &moved).is_ok() {
        return Some(moved);
    }

    std::fs::copy(path, &moved).ok().map(|_| moved)
}

/// Whether the file on disk is plain text this build could seal. The app re-saves once a run
/// when it's true, so keys an older build left plain stop being plain text by themselves.
pub fn needs_sealing(path: &Path) -> bool {
    protect::needs_sealing(path)
}

pub fn save(path: &Path, config: &AppConfig) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }

    // Write then rename, so a crash mid-save can't leave a half-written file. If anything
    // short of the rename fails, the temp is removed rather than left as {name}.json.tmp,
    // holding the user's sealed keys next to the real file indefinitely.
    let temp = PathBuf::from(format!("{}.tmp", path.display()));
    let text = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
    let written = write_restricted(&temp, &protect::wrap(&text)?)
        .and_then(|()| restrict_to_owner(&temp))
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }

    written
}

/// On Unix, creates the file readable only by its owner from the first byte. Without this
/// there's a short window where the sealed keys sit world-readable before the chmod lands.
#[cfg(unix)]
fn write_restricted(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_restricted(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_to_owner(_: &Path) -> std::io::Result<()> {
    Ok(())
}

fn new_id() -> String {
    let random = || RandomState::new().build_hasher().finish();
    format!("{:016x}{:016x}", random(), random())
}
