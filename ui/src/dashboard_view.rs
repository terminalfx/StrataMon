//! The main window: a card for each server, and the events feed along the bottom. The cards follow
//! Strata's own Monitor, with the same tiles in the same order. The feed follows the previous
//! version's: newest first, scrollable, and its top edge can be dragged to resize it.

use crate::Scroll;
use crate::canvas::Icon;
use crate::text::Face;
use crate::theme;
use crate::view::{BODY, CONTROL_HEIGHT, SMALL, Target, View};
use stratamon_core::card::{AlertRow, Bar, MetricLevel, RequestLine, ServerCard, Stat, Tile};
use stratamon_core::dashboard::Dashboard;
use stratamon_core::state::ServerStatus;

const MARGIN: f32 = 20.0;
const TOP: f32 = 16.0;
const CARD_WIDTH: f32 = 344.0;
const CARD_GAP: f32 = 16.0;
const CARD_PADDING_X: f32 = 16.0;
const CARD_PADDING_Y: f32 = 14.0;
const CARD_RADIUS: f32 = 8.0;
const INNER: f32 = CARD_WIDTH - CARD_PADDING_X * 2.0;
const TILE_GAP_X: f32 = 14.0;
const TILE_GAP_Y: f32 = 6.0;
const TILE_WIDTH: f32 = (INNER - TILE_GAP_X) / 2.0;
const TREND_HEIGHT: f32 = 26.0;
const ALERT_KIND_WIDTH: f32 = 72.0;

/// The feed's height stays inside these, and leaves this much of the window for the cards.
pub const FEED_MIN_HEIGHT: f32 = 88.0;
pub const FEED_MAX_HEIGHT: f32 = 460.0;
pub const FEED_DEFAULT_HEIGHT: f32 = 160.0;
const CARDS_ROOM: f32 = 140.0;

/// The strip along the feed's top edge that can be dragged to resize it.
const DIVIDER_HEIGHT: f32 = 8.0;

const REQUEST_COLUMNS: [f32; 6] = [58.0, 70.0, 46.0, 44.0, 44.0, 50.0];
const REQUEST_ICON_WIDTH: f32 = 16.0;

pub struct Offsets {
    pub feed: usize,
}

/// Keeps the feed's height inside its range and leaves the cards somewhere to live.
pub fn clamp_height(height: f32, window_height: f32) -> f32 {
    let most = FEED_MAX_HEIGHT.min((window_height - CARDS_ROOM).max(FEED_MIN_HEIGHT));
    height.clamp(FEED_MIN_HEIGHT, most)
}

pub fn draw(v: &mut View, dashboard: &Dashboard, width: f32, height: f32, scroll: &mut Scroll, offsets: &mut Offsets, feed_height: f32) {
    let has_servers = !dashboard.servers.is_empty();
    let header_bottom = TOP + CONTROL_HEIGHT + 14.0;
    let notice_height = dashboard.config_notice.as_ref().map_or(0.0, |notice| {
        let lines = v.c.wrap(Face::Regular, notice, BODY, width - MARGIN * 2.0 - 24.0).len() as f32;
        lines * v.c.line_height(Face::Regular, BODY) + 16.0 + 14.0
    });
    let cards_top = header_bottom + notice_height;

    // On a very short window the feed shrinks rather than covering the header.
    let panel_height = if has_servers { feed_height.min((height - cards_top - TOP * 2.0 - 4.0).max(0.0)) } else { 0.0 };
    let bottom = height - if has_servers { panel_height + TOP + 2.0 } else { TOP };

    if has_servers {
        v.visible = (cards_top, bottom);
        draw_cards(v, dashboard, width, cards_top, bottom, scroll);
    } else {
        scroll.set(0.0, 0.0);
        draw_empty_state(v, width, height);
    }

    // Scrolled content can run under these, so they're painted over it.
    let page = v.theme.page;
    v.visible = (0.0, height);
    v.c.fill_rect(0.0, 0.0, width, cards_top, page);
    v.c.fill_rect(0.0, bottom, width, height - bottom, page);
    draw_header(v, dashboard, width, header_bottom);

    if has_servers && panel_height > 0.0 {
        draw_feed_panel(v, dashboard, width, bottom + 2.0, panel_height, offsets);
    }

    if scroll.max > 0.0 {
        draw_scrollbar(v, width - 10.0, cards_top, bottom - cards_top, scroll);
    }
}

