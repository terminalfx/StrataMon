//! The samples folder holds real responses. Most came from Strata's own server code with its mock
//! engine, and two-gpus-metrics.json is a capture from real hardware in Strata's benchmark results.

use serde_json::Value;
use stratamon_core::format;
use stratamon_core::models::*;
use stratamon_core::parser::parse;

const WORKING_METRICS: &str = include_str!("samples/working-metrics.json");
const WORKING_TELEMETRY: &str = include_str!("samples/working-telemetry.json");
const BATCH_METRICS: &str = include_str!("samples/batch-metrics.json");
const BATCH_TELEMETRY: &str = include_str!("samples/batch-telemetry.json");
const IDLE_METRICS: &str = include_str!("samples/idle-metrics.json");
const IDLE_TELEMETRY: &str = include_str!("samples/idle-telemetry.json");
const TWO_GPUS: &str = include_str!("samples/two-gpus-metrics.json");

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn full(metrics: &str, telemetry: &str) -> StrataSnapshot {
    parse(Some(&value(metrics)), Some(&value(telemetry)))
}

fn from_metrics(text: &str) -> StrataSnapshot {
    parse(Some(&value(text)), None)
}

fn from_telemetry(text: &str) -> StrataSnapshot {
    parse(None, Some(&value(text)))
}

#[test]
fn reads_a_server_in_the_middle_of_an_answer() {
    let snapshot = full(WORKING_METRICS, WORKING_TELEMETRY);

    assert_eq!(snapshot.access, TelemetryAccess::Full);
    assert_eq!(snapshot.model.as_deref(), Some("qwen3.8-flash-next-mock-solo"));
    assert_eq!(snapshot.context_tokens, Some(32768));
    assert_eq!(snapshot.kv_cache.as_deref(), Some("q8_0"));
    assert_eq!(snapshot.activity, StrataActivity::Generating);
    assert_eq!(snapshot.speed, Some(49.3));
    assert_eq!(snapshot.speed_basis, SpeedBasis::Now);
    assert_eq!(snapshot.progress_done, Some(40));
    assert_eq!(snapshot.progress_total, Some(60));
    assert_eq!(snapshot.context_used, Some(523));
    assert_eq!(snapshot.slots, Some(1));
    assert_eq!(snapshot.running, Some(1));
    assert_eq!(snapshot.waiting, Some(0));
    assert_eq!(snapshot.instance_id.as_deref(), Some("e38056aec3a84908a9662747b3eaabdc"));
    assert!(!snapshot.draining);
    assert_eq!(snapshot.recent.len(), 5);
}

#[test]
fn reads_the_one_gpu_from_the_aggregate_fields() {
    let snapshot = full(WORKING_METRICS, WORKING_TELEMETRY);

    assert_eq!(snapshot.gpus.len(), 1);
    let gpu = &snapshot.gpus[0];
    assert_eq!(gpu.name.as_deref(), Some("NVIDIA GeForce RTX 5070 Laptop GPU"));
    assert!(gpu.load_percent.is_some());
    assert_eq!(*gpu, snapshot.gpu);
    assert_eq!(gpu.power_limit_watts, Some(115.0));
    assert!(snapshot.pcie_generation.is_some());
}

#[test]
fn reads_a_server_running_parallel_slots() {
    let snapshot = full(BATCH_METRICS, BATCH_TELEMETRY);

    assert_eq!(snapshot.slots, Some(4));
    assert_eq!(snapshot.running, Some(4));
    assert_eq!(snapshot.speed, Some(196.7));
}

#[test]
fn an_idle_server_that_has_served_nothing_has_no_speed_rather_than_zero() {
    let snapshot = full(IDLE_METRICS, IDLE_TELEMETRY);

    assert_eq!(snapshot.activity, StrataActivity::Idle);
    assert_eq!(snapshot.speed, None);
    assert_eq!(snapshot.speed_basis, SpeedBasis::None);
    assert_eq!(snapshot.progress_percent(), None);
    assert_eq!(snapshot.context_used, None);
    assert_eq!(snapshot.live_speed, Some(0.0));
}

#[test]
fn works_out_uptime_from_the_servers_own_clock() {
    let uptime = full(IDLE_METRICS, IDLE_TELEMETRY).uptime_seconds.unwrap();
    assert!((9.0..=10.0).contains(&uptime), "{uptime}");
}

