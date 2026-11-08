//! Turns a stream of server states into alerts. A reading must stay over its limit for the
//! sustain period before alerting, and must drop clearly below it to resolve, so a value hovering
//! at the limit doesn't flap. Things Strata reports as already having happened, such as a restart
//! or a failed request, become one-off notices.

use crate::client::PollFailure;
use crate::config::{AlertThresholds, ServerConfig};
use crate::format;
use crate::models::{RequestOutcome, StrataActivity, StrataSnapshot, TelemetryAccess};
use crate::state::{ServerState, ServerStatus, Time};
use std::collections::{HashMap, HashSet};

/// How far below its limit a reading must fall before its alert resolves.
pub const RESOLVE_MARGIN: f64 = 5.0;

/// A limit that isn't a number, or isn't above zero, isn't a threshold: left alone it puts
/// every ordinary reading over the line forever, where nothing can pull one back under. It
/// becomes a ceiling no reading reaches instead, so the alert never starts, and one raised
/// under a real limit resolves the moment a reading comes in under the ceiling. A hand-edited
/// settings file is the only way to get here; the settings form clamps its own fields.
fn usable(thresholds: AlertThresholds) -> AlertThresholds {
    fn ceiling(limit: f64) -> f64 {
        if limit.is_finite() && limit > 0.0 { limit } else { f64::INFINITY }
    }

    AlertThresholds {
        ram_percent: ceiling(thresholds.ram_percent),
        gpu_temperature_c: ceiling(thresholds.gpu_temperature_c),
        sustain_seconds: thresholds.sustain_seconds.max(0),
        queue_wait_seconds: thresholds.queue_wait_seconds.max(0),
    }
}

