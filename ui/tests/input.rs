//! Drives the interface the way a person does, with mouse and keyboard, and no window.

mod common;

use common::*;
use stratamon_core::client::PollFailure;
use stratamon_core::config::AppConfig;
use stratamon_core::update::UpdateStatus;
use stratamon_ui::field::{Field, MASK};
use stratamon_ui::{Command, Cursor, Input, Key, Ui};

fn settings() -> Ui {
    let mut ui = filled_ui(true);
    click(&mut ui, "OpenSettings");
    frame(&mut ui, 1130, 960);
    ui
}

fn key(ui: &mut Ui, key: Key) -> Vec<Command> {
    ui.handle(Input::Key { key, shift: false })
}

fn focus(ui: &mut Ui, name: &str) {
    click(ui, name);
    frame(ui, 1130, 960);
}

fn typed(ui: &mut Ui, text: &str) {
    ui.handle(Input::Text(text.to_string()));
}

fn saved(commands: Vec<Command>) -> AppConfig {
    match commands.into_iter().next() {
        Some(Command::SaveSettings(config)) => config,
        other => panic!("expected a save, got {other:?}"),
    }
}

#[test]
fn settings_opens_and_cancel_closes_without_saving() {
    let mut ui = filled_ui(true);
    assert!(!ui.is_settings_open());

    assert!(click(&mut ui, "OpenSettings").is_empty());
    assert!(ui.is_settings_open());

    assert!(click(&mut ui, "Cancel").is_empty());
    assert!(!ui.is_settings_open());
}

#[test]
fn escape_cancels() {
    let mut ui = settings();
    assert!(key(&mut ui, Key::Escape).is_empty());
    assert!(!ui.is_settings_open());
}

#[test]
fn typing_in_a_field_is_saved() {
    let mut ui = settings();
    focus(&mut ui, "Field(Lan(1))");
    key(&mut ui, Key::Control('a'));
    typed(&mut ui, "192.168.1.77");

    let config = saved(click(&mut ui, "Save"));

    assert_eq!(config.servers[1].addresses, ["192.168.1.77"]);
    assert!(!ui.is_settings_open());
}

// A field holds a host, a port, or a key. A paste of megabytes belongs nowhere near it, and
// a very long string makes clicking and typing in that field slow. (512 is MAX_FIELD_CHARS.)
#[test]
fn a_field_never_grows_past_its_room() {
    let mut field = Field::new("", false);
    field.insert(&"x".repeat(10_000));
    assert_eq!(field.text().chars().count(), 512);

    field.insert("more");
    assert_eq!(field.text().chars().count(), 512);
}

#[test]
fn enter_saves() {
    let mut ui = settings();
    focus(&mut ui, "Field(Name(0))");
    typed(&mut ui, "X");
    let config = saved(key(&mut ui, Key::Enter));
    assert_eq!(config.servers[0].name, "NEDMX");
}

#[test]
fn a_pasted_key_is_saved_without_a_trailing_newline() {
    let mut ui = settings();
    focus(&mut ui, "Field(Key(0))");
    assert_eq!(key(&mut ui, Key::Control('v')), [Command::RequestPaste]);
    ui.handle(Input::Paste("sk-strata-pasted\r\n".to_string()));

    let config = saved(click(&mut ui, "Save"));
    assert_eq!(config.servers[0].key, "sk-strata-pasted");
}

#[test]
fn a_key_can_be_pasted_in_but_never_copied_out() {
    let mut ui = settings();
    focus(&mut ui, "Field(Key(0))");
    ui.handle(Input::Paste("secret".to_string()));
    key(&mut ui, Key::Control('a'));

    assert!(key(&mut ui, Key::Control('c')).is_empty());
    assert!(key(&mut ui, Key::Control('x')).is_empty());
}

#[test]
fn an_ordinary_field_can_be_copied() {
    let mut ui = settings();
    focus(&mut ui, "Field(Name(0))");
    key(&mut ui, Key::Control('a'));
    assert_eq!(key(&mut ui, Key::Control('c')), [Command::CopyText("NEDM".to_string())]);
}

