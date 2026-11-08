use stratamon_core::client::{PollFailure, PollResult};
use stratamon_core::config::{AppConfig, ServerConfig};
use stratamon_core::dashboard::Dashboard;
use stratamon_core::models::StrataSnapshot;

const NOW: f64 = 1_800_000_000.0;

fn config(notifications: bool) -> AppConfig {
    let server = |name: &str| ServerConfig { id: name.into(), name: name.into(), addresses: vec!["a".into()], ..Default::default() };
    AppConfig { servers: vec![server("NEDM"), server("NEDM2")], desktop_notifications: notifications, ..Default::default() }
}

fn good() -> PollResult {
    PollResult { snapshot: Some(StrataSnapshot::default()), failure: PollFailure::None, address: Some("a".into()), detail: None }
}

fn bad(failure: PollFailure) -> PollResult {
    PollResult { snapshot: None, failure, address: Some("a".into()), detail: Some("no".into()) }
}

#[test]
fn makes_a_card_for_each_server() {
    let dashboard = Dashboard::new(config(true));
    let names: Vec<&str> = dashboard.servers.iter().map(|s| s.card.name.as_str()).collect();
    assert_eq!(names, ["NEDM", "NEDM2"]);
}

#[test]
fn a_poll_updates_its_card_and_the_summary() {
    let mut dashboard = Dashboard::new(config(true));
    let generation = dashboard.generation;

    dashboard.apply(generation, 1, good(), NOW);

    assert!(dashboard.servers[1].card.is_online());
    assert!(!dashboard.servers[0].card.is_online());
    assert_eq!(dashboard.summary, "1 of 2 online · no active alerts");
}

#[test]
fn a_poll_from_before_a_settings_change_is_ignored() {
    let mut dashboard = Dashboard::new(config(true));
    let old = dashboard.generation;

    dashboard.watch(config(true));
    dashboard.apply(old, 0, good(), NOW);

    assert!(!dashboard.servers[0].card.is_online());
}

#[test]
fn a_poll_for_a_server_that_no_longer_exists_is_ignored() {
    let mut dashboard = Dashboard::new(config(true));
    let generation = dashboard.generation;
    assert!(dashboard.apply(generation, 9, good(), NOW).is_empty());
}

#[test]
fn a_new_alert_notifies_and_counts_as_active() {
    let mut dashboard = Dashboard::new(config(true));
    let generation = dashboard.generation;

    let notifications = dashboard.apply(generation, 0, bad(PollFailure::KeyRejected), NOW);

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].critical);
    assert_eq!(dashboard.active_alerts.len(), 1);
    assert_eq!(dashboard.recent_alerts.len(), 1);
    assert_eq!(dashboard.summary, "0 of 2 online · 1 active alert");
}

#[test]
fn notifications_can_be_turned_off_without_hiding_the_alert() {
    let mut dashboard = Dashboard::new(config(false));
    let generation = dashboard.generation;

    assert!(dashboard.apply(generation, 0, bad(PollFailure::KeyRejected), NOW).is_empty());
    assert_eq!(dashboard.active_alerts.len(), 1);
}

#[test]
fn coming_back_online_notifies_but_is_not_critical() {
    let mut dashboard = Dashboard::new(config(true));
    let generation = dashboard.generation;
    dashboard.apply(generation, 0, bad(PollFailure::KeyRejected), NOW);

    let notifications = dashboard.apply(generation, 0, good(), NOW);

    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].message, "NEDM is back online");
    assert!(!notifications[0].critical);
    assert!(dashboard.active_alerts.is_empty());
}

#[test]
fn newest_alerts_come_first_and_the_list_has_a_limit() {
    let mut dashboard = Dashboard::new(config(false));
    let generation = dashboard.generation;
    for i in 0..60 {
        dashboard.apply(generation, 0, bad(PollFailure::KeyRejected), NOW + f64::from(i));
        dashboard.apply(generation, 0, good(), NOW + f64::from(i));
    }

    assert_eq!(dashboard.recent_alerts.len(), 50);
    assert_eq!(dashboard.recent_alerts[0].kind, "Resolved");
}

#[test]
fn changing_settings_clears_active_alerts_but_keeps_the_history() {
    let mut dashboard = Dashboard::new(config(false));
    let generation = dashboard.generation;
    dashboard.apply(generation, 0, bad(PollFailure::KeyRejected), NOW);

    dashboard.watch(config(false));

    assert!(dashboard.active_alerts.is_empty());
    assert_eq!(dashboard.recent_alerts.len(), 1);
}

#[test]
fn no_servers_means_no_summary() {
    assert_eq!(Dashboard::new(AppConfig::default()).summary, "");
}
