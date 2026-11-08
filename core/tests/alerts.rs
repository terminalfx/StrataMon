use stratamon_core::alerts::*;
use stratamon_core::client::{PollFailure, PollResult};
use stratamon_core::config::{AlertThresholds, ServerConfig};
use stratamon_core::models::*;
use stratamon_core::state::{FAILURES_BEFORE_OFFLINE, ServerState, ServerStatus};

const START: f64 = 1_800_000_000.0;

fn server() -> ServerConfig {
    ServerConfig { id: "s1".into(), name: "strata-1".into(), addresses: vec!["a".into()], ..Default::default() }
}

fn snapshot(temperature: Option<f64>, ram: Option<i64>) -> StrataSnapshot {
    StrataSnapshot {
        instance_id: Some("one".into()),
        waiting: Some(0),
        ram_used_bytes: ram,
        ram_total_bytes: Some(100),
        gpus: vec![GpuReading { temperature_c: temperature, ..Default::default() }],
        ..Default::default()
    }
}

fn normal() -> StrataSnapshot {
    snapshot(Some(40.0), Some(10))
}

fn hot(temperature: f64) -> StrataSnapshot {
    snapshot(Some(temperature), Some(10))
}

fn success(snapshot: StrataSnapshot) -> PollResult {
    PollResult { snapshot: Some(snapshot), failure: PollFailure::None, address: Some("a".into()), detail: None }
}

struct Run {
    engine: AlertEngine,
    state: ServerState,
}

impl Run {
    fn new() -> Self {
        let limits = AlertThresholds { ram_percent: 90.0, gpu_temperature_c: 85.0, sustain_seconds: 60, queue_wait_seconds: 30 };
        Self { engine: AlertEngine::new(limits), state: ServerState::default() }
    }

    fn with_limits(ram_percent: f64, gpu_temperature_c: f64) -> Self {
        let limits = AlertThresholds { ram_percent, gpu_temperature_c, sustain_seconds: 60, queue_wait_seconds: 30 };
        Self { engine: AlertEngine::new(limits), state: ServerState::default() }
    }

    fn report(&mut self, snapshot: StrataSnapshot, at_second: u32) -> Vec<AlertEvent> {
        let now = START + f64::from(at_second);
        self.state.apply(success(snapshot), now);
        self.engine.evaluate(&server(), &self.state, now)
    }

    fn fail(&mut self, failure: PollFailure, at_second: u32, detail: &str) -> Vec<AlertEvent> {
        let now = START + f64::from(at_second);
        let result = PollResult { snapshot: None, failure, address: Some("a".into()), detail: Some(detail.into()) };
        self.state.apply(result, now);
        self.engine.evaluate(&server(), &self.state, now)
    }

    fn go_offline(&mut self, at_second: u32, failure: PollFailure) -> Vec<AlertEvent> {
        self.fail(failure, at_second, "");
        self.fail(failure, at_second + 1, "");
        self.fail(failure, at_second + 2, "")
    }

    fn active(&self) -> usize {
        self.engine.active().len()
    }
}

fn single(events: Vec<AlertEvent>) -> AlertEvent {
    assert_eq!(events.len(), 1, "{events:?}");
    events.into_iter().next().unwrap()
}

#[test]
fn a_brief_spike_does_not_alert() {
    let mut run = Run::new();
    assert!(run.report(hot(99.0), 0).is_empty());
    assert!(run.report(hot(99.0), 30).is_empty());
    assert!(run.report(hot(40.0), 45).is_empty());
    assert_eq!(run.active(), 0);
}

#[test]
fn a_sustained_breach_alerts_exactly_once() {
    let mut run = Run::new();
    run.report(hot(90.0), 0);
    let alert = single(run.report(hot(90.0), 60));

    assert_eq!(alert.change, AlertChange::Raised);
    assert_eq!(alert.severity, AlertSeverity::Warning);
    assert_eq!(alert.message, "strata-1: GPU temperature at 90 °C (limit 85 °C)");

    assert!(run.report(hot(91.0), 90).is_empty());
    assert_eq!(run.active(), 1);
}

