//! Menu icon fixtures for the MenuBar and the Context Menu tab: one icon per `IconSource` kind the
//! native menus must realize (System, Encoded raster, Rgba8 raster, Vector), plus a "dynamic" icon
//! that a button cycles through System -> Rgba8 -> none to exercise live set / replace / clear and
//! a later context-menu realization picking up the latest icon.

use elwindui::core::graphics::{
    AlphaMode, BitmapImage, IconSource, ImageSource, SystemIcon,
};

/// A 16x16 solid green PNG, embedded so the Encoded raster path needs no external asset.
const GREEN_PNG: [u8; 82] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0xf3, 0xff,
    0x61, 0x00, 0x00, 0x00, 0x19, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xd0, 0x58, 0x15, 0xf0,
    0x9f, 0x12, 0xcc, 0x30, 0x6a, 0xc0, 0xa8, 0x01, 0xa3, 0x06, 0x0c, 0x17, 0x03, 0x00, 0x37, 0x5e,
    0x21, 0x1f, 0xc5, 0xa3, 0xab, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
    0x60, 0x82,
];

pub(crate) fn encoded_png_icon() -> Option<IconSource> {
    Some(IconSource::Image(ImageSource::Raster(
        BitmapImage::from_encoded(GREEN_PNG.to_vec()),
    )))
}

/// An 8x8 solid orange straight-alpha Rgba8 bitmap.
pub(crate) fn rgba8_icon() -> Option<IconSource> {
    let (width, height) = (8u32, 8u32);
    let pixels = [240u8, 140, 30, 255].repeat((width * height) as usize);
    let bitmap = BitmapImage::from_rgba8(width, height, width * 4, pixels, AlphaMode::Straight)
        .expect("demo Rgba8 menu icon pixel buffer is well-formed");
    Some(IconSource::Image(ImageSource::Raster(bitmap)))
}

/// The dynamic icon for `mode % 3`: 0 = `SystemIcon::Edit`, 1 = Rgba8 raster, 2 = no icon.
pub(crate) fn dynamic_icon(mode: usize) -> Option<IconSource> {
    match mode % 3 {
        0 => Some(IconSource::System(SystemIcon::Edit)),
        1 => rgba8_icon(),
        _ => None,
    }
}

pub(crate) fn dynamic_icon_label(mode: usize) -> String {
    match mode % 3 {
        0 => "Dynamic icon: System(Edit)".to_string(),
        1 => "Dynamic icon: Rgba8 raster".to_string(),
        _ => "Dynamic icon: (none)".to_string(),
    }
}
