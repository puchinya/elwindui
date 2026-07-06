//! The framework-owned scene tree that finally realizes what docs/elwindui_spec.md 付録H.2
//! always intended: layout computed centrally here, with backends only ever asked to paint
//! already-computed absolute rects. Only genuinely native leaf widgets (`Button`/`TextArea`/
//! `Text`/`MenuBar`/`TabView`, the "NativeComponent" family) carry a real backend handle;
//! everything else (`VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`, and
//! anything added later) is a purely elwindui-side `Virtual` node with no native object at all.

use crate::layout::{stack_arrange, stack_natural_size, CrossAlign, LayoutNode, Orientation, Rect, Size};

/// A node in the scene tree. `H` is whatever a backend uses as its native widget handle
/// (`elwindui-backend-appkit`'s `AnyView`, for instance).
pub enum Node<H> {
    /// A real native widget. Whatever lives beneath it in its own backend-managed hierarchy
    /// (e.g. `TabView`'s tab-switching) is opaque to this tree — it is a leaf as far as layout
    /// here is concerned.
    Native(H),
    /// A layout container or drawing primitive with no native backing of its own. New layout/
    /// shape kinds (a future `Grid`, say) are added by implementing `VirtualNode` — this variant,
    /// and the tree-walking code below, never need to change.
    Virtual { content: Box<dyn VirtualNode>, children: Vec<Node<H>> },
}

