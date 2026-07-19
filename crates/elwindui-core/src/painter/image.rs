use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
    Gif,
    Bmp,
    Tiff,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Premultiplied,
    Straight,
    Opaque,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageData {
    Encoded {
        bytes: Arc<[u8]>,
        format_hint: Option<ImageFormat>,
    },
    Rgba8 {
        width: u32,
        height: u32,
        stride: u32,
        pixels: Arc<[u8]>,
        alpha: AlphaMode,
    },
    /// Type-erased backend-native handle (e.g. an already-decoded/uploaded native bitmap). Not
    /// portable across backends — see painter design doc §13.1.
    Backend(BackendImageHandle),
}

/// Opaque, backend-owned image handle. `elwindui-core` never inspects its contents; it only
/// carries it through the retained render tree.
#[derive(Clone)]
pub struct BackendImageHandle(pub Arc<dyn std::any::Any + Send + Sync>);

impl fmt::Debug for BackendImageHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackendImageHandle").finish_non_exhaustive()
    }
}
impl PartialEq for BackendImageHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageError;

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RGBA8 pixel buffer size does not match width*height*4 given the stride"
        )
    }
}
impl std::error::Error for ImageError {}

/// A decode-agnostic, cheaply-`Clone`able (via `Arc`) image handle — never re-decoded/re-uploaded
/// on repaint (painter design doc §13.1/§14 "画像・pathリソースをフレーム再生成しない").
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    inner: Arc<ImageData>,
}

impl Image {
    pub fn from_encoded(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            inner: Arc::new(ImageData::Encoded {
                bytes: bytes.into(),
                format_hint: None,
            }),
        }
    }

    pub fn from_encoded_with_format(bytes: impl Into<Arc<[u8]>>, format: ImageFormat) -> Self {
        Self {
            inner: Arc::new(ImageData::Encoded {
                bytes: bytes.into(),
                format_hint: Some(format),
            }),
        }
    }

    pub fn from_rgba8(
        width: u32,
        height: u32,
        stride: u32,
        pixels: impl Into<Arc<[u8]>>,
        alpha: AlphaMode,
    ) -> Result<Self, ImageError> {
        let pixels = pixels.into();
        let required = stride as usize * height as usize;
        if stride < width * 4 || pixels.len() < required {
            return Err(ImageError);
        }
        Ok(Self {
            inner: Arc::new(ImageData::Rgba8 {
                width,
                height,
                stride,
                pixels,
                alpha,
            }),
        })
    }

    pub fn from_backend_handle(handle: BackendImageHandle) -> Self {
        Self {
            inner: Arc::new(ImageData::Backend(handle)),
        }
    }

    pub fn data(&self) -> &ImageData {
        &self.inner
    }

    pub fn pixel_size(&self) -> Option<(u32, u32)> {
        match &*self.inner {
            ImageData::Rgba8 { width, height, .. } => Some((*width, *height)),
            _ => None,
        }
    }

    pub fn is_opaque(&self) -> Option<bool> {
        match &*self.inner {
            ImageData::Rgba8 { alpha, .. } => Some(*alpha == AlphaMode::Opaque),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSampling {
    Nearest,
    Linear,
    Cubic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFit {
    Fill,
    Contain,
    Cover,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDrawOptions {
    pub opacity: f32,
    pub sampling: ImageSampling,
    pub fit: ImageFit,
    pub alignment_x: super::brush::AlignmentX,
    pub alignment_y: super::brush::AlignmentY,
    pub repeat: super::brush::TileMode,
}

impl Default for ImageDrawOptions {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            sampling: ImageSampling::Linear,
            fit: ImageFit::Fill,
            alignment_x: super::brush::AlignmentX::Center,
            alignment_y: super::brush::AlignmentY::Center,
            repeat: super::brush::TileMode::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba8_validates_buffer_size() {
        let pixels = vec![0u8; 4 * 4 * 4];
        assert!(Image::from_rgba8(4, 4, 16, pixels.clone(), AlphaMode::Straight).is_ok());
        assert!(Image::from_rgba8(4, 4, 16, vec![0u8; 4], AlphaMode::Straight).is_err());
    }

    #[test]
    fn rgba8_rejects_stride_smaller_than_row_bytes() {
        let pixels = vec![0u8; 4 * 4 * 4];
        assert!(Image::from_rgba8(4, 4, 8, pixels, AlphaMode::Straight).is_err());
    }

    #[test]
    fn encoded_image_has_no_known_pixel_size() {
        let image = Image::from_encoded(vec![0u8; 10]);
        assert_eq!(image.pixel_size(), None);
    }
}
