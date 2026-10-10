//! Mode/target contract of the WinUI3 native build selection (Issue #294), exercised through the
//! same pure module `build.rs` uses.

#[allow(dead_code)]
#[path = "../build_nuget.rs"]
mod build_nuget;
#[allow(dead_code)]
#[path = "../build_support.rs"]
mod build_support;

use build_support::{
    ExportDirError, NativeBuildMode, PREBUILT_ARTIFACT_FILES, SUPPORTED_PREBUILT_TARGETS,
    export_dir_conflicts, missing_prebuilt_message, parse_build_mode, prebuilt_target_dir,
    resolve_export_dir,
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

#[test]
fn export_destination_inside_equal_to_or_above_tracked_tree_conflicts() {
    let tracked = Path::new(r"C:\repo\crates\elwindui-backend-winui3\native\prebuilt");
    for requested in [
        r"C:\repo\crates\elwindui-backend-winui3\native\prebuilt",
        r"C:\repo\crates\elwindui-backend-winui3\native\prebuilt\unwanted-dir",
        r"C:\repo\crates\elwindui-backend-winui3\native\prebuilt\x86_64-pc-windows-msvc\x",
        r"C:\repo\crates\elwindui-backend-winui3\NATIVE\Prebuilt\unwanted-dir",
        r"C:\repo\crates\elwindui-backend-winui3\native",
        r"C:\repo",
        r"C:\",
    ] {
        assert!(
            export_dir_conflicts(Path::new(requested), tracked),
            "{requested}"
        );
    }
    for requested in [
        r"C:\repo\.build\winui3-native-staging",
        r"C:\repo\crates\elwindui-backend-winui3\native-staging",
        r"C:\repo\crates\elwindui-backend-winui3\native\prebuilt-export",
        r"D:\a\_temp\winui3-native",
    ] {
        assert!(
            !export_dir_conflicts(Path::new(requested), tracked),
            "{requested}"
        );
    }
}

/// A throwaway NuGet cache under the system temp directory.
struct FakeNugetCache(PathBuf);

impl FakeNugetCache {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "elwindui-nuget-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn package(&self, id: &str, version: &str, dependencies: &[(&str, &str)]) {
        let dir = self.0.join(id).join(version);
        std::fs::create_dir_all(&dir).unwrap();
        let deps: String = dependencies
            .iter()
            .map(|(dep, ver)| format!("      <dependency id=\"{dep}\" version=\"{ver}\" />\n"))
            .collect();
        // Two dependency groups, like the real packages, declaring the same versions.
        std::fs::write(
            dir.join(format!("{id}.nuspec")),
            format!(
                "<package><metadata><id>{id}</id><version>{version}</version><dependencies>\n    <group targetFramework=\"native0.0\">\n{deps}    </group>\n    <group targetFramework=\"net6.0-windows10.0.17763.0\">\n{deps}    </group>\n</dependencies></metadata></package>"
            ),
        )
        .unwrap();
    }

    /// The pinned 1.8 package graph plus newer versions of every package next to it.
    fn pinned_with_newer_versions(name: &str) -> Self {
        let cache = Self::new(name);
        for (app_sdk, winui, foundation, ix) in [
            (
                "1.8.260209005",
                "1.8.260204000",
                "1.8.260203002",
                "1.8.260125001",
            ),
            (
                "1.8.260317003",
                "1.8.260224000",
                "1.8.260222000",
                "1.8.260301000",
            ),
        ] {
            cache.package(
                "microsoft.windowsappsdk",
                app_sdk,
                &[
                    (
                        "Microsoft.WindowsAppSDK.Foundation",
                        &format!("[{foundation}]"),
                    ),
                    (
                        "Microsoft.WindowsAppSDK.InteractiveExperiences",
                        &format!("[{ix}]"),
                    ),
                    ("Microsoft.WindowsAppSDK.WinUI", &format!("[{winui}]")),
                ],
            );
        }
        cache.package(
            "microsoft.windowsappsdk.winui",
            "1.8.260204000",
            &[("Microsoft.Web.WebView2", "1.0.3179.45")],
        );
        cache.package(
            "microsoft.windowsappsdk.winui",
            "1.8.260224000",
            &[("Microsoft.Web.WebView2", "1.0.3200.0")],
        );
        for (id, versions) in [
            (
                "microsoft.windowsappsdk.foundation",
                ["1.8.260203002", "1.8.260222000"],
            ),
            (
                "microsoft.windowsappsdk.interactiveexperiences",
                ["1.8.260125001", "1.8.260301000"],
            ),
            ("microsoft.graphics.win2d", ["1.4.0", "1.5.0"]),
            ("microsoft.web.webview2", ["1.0.3179.45", "1.0.3200.0"]),
        ] {
            for version in versions {
                cache.package(id, version, &[]);
            }
        }
        cache
    }
}

