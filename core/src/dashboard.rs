//! The state of the main window: every watched server, the alert lists and the summary line.

use crate::alerts::{AlertChange, AlertEngine, AlertEvent, AlertSeverity, OFFLINE_KEY};
use crate::card::{AlertRow, ServerCard};
use crate::client::PollResult;
use crate::config::{AppConfig, ServerConfig};
use crate::state::{ServerState, ServerStatus, Time};

const MAX_RECENT_ALERTS: usize = 50;

pub struct Watched {
    pub config: ServerConfig,
    pub state: ServerState,
    pub card: ServerCard,
}

/// A desktop notification the app should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub message: String,
    pub critical: bool,
}

pub struct Dashboard {
    pub config: AppConfig,

    /// Goes up with every settings change, so a poll from before the change can be ignored.
    pub generation: u64,
    pub servers: Vec<Watched>,
    pub active_alerts: Vec<AlertRow>,
    pub recent_alerts: Vec<AlertRow>,
    pub summary: String,
    pub config_notice: Option<String>,
    alerts: AlertEngine,
}

impl Dashboard {
    pub fn new(config: AppConfig) -> Self {
        let mut dashboard = Self {
            alerts: AlertEngine::new(config.alerts.clone()),
            config: AppConfig::default(),
            generation: 0,
            servers: Vec::new(),
            active_alerts: Vec::new(),
            recent_alerts: Vec::new(),
            summary: String::new(),
            config_notice: None,
        };
        dashboard.watch(config);
        dashboard
    }

    /// Starts watching a new set of servers, keeping the recent alerts as history.
    pub fn watch(&mut self, config: AppConfig) {
        self.generation += 1;
        self.active_alerts.clear();
        self.alerts = AlertEngine::new(config.alerts.clone());
        self.servers = config
            .servers
            .iter()
            .map(|server| Watched {
                config: server.clone(),
                state: ServerState::default(),
                card: ServerCard::new(&server.name, config.alerts.clone(), config.poll_seconds),
            })
            .collect();
        self.config = config;
        self.update_summary();
    }

    /// Applies one poll, and returns any notifications it calls for.
    pub fn apply(&mut self, generation: u64, index: usize, result: PollResult, now: Time) -> Vec<Notification> {
        // A poll from before a settings change can land after it.
        if generation != self.generation {
            return Vec::new();
        }

        let Some(watched) = self.servers.get_mut(index) else {
            return Vec::new();
        };

        watched.state.apply(result, now);
        watched.card.update(&watched.state, now);
        let events = self.alerts.evaluate(&watched.config, &watched.state, now);

        let notifications = self.record(&events);
        self.update_summary();
        notifications
    }

    fn record(&mut self, events: &[AlertEvent]) -> Vec<Notification> {
        let mut notifications = Vec::new();
        for alert in events {
            self.recent_alerts.insert(0, AlertRow::new(alert));
            self.recent_alerts.truncate(MAX_RECENT_ALERTS);

            let worth_interrupting = alert.change != AlertChange::Resolved || alert.key == OFFLINE_KEY;
            if self.config.desktop_notifications && worth_interrupting {
                notifications.push(Notification {
                    message: alert.message.clone(),
                    critical: alert.severity == AlertSeverity::Critical && alert.change == AlertChange::Raised,
                });
            }
        }

        if !events.is_empty() {
            self.active_alerts = self.alerts.active().into_iter().map(AlertRow::new).collect();
        }

        notifications
    }

    fn update_summary(&mut self) {
        let online = self.servers.iter().filter(|s| s.state.status == ServerStatus::Online).count();
        let alerts = match self.active_alerts.len() {
            0 => "no active alerts".to_string(),
            1 => "1 active alert".to_string(),
            count => format!("{count} active alerts"),
        };

        self.summary = if self.servers.is_empty() { String::new() } else { format!("{online} of {} online · {alerts}", self.servers.len()) };
    }
}
