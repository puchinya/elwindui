//! Generates the WinUI 3 projection from the Windows App SDK and Windows SDK metadata, and links
//! the native C++/WinRT application host.
//!
//! The native host is obtained in one of two explicitly selected modes (Issue #294, see
//! `docs/design/backends/winui3_backend_design.md`, "Native artifact manufacturing"):
//!
//! - prebuilt (default; `ELWINDUI_WINUI3_BUILD_NATIVE` unset or `0`): link/deploy the checked-in
//!   `native/prebuilt/<TARGET>/` artifacts. No MIDL, C++/WinRT, MSVC compiler or makepri run.
//! - source (`ELWINDUI_WINUI3_BUILD_NATIVE=1`): run the MIDL -> cppwinrt -> MSVC and makepri
//!   pipeline from `cpp/`, optionally exporting the outputs to
//!   `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR/<TARGET>/`.
//!
//! The Rust `windows-bindgen` projection and the Win2D / Windows App Runtime bootstrap DLL
//! deployment are common to both modes.

#[cfg(target_os = "windows")]
#[allow(dead_code)]
mod build_nuget;
#[cfg(target_os = "windows")]
#[allow(dead_code)]
mod build_support;

#[cfg(target_os = "windows")]
fn main() {
    use build_support::{
        BUILD_NATIVE_ENV, NativeBuildMode, PREBUILT_EXPORT_DIR_ENV, parse_build_mode,
        resolve_export_dir,
    };

    println!("cargo:rerun-if-env-changed=WINDOWS_APP_SDK_WINMD");
    println!("cargo:rerun-if-env-changed=WIN2D_WINMD");
    println!("cargo:rerun-if-env-changed=WEBVIEW2_WINMD");
    println!("cargo:rerun-if-env-changed=NUGET_PACKAGES");
    println!("cargo:rerun-if-env-changed={BUILD_NATIVE_ENV}");
    println!("cargo:rerun-if-env-changed={PREBUILT_EXPORT_DIR_ENV}");

    let mode_value = std::env::var_os(BUILD_NATIVE_ENV).map(|v| v.to_string_lossy().into_owned());
    let mode = parse_build_mode(mode_value.as_deref()).unwrap_or_else(|error| panic!("{error}"));
    let export_value =
        std::env::var_os(PREBUILT_EXPORT_DIR_ENV).map(|v| v.to_string_lossy().into_owned());
    // Validated (never created) here, before any build work: an export destination inside or
    // above the tracked prebuilt tree is rejected without touching the file system.
    let export_dir = resolve_export_dir(mode, export_value.as_deref())
        .unwrap_or_else(|error| panic!("{error}"))
        .map(|dir| validated_export_destination(&dir));
    let target = std::env::var("TARGET").expect("TARGET");
    // Resolved up front so a missing/unsupported prebuilt bundle fails before binding generation.
    let prebuilt_dir = match mode {
        NativeBuildMode::Prebuilt => Some(resolve_prebuilt_dir(&target)),
        NativeBuildMode::Source => None,
    };

    // Exactly the pinned packages (and the transitive versions they declare), never another
    // version that happens to be in the NuGet cache. See `build_nuget.rs`.
    let packages = build_nuget::resolve_pinned_packages(&nuget_packages_root())
        .unwrap_or_else(|error| panic!("{error}"));
    let app_sdk = pinned_override("WINDOWS_APP_SDK_WINMD", &packages.winui).unwrap_or_else(|| {
        packages
            .winui
            .dir
            .join("metadata")
            .join("Microsoft.UI.Xaml.winmd")
    });
    assert!(
        app_sdk.is_file(),
        "WINDOWS_APP_SDK_WINMD is not a file: {}",
        app_sdk.display()
    );

    let contract_dir = app_sdk
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(|lib| {
            let mut candidates: Vec<_> = std::fs::read_dir(lib)
                .ok()?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_dir()
                        && path
                            .file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with("uap10.0."))
                })
                .collect();
            candidates.sort();
            candidates.pop()
        });
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let interop_path = format!("{out_dir}/xaml_interop.rs");
    let out_path = format!("{out_dir}/bindings.rs");
    let interop_warnings = windows_bindgen::bindgen([
        "--in",
        "default",
        "--out",
        &interop_path,
        "--no-deps",
        "--filter",
        "Windows.UI.Xaml.Interop.TypeKind",
        "Windows.UI.Xaml.Interop.TypeName",
    ]);
    let mut args = vec![
        "--in".to_owned(),
        "default".to_owned(),
        "--in".to_owned(),
        app_sdk.to_string_lossy().into_owned(),
    ];
    for metadata in [
        "Microsoft.Foundation.winmd",
        "Microsoft.Graphics.winmd",
        "Microsoft.UI.winmd",
    ] {
        if let Some(path) = find_contract_winmd(&packages.interactive_experiences.dir, metadata) {
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
    // Windows App SDK 1.8 ships this contract in the separate Foundation package. Keep the
    // runtime-package lookup (of the same pinned Windows App SDK) as a fallback for its older
    // layout, but prefer the Foundation metadata directory so the exact IResourceManager filter
    // below is available to the no-deps projection.
    let resources_winmd = packages
        .foundation
        .dir
        .join("metadata")
        .join("Microsoft.Windows.ApplicationModel.Resources.winmd");
    if let Some(resources) = Some(resources_winmd)
        .filter(|path| path.is_file())
        .or_else(|| {
            find_lib_winmd(
                &packages.app_sdk.dir,
                "Microsoft.Windows.ApplicationModel.Resources.winmd",
            )
        })
    {
        args.push("--in".to_owned());
        args.push(resources.to_string_lossy().into_owned());
    }
    if let Some(webview2) = pinned_override("WEBVIEW2_WINMD", &packages.webview2) {
        args.push("--in".to_owned());
        args.push(webview2.to_string_lossy().into_owned());
    }
    if let Some(win2d) = pinned_override("WIN2D_WINMD", &packages.win2d)
        .or_else(|| find_lib_winmd(&packages.win2d.dir, "Microsoft.Graphics.Canvas.winmd"))
    {
        args.push("--in".to_owned());
        args.push(win2d.to_string_lossy().into_owned());
    }
    if let Some(dir) = contract_dir.clone() {
        for entry in std::fs::read_dir(dir)
            .expect("read Windows App SDK contracts")
            .flatten()
        {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "winmd")
            {
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
        // `windows-bindgen`'s automatic dependency references route every
        // `Windows.Foundation.Collections` type through `windows-collections`. That companion
        // crate owns the stock IIterable/IVector types, but observable collections remain in the
        // `windows` crate; treating the whole namespace as `windows-collections` makes WinUI's
        // `ItemsControl.Items` unprojectable. Disable the automatic references and reproduce them
        // with the two split namespaces (plus the equivalent Quaternion/numerics split) explicitly.
        "--no-deps".to_owned(),
        "--reference".to_owned(),
        "crate,full,Windows.UI.Xaml.Interop".to_owned(),
        "--reference".to_owned(),
        "windows_future,flat,Windows.Foundation.Async*".to_owned(),
        "--reference".to_owned(),
        "windows_future,flat,Windows.Foundation.IAsync*".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows.Foundation.Collections.IObservableVector".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows.Foundation.Collections.IVectorChangedEventArgs".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows.Foundation.Collections.VectorChangedEventHandler".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows.Foundation.Collections.CollectionChange".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows.Foundation.Collections.IPropertySet".to_owned(),
        "--reference".to_owned(),
        "windows_collections,flat,Windows.Foundation.Collections".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows.Foundation.Numerics.Quaternion".to_owned(),
        "--reference".to_owned(),
        "windows_numerics,flat,Windows.Foundation.Numerics".to_owned(),
        "--reference".to_owned(),
        "windows,skip-root,Windows".to_owned(),
        "--out".to_owned(),
        out_path.clone(),
        "--filter".to_owned(),
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
        "Microsoft.UI.Input.PointerPoint".to_owned(),
        "Microsoft.UI.Input.PointerPointProperties".to_owned(),
        "Microsoft.UI.Input.PointerUpdateKind".to_owned(),
        "Microsoft.UI.Content.ContentCoordinateConverter".to_owned(),
        "Microsoft.UI.Content.ContentIsland".to_owned(),
        "Microsoft.UI.Content.ContentIslandEnvironment".to_owned(),
        "Microsoft.UI.WindowId".to_owned(),
        "Microsoft.UI.Windowing.AppWindow".to_owned(),
        "Microsoft.UI.Windowing.AppWindowChangedEventArgs".to_owned(),
        "Microsoft.UI.Windowing.AppWindowPresenter".to_owned(),
        "Microsoft.UI.Windowing.AppWindowClosingEventArgs".to_owned(),
        "Microsoft.UI.Windowing.DisplayArea".to_owned(),
        "Microsoft.UI.Windowing.DisplayAreaFallback".to_owned(),
        "Microsoft.UI.Windowing.OverlappedPresenter".to_owned(),
        "Microsoft.UI.Xaml.DependencyObject".to_owned(),
        "Microsoft.UI.Xaml.DependencyProperty".to_owned(),
        "Microsoft.UI.Xaml.ElementTheme".to_owned(),
        "Microsoft.UI.Xaml.FocusState".to_owned(),
        "Microsoft.UI.Xaml.FrameworkElement".to_owned(),
        "Microsoft.UI.Xaml.Input.GettingFocusEventArgs".to_owned(),
        "Microsoft.UI.Xaml.RoutedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.SizeChangedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.TextAlignment".to_owned(),
        "Microsoft.UI.Xaml.TextWrapping".to_owned(),
        "Microsoft.UI.Xaml.HorizontalAlignment".to_owned(),
        "Microsoft.UI.Xaml.VerticalAlignment".to_owned(),
        "Microsoft.UI.Xaml.UIElement".to_owned(),
        "Microsoft.UI.Xaml.Automation.Peers.AccessibilityView".to_owned(),
        "Microsoft.UI.Xaml.Automation.AutomationProperties".to_owned(),
        "Microsoft.UI.Xaml.XamlRoot".to_owned(),
        "Microsoft.UI.Xaml.Window".to_owned(),
        "Microsoft.UI.Xaml.WindowEventArgs".to_owned(),
        // Issue #225: the top-level `Window`'s own `SizeChanged` (WinUI effective/logical pixels,
        // matching XAML layout units directly) is the authoritative content-host viewport signal —
        // the root `TreeHost` `Canvas`'s own `SizeChanged` does not reliably fire as a
        // bootstrap signal for a plain `Window.Content` (see `docs/design/backends/
        // winui3_backend_design.md`, "Native hosting and layout").
        "Microsoft.UI.Xaml.WindowSizeChangedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.UserControl".to_owned(),
        "Microsoft.UI.Xaml.Controls.Button".to_owned(),
        "Microsoft.UI.Xaml.Controls.Canvas".to_owned(),
        "Microsoft.UI.Xaml.Controls.ContentControl".to_owned(),
        "Microsoft.UI.Xaml.Controls.Control".to_owned(),
        "Microsoft.UI.Xaml.Controls.IControlStatics".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuFlyout".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuFlyoutItem".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuFlyoutItemBase".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuBar".to_owned(),
        "Microsoft.UI.Xaml.Controls.MenuBarItem".to_owned(),
        // `MenuItem.icon` (Issue #170, docs/design/runtime/icon_source_design.md): `MenuFlyoutItem
        // .Icon` is typed `IconElement`; `SystemIcon` maps to a `SymbolIcon` (built from the
        // `Symbol` enum), a user `ImageSource::Raster` maps to an `ImageIcon` wrapping a
        // `BitmapImage` fed from the same `InMemoryRandomAccessStream`/`DataWriter` byte-stream
        // pattern `render/composition/cache.rs`'s `surface_for` already uses.
        "Microsoft.UI.Xaml.Controls.IconElement".to_owned(),
        "Microsoft.UI.Xaml.Controls.SymbolIcon".to_owned(),
        "Microsoft.UI.Xaml.Controls.Symbol".to_owned(),
        "Microsoft.UI.Xaml.Controls.ImageIcon".to_owned(),
        "Microsoft.UI.Xaml.Media.ImageSource".to_owned(),
        "Microsoft.UI.Xaml.Media.Imaging.BitmapImage".to_owned(),
        "Microsoft.UI.Xaml.Media.Imaging.BitmapSource".to_owned(),
        "Microsoft.UI.Xaml.Media.GeneralTransform".to_owned(),
        "Microsoft.UI.Xaml.Media.Transform".to_owned(),
        "Microsoft.UI.Xaml.Media.CompositeTransform".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.Popup".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.FlyoutBase".to_owned(),
        "Microsoft.UI.Xaml.Input.ContextRequestedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Input.RightTappedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Input.RightTappedRoutedEventArgs".to_owned(),
        // `PasswordBox.PasswordChanged`'s event type is the same plain `RoutedEventHandler`
        // `Button.Click`/`TabView` already use (unlike `TextBox.TextChanged`, which has its own
        // `TextChangedEventHandler`) — no separate event-args/handler type needs listing here. If
        // that turns out wrong once this actually builds on Windows, `windows-bindgen`'s own error
        // will name the missing type to add.
        "Microsoft.UI.Xaml.Controls.PasswordBox".to_owned(),
        "Microsoft.UI.Xaml.Controls.PasswordRevealMode".to_owned(),
        "Microsoft.UI.Xaml.Controls.ScrollViewer".to_owned(),
        "Microsoft.UI.Xaml.Controls.ScrollMode".to_owned(),
        // `Button.role`/`is_default`/`tooltip` (`elwindui_core::ui::ButtonRole`,
        // `NativeControl::set_tooltip`). `Style` is looked up by key from `Application.Resources`
        // for `AccentButtonStyle`; `Brush` for the destructive foreground; `KeyboardAccelerator` +
        // `VirtualKey` stand in for the `IsDefault` that WinUI 3's `Button` (unlike
        // `ContentDialog`'s) does not have. `ToolTipService` is the attached property both the
        // tooltip setter and every other native leaf go through.
        "Microsoft.UI.Xaml.Application".to_owned(),
        "Microsoft.UI.Xaml.Style".to_owned(),
        "Microsoft.UI.Xaml.Media.Brush".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyboardAccelerator".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyboardAcceleratorInvokedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.ToolTipService".to_owned(),
        "Microsoft.UI.Xaml.Controls.ToolTip".to_owned(),
        // `CheckBox`/`RadioButton`/`ToggleSwitch` (selection controls). `CheckBox.IsChecked`/
        // `RadioButton.IsChecked` are `Windows.Foundation.IReference<bool>` (nullable — `null`
        // means indeterminate for `CheckBox`), set via `PropertyValue::CreateBoolean(..)?.cast()?`
        // the same way `win2d.rs`'s existing `IReference<Rect>` usage already does.
        "Microsoft.UI.Xaml.Controls.CheckBox".to_owned(),
        "Microsoft.UI.Xaml.Controls.RadioButton".to_owned(),
        "Microsoft.UI.Xaml.Controls.ToggleSwitch".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.ToggleButton".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.Thumb".to_owned(),
        "Microsoft.UI.Xaml.Controls.Grid".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.DragStartedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.DragStartedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.DragDeltaEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.DragDeltaEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.DragCompletedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.DragCompletedEventHandler".to_owned(),
        // `Dropdown`. `ComboBox.Items` is an `IObservableVector<IInspectable>` populated with
        // plain `HSTRING`s (via `PropertyValue::CreateString`), the same way `win2d.rs`'s existing
        // `IReference<..>` usage already boxes primitives for a WinRT collection.
        "Microsoft.UI.Xaml.Controls.ComboBox".to_owned(),
        "Microsoft.UI.Xaml.Controls.ItemsControl".to_owned(),
        "Microsoft.UI.Xaml.Controls.ItemCollection".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.Selector".to_owned(),
        // `Slider`. `Minimum`/`Maximum`/`Value` are plain `f64`; `ValueChanged` carries the new
        // value directly in its args (no need to read `Value` back off the sender).
        "Microsoft.UI.Xaml.Controls.Slider".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.RangeBase".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.RangeBaseValueChangedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.RangeBaseValueChangedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Controls.ListViewItem".to_owned(),
        "Microsoft.UI.Xaml.Controls.Panel".to_owned(),
        "Microsoft.UI.Xaml.Controls.IPanelStatics".to_owned(),
        "Microsoft.UI.Xaml.Controls.UIElementCollection".to_owned(),
        "Microsoft.UI.Xaml.Controls.SelectionChangedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.SelectionChangedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabView".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewCloseButtonOverlayMode".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewItem".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewTabCloseRequestedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.TabViewWidthMode".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextBlock".to_owned(),
        "Microsoft.UI.Xaml.Controls.ITextBlockStatics".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextBox".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextChangedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Controls.TextChangedEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.ButtonBase".to_owned(),
        "Microsoft.UI.Xaml.Controls.Primitives.SelectorItem".to_owned(),
        "Microsoft.UI.Xaml.Input.CharacterReceivedRoutedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyRoutedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Input.KeyboardAccelerator".to_owned(),
        "Microsoft.UI.Xaml.Input.PointerEventHandler".to_owned(),
        "Microsoft.UI.Xaml.Input.Pointer".to_owned(),
        "Microsoft.UI.Xaml.Input.PointerRoutedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Media.Brush".to_owned(),
        "Microsoft.UI.Xaml.Media.CompositionTarget".to_owned(),
        "Microsoft.UI.Xaml.Media.LoadedImageSurface".to_owned(),
        "Microsoft.UI.Xaml.Media.LoadedImageSourceLoadCompletedEventArgs".to_owned(),
        "Microsoft.UI.Xaml.Media.LoadedImageSourceLoadStatus".to_owned(),
        "Microsoft.UI.Xaml.Media.SolidColorBrush".to_owned(),
        // Font model support (docs/design/runtime/text_design.md) — `Control.FontFamily`/`FontSize`/
        // `FontWeight`/`FontStyle`/`FontStretch`/`CharacterSpacing`/`Foreground` are already
        // reachable through `Microsoft.UI.Xaml.Controls.Control` above; these three add the value
        // types those properties are typed with. **Unverified**: this crate is
        // `#![cfg(target_os = "windows")]` and has never been built/type-checked on this machine —
        // see that file's own doc comment.
        "Microsoft.UI.Xaml.Media.FontFamily".to_owned(),
        "Windows.UI.Text.FontWeight".to_owned(),
        "Windows.UI.Text.FontStyle".to_owned(),
        "Windows.UI.Text.FontStretch".to_owned(),
        "Microsoft.UI.Xaml.Hosting.ElementCompositionPreview".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Ellipse".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Line".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Rectangle".to_owned(),
        "Microsoft.UI.Xaml.Shapes.Shape".to_owned(),
        "Microsoft.UI.Composition.Compositor".to_owned(),
        "Microsoft.UI.Composition.CompositionObject".to_owned(),
        "Microsoft.UI.Composition.Visual".to_owned(),
        "Microsoft.UI.Composition.ContainerVisual".to_owned(),
        "Microsoft.UI.Composition.SpriteVisual".to_owned(),
        "Microsoft.UI.Composition.ShapeVisual".to_owned(),
        "Microsoft.UI.Composition.CompositionShape".to_owned(),
        "Microsoft.UI.Composition.CompositionSpriteShape".to_owned(),
        "Microsoft.UI.Composition.CompositionGeometry".to_owned(),
        "Microsoft.UI.Composition.CompositionRectangleGeometry".to_owned(),
        "Microsoft.UI.Composition.CompositionRoundedRectangleGeometry".to_owned(),
        "Microsoft.UI.Composition.CompositionEllipseGeometry".to_owned(),
        "Microsoft.UI.Composition.CompositionLineGeometry".to_owned(),
        "Microsoft.UI.Composition.CompositionPathGeometry".to_owned(),
        "Microsoft.UI.Composition.CompositionPath".to_owned(),
        "Microsoft.UI.Composition.CompositionBrush".to_owned(),
        "Microsoft.UI.Composition.CompositionColorBrush".to_owned(),
        "Microsoft.UI.Composition.CompositionGradientBrush".to_owned(),
        "Microsoft.UI.Composition.CompositionLinearGradientBrush".to_owned(),
        "Microsoft.UI.Composition.CompositionRadialGradientBrush".to_owned(),
        "Microsoft.UI.Composition.CompositionColorGradientStop".to_owned(),
        "Microsoft.UI.Composition.CompositionColorGradientStopCollection".to_owned(),
        "Microsoft.UI.Composition.CompositionSurfaceBrush".to_owned(),
        "Microsoft.UI.Composition.CompositionClip".to_owned(),
        "Microsoft.UI.Composition.InsetClip".to_owned(),
        "Microsoft.UI.Composition.CompositionGeometricClip".to_owned(),
        "Microsoft.UI.Composition.VisualCollection".to_owned(),
        "Microsoft.UI.Composition.CompositionShapeCollection".to_owned(),
        "Microsoft.UI.Composition.CompositionStrokeCap".to_owned(),
        "Microsoft.UI.Composition.CompositionStrokeLineJoin".to_owned(),
        "Microsoft.UI.Composition.CompositionStrokeDashArray".to_owned(),
        "Microsoft.UI.Composition.CompositionStretch".to_owned(),
        "Microsoft.UI.Composition.CompositionMappingMode".to_owned(),
        "Microsoft.UI.Composition.CompositionGradientExtendMode".to_owned(),
        "Microsoft.UI.Composition.ICompositionSurface".to_owned(),
        "Microsoft.UI.Composition.CompositionGraphicsDevice".to_owned(),
        "Microsoft.UI.Composition.CompositionDrawingSurface".to_owned(),
        "Microsoft.Graphics.Canvas.UI.Composition.CanvasComposition".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasDrawingSession".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasDevice".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasActiveLayer".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasBitmap".to_owned(),
        // Menu icon remediation (PR #171 delta): a `CanvasRenderTarget` is used as the offscreen
        // surface a user `VectorImage` menu icon is rasterized into, then encoded to PNG via
        // `CanvasBitmap.SaveAsync(..., CanvasBitmapFileFormat.Png, ...)` for the
        // `InMemoryRandomAccessStream` -> XAML `BitmapImage` bridge (`icon_source_design.md` §5).
        "Microsoft.Graphics.Canvas.CanvasRenderTarget".to_owned(),
        "Microsoft.Graphics.Canvas.CanvasBitmapFileFormat".to_owned(),
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
        "Microsoft.Graphics.Canvas.Geometry.CanvasGeometryCombine".to_owned(),
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
    std::fs::write(
        &out_path,
        generated
            .replacen("#![allow(", "#[allow(", 1)
            // windows-bindgen 0.62 emits this WinRT ABI type through the optional
            // `windows_numerics` crate, which has no Quaternion definition. The `windows`
            // crate provides the same official Windows.Foundation.Numerics ABI type.
            .replace(
                "windows_numerics::Quaternion",
                "windows::Foundation::Numerics::Quaternion",
            ),
    )
    .expect("write generated WinUI bindings");
    let interop =
        std::fs::read_to_string(&interop_path).expect("read generated XAML interop bindings");
    std::fs::write(&interop_path, interop.replacen("#![allow(", "#[allow(", 1))
        .expect("write generated XAML interop bindings");
    copy_win2d_runtime(&out_dir, &packages);
    write_nuget_selection(&out_dir, &packages);
    // Required by the native host in both modes.
    println!("cargo:rustc-link-lib=WindowsApp");
    match prebuilt_dir {
        Some(dir) => link_and_deploy_prebuilt(&out_dir, &dir),
        None => {
            let resources_pri = generate_resources_pri(&out_dir);
            let native = build_cpp_app_host(&out_dir, &winmd_inputs, &app_sdk, &packages);
            if let Some(export_dir) = export_dir {
                export_source_artifacts(&export_dir, &target, &native, &resources_pri);
            }
        }
    }
    write_native_mode_marker(&out_dir, mode);
    if !warnings.is_empty() || !interop_warnings.is_empty() {
        println!(
            "cargo:warning=WinUI binding generation omitted {} unsupported metadata member(s)",
            warnings.len()
        );
    }
    println!("cargo:rustc-env=ELWINDUI_WINUI3_BINDINGS={out_path}");
}

#[cfg(target_os = "windows")]
fn copy_win2d_runtime(out_dir: &str, packages: &build_nuget::PinnedPackages) {
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "win-x86",
        Ok("aarch64") => "win-arm64",
        _ => "win-x64",
    };
    let native_dll = |package: &build_nuget::PinnedPackage, name: &str| {
        let path = package
            .dir
            .join("runtimes")
            .join(arch)
            .join("native")
            .join(name);
        path.is_file().then_some(path)
    };
    let profile_dir = std::path::Path::new(out_dir)
        .ancestors()
        .nth(3)
        .expect("target profile directory");
    let deps_dir = profile_dir.join("deps");
    std::fs::create_dir_all(&deps_dir).expect("create target/<profile>/deps directory");

    let Some(source) = native_dll(&packages.win2d, "Microsoft.Graphics.Canvas.dll") else {
        panic!(
            "Microsoft.Graphics.Canvas.dll was not found for {arch} in {}",
            packages.win2d.dir.display()
        );
    };
    let target = profile_dir.join("Microsoft.Graphics.Canvas.dll");
    std::fs::copy(&source, &target)
        .expect("copy Microsoft.Graphics.Canvas.dll beside application binary");
    std::fs::copy(&source, deps_dir.join("Microsoft.Graphics.Canvas.dll"))
        .expect("copy Microsoft.Graphics.Canvas.dll beside test binaries");
    println!("cargo:rerun-if-changed={}", source.display());

    // Current Windows App SDK packages place the native bootstrap DLL in the Foundation package;
    // the older layout placed it directly in Microsoft.WindowsAppSDK. Both lookups are restricted
    // to the pinned versions.
    let bootstrap = "Microsoft.WindowsAppRuntime.Bootstrap.dll";
    let Some(source) = native_dll(&packages.foundation, bootstrap)
        .or_else(|| native_dll(&packages.app_sdk, bootstrap))
    else {
        panic!(
            "{bootstrap} was not found for {arch} in {} or {}",
            packages.foundation.dir.display(),
            packages.app_sdk.dir.display()
        );
    };
    let target = profile_dir.join(bootstrap);
    std::fs::copy(&source, &target)
        .expect("copy Microsoft.WindowsAppRuntime.Bootstrap.dll beside application binary");
    std::fs::copy(&source, deps_dir.join(bootstrap))
        .expect("copy Microsoft.WindowsAppRuntime.Bootstrap.dll beside test binaries");
    println!("cargo:rerun-if-changed={}", source.display());
}

/// `NUGET_PACKAGES`, else the user's default global packages folder.
#[cfg(target_os = "windows")]
fn nuget_packages_root() -> std::path::PathBuf {
    std::env::var_os("NUGET_PACKAGES")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|profile| std::path::PathBuf::from(profile).join(".nuget\\packages"))
        })
        .expect("NUGET_PACKAGES or USERPROFILE is required to locate the WinUI 3 NuGet packages")
}

