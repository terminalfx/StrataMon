//! Text drawn straight into the pixel buffer with LCD subpixel antialiasing, the technique behind
//! ClearType. Fonts are the system's own, read at runtime, so none are bundled. Glyphs are parsed
//! and rasterized only when first used, which keeps memory low.

use ab_glyph::{Font as _, FontVec, GlyphId, PxScale, ScaleFont as _, point};
use std::collections::HashMap;

/// FreeType's default LCD filter. It spreads each edge across neighboring subpixels, which is
/// what removes the color fringes that unfiltered LCD text has.
const LCD_FILTER: [f32; 5] = [8.0 / 256.0, 77.0 / 256.0, 86.0 / 256.0, 77.0 / 256.0, 8.0 / 256.0];

/// A label from a server could contain anything, so the cache can't be allowed to grow forever.
const MAX_CACHED_GLYPHS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    Regular,
    Semibold,
}

struct Glyph {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    coverage: Vec<u8>,
}

#[derive(PartialEq, Eq, Hash)]
struct GlyphKey {
    face: Face,
    glyph: GlyphId,
    em_bits: u32,
    third: u8,
}

pub struct Text {
    regular: FontVec,
    semibold: FontVec,
    cache: HashMap<GlyphKey, Glyph>,
    gamma: [u8; 256],
}

impl Text {
    pub fn new(regular: Vec<u8>, semibold: Vec<u8>) -> Result<Self, String> {
        let parse = |data| FontVec::try_from_vec(data).map_err(|_| "A system font couldn't be read.".to_string());
        let mut text = Self { regular: parse(regular)?, semibold: parse(semibold)?, cache: HashMap::new(), gamma: [0; 256] };
        text.set_gamma(0.85);
        Ok(text)
    }

    /// Finds usable fonts on this system.
    pub fn from_system() -> Result<Self, String> {
        let (regular, semibold) = find_fonts().ok_or("StrataMon couldn't find a font to draw with.")?;
        Self::new(regular, semibold)
    }

    /// Below 1 thickens text. Dark text on a light background needs more of this than light on dark.
    pub fn set_gamma(&mut self, gamma: f32) {
        // Cached glyphs were made with the old curve.
        self.cache.clear();
        for (value, entry) in self.gamma.iter_mut().enumerate() {
            *entry = ((value as f32 / 255.0).powf(gamma) * 255.0).round() as u8;
        }
    }

    fn font(&self, face: Face) -> &FontVec {
        match face {
            Face::Regular => &self.regular,
            Face::Semibold => &self.semibold,
        }
    }

    // ab_glyph's scale is the line height, while sizes everywhere else mean the em size.
    fn em(&self, face: Face, size: f32) -> f32 {
        let font = self.font(face);
        let em = font.units_per_em().filter(|em| *em > 0.0).unwrap_or(1000.0);
        size * font.height_unscaled() / em
    }

    pub fn ascent(&self, face: Face, size: f32) -> f32 {
        self.font(face).as_scaled(PxScale::from(self.em(face, size))).ascent()
    }

    pub fn line_height(&self, face: Face, size: f32) -> f32 {
        self.font(face).as_scaled(PxScale::from(self.em(face, size))).height().ceil()
    }

    pub fn width(&self, face: Face, text: &str, size: f32) -> f32 {
        let font = self.font(face);
        let scaled = font.as_scaled(PxScale::from(self.em(face, size)));
        let mut width = 0.0;
        let mut last = None;
        for c in text.chars() {
            let id = font.glyph_id(c);
            if let Some(last) = last {
                width += scaled.kern(last, id);
            }
            width += scaled.h_advance(id);
            last = Some(id);
        }
        width
    }

    /// The caret position after each character, for clicking and selecting in text fields.
    pub fn offsets(&self, face: Face, text: &str, size: f32) -> Vec<f32> {
        // Walk the text once, carrying the running width. Measuring each prefix separately
        // would be quadratic in the length of the text, and fields get typed into per frame.
        let font = self.font(face);
        let scaled = font.as_scaled(PxScale::from(self.em(face, size)));
        let mut offsets = Vec::with_capacity(text.chars().count() + 1);
        offsets.push(0.0);
        let mut width = 0.0;
        let mut last = None;
        for c in text.chars() {
            let id = font.glyph_id(c);
            if let Some(last) = last {
                width += scaled.kern(last, id);
            }
            width += scaled.h_advance(id);
            last = Some(id);
            offsets.push(width);
        }
        offsets
    }

    /// Draws into an RGBA buffer, which must be opaque where the text lands. Returns the pen's end.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, data: &mut [u8], width: u32, height: u32, face: Face, x: f32, baseline: f32, text: &str, size: f32, color: [u8; 3]) -> f32 {
        if self.cache.len() > MAX_CACHED_GLYPHS {
            self.cache.clear();
        }