fn draw_header(v: &mut View, dashboard: &Dashboard, width: f32, header_bottom: f32) {
    let (primary, secondary, _) = v.ink();
    let end = v.c.text(Face::Semibold, MARGIN, TOP + 1.0, "StrataMon", 20.0, primary);
    v.c.text(Face::Regular, end + 12.0, TOP + 7.0, &dashboard.summary, BODY, secondary);

    let label = "Settings";
    let button_x = width - MARGIN - v.button_width(label);
    v.button(button_x, TOP, label, Target::OpenSettings, false);

    if let Some(notice) = &dashboard.config_notice {
        let inner = width - MARGIN * 2.0 - 24.0;
        let height = v.c.wrap(Face::Regular, notice, BODY, inner).len() as f32 * v.c.line_height(Face::Regular, BODY) + 16.0;
        let (surface, hairline) = (v.theme.surface, v.theme.hairline);
        v.c.fill_rounded(MARGIN, header_bottom, width - MARGIN * 2.0, height, CARD_RADIUS, surface);
        v.c.stroke_rounded(MARGIN, header_bottom, width - MARGIN * 2.0, height, CARD_RADIUS, 1.0, hairline);
        v.c.text_wrapped(Face::Regular, MARGIN + 12.0, header_bottom + 8.0, inner, notice, BODY, primary);
    }
}

fn draw_empty_state(v: &mut View, width: f32, height: f32) {
    let (primary, secondary, _) = v.ink();
    let center = width / 2.0;
    let top = height / 2.0 - 70.0;
    let title = "No servers yet";
    let title_width = v.c.text_width(Face::Semibold, title, 17.0);
    v.c.text(Face::Semibold, center - title_width / 2.0, top, title, 17.0, primary);

    let message = "Add a server by its Strata address. Nothing needs installing on it: StrataMon reads Strata's own monitoring.";
    let room = 380.0_f32.min(width - MARGIN * 2.0);
    let lines = v.c.wrap(Face::Regular, message, BODY, room);
    let line_height = v.c.line_height(Face::Regular, BODY);
    for (index, line) in lines.iter().enumerate() {
        let line_width = v.c.text_width(Face::Regular, line, BODY);
        v.c.text(Face::Regular, center - line_width / 2.0, top + 34.0 + index as f32 * line_height, line, BODY, secondary);
    }

    let label = "Add a server";
    let button_x = center - v.button_width(label) / 2.0;
    v.button(button_x, top + 46.0 + lines.len() as f32 * line_height, label, Target::OpenSettings, false);
}

fn draw_cards(v: &mut View, dashboard: &Dashboard, width: f32, top: f32, bottom: f32, scroll: &mut Scroll) {
    let room = width - MARGIN - 4.0;
    let columns = (((room + CARD_GAP) / (CARD_WIDTH + CARD_GAP)).floor() as usize).max(1);

    v.measure(true);
    let heights: Vec<f32> = dashboard.servers.iter().enumerate().map(|(i, s)| draw_card(v, 0.0, 0.0, &s.card, i)).collect();
    v.measure(false);

    let rows: Vec<f32> = heights.chunks(columns).map(|row| row.iter().copied().fold(0.0, f32::max)).collect();
    let content: f32 = rows.iter().map(|height| height + CARD_GAP).sum();
    scroll.set(content, bottom - top);

    let mut y = top - scroll.offset;
    for (row, cards) in dashboard.servers.chunks(columns).enumerate() {
        // Off-screen rows are skipped, which matters on a small window.
        if y < bottom && y + rows[row] > top {
            for (column, server) in cards.iter().enumerate() {
                let x = MARGIN + column as f32 * (CARD_WIDTH + CARD_GAP);
                let index = row * columns + column;
                let height = heights[index];
                let (surface, hairline) = (v.theme.surface, v.theme.hairline);
                v.c.fill_rounded(x, y, CARD_WIDTH, height, CARD_RADIUS, surface);
                v.c.stroke_rounded(x, y, CARD_WIDTH, height, CARD_RADIUS, 1.0, hairline);
                draw_card(v, x, y, &server.card, index);
            }
        }
        y += rows[row] + CARD_GAP;
    }
}

