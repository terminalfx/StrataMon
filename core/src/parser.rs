//! Turns what a Strata server reports into a [`StrataSnapshot`]. /metrics is what Strata's own
//! Monitor reads, and /v1/telemetry adds the instance and the queue limits. Either may be missing,
//! so every value falls back to the other source or to `None`. The rules match
//! strata_cli/overview.py. Everything here came over a network, so it's treated as untrusted:
//! any field may be absent or the wrong type, and nothing here may panic.

use crate::models::*;
use serde_json::Value;

const MAX_TEXT_LENGTH: usize = 120;
const MAX_GPUS: usize = 16;
const MAX_REQUESTS: usize = 50;

static NOTHING: Value = Value::Null;

pub fn parse(metrics: Option<&Value>, telemetry: Option<&Value>) -> StrataSnapshot {
    let report = metrics.unwrap_or(&NOTHING);
    let live = section(report, "live");
    let engine = section(report, "engine");
    let hardware = section(report, "hardware");
    let machine = section(report, "hardware_static");
    let totals = section(report, "totals");

    let sample = telemetry.unwrap_or(&NOTHING);
    let instance = section(sample, "instance");
    let model = section(sample, "model");
    let work = section(sample, "work");
    let rates = section(sample, "rates");
    let ram = section(sample, "ram");
    let counts = section(sample, "totals");

    let waiting = sum(count(live, "queued"), count(live, "waiting")).or(sum(count(work, "queued"), count(work, "waiting")));
    let activity = if metrics.is_none() {
        infer_activity(model, rates, work, waiting.unwrap_or(0))
    } else {
        read_activity(live, engine, waiting.unwrap_or(0))
    };
    let busy = matches!(activity, StrataActivity::Reading | StrataActivity::Generating);

    let recent = read_recent(report, sample);
    let (progress_done, progress_total) = read_progress(live, activity);
    let answer_rate = number(live, "tok_s").or(number(rates, "answer_tok_s"));
    let prompt_rate = number(live, "prefill_tok_s_mean").or(number(rates, "prefill_tok_s"));
    let (speed, speed_basis) = read_speed(activity, answer_rate, prompt_rate, totals, &recent);

    let gpus = read_gpus(hardware, machine, sample);
    let now = number(report, "time").or(number(sample, "sampled_at"));
    let started_at = number(instance, "boot_time").or(number(totals, "since")).or(number(counts, "since"));

    StrataSnapshot {
        access: match (metrics, telemetry) {
            (Some(_), Some(_)) => TelemetryAccess::Full,
            (None, _) => TelemetryAccess::TelemetryOnly,
            _ => TelemetryAccess::MetricsOnly,
        },
        model: text(engine, "model").or(text(model, "id")),
        context_tokens: whole(engine, "max_context").or(whole(model, "context")),
        kv_cache: text(engine, "kv"),
        activity,
        // Kept apart rather than one standing in for the other: an id from each source would
        // never match, and a poll that missed /v1/telemetry would look like a restart.
        instance_id: text(instance, "id"),
        metrics_since: number(totals, "since"),
        draining: text(instance, "state").as_deref() == Some("draining"),
        uptime_seconds: now.zip(started_at).map(|(now, started)| now - started),
        progress_done,
        progress_total,
        context_used: read_context_used(live, busy, &recent),
        speed,
        speed_basis,
        live_speed: number(hardware, "tok_s").or(answer_rate).or(Some(0.0)),
        slots: count(work, "slots_effective").or(count(live, "parallel")).or(metrics.map(|_| 1)),
        running: count(live, "running").or(count(work, "active")).or(metrics.map(|_| i32::from(busy))),
        waiting,
        rejected: whole(work, "admission_rejected"),
        queue_timeouts: whole(work, "queue_timeouts"),
        gpu: summarize(&gpus, hardware, machine),
        gpus,
        pcie_generation: count(hardware, "gpu_pcie_gen").or(count(hardware, "gpu_pcie_gen_max")),
        pcie_width: count(hardware, "gpu_pcie_width"),
        pcie_to_gpu_megabytes_per_second: number(hardware, "gpu_pcie_rx_mb"),
        cpu_percent: number(hardware, "cpu_pct").or(number(hardware, "cpu")),
        cpu_cores: count(machine, "cores"),
        cpu_threads: count(machine, "threads"),
        ram_used_bytes: whole(hardware, "ram_used").or(whole(ram, "used")),
        ram_total_bytes: whole(hardware, "ram_total").or(whole(ram, "total")),
        disk_read_megabytes_per_second: number(hardware, "disk_read_mb"),
        disk_write_megabytes_per_second: number(hardware, "disk_write_mb"),
        experts_cached: whole(engine, "expert_slots"),
        expert_cache_mebibytes: number(engine, "expert_cache_mib"),
        total_requests: whole(totals, "requests").or(whole(counts, "requests")),
        total_output_tokens: whole(totals, "output_tokens").or(whole(counts, "output_tokens")),
        recent,
    }
}

