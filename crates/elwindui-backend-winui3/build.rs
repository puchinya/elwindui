//! Generates the WinUI 3 projection from the Windows App SDK and Windows SDK metadata.

#[cfg(target_os = "windows")]
fn main() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-env-changed=WINDOWS_APP_SDK_WINMD");
    println!("cargo:rerun-if-env-changed=WIN2D_WINMD");

    let app_sdk = std::env::var_os("WINDOWS_APP_SDK_WINMD")
        .map(PathBuf::from)
        .or_else(find_app_sdk_winmd)
        .expect(
            "Microsoft.UI.Xaml.winmd was not found. Restore Microsoft.WindowsAppSDK with NuGet, or set WINDOWS_APP_SDK_WINMD.",
        );
    assert!(app_sdk.is_file(), "WINDOWS_APP_SDK_WINMD is not a file: {}", app_sdk.display());

    let contract_dir = app_sdk.parent().and_then(std::path::Path::parent).and_then(|lib| {
        let mut candidates: Vec<_> = std::fs::read_dir(lib).ok()?.flatten().map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("uap10.0.")))
            .collect();
        candidates.sort();
        candidates.pop()
    });
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let interop_path = format!("{out_dir}/xaml_interop.rs");
    let out_path = format!("{out_dir}/bindings.rs");
    let interop_warnings = windows_bindgen::bindgen([
        "--in", "default", "--out", &interop_path, "--no-deps", "--filter",
        "Windows.UI.Xaml.Interop.TypeKind", "Windows.UI.Xaml.Interop.TypeName",
    ]);
    let mut args = vec![
        "--in".to_owned(),
        "default".to_owned(),
        "--in".to_owned(),
        app_sdk.to_string_lossy().into_owned(),
    ];
    for metadata in ["Microsoft.Foundation.winmd", "Microsoft.Graphics.winmd", "Microsoft.UI.winmd"] {
        if let Some(path) = find_package_contract_winmd(
            "microsoft.windowsappsdk.interactiveexperiences",
            metadata,
        ) {
            args.push("--in".to_owned());
            args.push(path.to_string_lossy().into_owned());
        }
    }
    for metadata in [
        "Microsoft.UI.Text.winmd",
        "Microsoft.Windows.ApplicationModel.Resources.winmd",
    ] {
        let path = app_sdk.with_file_name(metadata);
        if path.is_file() {
            args.push("--in".to_owned());
            args.push(path.to_string_lossy().into_owned());
        }
    }
    if let Some(resources) = find_package_winmd(
        "microsoft.windowsappsdk",
        "Microsoft.Windows.ApplicationModel.Resources.winmd",
    ) {
        args.push("--in".to_owned());
        args.push(resources.to_string_lossy().into_owned());
    }
    if let Some(webview2) = std::env::var_os("WEBVIEW2_WINMD") {
        args.push("--in".to_owned());
        args.push(PathBuf::from(webview2).to_string_lossy().into_owned());
    }
    if let Some(win2d) = std::env::var_os("WIN2D_WINMD")
        .map(PathBuf::from)
        .or_else(|| find_package_winmd("microsoft.graphics.win2d", "Microsoft.Graphics.Canvas.winmd"))
    {
        args.push("--in".to_owned());
        args.push(win2d.to_string_lossy().into_owned());
    }
    if let Some(dir) = contract_dir.clone() {
        for entry in std::fs::read_dir(dir).expect("read Windows App SDK contracts").flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "winmd") {
                args.push("--in".to_owned());
                args.push(path.to_string_lossy().into_owned());
            }
        }
    }
    // Captured before `--reference`/`--out`/`--filter` extend `args` further below — this is the
    // exact same winmd input set `windows-bindgen` uses, reused as `cppwinrt.exe`'s own `-input`
    // list in `build_cpp_app_host` so the two projections see identical metadata.
    let winmd_inputs: Vec<String> = args
        .iter()
        .zip(args.iter().skip(1))
        .filter(|(flag, _)| *flag == "--in")
        .map(|(_, value)| value.clone())
        .filter(|value| value != "default")
        .collect();

    args.extend([
        "--reference".to_owned(),
        "crate,full,Windows.UI.Xaml.Interop".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows".to_owned(),
        "--out".to_owned(), out_path.clone(), "--filter".to_owned(),
        "Microsoft.UI.Xaml.IApplicationOverrides".to_owned(),
        "Microsoft.UI.Xaml.LaunchActivatedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Markup.IXamlMetadataProvider".to_owned(),
        "Microsoft.UI.Xaml.Markup.IXamlType".to_owned(),
        "Microsoft.UI.Xaml.Markup.XmlnsDefinition".to_owned(),
        "Microsoft.UI.Xaml.XamlTypeInfo.XamlControlsXamlMetaDataProvider".to_owned(),
        "Microsoft.UI.Dispatching.DispatcherQueue".to_owned(),
        "Microsoft.UI.Dispatching.DispatcherQueueController".to_owned(),
        "Microsoft.UI.Dispatching.DispatcherQueueHandler".to_owned(),
        "Microsoft.UI.Xaml.Application".to_owned(),
        "Microsoft.UI.Xaml.ApplicationInitializationCallback".to_owned(),
        "Microsoft.UI.Xaml.ResourceDictionary".to_owned(),
        "Microsoft.UI.Xaml.Controls.XamlControlsResources".to_owned(),
        "Microsoft.Windows.ApplicationModel.Resources.IResourceManager".to_owned(),
        "Microsoft.UI.Input.InputKeyboardSource".to_owned(),
        "Microsoft.UI.Input.InputObject".to_owned(),
        "Microsoft.UI.Windowing.AppWindow".to_owned(),
        "Microsoft.UI.Xaml.DependencyObject".to_owned(),
        "Microsoft.UI.Xaml.FrameworkElement".to_owned(),
        "Microsoft.UI.Xaml.RoutedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.SizeChangedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.TextAlignment".to_owned(),
        "Microsoft.UI.Xaml.TextWrapping".to_owned(),
        "Microsoft.UI.Xaml.HorizontalAlignment".to_owned(),
        "Microsoft.UI.Xaml.VerticalAlignment".to_owned(),
        "Microsoft.UI.Xaml.UIElement".to_owned(),
        "Microsoft.UI.Xaml.Window".to_owned(),
        "Microsoft.UI.Xaml.WindowEventArgs".to_owned(),
        "Windows.Foundation.Collections.IVectorChangedEventArgs".to_owned(),
        "Windows.Foundation.Collections.CollectionChange".to_owned(),
        "Microsoft.UI.Xaml.Controls.UserControl".to_owned(),
        "Microsoft.UI.Xaml.Controls.Button".to_owned(),
        "Microsoft.UI.Xaml.Controls.Canvas".to_owned(),
        "Microsoft.UI.Xaml.Controls.ContentControl".to_owned(),
        "Microsoft.UI.Xaml.Controls.Control".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuFlyoutItem".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuFlyoutItemBase".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuBar".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuBarItem".to_owned(),
        // `PasswordBox.PasswordChanged`'s event type is the same plain `RoutedEventHandler`
        // `Button.Click`/`TabView` already use (unlike `TextBox.TextChanged`, which has its own
        // `TextChangedEventHandler`) — no separate event-args/handler type needs listing here. If
        // that turns out wrong once this actually builds on Windows, `windows-bindgen`'s own error
        // will name the missing type to add.
        "Microsoft.UI.Xaml.Controls.PasswordBox".to_owned(),
        "Microsoft.UI.Xaml.Controls.PasswordRevealMode".to_owned(),
        "Microsoft.UI.Xaml.Controls.ScrollViewer".to_owned(),
        "Microsoft.UI.Xaml.Controls.ScrollMode".to_owned(),
        "Microsoft.UI.Xaml.Controls.ListViewItem".to_owned(),
        "Microsoft.UI.Xaml.Controls.Panel".to_owned(),
        "Microsoft.UI.Xaml.Controls.UIElementCollection".to_owned(),
        "Microsoft.UI.Xaml.Controls.SelectionChangedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.SelectionChangedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabView".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewCloseButtonOverlayMode".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewItem".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewTabCloseRequestedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewWidthMode".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextBlock".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextBox".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextChangedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextChangedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.ButtonBase".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.SelectorItem".to_owned(),
        "Microsoft.UI.Xaml.Input.CharacterReceivedRoutedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyRoutedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyboardAccelerator".to_owned(),
        "Microsoft.UI.Xaml.Media.Brush".to_owned(),
        "Microsoft.UI.Xaml.Media.SolidColorBrush".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Ellipse".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Line".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Rectangle".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Shape".to_owned(),
        "Microsoft.Graphics.Canvas.UI.Xaml.CanvasControl".to_owned(),
        "Microsoft.Graphics.Canvas.UI.Xaml.CanvasDrawEventArgs".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasDrawingSession".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasActiveLayer".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasBitmap".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasAlphaMode".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasBlend".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasImageInterpolation".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasEdgeBehavior".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasAntialiasing".to_owned(),
        "Microsoft.Graphics.Canvas.ICanvasResourceCreator".to_owned(),
        "Microsoft.Graphics.Canvas.Brushes.ICanvasBrush".to_owned(),
        "Microsoft.Graphics.Canvas.Brushes.CanvasGradientStop".to_owned(),
        "Microsoft.Graphics.Canvas.Brushes.CanvasSolidColorBrush".to_owned(),
        "Microsoft.Graphics.Canvas.Brushes.CanvasImageBrush".to_owned(),
        "Microsoft.Graphics.Canvas.Brushes.CanvasLinearGradientBrush".to_owned(),
        "Microsoft.Graphics.Canvas.Brushes.CanvasRadialGradientBrush".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasPathBuilder".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasGeometry".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasFigureFill".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasFigureLoop".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasFilledRegionDetermination".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasSweepDirection".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasArcSize".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasStrokeStyle".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasCapStyle".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasLineJoin".to_owned(),
        "Microsoft.Graphics.Canvas.Geometry.CanvasDashStyle".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasCommandList".to_owned(),
        "Microsoft.Graphics.Canvas.Effects.BlendEffect".to_owned(),
        "Microsoft.Graphics.Canvas.Effects.BlendEffectMode".to_owned(),
        "Microsoft.Graphics.Canvas.Effects.LuminanceToAlphaEffect".to_owned(),
        "--implement".to_owned(),
    ]);
    let warnings = windows_bindgen::bindgen(&args);
    let generated = std::fs::read_to_string(&out_path).expect("read generated WinUI bindings");
    std::fs::write(&out_path, generated.replacen("#![allow(", "#[allow(", 1))
        .expect("write generated WinUI bindings");
    let interop = std::fs::read_to_string(&interop_path).expect("read generated XAML interop bindings");
    std::fs::write(&interop_path, interop.replacen("#![allow(", "#[allow(", 1))
        .expect("write generated XAML interop bindings");
    copy_win2d_runtime(&out_dir);
    generate_resources_pri(&out_dir);
    build_cpp_app_host(&out_dir, &winmd_inputs);
    if !warnings.is_empty() || !interop_warnings.is_empty() {
        println!("cargo:warning=WinUI binding generation omitted {} unsupported metadata member(s)", warnings.len());
    }
    println!("cargo:rustc-env=ELWINDUI_WINUI3_BINDINGS={out_path}");
}