#[test]
fn a_dip_below_the_limit_restarts_the_sustain_timer() {
    let mut run = Run::new();
    run.report(hot(90.0), 0);
    run.report(hot(70.0), 50);
    run.report(hot(90.0), 55);
    assert!(run.report(hot(90.0), 100).is_empty());
    single(run.report(hot(90.0), 115));
}

#[test]
fn a_reading_hovering_near_the_limit_does_not_flap() {
    let mut run = Run::new();
    run.report(hot(90.0), 0);
    run.report(hot(90.0), 60);

    assert!(run.report(hot(83.0), 63).is_empty());
    assert_eq!(run.active(), 1);

    let resolved = single(run.report(hot(80.0), 66));
    assert_eq!(resolved.change, AlertChange::Resolved);
    assert_eq!(resolved.message, "strata-1: GPU temperature back to 80 °C");
    assert_eq!(run.active(), 0);
}

// The settings form's smallest limits, and hand-written ones below them, used to sit under
// the resolve margin: clearing needed a reading less than the limit minus the margin, which
// for a 1-degree limit is less than zero, so the alert could never turn off.
#[test]
fn a_limit_below_the_resolve_margin_still_clears() {
    let mut run = Run::with_limits(90.0, 1.0);
    run.report(hot(9.0), 0);
    single(run.report(hot(9.0), 60));
    assert_eq!(run.active(), 1);

    // Half the limit is still high enough to hold the alert; under half, it clears.
    assert!(run.report(hot(0.6), 63).is_empty());
    assert_eq!(run.active(), 1);

    let resolved = single(run.report(hot(0.4), 66));
    assert_eq!(resolved.change, AlertChange::Resolved);
    assert_eq!(run.active(), 0);
}

// Zero used to be a limit everything is over, with nothing able to get under it again. It
// now means "watch nothing at all".
#[test]
fn a_limit_of_zero_watches_nothing() {
    let mut run = Run::with_limits(0.0, 0.0);
    assert!(run.report(normal(), 0).is_empty());
    assert!(run.report(hot(9.0), 60).is_empty());
    assert!(run.report(hot(9.0), 120).is_empty());
    assert_eq!(run.active(), 0);
}

#[test]
fn an_unknown_reading_neither_raises_nor_clears_an_alert() {
    let mut run = Run::new();
    run.report(hot(90.0), 0);
    run.report(snapshot(None, Some(10)), 30);
    assert!(run.report(hot(90.0), 60).is_empty());

    run.report(hot(90.0), 120);
    assert_eq!(run.active(), 1);
    assert!(run.report(snapshot(None, Some(10)), 123).is_empty());
    assert_eq!(run.active(), 1);
}

#[test]
fn ram_has_its_own_limit() {
    let mut run = Run::new();
    run.report(snapshot(Some(40.0), Some(95)), 0);
    assert!(single(run.report(snapshot(Some(40.0), Some(95)), 60)).message.contains("RAM at 95%"));
}

#[test]
fn full_vram_is_normal_for_strata_and_never_alerts() {
    let mut run = Run::new();
    let full = StrataSnapshot {
        gpus: vec![GpuReading { memory_used_bytes: Some(100), memory_total_bytes: Some(100), temperature_c: Some(40.0), ..Default::default() }],
        ..normal()
    };
    run.report(full.clone(), 0);
    assert!(run.report(full, 600).is_empty());
}

#[test]
fn each_gpu_is_tracked_separately_and_named_when_there_are_several() {
    let mut run = Run::new();
    let two = StrataSnapshot {
        gpus: vec![
            GpuReading { index: 0, temperature_c: Some(90.0), ..Default::default() },
            GpuReading { index: 1, temperature_c: Some(40.0), ..Default::default() },
        ],
        ..normal()
    };

    run.report(two.clone(), 0);
    assert!(single(run.report(two, 60)).message.contains("GPU 0 temperature at 90 °C"));
}