fn draw_card(v: &mut View, x: f32, top: f32, card: &ServerCard, index: usize) -> f32 {
    let (primary, secondary, muted) = v.ink();
    let left = x + CARD_PADDING_X;
    let right = left + INNER;
    let mut y = top + CARD_PADDING_Y;

    let (icon, color) = match card.status {
        ServerStatus::Online => (Icon::Check, theme::STATUS_GOOD),
        ServerStatus::Offline => (Icon::Cross, theme::STATUS_CRITICAL),
        ServerStatus::Rejected => (Icon::Lock, theme::STATUS_CRITICAL),
        ServerStatus::Connecting => (Icon::Clock, muted),
    };
    let status = card.status_text();
    let status_width = v.c.text_width(Face::Regular, status, BODY);
    v.c.text_right(Face::Regular, right, y + 3.0, status, BODY, secondary);
    v.c.icon(icon, right - status_width - 19.0, y + 5.0, 14.0, color);
    v.c.text_fit(Face::Semibold, left, y, INNER - status_width - 30.0, &card.name, 17.0, primary);
    y += 24.0;

    if let Some(subtitle) = &card.subtitle {
        v.c.text_fit(Face::Regular, left, y, INNER, subtitle, SMALL, muted);
        y += 17.0;
    }

    if let Some(route) = &card.route {
        v.c.text_fit(Face::Regular, left, y, INNER, route, SMALL, muted);
        y += 17.0;
    }

    if let Some(problem) = &card.problem {
        y += 4.0 + v.c.text_wrapped(Face::Regular, left, y + 4.0, INNER, problem, BODY, secondary);
    }

    if card.has_data {
        y += 7.0;
        if card.data_opacity < 1.0 {
            v.c.set_fade(Some((v.theme.surface, card.data_opacity)));
        }
        y = draw_readings(v, left, y, card, index);
        v.c.set_fade(None);
    }

    y + CARD_PADDING_Y - top
}

fn draw_readings(v: &mut View, left: f32, top: f32, card: &ServerCard, index: usize) -> f32 {
    let (primary, _, muted) = v.ink();
    let right = left + INNER;
    let mut y = top;

    v.c.text(Face::Semibold, left, y, &card.activity, 13.0, primary);
    if let Some(progress) = &card.progress_text {
        v.c.text_right(Face::Regular, right, y + 1.0, progress, SMALL, muted);
    }
    y += 21.0;
    v.meter(left, y, INNER, card.progress_percent, 100.0, MetricLevel::Normal);
    y += 15.0;

    for (position, tile) in card.tiles().iter().enumerate() {
        let tile_x = left + (position % 2) as f32 * (TILE_WIDTH + TILE_GAP_X);
        let tile_y = y + (position / 2) as f32 * (tile_height() + TILE_GAP_Y);
        draw_tile(v, tile_x, tile_y, tile, index, position);
    }
    y += 4.0 * tile_height() + 3.0 * TILE_GAP_Y + 7.0;

    if !card.gpu_rows.is_empty() {
        for row in &card.gpu_rows {
            let (_, secondary, _) = v.ink();
            v.c.text(Face::Regular, left, y, &row.name, BODY, secondary);
            let width = v.c.text_width(Face::Semibold, &row.reading, SMALL);
            v.c.text_right(Face::Semibold, right, y + 2.0, &row.reading, SMALL, primary);
            v.level_icon(right - width - 19.0, y + 3.0, row.level);
            y += 23.0;
        }
        y += 3.0;
    }

    v.hairline(left, y, INNER);
    y += 8.0;
    y = draw_bar(v, left, y, &card.context);
    y = draw_bar(v, left, y, &card.ram);
    y = draw_stat(v, left, y, &card.experts);
    y = draw_stat(v, left, y, &card.requests);
    y = draw_stat(v, left, y, &card.served);

    if !card.recent_requests.is_empty() {
        y += 1.0;
        v.hairline(left, y, INNER);
        y += 8.0;
        v.c.text(Face::Semibold, left, y, "Recent requests", 13.0, primary);
        y += 20.0;
        let header = ["Time", "Status", "Prompt", "Output", "Tok/s", "Took"];
        request_row(v, left, y, header, muted, 0.0);
        y += 18.0;
        for request in &card.recent_requests {
            y = draw_request(v, left, y, request);
        }
    }

    if let Some(note) = card.access_note {
        y += 6.0 + v.c.text_wrapped(Face::Regular, left, y + 6.0, INNER, note, SMALL, muted);
    }

    y
}