#[cfg(target_os = "windows")]
fn find_app_sdk_winmd() -> Option<std::path::PathBuf> {
    find_package_metadata_winmd("microsoft.windowsappsdk.winui", "Microsoft.UI.Xaml.winmd")
        .or_else(|| find_package_winmd("microsoft.windowsappsdk", "Microsoft.UI.Xaml.winmd"))
}

#[cfg(target_os = "windows")]
fn find_package_metadata_winmd(package: &str, filename: &str) -> Option<std::path::PathBuf> {
    let root = std::env::var_os("NUGET_PACKAGES")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|profile| std::path::PathBuf::from(profile).join(".nuget\\packages")))?
        .join(package);
    let mut candidates = Vec::new();
    for version in std::fs::read_dir(root).ok()?.flatten() {
        let winmd = version.path().join("metadata").join(filename);
        if winmd.is_file() {
            candidates.push(winmd);
        }
    }
    candidates.sort();
    candidates.pop()
}

#[cfg(target_os = "windows")]
fn find_package_contract_winmd(package: &str, filename: &str) -> Option<std::path::PathBuf> {
    let root = std::env::var_os("NUGET_PACKAGES")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|profile| std::path::PathBuf::from(profile).join(".nuget\\packages")))?
        .join(package);
    let mut candidates = Vec::new();
    for version in std::fs::read_dir(root).ok()?.flatten() {
        let metadata = version.path().join("metadata");
        for contract in std::fs::read_dir(metadata).ok().into_iter().flatten().flatten() {
            let winmd = contract.path().join(filename);
            if winmd.is_file() {
                candidates.push(winmd);
            }
        }
    }
    candidates.sort();
    candidates.pop()
}

