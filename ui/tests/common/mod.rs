//! Realistic servers for the interface tests, and a way to save what they look like.

#![allow(dead_code)]

use stratamon_core::client::{PollFailure, PollResult};
use stratamon_core::config::{AlertThresholds, AppConfig, ServerConfig};
use stratamon_core::models::*;
use stratamon_ui::text::Text;
use stratamon_ui::{Input, Ui};
use tiny_skia::Pixmap;

const GB: i64 = 1 << 30;
const GB_F: f64 = GB as f64;
pub const NOW: f64 = 1_793_900_000.0;

pub fn config() -> AppConfig {
    let server = |id: &str, name: &str, addresses: &[&str]| ServerConfig {
        id: id.into(),
        name: name.into(),
        addresses: addresses.iter().map(|a| a.to_string()).collect(),
        key: String::new(),
    };

    AppConfig {
        servers: vec![
            server("a", "NEDM", &["192.168.1.5", "100.123.227.92"]),
            server("b", "NEDM2", &["192.168.1.231"]),
            server("c", "NEDM3", &["192.168.1.209"]),
        ],
        alerts: AlertThresholds { sustain_seconds: 0, queue_wait_seconds: 0, ..Default::default() },
        desktop_notifications: false,
        ..Default::default()
    }
}

fn row(ago: f64, prompt: i64, output: i64, speed: f64, took: f64, outcome: RequestOutcome) -> RequestRow {
    RequestRow {
        started_at: Some(NOW - ago),
        prompt_tokens: Some(prompt),
        reused_tokens: Some(prompt - 500),
        output_tokens: Some(output),
        tokens_per_second: Some(speed),
        duration_seconds: Some(took),
        outcome,
    }
}

pub fn busy(speed: f64, cpu: f64) -> StrataSnapshot {
    let done = RequestOutcome::Done;
    StrataSnapshot {
        instance_id: Some("one".into()),
        model: Some("qwen3.8-flash-next-gyro-gyro-s".into()),
        context_tokens: Some(32768),
        kv_cache: Some("q8_0".into()),
        activity: StrataActivity::Generating,
        uptime_seconds: Some(91_000.0),
        progress_done: Some(3197),
        progress_total: Some(9830),
        context_used: Some(10_021),
        speed: Some(speed),
        speed_basis: SpeedBasis::Now,
        live_speed: Some(speed),
        slots: Some(1),
        running: Some(1),
        waiting: Some(0),
        gpu: GpuReading {
            name: Some("NVIDIA GeForce RTX 5070".into()),
            load_percent: Some(96.0),
            memory_used_bytes: Some((10.4 * GB_F) as i64),
            memory_total_bytes: Some(12 * GB),
            temperature_c: Some(59.0),
            power_watts: Some(116.0),
            power_limit_watts: Some(250.0),
            ..Default::default()
        },
        gpus: vec![GpuReading { name: Some("NVIDIA GeForce RTX 5070".into()), temperature_c: Some(59.0), ..Default::default() }],
        pcie_generation: Some(4),
        pcie_width: Some(16),
        pcie_to_gpu_megabytes_per_second: Some(3475.0),
        cpu_percent: Some(cpu),
        cpu_cores: Some(6),
        cpu_threads: Some(12),
        ram_used_bytes: Some((47.7 * GB_F) as i64),
        ram_total_bytes: Some(63 * GB),
        disk_read_megabytes_per_second: Some(9.0),
        disk_write_megabytes_per_second: Some(0.2),
        experts_cached: Some(3099),
        expert_cache_mebibytes: Some(3072.0),
        total_requests: Some(1204),
        total_output_tokens: Some(2_140_000),
        recent: vec![
            row(13.0, 18187, 261, 36.3, 8.7, done),
            row(26.0, 17598, 572, 55.7, 12.4, done),
            row(29.0, 14709, 126, 47.0, 3.2, done),
            row(35.0, 14407, 261, 52.8, 5.9, done),
            row(173.0, 6631, 7719, 56.7, 137.4, done),
        ],
        ..Default::default()
    }
}

