//! What each server's card shows, as plain text and numbers. There's no drawing here, so every
//! rule about what appears on a card can be tested without a window.

use crate::alerts::{AlertChange, AlertEvent, AlertSeverity};
use crate::client::PollFailure;
use crate::config::AlertThresholds;
use crate::format;
use crate::models::*;
use crate::state::{ServerState, ServerStatus, Time};
use std::collections::VecDeque;

const HISTORY_MINUTES: u32 = 6;
const RECENT_REQUESTS_SHOWN: usize = 5;
const PERCENTAGE: f64 = 100.0;

/// How far below its limit a reading starts to count as near it.
const NEAR_LIMIT_MARGIN: f64 = 10.0;

// Keeps a flat trend of small values from filling the tile as if it were at full scale.
const SMALLEST_SCALE: f64 = 1.0;
const HEADROOM: f64 = 1.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetricLevel {
    #[default]
    Normal,
    NearLimit,
    OverLimit,
}

pub fn level_for(value: Option<f64>, limit: f64) -> MetricLevel {
    match value {
        Some(reading) if reading >= limit => MetricLevel::OverLimit,
        // The margin can't outrun small limits: a 1-watt or 1-degree limit would otherwise call
        // everything near it, so the margin never takes more than half of the limit itself.
        Some(reading) if reading >= limit - NEAR_LIMIT_MARGIN.min(limit * 0.5) => MetricLevel::NearLimit,
        _ => MetricLevel::Normal,
    }
}

/// A fixed-length rolling history. Gaps stay as `None`, so a line never bridges an outage.
#[derive(Debug, Clone)]
pub struct History {
    capacity: usize,
    samples: VecDeque<Option<f64>>,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, samples: VecDeque::with_capacity(capacity + 1) }
    }

    pub fn add(&mut self, value: Option<f64>) {
        self.samples.push_back(value);
        if self.samples.len() > self.capacity {
            self.samples.pop_front();
        }
    }

    pub fn samples(&self) -> &VecDeque<Option<f64>> {
        &self.samples
    }
}

/// One of the tiles from Strata's Monitor: a reading, a note, and its recent trend.
#[derive(Debug, Clone)]
pub struct Tile {
    pub label: &'static str,
    pub value: String,
    pub note: Option<String>,
    pub level: MetricLevel,
    pub scale: f64,
    pub history_length: usize,
    pub sample_seconds: u32,
    pub format_sample: fn(Option<f64>) -> String,
    fixed_scale: Option<f64>,
    history: History,
}

impl Tile {
    pub fn new(
        label: &'static str,
        history_length: usize,
        sample_seconds: u32,
        format_sample: fn(Option<f64>) -> String,
        fixed_scale: Option<f64>,
    ) -> Self {
        Self {
            label,
            value: format::UNAVAILABLE.to_string(),
            note: None,
            level: MetricLevel::Normal,
            scale: fixed_scale.unwrap_or(SMALLEST_SCALE),
            history_length,
            sample_seconds,
            format_sample,
            fixed_scale,
            history: History::new(history_length),
        }
    }

    pub fn samples(&self) -> &VecDeque<Option<f64>> {
        self.history.samples()
    }

    pub fn update(&mut self, value: String, note: Option<String>, sample: Option<f64>, level: MetricLevel, scale: Option<f64>) {
        self.value = value;
        self.note = note;
        self.level = level;
        self.history.add(sample);

        // Always from zero, so a change of a few tokens a second can't look like a collapse.
        let peak = self.samples().iter().flatten().fold(0.0_f64, |peak, value| peak.max(*value));
        // The scale arrives from the server's own limits, so a zero or nonsense value falls back
        // to the history, keeping the trend's division safe and the line drawn.
        let scale = scale.filter(|value| value.is_finite() && *value > 0.0);
        self.scale = scale.or(self.fixed_scale).unwrap_or(SMALLEST_SCALE.max(peak * HEADROOM));
    }

    /// Records a gap in the trend without changing the last known reading.
    pub fn record_gap(&mut self) {
        self.history.add(None);
    }

    /// Blanks a live rate, which means nothing once the server has stopped answering.
    pub fn clear_reading(&mut self) {
        self.value = format::UNAVAILABLE.to_string();
        self.note = None;
    }
}

