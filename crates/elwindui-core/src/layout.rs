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

/// Measure/Arrange two-pass layout, implemented by every backend native handle. See
/// docs/elwindui_spec.md 付録H.2. `elwindui_core::tree`'s generic `measure`/`arrange` (the
/// `UIElement<H>`-wide Margin/Alignment wrapper) delegates to this for `NativeControl<H>`
/// specifically; every other `UIElement` kind (`Stack`/`Shape`/`TextBlock`/`Control`) implements
/// `measure_override`/`arrange_override` instead.
pub trait LayoutNode {
    fn measure(&self, available: Size) -> Size;
    fn arrange(&mut self, final_rect: Rect);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// WinUI3's `HorizontalAlignment` — a property of the *element itself* (via `UIElementBase`), not
/// of whatever container it happens to sit in (see `elwindui_core::tree`'s `UIElement` trait).
/// `Stretch` is every element's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HorizontalAlignment {
    Left,
    Center,
    Right,
    Stretch,
}

/// WinUI3's `VerticalAlignment` — see `HorizontalAlignment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlignment {
    Top,
    Center,
    Bottom,
    Stretch,
}

/// Shrinks `size` by a uniform `margin` on every side (never below zero) — the "how much room is
/// actually left for me to measure into" half of `UIElement`'s generic Margin handling.
pub fn shrink_by_margin(size: Size, margin: f32) -> Size {
    Size { width: (size.width - 2.0 * margin).max(0.0), height: (size.height - 2.0 * margin).max(0.0) }
}

/// The inverse of `shrink_by_margin` — what an element's *own* desired size (margin excluded)
/// grows back into once its margin is added back, for reporting to whatever measured it.
pub fn grow_by_margin(size: Size, margin: f32) -> Size {
    Size { width: size.width + 2.0 * margin, height: size.height + 2.0 * margin }
}

/// Shrinks `rect` by a uniform `margin` on every side (never below zero), keeping it centered
/// within the original bounds — the "arrange" half of `UIElement`'s generic Margin handling.
pub fn shrink_rect_by_margin(rect: Rect, margin: f32) -> Rect {
    Rect {
        x: rect.x + margin,
        y: rect.y + margin,
        width: (rect.width - 2.0 * margin).max(0.0),
        height: (rect.height - 2.0 * margin).max(0.0),
    }
}

/// Applies `h_align`/`v_align` to an element whose own desired size is `desired`, within `slot`
/// (the space its parent granted it, margin already excluded) — `Stretch` on an axis uses `slot`'s
/// full extent on that axis; any other value keeps `desired`'s size on that axis, positioned at
/// the corresponding edge/center of `slot`. This is `UIElement`'s generic per-element Arrange
/// step (see `elwindui_core::tree`), applied uniformly regardless of element kind — not specific
/// to `Stack` children the way the old `cross_align`-on-the-container design was.
pub fn align_within(slot: Rect, desired: Size, h_align: HorizontalAlignment, v_align: VerticalAlignment) -> Rect {
    let width = match h_align {
        HorizontalAlignment::Stretch => slot.width,
        _ => desired.width.min(slot.width),
    };
    let height = match v_align {
        VerticalAlignment::Stretch => slot.height,
        _ => desired.height.min(slot.height),
    };
    let x = match h_align {
        HorizontalAlignment::Left | HorizontalAlignment::Stretch => slot.x,
        HorizontalAlignment::Center => slot.x + (slot.width - width) / 2.0,
        HorizontalAlignment::Right => slot.x + (slot.width - width),
    };
    let y = match v_align {
        VerticalAlignment::Top | VerticalAlignment::Stretch => slot.y,
        VerticalAlignment::Center => slot.y + (slot.height - height) / 2.0,
        VerticalAlignment::Bottom => slot.y + (slot.height - height),
    };
    Rect { x, y, width, height }
}

/// Pure `VerticalLayout`/`HorizontalLayout` arrangement math — no widgets, no `Rc`/`RefCell`, just
/// sizes in and rects out, so it's trivially unit-testable (see tests below) independent of any
/// backend.
///
/// Main-axis size is always each child's own measured ("Auto") size — there is no WinUI3
/// `Grid`-style per-child fixed/`*`-proportional sizing yet (planned as a future `Grid` element).
/// Cross-axis: unlike the old container-level `cross_align`, this returns each child's "slot"
/// spanning the *entire* cross-axis extent of `available` — actually aligning/sizing a child
/// within its slot (`Stretch` vs `Start`/`Center`/`End`) is `elwindui_core::tree`'s generic
/// per-element `arrange` wrapper's job now, driven by that *child's own*
/// `HorizontalAlignment`/`VerticalAlignment`, not a single setting the container applies to every
/// child uniformly.
pub fn stack_arrange(available: Size, orientation: Orientation, spacing: f32, child_sizes: &[Size]) -> Vec<Rect> {
    let main_of = |s: Size| match orientation {
        Orientation::Horizontal => s.width,
        Orientation::Vertical => s.height,
    };
    let available_cross = match orientation {
        Orientation::Horizontal => available.height,
        Orientation::Vertical => available.width,
    };

    let mut rects = Vec::with_capacity(child_sizes.len());
    let mut cursor = 0.0_f32;
    for &size in child_sizes {
        let child_main = main_of(size);
        rects.push(match orientation {
            Orientation::Horizontal => Rect { x: cursor, y: 0.0, width: child_main, height: available_cross },
            Orientation::Vertical => Rect { x: 0.0, y: cursor, width: available_cross, height: child_main },
        });
        cursor += child_main + spacing;
    }

    rects
}