pub fn hot(temperature: f64) -> StrataSnapshot {
    let card = |index, load, used: f64, heat, power| GpuReading {
        index,
        load_percent: Some(load),
        memory_used_bytes: Some((used * GB_F) as i64),
        memory_total_bytes: Some(16 * GB),
        temperature_c: Some(heat),
        power_watts: Some(power),
        ..Default::default()
    };

    StrataSnapshot {
        instance_id: Some("two".into()),
        model: Some("qwen3.8-flash-next-unsloth-ud-iq4_xs".into()),
        context_tokens: Some(131_072),
        kv_cache: Some("int8".into()),
        activity: StrataActivity::Reading,
        uptime_seconds: Some(520.0),
        progress_done: Some(62_000),
        progress_total: Some(125_449),
        context_used: Some(125_449),
        speed: Some(412.5),
        speed_basis: SpeedBasis::ReadingPrompt,
        live_speed: Some(0.0),
        slots: Some(4),
        running: Some(3),
        waiting: Some(2),
        rejected: Some(2),
        gpu: GpuReading {
            name: Some("x".into()),
            load_percent: Some(100.0),
            memory_used_bytes: Some(31 * GB),
            memory_total_bytes: Some(32 * GB),
            temperature_c: Some(temperature),
            power_watts: Some(326.0),
            power_limit_watts: Some(330.0),
            ..Default::default()
        },
        gpus: vec![card(0, 64.0, 15.6, 72.0, 158.0), card(1, 100.0, 15.8, temperature, 168.0)],
        pcie_generation: Some(4),
        pcie_width: Some(8),
        pcie_to_gpu_megabytes_per_second: Some(14528.0),
        cpu_percent: Some(12.0),
        cpu_cores: Some(32),
        cpu_threads: Some(64),
        ram_used_bytes: Some(120 * GB),
        ram_total_bytes: Some(125 * GB),
        disk_read_megabytes_per_second: Some(842.0),
        disk_write_megabytes_per_second: Some(3.1),
        experts_cached: Some(7761),
        expert_cache_mebibytes: Some(17916.0),
        total_requests: Some(16),
        total_output_tokens: Some(2362),
        recent: vec![
            row(52.0, 125_449, 9, 35.7, 52.2, RequestOutcome::Done),
            row(120.0, 88120, 410, 41.2, 61.0, RequestOutcome::Failed),
            row(240.0, 5120, 96, 44.0, 2.5, RequestOutcome::Stopped),
        ],
        ..Default::default()
    }
}

pub fn read_only() -> StrataSnapshot {
    StrataSnapshot {
        access: TelemetryAccess::TelemetryOnly,
        instance_id: Some("three".into()),
        model: Some("qwen3.8-flash-next-iq3_s".into()),
        context_tokens: Some(204_800),
        uptime_seconds: Some(2_000_000.0),
        context_used: Some(4100),
        speed: Some(49.3),
        speed_basis: SpeedBasis::LastRequest,
        live_speed: Some(0.0),
        slots: Some(1),
        running: Some(0),
        waiting: Some(0),
        gpu: GpuReading {
            name: Some("NVIDIA GeForce RTX 4090".into()),
            load_percent: Some(2.0),
            memory_used_bytes: Some(14 * GB),
            memory_total_bytes: Some(24 * GB),
            temperature_c: Some(44.0),
            power_watts: Some(31.0),
            ..Default::default()
        },
        gpus: vec![GpuReading { name: Some("NVIDIA GeForce RTX 4090".into()), temperature_c: Some(44.0), ..Default::default() }],
        ram_used_bytes: Some(21 * GB),
        ram_total_bytes: Some(64 * GB),
        total_requests: Some(88),
        total_output_tokens: Some(40_120),
        recent: vec![row(300.0, 4000, 100, 49.3, 2.2, RequestOutcome::Done)],
        ..Default::default()
    }
}

pub fn good(snapshot: StrataSnapshot, address: &str) -> PollResult {
    PollResult { snapshot: Some(snapshot), failure: PollFailure::None, address: Some(address.into()), detail: None }
}

pub fn bad(failure: PollFailure, address: &str, detail: &str) -> PollResult {
    PollResult { snapshot: None, failure, address: Some(address.into()), detail: Some(detail.into()) }
}

pub fn new_ui(config: AppConfig, dark: bool) -> Ui {
    let text = Text::from_system().expect("a system font");
    let mut ui = Ui::new(config, text, dark);
    ui.set_viewport(1130, 960, 1.0);
    ui
}

/// Three servers with six minutes' worth of history, like a window left open.
pub fn filled_ui(dark: bool) -> Ui {
    let mut ui = new_ui(config(), dark);
    let generation = ui.dashboard.generation;
    for i in 0..100 {
        let t = f64::from(i);
        let wave = 44.0 + 9.0 * (t / 7.0).sin() + (t * 2.3).sin() * 2.0;
        ui.apply_poll(generation, 0, good(busy(wave, 86.0 + (t * 1.7).sin() * 4.0), "100.123.227.92"), NOW + t);
        ui.apply_poll(generation, 1, good(hot(70.0 + t / 4.0), "192.168.1.231"), NOW + t);
        ui.apply_poll(generation, 2, good(read_only(), "192.168.1.209"), NOW + t);
    }
    ui
}

pub fn frame(ui: &mut Ui, width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).unwrap();
    ui.draw(&mut pixmap);
    pixmap
}

/// Saves a frame when STRATAMON_SHOTS names a folder, so a person can look at it.
pub fn shot(pixmap: &Pixmap, name: &str) {
    if let Ok(folder) = std::env::var("STRATAMON_SHOTS") {
        pixmap.save_png(format!("{folder}/{name}.png")).unwrap();
    }
}

/// Presses something by name, the way a mouse would, scrolling to it first if it's off screen.
pub fn click(ui: &mut Ui, name: &str) -> Vec<stratamon_ui::Command> {
    let mut pixmap = Pixmap::new(1130, 960).unwrap();
    ui.draw(&mut pixmap);
    for lines in [-1.0, 1.0] {
        for _ in 0..80 {
            if ui.locate(name).is_some() {
                break;
            }
            ui.handle(Input::Wheel { lines });
            ui.draw(&mut pixmap);
        }
    }

    let (x, y) = ui.locate(name).unwrap_or_else(|| panic!("nothing called {name} on screen"));
    ui.handle(Input::MouseMove { x, y });
    ui.handle(Input::MouseDown);
    ui.handle(Input::MouseUp)
}