/// The extension point for adding a new virtual layout or shape kind. `Stack` and `Shape` below
/// are the first two implementations (backing `VerticalLayout`/`HorizontalLayout`
/// and `Rectangle`/`Ellipse` respectively); a future `Grid` would be a third, with no changes
/// needed anywhere else in this module.
pub trait VirtualNode {
    /// This node's own natural size, given its children's already-measured sizes.
    fn measure(&self, available: Size, child_sizes: &[Size]) -> Size;
    /// The rect to assign each child (in this node's own local coordinate space), given the
    /// final size this node itself was assigned.
    fn arrange(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect>;
    /// Content this node paints for itself, if any (`None` for pure layout containers like
    /// `Stack`, which only position children and draw nothing on their own account).
    fn paint(&self) -> Option<PaintKind>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintKind {
    Shape { kind: ShapeKind, fill: Option<String>, stroke: Option<String>, stroke_width: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeKind {
    RoundedRect { corner_radius: f32 },
    Oval,
}

/// `VerticalLayout`/`HorizontalLayout` — a thin wrapper around the existing
/// `stack_arrange`/`stack_natural_size` free functions (§H.2's original stack-layout math).
pub struct Stack {
    pub orientation: Orientation,
    pub spacing: f32,
    pub cross_align: CrossAlign,
}

impl VirtualNode for Stack {
    fn measure(&self, _available: Size, child_sizes: &[Size]) -> Size {
        stack_natural_size(self.orientation, self.spacing, child_sizes)
    }

    fn arrange(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        stack_arrange(final_size, self.orientation, self.spacing, self.cross_align, child_sizes).1
    }

    fn paint(&self) -> Option<PaintKind> {
        None
    }
}

/// `Rectangle`/`Ellipse`. Has no intrinsic size of its own — its natural size is the bounding box
/// of its children (so e.g. a `Rectangle` wrapping a single `Text` shrink-wraps it by default,
/// same as `Stack`'s "Auto" main-axis sizing), and every child simply overlays its full bounds
/// (no layout math *within* the shape — a single content slot, not a container in its own right).
pub struct Shape {
    pub kind: ShapeKind,
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: f32,
}

impl VirtualNode for Shape {
    fn measure(&self, _available: Size, child_sizes: &[Size]) -> Size {
        Size {
            width: child_sizes.iter().map(|s| s.width).fold(0.0_f32, f32::max),
            height: child_sizes.iter().map(|s| s.height).fold(0.0_f32, f32::max),
        }
    }

    fn arrange(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        vec![Rect { x: 0.0, y: 0.0, width: final_size.width, height: final_size.height }; child_sizes.len()]
    }

    fn paint(&self) -> Option<PaintKind> {
        Some(PaintKind::Shape {
            kind: self.kind,
            fill: self.fill.clone(),
            stroke: self.stroke.clone(),
            stroke_width: self.stroke_width,
        })
    }
}

fn measure<H: LayoutNode>(node: &Node<H>, available: Size) -> Size {
    match node {
        Node::Native(h) => h.measure(available),
        Node::Virtual { content, children } => {
            let child_sizes: Vec<Size> = children.iter().map(|c| measure(c, available)).collect();
            content.measure(available, &child_sizes)
        }
    }
}

fn walk<H: LayoutNode + Clone>(
    node: &Node<H>,
    offset: (f32, f32),
    final_size: Size,
    natives: &mut Vec<(H, Rect)>,
    paints: &mut Vec<(PaintKind, Rect)>,
) {
    let rect = Rect { x: offset.0, y: offset.1, width: final_size.width, height: final_size.height };
    match node {
        Node::Native(h) => natives.push((h.clone(), rect)),
        Node::Virtual { content, children } => {
            if let Some(paint) = content.paint() {
                paints.push((paint, rect));
            }
            let child_sizes: Vec<Size> = children.iter().map(|c| measure(c, final_size)).collect();
            let child_rects = content.arrange(final_size, &child_sizes);
            for (child, child_rect) in children.iter().zip(child_rects) {
                let child_offset = (offset.0 + child_rect.x, offset.1 + child_rect.y);
                let child_size = Size { width: child_rect.width, height: child_rect.height };
                walk(child, child_offset, child_size, natives, paints);
            }
        }
    }
}

/// This tree's natural (unconstrained) size — e.g. for a container that must report an
/// `intrinsicContentSize` to an Auto-Layout-managed ancestor (see `elwindui-backend-appkit`'s
/// `TreeHostView`) before it has ever actually been given a frame to lay out into.
pub fn natural_size<H: LayoutNode>(tree: &Node<H>) -> Size {
    measure(tree, Size { width: 0.0, height: 0.0 })
}

/// Recursively measures and arranges `tree` against `available`, returning every native leaf
/// (cloned — cheap for a thin `Retained<NSView>`-style handle) paired with its **absolute** rect
/// (accumulated through however many `Virtual` layers sit above it), and every self-painting
/// node's content paired with its own absolute rect. A backend's "host" (see
/// `elwindui-backend-appkit`'s `host_tree`) uses the first list to place native subviews and the
/// second to manage paint layers (e.g. `CAShapeLayer`s) — `elwindui-core` itself knows nothing
/// about `NSView`/`addSubview`/`CALayer`.
pub fn layout_tree<H: LayoutNode + Clone>(tree: &Node<H>, available: Size) -> (Vec<(H, Rect)>, Vec<(PaintKind, Rect)>) {
    let mut natives = Vec::new();
    let mut paints = Vec::new();
    let root_size = measure(tree, available);
    walk(tree, (0.0, 0.0), root_size, &mut natives, &mut paints);
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

    #[test]
    fn single_native_leaf_as_root_gets_its_own_measured_size() {
        // A bare leaf at the root has no `Stack`/`Shape` ancestor to stretch it, so it's arranged
        // at its own intrinsic size, not the full `available` — exactly like a lone `Button`
        // placed directly as a `Window`'s content wouldn't auto-fill the window.
        let tree = Node::Native(FakeHandle("a", size(10.0, 20.0)));
        let (natives, paints) = layout_tree(&tree, size(200.0, 100.0));
        assert_eq!(natives, vec![(FakeHandle("a", size(10.0, 20.0)), Rect { x: 0.0, y: 0.0, width: 10.0, height: 20.0 })]);
        assert!(paints.is_empty());
    }

    #[test]
    fn nested_stack_accumulates_absolute_offsets() {
        // Vertical outer stack containing a native leaf, then a horizontal inner stack of two
        // native leaves — checks that the inner stack's children get *absolute* coordinates,
        // not coordinates relative to the inner stack alone.
        let tree: Node<FakeHandle> = Node::Virtual {
            content: Box::new(Stack { orientation: Orientation::Vertical, spacing: 5.0, cross_align: CrossAlign::Start }),
            children: vec![
                Node::Native(FakeHandle("top", size(50.0, 10.0))),
                Node::Virtual {
                    content: Box::new(Stack { orientation: Orientation::Horizontal, spacing: 2.0, cross_align: CrossAlign::Start }),
                    children: vec![
                        Node::Native(FakeHandle("left", size(20.0, 20.0))),
                        Node::Native(FakeHandle("right", size(30.0, 20.0))),
                    ],
                },
            ],
        };

        let (natives, paints) = layout_tree(&tree, size(200.0, 200.0));
        assert!(paints.is_empty());
        assert_eq!(natives.len(), 3);
        assert_eq!(natives[0], (FakeHandle("top", size(50.0, 10.0)), Rect { x: 0.0, y: 0.0, width: 50.0, height: 10.0 }));
        // inner stack starts at y = 10 (top's height) + 5 (spacing) = 15
        assert_eq!(natives[1], (FakeHandle("left", size(20.0, 20.0)), Rect { x: 0.0, y: 15.0, width: 20.0, height: 20.0 }));
        assert_eq!(natives[2], (FakeHandle("right", size(30.0, 20.0)), Rect { x: 22.0, y: 15.0, width: 30.0, height: 20.0 }));
    }

    #[test]
    fn shape_reports_paint_and_overlays_children_at_its_own_absolute_rect() {
        let tree: Node<FakeHandle> = Node::Virtual {
            content: Box::new(Shape {
                kind: ShapeKind::RoundedRect { corner_radius: 8.0 },
                fill: Some("#3498db".to_string()),
                stroke: None,
                stroke_width: 0.0,
            }),
            children: vec![Node::Native(FakeHandle("label", size(40.0, 20.0)))],
        };

        let (natives, paints) = layout_tree(&tree, size(100.0, 50.0));
        assert_eq!(paints.len(), 1);
        assert_eq!(paints[0].1, Rect { x: 0.0, y: 0.0, width: 40.0, height: 20.0 }); // shrink-wraps its child
        assert_eq!(natives[0].1, Rect { x: 0.0, y: 0.0, width: 40.0, height: 20.0 }); // child overlays full bounds
    }

    #[test]
    fn empty_virtual_node_has_zero_size_and_no_leaves() {
        let tree: Node<FakeHandle> = Node::Virtual {
            content: Box::new(Stack { orientation: Orientation::Vertical, spacing: 0.0, cross_align: CrossAlign::Start }),
            children: vec![],
        };
        let (natives, paints) = layout_tree(&tree, size(100.0, 100.0));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
    }
}
