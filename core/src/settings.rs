//! The rules behind the Settings page: what the form holds, and what counts as a usable server.

use crate::address;
use crate::config::{AlertThresholds, AppConfig, ServerConfig};
use crate::models::{StrataSnapshot, TelemetryAccess};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerForm {
    id: String,
    pub name: String,
    pub lan_address: String,
    pub tailscale_address: String,
    pub key: String,
    pub test_result: Option<String>,

    // The form shows two addresses. Any others added by hand survive a save.
    other_addresses: Vec<String>,
}

impl ServerForm {
    pub fn new(server: &ServerConfig) -> Self {
        Self {
            id: server.id.clone(),
            name: server.name.clone(),
            lan_address: server.addresses.first().cloned().unwrap_or_default(),
            tailscale_address: server.addresses.get(1).cloned().unwrap_or_default(),
            key: server.key.clone(),
            test_result: None,
            other_addresses: server.addresses.iter().skip(2).cloned().collect(),
        }
    }

    /// Stays the same while the form is open, so a test result finds its server after edits.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn to_config(&self) -> ServerConfig {
        let typed = [self.lan_address.trim(), self.tailscale_address.trim()];
        // The same address in both boxes, or repeated in the file, would get polled twice a
        // round and spend the whole time budget on one server. Keep the first copy.
        let mut addresses: Vec<String> = Vec::new();
        for address in typed.into_iter().filter(|address| !address.is_empty()).map(String::from).chain(self.other_addresses.iter().cloned()) {
            if !addresses.iter().any(|kept| address::normalize(kept) == address::normalize(&address)) {
                addresses.push(address);
            }
        }

        ServerConfig {
            id: self.id.clone(),
            name: self.name.trim().to_string(),
            addresses,
            key: self.key.trim().to_string(),
        }
    }

    /// Returns what's wrong with this entry, or `None` if it's usable.
    pub fn validate(&self) -> Option<String> {
        let config = self.to_config();
        if config.name.is_empty() {
            return Some("Every server needs a name.".to_string());
        }

        if config.addresses.is_empty() {
            return Some(format!("{} needs at least one address.", config.name));
        }

        config
            .addresses
            .iter()
            .find(|address| address::normalize(address).is_none())
            .map(|invalid| format!("{}: \"{invalid}\" isn't a valid address.", config.name))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsForm {
    pub servers: Vec<ServerForm>,
    pub gpu_temperature: String,
    pub ram_percent: String,
    pub sustain_seconds: String,
    pub queue_wait_seconds: String,
    pub desktop_notifications: bool,
    pub error: Option<String>,
    original: AppConfig,
}

impl SettingsForm {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            servers: config.servers.iter().map(ServerForm::new).collect(),
            gpu_temperature: config.alerts.gpu_temperature_c.to_string(),
            ram_percent: config.alerts.ram_percent.to_string(),
            sustain_seconds: config.alerts.sustain_seconds.to_string(),
            queue_wait_seconds: config.alerts.queue_wait_seconds.to_string(),
            desktop_notifications: config.desktop_notifications,
            error: None,
            original: config.clone(),
        }
    }

    pub fn add_server(&mut self) {
        let server = ServerConfig { name: format!("strata-{}", self.servers.len() + 1), ..ServerConfig::default() };
        self.servers.push(ServerForm::new(&server));
    }

    /// Builds the new config, or sets `error` and returns `None` if something's wrong.
    pub fn try_build_config(&mut self) -> Option<AppConfig> {
        self.error = self.servers.iter().find_map(ServerForm::validate);
        if self.error.is_some() {
            return None;
        }

        // A cleared or unreadable limit falls back rather than becoming zero.
        let defaults = &self.original.alerts;
        Some(AppConfig {
            servers: self.servers.iter().map(ServerForm::to_config).collect(),
            desktop_notifications: self.desktop_notifications,
            alerts: AlertThresholds {
                gpu_temperature_c: limit(&self.gpu_temperature, 1.0, 150.0).unwrap_or(defaults.gpu_temperature_c),
                ram_percent: limit(&self.ram_percent, 1.0, 100.0).unwrap_or(defaults.ram_percent),
                sustain_seconds: limit(&self.sustain_seconds, 0.0, 3600.0).map_or(defaults.sustain_seconds, |v| v as i32),
                queue_wait_seconds: limit(&self.queue_wait_seconds, 0.0, 3600.0).map_or(defaults.queue_wait_seconds, |v| v as i32),
            },
            ..self.original.clone()
        })
    }
}

fn limit(text: &str, smallest: f64, largest: f64) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|value| value.is_finite()).map(|value| value.clamp(smallest, largest))
}

/// The result line for Test connection.
pub fn describe_connection(snapshot: &StrataSnapshot) -> String {
    let connected = format!("connected to {}", snapshot.model.as_deref().unwrap_or("Strata"));
    match snapshot.access {
        TelemetryAccess::TelemetryOnly => {
            format!("{connected}, read-only. The KV cache type, CPU, disk, PCIe and expert readings need Strata's API key.")
        }
        TelemetryAccess::MetricsOnly => format!("{connected}. This Strata is older, so queue limits aren't reported."),
        TelemetryAccess::Full => format!("{connected}, with every reading."),
    }
}
