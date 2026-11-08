//! Runs only when the person presses Check for updates or Update now. Nothing calls this at launch,
//! on a timer or in the background, and it sends nothing beyond the request itself.

use crate::transport::tls;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use stratamon_core::update::{self, MAX_DOWNLOAD_BYTES, MAX_RELEASE_INFO_BYTES, MAX_SIGNATURE_BYTES, UpdateStatus, VERSION};
use ureq::Agent;
use ureq::config::Config;

const NOT_SET_UP: &str = "Updates aren't set up in this build.";
const CANT_WRITE: &str = "This folder can't be written to, so the update wasn't installed. Use the link to download it.";

/// Set while an update is renaming program files, so the leftovers thread started at launch can't
/// race it and delete the copy the swap just made as its rollback.
static SWAPPING: AtomicBool = AtomicBool::new(false);

/// Which release feed to trust and how to check it. A shipped build can only ever get the real
/// ones, because the test values below don't exist unless the `test-updates` feature is compiled in.
struct Source {
    repository: String,
    public_key: String,
    /// Only for tests, which use a local server instead of GitHub.
    rewrite: Option<(String, String)>,
}

#[cfg(not(feature = "test-updates"))]
fn source() -> Option<Source> {
    Some(Source { repository: update::REPOSITORY?.to_string(), public_key: update::PUBLIC_KEY?.to_string(), rewrite: None })
}

#[cfg(feature = "test-updates")]
fn source() -> Option<Source> {
    let server = std::env::var("STRATAMON_TEST_SERVER").ok()?;
    Some(Source {
        repository: "owner/StrataMon".to_string(),
        public_key: std::env::var("STRATAMON_TEST_KEY").ok()?,
        rewrite: Some(("https://".to_string(), server)),
    })
}

impl Source {
    fn url(&self, url: String) -> String {
        match &self.rewrite {
            Some((from, to)) => url.replacen(from, to, 1),
            None => url,
        }
    }
}

fn agent(source: &Source) -> Agent {
    Config::builder()
        .https_only(source.rewrite.is_none())
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .user_agent("StrataMon")
        .tls_config(tls())
        .build()
        .into()
}

fn get(agent: &Agent, url: &str, limit: usize) -> Result<(u16, Vec<u8>), String> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| format!("GitHub couldn't be reached. {error}"))?;

    let status = response.status().as_u16();
    let mut body = Vec::new();
    let room = limit as u64 + 1;
    response
        .body_mut()
        .with_config()
        .limit(room)
        .reader()
        .take(room)
        .read_to_end(&mut body)
        .map_err(|_| "The download stopped before it finished.".to_string())?;

    if body.len() > limit {
        return Err("The download was bigger than expected, so it was stopped.".to_string());
    }

    Ok((status, body))
}

pub fn check() -> UpdateStatus {
    let Some(source) = source() else {
        return UpdateStatus::Failed(NOT_SET_UP.to_string());
    };

    let url = source.url(update::latest_release_url(&source.repository));
    match get(&agent(&source), &url, MAX_RELEASE_INFO_BYTES) {
        Ok((status, body)) => update::interpret_release(status, &body, VERSION),
        Err(message) => UpdateStatus::Failed(message),
    }
}

/// Downloads, verifies and swaps in a release, then starts it. The caller then closes this copy.
pub fn install(tag: &str) -> Result<(), String> {
    let source = source().ok_or(NOT_SET_UP)?;
    update::parse_version(tag).ok_or("That release's version can't be read.")?;

    let (file_url, signature_url) = update::download_urls(&source.repository, tag);
    let agent = agent(&source);

    let (status, file) = get(&agent, &source.url(file_url), MAX_DOWNLOAD_BYTES)?;
    if status != 200 {
        return Err(format!("The download failed (HTTP {status})."));
    }

    let (status, signature) = get(&agent, &source.url(signature_url), MAX_SIGNATURE_BYTES)?;
    if status != 200 {
        return Err(format!("The update's signature couldn't be downloaded (HTTP {status})."));
    }

    let signature = String::from_utf8(signature).map_err(|_| "The update's signature can't be read.")?;

    // Nothing on disk changes until this passes.
    update::verify_download(&file, &signature, &source.public_key, VERSION, tag)?;

    let current = std::env::current_exe().map_err(|_| "This program's own location couldn't be found.")?;

    // Hold the guard from the first rename until the restart is settled: the swap is about to make
    // a .old copy that is the only way back if the new build won't start.
    SWAPPING.store(true, Ordering::SeqCst);

    if let Err(problem) = swap_in(&current, &file) {
        SWAPPING.store(false, Ordering::SeqCst);
        return Err(problem);
    }

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let started = std::process::Command::new(&current).args(arguments).spawn();

    if let Err(error) = started {
        let put_back = restore(&current);
        SWAPPING.store(false, Ordering::SeqCst);
        // Only promise the old one is back when it really is.
        return Err(if put_back {
            format!("The new version wouldn't start, so the old one was put back. {error}")
        } else {
            format!("The new version wouldn't start, and the old one couldn't be put back. {error} Use the link to download it.")
        });
    }

    SWAPPING.store(false, Ordering::SeqCst);
    Ok(())
}

fn old_copy(current: &Path) -> PathBuf {
    current.with_extension("old")
}

fn new_copy(current: &Path) -> PathBuf {
    current.with_extension("new")
}