impl Drop for FakeNugetCache {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pinned_packages_win_over_newer_cached_versions() {
    let cache = FakeNugetCache::pinned_with_newer_versions("two-versions");
    let packages = build_nuget::resolve_pinned_packages(&cache.0).expect("resolve pinned");
    let selected: Vec<(&str, &str)> = packages
        .all()
        .iter()
        .map(|package| (package.id, package.version.as_str()))
        .collect();
    assert_eq!(
        selected,
        [
            ("microsoft.windowsappsdk", "1.8.260209005"),
            ("microsoft.windowsappsdk.winui", "1.8.260204000"),
            ("microsoft.windowsappsdk.foundation", "1.8.260203002"),
            (
                "microsoft.windowsappsdk.interactiveexperiences",
                "1.8.260125001"
            ),
            ("microsoft.graphics.win2d", "1.4.0"),
            ("microsoft.web.webview2", "1.0.3179.45"),
        ]
    );
    for package in packages.all() {
        assert_eq!(package.dir, cache.0.join(package.id).join(&package.version));
    }
}

#[test]
fn missing_pinned_package_is_an_error_even_if_other_versions_exist() {
    let cache = FakeNugetCache::pinned_with_newer_versions("missing-pin");
    std::fs::remove_dir_all(cache.0.join("microsoft.graphics.win2d").join("1.4.0")).unwrap();
    let message = build_nuget::resolve_pinned_packages(&cache.0)
        .expect_err("missing win2d pin")
        .to_string();
    assert!(
        message.contains("microsoft.graphics.win2d 1.4.0"),
        "{message}"
    );
    assert!(message.contains("restore-winui3.ps1"), "{message}");

    let cache = FakeNugetCache::pinned_with_newer_versions("missing-transitive");
    std::fs::remove_dir_all(
        cache
            .0
            .join("microsoft.windowsappsdk.foundation")
            .join("1.8.260203002"),
    )
    .unwrap();
    let message = build_nuget::resolve_pinned_packages(&cache.0)
        .expect_err("missing declared foundation")
        .to_string();
    assert!(
        message.contains("microsoft.windowsappsdk.foundation 1.8.260203002"),
        "{message}"
    );
}

#[test]
fn app_sdk_declaring_a_different_winui_is_rejected() {
    let cache = FakeNugetCache::pinned_with_newer_versions("winui-mismatch");
    cache.package(
        "microsoft.windowsappsdk",
        "1.8.260209005",
        &[
            ("Microsoft.WindowsAppSDK.Foundation", "[1.8.260203002]"),
            (
                "Microsoft.WindowsAppSDK.InteractiveExperiences",
                "[1.8.260125001]",
            ),
            ("Microsoft.WindowsAppSDK.WinUI", "[1.8.260224000]"),
        ],
    );
    let message = build_nuget::resolve_pinned_packages(&cache.0)
        .expect_err("winui mismatch")
        .to_string();
    assert!(message.contains("1.8.260224000"), "{message}");
}

#[test]
fn nuspec_dependency_versions_are_exact() {
    let nuspec = r#"<dependency id="A" version="[1.2.3]" /><dependency id="B" version="4.5" />"#;
    assert_eq!(
        build_nuget::nuspec_dependency_version(nuspec, "a"),
        Ok(Some("1.2.3".to_owned()))
    );
    assert_eq!(
        build_nuget::nuspec_dependency_version(nuspec, "B"),
        Ok(Some("4.5".to_owned()))
    );
    assert_eq!(
        build_nuget::nuspec_dependency_version(nuspec, "C"),
        Ok(None)
    );
    for range in ["[1.0,2.0)", "(1.0,)", "1.*", ""] {
        let nuspec = format!(r#"<dependency id="A" version="{range}" />"#);
        assert!(
            build_nuget::nuspec_dependency_version(&nuspec, "A").is_err(),
            "{range}"
        );
    }
    let conflicting = r#"<dependency id="A" version="[1]" /><dependency id="A" version="[2]" />"#;
    assert!(build_nuget::nuspec_dependency_version(conflicting, "A").is_err());
}

fn winui_package(cache: &FakeNugetCache) -> build_nuget::PinnedPackage {
    build_nuget::PinnedPackage {
        id: "microsoft.windowsappsdk.winui",
        version: "1.8.260204000".to_owned(),
        dir: cache
            .0
            .join("microsoft.windowsappsdk.winui")
            .join("1.8.260204000"),
    }
}

fn write_winmd(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let file = dir.join("Microsoft.UI.Xaml.winmd");
    std::fs::write(&file, b"winmd").unwrap();
    file
}

#[test]
fn metadata_overrides_must_resolve_into_the_pinned_version() {
    let cache = FakeNugetCache::pinned_with_newer_versions("override-ok");
    let package = winui_package(&cache);
    let pinned = write_winmd(&package.dir.join("metadata"));
    let resolved = build_nuget::validate_override(&pinned, &package).expect("pinned file");
    assert!(
        !resolved.to_string_lossy().starts_with(r"\\?\"),
        "{}",
        resolved.display()
    );
    assert_eq!(
        std::fs::canonicalize(&resolved).unwrap(),
        std::fs::canonicalize(&pinned).unwrap()
    );
    // Another cache location with the pinned `<id>/<version>` layout stays supported.
    let other = FakeNugetCache::new("override-other-location");
    let elsewhere = write_winmd(
        &other
            .0
            .join("Microsoft.WindowsAppSDK.WinUI")
            .join("1.8.260204000")
            .join("metadata"),
    );
    assert!(build_nuget::validate_override(&elsewhere, &package).is_ok());
}

#[test]
fn metadata_overrides_reject_lexical_pinned_segments_that_resolve_elsewhere() {
    let cache = FakeNugetCache::pinned_with_newer_versions("override-escape");
    let package = winui_package(&cache);
    write_winmd(&package.dir.join("metadata"));
    let newer = cache
        .0
        .join("microsoft.windowsappsdk.winui")
        .join("1.8.260224000");
    write_winmd(&newer.join("metadata"));

    // `<id>/<pinned>/../<newer>/...`: the pinned pair appears lexically but resolves away.
    let traversal = package
        .dir
        .join("..")
        .join("1.8.260224000")
        .join("metadata")
        .join("Microsoft.UI.Xaml.winmd");
    let message = build_nuget::validate_override(&traversal, &package)
        .expect_err("traversal")
        .to_string();
    assert!(message.contains("1.8.260224000"), "{message}");

    for (case, path) in [
        (
            "missing",
            package.dir.join("metadata").join("Missing.winmd"),
        ),
        ("directory", package.dir.join("metadata")),
        (
            "not winmd",
            package.dir.join("microsoft.windowsappsdk.winui.nuspec"),
        ),
        (
            "other version",
            newer.join("metadata").join("Microsoft.UI.Xaml.winmd"),
        ),
    ] {
        let message = build_nuget::validate_override(&path, &package)
            .expect_err(case)
            .to_string();
        assert!(message.contains("1.8.260204000"), "{case}: {message}");
    }
}

/// Creates a directory junction (no elevation or Developer Mode needed).
#[cfg(windows)]
fn junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .expect("run mklink /J");
    assert!(status.success(), "mklink /J {}", link.display());
}

#[cfg(windows)]
fn remove_junction(link: &Path) {
    // Removes only the junction itself, never the target's contents.
    std::fs::remove_dir(link).unwrap();
}

#[cfg(windows)]
#[test]
fn metadata_overrides_reject_junction_escapes() {
    let cache = FakeNugetCache::pinned_with_newer_versions("override-junction");
    let package = winui_package(&cache);
    let newer_metadata = cache
        .0
        .join("microsoft.windowsappsdk.winui")
        .join("1.8.260224000")
        .join("metadata");
    write_winmd(&newer_metadata);

    // A junction inside the pinned directory that points at another version.
    let inner = package.dir.join("escape");
    junction(&inner, &newer_metadata);
    let message = build_nuget::validate_override(&inner.join("Microsoft.UI.Xaml.winmd"), &package)
        .expect_err("junction inside pinned dir")
        .to_string();
    remove_junction(&inner);
    assert!(message.contains("1.8.260224000"), "{message}");

    // A pinned-looking `<id>/<version>` directory that is itself a junction to another version.
    let decoy = FakeNugetCache::new("override-decoy");
    let decoy_id = decoy.0.join("microsoft.windowsappsdk.winui");
    std::fs::create_dir_all(&decoy_id).unwrap();
    let decoy_version = decoy_id.join("1.8.260204000");
    junction(
        &decoy_version,
        &cache
            .0
            .join("microsoft.windowsappsdk.winui")
            .join("1.8.260224000"),
    );
    let result = build_nuget::validate_override(
        &decoy_version
            .join("metadata")
            .join("Microsoft.UI.Xaml.winmd"),
        &package,
    );
    remove_junction(&decoy_version);
    assert!(result.is_err(), "decoy version junction must be rejected");

    // Every junction removed: the newer package's files are still there.
    assert!(newer_metadata.join("Microsoft.UI.Xaml.winmd").is_file());
}

#[test]
fn verbatim_prefixes_are_removed_for_tool_arguments() {
    assert_eq!(
        build_nuget::without_verbatim_prefix(Path::new(r"\\?\C:\cache\a.winmd")),
        PathBuf::from(r"C:\cache\a.winmd")
    );
    assert_eq!(
        build_nuget::without_verbatim_prefix(Path::new(r"\\?\UNC\server\share\a.winmd")),
        PathBuf::from(r"\\server\share\a.winmd")
    );
    assert_eq!(
        build_nuget::without_verbatim_prefix(Path::new(r"C:\cache\a.winmd")),
        PathBuf::from(r"C:\cache\a.winmd")
    );
}

#[test]
fn pins_match_restore_script() {
    let script = include_str!("../../../tools/restore-winui3.ps1");
    for (name, (id, version)) in [
        ("Microsoft.WindowsAppSDK", build_nuget::WINDOWS_APP_SDK),
        (
            "Microsoft.WindowsAppSDK.WinUI",
            build_nuget::WINDOWS_APP_SDK_WINUI,
        ),
        ("Microsoft.Graphics.Win2D", build_nuget::WIN2D),
    ] {
        assert_eq!(id, name.to_ascii_lowercase());
        let reference = format!("PackageReference Include=\"{name}\" Version=\"{version}\"");
        assert!(script.contains(&reference), "{reference}");
    }
}