fn read_activity(live: &Value, engine: &Value, waiting: i32) -> StrataActivity {
    let state = text(live, "state");
    match state.as_deref() {
        Some("error") => StrataActivity::Error,
        _ if is_set(engine, "error") => StrataActivity::Error,
        Some("reading") => StrataActivity::Reading,
        Some("generating") => StrataActivity::Generating,
        _ if waiting > 0 => StrataActivity::Queued,
        Some("unloaded") => StrataActivity::Unloaded,
        _ => StrataActivity::Idle,
    }
}

// A read-only key can't see /metrics, so the state has to come from the telemetry counters.
fn infer_activity(model: &Value, rates: &Value, work: &Value, waiting: i32) -> StrataActivity {
    if model.get("loaded").and_then(Value::as_bool) == Some(false) {
        StrataActivity::Unloaded
    } else if number(rates, "answer_tok_s").is_some() {
        StrataActivity::Generating
    } else if number(work, "active").is_some_and(|active| active > 0.0) {
        StrataActivity::Reading
    } else if waiting > 0 {
        StrataActivity::Queued
    } else {
        StrataActivity::Idle
    }
}

fn read_progress(live: &Value, activity: StrataActivity) -> (Option<i64>, Option<i64>) {
    let progress = |done: &str, total: &str| match whole(live, total) {
        Some(total) if total > 0 => Some((Some(whole(live, done).unwrap_or(0)), Some(total))),
        _ => None,
    };

    let found = match activity {
        StrataActivity::Reading => progress("prompt_read", "prompt_total"),
        StrataActivity::Generating => progress("generated", "max_tokens"),
        _ => None,
    };
    found.unwrap_or((None, None))
}

fn read_context_used(live: &Value, busy: bool, recent: &[RequestRow]) -> Option<i64> {
    if let (true, Some(prompt), Some(generated)) = (busy, whole(live, "prompt_tokens"), whole(live, "generated")) {
        return Some(prompt.saturating_add(generated));
    }

    let last = recent.first()?;
    Some(last.prompt_tokens?.saturating_add(last.output_tokens?))
}

fn read_speed(
    activity: StrataActivity,
    answer_rate: Option<f64>,
    prompt_rate: Option<f64>,
    totals: &Value,
    recent: &[RequestRow],
) -> (Option<f64>, SpeedBasis) {
    if let (StrataActivity::Generating, Some(now)) = (activity, answer_rate) {
        return (Some(now), SpeedBasis::Now);
    }

    if let (StrataActivity::Reading, Some(reading)) = (activity, prompt_rate) {
        return (Some(reading), SpeedBasis::ReadingPrompt);
    }

    // Idle: one request is noisy, so prefer the average over everything since Strata started.
    if let Some(average) = decode_speed(number(totals, "output_tokens"), number(totals, "decode_ms")) {
        return (Some(average), SpeedBasis::RunAverage);
    }

    match recent.first().and_then(|last| last.tokens_per_second) {
        Some(last) => (Some(last), SpeedBasis::LastRequest),
        None => (None, SpeedBasis::None),
    }
}

