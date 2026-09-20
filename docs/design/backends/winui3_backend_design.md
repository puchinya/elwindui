# WinUI 3 backend design

Related specifications: [`../../specs/ui_spec.md`](../../specs/ui_spec.md), [`../../specs/graphics_spec.md`](../../specs/graphics_spec.md), and [`../../specs/theme_environment_spec.md`](../../specs/theme_environment_spec.md).

## Projection and startup

`build.rs` generates WinUI 3 / Win2D bindings and a separate `Windows.UI.Xaml.Interop` projection. Windows App SDK bootstrap and STA COM initialization occur on the UI thread before application startup.

Application hosting intentionally uses the small C++/WinRT `ApplicationT<App, IXamlMetadataProvider>` shim in `cpp/app_host.cpp`. The shim installs `XamlControlsResources` and calls one exported Rust startup callback. Window creation, controls, layout, rendering, events, and task execution remain in Rust.

This boundary is load-bearing: replacing the shim requires a separately approved design demonstrating correct WinRT composable-class behavior and `Application.Resources` initialization.

`crate::app`'s `WINDOWS` registry (Issue #254) is the application-layer strong lifetime authority for the final most-derived `Rc<dyn WindowExt>` once a `Window` has been shown for the first time — retained via `retain_window` on first `show()` (keyed off `__self_weak`, captured at `Window::construct()`), released reactively from the native `Window.Closed` event, with `Application::Exit()` called only once the registry is freshly observed empty after the release. See `docs/design/runtime/component_lifecycle_design.md` §4j for the full cross-backend invariant and rationale.

## Native hosting and layout

Tree hosts own XAML roots, ElwindUI owner mappings, viewport layout, activation, and native child reconciliation. WinUI widgets remain leaves selected by the common NativeControl design.

`TreeHost` is a viewport *consumer*, never its own viewport producer (Issue #261 review remediation): it stores one explicit `TreeHostViewport` (`Some(width)`/`Some(height)` constrained, `None` unconstrained), set exclusively by its owner through `TreeHost::set_viewport`, and `relayout_static_pass` measures/arranges against that stored value alone — never against `Canvas.Width`/`Height`/`ActualWidth`/`ActualHeight`, and `TreeHost` does not observe its own `Canvas.SizeChanged`. A host self-observing the native size its own layout just produced is a same-host feedback cascade, not a legitimate relayout trigger — confirmed directly: routing `Canvas.SizeChanged` back into the same host's own scheduling produced 595+ queued relayout cycles and 112.5s+ of cumulative text-measurement time on one host during live `docking-demo` testing before this was fixed.

Each backend host boundary has exactly one declared viewport authority that calls `set_viewport`:

- the top-level `Window` — `Window.Bounds` (synchronous, at construction/`set_content`/right after `Activate()`) and `Window.SizeChanged` (ongoing, registered once in `InnerWindow::new`) push a constrained width and constrained content height (with a `MENU_BAR_HEIGHT` inset when `set_menu_bar` is used) — the same authority a floating `Docking` window uses, with no Docking-specific sizing path;
- a `TabView` page host — `InnerTabView::resize_content_host`, driven by the native `TabView`'s own content viewport; only the selected page's host is laid out, and an unselected page's host stores its viewport without laying out (`set_viewport`'s inactive/suppressed short-circuit);
- a `ScrollView` content host — `sync_scroll_view_cross_axis`, driven by the native `ScrollViewer`'s own viewport; the non-scrolling axis is constrained, the scrolling axis is `None` (unconstrained), and the resulting natural-size growth on that axis is presentation output only, never fed back as a new viewport;
- a `Popup` host — `PopupRequest.size`, pushed once before the popup's content tree is attached.

Any future native control that owns a nested `TreeHost` must declare its own viewport authority the same way and update its child only through `set_viewport` — never through a native size-change notification on that same child.

`Window.transparent` sets or clears a transparent background on the root `TreeHost` without changing decorations or the host's input surface. `Window.always_on_top` is retained by `InnerWindow` and applied to the `AppWindow`'s `OverlappedPresenter`; `show()` reapplies it so a pre-activation setter is not lost while the native presenter is being established.

Arrange writes explicit `Width` / `Height` for Canvas positioning. Before every natural `Measure`, the adapter resets both values to `NaN` (`Auto`), invalidates native measure, and then measures with the current constraint. This prevents arrange-time sizes from becoming a self-reinforcing natural-size cache.

NativeControl natural measurement has a two-stage first-show lifecycle. The initial Core/render pass
is bootstrap-only: a newly projected FrameworkElement that is not yet `IsLoaded` contributes
provisional zero natural size while the replay appends it to the real XAML Canvas. Each replay batch
of genuinely new native controls then owns one explicit Loaded barrier. When the barrier completes,
one or more controls having reached Loaded causes one root Measure invalidation and interactive
RelayoutHost flush; all-cancelled batches cause no readiness relayout. A one-control batch and a
many-control batch therefore have the same one-relayout readiness cost, while retained controls and
render-only TextBlock projections receive no new Loaded wiring. Native child teardown removes the
Loaded token and cancels unresolved participation, so tree replacement cannot strand a batch.

The barrier holds only a weak Core root and resolves through the existing per-host pending/ticket
and `RelayoutCycleState` machinery. Queued, interactive, and dispatcher-fallback realizations report
whether an actual coalesced pass ran; a successful geometry-affecting pass refreshes the Core-owned
accessibility snapshot once. The synchronous force path keeps its existing single post-pass rebuild.
This publication follows realized geometry rather than individual Loaded notifications and adds no
Canvas-size feedback or recurring layout trigger.