/// Reads a metadata override environment variable. The override must resolve (`..`, symlinks and
/// junctions resolved) to an existing `.winmd` inside the pinned package version; the resolved path
/// is what the build then reads.
#[cfg(target_os = "windows")]
fn pinned_override(
    variable: &str,
    package: &build_nuget::PinnedPackage,
) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(std::env::var_os(variable)?);
    Some(
        build_nuget::validate_override(&path, package)
            .unwrap_or_else(|error| panic!("{variable}: {error}")),
    )
}

/// `<package dir>/metadata/<contract>/<filename>`, highest contract directory.
#[cfg(target_os = "windows")]
fn find_contract_winmd(
    package_dir: &std::path::Path,
    filename: &str,
) -> Option<std::path::PathBuf> {
    let mut candidates: Vec<_> = std::fs::read_dir(package_dir.join("metadata"))
        .ok()?
        .flatten()
        .map(|contract| contract.path().join(filename))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}

/// `<package dir>/lib/<filename>` or `<package dir>/lib/<tfm>/<filename>`.
#[cfg(target_os = "windows")]
fn find_lib_winmd(package_dir: &std::path::Path, filename: &str) -> Option<std::path::PathBuf> {
    let lib = package_dir.join("lib");
    let flat = lib.join(filename);
    if flat.is_file() {
        return Some(flat);
    }
    let mut candidates: Vec<_> = std::fs::read_dir(lib)
        .ok()?
        .flatten()
        .map(|target| target.path().join(filename))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}

