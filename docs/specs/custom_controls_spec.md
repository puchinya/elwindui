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
pub struct CustomGridSplitter { /* ... */ }
```

They use the existing `#[component]` and `template_view!` composition
mechanisms. Their appearance is authored as a default template subtree made
from ordinary visual primitives; the template frontend shares the ordinary
`view!` grammar. They do not introduce new `#[class]` controls, native
TabView/SplitView wrappers, or backend-native public types, and these controls
do not emit chrome directly from a `render()` override.

The generated class shape of a composed custom control carries its own public
`#[prop]` and `#[content]` metadata across crate boundaries. Consequently,
ordinary `view!` callers can use a custom control's typed properties and bare
children declaratively; computed/state fields remain implementation details and
are not exposed as writable construction properties.

## CustomTabView

`CustomTabView` owns an ordered typed `#[content(children)] children` list of
`Rc<CustomTabViewItem>`. Its public properties are:

- `selected_index: usize`, default `0`, TwoWay;
- `tab_strip_position: TabStripPosition`, default `Top`;
- `compact: bool`, default `false`;
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

The default presentation follows standard WinUI 3 TabView: a 40 logical-pixel
strip includes an 8-pixel outer inset and a 32-pixel item header. Normal headers
use a 100–240-pixel width range, constrained by the available strip when narrower.
Titles use 12-pixel type and selected titles are semibold. Closable headers use
8/4-pixel leading/trailing insets and a 32×24 close-button slot. Selected outlines
join the content edge; hover and pressed actions use subtle backgrounds. Colors
remain legible under platform light and dark themes.

Docking uses a framework-internal connected presentation, preserving its
WinUI.Dock decoration. In that presentation, `TabStripPosition::Top` reserves
a 32 logical-pixel strip above content;
`Bottom` reserves it below, except that a Bottom view with exactly one item
collapses the strip to zero height. Both positions use the same strip and
header height regardless of `compact`. Selected content occupies the remaining rectangle;
unselected items remain Visual children, are arranged to `0 x 0`, and are clipped.
Header widths reserve the same close slot for `Always` and `OnPointerOver`, so
hover does not resize a tab. Compact width measurement includes the title,
leading/trailing insets, icon, and reserved action slot. A measured compact tab
must retain enough width to show its title rather than collapsing to icon-only
or action-only chrome.

The connected strip baseline uses a six-logical-pixel leading rule and continues to the
right of the compact tab headers. Header text follows the WinUI.Dock header
insets: 12 logical pixels at the leading edge and 8 at the trailing edge. The
selected header has a one-pixel rounded top outline whose lower edge meets the
selected-content frame; its seam is filled by the frame color. Bottom position
mirrors the strip/header relationship while retaining upright text.

The selected header joins the selected-content frame with the corresponding
rounded outline treatment. Unselected headers retain their separator and
bottom-line structure; pointer-over presents the tab-header hover background
without changing selection. These visual states do not replace wrappers or page
content.

In the connected presentation, `compact = false` distributes available tab-strip width among the current items,
with each header capped at 200 logical pixels. `compact = true` measures each
header from its content and applies the same computed maximum width; long titles
therefore do not take the entire strip. If compact header widths together exceed
the finite strip width, Arrange reduces them proportionally to fit the strip and
clips each header's content to its own bounds. Compact mode changes width behavior
only, not strip height, header height, selection, or content arrangement.

The default template is a `Grid` containing a private non-rendering tab-strip
presenter and a private non-rendering content presenter. The strip uses the
existing `HorizontalLayout` semantics. A private strip host places the leading
rule, compact tab headers, and trailing baseline in that order. Top places the
strip in row 0 and Bottom places it in row 1; the other row is the
selected-content presenter and its rounded frame.

## CustomTabViewItem

`CustomTabViewItem` inherits `ContentControl` and exposes:

- `header: String`, default `""`;
- `icon: Option<elwindui::core::graphics::IconSource>`, default `None`;
- `closable: bool`, default `true`;
- inherited `content` as the single logical content element.