#[test]
fn tab_moves_through_fields_in_order_and_shift_tab_goes_back() {
    let mut ui = settings();
    focus(&mut ui, "Field(Name(0))");

    key(&mut ui, Key::Tab);
    typed(&mut ui, "10.0.0.1");
    key(&mut ui, Key::Tab);
    typed(&mut ui, "10.0.0.2");
    ui.handle(Input::Key { key: Key::Tab, shift: true });
    typed(&mut ui, "10.0.0.3");

    let config = saved(click(&mut ui, "Save"));
    assert_eq!(config.servers[0].addresses, ["10.0.0.3", "10.0.0.2"]);
}

#[test]
fn a_bad_entry_stays_open_and_says_why() {
    let mut ui = settings();
    focus(&mut ui, "Field(Name(0))");
    key(&mut ui, Key::Control('a'));
    key(&mut ui, Key::Backspace);

    assert!(click(&mut ui, "Save").is_empty());
    assert!(ui.is_settings_open());
}

#[test]
fn servers_can_be_added_and_removed() {
    let mut ui = settings();
    click(&mut ui, "AddServer");
    focus(&mut ui, "Field(Lan(3))");
    typed(&mut ui, "192.168.1.50");
    click(&mut ui, "Remove(0)");

    let config = saved(click(&mut ui, "Save"));
    let names: Vec<&str> = config.servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["NEDM2", "NEDM3", "strata-4"]);
    assert_eq!(config.servers[2].addresses, ["192.168.1.50"]);
}

#[test]
fn edits_survive_adding_a_server() {
    let mut ui = settings();
    focus(&mut ui, "Field(Name(0))");
    typed(&mut ui, "-edited");
    click(&mut ui, "AddServer");
    click(&mut ui, "Remove(3)");

    assert_eq!(saved(click(&mut ui, "Save")).servers[0].name, "NEDM-edited");
}

#[test]
fn test_connection_uses_what_is_typed_and_shows_the_result() {
    let mut ui = settings();
    focus(&mut ui, "Field(Lan(0))");
    key(&mut ui, Key::Control('a'));
    typed(&mut ui, "10.1.2.3");

    let commands = click(&mut ui, "Test(0)");
    let Some(Command::TestConnection { form_id, server }) = commands.into_iter().next() else {
        panic!("expected a test");
    };
    assert_eq!(server.addresses[0], "10.1.2.3");

    ui.set_test_result(&form_id, "10.1.2.3: connected".to_string());
    frame(&mut ui, 1130, 960);
}

#[test]
fn testing_with_no_address_asks_for_one() {
    let mut ui = settings();
    click(&mut ui, "AddServer");
    assert!(click(&mut ui, "Test(3)").is_empty());
}

#[test]
fn a_test_result_for_a_removed_server_is_ignored() {
    let mut ui = settings();
    ui.set_test_result("no-such-server", "connected".to_string());
    frame(&mut ui, 1130, 960);
}

#[test]
fn the_notification_box_toggles_and_is_saved() {
    let mut ui = settings();
    click(&mut ui, "ToggleNotifications");
    assert!(saved(click(&mut ui, "Save")).desktop_notifications);
}

#[test]
fn send_test_notification_asks_the_app_to_send_one() {
    let mut ui = settings();
    assert_eq!(click(&mut ui, "SendTestNotification"), [Command::SendTestNotification]);
}

#[test]
fn releasing_the_mouse_off_a_button_does_nothing() {
    let mut ui = filled_ui(true);
    frame(&mut ui, 1130, 960);
    let (x, y) = ui.locate("OpenSettings").unwrap();

    ui.handle(Input::MouseMove { x, y });
    ui.handle(Input::MouseDown);
    ui.handle(Input::MouseMove { x: 5.0, y: 500.0 });
    ui.handle(Input::MouseUp);

    assert!(!ui.is_settings_open());
}

