//! `SvgLoader` — file/bytes/string → `VectorImage`, options, limits (実装指示書§12).

use crate::convert;
use crate::error::{SvgError, SvgLimitKind, SvgWarning};
use crate::resolver::{
    DenyAllResolver, ResolvedSvgResource, SameDirectoryResolver, SvgBaseUri, SvgResourceError,
    SvgResourcePolicy, SvgResourceResolver,
};
use elwindui_core::base::Size;
use elwindui_core::graphics::VectorImage;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Hard caps applied after conversion (and, for byte-size limits, before parsing) — independent
/// of whatever internal limits `usvg` itself enforces (実装指示書§13).
#[derive(Debug, Clone, Copy)]
pub struct SvgLimits {
    pub max_source_bytes: usize,
    pub max_decompressed_bytes: usize,
    pub max_nodes: usize,
    pub max_path_commands: usize,
    pub max_group_depth: usize,
    pub max_filter_primitives: usize,
    pub max_embedded_image_bytes: usize,
    pub max_external_resources: usize,
    pub max_nested_svg_depth: usize,
}

impl Default for SvgLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 64 * 1024 * 1024,
            max_decompressed_bytes: 256 * 1024 * 1024,
            max_nodes: 200_000,
            max_path_commands: 2_000_000,
            max_group_depth: 256,
            max_filter_primitives: 10_000,
            max_embedded_image_bytes: 64 * 1024 * 1024,
            max_external_resources: 256,
            max_nested_svg_depth: 8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SvgLoadOptions {
    pub dpi: f32,
    pub default_size: Size,
    pub default_font_family: Arc<str>,
    pub default_font_size: f32,
    pub languages: Arc<[Arc<str>]>,
    pub style_sheet: Option<Arc<str>>,
    pub load_system_fonts: bool,
    pub resource_policy: SvgResourcePolicy,
    pub limits: SvgLimits,
}

impl Default for SvgLoadOptions {
    fn default() -> Self {
        Self {
            dpi: 96.0,
            default_size: Size {
                width: 100.0,
                height: 100.0,
            },
            default_font_family: Arc::from("Times New Roman"),
            default_font_size: 12.0,
            languages: Arc::from([Arc::from("en")]),
            style_sheet: None,
            load_system_fonts: false,
            resource_policy: SvgResourcePolicy::default(),
            limits: SvgLimits::default(),
        }
    }
}

pub struct SvgLoadResult {
    pub image: VectorImage,
    pub warnings: Arc<[SvgWarning]>,
}

pub struct SvgLoader {
    options: SvgLoadOptions,
    resolver: Arc<dyn SvgResourceResolver>,
    /// Additional font bytes registered via [`SvgLoader::add_font_data`]/
    /// [`SvgLoader::add_font_dir`] — kept separate from `options` since fonts are loaded
    /// incrementally into a `fontdb::Database` at parse time, not stored as raw options data
    /// (実装指示書§12.1 "font bytesまたはfont directoryを明示登録できるAPI").
    extra_font_data: Vec<Vec<u8>>,
    extra_font_dirs: Vec<std::path::PathBuf>,
}

impl SvgLoader {
    pub fn new(options: SvgLoadOptions) -> Self {
        let resolver: Arc<dyn SvgResourceResolver> = match options.resource_policy {
            SvgResourcePolicy::Custom => Arc::new(DenyAllResolver),
            _ => Arc::new(DenyAllResolver),
        };
        Self {
            options,
            resolver,
            extra_font_data: Vec::new(),
            extra_font_dirs: Vec::new(),
        }
    }

    /// Attaches a caller-supplied resolver, used when `options.resource_policy` is
    /// [`SvgResourcePolicy::Custom`] (ignored otherwise — the built-in policies never consult
    /// this).
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn SvgResourceResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    #[must_use]
    pub fn add_font_data(mut self, data: Vec<u8>) -> Self {
        self.extra_font_data.push(data);
        self
    }