#[test]
fn a_gpu_that_disappears_resolves_its_alert() {
    let mut run = Run::new();
    let hot_card = StrataSnapshot { gpus: vec![GpuReading { index: 1, temperature_c: Some(95.0), ..Default::default() }], ..normal() };
    run.report(hot_card.clone(), 0);
    single(run.report(hot_card, 60));

    let resolved = single(run.report(StrataSnapshot { gpus: vec![], ..normal() }, 63));
    assert_eq!(resolved.change, AlertChange::Resolved);
    assert_eq!(resolved.message, "strata-1: GPU temperature is no longer reported");
    assert_eq!(run.active(), 0);
}

// The card list comes from whichever sources answered, so a poll that missed one of them
// can't say a card has gone. The alert stays until a poll like the last one leaves it out.
#[test]
fn a_gpu_missing_from_a_poll_that_read_less_is_unknown_rather_than_gone() {
    let mut run = Run::new();
    let hot_card = StrataSnapshot { gpus: vec![GpuReading { index: 1, temperature_c: Some(95.0), ..Default::default() }], ..normal() };
    run.report(hot_card.clone(), 0);
    single(run.report(hot_card, 60));

    let fewer_sources = StrataSnapshot { access: TelemetryAccess::TelemetryOnly, gpus: vec![], ..normal() };
    assert!(run.report(fewer_sources.clone(), 63).is_empty());
    assert_eq!(run.active(), 1);

    assert_eq!(single(run.report(fewer_sources, 66)).change, AlertChange::Resolved);
}

#[test]
fn requests_stuck_in_the_queue_alert_after_the_queue_limit() {
    let mut run = Run::new();
    let queued = StrataSnapshot { waiting: Some(2), ..normal() };

    assert!(run.report(queued.clone(), 0).is_empty());
    assert!(run.report(queued.clone(), 20).is_empty());
    let raised = single(run.report(queued, 30));
    assert_eq!(raised.severity, AlertSeverity::Warning);
    assert!(raised.message.contains("waiting over 30s"));

    assert_eq!(single(run.report(normal(), 33)).change, AlertChange::Resolved);
}

#[test]
fn a_queue_that_empties_in_time_never_alerts() {
    let mut run = Run::new();
    let queued = StrataSnapshot { waiting: Some(2), ..normal() };

    run.report(queued.clone(), 0);
    assert!(run.report(normal(), 20).is_empty());
    assert!(run.report(queued.clone(), 25).is_empty());
    assert!(run.report(queued, 50).is_empty());
}

#[test]
fn an_unavailable_model_is_critical_straight_away() {
    let mut run = Run::new();
    run.report(normal(), 0);
    let raised = single(run.report(StrataSnapshot { activity: StrataActivity::Error, ..normal() }, 3));

    assert_eq!(raised.severity, AlertSeverity::Critical);
    assert!(raised.message.contains("unavailable"));
    assert_eq!(single(run.report(normal(), 6)).change, AlertChange::Resolved);
}

#[test]
fn an_unloaded_model_is_normal_because_strata_unloads_when_idle() {
    let mut run = Run::new();
    run.report(normal(), 0);
    assert!(run.report(StrataSnapshot { activity: StrataActivity::Unloaded, ..normal() }, 3).is_empty());
    assert_eq!(run.active(), 0);
}

#[test]
fn draining_is_critical_because_requests_are_being_refused() {
    let mut run = Run::new();
    run.report(normal(), 0);
    let raised = single(run.report(StrataSnapshot { draining: true, ..normal() }, 3));

    assert_eq!(raised.severity, AlertSeverity::Critical);
    assert!(raised.message.contains("refusing new requests"));
}

#[test]
fn a_restart_is_a_one_off_notice() {
    let mut run = Run::new();
    run.report(normal(), 0);
    let restarted = StrataSnapshot { instance_id: Some("two".into()), ..normal() };
    let notice = single(run.report(restarted.clone(), 3));

    assert_eq!(notice.change, AlertChange::Notice);
    assert_eq!(notice.message, "strata-1: Strata restarted");
    assert_eq!(run.active(), 0);
    assert!(run.report(restarted, 6).is_empty());
}

