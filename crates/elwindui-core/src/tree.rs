//! The framework-owned Visual tree, following WinUI3's `UIElement` hierarchy: `Box<dyn UIElement>`
//! nodes *are* the tree (no separate wrapper/enum type) — `NativeControl<H>` (`Button`/`TextArea`/
//! `MenuBar`/`TabView`, the "NativeControl" family), `TextBlock` (self-drawn primitive text),
//! `Shape` (`Rectangle`/`Ellipse`), `Stack` (`VerticalLayout`/`HorizontalLayout`), and `Control`
//! (a composable multi-part component) are all peer implementations of the same `UIElement` trait.
//! `Margin`/`HorizontalAlignment`/`VerticalAlignment` (`UIElementBase`) are common to every one of
//! them, applied generically by this module's `measure`/`arrange` (WinUI3's
//! `UIElement.Measure`/`Arrange` wrapping each type's own `MeasureOverride`/`ArrangeOverride`) —
//! see docs/elwindui_spec.md 付録H.2.
//!
//! `H` (whatever a backend uses as its native widget handle, e.g. `elwindui-backend-appkit`'s
//! `AnyView`) appears only on `NativeControl<H>` itself and on the functions that walk a tree
//! looking for one (`layout_tree`) — the `UIElement` trait and every other concrete type
//! (`Stack`/`Shape`/`TextBlock`/`Control`) are handle-agnostic, since they never hold one.
//!
//! `Window` is deliberately *not* a `UIElement` — like WinUI3's `Window`, it's a separate
//! top-level host that owns a `Box<dyn UIElement>` (its content) and drives `layout_tree` against
//! its own client area (see `elwindui-backend-appkit`'s `TreeHostView`).

use crate::layout::{
    align_within, grow_by_margin, shrink_by_margin, shrink_rect_by_margin, stack_arrange, stack_natural_size,
    HorizontalAlignment, LayoutNode, Orientation, Rect, Size, VerticalAlignment,
};
use std::any::Any;

/// Lets the generic tree-walker (`layout_tree`) downcast a `&dyn UIElement` to a concrete
/// `NativeControl<H>` to pull out its handle — the *only* place `native_handle`-style access
/// exists (deliberately not a method on `UIElement` itself: every other implementor would have to
/// carry a meaningless default for a concept that doesn't apply to it). Blanket-implemented for
/// every `'static` type, so no concrete `UIElement` impl needs its own boilerplate.
pub trait AsAny: Any {
    fn as_any(&self) -> &dyn Any;
}
impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The fields every `UIElement` carries (WinUI3's `FrameworkElement` base class, via composition
/// since Rust has no class inheritance — each concrete type embeds one of these and delegates
/// `UIElement::base`).
///
/// `data_context` (WinUI3's `FrameworkElement.DataContext`) is `Rc<dyn Any>`-erased like every
/// other cross-type-parameter value in this crate (see e.g. `elwindui-builtins::appkit::tab_view`'s
/// `erase_tabs`) — it drops `UIElementBase`'s former `Copy`/`PartialEq` derives (`Rc<dyn Any>`
/// supports neither), which nothing in the tree relied on.
#[derive(Clone)]
pub struct UIElementBase {
    pub margin: f32,
    pub horizontal_alignment: HorizontalAlignment,
    pub vertical_alignment: VerticalAlignment,
    pub data_context: Option<std::rc::Rc<dyn Any>>,
}

impl std::fmt::Debug for UIElementBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UIElementBase")
            .field("margin", &self.margin)
            .field("horizontal_alignment", &self.horizontal_alignment)
            .field("vertical_alignment", &self.vertical_alignment)
            .field("data_context", &self.data_context.is_some())
            .finish()
    }
}

impl Default for UIElementBase {
    fn default() -> Self {
        UIElementBase {
            margin: 0.0,
            horizontal_alignment: HorizontalAlignment::Stretch,
            vertical_alignment: VerticalAlignment::Stretch,
            data_context: None,
        }
    }
}

