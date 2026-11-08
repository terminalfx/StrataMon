//! The widgets every page draws with. They record where they landed, so the next mouse event can
//! tell what's under it.

use crate::canvas::{Canvas, Icon};
use crate::field::Field;
use crate::text::Face;
use crate::theme::{self, Color, Theme};
use stratamon_core::card::{MetricLevel, Tile};

pub const SMALL: f32 = 12.0;
pub const BODY: f32 = 14.0;
pub const CONTROL_HEIGHT: f32 = 32.0;
const CONTROL_RADIUS: f32 = 4.0;
const METER_HEIGHT: f32 = 8.0;
const METER_RADIUS: f32 = 4.0;
const METER_TRACK_OPACITY: f32 = 0.22;
/// Space between a field's edge and its text. Clicks are mapped to the caret with it too.
pub(crate) const FIELD_PADDING: f32 = 10.0;

// Trend marks, the same as the C# build's.
const TREND_LINE: f32 = 2.0;
const TREND_AREA_OPACITY: f32 = 0.10;
const TREND_DOT: f32 = 4.0;
const TREND_RING: f32 = 2.0;
const TREND_INSET: f32 = TREND_DOT + TREND_RING;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    Name(usize),
    Lan(usize),
    Tailscale(usize),
    Key(usize),
    GpuTemperature,
    RamPercent,
    SustainSeconds,
    QueueWaitSeconds,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    OpenSettings,
    Tile { card: usize, tile: usize },
    /// Somewhere with a tooltip and nothing to click.
    Note,
    /// The events feed's list of rows.
    Feed,
    /// The feed's top edge, which can be dragged to resize it.
    FeedDivider,
    ScrollThumb,
    Field(FieldId),
    Test(usize),
    Remove(usize),
    AddServer,
    ToggleNotifications,
    SendTestNotification,
    CheckForUpdates,
    InstallUpdate,
    OpenReleasePage,
    DismissUpdate,
    Cancel,
    Save,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub target: Target,
    pub tip: Option<String>,
}

impl Hit {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

pub struct View<'a> {
    pub c: Canvas<'a>,
    pub theme: Theme,
    pub hits: &'a mut Vec<Hit>,
    pub hot: Option<Target>,
    pub pressed: Option<Target>,
    pub mouse: Option<(f32, f32)>,

    /// The part of the window a scrolled page is visible in. Hits outside it don't count.
    pub visible: (f32, f32),
    pub measuring: bool,
}