#[test]
fn clicking_nowhere_does_nothing() {
    let mut ui = filled_ui(true);
    frame(&mut ui, 1130, 960);
    ui.handle(Input::MouseMove { x: 5.0, y: 500.0 });
    ui.handle(Input::MouseDown);
    assert!(ui.handle(Input::MouseUp).is_empty());
}

#[test]
fn the_cursor_shows_what_can_be_clicked() {
    let mut ui = settings();
    let (x, y) = ui.locate("Field(Lan(0))").unwrap();
    ui.handle(Input::MouseMove { x, y });
    assert_eq!(ui.cursor(), Cursor::Text);

    let (x, y) = ui.locate("Cancel").unwrap();
    ui.handle(Input::MouseMove { x, y });
    assert_eq!(ui.cursor(), Cursor::Hand);

    ui.handle(Input::MouseLeft);
    assert_eq!(ui.cursor(), Cursor::Normal);
}

#[test]
fn it_only_redraws_when_something_changed() {
    let mut ui = filled_ui(true);
    frame(&mut ui, 1130, 960);
    assert!(!ui.needs_redraw());

    ui.handle(Input::MouseMove { x: 5.0, y: 500.0 });
    assert!(!ui.needs_redraw(), "moving over nothing shouldn't redraw");

    let generation = ui.dashboard.generation;
    ui.apply_poll(generation, 0, good(busy(40.0, 50.0), "a"), NOW);
    assert!(ui.needs_redraw());
}

#[test]
fn a_narrow_window_still_shows_the_feed() {
    let mut ui = new_ui(config(), true);
    let generation = ui.dashboard.generation;
    ui.apply_poll(generation, 0, bad(PollFailure::KeyMissing, "192.168.1.5", "This Strata needs a key. Add its API key or telemetry key in Settings."), NOW);
    ui.set_viewport(420, 700, 1.0);
    frame(&mut ui, 420, 700);
    assert!(ui.locate("Feed").is_some());

    let wide = frame(&mut ui, 420, 700);
    shot(&wide, "narrow-feed-dark");
}

#[test]
fn the_scrollbar_can_be_dragged() {
    let mut ui = filled_ui(true);
    ui.set_viewport(420, 700, 1.0);
    frame(&mut ui, 420, 700);
    ui.handle(Input::MouseMove { x: 200.0, y: 300.0 });
    frame(&mut ui, 420, 700);
    let before = frame(&mut ui, 420, 700);

    let (x, y) = ui.locate("ScrollThumb").unwrap();
    ui.handle(Input::MouseMove { x, y });
    ui.handle(Input::MouseDown);
    ui.handle(Input::MouseMove { x, y: y + 80.0 });
    ui.handle(Input::MouseUp);

    assert!(frame(&mut ui, 420, 700).data() != before.data());
}

/// Stops and restarts one server over and over, so the feed fills with events.
fn fill_the_feed(ui: &mut Ui, cycles: usize) {
    let generation = ui.dashboard.generation;
    for cycle in 0..cycles {
        let t = NOW + 100.0 + f64::from(cycle as u32) * 10.0;
        for _ in 0..3 {
            ui.apply_poll(generation, 0, bad(PollFailure::NotListening, "100.123.227.92", "Nothing is listening on this port."), t);
        }
        for _ in 0..3 {
            ui.apply_poll(generation, 0, good(busy(40.0, 50.0), "100.123.227.92"), t + 1.0);
        }
    }
}

#[test]
fn the_feed_scrolls_its_rows_with_the_wheel() {
    let mut ui = filled_ui(true);
    fill_the_feed(&mut ui, 12);
    let before = frame(&mut ui, 1130, 960);

    let (x, y) = ui.locate("Feed").unwrap();
    ui.handle(Input::MouseMove { x, y });
    assert_eq!(ui.cursor(), Cursor::Normal);
    for _ in 0..10 {
        ui.handle(Input::Wheel { lines: -1.0 });
    }

    let after = frame(&mut ui, 1130, 960);
    shot(&after, "feed-scrolled-dark");
    assert!(after.data() != before.data(), "scrolling the feed should change the rows it shows");
}

