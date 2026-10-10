//! Pure build-decision logic for `build.rs` (Issue #294).
//!
//! Dependency-free and side-effect-free on purpose: `build.rs` includes this file as a module, and
//! `tests/build_support.rs` includes the same file by path so the mode/target contract is unit-tested
//! without running a Cargo build script. Process launching, file copying and environment access
//! stay in `build.rs`.

use std::fmt;
use std::path::{Path, PathBuf};

/// Environment variable selecting how the native WinUI shim is obtained.
pub const BUILD_NATIVE_ENV: &str = "ELWINDUI_WINUI3_BUILD_NATIVE";

/// Environment variable naming the staging root a source-native build exports its artifacts to.
pub const PREBUILT_EXPORT_DIR_ENV: &str = "ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR";

/// Native C ABI generation. Must match the `elwindui_winui3_native_abi_v<N>` anchor exported by
/// `cpp/app_host.cpp` and referenced by `src/app.rs`, and `native_abi` in the prebuilt manifest.
pub const NATIVE_ABI_VERSION: u32 = 1;

/// Exact Cargo `TARGET` triples that have checked-in prebuilt native artifacts.
pub const SUPPORTED_PREBUILT_TARGETS: [&str; 2] =
    ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"];

/// Prebuilt bundle root, relative to the crate manifest directory, as path components.
pub const PREBUILT_ROOT: [&str; 2] = ["native", "prebuilt"];

/// Logical name of the native static library (`<name>.lib`).
pub const NATIVE_LIB_NAME: &str = "elwindui_winui3_app_host";

/// File names of the three per-target prebuilt artifacts.
pub const NATIVE_LIB_FILE: &str = "elwindui_winui3_app_host.lib";
pub const ACCESSIBILITY_WINMD_FILE: &str = "Elwindui.WinUI3.Accessibility.winmd";
pub const RESOURCES_PRI_FILE: &str = "resources.pri";
pub const PREBUILT_ARTIFACT_FILES: [&str; 3] = [
    NATIVE_LIB_FILE,
    ACCESSIBILITY_WINMD_FILE,
    RESOURCES_PRI_FILE,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeBuildMode {
    /// Link and deploy the checked-in artifacts under `native/prebuilt/<TARGET>/`.
    Prebuilt,
    /// Run the MIDL / C++/WinRT / MSVC / makepri pipeline from `cpp/`.
    Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidBuildMode {
    pub value: String,
}

impl fmt::Display for InvalidBuildMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{BUILD_NATIVE_ENV}={:?} is invalid; accepted values are unset, \"0\" (prebuilt native artifacts) or \"1\" (build native artifacts from source)",
            self.value
        )
    }
}

/// Parses `ELWINDUI_WINUI3_BUILD_NATIVE`. `None` means the variable is unset.
pub fn parse_build_mode(value: Option<&str>) -> Result<NativeBuildMode, InvalidBuildMode> {
    match value {
        None | Some("0") => Ok(NativeBuildMode::Prebuilt),
        Some("1") => Ok(NativeBuildMode::Source),
        Some(other) => Err(InvalidBuildMode {
            value: other.to_owned(),
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportDirError {
    /// The export variable was set while the mode is not `Source`.
    RequiresSourceMode,
    /// The export variable was set but empty.
    Empty,
}

impl fmt::Display for ExportDirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequiresSourceMode => write!(
                f,
                "{PREBUILT_EXPORT_DIR_ENV} is only honored when {BUILD_NATIVE_ENV}=1; checked-in prebuilt artifacts are never exported as newly manufactured output"
            ),
            Self::Empty => write!(f, "{PREBUILT_EXPORT_DIR_ENV} is set but empty"),
        }
    }
}

/// Validates `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR` against the selected mode. `None` means unset.
pub fn resolve_export_dir(
    mode: NativeBuildMode,
    value: Option<&str>,
) -> Result<Option<PathBuf>, ExportDirError> {
    match (mode, value) {
        (_, None) => Ok(None),
        (NativeBuildMode::Prebuilt, Some(_)) => Err(ExportDirError::RequiresSourceMode),
        (NativeBuildMode::Source, Some("")) => Err(ExportDirError::Empty),
        (NativeBuildMode::Source, Some(dir)) => Ok(Some(PathBuf::from(dir))),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedPrebuiltTarget {
    pub target: String,
}

impl fmt::Display for UnsupportedPrebuiltTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TARGET {} has no prebuilt WinUI3 native artifacts; supported targets are {}",
            self.target,
            SUPPORTED_PREBUILT_TARGETS.join(", ")
        )
    }
}

/// Maps the exact Cargo `TARGET` to `<manifest_dir>/native/prebuilt/<TARGET>`. No host
/// architecture or alias substitution.
pub fn prebuilt_target_dir(
    manifest_dir: &Path,
    target: &str,
) -> Result<PathBuf, UnsupportedPrebuiltTarget> {
    if SUPPORTED_PREBUILT_TARGETS.contains(&target) {
        Ok(manifest_dir
            .join(PREBUILT_ROOT[0])
            .join(PREBUILT_ROOT[1])
            .join(target))
    } else {
        Err(UnsupportedPrebuiltTarget {
            target: target.to_owned(),
        })
    }
}

/// Actionable message for a missing prebuilt artifact. No fallback is taken.
pub fn missing_prebuilt_message(target: &str, missing: &Path) -> String {
    format!(
        "prebuilt WinUI3 native artifact for TARGET {target} is missing: {}. \
         Normal builds use the checked-in artifacts under native/prebuilt/<TARGET>/ and never fall \
         back to compiling native sources. ElwindUI maintainers can build the native shim from \
         source with {BUILD_NATIVE_ENV}=1 (requires Visual Studio C++ and Windows SDK tools; see \
         docs/agents/winui3.md).",
        missing.display()
    )
}
