/// See docs/elwindui_spec.md 付録H.2.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Measure/Arrange two-pass layout, implemented by every builtin (`Stack`, `Canvas`, ...).
/// See docs/elwindui_spec.md 付録H.2.
pub trait LayoutNode {
    fn measure(&self, available: Size) -> Size;
    fn arrange(&mut self, final_rect: Rect);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// Cross-axis alignment for a stack layout — WinUI3's `HorizontalAlignment`/`VerticalAlignment`
/// on a `StackPanel`'s children, applied uniformly to every child (no per-child override, unlike
/// `Grid`'s attached properties — ElwindUIL has no attached-property syntax yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossAlign {
    Start,
    Center,
    End,
    Stretch,
}

/// Pure `VerticalLayout`/`HorizontalLayout` arrangement math — no widgets, no `Rc`/`RefCell`,
/// just sizes in and rects out, so it's trivially unit-testable (see tests below) independent of
/// any backend. `elwindui-backend-appkit`'s `StackLayoutView` (a plain `NSView` subclass, not
/// `NSStackView`) is the only caller: it measures each child via `AnyView`'s `LayoutNode` impl
/// (`fittingSize`), calls this, then applies each returned `Rect` via `setFrame`.
///
/// Main-axis size is always each child's own measured ("Auto") size — there is no WinUI3
/// `Grid`-style per-child fixed/`*`-proportional sizing yet, since that needs attached-property
/// syntax (`Grid.Row="1"` on the child) ElwindUIL doesn't have. `available`'s main-axis dimension
/// is therefore unused for arrangement math today; it only matters via `cross_align`'s `Stretch`
/// case. Returns the container's own natural size (sum of children along the main axis, max
/// along the cross axis) alongside each child's rect — the natural size is what a `measure()`
/// call on the container itself should report to whatever laid *it* out.
pub fn stack_arrange(
    available: Size,
    orientation: Orientation,
    spacing: f32,
    cross_align: CrossAlign,
    child_sizes: &[Size],
) -> (Size, Vec<Rect>) {
    let main_of = |s: Size| match orientation {
        Orientation::Horizontal => s.width,
        Orientation::Vertical => s.height,
    };
    let cross_of = |s: Size| match orientation {
        Orientation::Horizontal => s.height,
        Orientation::Vertical => s.width,
    };

    let available_cross = cross_of(available);
    let natural_size = stack_natural_size(orientation, spacing, child_sizes);

    let mut rects = Vec::with_capacity(child_sizes.len());
    let mut cursor = 0.0_f32;
    for &size in child_sizes {
        let child_main = main_of(size);
        let child_cross = cross_of(size);
        let arranged_cross = match cross_align {
            CrossAlign::Stretch => available_cross,
            _ => child_cross,
        };
        let cross_offset = match cross_align {
            CrossAlign::Start | CrossAlign::Stretch => 0.0,
            CrossAlign::Center => (available_cross - child_cross) / 2.0,
            CrossAlign::End => available_cross - child_cross,
        };
        rects.push(match orientation {
            Orientation::Horizontal => {
                Rect { x: cursor, y: cross_offset, width: child_main, height: arranged_cross }
            }
            Orientation::Vertical => {
                Rect { x: cross_offset, y: cursor, width: arranged_cross, height: child_main }
            }
        });
        cursor += child_main + spacing;
    }

    (natural_size, rects)
}