#[test]
fn a_restart_while_offline_is_still_noticed() {
    let mut run = Run::new();
    run.report(normal(), 0);
    run.go_offline(3, PollFailure::Unreachable);

    let events = run.report(StrataSnapshot { instance_id: Some("two".into()), ..normal() }, 9);

    assert!(events.iter().any(|e| e.message == "strata-1 is back online"));
    assert!(events.iter().any(|e| e.message == "strata-1: Strata restarted"));
}

#[test]
fn the_first_reading_is_only_a_baseline() {
    let mut run = Run::new();
    let busy = StrataSnapshot {
        rejected: Some(9),
        queue_timeouts: Some(4),
        recent: vec![RequestRow { started_at: Some(5.0), outcome: RequestOutcome::Failed, ..Default::default() }],
        ..normal()
    };

    assert!(run.report(busy, 0).is_empty());
}

#[test]
fn newly_rejected_requests_are_counted() {
    let mut run = Run::new();
    run.report(StrataSnapshot { rejected: Some(2), ..normal() }, 0);

    let more = StrataSnapshot { rejected: Some(5), ..normal() };
    assert!(single(run.report(more.clone(), 3)).message.contains("3 requests turned away"));
    assert!(run.report(more, 6).is_empty());
}

#[test]
fn queue_timeouts_are_counted() {
    let mut run = Run::new();
    run.report(StrataSnapshot { queue_timeouts: Some(0), ..normal() }, 0);
    assert!(single(run.report(StrataSnapshot { queue_timeouts: Some(1), ..normal() }, 3)).message.contains("1 request timed out"));
}

#[test]
fn a_counter_restarting_at_zero_is_not_a_negative_change() {
    let mut run = Run::new();
    run.report(StrataSnapshot { rejected: Some(5), ..normal() }, 0);
    assert!(run.report(StrataSnapshot { rejected: Some(0), ..normal() }, 3).is_empty());
}

#[test]
fn a_counter_that_telemetry_does_not_report_stays_quiet() {
    let mut run = Run::new();
    run.report(StrataSnapshot { rejected: None, ..normal() }, 0);
    assert!(run.report(StrataSnapshot { rejected: Some(4), ..normal() }, 3).is_empty());
}

#[test]
fn each_failed_request_is_reported_once() {
    let mut run = Run::new();
    let good = RequestRow { started_at: Some(1.0), ..Default::default() };
    let failed = RequestRow { started_at: Some(2.0), outcome: RequestOutcome::Failed, ..Default::default() };
    let timed_out = RequestRow { started_at: Some(3.0), outcome: RequestOutcome::TimedOut, ..Default::default() };

    run.report(StrataSnapshot { recent: vec![good.clone()], ..normal() }, 0);
    let first = StrataSnapshot { recent: vec![failed.clone(), good.clone()], ..normal() };
    assert!(single(run.report(first.clone(), 3)).message.contains("1 request failed"));
    assert!(run.report(first, 6).is_empty());
    let second = StrataSnapshot { recent: vec![timed_out, failed, good], ..normal() };
    assert!(single(run.report(second, 9)).message.contains("1 request failed"));
}

// Strata stamps requests with a clock that only reads to the second, so two failures can
// share a start time. Counting each start time once made the second one invisible.
#[test]
fn two_failed_requests_in_the_same_second_are_both_reported() {
    let mut run = Run::new();
    let failed = RequestRow { started_at: Some(2.0), outcome: RequestOutcome::Failed, ..Default::default() };
    let also = RequestRow { started_at: Some(2.0), outcome: RequestOutcome::TimedOut, ..Default::default() };

    run.report(StrataSnapshot { recent: vec![failed.clone()], ..normal() }, 0);
    let pair = StrataSnapshot { recent: vec![failed.clone(), also], ..normal() };
    assert!(single(run.report(pair.clone(), 3)).message.contains("1 request failed"));
    assert!(run.report(pair, 6).is_empty());
}