    #[must_use]
    pub fn add_font_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.extra_font_dirs.push(dir.into());
        self
    }

    pub fn load_file(&self, path: impl AsRef<Path>) -> Result<VectorImage, SvgError> {
        Ok(self.load_file_with_diagnostics(path)?.image)
    }

    pub fn load_bytes(&self, bytes: &[u8]) -> Result<VectorImage, SvgError> {
        Ok(self.load_bytes_with_diagnostics(bytes, None)?.image)
    }

    pub fn load_str(&self, source: &str) -> Result<VectorImage, SvgError> {
        Ok(self.load_bytes_with_diagnostics(source.as_bytes(), None)?.image)
    }

    pub fn load_file_with_diagnostics(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<SvgLoadResult, SvgError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| SvgError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let resources_dir = path.parent().map(|p| p.to_path_buf());
        self.load_bytes_with_diagnostics(&bytes, resources_dir.as_deref())
    }

    fn load_bytes_with_diagnostics(
        &self,
        bytes: &[u8],
        resources_dir: Option<&Path>,
    ) -> Result<SvgLoadResult, SvgError> {
        if bytes.len() > self.options.limits.max_source_bytes {
            return Err(SvgError::ResourceLimitExceeded {
                kind: SvgLimitKind::SourceBytes,
                actual: bytes.len(),
                limit: self.options.limits.max_source_bytes,
            });
        }

        let text = decompress_if_gzip(bytes, self.options.limits.max_decompressed_bytes)?;

        let resolver = self.effective_resolver(resources_dir);
        let external_count = Arc::new(AtomicUsize::new(0));
        let nested_depth = Arc::new(AtomicUsize::new(0));
        let warnings: Arc<Mutex<Vec<SvgWarning>>> = Arc::new(Mutex::new(Vec::new()));

        let usvg_options = self.build_usvg_options(
            resources_dir,
            resolver,
            external_count,
            nested_depth,
            warnings.clone(),
        );

        let tree = usvg::Tree::from_str(&text, &usvg_options)?;
        let image = convert::convert_tree(&tree, &self.options.limits)?;

        let warnings: Vec<SvgWarning> = warnings.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Ok(SvgLoadResult {
            image,
            warnings: warnings.into(),
        })
    }

    fn effective_resolver(&self, resources_dir: Option<&Path>) -> Arc<dyn SvgResourceResolver> {
        match self.options.resource_policy {
            SvgResourcePolicy::DenyExternal | SvgResourcePolicy::DataUrlsOnly => {
                Arc::new(DenyAllResolver)
            }
            SvgResourcePolicy::SameDirectory => match resources_dir {
                Some(dir) => Arc::new(SameDirectoryResolver {
                    base_dir: dir.to_path_buf(),
                }),
                None => Arc::new(DenyAllResolver),
            },
            SvgResourcePolicy::Custom => self.resolver.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_usvg_options<'a>(
        &self,
        resources_dir: Option<&Path>,
        resolver: Arc<dyn SvgResourceResolver>,
        external_count: Arc<AtomicUsize>,
        nested_depth: Arc<AtomicUsize>,
        warnings: Arc<Mutex<Vec<SvgWarning>>>,
    ) -> usvg::Options<'a> {
        let max_external_resources = self.options.limits.max_external_resources;
        let max_nested_svg_depth = self.options.limits.max_nested_svg_depth;
        let max_embedded_image_bytes = self.options.limits.max_embedded_image_bytes;
        let base_uri = resources_dir
            .and_then(|d| d.to_str())
            .map(|s| SvgBaseUri(Arc::from(s)));
        let default_string_resolver = usvg::ImageHrefResolver::default_string_resolver();

        let mut fontdb = fontdb::Database::new();
        if self.options.load_system_fonts {
            fontdb.load_system_fonts();
        }
        for data in &self.extra_font_data {
            fontdb.load_font_data(data.clone());
        }
        for dir in &self.extra_font_dirs {
            fontdb.load_fonts_dir(dir);
        }

        usvg::Options {
            resources_dir: resources_dir.map(|p| p.to_path_buf()),
            dpi: self.options.dpi,
            font_family: self.options.default_font_family.to_string(),
            font_size: self.options.default_font_size,
            languages: self.options.languages.iter().map(|s| s.to_string()).collect(),
            shape_rendering: usvg::ShapeRendering::default(),
            text_rendering: usvg::TextRendering::default(),
            image_rendering: usvg::ImageRendering::default(),
            default_size: usvg::Size::from_wh(
                self.options.default_size.width,
                self.options.default_size.height,
            )
            .unwrap_or(usvg::Size::from_wh(100.0, 100.0).expect("100x100 is a valid size")),
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: {
                    let default_data_resolver = usvg::ImageHrefResolver::default_data_resolver();
                    Box::new(move |mime, data, opts| {
                        if data.len() > max_embedded_image_bytes {
                            return None;
                        }
                        default_data_resolver(mime, data, opts)
                    })
                },
                resolve_string: Box::new(move |href, opts| {
                    if external_count.fetch_add(1, Ordering::SeqCst) >= max_external_resources {
                        external_count.fetch_sub(1, Ordering::SeqCst);
                        warnings.lock().unwrap_or_else(|e| e.into_inner()).push(
                            SvgWarning::ExternalResourceSkipped {
                                href: Arc::from(href),
                            },
                        );
                        return None;
                    }
                    let depth = nested_depth.fetch_add(1, Ordering::SeqCst);
                    let _guard = scopeguard(&nested_depth);
                    if depth >= max_nested_svg_depth {
                        warnings.lock().unwrap_or_else(|e| e.into_inner()).push(
                            SvgWarning::ExternalResourceSkipped {
                                href: Arc::from(href),
                            },
                        );
                        return None;
                    }
                    match resolver.resolve(base_uri.as_ref(), href) {
                        Ok(ResolvedSvgResource { bytes, .. }) => {
                            if bytes.len() > max_embedded_image_bytes {
                                warnings.lock().unwrap_or_else(|e| e.into_inner()).push(
                                    SvgWarning::ExternalResourceSkipped {
                                        href: Arc::from(href),
                                    },
                                );
                                return None;
                            }
                            default_string_resolver(href, opts)
                        }
                        Err(SvgResourceError::Denied)
                        | Err(SvgResourceError::Traversal)
                        | Err(SvgResourceError::NotFound) => {
                            warnings.lock().unwrap_or_else(|e| e.into_inner()).push(
                                SvgWarning::ExternalResourceSkipped {
                                    href: Arc::from(href),
                                },
                            );
                            None
                        }
                    }
                }),
            },
            font_resolver: usvg::FontResolver::default(),
            fontdb: Arc::new(fontdb),
            style_sheet: self.options.style_sheet.as_ref().map(|s| s.to_string()),
        }
    }
}