#[cfg(target_os = "windows")]
fn copy_win2d_runtime(out_dir: &str) {
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "win-x86",
        Ok("aarch64") => "win-arm64",
        _ => "win-x64",
    };
    let root = std::env::var_os("NUGET_PACKAGES")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|profile| std::path::PathBuf::from(profile).join(".nuget\\packages")))
        .expect("NUGET_PACKAGES or USERPROFILE is required to locate Win2D runtime");
    let mut candidates = Vec::new();
    let package = root.join("microsoft.graphics.win2d");
    for version in std::fs::read_dir(package).expect("read Win2D NuGet package").flatten() {
        let dll = version.path().join("runtimes").join(arch).join("native").join("Microsoft.Graphics.Canvas.dll");
        if dll.is_file() {
            candidates.push(dll);
        }
    }
    candidates.sort();
    let Some(source) = candidates.pop() else { panic!("Microsoft.Graphics.Canvas.dll was not found for {arch}"); };
    let profile_dir = std::path::Path::new(out_dir).ancestors().nth(3).expect("target profile directory");
    let target = profile_dir.join("Microsoft.Graphics.Canvas.dll");
    std::fs::copy(&source, &target).expect("copy Microsoft.Graphics.Canvas.dll beside application binary");
    println!("cargo:rerun-if-changed={}", source.display());

    let mut bootstrap_candidates = Vec::new();
    let package = root.join("microsoft.windowsappsdk");
    for version in std::fs::read_dir(package).expect("read Windows App SDK NuGet package").flatten() {
        let dll = version.path().join("runtimes").join(arch).join("native").join("Microsoft.WindowsAppRuntime.Bootstrap.dll");
        if dll.is_file() {
            bootstrap_candidates.push(dll);
        }
    }
    bootstrap_candidates.sort();
    let Some(source) = bootstrap_candidates.pop() else {
        panic!("Microsoft.WindowsAppRuntime.Bootstrap.dll was not found for {arch}");
    };
    let target = profile_dir.join("Microsoft.WindowsAppRuntime.Bootstrap.dll");
    std::fs::copy(&source, &target)
        .expect("copy Microsoft.WindowsAppRuntime.Bootstrap.dll beside application binary");
    println!("cargo:rerun-if-changed={}", source.display());
}

