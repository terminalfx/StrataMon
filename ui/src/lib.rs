//! The whole interface as a plain function of state and input: events go in, pixels come out, and
//! no window is involved. The app supplies a window around it, and tests can click, type and render
//! with no display at all, on any operating system.

pub mod canvas;
pub mod field;
pub mod text;
pub mod theme;

mod dashboard_view;
mod settings_view;
mod view;

use canvas::Canvas;
use dashboard_view::Offsets;
use settings_view::SettingsPage;
use stratamon_core::client::PollResult;
use stratamon_core::config::{AppConfig, ServerConfig};
use stratamon_core::dashboard::{Dashboard, Notification};
use stratamon_core::update::UpdateStatus;
use text::{Face, Text};
use theme::Theme;
use tiny_skia::Pixmap;
use view::{BODY, FieldId, Hit, Target, View};

pub const DEFAULT_WIDTH: f32 = 1130.0;
pub const DEFAULT_HEIGHT: f32 = 960.0;

/// How far one wheel notch scrolls, in logical pixels. The app turns a touchpad's pixel deltas
/// into notches with it too.
pub const WHEEL_LINE: f32 = 48.0;

// Dark text on a light background needs more help than light on dark.
const LIGHT_TEXT_GAMMA: f32 = 0.80;
const DARK_TEXT_GAMMA: f32 = 0.92;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Tab,
    Enter,
    Escape,
    /// A letter pressed with Ctrl, for select all, copy, cut and paste.
    Control(char),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// In physical pixels, as windows report them.
    MouseMove { x: f32, y: f32 },
    MouseDown,
    MouseUp,
    MouseLeft,
    Wheel { lines: f32 },
    Key { key: Key, shift: bool },
    Text(String),
    Paste(String),
}

/// Something only the app around the interface can do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    SaveSettings(AppConfig),
    TestConnection { form_id: String, server: ServerConfig },
    SendTestNotification,
    CheckForUpdates,
    InstallUpdate { tag: String },
    OpenLink(String),
    CopyText(String),
    RequestPaste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Normal,
    Hand,
    Text,
    /// Over a divider that can be dragged to resize something.
    Resize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Scroll {
    pub offset: f32,
    pub max: f32,
    content: f32,
    viewport: f32,
}

impl Scroll {
    pub fn set(&mut self, content: f32, viewport: f32) {
        self.content = content;
        self.viewport = viewport.max(0.0);
        self.max = (content - self.viewport).max(0.0);
        self.offset = self.offset.clamp(0.0, self.max);
    }

    fn by(&mut self, amount: f32) {
        self.offset = (self.offset + amount).clamp(0.0, self.max);
    }

    /// Where the scrollbar's thumb sits within a track.
    fn thumb(&self, top: f32, height: f32) -> (f32, f32) {
        let size = (height * self.viewport / self.content.max(1.0)).max(24.0).min(height);
        let travel = height - size;
        let share = if self.max > 0.0 { self.offset / self.max } else { 0.0 };
        (top + travel * share, size)
    }

    fn drag(&mut self, delta: f32) {
        // Use the track this scrollbar's thumb was measured against, not the caller's guess:
        // dragging on a different height than the drawing moves the thumb off its own rate.
        let track = self.viewport.max(1.0);
        let (_, size) = self.thumb(0.0, track);
        let travel = (track - size).max(1.0);
        self.by(delta / travel * self.max);
    }
}

pub struct Ui {
    pub dashboard: Dashboard,
    text: Text,
    theme: Theme,
    scale: f32,
    width: f32,
    height: f32,
    settings: Option<SettingsPage>,
    update: UpdateStatus,
    scroll: Scroll,
    feed_offsets: Offsets,

    /// The height the feed was given. What's drawn is this clamped to the window, so a window
    /// that's briefly small, or minimized to nothing, gives it back when it's big again.
    feed_height: f32,
    feed_drag_from: Option<f32>,