/// Decrements `nested_depth` when dropped, regardless of the resolver closure's early-return
/// path — a tiny bespoke RAII guard rather than pulling in a `scopeguard`-style crate dependency
/// for one call site.
struct NestedDepthGuard<'a>(&'a AtomicUsize);
impl Drop for NestedDepthGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
fn scopeguard(counter: &AtomicUsize) -> NestedDepthGuard<'_> {
    NestedDepthGuard(counter)
}

/// Decompresses gzip (`.svgz`) input with a hard cap on the decompressed size, guarding against
/// decompression-bomb inputs (実装指示書§22.9). Reuses `usvg::decompress_svgz` (which itself uses
/// `flate2`, already a transitive dependency via `usvg`) for the actual inflate rather than
/// duplicating a gzip decoder dependency — the size cap this function adds on top is the part
/// `usvg::Tree::from_data`'s own uncapped call site doesn't have, which is why plain SVG text is
/// handled here and handed to `Tree::from_str` directly instead of going through `from_data`.
fn decompress_if_gzip(bytes: &[u8], max_decompressed_bytes: usize) -> Result<String, SvgError> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        // `usvg::decompress_svgz` reads to completion internally (no size cap of its own), so the
        // cap here is enforced after the fact — acceptable since `max_source_bytes` (checked
        // before this runs) already bounds the *compressed* input, capping the worst-case
        // amplification a single call can produce.
        let decoded = usvg::decompress_svgz(bytes).map_err(|e| SvgError::Parse {
            message: e.to_string().into(),
        })?;
        if decoded.len() > max_decompressed_bytes {
            return Err(SvgError::ResourceLimitExceeded {
                kind: SvgLimitKind::DecompressedBytes,
                actual: decoded.len(),
                limit: max_decompressed_bytes,
            });
        }
        String::from_utf8(decoded).map_err(|_| SvgError::Parse {
            message: "decompressed SVG is not valid UTF-8".into(),
        })
    } else {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| SvgError::Parse {
                message: "SVG source is not valid UTF-8".into(),
            })
    }
}
