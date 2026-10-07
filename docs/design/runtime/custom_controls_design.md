# Custom controls runtime design

`elwindui-custom-controls` is the component-layer prerequisite for Docking. It
uses Core’s tree, lifecycle, property, layout, render-tree, and routed-event
machinery. There is no second observable system and no backend-specific code.

## Templated component architecture

`CustomTabView` and `CustomGridSplitter` are
`#[component(inherits = Control)]` controls. `CustomTabViewItem` is a
`#[component(inherits = ContentControl)]`. Their authored
`template: template_view! { ... }` is the default visual composition, using the
same shared view grammar as ordinary `view!`. Standard `Grid`, `HorizontalLayout`,
`Rectangle`, `TextBlock`, and `IconSourceElement` nodes provide the chrome.
None of these controls, nor their private presenters, implements `render()` to
draw chrome through `RenderContext`.

`CustomTabViewItem` keeps the inherited `ContentControl::content` as its single
logical page-content property. Its authored `template_view!` is header
presentation only and is installed through the Control template-root path,
without replacing the inherited `#[content(content)]` destination or
introducing a second content property. The template root and the inherited
logical content slot therefore remain separate ownership paths.

The generated class declaration forwards the component's own `#[prop]` fields
and `#[content]` designation into the cross-crate class-shape macro. This is
metadata transport only: `#[computed]`, `#[state]`, and environment fields are
not exposed as ordinary writable properties, and no runtime property registry
is introduced. Generated component setters retain their owned Rust field types;
the shape metadata marks that conversion boundary separately from the borrowed
string setters of hand-written builtins.

Each `#[component]` in `elwindui-custom-controls` is maintained in its own source
file. `lib.rs` is the public facade and module root. `types.rs` contains only
the public shared value/event types and their public aliases; component- and
presenter-private state is owned by the implementation file that uses it:
`custom_tab_view.rs` owns tab gesture state and item pointer events,
`custom_tab_content_presenter.rs` owns `ContentEntry`, and
`custom_grid_splitter.rs` owns the private resize session. Those implementation modules stay
private, and their state types are not crate-root API. The facade lists the
intended public `types.rs` names explicitly rather than re-exporting the module
wildcard. Non-component cross-cutting implementation support is limited to
`support.rs`.

The generic tab view uses a 40-pixel strip row (8-pixel outer inset and 32-pixel
item); Docking uses the connected 32-pixel presentation. The doc-hidden `set_on_tab_pressed` integration hook reports every left
header press, including one on the selected header that changes no selection. One doc-hidden
`CustomTabView::set_connected_chrome` integration setter propagates presentation
to retained strip/items without exposing a DSL property or depending on Docking.
An item moved to a generic host resets this state. The presentation changes
dimensions and paint, never page ownership, callback authority or selection
invalidation. Normal generic widths are 100–240 pixels and close actions are
32×24; connected widths retain the 200-pixel maximum. Header outlines are
independently constructed vector geometry rather than copied native XAML paths.
Fixed edge pieces preserve corner radii while the center track stretches.
The generic strip host starts with an 8-pixel leading inset, rounds equal header
widths down to whole pixels so the edge pieces never meet on a fractional pixel
(where antialiasing shows a seam), and arranges each generic header 4 pixels wider
on both sides. That overhang carries the selected outline's feet past the logical
edge; header insets, hover fill, intrinsic width and insertion boundaries subtract
it, so content and insertion geometry keep their logical positions; within the
4-pixel overlap between neighbours the later header receives pointer input. Both
presentations retain their separate radii and seam dimensions. Docking installs
a weak document pin callback on each stable item; generic hosts hide that action,
and close/pin input is consumed before the tab gesture route. Application
foreground environment changes select the light/dark palette through component
notification rather than replacing items or retaining stale colors. Collapsed
presentation subtrees do not invalidate a measured strip pass merely because
their unmeasured descendants have no metrics; making them visible still
invalidates measurement through the normal visibility setter.
With connected (Docking) chrome and a visible strip, the content frame is open on the strip side:
the strip's baseline rule is that edge, as in WinUI.Dock's content border
(`BorderThickness="1,0,1,1"`, `CornerRadius="0,0,4,4"`). A private `Image`-derived frame drawn above
the page paints the sides and the far edge with square strip-side corners; its vector contour is
rebuilt during paint only when its size, side or separator brush changes, and the
`#[environment(separator)]` field re-records it on theme changes. A collapsed strip and the generic
TabView keep the closed rounded frame. Docking marks its active group with the doc-hidden
`set_active_chrome`, which switches the connected strip rules and both frames to the accent, and
places its content header with the doc-hidden `set_content_header` in an Auto row of the content
area above the page, inside the frame.
The content presenter retains the last arranged content size as well as selected
identity. A viewport/style change with the same selection rearranges the selected
page once; it does not revisit unchanged hidden pages. Selection-only updates
continue to arrange only the previous and current page.
The same-size shortcut also requires valid arrangement throughout the selected
page's participating visual subtree. A subtree invalidated after a splitter
commit must be arranged again even when the final size matches the last preview.

