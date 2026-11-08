//! Drawing in logical units, which it converts to pixels for the display's scale.

use crate::text::{Face, Text};
use crate::theme::Color;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Check,
    Cross,
    Warning,
    Clock,
    Lock,
}

pub struct Canvas<'a> {
    pixmap: &'a mut Pixmap,
    text: &'a mut Text,
    scale: f32,

    /// Faded content is drawn in colors mixed toward this, which fades shapes and text alike.
    fade: Option<(Color, f32)>,

    /// Measures without drawing, for working out how tall something is before placing it.
    measuring: bool,
}

impl<'a> Canvas<'a> {
    pub fn new(pixmap: &'a mut Pixmap, text: &'a mut Text, scale: f32) -> Self {
        Self { pixmap, text, scale, fade: None, measuring: false }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn set_measuring(&mut self, measuring: bool) {
        self.measuring = measuring;
    }

    pub fn set_fade(&mut self, fade: Option<(Color, f32)>) {
        self.fade = fade;
    }

    fn color(&self, color: Color) -> Color {
        match self.fade {
            Some((toward, opacity)) => toward.mix(color, opacity),
            None => color,
        }
    }

    fn paint(&self, color: Color, opacity: f32) -> Paint<'static> {
        let color = self.color(color);
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.0, color.1, color.2, (opacity * 255.0).round() as u8);
        paint.anti_alias = true;
        paint
    }

    /// Lines a value up with the pixel grid, so edges stay crisp at any scale.
    fn snap(&self, value: f32) -> f32 {
        (value * self.scale).round()
    }

    pub fn clear(&mut self, color: Color) {
        self.pixmap.fill(tiny_skia::Color::from_rgba8(color.0, color.1, color.2, 255));
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        if self.measuring {
            return;
        }

        let (left, top) = (self.snap(x), self.snap(y));
        let (right, bottom) = (self.snap(x + width), self.snap(y + height));
        if let Some(rect) = Rect::from_ltrb(left, top, right, bottom) {
            let mut paint = self.paint(color, 1.0);
            paint.anti_alias = false;
            self.pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    pub fn fill_rounded(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color) {
        self.fill_rounded_with_opacity(x, y, width, height, radius, color, 1.0);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_rounded_with_opacity(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color, opacity: f32) {
        if self.measuring {
            return;
        }

        let (left, top) = (self.snap(x), self.snap(y));
        let (right, bottom) = (self.snap(x + width), self.snap(y + height));
        if let Some(path) = rounded(left, top, right - left, bottom - top, radius * self.scale) {
            let paint = self.paint(color, opacity);
            self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rounded(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, thickness: f32, color: Color) {
        if self.measuring {
            return;
        }

        // Half a pixel in, so a one pixel line covers exactly one row of pixels.
        let inset = (thickness * self.scale).max(1.0) / 2.0;
        let (left, top) = (self.snap(x) + inset, self.snap(y) + inset);
        let (right, bottom) = (self.snap(x + width) - inset, self.snap(y + height) - inset);
        if let Some(path) = rounded(left, top, right - left, bottom - top, radius * self.scale) {
            let stroke = Stroke { width: (thickness * self.scale).max(1.0), ..Stroke::default() };
            let paint = self.paint(color, 1.0);
            self.pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    pub fn circle(&mut self, x: f32, y: f32, radius: f32, color: Color) {
        if self.measuring {
            return;
        }

        if let Some(path) = PathBuilder::from_circle(x * self.scale, y * self.scale, radius * self.scale) {
            let paint = self.paint(color, 1.0);
            self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pub fn stroke_line(&mut self, points: &[(f32, f32)], thickness: f32, color: Color) {
        if self.measuring || points.len() < 2 {
            return;
        }

        if let Some(path) = polyline(points, self.scale) {
            let stroke = Stroke { width: thickness * self.scale, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
            let paint = self.paint(color, 1.0);
            self.pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    pub fn fill_polygon(&mut self, points: &[(f32, f32)], color: Color, opacity: f32) {
        if self.measuring || points.len() < 3 {
            return;
        }

        let mut builder = PathBuilder::new();
        builder.move_to(points[0].0 * self.scale, points[0].1 * self.scale);
        for point in &points[1..] {
            builder.line_to(point.0 * self.scale, point.1 * self.scale);
        }
        builder.close();
        if let Some(path) = builder.finish() {
            let paint = self.paint(color, opacity);
            self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pub fn icon(&mut self, icon: Icon, x: f32, y: f32, size: f32, color: Color) {
        if self.measuring {
            return;
        }

        // The shapes are on a 24 unit grid.
        let unit = size / 24.0 * self.scale;
        let (left, top) = (x * self.scale, y * self.scale);
        let at = |px: f32, py: f32| (left + px * unit, top + py * unit);
        let mut builder = PathBuilder::new();
        let polygon = |builder: &mut PathBuilder, points: &[(f32, f32)]| {
            let (first, rest) = (points[0], &points[1..]);
            let start = at(first.0, first.1);
            builder.move_to(start.0, start.1);
            for point in rest {
                let next = at(point.0, point.1);
                builder.line_to(next.0, next.1);
            }
            builder.close();
        };

        match icon {
            Icon::Check => polygon(&mut builder, &[(9.0, 16.2), (4.8, 12.0), (3.4, 13.4), (9.0, 19.0), (21.0, 7.0), (19.6, 5.6)]),
            Icon::Cross => polygon(
                &mut builder,
                &[
                    (19.0, 6.4),
                    (17.6, 5.0),
                    (12.0, 10.6),
                    (6.4, 5.0),
                    (5.0, 6.4),
                    (10.6, 12.0),
                    (5.0, 17.6),
                    (6.4, 19.0),
                    (12.0, 13.4),
                    (17.6, 19.0),
                    (19.0, 17.6),
                    (13.4, 12.0),
                ],
            ),
            Icon::Warning => {
                polygon(&mut builder, &[(1.0, 21.0), (23.0, 21.0), (12.0, 2.0)]);
                polygon(&mut builder, &[(13.0, 18.0), (11.0, 18.0), (11.0, 16.0), (13.0, 16.0)]);
                polygon(&mut builder, &[(13.0, 14.0), (11.0, 14.0), (11.0, 10.0), (13.0, 10.0)]);
            }
            Icon::Clock => {
                let center = at(12.0, 12.0);
                builder.push_circle(center.0, center.1, 10.0 * unit);
                polygon(&mut builder, &[(13.0, 7.0), (11.0, 7.0), (11.0, 13.0), (16.0, 16.0), (17.0, 14.3), (13.0, 12.0)]);
            }
            Icon::Lock => {
                let (a, b) = (at(4.0, 8.0), at(20.0, 22.0));
                if let Some(body) = Rect::from_ltrb(a.0, a.1, b.0, b.1) {
                    builder.push_rect(body);
                }
                let (c, d) = (at(7.0, 3.0), at(17.0, 8.0));
                let top_of_arc = at(12.0, 1.0);
                builder.move_to(c.0, d.1);
                builder.line_to(c.0, c.1 + unit * 3.0);
                builder.quad_to(c.0, top_of_arc.1, top_of_arc.0, top_of_arc.1);
                builder.quad_to(d.0, top_of_arc.1, d.0, c.1 + unit * 3.0);
                builder.line_to(d.0, d.1);
                builder.line_to(d.0 - unit * 2.0, d.1);
                builder.line_to(d.0 - unit * 2.0, c.1 + unit * 3.0);
                builder.quad_to(d.0 - unit * 2.0, top_of_arc.1 + unit * 2.0, top_of_arc.0, top_of_arc.1 + unit * 2.0);
                builder.quad_to(c.0 + unit * 2.0, top_of_arc.1 + unit * 2.0, c.0 + unit * 2.0, c.1 + unit * 3.0);
                builder.line_to(c.0 + unit * 2.0, d.1);
                builder.close();
            }
        }

        if let Some(path) = builder.finish() {
            let paint = self.paint(color, 1.0);
            self.pixmap.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None);
        }
    }

    pub fn text_width(&self, face: Face, text: &str, size: f32) -> f32 {
        self.text.width(face, text, size * self.scale) / self.scale
    }

    pub fn line_height(&self, face: Face, size: f32) -> f32 {
        self.text.line_height(face, size * self.scale) / self.scale
    }

    pub fn caret_offsets(&self, face: Face, text: &str, size: f32) -> Vec<f32> {
        self.text.offsets(face, text, size * self.scale).into_iter().map(|offset| offset / self.scale).collect()
    }

    /// Draws one line with `y` as the top of the line, and returns where it ended.
    pub fn text(&mut self, face: Face, x: f32, y: f32, text: &str, size: f32, color: Color) -> f32 {
        if self.measuring {
            return x + self.text_width(face, text, size);
        }

        let color = self.color(color);
        let (width, height) = (self.pixmap.width(), self.pixmap.height());
        let baseline = (y * self.scale).round() + self.text.ascent(face, size * self.scale).round();
        let end = self.text.draw(self.pixmap.data_mut(), width, height, face, x * self.scale, baseline, text, size * self.scale, [color.0, color.1, color.2]);
        end / self.scale
    }

    /// Draws one line ending at `right`.
    pub fn text_right(&mut self, face: Face, right: f32, y: f32, text: &str, size: f32, color: Color) {
        let width = self.text_width(face, text, size);
        self.text(face, right - width, y, text, size, color);
    }

    /// Draws one line within `width`, cutting it off with an ellipsis if it doesn't fit.
    #[allow(clippy::too_many_arguments)]
    pub fn text_fit(&mut self, face: Face, x: f32, y: f32, width: f32, text: &str, size: f32, color: Color) {
        let fitted = self.fit(face, text, size, width);
        self.text(face, x, y, &fitted, size, color);
    }

    /// Like [`Self::text_fit`], ending at `right`.
    #[allow(clippy::too_many_arguments)]
    pub fn text_fit_right(&mut self, face: Face, right: f32, y: f32, width: f32, text: &str, size: f32, color: Color) {
        let fitted = self.fit(face, text, size, width);
        self.text_right(face, right, y, &fitted, size, color);
    }

    fn fit(&self, face: Face, text: &str, size: f32, width: f32) -> String {
        if self.text_width(face, text, size) <= width {
            return text.to_string();
        }

        let mut kept: String = text.to_string();
        while !kept.is_empty() {
            kept.pop();
            let candidate = format!("{}…", kept.trim_end());
            if self.text_width(face, &candidate, size) <= width {
                return candidate;
            }
        }

        "…".to_string()
    }

    /// Splits text into lines that fit `width`, breaking at spaces where it can.
    pub fn wrap(&self, face: Face, text: &str, size: f32, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.text_width(face, &candidate, size) <= width {
                    line = candidate;
                    continue;
                }

                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }

                // A single word wider than the line, such as a long address, has to break anywhere.
                let mut piece = String::new();
                for c in word.chars() {
                    piece.push(c);
                    if self.text_width(face, &piece, size) > width && piece.chars().count() > 1 {
                        let overflow = piece.pop();
                        lines.push(std::mem::take(&mut piece));
                        piece.extend(overflow);
                    }
                }
                line = piece;
            }
            lines.push(line);
        }
        lines
    }

    /// Draws wrapped lines and returns the height they took.
    #[allow(clippy::too_many_arguments)]
    pub fn text_wrapped(&mut self, face: Face, x: f32, y: f32, width: f32, text: &str, size: f32, color: Color) -> f32 {
        let line_height = self.line_height(face, size);
        let lines = self.wrap(face, text, size, width);
        for (index, line) in lines.iter().enumerate() {
            self.text(face, x, y + index as f32 * line_height, line, size, color);
        }
        lines.len() as f32 * line_height
    }
}

fn rounded(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<Path> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    // The control point for a quarter circle drawn with one cubic curve: 4(√2 − 1)/3.
    const KAPPA: f32 = 0.552_284_7;
    let r = radius.max(0.0).min(width / 2.0).min(height / 2.0);
    let c = r * KAPPA;
    let (right, bottom) = (x + width, y + height);
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(right - r, y);
    b.cubic_to(right - c, y, right, y + c, right, y + r);
    b.line_to(right, bottom - r);
    b.cubic_to(right, bottom - c, right - c, bottom, right - r, bottom);
    b.line_to(x + r, bottom);
    b.cubic_to(x + c, bottom, x, bottom - c, x, bottom - r);
    b.line_to(x, y + r);
    b.cubic_to(x, y + c, x + c, y, x + r, y);
    b.close();
    b.finish()
}

fn polyline(points: &[(f32, f32)], scale: f32) -> Option<Path> {
    let mut b = PathBuilder::new();
    b.move_to(points[0].0 * scale, points[0].1 * scale);
    for point in &points[1..] {
        b.line_to(point.0 * scale, point.1 * scale);
    }
    b.finish()
}
