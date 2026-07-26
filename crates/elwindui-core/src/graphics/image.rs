use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

/// Stable identity of one logical [`Image`] resource.
///
/// Clones of an image share its ID. Independently created images intentionally receive distinct
/// IDs even when their pixel contents are identical, so backends can cache decoded native images
/// without hashing potentially large buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(u64);

fn next_image_id() -> ImageId {
    static NEXT_IMAGE_ID: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT_IMAGE_ID.get_or_init(|| AtomicU64::new(1));
    ImageId(next.fetch_add(1, Ordering::Relaxed))
}

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

fn image_format_from_extension(path: &std::path::Path) -> ImageFormat {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => ImageFormat::Png,
        Some("jpg" | "jpeg") => ImageFormat::Jpeg,
        Some("webp") => ImageFormat::WebP,
        Some("gif") => ImageFormat::Gif,
        Some("bmp") => ImageFormat::Bmp,
        Some("tif" | "tiff") => ImageFormat::Tiff,
        _ => ImageFormat::Unknown,
    }
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
#[derive(Debug, Clone)]
pub struct Image {
    inner: Arc<ImageResource>,
}

impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.data() == other.data()
    }
}

#[derive(Debug)]
struct ImageResource {
    id: ImageId,
    data: ImageData,
}

impl ImageResource {
    fn new(data: ImageData) -> Self {
        Self {
            id: next_image_id(),
            data,
        }
    }
}