/// Records the NuGet package versions this build read in `OUT_DIR` (diagnostics/CI evidence).
#[cfg(target_os = "windows")]
fn write_nuget_selection(out_dir: &str, packages: &build_nuget::PinnedPackages) {
    let text: String = packages
        .all()
        .iter()
        .map(|package| format!("{} {}\n", package.id, package.version))
        .collect();
    std::fs::write(
        std::path::Path::new(out_dir).join("elwindui-winui3-nuget-packages.txt"),
        text,
    )
    .expect("write NuGet package selection");
}

/// An unpackaged Windows App SDK process cannot resolve *any* `ms-appx://` resource — including a
/// framework package's own bundled resources, such as WinUI 3's default control theme
/// (`ms-appx:///Microsoft.UI.Xaml/Themes/themeresources.xaml`, consulted by
/// `install_default_control_resources` in `src/inner.rs`) — unless a `resources.pri` sits next to
/// its own executable to bootstrap MRT resource-context resolution. This generates a minimal one
/// (indexing none of this crate's own resources — it exists purely to make that resolution
/// possible at all) and copies it beside the built exe, the same way `copy_win2d_runtime` places
/// `Microsoft.Graphics.Canvas.dll` there. Requires `makepri.exe` from the Windows SDK, i.e.
/// `tools/setup-vs-env.ps1` sourced first — source-native mode only; prebuilt mode deploys the
/// checked-in `resources.pri` through the same `deploy_resources_pri` destinations instead.
#[cfg(target_os = "windows")]
fn generate_resources_pri(out_dir: &str) -> std::path::PathBuf {
    let makepri =
        find_makepri().expect("makepri.exe was not found; source tools/setup-vs-env.ps1 first");
    let pri_root = std::path::Path::new(out_dir).join("resources_pri");
    std::fs::create_dir_all(&pri_root).expect("create resources.pri project root");

    let config = pri_root.join("priconfig.xml");
    let status = std::process::Command::new(&makepri)
        .arg("createconfig")
        .arg("/cf")
        .arg(&config)
        .arg("/dq")
        .arg("en-US")
        .arg("/pv")
        .arg("10.0.0")
        .arg("/o")
        .status()
        .expect("run makepri.exe createconfig");
    assert!(status.success(), "makepri.exe createconfig failed");

    let generated = pri_root.join("resources.pri");
    let status = std::process::Command::new(&makepri)
        .arg("new")
        .arg("/pr")
        .arg(&pri_root)
        .arg("/cf")
        .arg(&config)
        .arg("/of")
        .arg(&generated)
        .arg("/o")
        .status()
        .expect("run makepri.exe new");
    assert!(status.success(), "makepri.exe new failed");

    deploy_resources_pri(out_dir, &generated);
    generated
}