/// The common interface every element in the Visual tree implements — `NativeControl<H>`,
/// `TextBlock`, `Shape`, `Stack`, and `Control` are all peers here, not variants of some enum.
/// New kinds (a future `Grid`, say) are added by implementing this trait; nothing here or in
/// `layout_tree` needs to change.
pub trait UIElement: AsAny {
    fn base(&self) -> &UIElementBase;
    fn margin(&self) -> f32 {
        self.base().margin
    }
    fn horizontal_alignment(&self) -> HorizontalAlignment {
        self.base().horizontal_alignment
    }
    fn vertical_alignment(&self) -> VerticalAlignment {
        self.base().vertical_alignment
    }
    /// WinUI3's `FrameworkElement.DataContext` — an ambient, type-erased data value an element
    /// carries (set explicitly via the `data_context:` common attribute, or populated internally by
    /// `TabView`'s `items_source` mode for each generated `TabViewItem`). `None` when unset.
    fn data_context(&self) -> Option<&std::rc::Rc<dyn Any>> {
        self.base().data_context.as_ref()
    }
    /// This element's own children (`&[]` for a leaf like `NativeControl`/`TextBlock`).
    fn children(&self) -> &[Box<dyn UIElement>];
    /// This element's own desired size, given `available` (margin already excluded by the caller)
    /// and its children's already-measured sizes (WinUI3's `MeasureOverride`).
    fn measure_override(&self, available: Size, child_sizes: &[Size]) -> Size;
    /// The rect to assign each child (in this element's own local coordinate space), given the
    /// final size this element itself was assigned (WinUI3's `ArrangeOverride`).
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect>;
    /// Content this element paints for itself, if any (`None` for pure layout containers like
    /// `Stack`, which only position children and draw nothing on their own account).
    fn paint(&self) -> Option<PaintKind> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintKind {
    Shape { kind: ShapeKind, fill: Option<String>, stroke: Option<String>, stroke_width: f32 },
    /// `TextBlock`'s self-drawn content. No font/size here yet (kept minimal for this pass) — a
    /// backend measures/renders the string itself (e.g. AppKit via `NSAttributedString`/
    /// `CATextLayer`), the same "elwindui-core doesn't know how to actually draw" split `Shape`
    /// already has with `CAShapeLayer`.
    Text { content: String, color: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeKind {
    RoundedRect { corner_radius: f32 },
    Oval,
}

/// `Button`/`TextArea`/`MenuBar`/`TabView` (the "NativeControl" family) — the only `UIElement`
/// with a real backend handle. Always a leaf as far as this tree is concerned: whatever lives
/// beneath it in its own backend-managed hierarchy (e.g. `TabView`'s tab-switching) is opaque here.
pub struct NativeControl<H> {
    pub base: UIElementBase,
    pub handle: H,
}

impl<H: LayoutNode + 'static> UIElement for NativeControl<H> {
    fn base(&self) -> &UIElementBase {
        &self.base
    }
    fn children(&self) -> &[Box<dyn UIElement>] {
        &[]
    }
    fn measure_override(&self, available: Size, _child_sizes: &[Size]) -> Size {
        self.handle.measure(available)
    }
    fn arrange_override(&self, _final_size: Size, _child_sizes: &[Size]) -> Vec<Rect> {
        Vec::new()
    }
}

/// `VerticalLayout`/`HorizontalLayout` — a thin wrapper around `elwindui_core::layout`'s
/// `stack_arrange`/`stack_natural_size` free functions.
pub struct Stack {
    pub base: UIElementBase,
    pub orientation: Orientation,
    pub spacing: f32,
    pub children: Vec<Box<dyn UIElement>>,
}

impl UIElement for Stack {
    fn base(&self) -> &UIElementBase {
        &self.base
    }
    fn children(&self) -> &[Box<dyn UIElement>] {
        &self.children
    }
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        stack_natural_size(self.orientation, self.spacing, child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        stack_arrange(final_size, self.orientation, self.spacing, child_sizes)
    }
}

/// `Rectangle`/`Ellipse`. Has no intrinsic size of its own — its natural size is the bounding box
/// of its children — and every child simply overlays its full bounds (no layout math *within* the
/// shape — a single content slot, not a container in its own right).
pub struct Shape {
    pub base: UIElementBase,
    pub kind: ShapeKind,
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: f32,
    pub children: Vec<Box<dyn UIElement>>,
}

impl UIElement for Shape {
    fn base(&self) -> &UIElementBase {
        &self.base
    }
    fn children(&self) -> &[Box<dyn UIElement>] {
        &self.children
    }
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        Size {
            width: child_sizes.iter().map(|s| s.width).fold(0.0_f32, f32::max),
            height: child_sizes.iter().map(|s| s.height).fold(0.0_f32, f32::max),
        }
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        vec![Rect { x: 0.0, y: 0.0, width: final_size.width, height: final_size.height }; child_sizes.len()]
    }
    fn paint(&self) -> Option<PaintKind> {
        Some(PaintKind::Shape { kind: self.kind, fill: self.fill.clone(), stroke: self.stroke.clone(), stroke_width: self.stroke_width })
    }
}

/// Self-drawn primitive text (WinUI3's `TextBlock`) — no native widget, unlike the native `Text`
/// this replaces. A leaf, like `NativeControl`.
pub struct TextBlock {
    pub base: UIElementBase,
    pub content: String,
    pub color: Option<String>,
}

impl UIElement for TextBlock {
    fn base(&self) -> &UIElementBase {
        &self.base
    }
    fn children(&self) -> &[Box<dyn UIElement>] {
        &[]
    }
    fn measure_override(&self, _available: Size, _child_sizes: &[Size]) -> Size {
        // `elwindui-core` has no font metrics of its own (measurement, like painting, is a
        // backend concern for self-drawn content — see `Shape`'s same split) — a rough per-
        // character estimate is enough to avoid collapsing to zero size; a backend may still
        // render a string that overflows this estimate.
        Size { width: self.content.chars().count() as f32 * 8.0, height: 16.0 }
    }
    fn arrange_override(&self, _final_size: Size, _child_sizes: &[Size]) -> Vec<Rect> {
        Vec::new()
    }
    fn paint(&self) -> Option<PaintKind> {
        Some(PaintKind::Text { content: self.content.clone(), color: self.color.clone() })
    }
}

/// A composable, multi-part component (WinUI3's `Control`) — Visually built from any number of
/// other `UIElement`s (`Stack`/`Shape`/`TextBlock`/`NativeControl`/other `Control`s), unlike
/// `Shape`'s single decorative content slot. `padding` shrinks the area its children are overlaid
/// into, the `Control`-level analog of `margin` on an individual element.
///
/// Scope note: this is intentionally minimal for now — `content_horizontal_alignment`/
/// `content_vertical_alignment` are stored but not yet consulted by `arrange_override` (each
/// child's *own* `horizontal_alignment`/`vertical_alignment`, applied generically by `arrange`
/// below, already governs its placement within the padded content area); template
/// replacement/Logical-tree wiring is future work (see `LogicalNode`).
pub struct Control {
    pub base: UIElementBase,
    pub padding: f32,
    pub content_horizontal_alignment: HorizontalAlignment,
    pub content_vertical_alignment: VerticalAlignment,
    pub children: Vec<Box<dyn UIElement>>,
}

impl UIElement for Control {
    fn base(&self) -> &UIElementBase {
        &self.base
    }
    fn children(&self) -> &[Box<dyn UIElement>] {
        &self.children
    }
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        let inner = child_sizes
            .iter()
            .fold(Size { width: 0.0, height: 0.0 }, |acc, s| Size { width: acc.width.max(s.width), height: acc.height.max(s.height) });
        grow_by_margin(inner, self.padding)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        let full = Rect { x: 0.0, y: 0.0, width: final_size.width, height: final_size.height };
        vec![shrink_rect_by_margin(full, self.padding); child_sizes.len()]
    }
}

