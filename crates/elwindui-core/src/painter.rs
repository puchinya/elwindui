use crate::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Modeled on WinUI3's `Windows.UI.Color { A, R, G, B }`. See docs/elwindui_spec.md 付録G.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// Parses `"#rrggbb"` or `"#rrggbbaa"` (alpha defaults to opaque).
    pub fn hex(s: &str) -> Self {
        let s = s.trim_start_matches('#');
        let r = u8::from_str_radix(&s[0..2], 16).expect("invalid hex color");
        let g = u8::from_str_radix(&s[2..4], 16).expect("invalid hex color");
        let b = u8::from_str_radix(&s[4..6], 16).expect("invalid hex color");
        let a = if s.len() >= 8 {
            u8::from_str_radix(&s[6..8], 16).expect("invalid hex color")
        } else {
            0xff
        };
        Color { r, g, b, a }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Path;

#[derive(Debug, Clone, Copy, Default)]
pub struct PaintStyle;

#[derive(Debug, Clone, Copy, Default)]
pub struct Font;

#[derive(Debug, Clone, Copy, Default)]
pub struct Image;

/// Minimal backend-agnostic drawing surface for `Canvas`/`on_paint`. See docs/elwindui_spec.md 付録G.2.
///
/// `target::backend()` is a compile-time constant (付録D), so a given build only ever links one
/// concrete `Painter` implementation. Generated `Canvas` code should therefore monomorphize
/// `on_paint` over that concrete type (e.g. a codegen-selected `type ActivePainter = ...;`) rather
/// than calling through `&mut dyn Painter`, matching 付録O.5's "avoid dynamic dispatch on hot paths".
pub trait Painter {
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);
    fn stroke_circle(&mut self, center: Point, radius: f32, color: Color, width: f32);
    fn draw_line(&mut self, from: Point, to: Point, color: Color, width: f32);
    fn draw_path(&mut self, path: &Path, style: PaintStyle);
    fn draw_text(&mut self, text: &str, pos: Point, font: Font, color: Color);
    fn draw_image(&mut self, image: &Image, rect: Rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_without_alpha_defaults_to_opaque() {
        assert_eq!(
            Color::hex("#eeeeee"),
            Color {
                r: 0xee,
                g: 0xee,
                b: 0xee,
                a: 0xff
            }
        );
    }

    #[test]
    fn hex_with_alpha_is_parsed() {
        assert_eq!(
            Color::hex("#11223344"),
            Color {
                r: 0x11,
                g: 0x22,
                b: 0x33,
                a: 0x44
            }
        );
    }
}
