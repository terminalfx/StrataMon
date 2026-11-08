//! Checking for a newer StrataMon. This only ever runs when the person clicks the button. Nothing
//! here happens at launch, on a timer or in the background, and there's no telemetry of any kind.

use minisign_verify::{PublicKey, Signature};
use serde_json::Value;

/// The GitHub repository releases come from, as `owner/name`. Until it's set, the button says
/// updates aren't set up in this build.
pub const REPOSITORY: Option<&str> = None;

/// The public half of the release signing key. The private half never leaves the PC it was made
/// on, so someone who takes over the GitHub account still can't push code to anyone. Until it's
/// set, the button says updates aren't set up in this build.
pub const PUBLIC_KEY: Option<&str> = None;

/// Far bigger than the app, and small enough that a bad reply can't fill memory.
pub const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_SIGNATURE_BYTES: usize = 4096;
pub const MAX_RELEASE_INFO_BYTES: usize = 1024 * 1024;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The name of this platform's download in each release.
pub const ASSET: &str = if cfg!(windows) { "StrataMon-win-x64.exe" } else { "StrataMon-linux-x64" };

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available {
        version: String,
        tag: String,
        link: String,
    },
    Installing,
    Failed(String),

    /// An install didn't happen. The release is still out there, so the link stays on offer.
    InstallFailed {
        message: String,
        link: String,
    },
}

/// Reads a tag shaped exactly like `v1.2.3`. Anything else isn't a release of ours.
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut next = || {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<u32>().ok()
    };

    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// What a reply from GitHub's latest-release address means for this build.
pub fn interpret_release(status: u16, body: &[u8], current: &str) -> UpdateStatus {
    match status {
        200 => {}
        404 => return UpdateStatus::Failed("No releases have been published yet.".to_string()),
        403 | 429 => return UpdateStatus::Failed("GitHub is limiting requests right now. Try again later.".to_string()),
        other => return UpdateStatus::Failed(format!("GitHub couldn't answer (HTTP {other}).")),
    }

    let Some(release) = serde_json::from_slice::<Value>(body).ok() else {
        return UpdateStatus::Failed("GitHub's reply couldn't be read.".to_string());
    };

    let tag = release.get("tag_name").and_then(Value::as_str).unwrap_or("");
    let (Some(latest), Some(running)) = (parse_version(tag), parse_version(current)) else {
        return UpdateStatus::Failed("The latest release has a version this build can't read.".to_string());
    };

    if latest <= running {
        return UpdateStatus::UpToDate;
    }

    UpdateStatus::Available {
        version: format!("{}.{}.{}", latest.0, latest.1, latest.2),
        tag: tag.to_string(),
        link: release_link(tag),
    }
}

/// The repo's page for a release. Built from known parts, not from whatever a reply says.
pub fn release_link(tag: &str) -> String {
    format!("https://github.com/{}/releases/tag/{tag}", REPOSITORY.unwrap_or_default())
}

pub fn latest_release_url(repository: &str) -> String {
    format!("https://api.github.com/repos/{repository}/releases/latest")
}

/// Where this platform's file lives for a release, followed by its signature's address.
pub fn download_urls(repository: &str, tag: &str) -> (String, String) {
    let file = format!("https://github.com/{repository}/releases/download/{tag}/{ASSET}");
    let signature = format!("{file}.minisig");
    (file, signature)
}

/// Decides whether a download may replace the running program. Anything but a clean pass means no.
pub fn verify_download(file: &[u8], signature: &str, public_key: &str, current: &str, expected_tag: &str) -> Result<(), String> {
    let key = PublicKey::from_base64(public_key).map_err(|_| "This build's update key can't be read.".to_string())?;
    let signature = Signature::decode(signature).map_err(|_| "The update's signature can't be read.".to_string())?;

    // Legacy mode stays off. The check also covers the signed comment, not only the file.
    key.verify(file, &signature, false).map_err(|_| "The update's signature doesn't match, so it wasn't installed.".to_string())?;
    check_signed_comment(signature.trusted_comment(), current, expected_tag)
}

/// Checks the text inside a release's signature, which the signature itself protects. It has to
/// name this platform's file and a version newer than the one running, so an old signed release
/// or the other platform's file can't be swapped in.
pub fn check_signed_comment(comment: &str, current: &str, expected_tag: &str) -> Result<(), String> {
    let mut version = None;
    let mut file = None;
    for part in comment.split_whitespace() {
        match part.split_once(':') {
            Some(("version", value)) => version = Some(value),
            Some(("file", value)) => file = Some(value),
            _ => {}
        }
    }

    if file != Some(ASSET) {
        return Err("This download is for a different platform.".to_string());
    }

    let (Some(signed), Some(running)) = (version.and_then(parse_version), parse_version(current)) else {
        return Err("This download's version couldn't be read.".to_string());
    };

    if signed <= running {
        return Err("This download isn't newer than the running version.".to_string());
    }

    if parse_version(expected_tag) != Some(signed) {
        return Err("This download doesn't match the release it came from.".to_string());
    }

    Ok(())
}
