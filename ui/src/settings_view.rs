//! The Settings page: servers, alert limits, notifications and updates.

use crate::Scroll;
use crate::dashboard_view::draw_scrollbar;
use crate::field::Field;
use crate::text::Face;
use crate::view::{BODY, CONTROL_HEIGHT, FieldId, SMALL, Target, View};
use stratamon_core::config::AppConfig;
use stratamon_core::settings::{ServerForm, SettingsForm};
use stratamon_core::update::{self, UpdateStatus};

const MARGIN: f32 = 20.0;
const PAGE_WIDTH: f32 = 640.0;
const LABEL_WIDTH: f32 = 130.0;
const LIMIT_LABEL_WIDTH: f32 = 220.0;
const LIMIT_WIDTH: f32 = 140.0;
const ROW: f32 = CONTROL_HEIGHT + 8.0;
const FOOTER: f32 = 64.0;

pub struct ServerFields {
    pub name: Field,
    pub lan: Field,
    pub tailscale: Field,
    pub key: Field,
}

impl ServerFields {
    fn new(form: &ServerForm) -> Self {
        Self {
            name: Field::new(&form.name, false),
            lan: Field::new(&form.lan_address, false),
            tailscale: Field::new(&form.tailscale_address, false),
            key: Field::new(&form.key, true),
        }
    }
}

pub struct SettingsPage {
    pub form: SettingsForm,
    pub servers: Vec<ServerFields>,
    pub gpu_temperature: Field,
    pub ram_percent: Field,
    pub sustain_seconds: Field,
    pub queue_wait_seconds: Field,
    pub focused: Option<FieldId>,
    pub scroll: Scroll,

    /// Every field in the order Tab moves through them.
    pub order: Vec<FieldId>,
}

impl SettingsPage {
    pub fn new(config: &AppConfig) -> Self {
        let form = SettingsForm::new(config);
        Self {
            servers: form.servers.iter().map(ServerFields::new).collect(),
            gpu_temperature: Field::new(&form.gpu_temperature, false),
            ram_percent: Field::new(&form.ram_percent, false),
            sustain_seconds: Field::new(&form.sustain_seconds, false),
            queue_wait_seconds: Field::new(&form.queue_wait_seconds, false),
            form,
            focused: None,
            scroll: Scroll::default(),
            order: Vec::new(),
        }
    }

    pub fn field_mut(&mut self, id: FieldId) -> Option<&mut Field> {
        match id {
            FieldId::Name(i) => self.servers.get_mut(i).map(|s| &mut s.name),
            FieldId::Lan(i) => self.servers.get_mut(i).map(|s| &mut s.lan),
            FieldId::Tailscale(i) => self.servers.get_mut(i).map(|s| &mut s.tailscale),
            FieldId::Key(i) => self.servers.get_mut(i).map(|s| &mut s.key),
            FieldId::GpuTemperature => Some(&mut self.gpu_temperature),
            FieldId::RamPercent => Some(&mut self.ram_percent),
            FieldId::SustainSeconds => Some(&mut self.sustain_seconds),
            FieldId::QueueWaitSeconds => Some(&mut self.queue_wait_seconds),
        }
    }

    /// Copies what's typed into the form, which holds the rules for what's valid.
    pub fn sync_form(&mut self) {
        for (form, fields) in self.form.servers.iter_mut().zip(&self.servers) {
            form.name = fields.name.text().to_string();
            form.lan_address = fields.lan.text().to_string();
            form.tailscale_address = fields.tailscale.text().to_string();
            form.key = fields.key.text().to_string();
        }

        self.form.gpu_temperature = self.gpu_temperature.text().to_string();
        self.form.ram_percent = self.ram_percent.text().to_string();
        self.form.sustain_seconds = self.sustain_seconds.text().to_string();
        self.form.queue_wait_seconds = self.queue_wait_seconds.text().to_string();
    }

    pub fn add_server(&mut self) {
        self.sync_form();
        self.form.add_server();
        if let Some(added) = self.form.servers.last() {
            self.servers.push(ServerFields::new(added));
        }
    }

    pub fn remove_server(&mut self, index: usize) {
        self.sync_form();
        if index < self.servers.len() {
            self.servers.remove(index);
            self.form.servers.remove(index);
            self.focused = None;
        }
    }
}

