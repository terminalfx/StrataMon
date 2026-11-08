//! Renders each screen with no window. These check that drawing works and differs where it
//! should. With STRATAMON_SHOTS set, they also save pictures for a person to look at.

mod common;

use common::*;
use stratamon_core::client::PollFailure;
use stratamon_core::config::AppConfig;
use stratamon_core::update::UpdateStatus;
use stratamon_ui::Input;

fn differs(a: &tiny_skia::Pixmap, b: &tiny_skia::Pixmap) -> bool {
    a.data() != b.data()
}

#[test]
fn draws_the_main_window_in_both_themes() {
    let mut light = filled_ui(false);
    let mut dark = filled_ui(true);
    let (light_frame, dark_frame) = (frame(&mut light, 1130, 960), frame(&mut dark, 1130, 960));

    shot(&light_frame, "main-light");
    shot(&dark_frame, "main-dark");
    assert!(differs(&light_frame, &dark_frame));
}

#[test]
fn hovering_a_trend_shows_its_readout() {
    let mut ui = filled_ui(false);
    let before = frame(&mut ui, 1130, 960);
    let (x, y) = ui.locate("Tile { card: 0, tile: 0 }").unwrap();

    ui.handle(Input::MouseMove { x, y: y + 20.0 });
    let after = frame(&mut ui, 1130, 960);

    shot(&after, "hover-light");
    assert!(differs(&before, &after));
    let _ = x;
}

#[test]
fn a_server_that_stops_answering_fades_and_says_why() {
    let mut ui = filled_ui(true);
    let generation = ui.dashboard.generation;
    let before = frame(&mut ui, 1130, 960);
    for i in 0..3 {
        ui.apply_poll(
            generation,
            2,
            bad(PollFailure::NotListening, "192.168.1.209", "The PC answered, but nothing is listening on this port. Strata may be stopped, or set to this PC only."),
            NOW + 200.0 + f64::from(i),
        );
    }

    let after = frame(&mut ui, 1130, 960);
    shot(&after, "offline-dark");
    assert!(differs(&before, &after));
    assert_eq!(ui.dashboard.servers[2].card.status_text(), "Not running");
}

#[test]
fn the_feed_lists_events_newest_first() {
    let mut ui = filled_ui(true);
    let generation = ui.dashboard.generation;
    for i in 0..3 {
        ui.apply_poll(generation, 2, bad(PollFailure::NotListening, "192.168.1.209", "The PC answered, but nothing is listening."), NOW + 100.0 + f64::from(i));
    }
    ui.apply_poll(generation, 2, good(read_only(), "192.168.1.209"), NOW + 103.0);

    let feed = frame(&mut ui, 1130, 960);
    shot(&feed, "feed-dark");
    assert!(ui.locate("Feed").is_some());
    assert!(ui.dashboard.recent_alerts[0].message.contains("back online"), "the newest event should be the top row");
}

#[test]
fn draws_the_empty_state() {
    let mut ui = new_ui(AppConfig::default(), true);
    let empty = frame(&mut ui, 1130, 960);
    shot(&empty, "empty-dark");
    assert!(ui.locate("OpenSettings").is_some());
}

#[test]
fn draws_settings_with_a_test_result_and_an_error() {
    let mut ui = filled_ui(true);
    click(&mut ui, "OpenSettings");
    assert!(ui.is_settings_open());
    ui.set_test_result("a", "192.168.1.5: connected to qwen3.8-flash-next-coder-iq1_m, with every reading.\n100.123.227.92: Timed out.".into());

    let settings = frame(&mut ui, 1130, 960);
    shot(&settings, "settings-dark");
}

#[test]
fn draws_the_update_panel() {
    let mut ui = filled_ui(false);
    click(&mut ui, "OpenSettings");
    ui.set_update_status(UpdateStatus::Available { version: "0.3.0".into(), tag: "v0.3.0".into(), link: "https://github.com/x/y".into() });
    frame(&mut ui, 1130, 960);
    for _ in 0..40 {
        ui.handle(Input::Wheel { lines: -1.0 });
    }

    let page = frame(&mut ui, 1130, 960);
    shot(&page, "update-light");
    assert!(ui.locate("InstallUpdate").is_some());
    assert!(ui.locate("OpenReleasePage").is_some());
    assert!(ui.locate("DismissUpdate").is_some());
}

#[test]
fn draws_at_other_display_scales() {
    for (scale, name) in [(1.25, "scale-125"), (1.5, "scale-150"), (2.0, "scale-200")] {
        let mut ui = filled_ui(true);
        let (width, height) = ((1130.0 * scale) as u32, (960.0 * scale) as u32);
        ui.set_viewport(width, height, scale);
        let scaled = frame(&mut ui, width, height);
        shot(&scaled, name);
    }
}

#[test]
fn a_narrow_window_stacks_cards_and_scrolls() {
    let mut ui = filled_ui(true);
    ui.set_viewport(420, 700, 1.0);
    let top = frame(&mut ui, 420, 700);
    ui.handle(Input::MouseMove { x: 200.0, y: 300.0 });
    frame(&mut ui, 420, 700);
    ui.handle(Input::Wheel { lines: -10.0 });
    let scrolled = frame(&mut ui, 420, 700);

    shot(&top, "narrow-dark");
    assert!(differs(&top, &scrolled));
}

#[test]
fn a_tiny_window_does_not_panic() {
    let mut ui = filled_ui(true);
    for (width, height) in [(1, 1), (40, 20), (120, 900), (900, 60)] {
        ui.set_viewport(width, height, 1.0);
        frame(&mut ui, width, height);
    }
}

#[test]
fn hostile_labels_do_not_panic() {
    let mut config = config();
    config.servers[0].name = "x".repeat(500);
    let mut ui = new_ui(config, true);
    let generation = ui.dashboard.generation;
    let snapshot = stratamon_core::models::StrataSnapshot { model: Some("🙂 مرحبا 漢字 ".repeat(40)), ..busy(10.0, 10.0) };
    ui.apply_poll(generation, 0, good(snapshot, &"a".repeat(300)), NOW);
    frame(&mut ui, 1130, 960);
}