// A row that never says when it started used to be skipped entirely: every poll it showed up
// in, the alert engine had no key for it and reported nothing at all.
#[test]
fn a_failed_request_without_a_start_time_is_still_reported() {
    let mut run = Run::new();
    let anonymous = RequestRow { started_at: None, outcome: RequestOutcome::Failed, ..Default::default() };

    run.report(StrataSnapshot { recent: vec![], ..normal() }, 0);
    let first = StrataSnapshot { recent: vec![anonymous.clone()], ..normal() };
    assert!(single(run.report(first.clone(), 3)).message.contains("1 request failed"));
    assert!(run.report(first, 6).is_empty());
}

#[test]
fn a_stopped_request_is_not_a_failure() {
    let mut run = Run::new();
    run.report(normal(), 0);
    let stopped = RequestRow { started_at: Some(1.0), outcome: RequestOutcome::Stopped, ..Default::default() };
    assert!(run.report(StrataSnapshot { recent: vec![stopped], ..normal() }, 3).is_empty());
}

#[test]
fn going_offline_raises_one_critical_alert_and_coming_back_resolves_it() {
    let mut run = Run::new();
    run.report(normal(), 0);
    let raised = single(run.go_offline(3, PollFailure::Unreachable));
    assert_eq!(raised.severity, AlertSeverity::Critical);
    assert_eq!(raised.message, "strata-1 is offline");

    assert!(run.fail(PollFailure::Unreachable, 9, "").is_empty());

    let resolved = single(run.report(normal(), 12));
    assert_eq!(resolved.change, AlertChange::Resolved);
    assert_eq!(resolved.message, "strata-1 is back online");
}

#[test]
fn a_stopped_strata_on_a_running_pc_says_so() {
    let mut run = Run::new();
    run.report(normal(), 0);
    let raised = single(run.go_offline(3, PollFailure::NotListening));
    assert_eq!(raised.message, "strata-1: Strata isn't answering, but the PC is on");
}

#[test]
fn one_failed_poll_does_not_raise_an_offline_alert() {
    let mut run = Run::new();
    run.report(normal(), 0);
    assert!(run.fail(PollFailure::Unreachable, 3, "").is_empty());
    assert!(run.report(normal(), 6).is_empty());
}

// While a server is still online after a failed poll, the last snapshot is all there is. It
// mustn't let a sustain timer run out on a reading the failed poll never confirmed.
#[test]
fn a_failed_poll_does_not_finish_a_sustain_timer_on_old_readings() {
    let mut run = Run::new();
    let queued = StrataSnapshot { waiting: Some(2), ..normal() };
    run.report(queued.clone(), 0);

    assert!(run.fail(PollFailure::Unreachable, 30, "").is_empty());
    assert_eq!(run.state.status, ServerStatus::Online);

    // The timer started again with the next good poll, so it needs the full wait from there.
    assert!(run.report(queued.clone(), 33).is_empty());
    assert!(single(run.report(queued, 63)).message.contains("waiting"));
}

#[test]
fn a_poll_without_metrics_does_not_clear_an_engine_error() {
    let mut run = Run::new();
    let broken = StrataSnapshot { activity: StrataActivity::Error, ..normal() };
    single(run.report(broken.clone(), 0));

    // A read-only key or a /metrics hiccup: the activity comes from telemetry, which never says error.
    let without_metrics = StrataSnapshot { access: TelemetryAccess::TelemetryOnly, activity: StrataActivity::Idle, ..normal() };
    assert!(run.report(without_metrics, 3).is_empty());
    assert_eq!(run.active(), 1);

    assert!(run.report(broken, 6).is_empty());
    assert_eq!(single(run.report(normal(), 9)).message, "strata-1: the model is available again");
}

#[test]
fn a_poll_without_telemetry_does_not_clear_draining() {
    let mut run = Run::new();
    let draining = StrataSnapshot { draining: true, ..normal() };
    single(run.report(draining, 0));

    let without_telemetry = StrataSnapshot { access: TelemetryAccess::MetricsOnly, draining: false, ..normal() };
    assert!(run.report(without_telemetry, 3).is_empty());
    assert_eq!(run.active(), 1);
}