/// The "natural" size a stack of `child_sizes` wants, independent of any `available` space: sum
/// along the main axis (plus inter-child `spacing`), max along the cross axis. Shared by
/// `stack_arrange` above and by a container's own `measure()`/`intrinsicContentSize` (nesting one
/// layout inside another needs the outer one to know the inner one's natural size).
pub fn stack_natural_size(orientation: Orientation, spacing: f32, child_sizes: &[Size]) -> Size {
    let main_of = |s: Size| match orientation {
        Orientation::Horizontal => s.width,
        Orientation::Vertical => s.height,
    };
    let cross_of = |s: Size| match orientation {
        Orientation::Horizontal => s.height,
        Orientation::Vertical => s.width,
    };

    let natural_cross = child_sizes.iter().copied().map(cross_of).fold(0.0_f32, f32::max);
    let total_main: f32 = child_sizes.iter().copied().map(main_of).sum::<f32>()
        + if child_sizes.is_empty() { 0.0 } else { spacing * (child_sizes.len() - 1) as f32 };

    match orientation {
        Orientation::Horizontal => Size { width: total_main, height: natural_cross },
        Orientation::Vertical => Size { width: natural_cross, height: total_main },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedSize {
        size: Size,
        arranged: Option<Rect>,
    }

    impl LayoutNode for FixedSize {
        fn measure(&self, _available: Size) -> Size {
            self.size
        }

        fn arrange(&mut self, final_rect: Rect) {
            self.arranged = Some(final_rect);
        }
    }

    #[test]
    fn measure_ignores_available_and_arrange_records_final_rect() {
        let mut node = FixedSize {
            size: Size {
                width: 10.0,
                height: 20.0,
            },
            arranged: None,
        };

        let measured = node.measure(Size {
            width: 100.0,
            height: 100.0,
        });
        assert_eq!(measured, node.size);

        let rect = Rect {
            x: 1.0,
            y: 2.0,
            width: 10.0,
            height: 20.0,
        };
        node.arrange(rect);
        assert_eq!(node.arranged, Some(rect));
    }

    fn size(width: f32, height: f32) -> Size {
        Size { width, height }
    }

    #[test]
    fn horizontal_stack_places_children_left_to_right_with_spacing() {
        let sizes = [size(10.0, 20.0), size(30.0, 40.0)];
        let (natural, rects) =
            stack_arrange(size(1000.0, 1000.0), Orientation::Horizontal, 5.0, CrossAlign::Start, &sizes);

        assert_eq!(natural, size(45.0, 40.0)); // 10 + 5 + 30 wide, tallest child (40) high
        assert_eq!(rects[0], Rect { x: 0.0, y: 0.0, width: 10.0, height: 20.0 });
        assert_eq!(rects[1], Rect { x: 15.0, y: 0.0, width: 30.0, height: 40.0 });
    }

    #[test]
    fn vertical_stack_places_children_top_to_bottom_with_spacing() {
        let sizes = [size(10.0, 20.0), size(30.0, 40.0)];
        let (natural, rects) =
            stack_arrange(size(1000.0, 1000.0), Orientation::Vertical, 5.0, CrossAlign::Start, &sizes);

        assert_eq!(natural, size(30.0, 65.0)); // widest child (30) wide, 20 + 5 + 40 tall
        assert_eq!(rects[0], Rect { x: 0.0, y: 0.0, width: 10.0, height: 20.0 });
        assert_eq!(rects[1], Rect { x: 0.0, y: 25.0, width: 30.0, height: 40.0 });
    }

    #[test]
    fn cross_align_positions_children_across_the_cross_axis() {
        let sizes = [size(10.0, 20.0)];
        let available = size(100.0, 100.0);

        let (_, start) = stack_arrange(available, Orientation::Horizontal, 0.0, CrossAlign::Start, &sizes);
        assert_eq!(start[0], Rect { x: 0.0, y: 0.0, width: 10.0, height: 20.0 });

        let (_, center) = stack_arrange(available, Orientation::Horizontal, 0.0, CrossAlign::Center, &sizes);
        assert_eq!(center[0], Rect { x: 0.0, y: 40.0, width: 10.0, height: 20.0 });

        let (_, end) = stack_arrange(available, Orientation::Horizontal, 0.0, CrossAlign::End, &sizes);
        assert_eq!(end[0], Rect { x: 0.0, y: 80.0, width: 10.0, height: 20.0 });

        let (_, stretch) = stack_arrange(available, Orientation::Horizontal, 0.0, CrossAlign::Stretch, &sizes);
        assert_eq!(stretch[0], Rect { x: 0.0, y: 0.0, width: 10.0, height: 100.0 });
    }

    #[test]
    fn empty_children_yield_zero_natural_size_and_no_rects() {
        let (natural, rects) =
            stack_arrange(size(100.0, 100.0), Orientation::Vertical, 5.0, CrossAlign::Start, &[]);
        assert_eq!(natural, size(0.0, 0.0));
        assert!(rects.is_empty());
    }
}