/// Replaces a program file, even one that's running. A running exe can't be overwritten on Windows
/// but it can be renamed, so it steps aside first. Any failure leaves the original in place.
fn swap_in(current: &Path, contents: &[u8]) -> Result<(), String> {
    let fresh = new_copy(current);
    // Nothing is renamed yet, but a half-written new copy mustn't linger either.
    if fs::write(&fresh, contents).and_then(|()| mark_executable(&fresh)).is_err() {
        let _ = fs::remove_file(&fresh);
        return Err(CANT_WRITE.to_string());
    }

    let old = old_copy(current);
    let _ = fs::remove_file(&old);
    if fs::rename(current, &old).is_err() {
        let _ = fs::remove_file(&fresh);
        return Err(CANT_WRITE.to_string());
    }

    if fs::rename(&fresh, current).is_err() {
        let _ = fs::rename(&old, current);
        let _ = fs::remove_file(&fresh);
        return Err(CANT_WRITE.to_string());
    }

    Ok(())
}

/// Puts the pre-update copy back where the new one is. True if the old one is in its place again.
fn restore(current: &Path) -> bool {
    let old = old_copy(current);
    if !old.exists() {
        return false;
    }

    // A rename replaces the failed new copy in one step, so the folder is never without a
    // program file. Only if that's refused, say because something still has the new copy open,
    // is the new copy removed first. Should the rename then fail again, only the .old is left.
    if fs::rename(&old, current).is_ok() {
        return true;
    }

    if fs::remove_file(current).is_err() {
        return false;
    }

    fs::rename(&old, current).is_ok()
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn mark_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Removes the copies an update left behind. The previous program file is still closing when this
/// starts, so it keeps trying. If an update begins meanwhile its .old is a rollback copy, and this
/// leaves it alone.
pub fn clean_up_after_update() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };

    // A .new at launch is only ever from an update that failed before the swap; nothing else clears it.
    let _ = fs::remove_file(new_copy(&current));

    let old = old_copy(&current);
    if !old.exists() {
        return;
    }

    std::thread::spawn(move || {
        for _ in 0..50 {
            if SWAPPING.load(Ordering::SeqCst) {
                return;
            }
            if fs::remove_file(&old).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            let name = format!("stratamon-swap-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
            let path = std::env::temp_dir().join(name);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn program(&self) -> PathBuf {
            let path = self.0.join("StrataMon.exe");
            fs::write(&path, b"old version").unwrap();
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn swapping_replaces_the_file_and_keeps_the_old_one_aside() {
        let folder = Folder::new();
        let program = folder.program();

        swap_in(&program, b"new version").unwrap();

        assert_eq!(fs::read(&program).unwrap(), b"new version");
        assert_eq!(fs::read(old_copy(&program)).unwrap(), b"old version");
        assert!(!new_copy(&program).exists());
    }

    #[test]
    fn swapping_twice_replaces_an_old_copy_left_behind() {
        let folder = Folder::new();
        let program = folder.program();

        swap_in(&program, b"second").unwrap();
        swap_in(&program, b"third").unwrap();

        assert_eq!(fs::read(&program).unwrap(), b"third");
        assert_eq!(fs::read(old_copy(&program)).unwrap(), b"second");
    }

    #[test]
    fn restoring_puts_the_old_version_back() {
        let folder = Folder::new();
        let program = folder.program();

        swap_in(&program, b"new version").unwrap();
        assert!(restore(&program));

        assert_eq!(fs::read(&program).unwrap(), b"old version");
        assert!(!old_copy(&program).exists());
    }

    #[test]
    fn restoring_reports_when_there_is_nothing_to_put_back() {
        let folder = Folder::new();
        let program = folder.program();

        assert!(!restore(&program));
        assert_eq!(fs::read(&program).unwrap(), b"old version");
    }

    #[test]
    fn a_missing_program_fails_cleanly_and_leaves_nothing_behind() {
        let folder = Folder::new();
        let program = folder.0.join("gone.exe");

        assert!(swap_in(&program, b"new").is_err());
        assert!(!new_copy(&program).exists());
    }

    #[test]
    fn an_unwritable_place_fails_cleanly() {
        let folder = Folder::new();
        let program = folder.0.join("no-such-folder").join("StrataMon.exe");
        assert_eq!(swap_in(&program, b"new").unwrap_err(), CANT_WRITE);
    }

    // A file held by an exclusive handle stands in for something not letting the exe move: the swap
    // must fail and clear its .new without disturbing the file or leaving a stray .old.
    #[cfg(windows)]
    #[test]
    fn a_running_file_can_not_be_moved_and_the_swap_backs_off() {
        use std::os::windows::fs::OpenOptionsExt;
        let folder = Folder::new();
        let program = folder.program();
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&program).unwrap();

        let outcome = swap_in(&program, b"new");
        drop(held);

        assert_eq!(outcome.unwrap_err(), CANT_WRITE);
        assert_eq!(fs::read(&program).unwrap(), b"old version");
        assert!(!new_copy(&program).exists());
        assert!(!old_copy(&program).exists());
    }

    // A .new left by a crashed swap mustn't poison the next one: the fresh write replaces it.
    #[test]
    fn a_new_copy_left_by_a_crashed_swap_is_replaced_not_confused() {
        let folder = Folder::new();
        let program = folder.program();
        fs::write(new_copy(&program), b"left from a crashed swap").unwrap();

        swap_in(&program, b"new").unwrap();

        assert_eq!(fs::read(&program).unwrap(), b"new");
        assert_eq!(fs::read(old_copy(&program)).unwrap(), b"old version");
        assert!(!new_copy(&program).exists());
    }
}