// A poll that missed /v1/telemetry has no instance id. Strata didn't restart, and the poll
// after, with telemetry back, mustn't say so either.
#[test]
fn a_poll_without_telemetry_is_not_a_restart() {
    let mut run = Run::new();
    let full = StrataSnapshot { metrics_since: Some(1000.5), ..normal() };
    let without_telemetry = StrataSnapshot { access: TelemetryAccess::MetricsOnly, instance_id: None, ..full.clone() };

    run.report(full.clone(), 0);
    assert!(run.report(without_telemetry, 3).is_empty());
    assert!(run.report(full, 6).is_empty());
}

#[test]
fn a_restart_while_telemetry_is_missing_is_noticed_once() {
    let mut run = Run::new();
    run.report(StrataSnapshot { metrics_since: Some(1000.5), ..normal() }, 0);

    let restarted = StrataSnapshot { access: TelemetryAccess::MetricsOnly, instance_id: None, metrics_since: Some(2000.5), ..normal() };
    assert_eq!(single(run.report(restarted, 3)).message, "strata-1: Strata restarted");

    // Telemetry is back with the new run's id, which is no second restart.
    let back = StrataSnapshot { instance_id: Some("two".into()), metrics_since: Some(2000.5), ..normal() };
    assert!(run.report(back, 6).is_empty());
}

#[test]
fn an_older_strata_restart_is_told_by_its_start_time() {
    let mut run = Run::new();
    let old = |since| StrataSnapshot { access: TelemetryAccess::MetricsOnly, instance_id: None, metrics_since: Some(since), ..normal() };
    run.report(old(1000.5), 0);
    assert!(run.report(old(1000.5), 3).is_empty());
    assert_eq!(single(run.report(old(2000.5), 6)).message, "strata-1: Strata restarted");
}

// Rejections that happen while telemetry is missing for a poll are counted when it's back.
#[test]
fn a_counter_is_compared_across_a_poll_that_missed_it() {
    let mut run = Run::new();
    run.report(StrataSnapshot { rejected: Some(2), ..normal() }, 0);
    assert!(run.report(StrataSnapshot { access: TelemetryAccess::MetricsOnly, rejected: None, ..normal() }, 3).is_empty());
    assert!(single(run.report(StrataSnapshot { rejected: Some(5), ..normal() }, 6)).message.contains("3 requests turned away"));
}

#[test]
fn a_server_that_never_answers_still_alerts() {
    let mut run = Run::new();
    assert!(run.fail(PollFailure::Unreachable, 0, "").is_empty());
    assert!(run.fail(PollFailure::Unreachable, 3, "").is_empty());
    single(run.fail(PollFailure::Unreachable, 6, ""));
}

#[test]
fn a_rejected_key_alerts_immediately_with_the_reason() {
    let mut run = Run::new();
    let alert = single(run.fail(PollFailure::KeyRejected, 0, "Strata rejected the key."));
    assert_eq!(alert.severity, AlertSeverity::Critical);
    assert!(alert.message.contains("rejected the key"));
}

#[test]
fn going_offline_does_not_pretend_a_raised_alert_cleared() {
    let mut run = Run::new();
    run.report(hot(90.0), 0);
    run.report(hot(90.0), 60);

    let events = run.go_offline(63, PollFailure::Unreachable);
    assert!(events.iter().all(|e| e.change != AlertChange::Resolved));
    assert_eq!(run.active(), 2);
}

#[test]
fn going_offline_restarts_a_pending_sustain_timer() {
    let mut run = Run::new();
    run.report(hot(90.0), 0);
    run.go_offline(10, PollFailure::Unreachable);
    run.report(hot(90.0), 20);
    assert!(run.report(hot(90.0), 70).is_empty());
    single(run.report(hot(90.0), 80));
}

