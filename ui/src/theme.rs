//! The same palette as the C# build, for both themes. Status colors mean the same in each, and
//! always appear with an icon and a label, never alone.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    /// Part way from this color to `other`, where 0 is all this and 1 is all `other`.
    pub fn mix(self, other: Color, amount: f32) -> Color {
        let channel = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8;
        Color(channel(self.0, other.0), channel(self.1, other.1), channel(self.2, other.2))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub dark: bool,
    pub page: Color,
    pub surface: Color,
    pub ink_primary: Color,
    pub ink_secondary: Color,
    pub ink_muted: Color,
    pub hairline: Color,
    pub empty_track: Color,
    pub accent: Color,
    pub control: Color,
    pub control_hover: Color,
    pub control_border: Color,
    pub on_accent: Color,
    pub selection: Color,
}

pub const STATUS_GOOD: Color = Color(0x0c, 0xa3, 0x0c);
pub const STATUS_WARNING: Color = Color(0xfa, 0xb2, 0x19);
pub const STATUS_CRITICAL: Color = Color(0xd0, 0x3b, 0x3b);

pub const LIGHT: Theme = Theme {
    dark: false,
    page: Color(0xf9, 0xf9, 0xf7),
    surface: Color(0xfc, 0xfc, 0xfb),
    ink_primary: Color(0x0b, 0x0b, 0x0b),
    ink_secondary: Color(0x52, 0x51, 0x4e),
    ink_muted: Color(0x89, 0x87, 0x81),
    hairline: Color(0xe1, 0xe0, 0xd9),
    empty_track: Color(0xe1, 0xe0, 0xd9),
    accent: Color(0x2a, 0x78, 0xd6),
    control: Color(0xee, 0xee, 0xea),
    control_hover: Color(0xe1, 0xe0, 0xd9),
    control_border: Color(0xc9, 0xc8, 0xc0),
    on_accent: Color(0xff, 0xff, 0xff),
    selection: Color(0xb9, 0xd3, 0xf2),
};

pub const DARK: Theme = Theme {
    dark: true,
    page: Color(0x0d, 0x0d, 0x0d),
    surface: Color(0x1a, 0x1a, 0x19),
    ink_primary: Color(0xf0, 0xef, 0xec),
    ink_secondary: Color(0xc3, 0xc2, 0xb7),
    ink_muted: Color(0x89, 0x87, 0x81),
    hairline: Color(0x2c, 0x2c, 0x2a),
    empty_track: Color(0x2c, 0x2c, 0x2a),
    accent: Color(0x39, 0x87, 0xe5),
    control: Color(0x2c, 0x2c, 0x2a),
    control_hover: Color(0x3a, 0x3a, 0x37),
    control_border: Color(0x4a, 0x4a, 0x46),
    on_accent: Color(0xff, 0xff, 0xff),
    selection: Color(0x1f, 0x4e, 0x86),
};