    /// Wheel movement over the feed that hasn't added up to a whole row yet.
    feed_wheel: f32,
    hits: Vec<Hit>,
    hot: Option<Target>,
    tip: Option<String>,
    pressed: Option<Target>,
    selecting: Option<FieldId>,
    drag_from: Option<f32>,
    mouse: Option<(f32, f32)>,
    dirty: bool,
}

impl Ui {
    pub fn new(config: AppConfig, text: Text, dark: bool) -> Self {
        let mut ui = Self {
            dashboard: Dashboard::new(config),
            text,
            theme: theme::LIGHT,
            scale: 1.0,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            settings: None,
            update: UpdateStatus::Idle,
            scroll: Scroll::default(),
            feed_offsets: Offsets { feed: 0 },
            feed_height: dashboard_view::FEED_DEFAULT_HEIGHT,
            feed_drag_from: None,
            feed_wheel: 0.0,
            hits: Vec::new(),
            hot: None,
            tip: None,
            pressed: None,
            selecting: None,
            drag_from: None,
            mouse: None,
            dirty: true,
        };
        ui.set_dark(dark);
        ui
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.theme = if dark { theme::DARK } else { theme::LIGHT };
        self.text.set_gamma(if dark { DARK_TEXT_GAMMA } else { LIGHT_TEXT_GAMMA });
        self.dirty = true;
    }

    /// The window's size in physical pixels, and how many of them make one logical unit.
    pub fn set_viewport(&mut self, width: u32, height: u32, scale: f32) {
        self.scale = scale.max(0.5);
        self.width = width as f32 / self.scale;
        self.height = height as f32 / self.scale;
        self.dirty = true;
    }

    fn shown_feed_height(&self) -> f32 {
        dashboard_view::clamp_height(self.feed_height, self.height)
    }

    pub fn needs_redraw(&self) -> bool {
        self.dirty
    }

    pub fn is_settings_open(&self) -> bool {
        self.settings.is_some()
    }

    pub fn update_status(&self) -> &UpdateStatus {
        &self.update
    }

    pub fn set_update_status(&mut self, status: UpdateStatus) {
        self.update = status;
        self.dirty = true;
    }

    pub fn apply_poll(&mut self, generation: u64, index: usize, result: PollResult, now: f64) -> Vec<Notification> {
        self.dirty = true;
        self.dashboard.apply(generation, index, result, now)
    }

    pub fn set_test_result(&mut self, form_id: &str, text: String) {
        if let Some(page) = &mut self.settings
            && let Some(form) = page.form.servers.iter_mut().find(|form| form.id() == form_id) {
                form.test_result = Some(text);
                self.dirty = true;
            }
    }

    /// The middle of something clickable, in physical pixels, named as its target prints.
    /// This is how tests press buttons by name rather than by coordinates.
    #[doc(hidden)]
    pub fn locate(&self, name: &str) -> Option<(f32, f32)> {
        let hit = self.hits.iter().find(|hit| format!("{:?}", hit.target) == name)?;
        Some(((hit.x + hit.width / 2.0) * self.scale, (hit.y + hit.height / 2.0) * self.scale))
    }

    pub fn cursor(&self) -> Cursor {
        match &self.hot {
            Some(Target::Field(_)) => Cursor::Text,
            Some(Target::FeedDivider) => Cursor::Resize,
            None | Some(Target::Note | Target::Tile { .. } | Target::Feed) => Cursor::Normal,
            Some(_) => Cursor::Hand,
        }
    }

    pub fn draw(&mut self, pixmap: &mut Pixmap) {
        self.hits.clear();
        let mut view = View {
            c: Canvas::new(pixmap, &mut self.text, self.scale),
            theme: self.theme,
            hits: &mut self.hits,
            hot: self.hot.clone(),
            pressed: self.pressed.clone(),
            mouse: self.mouse,
            visible: (0.0, self.height),
            measuring: false,
        };

        let feed_height = dashboard_view::clamp_height(self.feed_height, self.height);
        view.c.clear(self.theme.page);
        match &mut self.settings {
            Some(page) => settings_view::draw(&mut view, page, &self.update, self.width, self.height),
            None => dashboard_view::draw(&mut view, &self.dashboard, self.width, self.height, &mut self.scroll, &mut self.feed_offsets, feed_height),
        }

        if let Some(tip) = &self.tip {
            view.tooltip(tip, self.width, self.height);
        }

        self.dirty = false;
    }