For bounded first-layout verification, `ELWINDUI_WINUI3_DIAGNOSTICS` reports the host id and member
count for each new-native batch, its completion, the relayout source/kind and realization result,
and changed NativeControl projection rectangles. These records are diagnostic observability only;
they do not add a public API or a recurring layout trigger.

The root Canvas forwards self-drawn pointer press/move/release/canceled events to the common `PointerDispatcher`. Every `TreeHost` permanently owns one transparent, hit-test-visible `Rectangle` at `Canvas.Children()[0]`; it spans the final relayout extent, including an unconstrained axis's natural arranged size, and is never part of dynamic reconciliation. Render-only XAML projections remain input-transparent, so blank/self-drawn hit testing resolves to this surface and bubbles to the root Canvas. The source boundary is exact identity: only the root Canvas or that host's exact input surface is forwarded to Core; native XAML children and unrelated descendants are rejected. A successful press captures the native pointer and release relinquishes it. `PointerCanceled` clears and notifies the Core capture before the Canvas releases its native captures; the resulting `PointerCaptureLost`, or an independently initiated capture loss, enters the same idempotent Core cancellation path. A weak `WinUI3PointerGestureHost` applies the Core-first ordering for subtree unmount, host deactivation, and tree replacement/clear.

Private E2E observability is gated by `ELWINDUI_WINUI3_POINTER_TRACE_PATH`. When set, the native
root-Canvas bridge appends and flushes JSONL records containing event, pointer id, source
classification, Core forwarding, root/screen positions, monotonic order, capture success, and the
actual success/failure of native capture release recorded after the release call. The trace never
emits a fabricated `CoreCancellation` callback marker; exactly-once Core cancellation is proved by
combining forwarded native cancellation events with the application Probe canceled count. Trace
failures are ignored by production behavior and never alter `Handled`. When the trace path and
`ELWINDUI_WINUI3_E2E_RELEASE_CAPTURE_ON_PRESS=1` are both present, a Core-accepted press whose
`CapturePointer` succeeds immediately releases that same native pointer and records the actual
release result. The hook performs no Core cancellation; the resulting native `PointerCaptureLost`
travels through the existing callback. Both mechanisms are absent by default and are not exposed
through public APIs or DSL properties.

Real `NativeControl` children remain native input owners and therefore remain hit-testable. Host activation gates the root Canvas: deactivation performs the existing cancellation/native-capture release first, then disables root hit testing while retaining the permanent surface; reactivation restores root hit testing before the existing relayout. The surface's identity, fill, position, size path, and own hit-testability are independent of `Window.transparent`. `WinUI3CoordinateHost` weakly references the Canvas and promotes the existing `ContentCoordinateConverter`/rasterization-scale path for both root-to-screen and screen-to-root conversion, including transforms between Canvas and XamlRoot content.

## Rendering

Win2D handles retained primitive replay for paths, images, gradients, brushes, clipping, opacity, strokes, and supported blend operations. Composition resources and image caches are owned by the render group or host that created them and are released on removal/deactivation.

Native XAML children and Win2D/Composition islands are reconciled from the same active visual tree. A non-selected hosted subtree keeps UI/native control state but releases render resources.

## Theme and text

Theme adapters set or clear dependency properties, apply `RequestedTheme`, and observe `ActualThemeChanged`. Text measurement uses a scratch XAML `TextBlock` with the same conversions used by rendered text. `PlatformDefault` uses ClearValue-equivalent behavior.

Windows environment setup and troubleshooting commands belong in [`../../agents/winui3.md`](../../agents/winui3.md); support and verification belong in [`../../status/backend_status.md`](../../status/backend_status.md).

## Animation projection

`CompositionTarget.Rendering` is the frame source only. The Core presentation
transform (uniform scale, rotation, translation, and origin) and effective
opacity are projected to native XAML children, preserving the common/native
input boundary. Exiting children suppress pointer, focus, and default
activation while remaining visible until Core completes the transition.

The Rendering event token is revoked when the runtime becomes idle and during
host teardown. Projection failure follows the Core safe-cleanup path and cannot
retain an interactive outgoing native child.

## Accessibility projection

WinUI accessibility is a projection of the Core semantic snapshot, not a read of
native child control state. The actual `TreeHost` backing element is a
Canvas-compatible C++/WinRT XAML subclass whose `OnCreateAutomationPeer` returns
the custom root peer. `GetChildrenCore` and virtual child peers read Core values
through the narrow Rust C ABI; peers are cached by integer `AccessibilityId`.

C++ owns only composable host subclassing, AutomationPeer mechanics, copied-value
translation, and callback-context lifetime. Rust/Core owns roles, traversal,
state, IDs, action dispatch, and user callbacks. The bridge detaches before host
destruction and returns empty/unavailable data for late calls. Native XAML
projection children use the narrowest accessibility-view suppression needed to
avoid public duplicates. A raw HWND-wide `WM_GETOBJECT` provider is not used.

The root peer exposes Core name, control type, root-relative-to-screen bounds,
enabled state, keyboard focus, semantic children, and focus. Invoke, Toggle,
RangeValue, Value, SelectionItem, and ExpandCollapse patterns are advertised only
when the Core node advertises an executable matching action.

The snapshot is rebuilt after a successfully realized geometry-affecting queued or interactive
relayout, including the immediate fallback used when dispatcher enqueue is unavailable or rejected.
Loaded notifications are barrier inputs, not accessibility publication triggers; one reconciliation
batch therefore produces at most one post-layout snapshot refresh. Synchronous TreeHost force paths
retain their existing single post-pass rebuild so the same geometry change is never published twice.
