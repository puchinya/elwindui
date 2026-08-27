# Custom controls runtime design

`elwindui-custom-controls` is the component-layer prerequisite for Docking. It
uses Core’s tree, lifecycle, property, layout, render-tree, and routed-event
machinery. There is no second observable system and no backend-specific code.

## Templated component architecture

`CustomTabView` and `CustomSplitter` are
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
introducing a second content property. The component does not use a
`header_root` workaround.

The tab view template is a `Grid` with two rows. The private
`CustomTabStripPresenter` is a `HorizontalLayout` that owns the ordered item
controls; it retains an ordinary lifecycle-only `body: view!` because it has no
authored root of its own. The private `CustomTabContentPresenter` owns the visual presentation
of every current item content. Top uses `Fixed(32)` then `Star(1)`; Bottom uses
`Star(1)` then `Fixed(32)`, with the presenters’ attached `Grid::row` values
updated together.

## Visual ownership and reconciliation

`CustomTabView` strongly owns the ordered `Rc<CustomTabViewItem>` list. The
strip presenter attaches the item controls without recreating them. The item
content remains logically owned by `CustomTabViewItem`; the content presenter
attaches each current content visual exactly once and keeps it attached while
selection changes. The selected content is arranged to the full presenter
rect; unselected content is arranged to `0 x 0` and clipped. Replacing content
detaches the old visual before attaching the new one. Removing an item drops
its weak subscription and detaches its content without destroying external
`Rc` ownership.

All reconciliation validates one visual owner and duplicate item identity.
Callbacks and content subscriptions capture weak owners. If cancellation or a
content callback mutates the public children/content property, internal state
is committed before the callback and reconciliation restarts from the current
authoritative value.

## Header template and close affordance

Each item header is a composed `Grid` containing a header row with an optional
`IconSourceElement`, a bound `TextBlock`, and a private
`CustomTabCloseButton`. A `Rectangle` in a fixed two-pixel slot is the selected
indicator; the slot remains present for unselected items. The close helper uses
a fixed 20-pixel slot and a composed `TextBlock` `×` glyph. `Always` and
`OnPointerOver` reserve identical width; only glyph visibility changes for
hover. `Never` collapses the slot. No SystemIcon geometry or direct close-X
drawing is duplicated here.

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
or cancellation emits one canceled completion. Splitter orientation is frozen
at press time, deltas are incremental on the active axis, and completion clears
state before invoking callbacks.

## Scope boundary

Common pointer cancellation/capture-loss semantics are owned by Issue #179/PR
#181. These controls consume Core cancellation events and do not add capture
APIs. Docking remains a downstream crate with no dependency from this crate.
