//! Mode/target contract of the WinUI3 native build selection (Issue #294), exercised through the
//! same pure module `build.rs` uses.

#[allow(dead_code)]
#[path = "../build_support.rs"]
mod build_support;

use build_support::{
    ExportDirError, NativeBuildMode, PREBUILT_ARTIFACT_FILES, SUPPORTED_PREBUILT_TARGETS,
    missing_prebuilt_message, parse_build_mode, prebuilt_target_dir, resolve_export_dir,
};
use std::path::{Path, PathBuf};

#[test]
fn unset_mode_is_prebuilt() {
    assert_eq!(parse_build_mode(None), Ok(NativeBuildMode::Prebuilt));
}

#[test]
fn explicit_zero_is_prebuilt() {
    assert_eq!(parse_build_mode(Some("0")), Ok(NativeBuildMode::Prebuilt));
}

#[test]
fn explicit_one_is_source() {
    assert_eq!(parse_build_mode(Some("1")), Ok(NativeBuildMode::Source));
}

#[test]
fn invalid_modes_are_rejected_with_accepted_values() {
    for value in ["true", "source", "2", "", " 1", "1 ", "01", "yes"] {
        let error = parse_build_mode(Some(value)).expect_err(value);
        assert_eq!(error.value, value);
        let message = error.to_string();
        assert!(
            message.contains("ELWINDUI_WINUI3_BUILD_NATIVE"),
            "{message}"
        );
        assert!(
            message.contains("\"0\"") && message.contains("\"1\""),
            "{message}"
        );
    }
}

#[test]
fn supported_targets_map_to_exact_triple_directories() {
    let manifest = Path::new("crate-root");
    assert_eq!(
        SUPPORTED_PREBUILT_TARGETS,
        ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"]
    );
    for target in SUPPORTED_PREBUILT_TARGETS {
        assert_eq!(
            prebuilt_target_dir(manifest, target),
            Ok(PathBuf::from("crate-root")
                .join("native")
                .join("prebuilt")
                .join(target))
        );
    }
}

#[test]
fn unsupported_targets_have_no_prebuilt_directory() {
    for target in [
        "x86_64-pc-windows-gnu",
        "i686-pc-windows-msvc",
        "arm64ec-pc-windows-msvc",
        "aarch64-pc-windows-gnullvm",
        "x64",
        "arm64",
        "",
    ] {
        let error = prebuilt_target_dir(Path::new("crate-root"), target).expect_err(target);
        assert_eq!(error.target, target);
        let message = error.to_string();
        assert!(message.contains("x86_64-pc-windows-msvc"), "{message}");
        assert!(message.contains("aarch64-pc-windows-msvc"), "{message}");
    }
}

#[test]
fn prebuilt_artifact_set_is_the_three_native_outputs() {
    assert_eq!(
        PREBUILT_ARTIFACT_FILES,
        [
            "elwindui_winui3_app_host.lib",
            "Elwindui.WinUI3.Accessibility.winmd",
            "resources.pri",
        ]
    );
}

#[test]
fn missing_artifact_message_names_target_file_and_escape_hatch() {
    let missing = Path::new("native/prebuilt/aarch64-pc-windows-msvc/resources.pri");
    let message = missing_prebuilt_message("aarch64-pc-windows-msvc", missing);
    assert!(message.contains("aarch64-pc-windows-msvc"), "{message}");
    assert!(message.contains("resources.pri"), "{message}");
    assert!(message.contains("never fall"), "{message}");
    assert!(
        message.contains("ELWINDUI_WINUI3_BUILD_NATIVE=1"),
        "{message}"
    );
}

#[test]
fn export_dir_requires_source_mode() {
    assert_eq!(
        resolve_export_dir(NativeBuildMode::Prebuilt, None),
        Ok(None)
    );
    assert_eq!(resolve_export_dir(NativeBuildMode::Source, None), Ok(None));
    let error = resolve_export_dir(NativeBuildMode::Prebuilt, Some("staging"))
        .expect_err("export without source mode");
    assert_eq!(error, ExportDirError::RequiresSourceMode);
    assert!(
        error
            .to_string()
            .contains("ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR")
    );
    assert_eq!(
        resolve_export_dir(NativeBuildMode::Source, Some("")),
        Err(ExportDirError::Empty)
    );
    assert_eq!(
        resolve_export_dir(NativeBuildMode::Source, Some("staging")),
        Ok(Some(PathBuf::from("staging")))
    );
}
