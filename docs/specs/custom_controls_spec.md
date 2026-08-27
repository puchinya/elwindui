# Custom controls specification

This document is the normative public contract for the reusable controls in
`elwindui-custom-controls`. It is a prerequisite for Docking; it does not define
Docking layout or persistence.

## Authoring and inheritance

The controls are ordinary authored components:

```rust
#[elwindui::component(inherits Control)]
pub struct CustomTabView { /* ... */ }

#[elwindui::component(inherits ContentControl)]
pub struct CustomTabViewItem { /* ... */ }

#[elwindui::component(inherits Control)]
pub struct CustomSplitter { /* ... */ }
```

They use the existing `#[component]` and `template_view!` composition
mechanisms. Their appearance is authored as a default template subtree made
from ordinary visual primitives; the template frontend shares the ordinary
`view!` grammar. They do not introduce new `#[class]` controls, native
TabView/SplitView wrappers, or backend-native public types, and these controls
do not emit chrome directly from a `render()` override.

## CustomTabView

`CustomTabView` owns an ordered typed `#[content(children)]` list of
`Rc<CustomTabViewItem>`. Its public properties are:

- `selected_index: usize`, default `0`, TwoWay;
- `tab_strip_position: TabStripPosition`, default `Top`;
- `close_button_presentation: CloseButtonPresentation`, default `Always`.

The ordered-list surface is `children(&self) -> &dyn
ListExt<dyn CustomTabViewItemExt>`, with the established `ListExt` mutation
operations. `set_children(Vec<Rc<CustomTabViewItem>>)` is the concrete
replacement convenience API used by authored and programmatic callers.

The source setter stores a changed `selected_index` without invoking the
write-back callback. Equal assignments are no-ops. A valid user selection stores
the new index and invokes `set_on_selected_index_change` exactly once; selecting
the already selected item is a no-op. An out-of-range source value is preserved
and means that no item has selected content. Child list mutations do not rewrite
the numeric selection or invoke the TwoWay callback.

`set_on_close_request` is advisory. A request for an item whose `closable` value
is `false` is rejected. `CloseButtonPresentation::Never` hides the pointer
affordance; it does not disable an application-issued close notification.
Accepted requests emit one `TabCloseRequestedEventArgs`/index notification;
the control never removes a child automatically.

Tab drag callbacks use `TabDragStartedEventArgs`, `TabDragMovedEventArgs`, and
`TabDragCompletedEventArgs`. Each carries the current child index, root-relative
position, and optional normalized logical-desktop `screen_position`; completion
also carries `canceled`. A left header press becomes a drag at 4 logical pixels.
The press below that threshold emits no drag callbacks. Core cancellation emits
one canceled completion and item removal cancels an active drag before detach.

`TabStripPosition::Top` reserves a 32 logical-pixel strip above content;
`Bottom` reserves it below. Selected content occupies the remaining rectangle;
unselected items remain Visual children, are arranged to `0 x 0`, and are clipped.
Header widths reserve the same close slot for `Always` and `OnPointerOver`, so
hover does not resize a tab.

The default template is a `Grid` containing a private non-rendering tab-strip
presenter and a private non-rendering content presenter. The strip uses the
existing `HorizontalLayout` semantics. Top places the strip in row 0 and
Bottom places it in row 1; the other row is the selected-content presenter.

## CustomTabViewItem

`CustomTabViewItem` inherits `ContentControl` and exposes:

- `header: String`, default `""`;
- `icon: Option<elwindui::core::graphics::IconSource>`, default `None`;
- `closable: bool`, default `true`;
- inherited `content` as the single logical content element.

The item’s authored default template subtree is the tab header: it contains a
`TextBlock`, an optional `IconSourceElement`, a fixed close slot, and a
`Rectangle` selected-indicator slot. The inherited `content` is not rendered by
the header. A private content presenter owns the visual presentation of all
current item contents while preserving each item as the logical owner;
selection only changes arrangement and never reparents content.
The item header tracks are `30` logical pixels for the header and `2` for the
indicator at `Top`, and `2` for the indicator followed by `30` for the header at
`Bottom`; the total item height remains `32`.

The default close affordance is a private composed component using a 20-pixel
slot and a `TextBlock` `×` glyph. `Always` and `OnPointerOver` reserve the same
slot width; `Never` removes the slot. Close press/release is handled by that
private visual through Core routed input and implicit capture.

Equal `header` and `closable` assignments are no-ops. Metadata updates refresh
the owning tab through a private weak callback. `IconSource` values are realized
only by Core's `IconSourceElement`; user images are not recolored and no
SystemIcon geometry is copied into this crate.

## CustomSplitter

`CustomSplitter` inherits `Control`, declares no child collection, and exposes
`orientation: Orientation`, default `Horizontal`. Horizontal panes use the X
axis and a 6-pixel width; vertical panes use the Y axis and a 6-pixel height.
`SplitterDragStartedEventArgs`, `SplitterDragDeltaEventArgs`, and
`SplitterDragCompletedEventArgs` carry root/screen positions, incremental and
cumulative logical-pixel movement, and a cancellation flag on completion.
Orientation is frozen at press time, zero deltas are suppressed, and Core
cancellation completes an active gesture with `canceled = true`.

The default splitter template is an orientation-dependent `Rectangle` with
the six-pixel natural thickness. It does not draw its line through a
`RenderContext` override.

## Ownership and input

Each content element has one Visual owner. Reconciliation detaches old tab items
before attaching replacements and rejects duplicate/already-owned items rather
than stealing them. Selection does not detach or unmount content; removal is a
detach only. Parent callbacks and routed handlers capture `Weak` references.

Pointer delivery uses Core's `PointerDispatcher` and routed events. The custom
controls do not implement native pointer capture or backend coordinate
conversion. `PointerEventArgs.position` remains root-relative and
`screen_position` is passed through unchanged.

## Scope boundary

The crate has no dependency on Docking and contains no Docking IDs, layout
models, floating windows, or persistence format. Common cancellation/capture-loss
semantics are owned by Issue #179/PR #181; these controls consume
`on_pointer_canceled`.
