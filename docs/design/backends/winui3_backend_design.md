# WinUI 3 backend design

Related specifications: [`../../specs/ui_spec.md`](../../specs/ui_spec.md), [`../../specs/graphics_spec.md`](../../specs/graphics_spec.md), and [`../../specs/theme_environment_spec.md`](../../specs/theme_environment_spec.md).

## Projection and startup

`build.rs` generates WinUI 3 / Win2D bindings and a separate `Windows.UI.Xaml.Interop` projection. Windows App SDK bootstrap and STA COM initialization occur on the UI thread before application startup.

Application hosting intentionally uses the small C++/WinRT `ApplicationT<App, IXamlMetadataProvider>` shim in `cpp/app_host.cpp`. The shim installs `XamlControlsResources` and calls one exported Rust startup callback. Window creation, controls, layout, rendering, events, and task execution remain in Rust.

This boundary is load-bearing: replacing the shim requires a separately approved design demonstrating correct WinRT composable-class behavior and `Application.Resources` initialization.

## Native hosting and layout

Tree hosts own XAML roots, ElwindUI owner mappings, viewport layout, activation, and native child reconciliation. WinUI widgets remain leaves selected by the common NativeControl design.

The top-level `Window` — not its root `TreeHostPanel` `Canvas` — owns logical viewport truth (Issue #225): `Window.Bounds` (synchronous, at construction/`set_content`/right after `Activate()`) and `Window.SizeChanged` (ongoing, registered once in `InnerWindow::new`) are the sole sizing authority, applied through `TreeHostPanel::set_viewport_size` (explicit `Width`/`Height` plus a synchronous `force_relayout()`) — the same explicit-size-then-relayout convention `TabView`/`ScrollView` already use for their own "size pushed in explicitly" hosts. `TreeHostPanel`'s own root `Canvas.SizeChanged` is *not* the authoritative top-level bootstrap signal: for a plain `Window.Content` (no menu bar), it does not reliably fire at all, which left the Core layout root permanently arranged at `0x0` (confirmed and fixed as part of #225). A `Window` with a menu bar (`set_menu_bar`) routes its wrapping `Canvas` and the content host through this same single Window-owned sizing authority (with a `MENU_BAR_HEIGHT` inset on the content host) rather than an independent `SizeChanged` handler of its own.

`Window.transparent` sets or clears a transparent background on the root `TreeHostPanel` without changing decorations. `Window.always_on_top` is retained by `InnerWindow` and applied to the `AppWindow`'s `OverlappedPresenter`; `show()` reapplies it so a pre-activation setter is not lost while the native presenter is being established.

Arrange writes explicit `Width` / `Height` for Canvas positioning. Before every natural `Measure`, the adapter resets both values to `NaN` (`Auto`), invalidates native measure, and then measures with the current constraint. This prevents arrange-time sizes from becoming a self-reinforcing natural-size cache.

The root Canvas forwards self-drawn pointer press/move/release/canceled events to the common `PointerDispatcher`. Events originating from native XAML children are excluded through `OriginalSource`; a successful press captures the native pointer and release relinquishes it. `PointerCanceled` clears and notifies the Core capture before the Canvas releases its native captures; the resulting `PointerCaptureLost`, or an independently initiated capture loss, enters the same idempotent Core cancellation path. A weak `WinUI3PointerGestureHost` applies the Core-first ordering for subtree unmount, host deactivation, and tree replacement/clear.

XAML elements created only as render projections of self-drawn nodes are input-transparent, so native hit testing passes through to the Canvas and the common render-tree hit test selects the target. Real `NativeControl` children remain native input owners and therefore remain hit-testable. `WinUI3CoordinateHost` weakly references the Canvas and promotes the existing `ContentCoordinateConverter`/rasterization-scale path for both root-to-screen and screen-to-root conversion, including transforms between Canvas and XamlRoot content.

## Rendering

Win2D handles retained primitive replay for paths, images, gradients, brushes, clipping, opacity, strokes, and supported blend operations. Composition resources and image caches are owned by the render group or host that created them and are released on removal/deactivation.

Native XAML children and Win2D/Composition islands are reconciled from the same active visual tree. A non-selected hosted subtree keeps UI/native control state but releases render resources.

## Theme and text

Theme adapters set or clear dependency properties, apply `RequestedTheme`, and observe `ActualThemeChanged`. Text measurement uses a scratch XAML `TextBlock` with the same conversions used by rendered text. `PlatformDefault` uses ClearValue-equivalent behavior.

Windows environment setup and troubleshooting commands belong in [`../../agents/winui3.md`](../../agents/winui3.md); support and verification belong in [`../../status/backend_status.md`](../../status/backend_status.md).
