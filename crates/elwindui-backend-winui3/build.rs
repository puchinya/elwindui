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
    if let Some(dir) = contract_dir {
        for entry in std::fs::read_dir(dir).expect("read Windows App SDK contracts").flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "winmd") {
                args.push("--in".to_owned());
                args.push(path.to_string_lossy().into_owned());
            }
        }
    }
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
