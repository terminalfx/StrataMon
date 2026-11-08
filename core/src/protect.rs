//! A seal for the settings file, so the server keys don't sit on disk as plain text.
//!
//! On Windows the seal is DPAPI, the system's own "lock it to this user" call. The key that
//! opens it lives in the system's copy of this Windows account's secrets — not in the file and
//! not in the program — so a settings file copied to another PC or read by another account
//! opens nowhere. What it can't promise is safety from inside this very account: a program
//! running as you can ask the system the same question the app does. The seal carries a note
//! only StrataMon repeats, so that answer isn't one any curious program gets with a casual
//! call — but the note is inside the program for whoever goes looking, so it's a closed door,
//! not a locked one. What it does stop: the file read as it is, and other accounts on the PC.
//!
//! On Linux there's no system store every desktop ships, so the settings stay as they are,
//! with the file's owner-only permission the one protection. The file format carries a version
//! number either way, so a Linux seal can be added later without breaking saved files.

use std::io;
use std::path::Path;

/// The bytes that start a sealed settings file. A plain one is JSON, so it can't start with
/// these, and that's how the two are told apart without reading the whole file.
const MAGIC: &[u8] = b"SMONCFIG";

/// The one seal version there is: what follows is DPAPI's own output.
#[cfg(windows)]
const VERSION: u8 = 1;

/// The note that closes the seal. It never changes: a different note can't open files saved
/// with the old one, and the keys inside are the user's to keep.
#[cfg(windows)]
const NOTE: &[u8] = b"91c6d940-d454-4a9b-a839-4367e1b623a9";

/// The bytes to write for this settings text: sealed on Windows, the text itself elsewhere.
pub(crate) fn wrap(plain: &str) -> io::Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&dpapi::protect(plain.as_bytes())?);
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        Ok(plain.as_bytes().to_vec())
    }
}

/// The settings text a file holds. `Ok(None)` means the file is plain text, which is what
/// older builds and Linux write. `Err` means it's a seal this build can't open, and files
/// like that are kept aside rather than read or replaced.
pub(crate) fn unwrap(bytes: &[u8]) -> io::Result<Option<Vec<u8>>> {
    #[cfg(windows)]
    {
        let Some(rest) = bytes.strip_prefix(MAGIC) else {
            return Ok(None);
        };
        match rest.first().copied() {
            Some(VERSION) => dpapi::unprotect(&rest[1..]).map(Some),
            _ => Err(io::Error::other("the settings file was sealed by a newer StrataMon, which this one can't open")),
        }
    }
    #[cfg(not(windows))]
    {
        if bytes.starts_with(MAGIC) {
            return Err(io::Error::other("the settings file was sealed on Windows, and there's no system store here to open it with"));
        }
        Ok(None)
    }
}

/// Whether the file on disk is plain text that this build could seal. The app asks once a
/// run and re-saves when it's true, so keys stop being plain text without the user doing
/// anything. There's nothing to do where there's no seal to apply.
pub fn needs_sealing(path: &Path) -> bool {
    #[cfg(windows)]
    {
        std::fs::read(path).is_ok_and(|bytes| !bytes.starts_with(MAGIC))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

#[cfg(windows)]
mod dpapi {
    use std::io;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};

    use super::NOTE;

    pub(super) fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(plain);
        let note = blob(NOTE);
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };

        // UI_FORBIDDEN so the system never asks the user anything, and no description: these
        // bytes belong to this program alone, not to a person choosing to let them be read.
        let okay = unsafe {
            CryptProtectData(&input, std::ptr::null(), &note, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };

        if okay == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(output))
    }

    pub(super) fn unprotect(sealed: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(sealed);
        let note = blob(NOTE);
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };

        let okay = unsafe {
            CryptUnprotectData(&input, std::ptr::null_mut(), &note, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };

        if okay == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(output))
    }

    fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr().cast_mut() }
    }

    /// The system says it's holding the unsealed bytes: copy them out, then let go.
    fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if output.pbData.is_null() {
            return Vec::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe { LocalFree(output.pbData.cast()) };
        bytes
    }
}