#[test]
fn the_feed_can_be_dragged_taller() {
    let mut ui = filled_ui(true);
    fill_the_feed(&mut ui, 12);
    let before = frame(&mut ui, 1130, 960);

    let (x, y) = ui.locate("FeedDivider").unwrap();
    ui.handle(Input::MouseMove { x, y });
    assert_eq!(ui.cursor(), Cursor::Resize);
    ui.handle(Input::MouseDown);
    ui.handle(Input::MouseMove { x, y: y - 150.0 });
    ui.handle(Input::MouseUp);

    let after = frame(&mut ui, 1130, 960);
    shot(&after, "feed-tall-dark");
    assert!(after.data() != before.data(), "dragging the feed's top edge should resize it");
}

// Updates, and the promise that nothing reaches GitHub unasked.

fn available() -> UpdateStatus {
    UpdateStatus::Available { version: "0.3.0".into(), tag: "v0.3.0".into(), link: "https://github.com/owner/StrataMon/releases/tag/v0.3.0".into() }
}

fn scrolled_to_updates(ui: &mut Ui) {
    frame(ui, 1130, 960);
    for _ in 0..60 {
        ui.handle(Input::Wheel { lines: -1.0 });
    }
    frame(ui, 1130, 960);
}

#[test]
fn nothing_contacts_github_unless_the_button_is_pressed() {
    let mut ui = filled_ui(true);
    let mut commands = Vec::new();

    // A long session of ordinary use: everything except that one button.
    frame(&mut ui, 1130, 960);
    commands.extend(ui.handle(Input::MouseMove { x: 300.0, y: 300.0 }));
    commands.extend(ui.handle(Input::Wheel { lines: -3.0 }));
    commands.extend(click(&mut ui, "OpenSettings"));
    frame(&mut ui, 1130, 960);
    commands.extend(click(&mut ui, "Field(Name(0))"));
    commands.extend(ui.handle(Input::Text("x".into())));
    commands.extend(ui.handle(Input::Key { key: Key::Tab, shift: false }));
    commands.extend(click(&mut ui, "ToggleNotifications"));
    commands.extend(click(&mut ui, "AddServer"));
    scrolled_to_updates(&mut ui);
    commands.extend(click(&mut ui, "Cancel"));
    let generation = ui.dashboard.generation;
    for i in 0..60 {
        ui.apply_poll(generation, 0, good(busy(40.0, 50.0), "a"), NOW + f64::from(i));
        frame(&mut ui, 1130, 960);
    }

    assert!(commands.iter().all(|c| !matches!(c, Command::CheckForUpdates | Command::InstallUpdate { .. } | Command::OpenLink(_))), "{commands:?}");
    assert_eq!(*ui.update_status(), UpdateStatus::Idle);
}

#[test]
fn pressing_the_button_is_what_checks() {
    let mut ui = settings();
    scrolled_to_updates(&mut ui);

    assert_eq!(click(&mut ui, "CheckForUpdates"), [Command::CheckForUpdates]);
    assert_eq!(*ui.update_status(), UpdateStatus::Checking);
}

#[test]
fn the_button_is_hidden_while_checking_so_it_cannot_be_pressed_twice() {
    let mut ui = settings();
    ui.set_update_status(UpdateStatus::Checking);
    scrolled_to_updates(&mut ui);
    assert!(ui.locate("CheckForUpdates").is_none());
}

#[test]
fn update_now_installs_that_version() {
    let mut ui = settings();
    ui.set_update_status(available());
    scrolled_to_updates(&mut ui);

    assert_eq!(click(&mut ui, "InstallUpdate"), [Command::InstallUpdate { tag: "v0.3.0".to_string() }]);
    assert_eq!(*ui.update_status(), UpdateStatus::Installing);
}

#[test]
fn the_link_opens_the_repo_page() {
    let mut ui = settings();
    ui.set_update_status(available());
    scrolled_to_updates(&mut ui);

    assert_eq!(click(&mut ui, "OpenReleasePage"), [Command::OpenLink("https://github.com/owner/StrataMon/releases/tag/v0.3.0".to_string())]);
}