pub fn draw(v: &mut View, page: &mut SettingsPage, status: &UpdateStatus, width: f32, height: f32) {
    let bottom = height - FOOTER;
    let column = (width - MARGIN * 2.0 - 14.0).min(PAGE_WIDTH);
    let left = ((width - column) / 2.0).max(MARGIN);

    v.measure(true);
    let content = draw_content(v, left, 0.0, column, page, status);
    v.measure(false);
    page.scroll.set(content + 32.0, bottom);

    v.visible = (0.0, bottom);
    let top = 16.0 - page.scroll.offset;
    page.order.clear();
    draw_content(v, left, top, column, page, status);
    v.visible = (0.0, height);

    let surface = v.theme.page;
    v.c.fill_rect(0.0, bottom, width, FOOTER, surface);
    v.hairline(0.0, bottom, width);

    let right = left + column;
    let save = v.button_width("Save");
    let cancel = v.button_width("Cancel");
    let footer_y = bottom + 16.0;
    v.button(right - save, footer_y, "Save", Target::Save, true);
    v.button(right - save - 8.0 - cancel, footer_y, "Cancel", Target::Cancel, false);

    if let Some(error) = &page.form.error {
        let primary = v.theme.ink_primary;
        v.c.text_fit(Face::Regular, left, footer_y + 6.0, column - save - cancel - 28.0, error, BODY, primary);
    }

    if page.scroll.max > 0.0 {
        draw_scrollbar(v, width - 10.0, 0.0, bottom, &page.scroll);
    }
}

fn draw_content(v: &mut View, left: f32, top: f32, column: f32, page: &mut SettingsPage, status: &UpdateStatus) -> f32 {
    let (primary, secondary, _) = v.ink();
    let mut y = top;

    v.c.text(Face::Semibold, left, y, "Servers", 15.0, primary);
    y += 26.0;
    let intro = "StrataMon reads Strata's own monitoring on port 8080 unless you add another. Strata only answers on its own PC until it's opened to your network or tailnet. The README shows how.";
    y += v.c.text_wrapped(Face::Regular, left, y, column, intro, BODY, secondary) + 14.0;

    for index in 0..page.servers.len() {
        y = draw_server(v, left, y, column, page, index) + 12.0;
    }

    y += CONTROL_HEIGHT.min(v.button(left, y, "Add a server", Target::AddServer, false)) + 16.0;
    v.hairline(left, y, column);
    y += 16.0;

    v.c.text(Face::Semibold, left, y, "Alert limits", 15.0, primary);
    y += 28.0;
    let focused = page.focused;
    for (label, id) in [
        ("GPU temperature (°C)", FieldId::GpuTemperature),
        ("System RAM (%)", FieldId::RamPercent),
        ("Must last for (seconds)", FieldId::SustainSeconds),
        ("Requests queued for (seconds)", FieldId::QueueWaitSeconds),
    ] {
        v.c.text(Face::Regular, left, y + 6.0, label, BODY, secondary);
        if let Some(field) = page.field_mut(id) {
            v.field((left + LIMIT_LABEL_WIDTH, y), LIMIT_WIDTH, id, field, "", focused == Some(id));
        }
        if !v.measuring {
            page.order.push(id);
        }
        y += ROW;
    }

    y += 6.0;
    let checkbox = v.checkbox(left, y, "Show desktop notifications", page.form.desktop_notifications, Target::ToggleNotifications);
    v.button(left + checkbox + 16.0, y, "Send test notification", Target::SendTestNotification, false);
    y += CONTROL_HEIGHT + 16.0;
    v.hairline(left, y, column);
    y += 16.0;

    draw_updates(v, left, y, column, status)
}