#[cfg(target_os = "windows")]
fn deploy_resources_pri(out_dir: &str, resources_pri: &std::path::Path) {
    let profile_dir = std::path::Path::new(out_dir)
        .ancestors()
        .nth(3)
        .expect("target profile directory");
    std::fs::copy(resources_pri, profile_dir.join("resources.pri"))
        .expect("copy resources.pri beside application binary");
    // `cargo test`/`cargo bench` binaries run from `target/<profile>/deps/`, not
    // `target/<profile>/` itself — MRT resource-context resolution looks beside the actual running
    // executable, so a Windows-only test that touches any native control (anything going through
    // `install_default_control_resources`) needs its own copy here too, or it fails with
    // `Cannot locate resource from 'ms-appx:///Microsoft.UI.Xaml/Themes/themeresources.xaml'`
    // despite the example binaries (which do live directly in `target/<profile>/`) working fine.
    let deps_dir = profile_dir.join("deps");
    std::fs::create_dir_all(&deps_dir).expect("create target/<profile>/deps directory");
    std::fs::copy(resources_pri, deps_dir.join("resources.pri"))
        .expect("copy resources.pri beside test binaries");
}

#[cfg(target_os = "windows")]
fn deploy_accessibility_winmd(out_dir: &str, component_winmd: &std::path::Path) {
    let profile_dir = std::path::Path::new(out_dir)
        .ancestors()
        .nth(3)
        .expect("target profile directory");
    let deps_dir = profile_dir.join("deps");
    std::fs::create_dir_all(&deps_dir).expect("create target/<profile>/deps directory");
    let filename = component_winmd
        .file_name()
        .expect("accessibility component WinMD filename");
    std::fs::copy(component_winmd, profile_dir.join(filename))
        .expect("copy accessibility component WinMD beside application binary");
    std::fs::copy(component_winmd, deps_dir.join(filename))
        .expect("copy accessibility component WinMD beside test binaries");
}