The connected tab view template is a `Grid` with a 32-pixel strip row and a content row.
The default tab-title template uses a 12-pixel font, matching the pinned WinUI TabView header
resource; page text and bottom content-header text keep their independent inherited styles.
Its declarative content field is exactly `#[content(children)] children:
Vec<Rc<CustomTabViewItem>>`. The private `CustomTabStripPresenter` is a
`HorizontalLayout` that owns the ordered item controls; it retains an ordinary
lifecycle-only `body: view!` because it has no authored root of its own. A strip
host places a 6-pixel leading baseline segment beside one bounded star track.
The remaining baseline spans that track behind the presenter, which measures
and arranges its headers within the finite width. This preserves equal widths
for non-compact headers, allows an empty strip to retain its full hit area, and
shows the baseline after compact headers. The private
`CustomTabContentPresenter` owns the visual presentation of every current item
content, and a rounded frame composes its outer border with the selected tab's
top outline. Top uses `Fixed(32)` then `Star(1)`; Bottom uses `Star(1)` then
`Fixed(32)`, except a Bottom view with exactly one item uses `Fixed(0)` for the
strip row. The presenters’ attached `Grid::row` values are updated together,
leaving the selected content in the full available rectangle when that strip
collapses.

The strip presenter derives each pass's item width from that pass's available or final width and
passes the result to child `measure`/`arrange`. It does not write persistent item `min_width` or
`max_width` properties during layout: those setters invalidate measure, and deriving their values
from transient parent constraints can feed back when the retained tree alternates between bounded
and unbounded measurements.

Measure and allocation are separate. Measure measures each visible header once and retains its
natural width: compact headers are measured against a fixed ceiling (200 pixels connected,
240 generic) and read their natural width from the header row's retained child measurements, with no
second measurement walk; other headers are measured at their equal slot. The retained pass records
the ordered item identities, the measurement inputs (ceiling or slot, height, compact/connected mode,
strip position, close-button presentation) and each header subtree's measured sizes and layout
participation. A later Measure with the same inputs and unchanged subtrees reuses the natural widths,
so a strip-width-only change does not remeasure compact headers; a header text, visibility or marker
change invalidates its subtree and the next Measure refreshes it. Arrange never measures: it
allocates from the retained natural widths (each capped by the current slot, or the equal slot) and,
if compact widths exceed their finite strip, scales them proportionally to fit the content span.
This keeps Core's unconditional `UIElement.measure` semantics and the compact/non-compact width
rules. Each header uses a Grid with a Star title column and
Auto marker/icon/action columns between fixed edge insets, so Arrange constrains the title to
the remaining tab width instead of preserving an unbounded horizontal-stack width. Header bounds
also clip their chrome; labels and action slots cannot paint over neighboring tabs.

## CustomGridSplitter transaction ownership

`CustomGridSplitter` is the only owner of live Grid mutation for splitter input.
At transaction start it walks the visual-parent chain according to
`parent_level`, resolves the target's resize Grid, and reads the target placement
from its attached `Grid::row` or `Grid::column`. Normally the resize Grid is the
target's visual-parent Grid. Docking's full-span splitter overlay supplies a
weak reference to its pane Grid, allowing the overlaid hit target to resize the
Grid that owns the split tracks without extending that Grid's lifetime. The
splitter snapshots the exact active-axis definitions, the Grid-owned
constraints, and the authoritative resolved sizes from that Grid. Direction,
behavior, indices, and effective increments are frozen in the private session.

Pointer moves calculate one cumulative axis delta from the original press,
truncate it to the effective drag increment, clamp it using the two baseline
actual sizes and their effective min/max values, and derive a complete preview
from the baseline definitions. The pure definition transformation preserves
Star/Star semantics by using every Star track's baseline resolved size as its
weight basis. A valid preview updates definitions, invalidates the Grid, and
uses the existing interactive relayout path before the delta callback is
invoked. The callback is therefore a notification and cannot be the owner of
the preview.

Cancellation clears the session before restoring the exact original
definitions, relayouts the Grid, and then emits one canceled completion.
Release applies the final cumulative position if needed, clears the session,
keeps the preview, and emits one non-canceled completion. Late release or
cancellation is a no-op. Keyboard arrows call the same pure transformation in
one atomic transaction, with no pointer positions; pointer-active sessions
suppress keyboard resizing. Routed handlers and callback closures use weak
owners, and mutable session/Grid borrows are released before notifications.

