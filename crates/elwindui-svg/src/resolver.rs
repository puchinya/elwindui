//! External/data-URL resource resolution and its security policy (実装指示書§13).

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SvgBaseUri(pub Arc<str>);

#[derive(Debug, Clone)]
pub struct ResolvedSvgResource {
    pub bytes: Arc<[u8]>,
    pub media_type: Option<Arc<str>>,
    pub canonical_uri: Option<Arc<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgResourceError {
    Denied,
    NotFound,
    Traversal,
}

/// Resolves an `xlink:href`-style external reference to bytes. Only consulted for plain string
/// hrefs (file paths) — data URLs are always decoded in-place by `usvg` itself, since they carry
/// no filesystem/network access surface to police (実装指示書§13; see `loader.rs`'s own doc
/// comment on why the policy machinery only gates `resolve_string`, not `resolve_data`).
pub trait SvgResourceResolver: Send + Sync {
    fn resolve(
        &self,
        base: Option<&SvgBaseUri>,
        href: &str,
    ) -> Result<ResolvedSvgResource, SvgResourceError>;
}

/// Denies every external string href — the `Custom` policy's placeholder until a real resolver is
/// supplied via `SvgLoader::with_resolver`.
pub(crate) struct DenyAllResolver;

impl SvgResourceResolver for DenyAllResolver {
    fn resolve(
        &self,
        _base: Option<&SvgBaseUri>,
        _href: &str,
    ) -> Result<ResolvedSvgResource, SvgResourceError> {
        Err(SvgResourceError::Denied)
    }
}

/// Resolves file paths strictly within a fixed base directory, rejecting `..` traversal, symlink
/// escape, and absolute/UNC path substitution (実装指示書§13).
pub(crate) struct SameDirectoryResolver {
    pub base_dir: PathBuf,
}

impl SvgResourceResolver for SameDirectoryResolver {
    fn resolve(
        &self,
        _base: Option<&SvgBaseUri>,
        href: &str,
    ) -> Result<ResolvedSvgResource, SvgResourceError> {
        let requested = Path::new(href);
        if requested.is_absolute() {
            return Err(SvgResourceError::Traversal);
        }
        let candidate = self.base_dir.join(requested);
        let canonical_base = self
            .base_dir
            .canonicalize()
            .map_err(|_| SvgResourceError::NotFound)?;
        let canonical_candidate = candidate
            .canonicalize()
            .map_err(|_| SvgResourceError::NotFound)?;
        if !canonical_candidate.starts_with(&canonical_base) {
            // Catches both `..`-traversal and symlink escape: `canonicalize()` resolves symlinks,
            // so an in-directory symlink pointing outside `base_dir` still fails this prefix check.
            return Err(SvgResourceError::Traversal);
        }
        let bytes = std::fs::read(&canonical_candidate).map_err(|_| SvgResourceError::NotFound)?;
        Ok(ResolvedSvgResource {
            bytes: bytes.into(),
            media_type: None,
            canonical_uri: canonical_candidate.to_str().map(Arc::from),
        })
    }
}

/// Governs how `<image xlink:href="...">` string references (not data URLs — see
/// `SvgResourceResolver`'s own doc comment) are resolved. Default denies everything that isn't
/// embedded in the document itself; a network fetch is never performed by any policy, since
/// `usvg`'s own resolvers only ever touch the local filesystem (実装指示書§13).
#[derive(Debug, Clone, Default)]
pub enum SvgResourcePolicy {
    #[default]
    DenyExternal,
    /// Behaviorally identical to `DenyExternal` today: with `resolve_data` (data URLs) always
    /// enabled regardless of policy, "only data URLs resolve" and "deny every external href" are
    /// the same outcome. Kept as a distinct variant to match 実装指示書§13's named policy and to
    /// leave room for the two to diverge later (e.g. if `DenyExternal` grows an allowance this
    /// variant intentionally excludes).
    DataUrlsOnly,
    SameDirectory,
    Custom,
}
