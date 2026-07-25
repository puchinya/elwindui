//! A CPU `CGBitmapContext` the golden tests render into and sample pixels from.
//!
//! Previously defined twice, identically, inside `inner.rs`'s two inline test modules.

use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGColorSpace;

pub(crate) struct Bitmap {
    pub(crate) ctx: CFRetained<objc2_core_graphics::CGContext>,
    pub(crate) pixels: Box<[u8]>,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) bytes_per_row: usize,
}

impl Bitmap {
    pub(crate) fn new(width: usize, height: usize) -> Self {
        let bytes_per_row = width * 4;
        let mut pixels = vec![0u8; bytes_per_row * height].into_boxed_slice();
        let color_space = CGColorSpace::new_device_rgb().expect("device RGB color space");
        #[allow(deprecated)]
        let bitmap_info = objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast.0
            | objc2_core_graphics::CGBitmapInfo::ByteOrder32Big.0;
        let ctx = unsafe {
            objc2_core_graphics::CGBitmapContextCreate(
                pixels.as_mut_ptr() as *mut _,
                width,
                height,
                8,
                bytes_per_row,
                Some(&color_space),
                bitmap_info,
            )
        }
        .expect("CGBitmapContextCreate");
        Self {
            ctx,
            pixels,
            width,
            height,
            bytes_per_row,
        }
    }

    pub(crate) fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        assert!(x < self.width && y < self.height);
        let offset = y * self.bytes_per_row + x * 4;
        (
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        )
    }
}