#[test]
fn clicks_land_on_the_right_control_at_other_display_scales() {
    for scale in [1.25, 1.5, 2.0] {
        let mut ui = filled_ui(true);
        ui.set_viewport((1130.0 * scale) as u32, (960.0 * scale) as u32, scale);
        frame(&mut ui, (1130.0 * scale) as u32, (960.0 * scale) as u32);

        let (x, y) = ui.locate("OpenSettings").unwrap();
        ui.handle(Input::MouseMove { x, y });
        ui.handle(Input::MouseDown);
        ui.handle(Input::MouseUp);
        frame(&mut ui, (1130.0 * scale) as u32, (960.0 * scale) as u32);

        assert!(ui.locate("Save").is_some(), "at {scale}");
    }
}

#[test]
fn a_failed_install_keeps_the_link_it_points_to() {
    let mut ui = settings();
    let link = "https://github.com/owner/StrataMon/releases/tag/v0.3.0".to_string();
    ui.set_update_status(UpdateStatus::InstallFailed { message: "This folder can't be written to.".to_string(), link: link.clone() });
    scrolled_to_updates(&mut ui);

    assert_eq!(click(&mut ui, "OpenReleasePage"), [Command::OpenLink(link)]);
    assert!(ui.locate("InstallUpdate").is_none());
    assert!(click(&mut ui, "DismissUpdate").is_empty());
    assert_eq!(*ui.update_status(), UpdateStatus::Idle);
}

#[test]
fn dismiss_closes_the_offer_and_does_nothing_else() {
    let mut ui = settings();
    ui.set_update_status(available());
    scrolled_to_updates(&mut ui);

    assert!(click(&mut ui, "DismissUpdate").is_empty());
    assert_eq!(*ui.update_status(), UpdateStatus::Idle);
    frame(&mut ui, 1130, 960);
    assert!(ui.locate("InstallUpdate").is_none());
}

#[test]
fn update_choices_only_appear_when_there_is_an_update() {
    let mut ui = settings();
    scrolled_to_updates(&mut ui);
    assert!(ui.locate("InstallUpdate").is_none());
    assert!(ui.locate("OpenReleasePage").is_none());
    assert!(ui.locate("DismissUpdate").is_none());
}

// The text field on its own.

#[test]
fn a_field_starts_with_the_caret_at_the_end() {
    assert_eq!(Field::new("abc", false).caret(), 3);
}

#[test]
fn a_field_handles_text_that_is_not_ascii() {
    let mut field = Field::new("héllo→", false);
    field.backspace();
    field.left(false);
    field.backspace();
    assert_eq!(field.text(), "hélo");
}

#[test]
fn a_field_replaces_a_selection_when_typing() {
    let mut field = Field::new("abcdef", false);
    field.home(false);
    field.right(false);
    field.right(true);
    field.right(true);
    field.insert("X");
    assert_eq!(field.text(), "aXdef");
    assert_eq!(field.caret(), 2);
}

#[test]
fn a_field_collapses_a_selection_with_the_arrows() {
    let mut field = Field::new("abcdef", false);
    field.select_all();
    field.left(false);
    assert_eq!(field.caret(), 0);
    assert_eq!(field.selection(), None);

    field.select_all();
    field.right(false);
    assert_eq!(field.caret(), 6);
}

#[test]
fn a_field_keeps_its_caret_in_bounds() {
    let mut field = Field::new("ab", false);
    field.home(false);
    field.left(false);
    field.backspace();
    field.end(false);
    field.right(false);
    field.delete();
    field.move_to(99, false);
    assert_eq!(field.text(), "ab");
    assert_eq!(field.caret(), 2);
}

#[test]
fn a_field_drops_control_characters_and_extra_lines_from_a_paste() {
    let mut field = Field::new("", false);
    field.insert("first\tline\nsecond line");
    assert_eq!(field.text(), "firstline");
}

#[test]
fn a_password_shows_only_its_length() {
    let field = Field::new("secret", true);
    assert_eq!(field.display(), MASK.to_string().repeat(6));
    assert!(!field.display().contains("secret"));
}