        let em = self.em(face, size);
        let (width, height) = (width as i32, height as i32);
        let baseline = baseline.round() as i32;
        let mut pen = x;
        let mut last = None;

        for c in text.chars() {
            let font = match face {
                Face::Regular => &self.regular,
                Face::Semibold => &self.semibold,
            };
            let scaled = font.as_scaled(PxScale::from(em));
            let id = font.glyph_id(c);
            if let Some(last) = last {
                pen += scaled.kern(last, id);
            }

            let advance = scaled.h_advance(id);

            // Thirds of a pixel, because each pixel has three subpixels to land on.
            let thirds = (pen * 3.0).round() as i32;
            let whole = thirds.div_euclid(3);
            let third = thirds.rem_euclid(3) as u8;
            let key = GlyphKey { face, glyph: id, em_bits: em.to_bits(), third };
            let glyph = self.cache.entry(key).or_insert_with(|| rasterize(font, id, em, third, &self.gamma));

            for row in 0..glyph.height {
                let y = baseline + glyph.top + row as i32;
                if y < 0 || y >= height {
                    continue;
                }

                for column in 0..glyph.width {
                    let subpixel = glyph.left + column as i32;
                    let px = whole + subpixel.div_euclid(3);
                    if px < 0 || px >= width {
                        continue;
                    }

                    let coverage = u32::from(glyph.coverage[row * glyph.width + column]);
                    if coverage > 0 {
                        let channel = subpixel.rem_euclid(3) as usize;
                        let target = &mut data[(y * width + px) as usize * 4 + channel];
                        *target = ((u32::from(*target) * (255 - coverage) + u32::from(color[channel]) * coverage) / 255) as u8;
                    }
                }
            }

            pen += advance;
            last = Some(id);
        }

        pen
    }
}

fn rasterize(font: &FontVec, id: GlyphId, em: f32, third: u8, gamma: &[u8; 256]) -> Glyph {
    // Three times as wide, so each column of the outline lands on one subpixel.
    let scaled = id.with_scale_and_position(PxScale { x: em * 3.0, y: em }, point(f32::from(third), 0.0));
    let Some(outline) = font.outline_glyph(scaled) else {
        return Glyph { left: 0, top: 0, width: 0, height: 0, coverage: Vec::new() };
    };

    let bounds = outline.px_bounds();
    let (source_width, height) = (bounds.width() as usize, bounds.height() as usize);
    let mut source = vec![0f32; source_width * height];
    outline.draw(|x, y, coverage| {
        if (x as usize) < source_width && (y as usize) < height {
            source[y as usize * source_width + x as usize] = coverage;
        }
    });

    // The filter reaches two subpixels past each edge.
    let width = source_width + 4;
    let mut coverage = vec![0u8; width * height];
    for row in 0..height {
        for column in 0..width {
            let mut value = 0.0;
            for (offset, weight) in LCD_FILTER.iter().enumerate() {
                let from = column as isize + offset as isize - 4;
                if from >= 0 && (from as usize) < source_width {
                    value += source[row * source_width + from as usize] * weight;
                }
            }
            coverage[row * width + column] = gamma[(value.clamp(0.0, 1.0) * 255.0).round() as usize];
        }
    }

    Glyph { left: bounds.min.x as i32 - 2, top: bounds.min.y as i32, width, height, coverage }
}

fn find_fonts() -> Option<(Vec<u8>, Vec<u8>)> {
    let read = |paths: &[String]| paths.iter().find_map(|path| std::fs::read(path).ok());

    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        let font = |name: &str| format!(r"{root}\Fonts\{name}");
        return Some((read(&[font("segoeui.ttf"), font("arial.ttf")])?, read(&[font("seguisb.ttf"), font("segoeuib.ttf"), font("arialbd.ttf")])?));
    }

    let places = [
        ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf"),
        ("/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-SemiBold.ttf"),
        ("/usr/share/fonts/google-noto/NotoSans-Regular.ttf", "/usr/share/fonts/google-noto/NotoSans-SemiBold.ttf"),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        ("/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"),
        ("/usr/share/fonts/dejavu/DejaVuSans.ttf", "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf"),
        ("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf"),
        ("/usr/share/fonts/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/liberation/LiberationSans-Bold.ttf"),
    ];

    places.iter().find_map(|(regular, bold)| {
        let regular = std::fs::read(regular).ok()?;
        let bold = std::fs::read(bold).unwrap_or_else(|_| regular.clone());
        Some((regular, bold))
    })
}