    pub fn handle(&mut self, input: Input) -> Vec<Command> {
        match input {
            Input::MouseMove { x, y } => self.mouse_moved(x / self.scale, y / self.scale),
            Input::MouseLeft => {
                // The cursor left the window mid-press or mid-drag. Nothing else will end
                // those, and a drag left running tracks a pointer that's no longer here.
                self.mouse = None;
                self.pressed = None;
                self.drag_from = None;
                self.feed_drag_from = None;
                self.selecting = None;
                self.set_hot(None, None);
                Vec::new()
            }
            Input::MouseDown => self.mouse_down(),
            Input::MouseUp => self.mouse_up(),
            Input::Wheel { lines } => self.wheel(lines),
            Input::Key { key, shift } => self.key(key, shift),
            Input::Text(typed) => {
                self.edit(|field| field.insert(&typed));
                Vec::new()
            }
            Input::Paste(pasted) => {
                self.edit(|field| field.insert(&pasted));
                Vec::new()
            }
        }
    }

    fn set_hot(&mut self, hot: Option<Target>, tip: Option<String>) {
        if hot != self.hot || tip != self.tip {
            self.hot = hot;
            self.tip = tip;
            self.dirty = true;
        }
    }

    fn update_hot(&mut self) {
        let found = self.mouse.and_then(|(x, y)| self.hits.iter().rev().find(|hit| hit.contains(x, y)));
        let (hot, tip) = found.map_or((None, None), |hit| (Some(hit.target.clone()), hit.tip.clone()));
        self.set_hot(hot, tip);
    }

    fn mouse_moved(&mut self, x: f32, y: f32) -> Vec<Command> {
        self.mouse = Some((x, y));

        if let Some(from) = self.feed_drag_from {
            // From the height on screen, so a drag never has to win back a clamp it can't see.
            self.feed_height = dashboard_view::clamp_height(self.shown_feed_height() - (y - from), self.height);
            self.feed_drag_from = Some(y);
            self.dirty = true;
            return Vec::new();
        }

        if let Some(from) = self.drag_from {
            match &mut self.settings {
                Some(page) => page.scroll.drag(y - from),
                None => self.scroll.drag(y - from),
            }
            self.drag_from = Some(y);
            self.dirty = true;
            return Vec::new();
        }

        if let Some(id) = self.selecting {
            self.place_caret(id, true);
            return Vec::new();
        }

        self.update_hot();

        // The readout follows the mouse along a trend.
        if matches!(self.hot, Some(Target::Tile { .. })) {
            self.dirty = true;
        }

        Vec::new()
    }

    fn mouse_down(&mut self) -> Vec<Command> {
        self.update_hot();
        self.pressed = self.hot.clone();
        self.dirty = true;

        match self.hot {
            Some(Target::ScrollThumb) => self.drag_from = self.mouse.map(|mouse| mouse.1),
            Some(Target::FeedDivider) => self.feed_drag_from = self.mouse.map(|mouse| mouse.1),
            Some(Target::Field(id)) => {
                if let Some(page) = &mut self.settings {
                    page.focused = Some(id);
                }
                self.place_caret(id, false);
                self.selecting = Some(id);
            }
            _ => {
                if let Some(page) = &mut self.settings {
                    page.focused = None;
                }
            }
        }

        Vec::new()
    }

