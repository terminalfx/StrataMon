//! Makes the release signing key and signs releases. This is a developer tool and is never shipped.
//!
//!     stratamon-sign keygen <folder>
//!     stratamon-sign sign <secret key file> <file> <version> <asset name>
//!
//! The private key is what lets a release update people's copies of StrataMon, so it stays on the
//! PC that made it and never goes in the repository or on GitHub.

use minisign::{KeyPair, SecretKey, sign};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

const SECRET_FILE: &str = "stratamon-update.key";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();

    let result = match arguments.as_slice() {
        ["keygen", folder] => keygen(Path::new(folder)),
        ["sign", key, file, version, asset] => sign_file(Path::new(key), Path::new(file), version, asset),
        _ => Err("Usage:\n  stratamon-sign keygen <folder>\n  stratamon-sign sign <secret key file> <file> <version> <asset name>".to_string()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

// Tests can't answer a prompt, so they supply the password. People get prompted, with no echo.
// Anything running as this user can read that environment value, so set it on the same command
// line that runs the tool, the way scripts/sign-release.ps1 does.
fn password() -> Option<String> {
    std::env::var("STRATAMON_SIGN_PASSWORD").ok()
}

fn keygen(folder: &Path) -> Result<(), String> {
    let path = folder.join(SECRET_FILE);

    // Replacing a key would strand every installed copy, which trusts only the old one.
    if path.exists() {
        return Err(format!("{} already exists, and it won't be overwritten.", path.display()));
    }

    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let pair = KeyPair::generate_encrypted_keypair(password()).map_err(|error| error.to_string())?;
    let secret = pair.sk.to_box(None).map_err(|error| error.to_string())?.into_string();

    let mut file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|error| error.to_string())?;

    // Restrict before the secret is in the file, and don't make a key at all if it can't be kept private.
    if let Err(problem) = restrict_to_owner(&file, &path) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(format!("The key file couldn't be restricted to its owner. {problem}"));
    }

    file.write_all(secret.as_bytes()).map_err(|error| error.to_string())?;

    println!("Private key: {}", path.display());
    println!("Back this file up and keep it private. Without it, you can't update existing copies.");
    println!("Public key, for PUBLIC_KEY in core/src/update.rs:");
    println!("{}", pair.pk.to_base64());
    Ok(())
}

#[cfg(unix)]
fn restrict_to_owner(file: &File, _: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())
}

/// A new file inherits who may read the folder it's in, and a drive root often says everyone who
/// can sign in. Strip the inheritance and leave just this user, the way a profile folder is set up.
#[cfg(not(unix))]
fn restrict_to_owner(_: &File, path: &Path) -> Result<(), String> {
    let user = std::env::var("USERNAME").map_err(|_| "can't tell which user to grant the file to".to_string())?;

    let outcome = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{user}:F"))
        .output()
        .map_err(|error| format!("couldn't run icacls: {error}"))?;

    if outcome.status.success() {
        return Ok(());
    }

    let complaint = String::from_utf8_lossy(&outcome.stderr).trim().to_string();
    Err(format!("icacls wouldn't restrict the file. {complaint}"))
}

fn sign_file(key: &Path, file: &Path, version: &str, asset: &str) -> Result<(), String> {
    let secret = SecretKey::from_file(key, password()).map_err(|error| format!("The key couldn't be opened. {error}"))?;
    let data = File::open(file).map_err(|error| format!("{}: {error}", file.display()))?;

    // The app reads these, and this comment is covered by the signature.
    let comment = format!("version:{version} file:{asset}");
    let signature = sign(None, &secret, data, Some(&comment), None).map_err(|error| error.to_string())?;

    let target = format!("{}.minisig", file.display());
    fs::write(&target, signature.into_string()).map_err(|error| error.to_string())?;
    println!("Signed: {target}");
    Ok(())
}
