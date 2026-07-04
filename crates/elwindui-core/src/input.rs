use crate::focus::ElementId;
use crate::painter::Point;

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

/// Modeled on WinUI3's routed events: hit-testing picks the deepest element, `Preview*` variants
/// tunnel root-to-target, plain variants bubble target-to-root, and `handled` stops propagation.
/// Unlike `LayoutNode`, this necessarily dispatches over `dyn Element` since the tree is
/// heterogeneous by construction (see docs/elwindui_gui_framework_design.md §2.11).
pub trait InputRouter {
    fn hit_test(&self, root: &dyn crate::Element, at: Point) -> Option<ElementId>;
    fn dispatch(&mut self, root: &dyn crate::Element, event: PointerEvent);
}
