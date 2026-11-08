use stratamon_core::alerts::{AlertChange, AlertEvent, AlertSeverity};
use stratamon_core::card::*;
use stratamon_core::client::{PollFailure, PollResult};
use stratamon_core::config::{AlertThresholds, AppConfig, ServerConfig};
use stratamon_core::format;
use stratamon_core::models::*;
use stratamon_core::settings::{ServerForm, SettingsForm, describe_connection};
use stratamon_core::state::{FAILURES_BEFORE_OFFLINE, ServerState};

const GB: i64 = 1 << 30;
const NOW: f64 = 1_800_000_000.0;
const ADDRESS: &str = "192.168.1.10";

#[test]
fn levels_are_measured_against_the_limit() {
    for (value, expected) in [
        (None, MetricLevel::Normal),
        (Some(79.9), MetricLevel::Normal),
        (Some(80.0), MetricLevel::NearLimit),
        (Some(89.9), MetricLevel::NearLimit),
        (Some(90.0), MetricLevel::OverLimit),
        (Some(100.0), MetricLevel::OverLimit),
    ] {
        assert_eq!(level_for(value, 90.0), expected, "{value:?}");
    }
}

#[test]
fn history_keeps_only_the_newest_samples() {
    let mut history = History::new(3);
    for value in [1.0, 2.0, 3.0, 4.0] {
        history.add(Some(value));
    }
    assert_eq!(history.samples().iter().copied().collect::<Vec<_>>(), [Some(2.0), Some(3.0), Some(4.0)]);
}

#[test]
fn history_keeps_gaps_so_a_line_never_bridges_an_outage() {
    let mut history = History::new(3);
    history.add(Some(1.0));
    history.add(None);
    history.add(Some(2.0));
    assert_eq!(history.samples().iter().copied().collect::<Vec<_>>(), [Some(1.0), None, Some(2.0)]);
}

fn tile(fixed_scale: Option<f64>) -> Tile {
    Tile::new("Speed", 10, 3, format::speed, fixed_scale)
}

fn update(tile: &mut Tile, sample: f64) {
    tile.update(String::new(), None, Some(sample), MetricLevel::Normal, None);
}

#[test]
fn a_percentage_keeps_its_fixed_scale() {
    let mut tile = tile(Some(100.0));
    update(&mut tile, 5.0);
    assert_eq!(tile.scale, 100.0);
}

#[test]
fn open_ended_readings_scale_to_their_recent_peak_from_zero() {
    let mut tile = tile(None);
    for sample in [40.0, 60.0, 50.0] {
        update(&mut tile, sample);
    }
    assert!((tile.scale - 66.0).abs() < 0.001);
}

#[test]
fn a_flat_zero_trend_stays_at_the_bottom_of_the_tile() {
    let mut tile = tile(None);
    update(&mut tile, 0.0);
    assert_eq!(tile.scale, 1.0);
}

#[test]
fn a_known_limit_sets_the_scale() {
    let mut tile = tile(None);
    tile.update("116 W".into(), Some("of 250 W limit".into()), Some(116.0), MetricLevel::Normal, Some(250.0));
    assert_eq!(tile.scale, 250.0);
}

// The limit arrives straight from the network, and a zero or non-finite one used to reach
// the scale: the trend then divided every point by zero and drew nothing at all.
#[test]
fn a_nonsense_limit_from_the_network_falls_back_to_the_history_peak() {
    let mut tile = tile(None);
    for junk in [Some(0.0), Some(f64::NAN), Some(f64::INFINITY)] {
        tile.update("0 W".into(), None, Some(100.0), MetricLevel::Normal, junk);
        assert!((tile.scale - 110.0).abs() < 0.001, "scale {junk:?} was accepted");
    }
}

