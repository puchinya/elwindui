# Custom controls runtime design

`elwindui-custom-controls` is a component-layer prerequisite for Docking. It
uses Core's tree, lifecycle, property, layout, render, and routed-event
machinery. There is no second observable system and no backend-specific code.

## Component composition and ownership

`CustomTabView` and `CustomSplitter` are `#[component(inherits Control)]`
controls. `CustomTabViewItem` is a
`#[component(inherits ContentControl)]`. Their authored `body: view!` is the
default visual composition; low-level behavior is expressed with the existing
component override surface where that surface is available.

`CustomTabView` exposes the established `ListExt<dyn CustomTabViewItemExt>`
surface and keeps an ordered typed item list. `set_children` is the concrete
replacement convenience path; the erased list implementation validates that
incoming values are actual `CustomTabViewItem` instances before transferring
the Rc. Each attached item, its header
`TextBlock`, and its optional `IconSourceElement` are Visual children owned by
the tab view. Reconciliation clears old entries and weak metadata callbacks
before attaching the new ordered set. An item already owned elsewhere, or the
same item appearing twice, is a programming error and is rejected immediately.
Selection only changes arrangement/visibility state; it never detaches,
reparents, or unmounts item content. Removal detaches without destroying the
subtree.

`CustomTabViewItem` delegates logical content entirely to `ContentControl` and
does not add another content store. Header/closable changes use equality guards;
icon changes may notify even when `IconSource` has no semantic equality.

## Private tab chrome

The tab surface uses these private logical-pixel metrics:

```text
strip 32, horizontal padding 10, element gap 6, icon 16,
close slot 20, close glyph 10, drag threshold 4, selected indicator 2
```

Labels and icons are retained per item and have `hit_test_visible = false` so
the self-drawn tab surface receives pointer routing. Icons are always realized
through Core `IconSourceElement`; SystemIcon canonical geometry remains private
to Core. The close mark is two private `RenderContext::draw_line` calls, not a
public icon type and not `SystemIcon::Delete`.

Measurement reserves the close slot for both `Always` and `OnPointerOver`.
Arrangement places headers left-to-right, keeps the selected item in the
content rectangle, zero-arranges unselected items, and clips each item. Top and
Bottom move the 32-pixel strip without changing content ownership. No overflow
or scrolling subsystem is introduced.

## Selection and gestures

`selected_index` has a source-to-target setter with no echo. User selection is a
separate target-to-source operation and invokes its callback once only when the
value changes. An out-of-range source value is preserved and produces no
selected content.

Pointer selection is interpreted only by the left-press handler. The close
rectangle is resolved before the header, so a close press never also selects a
tab; routed tap delivery is not a second selection path. Hovered close
affordances update only the paint state and use equality-guarded render
invalidation (not measure invalidation).

Tab presses and splitter presses establish gesture state before external
callbacks. State is cleared before completion/cancellation callbacks. When a
drag-start callback mutates the child list, the gesture is re-read by item
identity and the moved index is resolved again; removal emits only canceled
completion, while reordering reports the new index. When a cancellation
completion callback replaces children, reconciliation restarts from the
current property value instead of continuing with a stale snapshot. Tab
dragging starts at 4 logical pixels; splitter orientation is frozen at press
time and zero deltas are suppressed. `screen_position` is passed through
unchanged. The Core `PointerDispatcher` owns implicit capture, while
`on_pointer_canceled` terminates active gestures without synthetic release.

## Lifetime and prerequisite boundaries

Routed handlers and metadata callbacks capture only `Weak` owners. Dropping a
tab view therefore releases its item/chrome graph without a strong callback
cycle. Abnormal cancellation and capture-loss semantics remain the common Core
prerequisite from Issue #179/PR #181. Docking is a separate downstream crate and
is not referenced here.