fn draw_server(v: &mut View, left: f32, top: f32, column: f32, page: &mut SettingsPage, index: usize) -> f32 {
    let (_, secondary, _) = v.ink();
    let test_result = page.form.servers.get(index).and_then(|form| form.test_result.clone());
    let padding = 14.0;
    let inner_left = left + padding;
    let inner = column - padding * 2.0;
    let field_x = inner_left + LABEL_WIDTH;
    let field_width = inner - LABEL_WIDTH;

    // The height depends on the test result, which wraps.
    let result_lines = test_result.as_ref().map_or(Vec::new(), |text| v.c.wrap(Face::Regular, text, BODY, field_width));
    let result_height = result_lines.len() as f32 * v.c.line_height(Face::Regular, BODY);
    let height = padding * 2.0 + ROW * 5.0 - 8.0 + if result_height > 0.0 { result_height + 8.0 } else { 0.0 };

    let (surface, hairline) = (v.theme.surface, v.theme.hairline);
    v.c.fill_rounded(left, top, column, height, 8.0, surface);
    v.c.stroke_rounded(left, top, column, height, 8.0, 1.0, hairline);

    let focused = page.focused;
    let mut y = top + padding;
    for (label, id, placeholder) in [
        ("Name", FieldId::Name(index), ""),
        ("LAN address", FieldId::Lan(index), "192.168.1.10 or 192.168.1.10:8095"),
        ("Tailscale address", FieldId::Tailscale(index), "100.x.y.z (optional)"),
        ("Strata key", FieldId::Key(index), "API key or telemetry key, if it has one"),
    ] {
        v.c.text(Face::Regular, inner_left, y + 6.0, label, BODY, secondary);
        if let Some(field) = page.field_mut(id) {
            v.field((field_x, y), field_width, id, field, placeholder, focused == Some(id));
        }
        if !v.measuring {
            page.order.push(id);
        }
        y += ROW;
    }

    let test = v.button(field_x, y, "Test connection", Target::Test(index), false);
    v.button(field_x + test + 8.0, y, "Remove", Target::Remove(index), false);
    y += ROW;

    let line_height = v.c.line_height(Face::Regular, BODY);
    for (line, text) in result_lines.iter().enumerate() {
        v.c.text(Face::Regular, field_x, y + line as f32 * line_height, text, BODY, secondary);
    }

    top + height
}

fn draw_updates(v: &mut View, left: f32, top: f32, column: f32, status: &UpdateStatus) -> f32 {
    let (primary, secondary, muted) = v.ink();
    let mut y = top;
    v.c.text(Face::Semibold, left, y, "Updates", 15.0, primary);
    y += 28.0;

    let version = format!("StrataMon {}", update::VERSION);
    let end = v.c.text(Face::Regular, left, y + 6.0, &version, BODY, secondary);
    if !matches!(status, UpdateStatus::Checking | UpdateStatus::Installing) {
        v.button(end + 16.0, y, "Check for updates", Target::CheckForUpdates, false);
    }
    y += CONTROL_HEIGHT + 8.0;

    let note = "StrataMon only contacts GitHub when you press this button. It never checks by itself, and it sends no telemetry.";
    y += v.c.text_wrapped(Face::Regular, left, y, column, note, SMALL, muted) + 8.0;

    match status {
        UpdateStatus::Idle => {}
        UpdateStatus::Checking => {
            y += v.c.text_wrapped(Face::Regular, left, y, column, "Checking GitHub…", BODY, secondary);
        }
        UpdateStatus::UpToDate => {
            y += v.c.text_wrapped(Face::Regular, left, y, column, "You're on the latest version.", BODY, secondary);
        }
        UpdateStatus::Installing => {
            y += v.c.text_wrapped(Face::Regular, left, y, column, "Downloading and checking the update…", BODY, secondary);
        }
        UpdateStatus::Failed(message) => {
            y += v.c.text_wrapped(Face::Regular, left, y, column, message, BODY, secondary);
        }
        UpdateStatus::InstallFailed { message, .. } => {
            y += v.c.text_wrapped(Face::Regular, left, y, column, message, BODY, secondary) + 8.0;
            let x = left + v.link(left, y, "View on GitHub", Target::OpenReleasePage) + 16.0;
            v.button(x, y, "Dismiss", Target::DismissUpdate, false);
            y += CONTROL_HEIGHT;
        }
        UpdateStatus::Available { version, .. } => {
            y += v.c.text_wrapped(Face::Semibold, left, y, column, &format!("Version {version} is available."), BODY, primary) + 6.0;
            let mut x = left;
            x += v.button(x, y, "Update now", Target::InstallUpdate, true) + 12.0;
            x += v.link(x, y, "View on GitHub", Target::OpenReleasePage) + 16.0;
            v.button(x, y, "Dismiss", Target::DismissUpdate, false);
            y += CONTROL_HEIGHT;
        }
    }

    y
}