// A single token has no decode span, and reporting one showed a false 1,250 tok/s.
fn decode_speed(tokens: Option<f64>, decode_milliseconds: Option<f64>) -> Option<f64> {
    match (tokens, decode_milliseconds) {
        // A decode span small enough to underflow when halved would read as an infinite speed.
        (Some(tokens), Some(ms)) if tokens >= 2.0 && ms > 0.0 => {
            let speed = (tokens / (ms / 1000.0) * 10.0).round() / 10.0;
            speed.is_finite().then_some(speed)
        }
        _ => None,
    }
}

fn read_gpus(hardware: &Value, machine: &Value, sample: &Value) -> Vec<GpuReading> {
    let devices: Vec<&Value> = rows(sample, "devices", MAX_GPUS).collect();
    let sole_name = text(machine, "gpu_name");

    let read = |row: &Value, position: usize, count_of: usize| {
        let index = count(row, "index").unwrap_or(position as i32);
        let device = devices.iter().find(|d| count(d, "index") == Some(index));
        GpuReading {
            index,
            name: text(row, "name")
                .or_else(|| device.and_then(|d| text(d, "name")))
                .or_else(|| if count_of == 1 { sole_name.clone() } else { None }),
            load_percent: number(row, "util"),
            memory_used_bytes: whole(row, "mem_used"),
            memory_total_bytes: whole(row, "mem_total"),
            temperature_c: number(row, "temp"),
            power_watts: number(row, "power"),
            power_limit_watts: number(row, "power_limit")
                .or(if count_of == 1 { number(hardware, "gpu_power_limit") } else { None }),
        }
    };

    let cards: Vec<&Value> = rows(hardware, "gpus", MAX_GPUS).collect();
    if !cards.is_empty() {
        return cards.iter().enumerate().map(|(i, row)| read(row, i, cards.len())).collect();
    }

    // One card: /metrics reports it in the aggregate fields rather than as a list.
    if sole_name.is_some() || number(hardware, "gpu_util").is_some() {
        return vec![GpuReading {
            name: sole_name.clone(),
            load_percent: number(hardware, "gpu_util"),
            memory_used_bytes: whole(hardware, "gpu_mem_used"),
            memory_total_bytes: whole(hardware, "gpu_mem_total")
                .or(whole(machine, "gpu_vram_total_mib").map(|mib| mib.saturating_mul(1 << 20))),
            temperature_c: number(hardware, "gpu_temp"),
            power_watts: number(hardware, "gpu_power"),
            power_limit_watts: number(hardware, "gpu_power_limit"),
            ..GpuReading::default()
        }];
    }

    devices.iter().enumerate().map(|(i, row)| read(row, i, devices.len())).collect()
}

fn summarize(gpus: &[GpuReading], hardware: &Value, machine: &Value) -> GpuReading {
    if gpus.len() < 2 {
        return gpus.first().cloned().unwrap_or_default();
    }

    GpuReading {
        name: text(machine, "gpu_name").or(gpus[0].name.clone()),
        load_percent: most(gpus.iter().map(|g| g.load_percent)),
        memory_used_bytes: total_whole(gpus.iter().map(|g| g.memory_used_bytes)),
        memory_total_bytes: total_whole(gpus.iter().map(|g| g.memory_total_bytes)),
        temperature_c: most(gpus.iter().map(|g| g.temperature_c)),
        power_watts: total(gpus.iter().map(|g| g.power_watts)),
        power_limit_watts: total(gpus.iter().map(|g| g.power_limit_watts)).or(number(hardware, "gpu_power_limit")),
        ..GpuReading::default()
    }
}