// Redraws that nothing else would ask for.

fn hover_nothing(ui: &mut Ui) {
    ui.handle(Input::MouseMove { x: 2.0, y: 2.0 });
    frame(ui, 1130, 960);
    assert!(!ui.needs_redraw(), "setup: nothing pending");
}

#[test]
fn escape_closes_settings_on_screen_with_nothing_hovered() {
    let mut ui = settings();
    hover_nothing(&mut ui);

    key(&mut ui, Key::Escape);

    assert!(!ui.is_settings_open());
    assert!(ui.needs_redraw(), "the closed settings page stays on screen");
}

#[test]
fn enter_saves_and_closes_settings_on_screen_with_nothing_hovered() {
    let mut ui = settings();
    hover_nothing(&mut ui);

    assert!(matches!(key(&mut ui, Key::Enter).as_slice(), [Command::SaveSettings(_)]));
    assert!(!ui.is_settings_open());
    assert!(ui.needs_redraw(), "the saved settings page stays on screen");
}

#[test]
fn enter_on_an_unfinished_form_draws_its_error() {
    let mut ui = new_ui(AppConfig::default(), true);
    click(&mut ui, "OpenSettings");
    frame(&mut ui, 1130, 960);
    click(&mut ui, "AddServer");
    frame(&mut ui, 1130, 960);
    hover_nothing(&mut ui);

    assert!(key(&mut ui, Key::Enter).is_empty());
    assert!(ui.is_settings_open());
    assert!(ui.needs_redraw(), "the validation error is never drawn");
}

// The window and the feed.

// Minimizing on Windows reports a window of no size at all. The feed's height mustn't be
// clamped to that and stay there once the window is back.
#[test]
fn minimizing_and_restoring_keeps_the_feed_height() {
    let mut ui = filled_ui(true);
    frame(&mut ui, 1130, 960);
    let before = ui.locate("FeedDivider").unwrap();

    ui.set_viewport(0, 0, 1.0);
    frame(&mut ui, 1, 1);
    ui.set_viewport(1130, 960, 1.0);
    frame(&mut ui, 1130, 960);

    assert_eq!(ui.locate("FeedDivider").unwrap(), before);
}

fn over_a_full_feed() -> (Ui, tiny_skia::Pixmap) {
    let mut ui = filled_ui(true);
    fill_the_feed(&mut ui, 12);
    frame(&mut ui, 1130, 960);
    let (x, y) = ui.locate("Feed").unwrap();
    ui.handle(Input::MouseMove { x, y });
    let before = frame(&mut ui, 1130, 960);
    (ui, before)
}

// A tilt wheel or a sideways swipe on a touchpad has no up-or-down movement at all.
#[test]
fn a_sideways_wheel_event_does_not_scroll_the_feed() {
    let (mut ui, before) = over_a_full_feed();

    ui.handle(Input::Wheel { lines: 0.0 });

    assert!(frame(&mut ui, 1130, 960).data() == before.data(), "a sideways scroll moved the feed");
}

// A touchpad sends many small movements. Each one used to move a whole row.
#[test]
fn small_wheel_movements_add_up_to_rows_in_the_feed() {
    let (mut ui, before) = over_a_full_feed();

    for _ in 0..3 {
        ui.handle(Input::Wheel { lines: -0.25 });
    }
    assert!(frame(&mut ui, 1130, 960).data() == before.data(), "three quarters of a row moved the feed");

    ui.handle(Input::Wheel { lines: -0.25 });
    assert!(frame(&mut ui, 1130, 960).data() != before.data(), "a whole row's worth didn't move the feed");
}

#[test]
fn opening_settings_during_an_install_keeps_it_installing() {
    let mut ui = filled_ui(true);
    ui.set_update_status(UpdateStatus::Installing);

    click(&mut ui, "OpenSettings");

    assert_eq!(*ui.update_status(), UpdateStatus::Installing);
    scrolled_to_updates(&mut ui);
    assert!(ui.locate("CheckForUpdates").is_none(), "a second check or install could start");
}