/// An unpackaged Windows App SDK process cannot resolve *any* `ms-appx://` resource — including a
/// framework package's own bundled resources, such as WinUI 3's default control theme
/// (`ms-appx:///Microsoft.UI.Xaml/Themes/themeresources.xaml`, consulted by
/// `install_default_control_resources` in `src/inner.rs`) — unless a `resources.pri` sits next to
/// its own executable to bootstrap MRT resource-context resolution. This generates a minimal one
/// (indexing none of this crate's own resources — it exists purely to make that resolution
/// possible at all) and copies it beside the built exe, the same way `copy_win2d_runtime` places
/// `Microsoft.Graphics.Canvas.dll` there. Requires `makepri.exe` from the Windows SDK, i.e.
/// `tools/setup-vs-env.ps1` sourced first — same precondition the whole crate already has for MSVC.
#[cfg(target_os = "windows")]
fn generate_resources_pri(out_dir: &str) {
    let makepri = find_makepri()
        .expect("makepri.exe was not found; source tools/setup-vs-env.ps1 first");
    let profile_dir = std::path::Path::new(out_dir).ancestors().nth(3).expect("target profile directory");
    let pri_root = std::path::Path::new(out_dir).join("resources_pri");
    std::fs::create_dir_all(&pri_root).expect("create resources.pri project root");

    let config = pri_root.join("priconfig.xml");
    let status = std::process::Command::new(&makepri)
        .arg("createconfig")
        .arg("/cf").arg(&config)
        .arg("/dq").arg("en-US")
        .arg("/pv").arg("10.0.0")
        .arg("/o")
        .status()
        .expect("run makepri.exe createconfig");
    assert!(status.success(), "makepri.exe createconfig failed");

    let generated = pri_root.join("resources.pri");
    let status = std::process::Command::new(&makepri)
        .arg("new")
        .arg("/pr").arg(&pri_root)
        .arg("/cf").arg(&config)
        .arg("/of").arg(&generated)
        .arg("/o")
        .status()
        .expect("run makepri.exe new");
    assert!(status.success(), "makepri.exe new failed");

    std::fs::copy(&generated, profile_dir.join("resources.pri"))
        .expect("copy resources.pri beside application binary");
    // `cargo test`/`cargo bench` binaries run from `target/<profile>/deps/`, not
    // `target/<profile>/` itself — MRT resource-context resolution looks beside the actual running
    // executable, so a Windows-only test that touches any native control (anything going through
    // `install_default_control_resources`) needs its own copy here too, or it fails with
    // `Cannot locate resource from 'ms-appx:///Microsoft.UI.Xaml/Themes/themeresources.xaml'`
    // despite the example binaries (which do live directly in `target/<profile>/`) working fine.
    let deps_dir = profile_dir.join("deps");
    std::fs::create_dir_all(&deps_dir).expect("create target/<profile>/deps directory");
    std::fs::copy(&generated, deps_dir.join("resources.pri"))
        .expect("copy resources.pri beside test binaries");
}