#[test]
fn reads_real_hardware_with_two_gpus_the_way_strata_sums_them() {
    let snapshot = from_metrics(TWO_GPUS);

    assert_eq!(snapshot.access, TelemetryAccess::MetricsOnly);
    assert_eq!(snapshot.kv_cache.as_deref(), Some("int8"));
    assert_eq!(snapshot.gpus.len(), 2);
    assert_eq!(snapshot.gpus.iter().map(|g| g.index).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(snapshot.gpu.load_percent, Some(100.0));
    assert_eq!(snapshot.gpu.temperature_c, Some(69.0));
    assert_eq!(snapshot.gpu.memory_used_bytes, Some(16785604608 + 17013014528));
    assert!((snapshot.gpu.power_watts.unwrap() - 126.632).abs() < 0.001);
    assert_eq!(snapshot.gpu.power_limit_watts, Some(330.0));
    assert_eq!(snapshot.cpu_percent, Some(1.9));
    assert_eq!(snapshot.cpu_cores, Some(32));
    assert_eq!(snapshot.cpu_threads, Some(64));
    assert_eq!(snapshot.experts_cached, Some(7761));
    assert_eq!(snapshot.expert_cache_mebibytes, Some(17916.0));
    assert_eq!(snapshot.total_requests, Some(16));
}

#[test]
fn an_idle_server_shows_the_average_of_the_run_from_its_totals() {
    let snapshot = from_metrics(TWO_GPUS);

    assert_eq!(snapshot.speed_basis, SpeedBasis::RunAverage);
    assert_eq!(snapshot.speed, Some(54.6));
}

#[test]
fn reads_a_finished_request() {
    let snapshot = from_metrics(TWO_GPUS);
    let request = &snapshot.recent[0];

    assert_eq!(request.outcome, RequestOutcome::Done);
    assert_eq!(request.prompt_tokens, Some(125449));
    assert_eq!(request.reused_tokens, Some(16384));
    assert_eq!(request.output_tokens, Some(9));
    assert_eq!(request.tokens_per_second, Some(35.7));
    assert_eq!(request.duration_seconds, Some(52.2));
}

#[test]
fn a_read_only_key_still_gets_model_gpu_ram_and_requests() {
    let snapshot = from_telemetry(WORKING_TELEMETRY);

    assert_eq!(snapshot.access, TelemetryAccess::TelemetryOnly);
    assert_eq!(snapshot.model.as_deref(), Some("qwen3.8-flash-next-mock-solo"));
    assert_eq!(snapshot.activity, StrataActivity::Generating);
    assert_eq!(snapshot.speed, Some(49.3));
    assert_eq!(snapshot.gpus.len(), 1);
    assert_eq!(snapshot.gpus[0].name.as_deref(), Some("NVIDIA GeForce RTX 5070 Laptop GPU"));
    assert_eq!(snapshot.ram_used_bytes, Some(17810481152));
    assert_eq!(snapshot.running, Some(1));
    assert_eq!(snapshot.recent.len(), 5);

    // These only exist in /metrics, so they must be unknown, not zero.
    assert_eq!(snapshot.cpu_percent, None);
    assert_eq!(snapshot.disk_read_megabytes_per_second, None);
    assert_eq!(snapshot.experts_cached, None);
}

#[test]
fn works_out_the_state_from_telemetry_alone() {
    for (json, expected) in [
        (r#"{"model":{"loaded":false},"work":{"active":0}}"#, StrataActivity::Unloaded),
        (r#"{"model":{"loaded":true},"work":{"active":1},"rates":{"answer_tok_s":40.5}}"#, StrataActivity::Generating),
        (r#"{"model":{"loaded":true},"work":{"active":1},"rates":{"answer_tok_s":null}}"#, StrataActivity::Reading),
        (r#"{"model":{"loaded":true},"work":{"active":0,"queued":2}}"#, StrataActivity::Queued),
        (r#"{"model":{"loaded":true},"work":{"active":0}}"#, StrataActivity::Idle),
    ] {
        assert_eq!(from_telemetry(json).activity, expected, "{json}");
    }
}

#[test]
fn reads_the_state_from_metrics() {
    for (json, expected) in [
        (r#"{"live":{"state":"idle"}}"#, StrataActivity::Idle),
        (r#"{"live":{"state":"reading"}}"#, StrataActivity::Reading),
        (r#"{"live":{"state":"generating"}}"#, StrataActivity::Generating),
        (r#"{"live":{"state":"unloaded"}}"#, StrataActivity::Unloaded),
        (r#"{"live":{"state":"idle","queued":1}}"#, StrataActivity::Queued),
        (r#"{"live":{"state":"idle","waiting":1}}"#, StrataActivity::Queued),
        (r#"{"live":{"state":"error"}}"#, StrataActivity::Error),
        (r#"{"live":{"state":"idle"},"engine":{"error":"model file missing"}}"#, StrataActivity::Error),
        (r#"{"live":{"state":"idle"},"engine":{"error":""}}"#, StrataActivity::Idle),
    ] {
        assert_eq!(from_metrics(json).activity, expected, "{json}");
    }
}

#[test]
fn shows_prompt_progress_while_reading() {
    let snapshot = from_metrics(r#"{"live":{"state":"reading","prompt_read":2500,"prompt_total":10000,"prefill_tok_s_mean":310.5}}"#);

    assert_eq!(snapshot.progress_percent(), Some(25.0));
    assert_eq!(snapshot.speed, Some(310.5));
    assert_eq!(snapshot.speed_basis, SpeedBasis::ReadingPrompt);
}

#[test]
fn falls_back_to_the_last_request_when_the_run_has_no_decode_time() {
    let snapshot = from_metrics(r#"{"live":{"state":"idle"},"totals":{"output_tokens":0},"requests":[{"output_tokens":100,"decode_ms":2000}]}"#);

    assert_eq!(snapshot.speed_basis, SpeedBasis::LastRequest);
    assert_eq!(snapshot.speed, Some(50.0));
}

#[test]
fn a_single_token_has_no_speed_because_it_has_no_decode_span() {
    let snapshot = from_metrics(r#"{"live":{"state":"idle"},"requests":[{"output_tokens":1,"decode_ms":0.8}]}"#);
    assert_eq!(snapshot.recent[0].tokens_per_second, None);
}

#[test]
fn an_idle_server_measures_context_from_its_last_request() {
    let snapshot = from_metrics(r#"{"live":{"state":"idle"},"engine":{"max_context":1000},"requests":[{"prompt_tokens":200,"output_tokens":50}]}"#);

    assert_eq!(snapshot.context_used, Some(250));
    assert_eq!(snapshot.context_percent(), Some(25.0));
}

#[test]
fn reads_how_a_request_ended() {
    for (finish, expected) in [
        ("stop", RequestOutcome::Done),
        ("length", RequestOutcome::Done),
        ("tool_calls", RequestOutcome::Done),
        ("cancel", RequestOutcome::Stopped),
        ("disconnect", RequestOutcome::Stopped),
        ("timeout", RequestOutcome::TimedOut),
        ("error", RequestOutcome::Failed),
    ] {
        let snapshot = from_metrics(&format!(r#"{{"live":{{}},"requests":[{{"finish":"{finish}"}}]}}"#));
        assert_eq!(snapshot.recent[0].outcome, expected, "{finish}");
    }
}

#[test]
fn a_request_with_an_error_counts_as_failed_whatever_it_finished_with() {
    let snapshot = from_metrics(r#"{"live":{},"requests":[{"finish":"stop","error":"boom"}]}"#);
    assert_eq!(snapshot.recent[0].outcome, RequestOutcome::Failed);
}

#[test]
fn reads_a_draining_instance() {
    assert!(from_telemetry(r#"{"instance":{"state":"draining"}}"#).draining);
    assert!(!from_telemetry(r#"{"instance":{"state":"ready"}}"#).draining);
}

#[test]
fn reads_the_queue_limits_only_telemetry_has() {
    let snapshot = from_telemetry(r#"{"work":{"admission_rejected":3,"queue_timeouts":1}}"#);

    assert_eq!(snapshot.rejected, Some(3));
    assert_eq!(snapshot.queue_timeouts, Some(1));
}

#[test]
fn without_an_instance_id_the_start_time_identifies_the_run() {
    let snapshot = from_metrics(r#"{"live":{},"totals":{"since":1793914420.5}}"#);
    assert_eq!(snapshot.instance_id, None);
    assert_eq!(snapshot.metrics_since, Some(1793914420.5));
}

#[test]
fn the_instance_id_comes_only_from_telemetry() {
    let metrics = value(r#"{"live":{},"totals":{"since":1793914420.5}}"#);
    let telemetry = value(r#"{"instance":{"id":"abc"}}"#);

    let snapshot = parse(Some(&metrics), Some(&telemetry));
    assert_eq!(snapshot.instance_id.as_deref(), Some("abc"));
    assert_eq!(snapshot.metrics_since, Some(1793914420.5));
}

#[test]
fn prefers_the_card_names_telemetry_reports_when_metrics_lists_none() {
    let metrics = value(r#"{"live":{},"hardware":{"gpus":[{"index":0,"util":5},{"index":1,"util":7}]}}"#);
    let telemetry = value(r#"{"devices":[{"index":0,"name":"Card A"},{"index":1,"name":"Card B"}]}"#);

    let snapshot = parse(Some(&metrics), Some(&telemetry));
    assert_eq!(snapshot.gpus[0].name.as_deref(), Some("Card A"));
    assert_eq!(snapshot.gpus[1].name.as_deref(), Some("Card B"));
}

#[test]
fn a_card_missing_a_reading_does_not_become_zero_in_the_summary() {
    let snapshot = from_metrics(r#"{"live":{},"hardware":{"gpus":[{"index":0,"temp":null,"power":100},{"index":1,"temp":70,"power":null}]}}"#);

    assert_eq!(snapshot.gpu.temperature_c, Some(70.0));
    assert_eq!(snapshot.gpu.power_watts, Some(100.0));
    assert_eq!(snapshot.gpus[0].temperature_c, None);
}

#[test]
fn missing_everything_means_unknown_rather_than_zero() {
    let snapshot = from_metrics(r#"{"live":{}}"#);

    assert_eq!(snapshot.model, None);
    assert_eq!(snapshot.cpu_percent, None);
    assert_eq!(snapshot.ram_percent(), None);
    assert_eq!(snapshot.gpu.load_percent, None);
    assert!(snapshot.gpus.is_empty());
    assert!(snapshot.recent.is_empty());
}

#[test]
fn wrong_types_from_the_network_are_treated_as_missing() {
    for json in [
        r#"{"live":{"tok_s":"fast"},"hardware":{"cpu":true,"ram_used":[1]}}"#,
        r#"{"live":"not an object","hardware":7,"engine":[],"requests":{},"totals":"x"}"#,
        r#"{"live":{},"hardware":{"gpus":["x",3,null]},"requests":["x",null]}"#,
    ] {
        let snapshot = from_metrics(json);

        assert_eq!(snapshot.cpu_percent, None, "{json}");
        assert!(snapshot.gpus.is_empty(), "{json}");
        assert!(snapshot.recent.is_empty(), "{json}");
    }
}

#[test]
fn absurd_numbers_do_not_panic() {
    let json = r#"{"live":{"state":"generating","generated":1e300,"prompt_tokens":1e300,"max_tokens":-5,"queued":-1,"waiting":1e300},
        "engine":{"max_context":0},"totals":{"output_tokens":1e300,"decode_ms":1e-300},
        "hardware":{"gpus":[{"index":-1,"mem_used":1e300,"mem_total":1e300}],"gpu_pcie_gen":1e300,"ram_used":1e300,"ram_total":-1},
        "requests":[{"prompt_tokens":9000000000000000000,"output_tokens":9000000000000000000}]}"#;
    let snapshot = from_metrics(json);
    assert!(snapshot.ram_percent().is_none());
}

// Every field Strata reports fits under 1e15 with room to spare: bytes are near 1e13, and
// seconds since 1970 near 1e9. Anything larger is junk, and left in, it summed, averaged,
// and date-shifted its way into nonsense cards rather than reading as a missing value.
#[test]
fn numbers_above_the_ceiling_read_as_missing() {
    let snapshot = from_metrics(r#"{"live":{"waiting":1e16},"hardware":{"ram_used":1e15,"ram_total":4e15,"gpus":[{"power_watts":9e15}]},"requests":[{"time":3e18}]}"#);

    assert_eq!(snapshot.waiting, None);
    assert!(snapshot.ram_percent().is_none());
    assert_eq!(snapshot.gpus[0].power_watts, None);
    assert_eq!(snapshot.recent[0].started_at, None);
}

// Four tokens in a decode span small enough to underflow when halved divides by zero and
// reads as an infinite token rate, which then scaled an entire trend to invisibility.
#[test]
fn a_decode_span_too_small_to_divide_by_reads_as_no_speed() {
    let snapshot = from_metrics(r#"{"requests":[{"output_tokens":4,"decode_ms":1e-308}]}"#);
    assert_eq!(snapshot.recent[0].tokens_per_second, None);
}

#[test]
fn strips_control_and_direction_characters_from_labels() {
    let direction = char::from_u32(0x202E).unwrap();
    let escapes = concat!("\\", "u0007", "\\", "n");
    let json = format!(r#"{{"live":{{}},"engine":{{"model":"safe{direction}gnp.exe{escapes}name"}}}}"#);
    let snapshot = from_metrics(&json);
    assert_eq!(snapshot.model.as_deref(), Some("safe gnp.exe  name"));
}

#[test]
fn cuts_off_absurdly_long_labels() {
    let long = "x".repeat(5000);
    let snapshot = from_metrics(&format!(r#"{{"live":{{}},"engine":{{"model":"{long}"}}}}"#));
    assert_eq!(snapshot.model.unwrap().chars().count(), 120);
}

#[test]
fn limits_how_many_cards_and_requests_it_reads() {
    let gpus = (0..40).map(|i| format!(r#"{{"index":{i}}}"#)).collect::<Vec<_>>().join(",");
    let requests = vec!["{}"; 200].join(",");
    let snapshot = from_metrics(&format!(r#"{{"live":{{}},"hardware":{{"gpus":[{gpus}]}},"requests":[{requests}]}}"#));

    assert_eq!(snapshot.gpus.len(), 16);
    assert_eq!(snapshot.recent.len(), 50);
}

#[test]
fn missing_values_never_read_as_zero() {
    assert_eq!(format::percent(None), "n/a");
    assert_eq!(format::speed(None), "n/a");
    assert_eq!(format::tokens(None), "n/a");
    assert_eq!(format::short(None), "n/a");
    assert_eq!(format::used_of(Some(5), None), "n/a");
    assert_eq!(format::megabytes_per_second(None), "n/a");
    assert_eq!(format::seconds(None), "n/a");
}

#[test]
fn matches_how_strata_writes_its_numbers() {
    assert_eq!(format::speed(Some(45.1)), "45.1 tok/s");
    assert_eq!(format::used_of(Some((10.4 * (1u64 << 30) as f64) as i64), Some(12 << 30)), "10.4 / 12 GB");
    assert_eq!(format::used_of(Some(16 << 30), Some(33693368320)), "16.0 / 31.4 GB");
    assert_eq!(format::tokens(Some(3197)), "3,197");
    assert_eq!(format::context_size(Some(32768)), "32K");
    assert_eq!(format::context_size(Some(131072)), "128K");
    assert_eq!(format::megabytes_per_second(Some(9.0)), "9.0 MB/s");
    assert_eq!(format::megabytes_per_second(Some(3475.0)), "3,475 MB/s");
    assert_eq!(format::seconds(Some(8.7)), "8.7 s");
}

#[test]
fn shortens_counts_for_table_cells() {
    for (value, expected) in [(261, "261"), (7719, "7,719"), (18187, "18.2K"), (2362000, "2.4M"), (2000000, "2M")] {
        assert_eq!(format::short(Some(value)), expected);
    }
}

#[test]
fn writes_durations_at_a_useful_scale() {
    for (seconds, expected) in [(0.0, "0s"), (42.0, "42s"), (60.0, "1m"), (3700.0, "1h 1m"), (91_000.0, "1d 1h"), (-5.0, "0s")] {
        assert_eq!(format::duration(seconds), expected);
    }
}

#[test]
fn writes_times_of_day_in_the_local_zone() {
    assert_eq!(format::clock(3661.9), "01:01:01");
    format::set_utc_offset(-3600);
    assert_eq!(format::clock(0.0), "23:00:00");
    format::set_utc_offset(0);
}