    fn mouse_up(&mut self) -> Vec<Command> {
        self.drag_from = None;
        self.feed_drag_from = None;
        self.selecting = None;
        self.update_hot();
        let pressed = self.pressed.take();
        self.dirty = true;

        match pressed {
            Some(target) if Some(&target) == self.hot.as_ref() => self.activate(target),
            _ => Vec::new(),
        }
    }

    fn wheel(&mut self, lines: f32) -> Vec<Command> {
        // A tilt wheel or a sideways swipe arrives with no up-or-down movement at all.
        if lines == 0.0 || !lines.is_finite() {
            return Vec::new();
        }

        let amount = -lines * WHEEL_LINE;
        match (&mut self.settings, &self.hot) {
            (Some(page), _) => page.scroll.by(amount),
            (None, Some(Target::Feed)) => {
                // The feed moves a row at a time, so a touchpad's stream of small movements
                // adds up to rows instead of each one jumping a row. Turning back starts afresh.
                if self.feed_wheel.signum() != lines.signum() {
                    self.feed_wheel = 0.0;
                }
                self.feed_wheel += lines;
                let rows = self.feed_wheel.trunc();
                self.feed_wheel -= rows;
                self.feed_offsets.feed = step(self.feed_offsets.feed, rows);
            }
            (None, _) => self.scroll.by(amount),
        }

        self.dirty = true;
        Vec::new()
    }