The splitter's appearance is also composed from ordinary template visuals. Its
centered grip is 24 by 4 logical pixels for row resizing and 4 by 24 for column
resizing; the control fills its hit target. Docking positions a 12-pixel hit
target in a 12-pixel `Grid` row/column spacing gap by placing it at the following
pane track and translating it -12 pixels along the split axis. There is no extra
splitter track; the gap remains between adjacent pane bounds. `Auto` keeps a
centered six-by-six grip until an explicit direction is selected. The template
is a single-cell Star Grid holding a full-size state background (4-pixel corner)
under the grip, following the WinUI 3 CommunityToolkit Sizers `GridSplitter`:
the background is transparent at rest, uses the subtle secondary fill on
pointer-over or focus and the subtle tertiary fill while pressed, and the grip
keeps the control strong fill. Light/dark values come from the live foreground
the same way as the custom tab palette. `measure_override` reports the grip's
desired size so the background never inflates the splitter's natural size.
These visual states are private component state and do not change the resize
transaction or public API. No
backend cursor, native GridSplitter wrapper, or VisualStateManager is involved.

## Visual ownership and reconciliation

`CustomTabView` strongly owns the ordered `Rc<CustomTabViewItem>` list. The
strip presenter attaches the item controls without recreating them. The item
content remains logically owned by `CustomTabViewItem`; the content presenter
attaches each current content visual exactly once and keeps it attached while
selection changes. `CustomTabView` caches weak references to its private strip and content
presenters after template application. When item identity/order is unchanged, selection and
presentation updates use those references without rewriting presenter item lists, repeating
visual-tree discovery, or entering structural validation/reconciliation. Structural changes still
validate duplicate identity and visual-parent ownership before rebinding. The selected content is
arranged to the full presenter rect; unselected content is arranged to `0 x 0` and clipped.
Replacing content detaches the old visual before attaching the new one, marks one structural
geometry pass, and preserves the presenter and unrelated page entries. Removing an item drops
its weak subscription and detaches its content without destroying external
`Rc` ownership.

The content presenter measures only the selected page. A structural item/content change marks a
full hidden-page zero-arrange pass for the next arrange; a later selection-only arrange updates
only the previous and current selected pages. Hidden pages remain attached and subscribed, but
retention does not cause them to be measured on the selection hot path.

All reconciliation validates one visual owner and duplicate item identity.
Callbacks and content subscriptions capture weak owners. If cancellation or a
content callback mutates the public children/content property, internal state
is committed before the callback and reconciliation restarts from the current
authoritative value.

## Header template and close affordance

Each item header is a composed `Grid` containing a header row with 12-pixel leading and 8-pixel
trailing insets, an optional `IconSourceElement`, a left-aligned bound `TextBlock`, a private
`CustomTabCloseButton`, and a separator. A 4-by-16 active marker with a two-pixel corner radius is
owned by an explicitly active item header, after the 12-pixel leading inset and before its title.
When inactive, the marker and its following spacer are collapsed; compact width measurement
includes the marker slot only while active, along with measured title text, so a document label
remains visible at the arranged width. The marker is not arranged by a group overlay over page
content. A
`Rectangle` in a fixed two-pixel seam slot matches the content background and covers the selected
header's lower stroke so the rounded top outline joins the content frame. Unselected headers keep
their bottom separator and pointer-over background. The close helper uses a vector glyph in a
32-by-24-pixel button with a 4-pixel leading gap for generic tabs, or a 24-by-24-pixel button for
Docking's connected chrome. `Always` and `OnPointerOver` reserve identical width; hover changes
the glyph's paint without changing that slot. The glyph stays structurally present with a
transparent brush while hidden. `Never` collapses the slot and invalidates normal measure/arrange
state. The private vector helper owns the close glyph geometry.

The item binds routed pointer handlers on its header root. The close helper
handles its own press/release first and marks the routed event handled, so a
close press cannot select or start a tab drag. Core’s `PointerDispatcher`
provides implicit capture; release outside and cancellation clear the helper
state without requesting close. Parent callbacks carry item identity rather
than cached rectangles or indices.

## Selection and gestures

Source `selected_index` assignments do not echo. User selection writes back
once only when the numeric value changes. Out-of-range values are preserved and
produce no selected page. Tab drag state is owned by `CustomTabView`; item
identity is the authority and indices are resolved at dispatch time. A
threshold-crossing callback sets `Dragging` before invoking external code,
then re-reads gesture state and current index before emitting `moved`. Removal
or cancellation emits one canceled completion. Grid splitter direction,
behavior, indices, and increments are frozen per transaction; pointer deltas
are baseline-derived cumulative values, while keyboard input uses the same
engine. Grid mutation or rollback and session clear precede notifications.

## Scope boundary

Common pointer cancellation/capture-loss semantics are owned by Issue #179/PR
#181. These controls consume Core cancellation events and do not add capture
APIs. Docking remains a downstream crate with no dependency from this crate.