impl Image {
    pub fn from_encoded(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            inner: Arc::new(ImageResource::new(ImageData::Encoded {
                bytes: bytes.into(),
                format_hint: None,
            })),
        }
    }

    pub fn from_encoded_with_format(bytes: impl Into<Arc<[u8]>>, format: ImageFormat) -> Self {
        Self {
            inner: Arc::new(ImageResource::new(ImageData::Encoded {
                bytes: bytes.into(),
                format_hint: Some(format),
            })),
        }
    }

    /// Reads `path`'s raw bytes off disk and wraps them as `ImageData::Encoded`, same as
    /// `from_encoded_with_format` but sourcing the bytes from a file instead of a caller-supplied
    /// buffer — `format_hint` is guessed from the file extension (`ImageFormat::Unknown` if
    /// unrecognized). The bytes are never decoded here; that stays each backend's own job (e.g.
    /// `elwindui-backend-appkit`'s `decode_cgimage` hands `Encoded` bytes straight to `NSImage`,
    /// which sniffs the actual format itself rather than trusting `format_hint`).
    pub fn from_file(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        Ok(Self::from_encoded_with_format(
            bytes,
            image_format_from_extension(path),
        ))
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
            inner: Arc::new(ImageResource::new(ImageData::Rgba8 {
                width,
                height,
                stride,
                pixels,
                alpha,
            })),
        })
    }

    pub fn from_backend_handle(handle: BackendImageHandle) -> Self {
        Self {
            inner: Arc::new(ImageResource::new(ImageData::Backend(handle))),
        }
    }

    /// Returns this logical image resource's stable cache identity.
    ///
    /// The value is shared by every clone and is distinct for separately constructed images;
    /// callers must not interpret it as a content hash or a serialization format.
    pub fn id(&self) -> ImageId {
        self.inner.id
    }

    /// Returns the decode-agnostic data associated with this image.
    pub fn data(&self) -> &ImageData {
        &self.inner.data
    }

    pub fn pixel_size(&self) -> Option<(u32, u32)> {
        match &self.inner.data {
            ImageData::Rgba8 { width, height, .. } => Some((*width, *height)),
            _ => None,
        }
    }

    pub fn is_opaque(&self) -> Option<bool> {
        match &self.inner.data {
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

/// `ImageBrush::stretch` -> `ImageFit` — the same four cases under the vocabulary
/// [`fitted_image_rect`] speaks, so an `ImageBrush` fill can reuse that placement helper as-is.
impl From<super::brush::Stretch> for ImageFit {
    fn from(stretch: super::brush::Stretch) -> Self {
        use super::brush::Stretch;
        match stretch {
            Stretch::None => ImageFit::None,
            Stretch::Fill => ImageFit::Fill,
            Stretch::Uniform => ImageFit::Contain,
            Stretch::UniformToFill => ImageFit::Cover,
        }
    }
}

/// Where an image of `image_size` (already-cropped pixel dimensions) actually lands inside `dest`
/// once `fit`/`alignment_x`/`alignment_y` are applied, in the same coordinate space as `dest`.
///
/// `Fill` always returns `dest` unchanged; `Contain`/`Cover` scale `image_size` to fit inside /
/// cover `dest` while preserving aspect ratio; `None` draws at intrinsic size. Leftover space
/// (`Contain`/`None`) or overflow (`Cover`/`None`) is distributed per the alignments — overflow is
/// why a caller drawing this generally needs its own clip-to-`dest` container rather than handing
/// the result straight to `dest`'s own layer.
///
/// A degenerate `image_size` (either axis `<= 0`) falls back to `dest`'s own size rather than
/// producing `NaN`/`inf` from the division.
///
/// Backend-independent (pure `f32` geometry over `elwindui_core` value types), so it lives here
/// rather than being re-derived per backend — AppKit, Win2D, and the WinUI3 Composition renderer
/// all place images with exactly this rule.
pub fn fitted_image_rect(
    dest: crate::base::Rect,
    image_size: (f32, f32),
    fit: ImageFit,
    alignment_x: super::brush::AlignmentX,
    alignment_y: super::brush::AlignmentY,
) -> crate::base::Rect {
    use super::brush::{AlignmentX, AlignmentY};
    let (iw, ih) = image_size;
    let (width, height) = if iw <= 0.0 || ih <= 0.0 {
        (dest.width, dest.height)
    } else {
        match fit {
            ImageFit::Fill => (dest.width, dest.height),
            ImageFit::None => (iw, ih),
            ImageFit::Contain => {
                let scale = (dest.width / iw).min(dest.height / ih);
                (iw * scale, ih * scale)
            }
            ImageFit::Cover => {
                let scale = (dest.width / iw).max(dest.height / ih);
                (iw * scale, ih * scale)
            }
        }
    };
    let x = match alignment_x {
        AlignmentX::Left => dest.x,
        AlignmentX::Center => dest.x + (dest.width - width) / 2.0,
        AlignmentX::Right => dest.x + dest.width - width,
    };
    let y = match alignment_y {
        AlignmentY::Top => dest.y,
        AlignmentY::Center => dest.y + (dest.height - height) / 2.0,
        AlignmentY::Bottom => dest.y + dest.height - height,
    };
    crate::base::Rect {
        x,
        y,
        width,
        height,
    }
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
    fn clones_share_a_stable_image_id_for_rgba_images() {
        let image = Image::from_rgba8(1, 1, 4, vec![0, 0, 0, 255], AlphaMode::Premultiplied)
            .expect("valid RGBA image");
        assert_eq!(image.id(), image.clone().id());
    }

    #[test]
    fn separately_created_images_have_distinct_ids() {
        let first = Image::from_encoded(vec![1, 2, 3]);
        let second = Image::from_encoded(vec![1, 2, 3]);
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn encoded_images_keep_their_id_when_cloned() {
        let image = Image::from_encoded_with_format(vec![1, 2, 3], ImageFormat::Png);
        assert_eq!(image.id(), image.clone().id());
    }

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

    #[test]
    fn from_file_reads_bytes_and_hints_format_from_extension() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("elwindui-image-test-{}.png", std::process::id()));
        std::fs::write(&path, b"not a real png, just bytes to round-trip").unwrap();
        let image = Image::from_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        match image.data() {
            ImageData::Encoded { bytes, format_hint } => {
                assert_eq!(&**bytes, b"not a real png, just bytes to round-trip");
                assert_eq!(*format_hint, Some(ImageFormat::Png));
            }
            other => panic!("expected ImageData::Encoded, got {other:?}"),
        }
    }

    #[test]
    fn from_file_reports_unknown_format_for_unrecognized_extension() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("elwindui-image-test-{}.bin", std::process::id()));
        std::fs::write(&path, b"bytes").unwrap();
        let image = Image::from_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        match image.data() {
            ImageData::Encoded { format_hint, .. } => {
                assert_eq!(*format_hint, Some(ImageFormat::Unknown))
            }
            other => panic!("expected ImageData::Encoded, got {other:?}"),
        }
    }

    #[test]
    fn from_file_errors_on_missing_file() {
        assert!(Image::from_file("/nonexistent/elwindui-image-test.png").is_err());
    }
}
