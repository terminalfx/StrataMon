//! Display text for readings. Missing values always render as [`UNAVAILABLE`], never 0.

use std::sync::atomic::{AtomicI32, Ordering};

pub const UNAVAILABLE: &str = "n/a";

const BYTE_UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
const GIBIBYTE: f64 = 1_073_741_824.0;

static UTC_OFFSET_SECONDS: AtomicI32 = AtomicI32::new(0);

fn or_unavailable<T>(value: Option<T>, show: impl FnOnce(T) -> String) -> String {
    value.map_or_else(|| UNAVAILABLE.to_string(), show)
}

pub fn percent(value: Option<f64>) -> String {
    or_unavailable(value, |v| format!("{:.0}%", v.round()))
}

pub fn temperature(value: Option<f64>) -> String {
    or_unavailable(value, |v| format!("{:.0} °C", v.round()))
}

pub fn watts(value: Option<f64>) -> String {
    or_unavailable(value, |v| format!("{:.0} W", v.round()))
}

pub fn bytes(value: Option<f64>) -> String {
    or_unavailable(value, |mut amount| {
        let mut unit = 0;
        while amount >= 1024.0 && unit < BYTE_UNITS.len() - 1 {
            amount /= 1024.0;
            unit += 1;
        }

        if unit == 0 || amount >= 100.0 {
            format!("{amount:.0} {}", BYTE_UNITS[unit])
        } else {
            format!("{amount:.1} {}", BYTE_UNITS[unit])
        }
    })
}

/// Used and total memory in the same unit, like `10.4 / 12 GB`.
pub fn used_of(used_bytes: Option<i64>, total_bytes: Option<i64>) -> String {
    match (used_bytes, total_bytes) {
        (Some(used), Some(total)) => {
            format!("{:.1} / {} GB", used as f64 / GIBIBYTE, one_decimal_at_most(total as f64 / GIBIBYTE))
        }
        _ => UNAVAILABLE.to_string(),
    }
}

pub fn speed(tokens_per_second: Option<f64>) -> String {
    or_unavailable(tokens_per_second, |v| format!("{v:.1} tok/s"))
}

pub fn megabytes_per_second(value: Option<f64>) -> String {
    or_unavailable(value, |v| {
        if v >= 100.0 {
            format!("{} MB/s", thousands(v.round() as i64))
        } else {
            format!("{v:.1} MB/s")
        }
    })
}

pub fn tokens(value: Option<i64>) -> String {
    or_unavailable(value, thousands)
}

/// A count shortened to fit a table cell, like `18.2K` or `1.4M`.
pub fn short(value: Option<i64>) -> String {
    or_unavailable(value, |v| {
        if v >= 1_000_000 {
            format!("{}M", one_decimal_at_most(v as f64 / 1_000_000.0))
        } else if v >= 10_000 {
            format!("{}K", one_decimal_at_most(v as f64 / 1000.0))
        } else {
            thousands(v)
        }
    })
}

/// A context size the way Strata names them, like `32K` for 32,768 tokens.
pub fn context_size(tokens: Option<i64>) -> String {
    or_unavailable(tokens, |v| format!("{:.0}K", (v as f64 / 1024.0).round()))
}

pub fn seconds(value: Option<f64>) -> String {
    or_unavailable(value, |v| format!("{v:.1} s"))
}

pub fn duration(seconds: f64) -> String {
    let total = seconds.max(0.0) as i64;
    let (days, hours, minutes) = (total / 86_400, total / 3600 % 24, total / 60 % 60);
    if days >= 1 {
        format!("{days}d {hours}h")
    } else if hours >= 1 {
        format!("{hours}h {minutes}m")
    } else if minutes >= 1 {
        format!("{minutes}m")
    } else {
        format!("{}s", total % 60)
    }
}

/// Sets the offset used for times of day. The app finds it from the operating system.
pub fn set_utc_offset(seconds: i32) {
    UTC_OFFSET_SECONDS.store(seconds, Ordering::Relaxed);
}

/// A time of day in the local zone, like `15:33:51`.
pub fn clock(unix_seconds: f64) -> String {
    // The parser only lets sane magnitudes through, but the float cast saturates to i64 bounds
    // on its own, and adding a local offset to an epoch near that bound would wrap.
    let shifted = unix_seconds.floor() as i64;
    let local = shifted.saturating_add(i64::from(UTC_OFFSET_SECONDS.load(Ordering::Relaxed)));
    let day = local.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, day / 60 % 60, day % 60)
}

fn thousands(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if value < 0 {
        text.push('-');
    }

    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }

    text
}

fn one_decimal_at_most(value: f64) -> String {
    let text = format!("{value:.1}");
    text.strip_suffix(".0").map_or(text.clone(), String::from)
}