/// Generates a C++/WinRT projection (via `cppwinrt.exe`) for just enough of the WinUI 3 surface to
/// host `Application`, and compiles `cpp/app_host.cpp` against it — see that file's own doc comment
/// (and `src/composed_application.rs`'s) for why this exists at all (microsoft/windows-rs#3404).
/// Source-native mode only.
#[cfg(target_os = "windows")]
fn build_cpp_app_host(
    out_dir: &str,
    winmd_inputs: &[String],
    app_sdk: &std::path::Path,
    packages: &build_nuget::PinnedPackages,
) -> SourceNativeOutputs {
    let cppwinrt = find_sdk_tool("cppwinrt.exe")
        .expect("cppwinrt.exe was not found; source tools/setup-vs-env.ps1 first");
    let midl = find_sdk_tool("midl.exe")
        .expect("midl.exe was not found; source tools/setup-vs-env.ps1 first");
    let projection_dir = std::path::Path::new(out_dir).join("cppwinrt_include");

    let component_dir = std::path::Path::new(out_dir).join("accessibility_component");
    std::fs::create_dir_all(&component_dir).expect("create accessibility component output");
    let component_winmd = component_dir.join("Elwindui.WinUI3.Accessibility.winmd");
    let idl =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
            .join("cpp/accessibility_semantic_peer.idl");
    let metadata_dir = find_sdk_union_metadata_dir()
        .expect("Windows SDK UnionMetadata was not found; source tools/setup-vs-env.ps1 first");

    let midl_env = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "win32",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    let mut midl_args = vec![
        "/nologo".to_owned(),
        "/winrt".to_owned(),
        "/env".to_owned(),
        midl_env.to_owned(),
        "/out".to_owned(),
        component_dir.to_string_lossy().into_owned(),
        "/winmd".to_owned(),
        component_winmd.to_string_lossy().into_owned(),
        "/metadata_dir".to_owned(),
        metadata_dir.to_string_lossy().into_owned(),
    ];
    midl_args.push("/reference".to_owned());
    midl_args.push(app_sdk.to_string_lossy().into_owned());
    for metadata in [
        "Windows.Foundation.FoundationContract.winmd",
        "Windows.Foundation.UniversalApiContract.winmd",
    ] {
        let path = find_sdk_reference_winmd(metadata).unwrap_or_else(|| {
            panic!(
                "Windows SDK reference metadata was not found: {metadata}; source tools/setup-vs-env.ps1 first"
            )
        });
        midl_args.push("/reference".to_owned());
        midl_args.push(path.to_string_lossy().into_owned());
    }
    midl_args.extend(["/h".to_owned(), "nul".to_owned(), "/nomidl".to_owned()]);
    midl_args.push(idl.to_string_lossy().into_owned());
    let midl_status = std::process::Command::new(&midl)
        .args(&midl_args)
        .status()
        .expect("run midl.exe for accessibility_semantic_peer.idl");
    assert!(
        midl_status.success(),
        "midl.exe failed generating the accessibility semantic peer metadata"
    );
    assert!(
        component_winmd.is_file(),
        "midl.exe did not produce {}",
        component_winmd.display()
    );
    deploy_accessibility_winmd(out_dir, &component_winmd);

    let mut component_args = vec![
        "-input".to_owned(),
        component_winmd.to_string_lossy().into_owned(),
    ];
    component_args.push("-reference".to_owned());
    component_args.push("sdk+".to_owned());
    for winmd in winmd_inputs {
        component_args.push("-reference".to_owned());
        component_args.push(winmd.clone());
    }
    if let Some(webview2) =
        find_lib_winmd(&packages.webview2.dir, "Microsoft.Web.WebView2.Core.winmd")
    {
        component_args.push("-reference".to_owned());
        component_args.push(webview2.to_string_lossy().into_owned());
    }
    component_args.extend([
        "-include".to_owned(),
        "Elwindui.WinUI3.Accessibility".to_owned(),
        "-output".to_owned(),
        component_dir.to_string_lossy().into_owned(),
        "-component".to_owned(),
        "-pch".to_owned(),
        ".".to_owned(),
        "-overwrite".to_owned(),
    ]);
    for excluded in [
        "Microsoft.UI.Xaml.Controls.WebView2",
        "Microsoft.UI.Xaml.Controls.IWebView2",
    ] {
        component_args.push("-exclude".to_owned());
        component_args.push(excluded.to_owned());
    }
    let component_status = std::process::Command::new(&cppwinrt)
        .args(&component_args)
        .status()
        .expect("run cppwinrt.exe for accessibility semantic peer");
    assert!(
        component_status.success(),
        "cppwinrt.exe failed generating the accessibility semantic peer component"
    );
    let component_source = component_dir.join("module.g.cpp");
    assert!(
        component_source.is_file(),
        "cppwinrt.exe did not produce {}",
        component_source.display()
    );

    let mut args: Vec<String> = vec!["-input".to_owned(), "sdk+".to_owned()];
    for winmd in winmd_inputs {
        args.push("-input".to_owned());
        args.push(winmd.clone());
    }
    // `Microsoft.UI.Xaml.winmd`'s own `IWebView2` interface references WebView2's winmd even
    // though this shim never touches WebView2 — cppwinrt validates the whole input database
    // up front, so the reference has to resolve even when `-exclude` drops the type from output.
    if let Some(webview2) =
        find_lib_winmd(&packages.webview2.dir, "Microsoft.Web.WebView2.Core.winmd")
    {
        args.push("-input".to_owned());
        args.push(webview2.to_string_lossy().into_owned());
    }
    for namespace in [
        "Microsoft.UI",
        "Windows.Foundation",
        "Windows.Foundation.Collections",
        "Windows.UI",
        "Windows.System",
        "Microsoft.UI.Xaml.Automation",
        "Microsoft.UI.Xaml.Automation.Peers",
    ] {
        args.push("-include".to_owned());
        args.push(namespace.to_owned());
    }
    // Excludes types this shim never touches whose own metadata references external winmd files
    // this build doesn't provide (e.g. WebView2's own winmd, only needed by real WebView2 users).
    for excluded in [
        "Microsoft.UI.Xaml.Controls.WebView2",
        "Microsoft.UI.Xaml.Controls.IWebView2",
    ] {
        args.push("-exclude".to_owned());
        args.push(excluded.to_owned());
    }
    args.push("-output".to_owned());
    args.push(projection_dir.to_string_lossy().into_owned());
    args.push("-overwrite".to_owned());

    let status = std::process::Command::new(&cppwinrt)
        .args(&args)
        .status()
        .expect("run cppwinrt.exe");
    assert!(
        status.success(),
        "cppwinrt.exe failed generating the C++/WinRT projection"
    );

    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .file("cpp/app_host.cpp")
        .file("cpp/accessibility_host.cpp")
        .file(&component_source)
        .include(&component_dir)
        .include(&projection_dir)
        .flag_if_supported("/await:strict")
        .flag_if_supported("/EHsc")
        .flag_if_supported("/utf-8")
        .compile(build_support::NATIVE_LIB_NAME);

    println!("cargo:rerun-if-changed=cpp/app_host.cpp");
    println!("cargo:rerun-if-changed=cpp/accessibility_host.h");
    println!("cargo:rerun-if-changed=cpp/accessibility_host.cpp");
    println!("cargo:rerun-if-changed=cpp/accessibility_semantic_peer.idl");

    let native_lib = std::path::Path::new(out_dir).join(build_support::NATIVE_LIB_FILE);
    assert!(
        native_lib.is_file(),
        "cc did not produce {}",
        native_lib.display()
    );
    SourceNativeOutputs {
        native_lib,
        accessibility_winmd: component_winmd,
    }
}