/// The "natural" size a stack of `child_sizes` wants, independent of any `available` space: sum
/// along the main axis (plus inter-child `spacing`), max along the cross axis. Shared by a
/// container's own `measure_override`/`intrinsicContentSize` (nesting one layout inside another
/// needs the outer one to know the inner one's natural size).
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
        let rects = stack_arrange(size(1000.0, 1000.0), Orientation::Horizontal, 5.0, &sizes);

        // Cross-axis (height) is now each child's *slot* spanning the full available height —
        // alignment/sizing within it is applied by `elwindui_core::tree`'s generic wrapper, not
        // this function — so both rects report `available`'s full height (1000), not their own
        // measured height.
        assert_eq!(rects[0], Rect { x: 0.0, y: 0.0, width: 10.0, height: 1000.0 });
        assert_eq!(rects[1], Rect { x: 15.0, y: 0.0, width: 30.0, height: 1000.0 });
    }

    #[test]
    fn vertical_stack_places_children_top_to_bottom_with_spacing() {
        let sizes = [size(10.0, 20.0), size(30.0, 40.0)];
        let rects = stack_arrange(size(1000.0, 1000.0), Orientation::Vertical, 5.0, &sizes);

        assert_eq!(rects[0], Rect { x: 0.0, y: 0.0, width: 1000.0, height: 20.0 });
        assert_eq!(rects[1], Rect { x: 0.0, y: 25.0, width: 1000.0, height: 40.0 });
    }

    #[test]
    fn stack_natural_size_sums_main_axis_and_maxes_cross_axis() {
        let sizes = [size(10.0, 20.0), size(30.0, 40.0)];
        assert_eq!(stack_natural_size(Orientation::Horizontal, 5.0, &sizes), size(45.0, 40.0));
        assert_eq!(stack_natural_size(Orientation::Vertical, 5.0, &sizes), size(30.0, 65.0));
    }

    #[test]
    fn empty_children_yield_zero_natural_size_and_no_rects() {
        let rects = stack_arrange(size(100.0, 100.0), Orientation::Vertical, 5.0, &[]);
        assert!(rects.is_empty());
        assert_eq!(stack_natural_size(Orientation::Vertical, 5.0, &[]), size(0.0, 0.0));
    }

    #[test]
    fn align_within_stretch_fills_the_slot_on_that_axis() {
        let slot = Rect { x: 10.0, y: 20.0, width: 100.0, height: 50.0 };
        let desired = size(10.0, 10.0);
        let rect = align_within(slot, desired, HorizontalAlignment::Stretch, VerticalAlignment::Stretch);
        assert_eq!(rect, slot);
    }

    #[test]
    fn align_within_start_top_keeps_desired_size_at_the_leading_edge() {
        let slot = Rect { x: 10.0, y: 20.0, width: 100.0, height: 50.0 };
        let desired = size(10.0, 10.0);
        let rect = align_within(slot, desired, HorizontalAlignment::Left, VerticalAlignment::Top);
        assert_eq!(rect, Rect { x: 10.0, y: 20.0, width: 10.0, height: 10.0 });
    }

    #[test]
    fn align_within_center_centers_desired_size_within_the_slot() {
        let slot = Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 };
        let desired = size(20.0, 10.0);
        let rect = align_within(slot, desired, HorizontalAlignment::Center, VerticalAlignment::Center);
        assert_eq!(rect, Rect { x: 40.0, y: 45.0, width: 20.0, height: 10.0 });
    }

    #[test]
    fn align_within_end_bottom_anchors_desired_size_at_the_trailing_edge() {
        let slot = Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 };
        let desired = size(20.0, 10.0);
        let rect = align_within(slot, desired, HorizontalAlignment::Right, VerticalAlignment::Bottom);
        assert_eq!(rect, Rect { x: 80.0, y: 90.0, width: 20.0, height: 10.0 });
    }

    #[test]
    fn margin_helpers_shrink_and_grow_symmetrically() {
        let s = size(100.0, 50.0);
        assert_eq!(shrink_by_margin(s, 10.0), size(80.0, 30.0));
        assert_eq!(grow_by_margin(shrink_by_margin(s, 10.0), 10.0), s);

        let r = Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 };
        assert_eq!(shrink_rect_by_margin(r, 10.0), Rect { x: 10.0, y: 10.0, width: 80.0, height: 30.0 });
    }
}