fn read_recent(report: &Value, sample: &Value) -> Vec<RequestRow> {
    if report.is_object() {
        rows(report, "requests", MAX_REQUESTS).map(read_request).collect()
    } else {
        rows(sample, "recent", MAX_REQUESTS).map(read_request).collect()
    }
}

fn read_request(row: &Value) -> RequestRow {
    let finish = text(row, "finish");
    RequestRow {
        started_at: number(row, "time"),
        outcome: if is_set(row, "error") {
            RequestOutcome::Failed
        } else {
            match finish.as_deref() {
                Some("error") => RequestOutcome::Failed,
                Some("timeout") => RequestOutcome::TimedOut,
                Some("cancel" | "cancelled" | "abort" | "disconnect") => RequestOutcome::Stopped,
                _ => RequestOutcome::Done,
            }
        },
        prompt_tokens: whole(row, "prompt_tokens"),
        reused_tokens: whole(row, "reused"),
        output_tokens: whole(row, "output_tokens"),
        tokens_per_second: number(row, "decode_tok_s").or(decode_speed(number(row, "output_tokens"), number(row, "decode_ms"))),
        duration_seconds: number(row, "duration_s"),
    }
}

fn most(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values.flatten().fold(None, |best, value| Some(best.map_or(value, |b: f64| b.max(value))))
}

fn total(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values.flatten().fold(None, |sum, value| Some(sum.unwrap_or(0.0) + value))
}

fn total_whole(values: impl Iterator<Item = Option<i64>>) -> Option<i64> {
    values.flatten().fold(None, |sum: Option<i64>, value| Some(sum.unwrap_or(0).saturating_add(value)))
}

fn sum(first: Option<i32>, second: Option<i32>) -> Option<i32> {
    match (first, second) {
        (None, None) => None,
        _ => Some(first.unwrap_or(0).saturating_add(second.unwrap_or(0))),
    }
}

fn section<'a>(parent: &'a Value, name: &str) -> &'a Value {
    match parent.get(name) {
        Some(value) if value.is_object() => value,
        _ => &NOTHING,
    }
}

fn rows<'a>(parent: &'a Value, name: &str, limit: usize) -> impl Iterator<Item = &'a Value> {
    parent.get(name).and_then(Value::as_array).into_iter().flatten().filter(|row| row.is_object()).take(limit)
}

fn number(parent: &Value, name: &str) -> Option<f64> {
    // Legitimate readings stay far under 1e15 (byte counts reach ~1e11, Unix seconds ~1e9).
    // Anything bigger is a lie or a parser test, and it turns sums of readings into infinity,
    // so an absurd value reads as no reading at all.
    parent.get(name).and_then(Value::as_f64).filter(|value| value.is_finite() && value.abs() < 1e15)
}

fn whole(parent: &Value, name: &str) -> Option<i64> {
    number(parent, name).filter(|value| value.abs() < i64::MAX as f64 / 2.0).map(|value| value.round() as i64)
}

fn count(parent: &Value, name: &str) -> Option<i32> {
    number(parent, name).filter(|value| (0.0..=f64::from(i32::MAX)).contains(value)).map(|value| value.round() as i32)
}

// The same test as Python's truthiness, which is how Strata's Monitor reads these fields.
fn is_set(parent: &Value, name: &str) -> bool {
    match parent.get(name) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Number(number)) => number.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::Object(fields)) => !fields.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        _ => false,
    }
}

fn text(parent: &Value, name: &str) -> Option<String> {
    let value = parent.get(name)?.as_str()?;

    // Strips control and bidi formatting characters, so a label can't disguise itself.
    Some(value.chars().take(MAX_TEXT_LENGTH).map(|c| if is_hidden(c) { ' ' } else { c }).collect())
}

fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}' | '\u{180E}'
            | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{E0001}' | '\u{E0020}'..='\u{E007F}')
}