/// A share of something, shown as a bar against its limit.
#[derive(Debug, Clone)]
pub struct Bar {
    pub label: String,
    pub value: String,
    pub percent: Option<f64>,
    pub limit: f64,
    pub level: MetricLevel,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            label: String::new(),
            value: format::UNAVAILABLE.to_string(),
            percent: None,
            limit: PERCENTAGE,
            level: MetricLevel::Normal,
        }
    }
}

impl Bar {
    pub fn update(&mut self, label: &str, value: String, percent: Option<f64>, limit: f64) {
        self.label = label.to_string();
        self.value = value;
        self.percent = percent;
        self.limit = limit;
        self.level = level_for(percent, limit);
    }
}

/// A labeled reading with no bar or trend.
#[derive(Debug, Clone)]
pub struct Stat {
    pub label: String,
    pub value: String,
}

impl Default for Stat {
    fn default() -> Self {
        Self { label: String::new(), value: format::UNAVAILABLE.to_string() }
    }
}

impl Stat {
    pub fn update(&mut self, label: &str, value: String) {
        self.label = label.to_string();
        self.value = value;
    }
}

/// One card on a server with several, which the tiles sum up as a whole.
#[derive(Debug, Clone, Default)]
pub struct GpuRow {
    pub name: String,
    pub reading: String,
    pub level: MetricLevel,
}