impl View<'_> {
    pub fn hit(&mut self, x: f32, y: f32, width: f32, height: f32, target: Target, tip: Option<String>) {
        if self.measuring {
            return;
        }

        let top = y.max(self.visible.0);
        let bottom = (y + height).min(self.visible.1);
        if bottom > top {
            self.hits.push(Hit { x, y: top, width, height: bottom - top, target, tip });
        }
    }

    pub fn measure(&mut self, measuring: bool) {
        self.measuring = measuring;
        self.c.set_measuring(measuring);
    }

    pub fn button_width(&self, label: &str) -> f32 {
        self.c.text_width(Face::Regular, label, BODY) + 24.0
    }

    /// Draws a button at `x` and returns its width.
    pub fn button(&mut self, x: f32, y: f32, label: &str, target: Target, accent: bool) -> f32 {
        let width = self.button_width(label);
        let is_hot = self.hot.as_ref() == Some(&target);
        let (fill, ink) = if accent {
            (if is_hot { self.theme.accent.mix(self.theme.on_accent, 0.15) } else { self.theme.accent }, self.theme.on_accent)
        } else {
            (if is_hot { self.theme.control_hover } else { self.theme.control }, self.theme.ink_primary)
        };

        self.c.fill_rounded(x, y, width, CONTROL_HEIGHT, CONTROL_RADIUS, fill);
        self.c.text(Face::Regular, x + 12.0, y + 6.0, label, BODY, ink);
        self.hit(x, y, width, CONTROL_HEIGHT, target, None);
        width
    }

    pub fn link(&mut self, x: f32, y: f32, label: &str, target: Target) -> f32 {
        let width = self.c.text_width(Face::Regular, label, BODY);
        let color = self.theme.accent;
        self.c.text(Face::Regular, x, y + 6.0, label, BODY, color);
        if self.hot.as_ref() == Some(&target) {
            self.c.fill_rect(x, y + 24.0, width, 1.0, color);
        }
        self.hit(x, y, width, CONTROL_HEIGHT, target, None);
        width
    }

    pub fn checkbox(&mut self, x: f32, y: f32, label: &str, checked: bool, target: Target) -> f32 {
        let size = 18.0;
        let top = y + (CONTROL_HEIGHT - size) / 2.0;
        if checked {
            self.c.fill_rounded(x, top, size, size, 3.0, self.theme.accent);
            self.c.icon(Icon::Check, x + 2.0, top + 2.0, 14.0, self.theme.on_accent);
        } else {
            self.c.stroke_rounded(x, top, size, size, 3.0, 1.0, self.theme.control_border);
        }

        let end = self.c.text(Face::Regular, x + size + 8.0, y + 6.0, label, BODY, self.theme.ink_secondary);
        self.hit(x, y, end - x, CONTROL_HEIGHT, target, None);
        end - x
    }

    pub fn field(&mut self, (x, y): (f32, f32), width: f32, id: FieldId, field: &mut Field, placeholder: &str, focused: bool) {
        let border = if focused { self.theme.accent } else { self.theme.control_border };
        self.c.fill_rounded(x, y, width, CONTROL_HEIGHT, CONTROL_RADIUS, self.theme.page);
        self.c.stroke_rounded(x, y, width, CONTROL_HEIGHT, CONTROL_RADIUS, 1.0, border);
        self.hit(x, y, width, CONTROL_HEIGHT, Target::Field(id), None);
        if self.measuring {
            return;
        }

        let left = x + FIELD_PADDING;
        let room = (width - FIELD_PADDING * 2.0).max(0.0);
        let top = y + 6.0;
        let shown = field.display();
        if shown.is_empty() {
            self.c.text_fit(Face::Regular, left, top, room, placeholder, BODY, self.theme.ink_muted);
        }

        let offsets = self.c.caret_offsets(Face::Regular, &shown, BODY);
        let caret = offsets[field.caret().min(offsets.len() - 1)];
        let total = offsets[offsets.len() - 1];

        // Keeps the caret in view as the text scrolls past the edge of the box.
        if caret - field.view > room {
            field.view = caret - room;
        } else if caret < field.view {
            field.view = caret;
        }
        field.view = field.view.min((total - room).max(0.0));

        if let Some((start, end)) = field.selection().filter(|_| focused) {
            let from = (offsets[start] - field.view).max(0.0);
            let to = (offsets[end] - field.view).min(room);
            if to > from {
                self.c.fill_rect(left + from, y + 6.0, to - from, 20.0, self.theme.selection);
            }
        }

        let first = offsets.iter().position(|offset| *offset >= field.view).unwrap_or(0);
        let last = offsets.iter().rposition(|offset| *offset - field.view <= room).unwrap_or(0);
        let visible: String = shown.chars().skip(first).take(last.saturating_sub(first)).collect();
        self.c.text(Face::Regular, left + offsets[first] - field.view, top, &visible, BODY, self.theme.ink_primary);

        if focused {
            self.c.fill_rect(left + caret - field.view, y + 6.0, 1.0, 20.0, self.theme.ink_primary);
        }
    }

    pub fn hairline(&mut self, x: f32, y: f32, width: f32) {
        self.c.fill_rect(x, y, width, 1.0, self.theme.hairline);
    }

    pub fn level_icon(&mut self, x: f32, y: f32, level: MetricLevel) -> bool {
        let (color, tip) = match level {
            MetricLevel::Normal => return false,
            MetricLevel::NearLimit => (theme::STATUS_WARNING, "Near its alert limit"),
            MetricLevel::OverLimit => (theme::STATUS_CRITICAL, "Over its alert limit"),
        };

        self.c.icon(Icon::Warning, x, y, 14.0, color);
        self.hit(x, y, 14.0, 14.0, Target::Note, Some(tip.to_string()));
        true
    }

    /// A percentage bar. The fill shows severity, the track is a tint of the same color so state
    /// reads across the whole bar, and a tick marks the alert limit.
    pub fn meter(&mut self, x: f32, y: f32, width: f32, percent: Option<f64>, limit: f64, level: MetricLevel) {
        let Some(percent) = percent else {
            // No reading: an empty neutral track, so it can't be read as a low value.
            self.c.fill_rounded(x, y, width, METER_HEIGHT, METER_RADIUS, self.theme.empty_track);
            return;
        };

        let fill = match level {
            MetricLevel::OverLimit => theme::STATUS_CRITICAL,
            MetricLevel::NearLimit => theme::STATUS_WARNING,
            MetricLevel::Normal => self.theme.accent,
        };

        self.c.fill_rounded_with_opacity(x, y, width, METER_HEIGHT, METER_RADIUS, fill, METER_TRACK_OPACITY);
        let filled = width * (percent.clamp(0.0, 100.0) / 100.0) as f32;
        if filled > 0.0 {
            self.c.fill_rounded(x, y, filled.max(METER_RADIUS), METER_HEIGHT, METER_RADIUS, fill);
        }

        if limit < 100.0 {
            let tick = x + (width * (limit / 100.0) as f32).round();
            self.c.fill_rect(tick, y, 1.0, METER_HEIGHT, self.theme.ink_muted);
        }
    }

    /// A trend from zero to the tile's scale. The x-axis is a fixed time window with now at the
    /// right edge, so a newly started trend grows in from the right rather than stretching to fill.
    pub fn sparkline(&mut self, x: f32, y: f32, width: f32, height: f32, tile: &Tile, hover_x: Option<f32>) {
        if self.measuring {
            return;
        }

        let samples: Vec<Option<f64>> = tile.samples().iter().copied().collect();
        let capacity = tile.history_length;
        if samples.is_empty() || capacity < 2 {
            return;
        }

        let plot_width = width - TREND_INSET - TREND_RING;
        let plot_height = height - TREND_INSET * 2.0;
        let right = x + width - TREND_INSET;
        let baseline = y + height - TREND_INSET;
        let position = |index: usize, value: f64| {
            let steps = (samples.len() - 1 - index) as f32;
            let share = (value / tile.scale).clamp(0.0, 1.0) as f32;
            (right - plot_width * steps / (capacity - 1) as f32, y + TREND_INSET + plot_height * (1.0 - share))
        };

        // Each unbroken stretch is drawn alone, so a line never bridges an outage.
        let mut start = None;
        for index in 0..=samples.len() {
            let present = index < samples.len() && samples[index].is_some();
            match (present, start) {
                (true, None) => start = Some(index),
                (false, Some(first)) => {
                    start = None;
                    let points: Vec<(f32, f32)> =
                        (first..index).filter_map(|i| samples[i].map(|value| position(i, value))).collect();
                    if points.len() > 1 {
                        let mut area = Vec::with_capacity(points.len() + 2);
                        area.push((points[0].0, baseline));
                        area.extend_from_slice(&points);
                        area.push((points[points.len() - 1].0, baseline));
                        self.c.fill_polygon(&area, self.theme.ink_muted, TREND_AREA_OPACITY);
                        self.c.stroke_line(&points, TREND_LINE, self.theme.ink_muted);
                    }
                }
                _ => {}
            }
        }

        if let Some(Some(latest)) = samples.last() {
            let (dot_x, dot_y) = position(samples.len() - 1, *latest);
            self.c.circle(dot_x, dot_y, TREND_DOT + TREND_RING, self.theme.surface);
            self.c.circle(dot_x, dot_y, TREND_DOT, self.theme.accent);
        }

        if let Some(mouse) = hover_x {
            let steps = ((right - mouse) / plot_width * (capacity - 1) as f32).round().max(0.0) as usize;
            let index = samples.len() - 1 - steps.min(samples.len() - 1);
            let line_x = position(index, 0.0).0;
            let age = (samples.len() - 1 - index) as f64 * f64::from(tile.sample_seconds);
            let when = if age < 1.0 { "now".to_string() } else { format!("{} ago", stratamon_core::format::duration(age)) };
            let label = format!("{} · {when}", (tile.format_sample)(samples[index]));

            self.c.fill_rect(line_x, y, 1.0, height, self.theme.ink_muted);
            let box_width = self.c.text_width(Face::Regular, &label, SMALL) + 8.0;
            let left = if line_x - box_width - 4.0 >= x { line_x - box_width - 4.0 } else { line_x + 4.0 };
            self.c.fill_rect(left, y, box_width, 16.0, self.theme.surface);
            self.c.text(Face::Regular, left + 4.0, y, &label, SMALL, self.theme.ink_primary);
        }
    }

    pub fn tooltip(&mut self, text: &str, window_width: f32, window_height: f32) {
        let Some((mouse_x, mouse_y)) = self.mouse else {
            return;
        };

        let width = self.c.text_width(Face::Regular, text, SMALL) + 16.0;
        let height = 24.0;
        let x = (mouse_x + 12.0).min(window_width - width - 4.0).max(4.0);
        let y = if mouse_y + 20.0 + height > window_height { mouse_y - height - 6.0 } else { mouse_y + 20.0 };
        self.c.fill_rounded(x, y, width, height, CONTROL_RADIUS, self.theme.surface);
        self.c.stroke_rounded(x, y, width, height, CONTROL_RADIUS, 1.0, self.theme.control_border);
        self.c.text(Face::Regular, x + 8.0, y + 4.0, text, SMALL, self.theme.ink_primary);
    }

    pub fn ink(&self) -> (Color, Color, Color) {
        (self.theme.ink_primary, self.theme.ink_secondary, self.theme.ink_muted)
    }
}
