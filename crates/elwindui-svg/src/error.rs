//! Loader error/warning/diagnostic types (SVG読み込み・ベクター描画対応 実装指示書§21).

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// A named `usvg`/elwindui-svg unsupported dynamic feature — JS, SMIL/CSS animation, event
/// handlers, `foreignObject`, etc. (指示書§3.2 対象外一覧). Kept as a `SvgWarning` payload rather
/// than an enum of its own, since the set is open-ended and only ever surfaced as text.
pub type SvgFeature = Arc<str>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgLimitKind {
    SourceBytes,
    DecompressedBytes,
    Nodes,
    PathCommands,
    GroupDepth,
    FilterPrimitives,
    EmbeddedImageBytes,
    ExternalResources,
    NestedSvgDepth,
}

impl fmt::Display for SvgLimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::SourceBytes => "source_bytes",
            Self::DecompressedBytes => "decompressed_bytes",
            Self::Nodes => "nodes",
            Self::PathCommands => "path_commands",
            Self::GroupDepth => "group_depth",
            Self::FilterPrimitives => "filter_primitives",
            Self::EmbeddedImageBytes => "embedded_image_bytes",
            Self::ExternalResources => "external_resources",
            Self::NestedSvgDepth => "nested_svg_depth",
        };
        write!(f, "{name}")
    }
}

#[derive(Debug)]
pub enum SvgError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        message: Arc<str>,
    },
    InvalidGeometry {
        message: Arc<str>,
    },
    UnsupportedFeature {
        feature: SvgFeature,
        element_id: Option<Arc<str>>,
    },
    MissingFont {
        family: Arc<str>,
    },
    ResourceDenied {
        href: Arc<str>,
    },
    ResourceNotFound {
        href: Arc<str>,
    },
    ResourceLimitExceeded {
        kind: SvgLimitKind,
        actual: usize,
        limit: usize,
    },
    Conversion {
        message: Arc<str>,
    },
}

impl fmt::Display for SvgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "failed to read '{}': {source}", path.display()),
            Self::Parse { message } => write!(f, "SVG parse error: {message}"),
            Self::InvalidGeometry { message } => write!(f, "invalid SVG geometry: {message}"),
            Self::UnsupportedFeature {
                feature,
                element_id,
            } => match element_id {
                Some(id) => write!(f, "unsupported SVG feature '{feature}' on element '{id}'"),
                None => write!(f, "unsupported SVG feature '{feature}'"),
            },
            Self::MissingFont { family } => write!(f, "missing font family '{family}'"),
            Self::ResourceDenied { href } => write!(f, "resource access denied: '{href}'"),
            Self::ResourceNotFound { href } => write!(f, "resource not found: '{href}'"),
            Self::ResourceLimitExceeded {
                kind,
                actual,
                limit,
            } => write!(f, "SVG limit '{kind}' exceeded: {actual} > {limit}"),
            Self::Conversion { message } => write!(f, "SVG conversion error: {message}"),
        }
    }
}

impl std::error::Error for SvgError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<usvg::Error> for SvgError {
    fn from(err: usvg::Error) -> Self {
        SvgError::Parse {
            message: err.to_string().into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SvgWarning {
    UnsupportedDynamicFeature { feature: SvgFeature },
    MissingFontFallbackUsed { requested: Arc<str>, selected: Arc<str> },
    ExternalResourceSkipped { href: Arc<str> },
    InvalidElementRemoved { id: Option<Arc<str>> },
    ApproximationUsed { feature: Arc<str>, detail: Arc<str> },
}

impl fmt::Display for SvgWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedDynamicFeature { feature } => {
                write!(f, "unsupported dynamic feature ignored: {feature}")
            }
            Self::MissingFontFallbackUsed { requested, selected } => write!(
                f,
                "font '{requested}' not found, falling back to '{selected}'"
            ),
            Self::ExternalResourceSkipped { href } => {
                write!(f, "external resource skipped: '{href}'")
            }
            Self::InvalidElementRemoved { id } => match id {
                Some(id) => write!(f, "invalid element removed: '{id}'"),
                None => write!(f, "invalid element removed"),
            },
            Self::ApproximationUsed { feature, detail } => {
                write!(f, "approximation used for '{feature}': {detail}")
            }
        }
    }
}
