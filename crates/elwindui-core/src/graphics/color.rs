use std::fmt;

/// sRGB color with straight (non-premultiplied) alpha, modeled on WinUI3's
/// `Windows.UI.Color { A, R, G, B }`. A future linear/wide-gamut color space is left as a
/// `ColorSpace` extension point — not implemented here (see painter design doc §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseColorError {
    input_len: usize,
}

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid hex color: expected 6 or 8 hex digits (optionally prefixed with '#'), got {} characters",
            self.input_len
        )
    }
}

impl std::error::Error for ParseColorError {}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 0xff }
    }

    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(0xff, 0xff, 0xff);

    pub const fn transparent() -> Self {
        Self::TRANSPARENT
    }
    pub const fn black() -> Self {
        Self::BLACK
    }
    pub const fn white() -> Self {
        Self::WHITE
    }

    pub fn from_rgba_f32(r: f32, g: f32, b: f32, a: f32) -> Self {
        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self {
            r: to_u8(r),
            g: to_u8(g),
            b: to_u8(b),
            a: to_u8(a),
        }
    }

    pub fn to_rgba_f32(self) -> (f32, f32, f32, f32) {
        (
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        )
    }

    /// Parses `"#rrggbb"`/`"rrggbb"` or `"#rrggbbaa"`/`"rrggbbaa"` (alpha defaults to opaque when
    /// omitted). This is the primary, panic-free hex API — prefer it over the deprecated
    /// [`Color::hex`].
    pub fn parse_hex(s: &str) -> Result<Self, ParseColorError> {
        let s = s.trim_start_matches('#');
        let err = || ParseColorError { input_len: s.len() };
        let byte = |slice: &str| u8::from_str_radix(slice, 16).map_err(|_| err());
        match s.len() {
            6 => Ok(Self {
                r: byte(&s[0..2])?,
                g: byte(&s[2..4])?,
                b: byte(&s[4..6])?,
                a: 0xff,
            }),
            8 => Ok(Self {
                r: byte(&s[0..2])?,
                g: byte(&s[2..4])?,
                b: byte(&s[4..6])?,
                a: byte(&s[6..8])?,
            }),
            _ => Err(err()),
        }
    }

    /// Parses `"#rrggbb"` or `"#rrggbbaa"` (alpha defaults to opaque). Panics on malformed input —
    /// prefer [`Color::parse_hex`], which reports malformed input as a `Result` instead.
    #[deprecated(note = "use Color::parse_hex, which returns a Result instead of panicking")]
    pub fn hex(s: &str) -> Self {
        Self::parse_hex(s).expect("invalid hex color")
    }

    /// Prefer constructing a [`crate::graphics::Brush`] directly rather than round-tripping through
    /// a hex string for backend consumption.
    #[deprecated(
        note = "hex strings are no longer the backend interchange format; construct a Brush/Color directly"
    )]
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_without_alpha_defaults_to_opaque() {
        assert_eq!(Color::parse_hex("#eeeeee").unwrap().a, 0xff);
    }

    #[test]
    fn hex_with_alpha_is_parsed() {
        let c = Color::parse_hex("#11223344").unwrap();
        assert_eq!(c, Color::rgba(0x11, 0x22, 0x33, 0x44));
    }

    #[test]
    fn hex_without_hash_prefix_is_accepted() {
        assert_eq!(
            Color::parse_hex("112233").unwrap(),
            Color::rgb(0x11, 0x22, 0x33)
        );
    }

    #[test]
    fn malformed_hex_is_an_error_not_a_panic() {
        assert!(Color::parse_hex("#zz0000").is_err());
        assert!(Color::parse_hex("#123").is_err());
    }

    #[test]
    fn rgba_f32_roundtrips_within_rounding_error() {
        let c = Color::rgb(10, 200, 255);
        let (r, g, b, a) = c.to_rgba_f32();
        assert_eq!(Color::from_rgba_f32(r, g, b, a), c);
    }
}