/// The tree of component *references* as authored in `.elwind` (WinUI3's Logical tree) — distinct
/// from the Visual tree (`Box<dyn UIElement>`) `layout_tree` actually walks. A `Control` (or any
/// user-defined component) is a single `LogicalNode` here even though its Visual representation
/// may expand into many `UIElement`s. Reserved for future use by `elwindui_core::element`'s
/// `find_by_id`/`find_all` and template support — not yet produced by `elwindui-codegen`.
pub struct LogicalNode {
    pub type_name: String,
    pub children: Vec<LogicalNode>,
}

fn measure(elem: &dyn UIElement, available: Size) -> Size {
    let inner_available = shrink_by_margin(available, elem.margin());
    let child_sizes: Vec<Size> = elem.children().iter().map(|c| measure(c.as_ref(), inner_available)).collect();
    let desired = elem.measure_override(inner_available, &child_sizes);
    grow_by_margin(desired, elem.margin())
}

fn arrange<H: Clone + 'static>(elem: &dyn UIElement, allotted: Rect, natives: &mut Vec<(H, Rect)>, paints: &mut Vec<(PaintKind, Rect)>) {
    let slot = shrink_rect_by_margin(allotted, elem.margin());
    let slot_size = Size { width: slot.width, height: slot.height };
    let child_sizes_for_measure: Vec<Size> = elem.children().iter().map(|c| measure(c.as_ref(), slot_size)).collect();
    let desired = elem.measure_override(slot_size, &child_sizes_for_measure);
    let final_rect = align_within(slot, desired, elem.horizontal_alignment(), elem.vertical_alignment());
    let final_size = Size { width: final_rect.width, height: final_rect.height };

    if let Some(native) = elem.as_any().downcast_ref::<NativeControl<H>>() {
        natives.push((native.handle.clone(), final_rect));
    }
    if let Some(paint) = elem.paint() {
        paints.push((paint, final_rect));
    }

    let child_sizes: Vec<Size> = elem.children().iter().map(|c| measure(c.as_ref(), final_size)).collect();
    let child_rects = elem.arrange_override(final_size, &child_sizes);
    for (child, child_rect) in elem.children().iter().zip(child_rects) {
        let absolute_child_rect =
            Rect { x: final_rect.x + child_rect.x, y: final_rect.y + child_rect.y, width: child_rect.width, height: child_rect.height };
        arrange::<H>(child.as_ref(), absolute_child_rect, natives, paints);
    }
}

