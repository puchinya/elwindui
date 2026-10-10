//! Pinned NuGet package resolution for `build.rs` (Issue #294 review remediation).
//!
//! Both native build modes must consume exactly the Windows App SDK / WinUI / Win2D versions the
//! prebuilt bundle is manufactured against, never "the lexicographically last version that happens
//! to be in the NuGet cache". The three direct pins mirror `tools/restore-winui3.ps1` (kept equal by
//! `tests/build_support.rs`); the transitive Foundation / InteractiveExperiences / WebView2 versions
//! are the ones the pinned packages' own `.nuspec` files declare.
//!
//! Reads the NuGet cache only: no process launching, copying or environment access. `build.rs` and
//! `tests/build_support.rs` include this file as a module.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// `(lowercase NuGet id, version)` of the directly pinned packages.
pub const WINDOWS_APP_SDK: (&str, &str) = ("microsoft.windowsappsdk", "1.8.260209005");
pub const WINDOWS_APP_SDK_WINUI: (&str, &str) = ("microsoft.windowsappsdk.winui", "1.8.260204000");
pub const WIN2D: (&str, &str) = ("microsoft.graphics.win2d", "1.4.0");

pub const FOUNDATION_ID: &str = "microsoft.windowsappsdk.foundation";
pub const INTERACTIVE_EXPERIENCES_ID: &str = "microsoft.windowsappsdk.interactiveexperiences";
pub const WEBVIEW2_ID: &str = "microsoft.web.webview2";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedPackage {
    /// Lowercase NuGet package id (the cache directory name).
    pub id: &'static str,
    pub version: String,
    /// `<nuget packages root>/<id>/<version>`.
    pub dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedPackages {
    pub app_sdk: PinnedPackage,
    pub winui: PinnedPackage,
    pub foundation: PinnedPackage,
    pub interactive_experiences: PinnedPackage,
    pub win2d: PinnedPackage,
    pub webview2: PinnedPackage,
}

impl PinnedPackages {
    pub fn all(&self) -> [&PinnedPackage; 6] {
        [
            &self.app_sdk,
            &self.winui,
            &self.foundation,
            &self.interactive_experiences,
            &self.win2d,
            &self.webview2,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NugetError(pub String);

impl fmt::Display for NugetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Restore the pinned packages with tools/restore-winui3.ps1 (or tools/setup-vs-env.ps1).",
            self.0
        )
    }
}

/// Returns the exact version `nuspec` declares for dependency `id` (case-insensitive). Accepts the
/// exact form `[v]` and the bare minimum form `v` (NuGet restores the lowest applicable version,
/// i.e. exactly `v`); rejects other ranges and inconsistent declarations across dependency groups.
pub fn nuspec_dependency_version(nuspec: &str, id: &str) -> Result<Option<String>, String> {
    let mut found: Option<String> = None;
    let mut rest = nuspec;
    while let Some(start) = rest.find("<dependency ") {
        let tag_and_rest = &rest[start..];
        let end = tag_and_rest
            .find('>')
            .ok_or_else(|| "unterminated <dependency> element".to_owned())?;
        let tag = &tag_and_rest[..end];
        rest = &tag_and_rest[end..];
        if !attribute(tag, "id").is_some_and(|value| value.eq_ignore_ascii_case(id)) {
            continue;
        }
        let raw = attribute(tag, "version")
            .ok_or_else(|| format!("dependency {id} declares no version"))?;
        let version = match raw.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            Some(exact) => exact,
            None => raw,
        };
        if version.is_empty() || version.contains([',', '(', ')', '[', ']', '*', ' ']) {
            return Err(format!(
                "dependency {id} has unsupported version range {raw:?}"
            ));
        }
        match &found {
            Some(previous) if previous != version => {
                return Err(format!(
                    "dependency {id} is declared with different versions {previous} and {version}"
                ));
            }
            _ => found = Some(version.to_owned()),
        }
    }
    Ok(found)
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let len = tag[start..].find('"')?;
    Some(&tag[start..start + len])
}

fn package(root: &Path, id: &'static str, version: &str) -> Result<PinnedPackage, NugetError> {
    let dir = root.join(id).join(version);
    if !dir.is_dir() {
        return Err(NugetError(format!(
            "Pinned NuGet package {id} {version} was not found at {} (other cached versions are never used).",
            dir.display()
        )));
    }
    Ok(PinnedPackage {
        id,
        version: version.to_owned(),
        dir,
    })
}

