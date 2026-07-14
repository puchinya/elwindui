use crate::base::Point;
use crate::ui::UIElementExt;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEventKind {
    Down,
    Move,
    Up,
}

/// See docs/elwindui_spec.md 付録G.6, 付録K.1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub kind: PointerEventKind,
    pub position: Point,
    pub handled: bool,
}

/// Passed to every handler `elwindui_core::ui::dispatch_routed` calls along a bubble path —
/// pure propagation control, deliberately without a payload (`dispatch_routed`'s own `payload: &T`
/// argument carries that, so this stays the same shape for every `#[routed]` field regardless of
/// its own callback signature). A handler sets `handled` to stop further bubbling — WinUI3's
/// `RoutedEventArgs.Handled`. See docs/elwindui_spec.md 4章 (`#[routed]`).
#[derive(Debug, Default)]
pub struct RoutedEventArgs {
    pub handled: Cell<bool>,
}

/// Modeled on WinUI3's routed events: hit-testing picks the deepest element, and bubbling from it
/// (or from any other known element, e.g. a native leaf's own click) follows real parent
/// back-references (`UIElement::parent`, WinUI3's `_parent`) up to the root, stopping as soon as a
/// handler sets `handled` — no tree search needed to bubble, and no dependence on the tree having
/// been built by a single static `.elwind` traversal (a dynamically-assembled one, e.g. `TabView`'s
/// `items_source`, works identically). `hit_test`/`dispatch` operate over `UIElement` (not the
/// separate `Element`/`ElementId` used for `#[id(...)]` name resolution) since only `UIElement`
/// carries the measured/arranged geometry (`measure_override`/`arrange_override`) hit-testing
/// needs — see `elwindui_core::ui::hit_test`/`dispatch_routed`, which this trait wraps.
pub trait InputRouter {
    fn hit_test(&self, root: &Rc<dyn UIElementExt>, at: Point) -> Option<Rc<dyn UIElementExt>>;
    fn dispatch(&mut self, root: &Rc<dyn UIElementExt>, event: PointerEvent);
}