/// Generates a C++/WinRT projection (via `cppwinrt.exe`) for just enough of the WinUI 3 surface to
/// host `Application`, and compiles `cpp/app_host.cpp` against it — see that file's own doc comment
/// (and `src/composed_application.rs`'s) for why this exists at all (microsoft/windows-rs#3404).
#[cfg(target_os = "windows")]
fn build_cpp_app_host(out_dir: &str, winmd_inputs: &[String]) {
    let cppwinrt = find_sdk_tool("cppwinrt.exe")
        .expect("cppwinrt.exe was not found; source tools/setup-vs-env.ps1 first");
    let projection_dir = std::path::Path::new(out_dir).join("cppwinrt_include");

    let mut args: Vec<String> = vec!["-input".to_owned(), "sdk+".to_owned()];
    for winmd in winmd_inputs {
        args.push("-input".to_owned());
        args.push(winmd.clone());
    }
    // `Microsoft.UI.Xaml.winmd`'s own `IWebView2` interface references WebView2's winmd even
    // though this shim never touches WebView2 — cppwinrt validates the whole input database
    // up front, so the reference has to resolve even when `-exclude` drops the type from output.
    if let Some(webview2) = find_package_lib_winmd("microsoft.web.webview2", "Microsoft.Web.WebView2.Core.winmd") {
        args.push("-input".to_owned());
        args.push(webview2.to_string_lossy().into_owned());
    }
    for namespace in [
        "Microsoft.UI",
        "Windows.Foundation",
        "Windows.Foundation.Collections",
        "Windows.UI",
        "Windows.System",
    ] {
        args.push("-include".to_owned());
        args.push(namespace.to_owned());
    }
    // Excludes types this shim never touches whose own metadata references external winmd files
    // this build doesn't provide (e.g. WebView2's own winmd, only needed by real WebView2 users).
    for excluded in ["Microsoft.UI.Xaml.Controls.WebView2", "Microsoft.UI.Xaml.Controls.IWebView2"] {
        args.push("-exclude".to_owned());
        args.push(excluded.to_owned());
    }
    args.push("-output".to_owned());
    args.push(projection_dir.to_string_lossy().into_owned());
    args.push("-overwrite".to_owned());

    let status = std::process::Command::new(&cppwinrt).args(&args).status().expect("run cppwinrt.exe");
    assert!(status.success(), "cppwinrt.exe failed generating the C++/WinRT projection");

    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .file("cpp/app_host.cpp")
        .include(&projection_dir)
        .flag_if_supported("/await:strict")
        .flag_if_supported("/EHsc")
        .flag_if_supported("/utf-8")
        .compile("elwindui_winui3_app_host");

    println!("cargo:rustc-link-lib=WindowsApp");
    println!("cargo:rerun-if-changed=cpp/app_host.cpp");
}

