//! One reading of a Strata server, built from what Strata reports about itself. A `None` means
//! Strata didn't report that value, which is different from zero, so the UI must show n/a.

/// What the model is doing, in the terms of Strata's own Monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrataActivity {
    #[default]
    Idle,
    Reading,
    Generating,
    Queued,
    Unloaded,
    Error,
}

/// Where the headline speed came from. An idle model has no live rate, so it falls back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeedBasis {
    #[default]
    None,
    Now,
    ReadingPrompt,
    RunAverage,
    LastRequest,
}

/// How much of Strata's reporting could be read from this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TelemetryAccess {
    /// Both /metrics and /v1/telemetry.
    #[default]
    Full,
    /// /v1/telemetry only, which is all a read-only telemetry key allows.
    TelemetryOnly,
    /// /metrics only, on a Strata too old to have /v1/telemetry.
    MetricsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequestOutcome {
    #[default]
    Done,
    Stopped,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StrataSnapshot {
    pub access: TelemetryAccess,

    pub model: Option<String>,
    pub context_tokens: Option<i64>,

    /// How the KV cache is stored, as Strata names it: `q8_0`, `int8`, `q4_k_m`, ...
    pub kv_cache: Option<String>,

    pub activity: StrataActivity,

    /// Identifies one run of the server, from /v1/telemetry. It changes when Strata restarts.
    pub instance_id: Option<String>,

    /// When /metrics says its totals start, which is when this run of Strata began. It tells
    /// runs apart when there's no `instance_id`, on a Strata too old to have /v1/telemetry.
    pub metrics_since: Option<f64>,

    /// Strata is shutting down and refusing new requests.
    pub draining: bool,

    pub uptime_seconds: Option<f64>,

    pub progress_done: Option<i64>,
    pub progress_total: Option<i64>,
    pub context_used: Option<i64>,

    pub speed: Option<f64>,
    pub speed_basis: SpeedBasis,

    /// The answer rate right now, which is zero when idle. This is what the trend plots.
    pub live_speed: Option<f64>,

    pub slots: Option<i32>,
    pub running: Option<i32>,
    pub waiting: Option<i32>,
    pub rejected: Option<i64>,
    pub queue_timeouts: Option<i64>,

    /// The whole machine: busiest and hottest card, memory and power added up.
    pub gpu: GpuReading,

    pub gpus: Vec<GpuReading>,
    pub pcie_generation: Option<i32>,
    pub pcie_width: Option<i32>,
    pub pcie_to_gpu_megabytes_per_second: Option<f64>,

    pub cpu_percent: Option<f64>,
    pub cpu_cores: Option<i32>,
    pub cpu_threads: Option<i32>,

    pub ram_used_bytes: Option<i64>,
    pub ram_total_bytes: Option<i64>,

    pub disk_read_megabytes_per_second: Option<f64>,
    pub disk_write_megabytes_per_second: Option<f64>,

    pub experts_cached: Option<i64>,
    pub expert_cache_mebibytes: Option<f64>,

    pub total_requests: Option<i64>,
    pub total_output_tokens: Option<i64>,

    /// Newest first.
    pub recent: Vec<RequestRow>,
}

impl StrataSnapshot {
    pub fn ram_percent(&self) -> Option<f64> {
        share(self.ram_used_bytes, self.ram_total_bytes)
    }

    pub fn context_percent(&self) -> Option<f64> {
        share(self.context_used, self.context_tokens)
    }

    pub fn progress_percent(&self) -> Option<f64> {
        share(self.progress_done, self.progress_total)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpuReading {
    pub index: i32,
    pub name: Option<String>,
    pub load_percent: Option<f64>,
    pub memory_used_bytes: Option<i64>,
    pub memory_total_bytes: Option<i64>,
    pub temperature_c: Option<f64>,
    pub power_watts: Option<f64>,
    pub power_limit_watts: Option<f64>,
}

impl GpuReading {
    pub fn memory_percent(&self) -> Option<f64> {
        share(self.memory_used_bytes, self.memory_total_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RequestRow {
    /// When the request started, in Unix seconds on the server's clock.
    pub started_at: Option<f64>,
    pub outcome: RequestOutcome,
    pub prompt_tokens: Option<i64>,
    pub reused_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub tokens_per_second: Option<f64>,
    pub duration_seconds: Option<f64>,
}

fn share(part: Option<i64>, whole: Option<i64>) -> Option<f64> {
    match (part, whole) {
        (Some(part), Some(whole)) if whole > 0 => Some(100.0 * part as f64 / whole as f64),
        _ => None,
    }
}