fn tile_height() -> f32 {
    17.0 + 21.0 + TREND_HEIGHT
}

fn draw_tile(v: &mut View, x: f32, y: f32, tile: &Tile, card: usize, position: usize) {
    let (primary, _, muted) = v.ink();
    let target = Target::Tile { card, tile: position };

    let end = v.c.text(Face::Regular, x, y, tile.label, SMALL, muted);
    if let Some(note) = &tile.note {
        v.c.text_fit_right(Face::Regular, x + TILE_WIDTH, y, x + TILE_WIDTH - end - 6.0, note, SMALL, muted);
    }

    let value_end = v.c.text(Face::Semibold, x, y + 16.0, &tile.value, 15.0, primary);

    let hover = (v.hot.as_ref() == Some(&target)).then(|| v.mouse.map(|mouse| mouse.0)).flatten();
    v.sparkline(x, y + 38.0, TILE_WIDTH, TREND_HEIGHT, tile, hover);
    v.hit(x, y, TILE_WIDTH, tile_height(), target, None);
    v.level_icon(value_end + 5.0, y + 19.0, tile.level);
}

fn draw_bar(v: &mut View, x: f32, y: f32, bar: &Bar) -> f32 {
    let (primary, secondary, _) = v.ink();
    let right = x + INNER;
    v.c.text(Face::Regular, x, y, &bar.label, BODY, secondary);
    let width = v.c.text_width(Face::Semibold, &bar.value, BODY);
    v.c.text_right(Face::Semibold, right, y, &bar.value, BODY, primary);
    v.level_icon(right - width - 19.0, y + 3.0, bar.level);
    v.meter(x, y + 21.0, INNER, bar.percent, bar.limit, bar.level);
    y + 36.0
}

fn draw_stat(v: &mut View, x: f32, y: f32, stat: &Stat) -> f32 {
    let (primary, secondary, _) = v.ink();
    let right = x + INNER;
    let label_end = v.c.text(Face::Regular, x, y, &stat.label, BODY, secondary);
    let room = right - label_end - 10.0;
    let lines = v.c.wrap(Face::Semibold, &stat.value, BODY, room);
    let line_height = v.c.line_height(Face::Semibold, BODY);
    for (index, line) in lines.iter().enumerate() {
        v.c.text_right(Face::Semibold, right, y + index as f32 * line_height, line, BODY, primary);
    }
    y + (lines.len() as f32 * line_height).max(19.0) + 7.0
}

fn request_row(v: &mut View, x: f32, y: f32, cells: [&str; 6], color: theme::Color, status_indent: f32) {
    let mut left = x;
    for (index, (cell, width)) in cells.iter().zip(REQUEST_COLUMNS).enumerate() {
        if index == 0 {
            v.c.text(Face::Regular, left, y, cell, SMALL, color);
        } else if index == 1 {
            v.c.text(Face::Regular, left + status_indent, y, cell, SMALL, color);
        } else {
            v.c.text_right(Face::Regular, left + width, y, cell, SMALL, color);
        }
        left += width;
    }
}

fn draw_request(v: &mut View, x: f32, y: f32, request: &RequestLine) -> f32 {
    let (_, _, muted) = v.ink();
    let status_x = x + REQUEST_COLUMNS[0];
    if request.is_problem {
        v.c.icon(Icon::Warning, status_x, y + 2.0, 13.0, theme::STATUS_CRITICAL);
    }

    let cells = [request.time.as_str(), request.status, &request.prompt, &request.output, &request.speed, &request.duration];
    let indent = if request.is_problem { REQUEST_ICON_WIDTH } else { 0.0 };
    request_row(v, x, y, cells, muted, indent);
    v.hit(x, y, INNER, 17.0, Target::Note, Some(request.reused.clone()));
    y + 17.0
}

fn message_lines(v: &View, row: &AlertRow, column: f32) -> Vec<String> {
    v.c.wrap(Face::Regular, &row.message, BODY, column - 20.0 - ALERT_KIND_WIDTH - 70.0)
}