/// Looks for `<nuget_packages>/<package>/<version>/lib/<filename>` (the WebView2 package's own
/// layout — a flat `lib/` with no per-TFM subfolder, unlike `find_package_winmd`'s `lib/<target>/`).
#[cfg(target_os = "windows")]
fn find_package_lib_winmd(package: &str, filename: &str) -> Option<std::path::PathBuf> {
    let root = std::env::var_os("NUGET_PACKAGES")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|profile| std::path::PathBuf::from(profile).join(".nuget\\packages")))?
        .join(package);
    let mut candidates = Vec::new();
    for version in std::fs::read_dir(root).ok()?.flatten() {
        let winmd = version.path().join("lib").join(filename);
        if winmd.is_file() {
            candidates.push(winmd);
        }
    }
    candidates.sort();
    candidates.pop()
}

#[cfg(target_os = "windows")]
fn find_sdk_tool(name: &str) -> Option<std::path::PathBuf> {
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    if let (Ok(sdk_dir), Ok(sdk_version)) =
        (std::env::var("WindowsSdkDir"), std::env::var("WindowsSDKVersion"))
    {
        let candidate = std::path::Path::new(&sdk_dir)
            .join("bin")
            .join(sdk_version.trim_end_matches('\\'))
            .join(arch)
            .join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let bin_root = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates: Vec<_> = std::fs::read_dir(bin_root)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(arch).join(name))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}

#[cfg(target_os = "windows")]
fn find_makepri() -> Option<std::path::PathBuf> {
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    if let (Ok(sdk_dir), Ok(sdk_version)) =
        (std::env::var("WindowsSdkDir"), std::env::var("WindowsSDKVersion"))
    {
        let candidate = std::path::Path::new(&sdk_dir)
            .join("bin")
            .join(sdk_version.trim_end_matches('\\'))
            .join(arch)
            .join("makepri.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let bin_root = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates: Vec<_> = std::fs::read_dir(bin_root)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(arch).join("makepri.exe"))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}

#[cfg(target_os = "windows")]
fn find_package_winmd(package: &str, filename: &str) -> Option<std::path::PathBuf> {
    let root = std::env::var_os("NUGET_PACKAGES")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|profile| std::path::PathBuf::from(profile).join(".nuget\\packages")))?
        .join(package);
    let mut candidates = Vec::new();
    for version in std::fs::read_dir(root).ok()?.flatten() {
        let lib = version.path().join("lib");
        for target in std::fs::read_dir(lib).ok().into_iter().flatten().flatten() {
            let winmd = target.path().join(filename);
            if winmd.is_file() {
                candidates.push(winmd);
            }
        }
    }
    candidates.sort();
    candidates.pop()
}

#[cfg(not(target_os = "windows"))]
fn main() {}