impl GpuRow {
    pub fn new(gpu: &GpuReading, temperature_limit: f64) -> Self {
        Self {
            name: format!("GPU {}", gpu.index),
            reading: format!(
                "{} · {} · {} · {}",
                format::percent(gpu.load_percent),
                format::used_of(gpu.memory_used_bytes, gpu.memory_total_bytes),
                format::temperature(gpu.temperature_c),
                format::watts(gpu.power_watts)
            ),
            level: level_for(gpu.temperature_c, temperature_limit),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestLine {
    pub time: String,
    pub status: &'static str,
    pub is_problem: bool,
    pub prompt: String,
    pub output: String,
    pub speed: String,
    pub duration: String,
    pub reused: String,
}

impl RequestLine {
    pub fn new(request: &RequestRow) -> Self {
        Self {
            time: request.started_at.map_or_else(|| format::UNAVAILABLE.to_string(), format::clock),
            status: match request.outcome {
                RequestOutcome::Failed => "Failed",
                RequestOutcome::TimedOut => "Timed out",
                RequestOutcome::Stopped => "Stopped",
                RequestOutcome::Done => "Done",
            },
            is_problem: matches!(request.outcome, RequestOutcome::Failed | RequestOutcome::TimedOut),
            prompt: format::short(request.prompt_tokens),
            output: format::short(request.output_tokens),
            speed: request.tokens_per_second.map_or_else(|| format::UNAVAILABLE.to_string(), |speed| format!("{speed:.1}")),
            duration: format::seconds(request.duration_seconds),
            reused: format!("{} of the prompt's tokens reused", format::tokens(request.reused_tokens)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertRow {
    pub message: String,
    pub time: String,
    pub kind: &'static str,
    pub is_critical: bool,
    pub is_over_limit: bool,
    pub is_resolved: bool,
    pub is_notice: bool,
}

impl AlertRow {
    pub fn new(alert: &AlertEvent) -> Self {
        let is_resolved = alert.change == AlertChange::Resolved;
        let is_notice = alert.change == AlertChange::Notice;
        let raised = alert.change == AlertChange::Raised;
        let is_critical = raised && alert.severity == AlertSeverity::Critical;
        let is_over_limit = raised && alert.severity == AlertSeverity::Warning;
        Self {
            message: alert.message.clone(),
            time: format::clock(alert.at),
            kind: if is_resolved {
                "Resolved"
            } else if is_notice {
                "Notice"
            } else if is_critical {
                "Critical"
            } else {
                "Over limit"
            },
            is_critical,
            is_over_limit,
            is_resolved,
            is_notice,
        }
    }
}

/// One server's card, which shows the same things as Strata's own Monitor.
#[derive(Debug, Clone)]
pub struct ServerCard {
    pub name: String,
    pub status: ServerStatus,
    pub failure: PollFailure,
    pub subtitle: Option<String>,
    pub route: Option<String>,
    pub problem: Option<String>,
    pub has_data: bool,
    pub data_opacity: f32,
    pub activity: String,
    pub progress_text: Option<String>,
    pub progress_percent: Option<f64>,
    pub access_note: Option<&'static str>,

    pub speed: Tile,
    pub gpu_load: Tile,
    pub vram: Tile,
    pub gpu_temperature: Tile,
    pub power: Tile,
    pub pcie: Tile,
    pub cpu: Tile,
    pub disk: Tile,

    pub context: Bar,
    pub ram: Bar,
    pub experts: Stat,
    pub requests: Stat,
    pub served: Stat,
    pub gpu_rows: Vec<GpuRow>,
    pub recent_requests: Vec<RequestLine>,

    limits: AlertThresholds,
    shown_requests: Vec<RequestRow>,
}

impl ServerCard {
    pub fn new(name: &str, limits: AlertThresholds, poll_seconds: u32) -> Self {
        let sample_seconds = poll_seconds.max(1);
        // Even polls farther apart than the history window keep one sample, so a tile always
        // has a baseline to draw against.
        let history_length = (HISTORY_MINUTES * 60 / sample_seconds) as usize;
        let history_length = history_length.max(1);
        let tile = |label, format_sample, scale| Tile::new(label, history_length, sample_seconds, format_sample, scale);

        Self {
            name: name.to_string(),
            status: ServerStatus::Connecting,
            failure: PollFailure::None,
            subtitle: None,
            route: None,
            problem: None,
            has_data: false,
            data_opacity: 1.0,
            activity: String::new(),
            progress_text: None,
            progress_percent: None,
            access_note: None,

            // Strata's own tiles, in Strata's order.
            speed: tile("Speed", format::speed, None),
            gpu_load: tile("GPU load", format::percent, Some(PERCENTAGE)),
            vram: tile("VRAM", format::percent, Some(PERCENTAGE)),
            gpu_temperature: tile("GPU temp", format::temperature, Some(PERCENTAGE)),
            power: tile("Power", format::watts, None),
            pcie: tile("PCIe", format::megabytes_per_second, None),
            cpu: tile("CPU", format::percent, Some(PERCENTAGE)),
            disk: tile("Disk read", format::megabytes_per_second, None),

            context: Bar::default(),
            ram: Bar::default(),
            experts: Stat::default(),
            requests: Stat::default(),
            served: Stat::default(),
            gpu_rows: Vec::new(),
            recent_requests: Vec::new(),

            limits,
            shown_requests: Vec::new(),
        }
    }

    pub fn tiles(&self) -> [&Tile; 8] {
        [&self.speed, &self.gpu_load, &self.vram, &self.gpu_temperature, &self.power, &self.pcie, &self.cpu, &self.disk]
    }

    fn tiles_mut(&mut self) -> [&mut Tile; 8] {
        [
            &mut self.speed,
            &mut self.gpu_load,
            &mut self.vram,
            &mut self.gpu_temperature,
            &mut self.power,
            &mut self.pcie,
            &mut self.cpu,
            &mut self.disk,
        ]
    }

    pub fn is_connecting(&self) -> bool {
        self.status == ServerStatus::Connecting
    }

    pub fn is_online(&self) -> bool {
        self.status == ServerStatus::Online
    }

    pub fn is_offline(&self) -> bool {
        self.status == ServerStatus::Offline
    }

    pub fn is_rejected(&self) -> bool {
        self.status == ServerStatus::Rejected
    }

    pub fn status_text(&self) -> &'static str {
        match self.status {
            ServerStatus::Online => "Online",
            ServerStatus::Offline if self.failure == PollFailure::NotListening => "Not running",
            ServerStatus::Offline => "Offline",
            ServerStatus::Rejected => match self.failure {
                PollFailure::KeyMissing => "Needs a key",
                PollFailure::KeyRejected => "Key rejected",
                PollFailure::HostRefused => "Name refused",
                _ => "Needs update",
            },
            ServerStatus::Connecting => "Connecting",
        }
    }

    pub fn update(&mut self, state: &ServerState, now: Time) {
        self.failure = state.last_failure;
        self.status = state.status;
        self.problem = if self.is_online() { None } else { state.last_error.clone() };

        // Keep the last reading on screen but faded, so a dropped server doesn't
        // blank its card, and old numbers can't pass for live ones.
        self.data_opacity = if self.is_online() { 1.0 } else { 0.45 };

        // A failed poll has no new reading, even before it's enough to count as offline.
        let Some(snapshot) = state.latest.as_ref().filter(|_| state.last_failure == PollFailure::None) else {
            self.route = Some(if self.is_online() {
                format!("via {}", state.active_address.as_deref().unwrap_or(""))
            } else {
                describe_last_seen(state, now)
            });

            for tile in self.tiles_mut() {
                tile.record_gap();
            }

            if !self.is_online() {
                self.show_last_known();
            }

            return;
        };

        self.has_data = true;
        self.show(snapshot, state.active_address.as_deref().unwrap_or(""));
    }

    // Slow-moving readings are still useful, but "generating at 45 tok/s" would read as a working server.
    fn show_last_known(&mut self) {
        self.activity = "Last reading".to_string();
        self.progress_percent = None;
        self.progress_text = None;
        self.speed.clear_reading();
    }

    fn show(&mut self, snapshot: &StrataSnapshot, address: &str) {
        let context = snapshot.context_tokens.map(|_| format!("{} context", format::context_size(snapshot.context_tokens)));
        let kv = snapshot.kv_cache.as_deref().map(|kv| format!("kv {kv}"));
        let parts: Vec<&str> = [snapshot.model.as_deref(), context.as_deref(), kv.as_deref()].into_iter().flatten().collect();
        self.subtitle = Some(parts.join(" · "));
        self.route = Some(match snapshot.uptime_seconds {
            Some(uptime) => format!("via {address} · up {}", format::duration(uptime)),
            None => format!("via {address}"),
        });

        self.activity = describe_activity(snapshot).to_string();
        self.progress_percent = snapshot.progress_percent();
        self.progress_text = snapshot.progress_percent().map(|percent| {
            format!(
                "{} / {} tokens · {}",
                format::tokens(snapshot.progress_done),
                format::tokens(snapshot.progress_total),
                format::percent(Some(percent))
            )
        });

        self.show_tiles(snapshot);

        self.context.update(
            "Context fill",
            format!("{} / {}", format::context_size(snapshot.context_used), format::context_size(snapshot.context_tokens)),
            snapshot.context_percent(),
            PERCENTAGE,
        );
        self.ram.update(
            "System RAM",
            format::used_of(snapshot.ram_used_bytes, snapshot.ram_total_bytes),
            snapshot.ram_percent(),
            self.limits.ram_percent,
        );
        self.experts.update("Experts in VRAM", describe_experts(snapshot));
        self.requests.update("Requests", describe_requests(snapshot));
        self.served.update(
            "Served",
            format!("{} requests · {} tokens", format::tokens(snapshot.total_requests), format::short(snapshot.total_output_tokens)),
        );

        self.access_note = match snapshot.access {
            TelemetryAccess::TelemetryOnly => {
                Some("This key is read-only, so the KV cache type, CPU, disk, PCIe and expert readings aren't available. Strata's API key shows them.")
            }
            TelemetryAccess::MetricsOnly => Some("This Strata is older, so queue limits aren't reported."),
            TelemetryAccess::Full => None,
        };

        self.gpu_rows = if snapshot.gpus.len() > 1 {
            snapshot.gpus.iter().map(|gpu| GpuRow::new(gpu, self.limits.gpu_temperature_c)).collect()
        } else {
            Vec::new()
        };

        let requests = &snapshot.recent[..snapshot.recent.len().min(RECENT_REQUESTS_SHOWN)];
        if requests != self.shown_requests {
            self.shown_requests = requests.to_vec();
            self.recent_requests = requests.iter().map(RequestLine::new).collect();
        }
    }

    fn show_tiles(&mut self, snapshot: &StrataSnapshot) {
        let gpu = &snapshot.gpu;
        let several = snapshot.gpus.len() > 1;
        let normal = MetricLevel::Normal;

        self.speed.update(
            format::speed(snapshot.speed),
            describe_speed(snapshot.speed_basis).map(String::from),
            snapshot.live_speed,
            normal,
            None,
        );
        self.gpu_load.update(
            format::percent(gpu.load_percent),
            if several { Some("busiest card".to_string()) } else { short_gpu_name(gpu.name.as_deref()) },
            gpu.load_percent,
            normal,
            None,
        );
        self.vram.update(
            format::used_of(gpu.memory_used_bytes, gpu.memory_total_bytes),
            Some("graphics memory".to_string()),
            gpu.memory_percent(),
            normal,
            None,
        );
        self.gpu_temperature.update(
            format::temperature(gpu.temperature_c),
            Some(if several {
                "hottest card".to_string()
            } else {
                format!("limit {}", format::temperature(Some(self.limits.gpu_temperature_c)))
            }),
            gpu.temperature_c,
            level_for(gpu.temperature_c, self.limits.gpu_temperature_c),
            None,
        );
        self.power.update(
            format::watts(gpu.power_watts),
            gpu.power_limit_watts.map(|limit| format!("of {} limit", format::watts(Some(limit)))),
            gpu.power_watts,
            normal,
            gpu.power_limit_watts,
        );
        self.pcie.update(
            snapshot.pcie_generation.map_or_else(
                || format::UNAVAILABLE.to_string(),
                |generation| format!("Gen{generation} x{}", snapshot.pcie_width.map_or_else(String::new, |width| width.to_string())),
            ),
            snapshot.pcie_to_gpu_megabytes_per_second.map(|rate| format!("to GPU {}", format::megabytes_per_second(Some(rate)))),
            snapshot.pcie_to_gpu_megabytes_per_second,
            normal,
            None,
        );
        self.cpu.update(
            format::percent(snapshot.cpu_percent),
            snapshot.cpu_cores.map(|cores| {
                format!("{cores} cores · {} threads", snapshot.cpu_threads.map_or_else(String::new, |threads| threads.to_string()))
            }),
            snapshot.cpu_percent,
            normal,
            None,
        );
        self.disk.update(
            format::megabytes_per_second(snapshot.disk_read_megabytes_per_second),
            snapshot.disk_write_megabytes_per_second.map(|rate| format!("write {}", format::megabytes_per_second(Some(rate)))),
            snapshot.disk_read_megabytes_per_second,
            normal,
            None,
        );
    }
}

fn describe_activity(snapshot: &StrataSnapshot) -> &'static str {
    if snapshot.draining {
        return "Shutting down";
    }

    match snapshot.activity {
        StrataActivity::Reading => "Reading prompt",
        StrataActivity::Generating => "Generating a reply",
        StrataActivity::Queued => "Requests are queued",
        StrataActivity::Unloaded => "Model not loaded",
        StrataActivity::Error => "Model unavailable",
        StrataActivity::Idle => "Waiting for a request",
    }
}

fn describe_speed(basis: SpeedBasis) -> Option<&'static str> {
    match basis {
        SpeedBasis::Now => Some("now"),
        SpeedBasis::ReadingPrompt => Some("reading the prompt"),
        SpeedBasis::RunAverage => Some("average of the run"),
        SpeedBasis::LastRequest => Some("last request"),
        SpeedBasis::None => None,
    }
}

fn describe_experts(snapshot: &StrataSnapshot) -> String {
    match (snapshot.experts_cached, snapshot.expert_cache_mebibytes) {
        (Some(experts), Some(size)) => format!("{} · {}", format::tokens(Some(experts)), format::bytes(Some(size * 1_048_576.0))),
        (Some(experts), None) => format::tokens(Some(experts)),
        _ => format::UNAVAILABLE.to_string(),
    }
}

fn describe_requests(snapshot: &StrataSnapshot) -> String {
    let running = snapshot.running.unwrap_or(0);
    let mut parts = vec![match snapshot.slots {
        Some(slots) if slots > 1 => format!("{running} of {slots} slots"),
        _ => format!("{running} running"),
    }];

    if let Some(waiting) = snapshot.waiting.filter(|count| *count > 0) {
        parts.push(format!("{waiting} waiting"));
    }

    if let Some(rejected) = snapshot.rejected.filter(|count| *count > 0) {
        parts.push(format!("{rejected} turned away"));
    }

    if let Some(timed_out) = snapshot.queue_timeouts.filter(|count| *count > 0) {
        parts.push(format!("{timed_out} timed out"));
    }

    parts.join(" · ")
}

fn describe_last_seen(state: &ServerState, now: Time) -> String {
    state.last_seen.map_or_else(
        || "not reached yet".to_string(),
        |seen| format!("last seen {} ago", format::duration(now - seen)),
    )
}

fn short_gpu_name(name: Option<&str>) -> Option<String> {
    name.map(|name| name.replace("NVIDIA GeForce ", "").replace("NVIDIA ", "").replace("AMD ", "").replace(" GPU", ""))
}
