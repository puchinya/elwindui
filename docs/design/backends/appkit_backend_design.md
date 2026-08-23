# AppKit backend design

Related specifications: [`../../specs/ui_spec.md`](../../specs/ui_spec.md), [`../../specs/graphics_spec.md`](../../specs/graphics_spec.md), and [`../../specs/theme_environment_spec.md`](../../specs/theme_environment_spec.md).

## Layering

The backend follows `native_ui -> inner -> host -> render -> ffi`.

- `native_ui` translates public control properties and events;
- `inner` owns Objective-C target/delegate objects and low-level view behavior;
- `host` owns root trees, viewport, layout scheduling, activation, and owner mapping;
- `render` replays retained groups through Core Graphics and Core Animation;
- `ffi` contains narrow Objective-C/runtime calls not provided by safe wrappers.

Native AppKit types never cross the common `elwindui-core` API.

## Application and host

Application startup runs on the main thread. Window and tree hosts own AppKit views and schedule coalesced layout/render passes on that thread. An independently hosted subtree, such as TabView content, has its own `TreeHostView` and activation state.

`Window.transparent` is applied by `InnerWindow` through `NSWindow.opaque` and a clear/window background color; decorations remain untouched. `Window.always_on_top` switches the same native window between `NSFloatingWindowLevel` and `NSNormalWindowLevel`, so both properties can change before or after display without recreating the host tree.

Owner mapping associates native views and event targets with weak ElwindUI owners. Disposal removes delegates, notifications, and mappings before releasing the view.

`TreeHostView` forwards self-drawn mouse press/move/release through the common `PointerDispatcher`; AppKit's drag event overrides preserve delivery during an active press. A self-drawn press also makes the host the window's first responder so Escape reaches the same host during the gesture. Escape, key-window/application deactivation, host suppression, view detachment/window transfer, and hosted-tree replacement/clear cancel the Core gesture before teardown. A weak `AppKitPointerGestureHost` lets common subtree unmount perform the same ordered cancellation without retaining the native view.

A weak `AppKitCoordinateHost` converts between flipped view-root coordinates and Core's top-left/Y-down logical desktop coordinates using `NSWindow` point conversion and the primary-screen-height flip. Context requests and ordinary pointer payloads share this conversion.

## Layout and native controls

Common layout measures native controls through AppKit fitting/intrinsic size and applies arranged rectangles in host coordinates. Scroll content is wrapped by the native scroll view plus an ElwindUI content root; the public content tree remains backend-neutral.

Native control properties are synchronized from local, text-style, and Theme revisions. Clearing a Theme-backed property restores system font, label color, appearance, or other AppKit default instead of assigning a common hard-coded value.

`NSSecureTextField` uses the system secure-entry font cascade. Ordinary text-style synthesis is not applied when it can break secure mask glyphs.

AppKit `TabView` keeps a custom embedded tab strip rather than `NSTabView`, because the shared `TabView` contract includes per-item close and new-tab affordances that a real `NSTabView`/`NSWindow` tab bar does not expose with the right ownership boundary. Each chip is a layerless `NSStackView` subclass that draws its own selection/hover state with semantic AppKit colors and uses an `NSTrackingArea` for internal hover state, adjacent to its neighbors (Safari/Xcode-style, not spaced pills). When the strip's own combined natural width would exceed the available space, every chip shrinks together (down to a minimum floor) rather than either scrolling or clipping past the window edge — matching how Safari/Xcode actually handle tab overflow. Tab content remains in persistent, independently activated `TreeHostView`s.

## Rendering and cache lifetime

Render groups replay into Core Graphics/layers with balanced clip, transform, and opacity state. Layer/image resources are owned by the corresponding render node and pruned when reconciliation removes or deactivates it.

Memory measurement reports are Issue evidence, not durable architecture. Durable cache ownership decisions belong here; current measured results belong in backend status.