/// This element's natural (unconstrained) size — e.g. for a container that must report an
/// `intrinsicContentSize` to an Auto-Layout-managed ancestor (see `elwindui-backend-appkit`'s
/// `TreeHostView`) before it has ever actually been given a frame to lay out into.
pub fn natural_size(elem: &dyn UIElement) -> Size {
    measure(elem, Size { width: 0.0, height: 0.0 })
}

/// Recursively measures and arranges `root` against `available`, returning every `NativeControl<H>`
/// leaf (its handle cloned — cheap for a thin `Retained<NSView>`-style handle) paired with its
/// **absolute** rect, and every self-painting element's content paired with its own absolute rect.
/// A backend's host (see `elwindui-backend-appkit`'s `TreeHostView`) uses the first list to place
/// native subviews and the second to manage paint layers (e.g. `CAShapeLayer`s) — `elwindui-core`
/// itself knows nothing about `NSView`/`addSubview`/`CALayer`.
///
/// `H` only needs to be named here (and on `NativeControl<H>` itself) — every other `UIElement` is
/// handle-agnostic. The root's own `horizontal_alignment`/`vertical_alignment` default to
/// `Stretch` (`UIElementBase::default`), so it fills `available` unless a caller explicitly
/// overrides them — the same default every mainstream UI framework gives a top-level content
/// element (`Window.Content`, an HTML `<body>`).
pub fn layout_tree<H: Clone + 'static>(root: &dyn UIElement, available: Size) -> (Vec<(H, Rect)>, Vec<(PaintKind, Rect)>) {
    let mut natives = Vec::new();
    let mut paints = Vec::new();
    let allotted = Rect { x: 0.0, y: 0.0, width: available.width, height: available.height };
    arrange::<H>(root, allotted, &mut natives, &mut paints);
    (natives, paints)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug)]
    struct FakeHandle(&'static str, Size);

    impl LayoutNode for FakeHandle {
        fn measure(&self, _available: Size) -> Size {
            self.1
        }
        fn arrange(&mut self, _final_rect: Rect) {}
    }

    fn size(width: f32, height: f32) -> Size {
        Size { width, height }
    }

    fn native(name: &'static str, size: Size) -> Box<dyn UIElement> {
        Box::new(NativeControl { base: UIElementBase::default(), handle: FakeHandle(name, size) })
    }

    fn stack(orientation: Orientation, spacing: f32, children: Vec<Box<dyn UIElement>>) -> Box<dyn UIElement> {
        Box::new(Stack { base: UIElementBase::default(), orientation, spacing, children })
    }

    #[test]
    fn single_native_leaf_as_root_fills_available_space() {
        // The root's default alignment is `Stretch`, so it fills `available` regardless of its
        // own measured size — this matters for e.g. `TabView` (a native leaf) as `Window`'s
        // content: it must fill the window, not shrink to its own `fittingSize()`.
        let tree = native("a", size(10.0, 20.0));
        let (natives, paints) = layout_tree::<FakeHandle>(tree.as_ref(), size(200.0, 100.0));
        assert_eq!(natives, vec![(FakeHandle("a", size(10.0, 20.0)), Rect { x: 0.0, y: 0.0, width: 200.0, height: 100.0 })]);
        assert!(paints.is_empty());
    }

    #[test]
    fn nested_stack_accumulates_absolute_offsets() {
        // Vertical outer stack containing a native leaf, then a horizontal inner stack of two
        // native leaves — checks that the inner stack's children get *absolute* coordinates, not
        // coordinates relative to the inner stack alone. Every element here uses `Left`/`Top`
        // alignment explicitly (not the `Stretch` default) so each child keeps its own measured
        // size instead of filling its stack-allocated cross-axis slot — matching the old
        // `CrossAlign::Start` behavior this test used to exercise.
        fn leaf(name: &'static str, s: Size) -> Box<dyn UIElement> {
            Box::new(NativeControl {
                base: UIElementBase { margin: 0.0, horizontal_alignment: HorizontalAlignment::Left, vertical_alignment: VerticalAlignment::Top, ..UIElementBase::default() },
                handle: FakeHandle(name, s),
            })
        }
        fn start_stack(orientation: Orientation, spacing: f32, children: Vec<Box<dyn UIElement>>) -> Box<dyn UIElement> {
            Box::new(Stack {
                base: UIElementBase { margin: 0.0, horizontal_alignment: HorizontalAlignment::Left, vertical_alignment: VerticalAlignment::Top, ..UIElementBase::default() },
                orientation,
                spacing,
                children,
            })
        }

        let tree = start_stack(
            Orientation::Vertical,
            5.0,
            vec![
                leaf("top", size(50.0, 10.0)),
                start_stack(Orientation::Horizontal, 2.0, vec![leaf("left", size(20.0, 20.0)), leaf("right", size(30.0, 20.0))]),
            ],
        );

        let (natives, paints) = layout_tree::<FakeHandle>(tree.as_ref(), size(200.0, 200.0));
        assert!(paints.is_empty());
        assert_eq!(natives.len(), 3);
        assert_eq!(natives[0], (FakeHandle("top", size(50.0, 10.0)), Rect { x: 0.0, y: 0.0, width: 50.0, height: 10.0 }));
        // inner stack starts at y = 10 (top's height) + 5 (spacing) = 15
        assert_eq!(natives[1], (FakeHandle("left", size(20.0, 20.0)), Rect { x: 0.0, y: 15.0, width: 20.0, height: 20.0 }));
        assert_eq!(natives[2], (FakeHandle("right", size(30.0, 20.0)), Rect { x: 22.0, y: 15.0, width: 30.0, height: 20.0 }));
    }

    #[test]
    fn stretch_default_fills_the_cross_axis_slot() {
        // Unlike the previous test, this one leaves alignment at its `Stretch` default — each
        // leaf should fill the *entire* stack width (the cross axis, for a vertical stack), not
        // just its own measured width.
        let tree = stack(Orientation::Vertical, 0.0, vec![native("a", size(10.0, 20.0))]);
        let (natives, _) = layout_tree::<FakeHandle>(tree.as_ref(), size(200.0, 100.0));
        assert_eq!(natives[0].1, Rect { x: 0.0, y: 0.0, width: 200.0, height: 20.0 });
    }

    #[test]
    fn shape_reports_paint_and_overlays_children_at_its_own_absolute_rect() {
        let tree: Box<dyn UIElement> = Box::new(Shape {
            base: UIElementBase::default(),
            kind: ShapeKind::RoundedRect { corner_radius: 8.0 },
            fill: Some("#3498db".to_string()),
            stroke: None,
            stroke_width: 0.0,
            children: vec![native("label", size(40.0, 20.0))],
        });

        let (natives, paints) = layout_tree::<FakeHandle>(tree.as_ref(), size(100.0, 50.0));
        assert_eq!(paints.len(), 1);
        // As the root, the shape fills `available` (default `Stretch`, not its own shrink-wrapped
        // natural size); its child, per `Shape::arrange_override`'s "overlay at full bounds" rule
        // *and* its own default `Stretch` alignment, gets that same full rect.
        assert_eq!(paints[0].1, Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 });
        assert_eq!(natives[0].1, Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 });
    }

    #[test]
    fn empty_virtual_node_has_zero_size_and_no_leaves() {
        let tree = stack(Orientation::Vertical, 0.0, vec![]);
        let (natives, paints) = layout_tree::<FakeHandle>(tree.as_ref(), size(100.0, 100.0));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
    }

    #[test]
    fn margin_shrinks_the_slot_an_element_is_arranged_into() {
        let tree: Box<dyn UIElement> = Box::new(NativeControl {
            base: UIElementBase { margin: 10.0, ..UIElementBase::default() },
            handle: FakeHandle("a", size(10.0, 20.0)),
        });
        let (natives, _) = layout_tree::<FakeHandle>(tree.as_ref(), size(100.0, 100.0));
        assert_eq!(natives[0].1, Rect { x: 10.0, y: 10.0, width: 80.0, height: 80.0 });
    }

    #[test]
    fn non_stretch_alignment_keeps_the_elements_own_measured_size() {
        let tree: Box<dyn UIElement> = Box::new(NativeControl {
            base: UIElementBase { margin: 0.0, horizontal_alignment: HorizontalAlignment::Center, vertical_alignment: VerticalAlignment::Center, ..UIElementBase::default() },
            handle: FakeHandle("a", size(10.0, 20.0)),
        });
        let (natives, _) = layout_tree::<FakeHandle>(tree.as_ref(), size(100.0, 100.0));
        assert_eq!(natives[0].1, Rect { x: 45.0, y: 40.0, width: 10.0, height: 20.0 });
    }
}