/// Native artifacts produced by `build_cpp_app_host`, located by their known output paths (never
/// by scanning `target/`).
#[cfg(target_os = "windows")]
struct SourceNativeOutputs {
    native_lib: std::path::PathBuf,
    accessibility_winmd: std::path::PathBuf,
}

/// Selects `native/prebuilt/<TARGET>/` and requires all three artifacts to be present. Panics with
/// an actionable message otherwise; there is no fallback to source compilation.
#[cfg(target_os = "windows")]
fn resolve_prebuilt_dir(target: &str) -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
    );
    let dir = build_support::prebuilt_target_dir(&manifest_dir, target).unwrap_or_else(|error| {
        panic!(
            "{error}. Normal builds never fall back to compiling native sources; ElwindUI \
             maintainers can build the native shim from source with {}=1.",
            build_support::BUILD_NATIVE_ENV
        )
    });
    for file in build_support::PREBUILT_ARTIFACT_FILES {
        let path = dir.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let present = std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_file());
        assert!(
            present,
            "{}",
            build_support::missing_prebuilt_message(target, &path)
        );
    }
    dir
}

/// Prebuilt mode: links the checked-in static library and deploys the checked-in `resources.pri`
/// and accessibility WinMD to the same executable/test locations source-native mode uses.
#[cfg(target_os = "windows")]
fn link_and_deploy_prebuilt(out_dir: &str, dir: &std::path::Path) {
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!(
        "cargo:rustc-link-lib=static={}",
        build_support::NATIVE_LIB_NAME
    );
    deploy_resources_pri(out_dir, &dir.join(build_support::RESOURCES_PRI_FILE));
    deploy_accessibility_winmd(out_dir, &dir.join(build_support::ACCESSIBILITY_WINMD_FILE));
}