#[test]
fn servers_do_not_affect_each_other() {
    let mut run = Run::new();
    let other = ServerConfig { id: "s2".into(), name: "strata-2".into(), ..server() };
    let mut other_state = ServerState::default();

    run.report(hot(90.0), 0);
    let later = START + 60.0;
    other_state.apply(success(hot(90.0)), later);

    assert!(run.engine.evaluate(&other, &other_state, later).is_empty());
    single(run.report(hot(90.0), 60));
}

#[test]
fn nothing_happens_before_the_first_poll() {
    let mut run = Run::new();
    assert!(run.engine.evaluate(&server(), &ServerState::default(), START).is_empty());
}

#[test]
fn the_most_serious_alerts_come_first() {
    let mut run = Run::new();
    run.report(hot(90.0), 0);
    run.report(hot(90.0), 60);
    run.go_offline(63, PollFailure::Unreachable);

    let alerts = run.engine.active();
    assert_eq!(alerts[0].severity, AlertSeverity::Critical);
    assert_eq!(alerts[1].severity, AlertSeverity::Warning);
}

// Connection state

fn timeout() -> PollResult {
    PollResult { snapshot: None, failure: PollFailure::Unreachable, address: Some("lan".into()), detail: Some("timed out".into()) }
}

fn model_success() -> PollResult {
    PollResult { address: Some("lan".into()), ..success(StrataSnapshot { model: Some("m".into()), ..Default::default() }) }
}

#[test]
fn stays_online_until_enough_polls_fail_in_a_row() {
    let mut state = ServerState::default();
    state.apply(model_success(), START);

    state.apply(timeout(), START);
    state.apply(timeout(), START);
    assert_eq!(state.status, ServerStatus::Online);

    state.apply(timeout(), START);
    assert_eq!(state.status, ServerStatus::Offline);
}

#[test]
fn a_success_resets_the_failure_count() {
    let mut state = ServerState::default();
    state.apply(timeout(), START);
    state.apply(timeout(), START);
    state.apply(model_success(), START);
    state.apply(timeout(), START);
    state.apply(timeout(), START);
    assert_eq!(state.status, ServerStatus::Online);
}

#[test]
fn keeps_the_last_snapshot_and_time_when_offline() {
    let mut state = ServerState::default();
    state.apply(model_success(), START);
    for i in 0..FAILURES_BEFORE_OFFLINE {
        state.apply(timeout(), START + f64::from(i + 1));
    }

    assert_eq!(state.latest.unwrap().model.as_deref(), Some("m"));
    assert_eq!(state.last_seen, Some(START));
    assert_eq!(state.last_error.as_deref(), Some("lan: timed out"));
    assert_eq!(state.last_failure, PollFailure::Unreachable);
}

#[test]
fn a_definite_answer_is_reported_straight_away() {
    for failure in [PollFailure::KeyMissing, PollFailure::KeyRejected, PollFailure::HostRefused, PollFailure::Incompatible, PollFailure::NoTelemetry] {
        let mut state = ServerState::default();
        state.apply(PollResult { failure, ..timeout() }, START);
        assert_eq!(state.status, ServerStatus::Rejected, "{failure:?}");
    }
}

#[test]
fn a_possible_blip_waits_for_more_failures() {
    for failure in [PollFailure::Unreachable, PollFailure::NotListening, PollFailure::NotStrata, PollFailure::ServerError] {
        let mut state = ServerState::default();
        state.apply(PollResult { failure, ..timeout() }, START);
        assert_eq!(state.status, ServerStatus::Connecting, "{failure:?}");
    }
}

#[test]
fn a_success_clears_the_last_failure() {
    let mut state = ServerState::default();
    state.apply(timeout(), START);
    state.apply(model_success(), START);
    assert_eq!(state.last_failure, PollFailure::None);
}

#[test]
fn remembers_which_address_answered() {
    let mut state = ServerState::default();
    state.apply(PollResult { address: Some("tailscale".into()), ..model_success() }, START);
    assert_eq!(state.active_address.as_deref(), Some("tailscale"));
}