fn declared(from: &PinnedPackage, dependency: &str) -> Result<String, NugetError> {
    let nuspec_path = from.dir.join(format!("{}.nuspec", from.id));
    let text = std::fs::read_to_string(&nuspec_path)
        .map_err(|error| NugetError(format!("Cannot read {}: {error}.", nuspec_path.display())))?;
    nuspec_dependency_version(&text, dependency)
        .map_err(|error| NugetError(format!("{}: {error}.", nuspec_path.display())))?
        .ok_or_else(|| {
            NugetError(format!(
                "{} declares no dependency on {dependency}.",
                nuspec_path.display()
            ))
        })
}

/// Resolves every package `build.rs` reads from the NuGet cache rooted at `root`.
pub fn resolve_pinned_packages(root: &Path) -> Result<PinnedPackages, NugetError> {
    let app_sdk = package(root, WINDOWS_APP_SDK.0, WINDOWS_APP_SDK.1)?;
    let winui = package(root, WINDOWS_APP_SDK_WINUI.0, WINDOWS_APP_SDK_WINUI.1)?;
    let declared_winui = declared(&app_sdk, WINDOWS_APP_SDK_WINUI.0)?;
    if declared_winui != winui.version {
        return Err(NugetError(format!(
            "{} {} declares {} {declared_winui}, but {} is pinned.",
            app_sdk.id, app_sdk.version, winui.id, winui.version
        )));
    }
    let foundation = package(root, FOUNDATION_ID, &declared(&app_sdk, FOUNDATION_ID)?)?;
    let interactive_experiences = package(
        root,
        INTERACTIVE_EXPERIENCES_ID,
        &declared(&app_sdk, INTERACTIVE_EXPERIENCES_ID)?,
    )?;
    let win2d = package(root, WIN2D.0, WIN2D.1)?;
    let webview2 = package(root, WEBVIEW2_ID, &declared(&winui, WEBVIEW2_ID)?)?;
    Ok(PinnedPackages {
        app_sdk,
        winui,
        foundation,
        interactive_experiences,
        win2d,
        webview2,
    })
}

/// An explicit metadata override (`WINDOWS_APP_SDK_WINMD`, `WIN2D_WINMD`, `WEBVIEW2_WINMD`) must be
/// an existing `.winmd` file that, **after resolution** (`..`, symlinks and junctions resolved by
/// `std::fs::canonicalize`), lies inside a `<package id>/<pinned version>/` directory, wherever that
/// cache lives. A pinned-looking segment that only appears lexically (for example
/// `<id>/<pinned>/../<other>/...`, or a junction inside the pinned directory pointing elsewhere) is
/// rejected. Returns the resolved path, which is the one the build must read.
pub fn validate_override(path: &Path, package: &PinnedPackage) -> Result<PathBuf, NugetError> {
    let reject = |reason: &str| {
        Err(NugetError(format!(
            "{} {reason}; it must be an existing .winmd file inside a {}/{} package directory (the pinned version).",
            path.display(),
            package.id,
            package.version
        )))
    };
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return reject("does not exist");
    };
    if !resolved.is_file() {
        return reject("is not a file");
    }
    if !resolved
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("winmd"))
    {
        return reject("is not a .winmd metadata file");
    }
    if !resolved_in_pinned_version(&resolved, package) {
        return Err(NugetError(format!(
            "{} resolves to {}, which is not inside a {}/{} package directory (the pinned version).",
            path.display(),
            resolved.display(),
            package.id,
            package.version
        )));
    }
    Ok(without_verbatim_prefix(&resolved))
}

/// Whether a fully resolved path contains the adjacent `<package id>/<pinned version>` directories.
pub fn resolved_in_pinned_version(resolved: &Path, package: &PinnedPackage) -> bool {
    let mut components = Vec::new();
    for component in resolved.components() {
        match component {
            Component::Normal(part) => components.push(part.to_string_lossy().into_owned()),
            Component::Prefix(_) | Component::RootDir => {}
            // A resolved path never contains these; refuse rather than reason about them.
            Component::CurDir | Component::ParentDir => return false,
        }
    }
    // The file itself is the last component; the pair must be among its parent directories.
    components.pop();
    components.windows(2).any(|pair| {
        pair[0].eq_ignore_ascii_case(package.id) && pair[1].eq_ignore_ascii_case(&package.version)
    })
}

/// `\\?\C:\x` -> `C:\x` and `\\?\UNC\server\share` -> `\\server\share`, so resolved paths stay
/// usable as arguments to MIDL / cppwinrt.exe. Other paths are returned unchanged.
pub fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        PathBuf::from(local)
    } else {
        path.to_path_buf()
    }
}