/// Source-native mode with `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR`: copies the three artifacts this
/// build produced into `<export_dir>/<TARGET>/`. A build never writes the checked-in
/// `native/prebuilt/` tree.
#[cfg(target_os = "windows")]
fn export_source_artifacts(
    export_dir: &std::path::Path,
    target: &str,
    native: &SourceNativeOutputs,
    resources_pri: &std::path::Path,
) {
    let destination = export_dir.join(target);
    std::fs::create_dir_all(&destination).unwrap_or_else(|error| {
        panic!(
            "create prebuilt export directory {}: {error}",
            destination.display()
        )
    });
    for (source, file) in [
        (native.native_lib.as_path(), build_support::NATIVE_LIB_FILE),
        (
            native.accessibility_winmd.as_path(),
            build_support::ACCESSIBILITY_WINMD_FILE,
        ),
        (resources_pri, build_support::RESOURCES_PRI_FILE),
    ] {
        std::fs::copy(source, destination.join(file)).unwrap_or_else(|error| {
            panic!(
                "export {} to {}: {error}",
                source.display(),
                destination.display()
            )
        });
    }
}

/// Resolves `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR` to an absolute path without creating anything
/// and panics if it is the tracked prebuilt tree, inside it, or one of its ancestors.
#[cfg(target_os = "windows")]
fn validated_export_destination(raw: &std::path::Path) -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
    );
    let requested = resolve_without_creating(&manifest_dir.join(raw)).unwrap_or_else(|| {
        panic!(
            "{}={} cannot be resolved safely (a `..` below its nearest existing directory)",
            build_support::PREBUILT_EXPORT_DIR_ENV,
            raw.display()
        )
    });
    let tracked = resolve_without_creating(
        &manifest_dir
            .join(build_support::PREBUILT_ROOT[0])
            .join(build_support::PREBUILT_ROOT[1]),
    )
    .expect("resolve the tracked prebuilt tree");
    assert!(
        !build_support::export_dir_conflicts(&requested, &tracked),
        "{}={} must not be, contain or lie inside the checked-in prebuilt tree {}",
        build_support::PREBUILT_EXPORT_DIR_ENV,
        raw.display(),
        tracked.display()
    );
    requested
}