The item’s authored default template subtree is the tab header: it contains a
`TextBlock`, an optional `IconSourceElement`, a fixed close slot, and a
selected-frame slot. The inherited `content` is not rendered by
the header. A private content presenter owns the visual presentation of all
current item contents while preserving each item as the logical owner;
selection only changes arrangement and never reparents content.
The default item uses a `32`-pixel header with no separate indicator track.
In Docking's connected presentation, the item header tracks are `30` logical pixels for the header and `2` for the
indicator at `Top`, and `2` for the indicator followed by `30` for the header at
`Bottom`; the total item height remains `32` for both compact and non-compact
presentation.

The default close affordance is a private composed component using a 32×24-pixel
slot preceded by a 4-pixel gap and a vector close glyph; connected chrome uses
a 24×24-pixel slot matching WinUI.Dock's 6-pixel action padding and 12-pixel icon box.
`Always` and `OnPointerOver` reserve the same
slot width; `Never` removes the slot. Close press/release is handled by that
private visual through Core routed input and implicit capture.

Equal `header` and `closable` assignments are no-ops. Metadata updates refresh
the owning tab through a private weak callback. `IconSource` values are realized
only by Core's `IconSourceElement`; user images are not recolored and no
SystemIcon geometry is copied into this crate.

## CustomGridSplitter

`CustomGridSplitter` inherits `Control`, declares no child collection, and is a
backend-neutral composed control. Its public properties are
`resize_direction: GridResizeDirection` (default `Auto`),
`resize_behavior: GridResizeBehavior` (default `BasedOnAlignment`),
`parent_level: usize` (default `0`), `drag_increment: f32` (default `1.0`),
and `keyboard_increment: f32` (default `8.0`). There is no orientation
property or compatibility alias.

At the beginning of each transaction the splitter resolves and freezes the
target visual ancestor, its parent Grid, the attached row/column index, the
affected pair, the active direction, the exact track definitions, resolved
track sizes, and effective track constraints. `Auto` chooses columns when
horizontal alignment is not `Stretch`, rows when vertical alignment is not
`Stretch`, then columns when arranged width is no greater than height, and
rows otherwise. `BasedOnAlignment` maps the non-stretch edge to the adjacent
pair and centered/stretch alignment to the previous-and-next pair. Invalid
ancestors, Grids, resolved sizes, or pair indices create no transaction.

The splitter owns live Grid mutation. Pointer deltas are cumulative from the
original press, truncated to the effective drag increment, clamped by both
affected tracks' Grid-owned min/max constraints, and derived from the original
track snapshot. Fixed/Auto pairs become Fixed when resized; mixed Star pairs
retain their Star side; Star/Star pairs retain Star definitions and use
baseline resolved sizes as their weight basis. Pointer cancellation restores
the exact original definitions before completion notification. A normal
completion keeps the resized definitions.

`CustomGridSplitter` is focusable. Relevant arrow keys create one atomic
keyboard transaction using the same resolution, constraint, and mutation engine;
keyboard positions are `None`, and keyboard input is ignored while a pointer
transaction is active. Invalid increments fall back to `1.0` and `8.0`.

The public notifications are `GridSplitterResizeStartedEventArgs`,
`GridSplitterResizeDeltaEventArgs`, and `GridSplitterResizeCompletedEventArgs`,
with `set_on_resize_started`, `set_on_resize_delta`, and
`set_on_resize_completed`. Notifications identify direction, target/sibling
indices, input kind, optional positions, and the effective cumulative delta.
Grid mutation or rollback, session update/clear, and then notification are the
required ordering. The default template is ordinary composed chrome with a
centered 24-by-4 logical-pixel grip for `Rows` and a 4-by-24 grip for `Columns`.
The grip stays inside the parent-assigned splitter hit target. Docking creates a
12-logical-pixel inter-pane gap with `Grid.column_spacing` or `Grid.row_spacing`
and places the 12-pixel hit target in that gap by translating the splitter from
the following pane track. This does not add a separate splitter track. `Auto`
uses a centered six-by-six grip until an explicit direction is selected. The presentation follows
the WinUI 3 CommunityToolkit Sizers `GridSplitter`: at rest the surface is transparent and the grip
uses the control strong fill with a 2-pixel radius; pointer-over fills the whole hit target with the
subtle secondary fill and pressed/dragging with the subtle tertiary fill, each with a 4-pixel
corner radius, while the grip stays visible in the control strong fill. Colors use the Fluent light
and dark values of those roles. The control does not draw chrome through a `RenderContext` override.

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