    fn activate(&mut self, target: Target) -> Vec<Command> {
        self.dirty = true;
        match target {
            Target::OpenSettings => {
                // A finished check's answer is old news by now, but a check or an install still
                // running isn't. Forgetting one would offer its buttons again, and a second
                // install would race the first over the same files.
                if !matches!(self.update, UpdateStatus::Checking | UpdateStatus::Installing) {
                    self.update = UpdateStatus::Idle;
                }
                self.settings = Some(SettingsPage::new(&self.dashboard.config));
                self.hits.clear();
                self.set_hot(None, None);
                Vec::new()
            }
            Target::Cancel => {
                self.close_settings();
                Vec::new()
            }
            Target::Save => self.save(),
            Target::AddServer => {
                if let Some(page) = &mut self.settings {
                    page.add_server();
                }
                Vec::new()
            }
            Target::Remove(index) => {
                if let Some(page) = &mut self.settings {
                    page.remove_server(index);
                }
                Vec::new()
            }
            Target::Test(index) => self.test_connection(index),
            Target::ToggleNotifications => {
                if let Some(page) = &mut self.settings {
                    page.form.desktop_notifications = !page.form.desktop_notifications;
                }
                Vec::new()
            }
            Target::SendTestNotification => vec![Command::SendTestNotification],

            // The only way anything reaches GitHub: a person pressing this button.
            Target::CheckForUpdates => {
                self.update = UpdateStatus::Checking;
                vec![Command::CheckForUpdates]
            }
            Target::InstallUpdate => match &self.update {
                UpdateStatus::Available { tag, .. } => {
                    let tag = tag.clone();
                    self.update = UpdateStatus::Installing;
                    vec![Command::InstallUpdate { tag }]
                }
                _ => Vec::new(),
            },
            Target::OpenReleasePage => match &self.update {
                UpdateStatus::Available { link, .. } | UpdateStatus::InstallFailed { link, .. } => vec![Command::OpenLink(link.clone())],
                _ => Vec::new(),
            },
            Target::DismissUpdate => {
                self.update = UpdateStatus::Idle;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn close_settings(&mut self) {
        self.settings = None;
        self.hits.clear();
        self.set_hot(None, None);
        // Escape and Enter get here with nothing hovered, so set_hot alone may not redraw.
        self.dirty = true;
    }

    fn save(&mut self) -> Vec<Command> {
        let Some(page) = &mut self.settings else {
            return Vec::new();
        };

        // Either the page closes or its errors appear, and both need drawing.
        self.dirty = true;
        page.sync_form();
        match page.form.try_build_config() {
            Some(config) => {
                self.close_settings();
                vec![Command::SaveSettings(config)]
            }
            None => Vec::new(),
        }
    }

    fn test_connection(&mut self, index: usize) -> Vec<Command> {
        let Some(page) = &mut self.settings else {
            return Vec::new();
        };

        page.sync_form();
        let Some(form) = page.form.servers.get_mut(index) else {
            return Vec::new();
        };

        let server = form.to_config();
        if server.addresses.is_empty() {
            form.test_result = Some("Enter an address first.".to_string());
            return Vec::new();
        }

        form.test_result = Some("Testing…".to_string());
        vec![Command::TestConnection { form_id: form.id().to_string(), server }]
    }

    fn edit(&mut self, change: impl FnOnce(&mut field::Field)) {
        if let Some(page) = &mut self.settings
            && let Some(field) = page.focused.and_then(|id| page.field_mut(id)) {
                change(field);
                self.dirty = true;
            }
    }

    fn key(&mut self, key: Key, shift: bool) -> Vec<Command> {
        if self.settings.is_none() {
            return Vec::new();
        }

        let mut commands = Vec::new();
        match key {
            Key::Left => self.edit(|field| field.left(shift)),
            Key::Right => self.edit(|field| field.right(shift)),
            Key::Home => self.edit(|field| field.home(shift)),
            Key::End => self.edit(|field| field.end(shift)),
            Key::Backspace => self.edit(field::Field::backspace),
            Key::Delete => self.edit(field::Field::delete),
            Key::Tab => self.move_focus(shift),
            Key::Enter => commands = self.save(),
            Key::Escape => self.close_settings(),
            Key::Control('a') => self.edit(field::Field::select_all),
            Key::Control('c') => {
                if let Some(copied) = self.focused_field().and_then(|field| field.copy()) {
                    commands.push(Command::CopyText(copied));
                }
            }
            Key::Control('x') => {
                let mut cut = None;
                self.edit(|field| cut = field.cut());
                commands.extend(cut.map(Command::CopyText));
            }
            Key::Control('v') => commands.push(Command::RequestPaste),
            Key::Control(_) => {}
        }

        commands
    }

    fn focused_field(&mut self) -> Option<&mut field::Field> {
        let page = self.settings.as_mut()?;
        page.focused.and_then(|id| page.field_mut(id))
    }

    fn move_focus(&mut self, backward: bool) {
        if let Some(page) = &mut self.settings {
            if page.order.is_empty() {
                return;
            }

            let count = page.order.len();
            let current = page.focused.and_then(|id| page.order.iter().position(|other| *other == id));
            let next = match (current, backward) {
                (Some(index), false) => (index + 1) % count,
                (Some(index), true) => (index + count - 1) % count,
                (None, false) => 0,
                (None, true) => count - 1,
            };

            let id = page.order[next];
            page.focused = Some(id);
            if let Some(field) = page.field_mut(id) {
                field.select_all();
            }
            self.dirty = true;
        }
    }

    fn place_caret(&mut self, id: FieldId, extend: bool) {
        let Some((mouse_x, _)) = self.mouse else {
            return;
        };

        let Some(hit) = self.hits.iter().find(|hit| hit.target == Target::Field(id)) else {
            return;
        };
        let left = hit.x + view::FIELD_PADDING;

        let Some(page) = &mut self.settings else {
            return;
        };
        let Some(field) = page.field_mut(id) else {
            return;
        };

        let scale = self.scale;
        let offsets = self.text.offsets(Face::Regular, &field.display(), BODY * scale);
        let wanted = (mouse_x - left + field.view) * scale;
        let index = offsets
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - wanted).abs().total_cmp(&(b.1 - wanted).abs()))
            .map_or(0, |(index, _)| index);

        field.move_to(index, extend);
        self.dirty = true;
    }
}

/// Moves the feed by whole rows: up toward the newest for a positive count, down otherwise.
fn step(offset: usize, rows: f32) -> usize {
    let count = rows.abs() as usize;
    if rows > 0.0 { offset.saturating_sub(count) } else { offset.saturating_add(count) }
}