/// Canonicalizes the nearest existing ancestor (resolving symlinks/junctions) and appends the
/// remaining, not yet existing components. `None` if those remaining components contain `..`.
#[cfg(target_os = "windows")]
fn resolve_without_creating(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let existing = path.ancestors().find(|ancestor| ancestor.exists())?;
    let remainder = path.strip_prefix(existing).ok()?;
    if remainder
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    let canonical = std::fs::canonicalize(existing).ok()?;
    if remainder.as_os_str().is_empty() {
        Some(canonical)
    } else {
        Some(canonical.join(remainder))
    }
}

/// Records which native mode this build script run used (`prebuilt` or `source`) in `OUT_DIR`, so
/// CI can assert that a consumer build took the prebuilt path.
#[cfg(target_os = "windows")]
fn write_native_mode_marker(out_dir: &str, mode: build_support::NativeBuildMode) {
    let value = match mode {
        build_support::NativeBuildMode::Prebuilt => "prebuilt\n",
        build_support::NativeBuildMode::Source => "source\n",
    };
    std::fs::write(
        std::path::Path::new(out_dir).join("elwindui-winui3-native-mode.txt"),
        value,
    )
    .expect("write native build mode marker");
}

#[cfg(target_os = "windows")]
fn find_sdk_tool(name: &str) -> Option<std::path::PathBuf> {
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    if let (Ok(sdk_dir), Ok(sdk_version)) = (
        std::env::var("WindowsSdkDir"),
        std::env::var("WindowsSDKVersion"),
    ) {
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
fn find_sdk_union_metadata_dir() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let (Ok(sdk_dir), Ok(sdk_version)) = (
        std::env::var("WindowsSdkDir"),
        std::env::var("WindowsSDKVersion"),
    ) {
        candidates.push(
            std::path::Path::new(&sdk_dir)
                .join("UnionMetadata")
                .join(sdk_version.trim_end_matches('\\')),
        );
    }
    let union_root = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata");
    if let Ok(entries) = std::fs::read_dir(union_root) {
        candidates.extend(
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir()),
        );
    }
    candidates.sort();
    candidates.into_iter().rev().find(|path| {
        path.file_name().is_some_and(|name| name != "Facade")
            && path.join("Windows.winmd").is_file()
    })
}

#[cfg(target_os = "windows")]
fn find_sdk_reference_winmd(filename: &str) -> Option<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let (Ok(sdk_dir), Ok(sdk_version)) = (
        std::env::var("WindowsSdkDir"),
        std::env::var("WindowsSDKVersion"),
    ) {
        roots.push(
            std::path::Path::new(&sdk_dir)
                .join("References")
                .join(sdk_version.trim_end_matches('\\')),
        );
    }
    roots.push(std::path::PathBuf::from(
        r"C:\Program Files (x86)\Windows Kits\10\References\10.0.26100.0",
    ));

    let mut candidates = Vec::new();
    for root in roots {
        let Ok(namespaces) = std::fs::read_dir(root) else {
            continue;
        };
        for namespace in namespaces.flatten() {
            let Ok(versions) = std::fs::read_dir(namespace.path()) else {
                continue;
            };
            for version in versions.flatten() {
                let path = version.path().join(filename);
                if path.is_file() {
                    candidates.push(path);
                }
            }
        }
    }
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
    if let (Ok(sdk_dir), Ok(sdk_version)) = (
        std::env::var("WindowsSdkDir"),
        std::env::var("WindowsSDKVersion"),
    ) {
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

#[cfg(not(target_os = "windows"))]
fn main() {}
