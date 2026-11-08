//! The few things that differ between Windows and Linux.

#[cfg(windows)]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_INFO, NIF_TIP, NIIF_ERROR, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, ShellExecuteW,
        Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDI_APPLICATION, LoadIconW, SW_SHOWNORMAL};

    const ICON_ID: u32 = 1;

    /// What GetTimeZoneInformation returns during daylight saving time.
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }

    fn copy_into(target: &mut [u16], text: &str) {
        let limit = target.len() - 1;
        let units: Vec<u16> = text.encode_utf16().take(limit).collect();
        target[..units.len()].copy_from_slice(&units);
        target[units.len()] = 0;
    }

    /// Notifications through the classic tray balloon API, which Windows 10 and 11 show as a toast.
    pub struct Notifier {
        window: HWND,
        added: bool,
    }

    impl Notifier {
        pub fn new(window: &Window) -> Self {
            let handle = window.window_handle().ok().and_then(|handle| match handle.as_raw() {
                RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as HWND),
                _ => None,
            });
            Self { window: handle.unwrap_or(null_mut()), added: false }
        }

        fn data(&self) -> NOTIFYICONDATAW {
            // An all-zero value is valid for this plain C struct.
            let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.window;
            data.uID = ICON_ID;
            data
        }

        /// Failing to show one must never disturb monitoring.
        pub fn show(&mut self, message: &str, critical: bool) {
            if self.window.is_null() {
                return;
            }

            if !self.added && !self.add_icon() {
                return;
            }

            let mut data = self.data();
            data.uFlags = NIF_INFO;
            copy_into(&mut data.szInfoTitle, "StrataMon");
            copy_into(&mut data.szInfo, message);
            data.dwInfoFlags = if critical { NIIF_ERROR } else { NIIF_WARNING };
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0 {
                return;
            }

            // Explorer restarting takes every tray icon with it, and from then on this fails,
            // which would silence notifications for the rest of the run. Put the icon back and
            // try once more. If adding fails too, the icon is likely still there and the shell
            // was only busy, so the next notification tries again as usual.
            if self.add_icon() {
                unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
            }
        }

        fn add_icon(&mut self) -> bool {
            let mut data = self.data();
            data.uFlags = NIF_ICON | NIF_TIP;
            // The stock application icon, loaded from Windows itself.
            data.hIcon = unsafe { LoadIconW(null_mut(), IDI_APPLICATION) };
            copy_into(&mut data.szTip, "StrataMon");
            let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
            self.added |= added;
            added
        }
    }

    impl Drop for Notifier {
        fn drop(&mut self) {
            if self.added {
                let data = self.data();
                unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            }
        }
    }

    pub fn open_link(url: &str) {
        let (verb, url) = (wide("open"), wide(url));
        unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), url.as_ptr(), null(), null(), SW_SHOWNORMAL) };
    }

    pub fn utc_offset_seconds() -> i32 {
        // An all-zero value is valid for this plain C struct, and the call fills it in.
        let mut zone: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        let kind = unsafe { GetTimeZoneInformation(&mut zone) };
        let bias = zone.Bias + if kind == TIME_ZONE_ID_DAYLIGHT { zone.DaylightBias } else { zone.StandardBias };
        -bias * 60
    }
}

#[cfg(not(windows))]
mod linux {
    use super::*;
    use std::process::{Command, Stdio};

    pub struct Notifier;

    impl Notifier {
        pub fn new(_: &Window) -> Self {
            Self
        }

        /// Failing to show one must never disturb monitoring, and notify-send may not be installed.
        pub fn show(&mut self, message: &str, critical: bool) {
            let urgency = if critical { "critical" } else { "normal" };
            run_quietly(Command::new("notify-send").args(["--urgency", urgency, "StrataMon", message]));
        }
    }

    pub fn open_link(url: &str) {
        run_quietly(Command::new("xdg-open").arg(url));
    }

    /// Starts a helper without waiting for it. It's waited on from a thread of its own, or every
    /// one that finished would stay behind as a zombie process until StrataMon exits.
    fn run_quietly(command: &mut Command) {
        if let Ok(mut child) = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    pub fn utc_offset_seconds() -> i32 {
        // An all-zero value is valid for this plain C struct, and the call fills it in.
        let mut local: libc::tm = unsafe { std::mem::zeroed() };
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        unsafe { libc::localtime_r(&now, &mut local) };
        local.tm_gmtoff as i32
    }
}

#[cfg(windows)]
pub use windows::{Notifier, open_link, utc_offset_seconds};

#[cfg(not(windows))]
pub use linux::{Notifier, open_link, utc_offset_seconds};

/// The icon built into the exe, so there's only one copy of it.
#[cfg(windows)]
pub fn window_icon() -> Option<winit::window::Icon> {
    use winit::platform::windows::IconExtWindows;
    winit::window::Icon::from_resource(1, None).ok()
}

#[cfg(not(windows))]
pub fn window_icon() -> Option<winit::window::Icon> {
    None
}
