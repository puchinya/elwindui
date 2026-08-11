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

Owner mapping associates native views and event targets with weak ElwindUI owners. Disposal removes delegates, notifications, and mappings before releasing the view.

## Layout and native controls

Common layout measures native controls through AppKit fitting/intrinsic size and applies arranged rectangles in host coordinates. Scroll content is wrapped by the native scroll view plus an ElwindUI content root; the public content tree remains backend-neutral.

Native control properties are synchronized from local, text-style, and Theme revisions. Clearing a Theme-backed property restores system font, label color, appearance, or other AppKit default instead of assigning a common hard-coded value.

`NSSecureTextField` uses the system secure-entry font cascade. Ordinary text-style synthesis is not applied when it can break secure mask glyphs.

## Rendering and cache lifetime

Render groups replay into Core Graphics/layers with balanced clip, transform, and opacity state. Layer/image resources are owned by the corresponding render node and pruned when reconciliation removes or deactivates it.

Memory measurement reports are Issue evidence, not durable architecture. Durable cache ownership decisions belong here; current measured results belong in backend status.