pub const OFFLINE_KEY: &str = "offline";
const ENGINE_ERROR_KEY: &str = "engine-error";
const DRAINING_KEY: &str = "draining";
const QUEUE_KEY: &str = "queue";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertSeverity {
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertChange {
    Raised,
    Resolved,
    /// Something happened and is already over, such as a restart. Nothing to resolve.
    Notice,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertEvent {
    pub server_id: String,
    pub server_name: String,
    pub key: String,
    pub severity: AlertSeverity,
    pub change: AlertChange,
    pub message: String,
    pub at: Time,
}

#[derive(Default)]
struct Tracked {
    over_since: Option<Time>,
    raised: Option<AlertEvent>,

    /// What the reading is called in messages, like "GPU 1 temperature".
    what: String,
}

/// What the last reading said, to spot what changed since. A value the latest poll didn't
/// report is carried over from the one before, so a poll that missed one of Strata's two
/// sources doesn't make the next one look like a change.
struct Previous {
    access: TelemetryAccess,
    instance_id: Option<String>,
    metrics_since: Option<f64>,
    rejected: Option<i64>,
    queue_timeouts: Option<i64>,
    failed_requests: HashMap<Option<u64>, usize>,
}

pub struct AlertEngine {
    thresholds: AlertThresholds,
    tracked: HashMap<(String, String), Tracked>,
    previous: HashMap<String, Previous>,
}

/// Something that is simply true or false, unlike a reading with a margin around its limit.
struct Condition<'a> {
    key: &'a str,
    active: Option<bool>,
    sustain: f64,
    severity: AlertSeverity,
    raised: String,
    resolved: String,
}

impl AlertEngine {
    pub fn new(thresholds: AlertThresholds) -> Self {
        Self { thresholds: usable(thresholds), tracked: HashMap::new(), previous: HashMap::new() }
    }

    /// Alerts in force, most severe first, then oldest first.
    pub fn active(&self) -> Vec<&AlertEvent> {
        let mut alerts: Vec<&AlertEvent> = self.tracked.values().filter_map(|t| t.raised.as_ref()).collect();
        alerts.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.at.total_cmp(&b.at)));
        alerts
    }

    pub fn evaluate(&mut self, server: &ServerConfig, state: &ServerState, now: Time) -> Vec<AlertEvent> {
        let mut events = Vec::new();

        match state.status {
            ServerStatus::Connecting => {}
            ServerStatus::Offline | ServerStatus::Rejected => {
                self.raise_not_reporting(server, state, now, &mut events);
                self.restart_timers(server);
            }
            ServerStatus::Online => {
                self.resolve_not_reporting(server, now, &mut events);
                match &state.latest {
                    // A failed poll that isn't enough to call the server offline still says
                    // nothing new. The last good snapshot would let a timer run out on a
                    // reading nobody confirmed.
                    _ if state.last_poll_failed() => self.restart_timers(server),
                    Some(snapshot) => self.evaluate_readings(server, snapshot, now, &mut events),
                    None => {}
                }
            }
        }

        events
    }

    /// Readings are unknown now. Pending timers restart, but alerts already raised stay up,
    /// since we can't tell that they've cleared.
    fn restart_timers(&mut self, server: &ServerConfig) {
        for (key, tracked) in &mut self.tracked {
            if key.0 == server.id {
                tracked.over_since = None;
            }
        }
    }

    fn evaluate_readings(&mut self, server: &ServerConfig, snapshot: &StrataSnapshot, now: Time, events: &mut Vec<AlertEvent>) {
        let name = &server.name;
        let mut seen: HashSet<String> = [OFFLINE_KEY, ENGINE_ERROR_KEY, DRAINING_KEY, QUEUE_KEY].map(String::from).into();

        // An engine error is only in /metrics, and draining only in /v1/telemetry. A poll that
        // couldn't read the one that says can't tell either way.
        let has_metrics = matches!(snapshot.access, TelemetryAccess::Full | TelemetryAccess::MetricsOnly);
        let has_telemetry = matches!(snapshot.access, TelemetryAccess::Full | TelemetryAccess::TelemetryOnly);

        let conditions = [
            Condition {
                key: ENGINE_ERROR_KEY,
                active: has_metrics.then_some(snapshot.activity == StrataActivity::Error),
                sustain: 0.0,
                severity: AlertSeverity::Critical,
                raised: format!("{name}: the model is unavailable"),
                resolved: format!("{name}: the model is available again"),
            },
            Condition {
                key: DRAINING_KEY,
                active: has_telemetry.then_some(snapshot.draining),
                sustain: 0.0,
                severity: AlertSeverity::Critical,
                raised: format!("{name}: Strata is shutting down and refusing new requests"),
                resolved: format!("{name}: Strata is taking requests again"),
            },
            Condition {
                key: QUEUE_KEY,
                active: snapshot.waiting.map(|waiting| waiting > 0),
                sustain: f64::from(self.thresholds.queue_wait_seconds),
                severity: AlertSeverity::Warning,
                raised: format!(
                    "{name}: requests have been waiting over {}",
                    format::duration(f64::from(self.thresholds.queue_wait_seconds))
                ),
                resolved: format!("{name}: the queue has cleared"),
            },
        ];
        for condition in conditions {
            self.track_condition(server, condition, now, events);
        }

        seen.insert("ram".to_string());
        let ram_limit = self.thresholds.ram_percent;
        self.track(server, "ram", snapshot.ram_percent(), ram_limit, "RAM", format::percent, now, events);

        let temperature_limit = self.thresholds.gpu_temperature_c;
        for gpu in &snapshot.gpus {
            let key = format!("gpu-temperature:{}", gpu.index);
            let what = if snapshot.gpus.len() > 1 { format!("GPU {} temperature", gpu.index) } else { "GPU temperature".to_string() };
            self.track(server, &key, gpu.temperature_c, temperature_limit, &what, format::temperature, now, events);
            seen.insert(key);
        }

        // A GPU that disappears would otherwise leave its alert up forever. But a poll that
        // read different sources than the last one can't tell a card that's gone from a card
        // the missing source would have listed, so its readings are only unknown for now.
        let same_sources = self.previous.get(&server.id).is_none_or(|previous| previous.access == snapshot.access);
        let unlisted: Vec<(String, String)> =
            self.tracked.keys().filter(|key| key.0 == server.id && !seen.contains(&key.1)).cloned().collect();
        for key in unlisted {
            if !same_sources {
                if let Some(tracked) = self.tracked.get_mut(&key) {
                    tracked.over_since = None;
                }
                continue;
            }

            if let Some(Tracked { raised: Some(raised), what, .. }) = self.tracked.remove(&key) {
                let what = if what.is_empty() { key.1.clone() } else { what };
                events.push(AlertEvent {
                    change: AlertChange::Resolved,
                    message: format!("{name}: {what} is no longer reported"),
                    at: now,
                    ..raised
                });
            }
        }

        self.report_changes(server, snapshot, now, events);
    }

    fn report_changes(&mut self, server: &ServerConfig, snapshot: &StrataSnapshot, now: Time, events: &mut Vec<AlertEvent>) {
        let name = &server.name;
        let mut notice = |key: &str, message: String| {
            events.push(AlertEvent {
                server_id: server.id.clone(),
                server_name: name.clone(),
                key: key.to_string(),
                severity: AlertSeverity::Warning,
                change: AlertChange::Notice,
                message,
                at: now,
            });
        };

        // Counted by start time, and the count matters: Strata's clock only reads to the second,
        // so two failures that began in the same second are two failures. Rows that don't say
        // when they began are kept under `None` instead of dropped without a word.
        let mut failed: HashMap<Option<u64>, usize> = HashMap::new();
        for row in &snapshot.recent {
            if matches!(row.outcome, RequestOutcome::Failed | RequestOutcome::TimedOut) {
                *failed.entry(row.started_at.map(f64::to_bits)).or_default() += 1;
            }
        }

        // The first reading is a baseline. What it shows happened before anyone was watching.
        let previous = self.previous.get(&server.id);
        let restarted = previous.is_some_and(|previous| match (&snapshot.instance_id, &previous.instance_id) {
            (Some(now_id), Some(then_id)) => now_id != then_id,
            // Telemetry's id is missing on one side, so the run is told apart by when /metrics
            // says it started. Comparing an id from one source to one from the other would
            // call every poll that missed /v1/telemetry a restart.
            _ => matches!((snapshot.metrics_since, previous.metrics_since), (Some(now_since), Some(then_since)) if now_since != then_since),
        });

        if let Some(previous) = previous {
            if restarted {
                // Counters start again from zero, so comparing them would mean nothing.
                notice("restart", format!("{name}: Strata restarted"));
            } else {
                if let Some(rejected) = increase(previous.rejected, snapshot.rejected) {
                    notice("rejected", format!("{name}: {} turned away because the queue was full", requests(rejected)));
                }

                if let Some(timed_out) = increase(previous.queue_timeouts, snapshot.queue_timeouts) {
                    notice("queue-timeout", format!("{name}: {} timed out in the queue", requests(timed_out)));
                }

                let new_failures: i64 = failed
                    .iter()
                    .map(|(when, count)| count.saturating_sub(previous.failed_requests.get(when).copied().unwrap_or(0)) as i64)
                    .sum();
                if new_failures > 0 {
                    notice("request-failed", format!("{name}: {} failed", requests(new_failures)));
                }
            }
        }

        // After a restart nothing from the old run is worth comparing against.
        let carried = previous.filter(|_| !restarted);
        let next = Previous {
            access: snapshot.access,
            instance_id: snapshot.instance_id.clone().or_else(|| carried.and_then(|previous| previous.instance_id.clone())),
            metrics_since: snapshot.metrics_since.or(carried.and_then(|previous| previous.metrics_since)),
            rejected: snapshot.rejected.or(carried.and_then(|previous| previous.rejected)),
            queue_timeouts: snapshot.queue_timeouts.or(carried.and_then(|previous| previous.queue_timeouts)),
            failed_requests: failed,
        };
        self.previous.insert(server.id.clone(), next);
    }

    fn track_condition(&mut self, server: &ServerConfig, condition: Condition, now: Time, events: &mut Vec<AlertEvent>) {
        let tracked = self.tracked.entry((server.id.clone(), condition.key.to_string())).or_default();

        match condition.active {
            // Unknown can't raise an alert or prove one has cleared.
            None => tracked.over_since = None,
            Some(true) => {
                let since = *tracked.over_since.get_or_insert(now);
                if tracked.raised.is_none() && now - since >= condition.sustain {
                    let alert = new_alert(server, condition.key, condition.severity, condition.raised, now);
                    tracked.raised = Some(alert.clone());
                    events.push(alert);
                }
            }
            Some(false) => {
                tracked.over_since = None;
                if let Some(raised) = tracked.raised.take() {
                    events.push(AlertEvent { change: AlertChange::Resolved, message: condition.resolved, at: now, ..raised });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn track(
        &mut self,
        server: &ServerConfig,
        key: &str,
        value: Option<f64>,
        limit: f64,
        what: &str,
        show: fn(Option<f64>) -> String,
        now: Time,
        events: &mut Vec<AlertEvent>,
    ) {
        let sustain = f64::from(self.thresholds.sustain_seconds);
        let tracked = self.tracked.entry((server.id.clone(), key.to_string())).or_default();
        if tracked.what != what {
            tracked.what = what.to_string();
        }

        // An unknown reading can't raise an alert or prove one has cleared.
        let Some(reading) = value else {
            tracked.over_since = None;
            return;
        };

        if tracked.raised.is_some() {
            // A limit smaller than the margin would need a reading below zero to ever clear,
            // which pins the alert on forever. Small limits clear at their own middle instead.
            if reading <= limit - RESOLVE_MARGIN.min(limit * 0.5) {
                tracked.over_since = None;
                if let Some(raised) = tracked.raised.take() {
                    events.push(AlertEvent {
                        change: AlertChange::Resolved,
                        message: format!("{}: {what} back to {}", server.name, show(Some(reading))),
                        at: now,
                        ..raised
                    });
                }
            }
            return;
        }

        if reading < limit {
            tracked.over_since = None;
            return;
        }

        let since = *tracked.over_since.get_or_insert(now);
        if now - since >= sustain {
            let message = format!("{}: {what} at {} (limit {})", server.name, show(Some(reading)), show(Some(limit)));
            let alert = new_alert(server, key, AlertSeverity::Warning, message, now);
            tracked.raised = Some(alert.clone());
            events.push(alert);
        }
    }

    fn raise_not_reporting(&mut self, server: &ServerConfig, state: &ServerState, now: Time, events: &mut Vec<AlertEvent>) {
        let tracked = self.tracked.entry((server.id.clone(), OFFLINE_KEY.to_string())).or_default();
        if tracked.raised.is_some() {
            return;
        }

        let message = if state.status == ServerStatus::Rejected {
            format!("{} can't be read. {}", server.name, state.last_error.as_deref().unwrap_or(""))
        } else if state.last_failure == PollFailure::NotListening {
            format!("{}: Strata isn't answering, but the PC is on", server.name)
        } else {
            format!("{} is offline", server.name)
        };

        let alert = new_alert(server, OFFLINE_KEY, AlertSeverity::Critical, message, now);
        tracked.raised = Some(alert.clone());
        events.push(alert);
    }

    fn resolve_not_reporting(&mut self, server: &ServerConfig, now: Time, events: &mut Vec<AlertEvent>) {
        let tracked = self.tracked.entry((server.id.clone(), OFFLINE_KEY.to_string())).or_default();
        if let Some(raised) = tracked.raised.take() {
            events.push(AlertEvent {
                change: AlertChange::Resolved,
                message: format!("{} is back online", server.name),
                at: now,
                ..raised
            });
        }
    }
}

fn new_alert(server: &ServerConfig, key: &str, severity: AlertSeverity, message: String, at: Time) -> AlertEvent {
    AlertEvent {
        server_id: server.id.clone(),
        server_name: server.name.clone(),
        key: key.to_string(),
        severity,
        change: AlertChange::Raised,
        message,
        at,
    }
}

fn increase(before: Option<i64>, after: Option<i64>) -> Option<i64> {
    after.zip(before).map(|(after, before)| after - before).filter(|change| *change > 0)
}

fn requests(count: i64) -> String {
    if count == 1 { "1 request".to_string() } else { format!("{count} requests") }
}