fn draw_feed_panel(v: &mut View, dashboard: &Dashboard, width: f32, y: f32, height: f32, offsets: &mut Offsets) {
    let (primary, secondary, muted) = v.ink();
    let (surface, hairline) = (v.theme.surface, v.theme.hairline);
    v.c.fill_rounded(MARGIN, y, width - MARGIN * 2.0, height, CARD_RADIUS, surface);
    v.c.stroke_rounded(MARGIN, y, width - MARGIN * 2.0, height, CARD_RADIUS, 1.0, hairline);

    // A grip across the top edge, with a strip of it that can be dragged. Recorded after the cards,
    // so it wins over them right at the boundary.
    let grip_width = 36.0;
    let grip_x = MARGIN + (width - MARGIN * 2.0 - grip_width) / 2.0;
    let grip = if v.hot.as_ref() == Some(&Target::FeedDivider) { v.theme.ink_muted } else { v.theme.control_border };
    v.c.fill_rounded(grip_x, y, grip_width, 3.0, 1.5, grip);
    v.hit(MARGIN, y - DIVIDER_HEIGHT / 2.0, width - MARGIN * 2.0, DIVIDER_HEIGHT, Target::FeedDivider, Some("Drag to make the feed taller or shorter".into()));

    let left = MARGIN + 14.0;
    let right = width - MARGIN - 14.0;
    let column = right - left;
    let top = y + 12.0;
    let bottom = y + height - 10.0;

    let count = dashboard.recent_alerts.len();
    let title = format!("Events ({count})");
    let end = v.c.text(Face::Semibold, left, top, &title, 13.0, primary);
    v.c.text_fit(Face::Regular, end + 10.0, top + 1.0, right - end - 150.0, "· newest first · drag the top edge to resize", SMALL, muted);

    let rows_top = top + 22.0;
    let rows = &dashboard.recent_alerts;
    if rows.is_empty() {
        v.c.text(Face::Regular, left, rows_top, "No events since StrataMon started.", SMALL, muted);
        offsets.feed = 0;
    } else {
        offsets.feed = offsets.feed.min(rows.len() - 1);
        let line_height = v.c.line_height(Face::Regular, BODY);
        let mut row_y = rows_top;
        for row in &rows[offsets.feed..] {
            let lines = message_lines(v, row, column);
            let row_height = lines.len() as f32 * line_height;

            // A partly visible row is left out, so nothing is ever cut in half.
            if row_y + row_height > bottom {
                break;
            }

            let (icon, color) = if row.is_critical {
                (Icon::Cross, theme::STATUS_CRITICAL)
            } else if row.is_over_limit {
                (Icon::Warning, theme::STATUS_CRITICAL)
            } else if row.is_notice {
                (Icon::Warning, theme::STATUS_WARNING)
            } else {
                (Icon::Check, theme::STATUS_GOOD)
            };

            v.c.icon(icon, left, row_y + 2.0, 14.0, color);
            v.c.text(Face::Regular, left + 20.0, row_y, row.kind, BODY, secondary);
            for (index, line) in lines.iter().enumerate() {
                v.c.text(Face::Regular, left + 20.0 + ALERT_KIND_WIDTH, row_y + index as f32 * line_height, line, BODY, primary);
            }
            v.c.text_right(Face::Regular, right, row_y + 1.0, &row.time, SMALL, muted);
            row_y += row_height + 6.0;
        }

        // Newest first, so the rows scrolled past at the top are the newer ones.
        if offsets.feed > 0 {
            v.c.text_right(Face::Regular, right, top + 1.0, &format!("{} newer above", offsets.feed), SMALL, muted);
        }
    }

    v.hit(left, top, column, (bottom - top).max(0.0), Target::Feed, None);
}

pub fn draw_scrollbar(v: &mut View, x: f32, top: f32, height: f32, scroll: &Scroll) {
    let (thumb_top, thumb_height) = scroll.thumb(top, height);
    let color = if v.hot.as_ref() == Some(&Target::ScrollThumb) || v.pressed.as_ref() == Some(&Target::ScrollThumb) {
        v.theme.ink_muted
    } else {
        v.theme.control_border
    };
    v.c.fill_rounded(x, thumb_top, 6.0, thumb_height, 3.0, color);
    v.hit(x - 4.0, thumb_top, 14.0, thumb_height, Target::ScrollThumb, None);
}