// Near the limit is half the margin away from it, but only ever by up to half the limit:
// a 1-watt GPU limit calls everything near, and 0.6 on a 1-watt limit is 60% of the way.
#[test]
fn a_small_limit_sets_a_proportionate_near_limit_band() {
    assert_eq!(level_for(Some(0.6), 1.0), MetricLevel::NearLimit);
    assert_eq!(level_for(Some(0.4), 1.0), MetricLevel::Normal);
    assert_eq!(level_for(Some(245.0), 250.0), MetricLevel::NearLimit);
    assert_eq!(level_for(Some(235.0), 250.0), MetricLevel::Normal);
}

#[test]
fn a_gap_keeps_the_last_reading_on_screen() {
    let mut tile = tile(None);
    tile.update("45.1 tok/s".into(), Some("now".into()), Some(45.1), MetricLevel::Normal, None);
    tile.record_gap();

    assert_eq!(tile.value, "45.1 tok/s");
    assert_eq!(tile.samples().back(), Some(&None));
}

fn snapshot() -> StrataSnapshot {
    StrataSnapshot {
        model: Some("qwen3.8-flash-next-gyro-s".into()),
        context_tokens: Some(32768),
        activity: StrataActivity::Generating,
        uptime_seconds: Some(3700.0),
        progress_done: Some(3197),
        progress_total: Some(9830),
        context_used: Some(9830),
        speed: Some(45.1),
        speed_basis: SpeedBasis::Now,
        live_speed: Some(45.1),
        slots: Some(1),
        running: Some(1),
        waiting: Some(0),
        gpu: GpuReading {
            name: Some("NVIDIA GeForce RTX 5070".into()),
            load_percent: Some(96.0),
            memory_used_bytes: Some(10 * GB),
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
        cpu_percent: Some(92.0),
        cpu_cores: Some(6),
        cpu_threads: Some(12),
        ram_used_bytes: Some(47 * GB),
        ram_total_bytes: Some(63 * GB),
        disk_read_megabytes_per_second: Some(9.0),
        disk_write_megabytes_per_second: Some(0.2),
        experts_cached: Some(3099),
        expert_cache_mebibytes: Some(3072.0),
        total_requests: Some(1204),
        total_output_tokens: Some(2_100_000),
        ..Default::default()
    }
}

fn success(snapshot: StrataSnapshot) -> PollResult {
    PollResult { snapshot: Some(snapshot), failure: PollFailure::None, address: Some(ADDRESS.into()), detail: None }
}

fn online(snapshot: StrataSnapshot) -> (ServerCard, ServerState) {
    let mut card = ServerCard::new("strata-1", AlertThresholds::default(), 3);
    let mut state = ServerState::default();
    state.apply(success(snapshot), NOW);
    card.update(&state, NOW);
    (card, state)
}

fn apply(card: &mut ServerCard, state: &mut ServerState, snapshot: StrataSnapshot) {
    state.apply(success(snapshot), NOW);
    card.update(state, NOW);
}

fn go_offline(card: &mut ServerCard, state: &mut ServerState, failure: PollFailure, detail: &str) {
    for i in 1..=FAILURES_BEFORE_OFFLINE {
        let now = NOW + f64::from(i * 60);
        state.apply(PollResult { snapshot: None, failure, address: Some(ADDRESS.into()), detail: Some(detail.into()) }, now);
        card.update(state, now);
    }
}

#[test]
fn shows_what_the_model_is_and_how_it_connected() {
    let (card, _) = online(snapshot());

    assert!(card.is_online());
    assert_eq!(card.status_text(), "Online");
    assert_eq!(card.subtitle.as_deref(), Some("qwen3.8-flash-next-gyro-s · 32K context"));
    assert_eq!(card.route.as_deref(), Some("via 192.168.1.10 · up 1h 1m"));
    assert_eq!(card.problem, None);
}

#[test]
fn the_kv_cache_type_lands_in_the_subtitle_when_strata_reports_it() {
    let mut reported = snapshot();
    reported.kv_cache = Some("int8".into());
    let (card, _) = online(reported);
    assert_eq!(card.subtitle.as_deref(), Some("qwen3.8-flash-next-gyro-s · 32K context · kv int8"));
}

#[test]
fn shows_strata_state_and_progress_like_its_own_monitor() {
    let (card, _) = online(snapshot());

    assert_eq!(card.activity, "Generating a reply");
    assert_eq!(card.progress_text.as_deref(), Some("3,197 / 9,830 tokens · 33%"));
    assert!((card.progress_percent.unwrap() - 32.5).abs() < 0.1);
}

#[test]
fn has_strata_tiles_in_strata_order() {
    let (card, _) = online(snapshot());
    let labels: Vec<&str> = card.tiles().iter().map(|t| t.label).collect();
    assert_eq!(labels, ["Speed", "GPU load", "VRAM", "GPU temp", "Power", "PCIe", "CPU", "Disk read"]);
}

#[test]
fn fills_every_tile() {
    let (card, _) = online(snapshot());

    assert_eq!(card.speed.value, "45.1 tok/s");
    assert_eq!(card.speed.note.as_deref(), Some("now"));
    assert_eq!(card.gpu_load.value, "96%");
    assert_eq!(card.gpu_load.note.as_deref(), Some("RTX 5070"));
    assert_eq!(card.vram.value, "10.0 / 12 GB");
    assert_eq!(card.gpu_temperature.value, "59 °C");
    assert_eq!(card.power.value, "116 W");
    assert_eq!(card.power.note.as_deref(), Some("of 250 W limit"));
    assert_eq!(card.pcie.value, "Gen4 x16");
    assert_eq!(card.pcie.note.as_deref(), Some("to GPU 3,475 MB/s"));
    assert_eq!(card.cpu.value, "92%");
    assert_eq!(card.cpu.note.as_deref(), Some("6 cores · 12 threads"));
    assert_eq!(card.disk.value, "9.0 MB/s");
    assert_eq!(card.disk.note.as_deref(), Some("write 0.2 MB/s"));
}

#[test]
fn fills_the_bars_and_totals() {
    let (card, _) = online(snapshot());

    assert_eq!(card.context.value, "10K / 32K");
    assert_eq!(card.ram.value, "47.0 / 63 GB");
    assert_eq!(card.experts.value, "3,099 · 3.0 GB");
    assert_eq!(card.requests.value, "1 running");
    assert_eq!(card.served.value, "1,204 requests · 2.1M tokens");
}

#[test]
fn names_every_state_in_strata_words() {
    for (activity, expected) in [
        (StrataActivity::Idle, "Waiting for a request"),
        (StrataActivity::Reading, "Reading prompt"),
        (StrataActivity::Generating, "Generating a reply"),
        (StrataActivity::Queued, "Requests are queued"),
        (StrataActivity::Unloaded, "Model not loaded"),
        (StrataActivity::Error, "Model unavailable"),
    ] {
        let (card, _) = online(StrataSnapshot { activity, ..snapshot() });
        assert_eq!(card.activity, expected);
    }
}

#[test]
fn draining_overrides_the_state_because_it_matters_more() {
    let (card, _) = online(StrataSnapshot { draining: true, ..snapshot() });
    assert_eq!(card.activity, "Shutting down");
}

#[test]
fn says_where_the_speed_came_from() {
    for (speed_basis, expected) in [
        (SpeedBasis::Now, "now"),
        (SpeedBasis::ReadingPrompt, "reading the prompt"),
        (SpeedBasis::RunAverage, "average of the run"),
        (SpeedBasis::LastRequest, "last request"),
    ] {
        let (card, _) = online(StrataSnapshot { speed_basis, ..snapshot() });
        assert_eq!(card.speed.note.as_deref(), Some(expected));
    }
}

#[test]
fn describes_parallel_work_and_problems_with_the_queue() {
    let (card, _) = online(StrataSnapshot {
        slots: Some(4),
        running: Some(3),
        waiting: Some(2),
        rejected: Some(5),
        queue_timeouts: Some(1),
        ..snapshot()
    });
    assert_eq!(card.requests.value, "3 of 4 slots · 2 waiting · 5 turned away · 1 timed out");
}

#[test]
fn flags_a_hot_gpu_and_full_ram_against_their_limits() {
    let (card, _) = online(StrataSnapshot {
        gpu: GpuReading { temperature_c: Some(90.0), ..Default::default() },
        ram_used_bytes: Some(96),
        ram_total_bytes: Some(100),
        ..snapshot()
    });

    assert_eq!(card.gpu_temperature.level, MetricLevel::OverLimit);
    assert_eq!(card.ram.level, MetricLevel::OverLimit);
    assert_eq!(card.gpu_load.level, MetricLevel::Normal);
}

#[test]
fn one_gpu_is_described_by_the_tiles_alone() {
    let (card, _) = online(snapshot());
    assert!(card.gpu_rows.is_empty());
}

#[test]
fn several_gpus_each_get_a_row_and_the_tiles_say_what_they_sum_up() {
    let card_at = |index, load, temperature, power| GpuReading {
        index,
        load_percent: Some(load),
        memory_used_bytes: Some(15 * GB),
        memory_total_bytes: Some(16 * GB),
        temperature_c: Some(temperature),
        power_watts: Some(power),
        ..Default::default()
    };
    let (card, _) = online(StrataSnapshot { gpus: vec![card_at(0, 4.0, 52.0, 27.0), card_at(1, 100.0, 90.0, 100.0)], ..snapshot() });

    assert_eq!(card.gpu_rows.len(), 2);
    assert_eq!(card.gpu_rows[1].name, "GPU 1");
    assert_eq!(card.gpu_rows[1].reading, "100% · 15.0 / 16 GB · 90 °C · 100 W");
    assert_eq!(card.gpu_rows[1].level, MetricLevel::OverLimit);
    assert_eq!(card.gpu_load.note.as_deref(), Some("busiest card"));
    assert_eq!(card.gpu_temperature.note.as_deref(), Some("hottest card"));
}

#[test]
fn gpus_can_come_and_go() {
    let gpus: Vec<GpuReading> = (0..3).map(|index| GpuReading { index, ..Default::default() }).collect();
    let (mut card, mut state) = online(StrataSnapshot { gpus: gpus.clone(), ..snapshot() });

    apply(&mut card, &mut state, StrataSnapshot { gpus: gpus[..2].to_vec(), ..snapshot() });
    assert_eq!(card.gpu_rows.len(), 2);
}

#[test]
fn shows_recent_requests_newest_first_and_marks_the_ones_that_failed() {
    let (card, _) = online(StrataSnapshot {
        recent: vec![
            RequestRow {
                started_at: Some(NOW),
                outcome: RequestOutcome::Failed,
                prompt_tokens: Some(18187),
                output_tokens: Some(261),
                tokens_per_second: Some(36.3),
                duration_seconds: Some(8.7),
                ..Default::default()
            },
            RequestRow { started_at: Some(NOW - 10.0), ..Default::default() },
        ],
        ..snapshot()
    });

    let first = &card.recent_requests[0];
    assert_eq!(card.recent_requests.len(), 2);
    assert_eq!(first.status, "Failed");
    assert!(first.is_problem);
    assert_eq!(first.prompt, "18.2K");
    assert_eq!(first.output, "261");
    assert_eq!(first.speed, "36.3");
    assert_eq!(first.duration, "8.7 s");
    assert!(!card.recent_requests[1].is_problem);
}

#[test]
fn shows_only_the_latest_few_requests() {
    let recent = (0..12).map(|i| RequestRow { started_at: Some(f64::from(i)), ..Default::default() }).collect();
    let (card, _) = online(StrataSnapshot { recent, ..snapshot() });
    assert_eq!(card.recent_requests.len(), 5);
}

#[test]
fn explains_what_a_read_only_key_hides() {
    let (card, _) = online(StrataSnapshot { access: TelemetryAccess::TelemetryOnly, ..snapshot() });
    assert!(card.access_note.unwrap().contains("read-only"));
}

#[test]
fn everything_visible_needs_no_explanation() {
    let (card, _) = online(snapshot());
    assert_eq!(card.access_note, None);
}

#[test]
fn missing_readings_show_as_unavailable_rather_than_zero() {
    let (card, _) = online(StrataSnapshot::default());

    assert!(card.tiles().iter().all(|tile| tile.value == format::UNAVAILABLE));
    assert_eq!(card.ram.value, format::UNAVAILABLE);
    assert_eq!(card.ram.percent, None);
    assert_eq!(card.experts.value, format::UNAVAILABLE);
}

#[test]
fn an_offline_server_keeps_its_last_readings_but_fades_them() {
    let (mut card, mut state) = online(snapshot());
    go_offline(&mut card, &mut state, PollFailure::Unreachable, "Timed out.");

    assert!(card.is_offline());
    assert_eq!(card.status_text(), "Offline");
    assert_eq!(card.cpu.value, "92%");
    assert!(card.data_opacity < 1.0);
    assert_eq!(card.route.as_deref(), Some("last seen 3m ago"));
    assert_eq!(card.problem.as_deref(), Some("192.168.1.10: Timed out."));
}

#[test]
fn an_offline_server_does_not_look_like_it_is_still_working() {
    let (mut card, mut state) = online(snapshot());
    go_offline(&mut card, &mut state, PollFailure::Unreachable, "Timed out.");

    assert_eq!(card.activity, "Last reading");
    assert_eq!(card.progress_percent, None);
    assert_eq!(card.progress_text, None);
    assert_eq!(card.speed.value, format::UNAVAILABLE);
    assert_eq!(card.speed.note, None);
}

#[test]
fn a_server_that_comes_back_shows_live_readings_again() {
    let (mut card, mut state) = online(snapshot());
    go_offline(&mut card, &mut state, PollFailure::Unreachable, "Timed out.");

    apply(&mut card, &mut state, snapshot());

    assert_eq!(card.activity, "Generating a reply");
    assert_eq!(card.speed.value, "45.1 tok/s");
    assert_eq!(card.data_opacity, 1.0);
}

#[test]
fn a_stopped_strata_is_told_apart_from_an_offline_pc() {
    let (mut card, mut state) = online(snapshot());
    go_offline(&mut card, &mut state, PollFailure::NotListening, "Nothing is listening.");

    assert!(card.is_offline());
    assert_eq!(card.status_text(), "Not running");
}

#[test]
fn a_refusal_says_what_kind() {
    for (failure, expected) in [
        (PollFailure::KeyMissing, "Needs a key"),
        (PollFailure::KeyRejected, "Key rejected"),
        (PollFailure::HostRefused, "Name refused"),
        (PollFailure::Incompatible, "Needs update"),
        (PollFailure::NoTelemetry, "Needs update"),
    ] {
        let mut card = ServerCard::new("strata-1", AlertThresholds::default(), 3);
        let mut state = ServerState::default();
        state.apply(PollResult { snapshot: None, failure, address: Some(ADDRESS.into()), detail: Some("no".into()) }, NOW);
        card.update(&state, NOW);

        assert!(card.is_rejected());
        assert_eq!(card.status_text(), expected);
    }
}

#[test]
fn an_outage_leaves_a_gap_in_every_trend() {
    let (mut card, mut state) = online(snapshot());
    go_offline(&mut card, &mut state, PollFailure::Unreachable, "Timed out.");

    assert!(card.tiles().iter().all(|tile| tile.samples().back() == Some(&None)));
}

#[test]
fn a_failed_poll_leaves_a_gap_even_before_the_server_counts_as_offline() {
    let (mut card, mut state) = online(snapshot());

    state.apply(PollResult { snapshot: None, failure: PollFailure::Unreachable, address: Some(ADDRESS.into()), detail: Some("Timed out.".into()) }, NOW);
    card.update(&state, NOW);

    assert!(card.is_online());
    assert_eq!(card.speed.samples().back(), Some(&None));
}

#[test]
fn a_server_that_never_answered_has_no_readings_to_show() {
    let mut card = ServerCard::new("strata-1", AlertThresholds::default(), 3);
    let mut state = ServerState::default();
    go_offline(&mut card, &mut state, PollFailure::Unreachable, "Timed out.");

    assert!(!card.has_data);
    assert_eq!(card.route.as_deref(), Some("not reached yet"));
}

#[test]
fn history_covers_six_minutes_at_any_poll_rate() {
    assert_eq!(ServerCard::new("a", AlertThresholds::default(), 3).cpu.history_length, 120);
    assert_eq!(ServerCard::new("a", AlertThresholds::default(), 5).cpu.history_length, 72);
    assert_eq!(ServerCard::new("a", AlertThresholds::default(), 5).cpu.sample_seconds, 5);
}

fn alert(change: AlertChange, severity: AlertSeverity) -> AlertRow {
    AlertRow::new(&AlertEvent {
        server_id: "s".into(),
        server_name: "n".into(),
        key: "k".into(),
        severity,
        change,
        message: "m".into(),
        at: 0.0,
    })
}

#[test]
fn each_kind_of_alert_has_its_own_label() {
    assert_eq!(alert(AlertChange::Raised, AlertSeverity::Critical).kind, "Critical");
    assert_eq!(alert(AlertChange::Raised, AlertSeverity::Warning).kind, "Over limit");
    assert_eq!(alert(AlertChange::Resolved, AlertSeverity::Critical).kind, "Resolved");
    assert_eq!(alert(AlertChange::Notice, AlertSeverity::Warning).kind, "Notice");
}

#[test]
fn a_notice_only_sets_its_own_icon() {
    let row = alert(AlertChange::Notice, AlertSeverity::Warning);

    assert!(row.is_notice);
    assert!(!row.is_over_limit);
    assert!(!row.is_critical);
    assert!(!row.is_resolved);
}

// Settings

fn editor(addresses: &[&str]) -> ServerForm {
    ServerForm::new(&ServerConfig { addresses: addresses.iter().map(|a| a.to_string()).collect(), ..Default::default() })
}

#[test]
fn shows_the_first_two_addresses_as_lan_and_tailscale() {
    let form = editor(&["192.168.1.10", "100.93.1.2"]);
    assert_eq!(form.lan_address, "192.168.1.10");
    assert_eq!(form.tailscale_address, "100.93.1.2");
}

#[test]
fn saving_keeps_addresses_the_form_does_not_show() {
    assert_eq!(editor(&["a", "b", "c", "d"]).to_config().addresses, ["a", "b", "c", "d"]);
}

// The same address twice means polling one place twice a round, and timeboxed polls spending
// the whole budget on that one server. The first copy stays; later ones go.
#[test]
fn the_same_address_typed_or_written_twice_is_polled_once() {
    assert_eq!(editor(&["127.0.0.1:8080", "127.0.0.1:8080", "10.0.0.5:8080"]).to_config().addresses, ["127.0.0.1:8080", "10.0.0.5:8080"]);

    // A hand-added address repeating one of the boxes goes the same way.
    let form = editor(&["127.0.0.1:8080", "10.0.0.5:8080", "127.0.0.1:8080"]);
    assert_eq!(form.to_config().addresses, ["127.0.0.1:8080", "10.0.0.5:8080"]);
}

#[test]
fn blank_addresses_and_stray_spaces_are_dropped() {
    let mut form = ServerForm::new(&ServerConfig { name: " strata-1 ".into(), key: " abc ".into(), ..Default::default() });
    form.lan_address = "   ".into();
    form.tailscale_address = " 100.93.1.2 ".into();

    let config = form.to_config();
    assert_eq!(config.name, "strata-1");
    assert_eq!(config.key, "abc");
    assert_eq!(config.addresses, ["100.93.1.2"]);
}

#[test]
fn editing_a_server_keeps_its_identity() {
    let server = ServerConfig { name: "before".into(), ..Default::default() };
    let mut form = ServerForm::new(&server);
    form.name = "after".into();
    assert_eq!(form.to_config().id, server.id);
}

#[test]
fn refuses_an_incomplete_server() {
    for (name, address, expected) in [
        ("", "192.168.1.10", "needs a name"),
        ("strata-1", "", "at least one address"),
        ("strata-1", "not a host", "valid address"),
    ] {
        let mut settings = SettingsForm::new(&AppConfig::default());
        settings.add_server();
        settings.servers[0].name = name.into();
        settings.servers[0].lan_address = address.into();

        assert!(settings.try_build_config().is_none());
        assert!(settings.error.as_ref().unwrap().contains(expected), "{:?}", settings.error);
    }
}

#[test]
fn a_server_with_no_key_is_fine_because_strata_may_not_have_one() {
    let mut settings = SettingsForm::new(&AppConfig::default());
    settings.add_server();
    settings.servers[0].lan_address = "127.0.0.1".into();

    assert!(settings.try_build_config().is_some());
}

#[test]
fn a_complete_form_becomes_a_config_with_the_new_limits() {
    let mut settings = SettingsForm::new(&AppConfig { poll_seconds: 7, ..Default::default() });
    settings.gpu_temperature = "75".into();
    settings.ram_percent = "80".into();
    settings.queue_wait_seconds = "20".into();
    settings.add_server();
    settings.servers[0].lan_address = "192.168.1.10".into();
    settings.servers[0].key = "abc".into();

    let config = settings.try_build_config().unwrap();

    assert_eq!(settings.error, None);
    assert_eq!(config.alerts.gpu_temperature_c, 75.0);
    assert_eq!(config.alerts.ram_percent, 80.0);
    assert_eq!(config.alerts.queue_wait_seconds, 20);
    assert_eq!(config.poll_seconds, 7);
    assert_eq!(config.servers.len(), 1);
}

#[test]
fn a_cleared_limit_falls_back_rather_than_becoming_zero() {
    let mut settings = SettingsForm::new(&AppConfig::default());
    settings.ram_percent = String::new();
    settings.gpu_temperature = "hot".into();

    let config = settings.try_build_config().unwrap();
    assert_eq!(config.alerts.ram_percent, 95.0);
    assert_eq!(config.alerts.gpu_temperature_c, 85.0);
}

#[test]
fn limits_stay_within_sensible_ranges() {
    let mut settings = SettingsForm::new(&AppConfig::default());
    settings.ram_percent = "500".into();
    settings.gpu_temperature = "-5".into();

    let config = settings.try_build_config().unwrap();
    assert_eq!(config.alerts.ram_percent, 100.0);
    assert_eq!(config.alerts.gpu_temperature_c, 1.0);
}

#[test]
fn test_connection_says_how_much_it_can_see() {
    let snapshot = |access| StrataSnapshot { access, model: Some("qwen".into()), ..Default::default() };

    assert_eq!(describe_connection(&snapshot(TelemetryAccess::Full)), "connected to qwen, with every reading.");
    assert!(describe_connection(&snapshot(TelemetryAccess::TelemetryOnly)).contains("read-only"));
    assert!(describe_connection(&snapshot(TelemetryAccess::MetricsOnly)).contains("older"));
}
