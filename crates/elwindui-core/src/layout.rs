use crate::base::{Rect, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// WinUI3's `HorizontalAlignment` — a property of the *element itself* (via `UIElementBase`), not
/// of whatever container it happens to sit in (see `elwindui_core::ui`'s `UIElement` trait).
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

/// WinUI3's `Visibility` — only `Visible` (the default) and `Collapsed` exist (unlike WPF, which
/// also has a `Hidden` value). `Collapsed` is handled by `elwindui_core::ui`'s `measure`/
/// `measure_and_align`/`arrange`/`hit_test_at`: a `Collapsed` element takes no space in its
/// parent's layout and is skipped entirely during rendering and hit-testing, along with its whole
/// subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Collapsed,
}

/// WinUI3's `FrameworkElement.MeasureCore`-style constraint application: clamps `size` to
/// `min`/`max` on each axis independently (an unset bound imposes no clamp on that side —
/// `elwindui_core::ui::UIElement`'s `min_width`/`max_width`/etc. are `Option<f32>`, WinUI3's
/// own `NaN`-sentinel equivalent). Used twice per `measure` call (`elwindui_core::ui`'s `measure`/
/// `measure_and_align`): once on the space handed down to `measure_override`, once on its
/// returned desired size.
pub fn apply_size_constraints(size: Size, min_width: Option<f32>, max_width: Option<f32>, min_height: Option<f32>, max_height: Option<f32>) -> Size {
    let clamp = |value: f32, min: Option<f32>, max: Option<f32>| -> f32 {
        let value = min.map_or(value, |min| value.max(min));
        max.map_or(value, |max| value.min(max))
    };
    Size { width: clamp(size.width, min_width, max_width), height: clamp(size.height, min_height, max_height) }
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
/// step (see `elwindui_core::ui`), applied uniformly regardless of element kind — not specific
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
/// within its slot (`Stretch` vs `Start`/`Center`/`End`) is `elwindui_core::ui`'s generic
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

/// WPF/WinUI3-style `Grid` sizing unit — `Auto` (a track's size follows its content's natural
/// size), `Fixed(px)` (a literal size), or `Star(weight)` (a share of whatever space is left after
/// every `Fixed`/`Auto` track has taken its own size, proportional to `weight` among all `Star`
/// tracks — WPF's `*`/`2*` etc.). See `builtin::Grid`, docs/elwindui_spec.md §3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridLength {
    Auto,
    Fixed(f32),
    Star(f32),
}

/// A `Grid`-child's attached `Grid::row`/`Grid::column` position (docs/elwindui_spec.md §3) —
/// 0-indexed, defaulting to the top-left cell (`0, 0`) like WPF's own `Grid.Row`/`Grid.Column`
/// defaults. Row/column spanning isn't implemented yet (each cell holds exactly one child).
/// Not stored as this shape anywhere — `elwindui_core::ui::UIElement::attached` holds `row`/
/// `column` independently in its generic type-erased bag, and `elwindui_core::ui::grid_cell_of`
/// assembles one of these from the two on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridCell {
    pub row: i32,
    pub column: i32,
}

/// Clamps a possibly out-of-range attached-property index into `0..track_count` — an empty
/// `row_definitions`/`column_definitions` is treated as a single implicit `Auto` track (see
/// `grid_natural_size`/`grid_arrange`'s own `.max(1)` on `track_count`), so this never divides by
/// (or indexes into) zero.
fn clamp_track_index(index: i32, track_count: usize) -> usize {
    let last = track_count.max(1) - 1;
    (index.max(0) as usize).min(last)
}

/// Per-axis track sizes with `Star` tracks treated exactly like `Auto` (their own children's max
/// natural size) — correct for `grid_natural_size` (no known final size to distribute "remaining
/// space" from yet, so a `Star` track can only report what its content actually needs, the same
/// way WPF's own `Grid.MeasureOverride` treats `*` columns when measured against infinite
/// available space) but *not* for `grid_arrange`, which needs `Star` tracks to start at `0.0` so
/// leftover space can be computed and redistributed — see `fixed_and_auto_track_sizes` for that
/// variant.
fn natural_track_sizes(defs: &[GridLength], indices: &[usize], dims: &[f32]) -> Vec<f32> {
    let n = defs.len().max(1);
    let mut sizes = vec![0.0_f32; n];
    for (i, &d) in defs.iter().enumerate() {
        if let GridLength::Fixed(v) = d {
            sizes[i] = v;
        }
    }
    for (&idx, &dim) in indices.iter().zip(dims) {
        if !matches!(defs.get(idx), Some(GridLength::Fixed(_))) {
            sizes[idx] = sizes[idx].max(dim);
        }
    }
    sizes
}

/// Per-axis track sizes for `grid_arrange`'s first pass: `Fixed` tracks take their literal value,
/// `Auto` tracks take their own children's max natural size, `Star` tracks are left at `0.0` to be
/// filled in afterward by `distribute_star` once the remaining space is known.
fn fixed_and_auto_track_sizes(defs: &[GridLength], indices: &[usize], dims: &[f32]) -> Vec<f32> {
    let n = defs.len().max(1);
    let mut sizes = vec![0.0_f32; n];
    for (i, &d) in defs.iter().enumerate() {
        if let GridLength::Fixed(v) = d {
            sizes[i] = v;
        }
    }
    for (&idx, &dim) in indices.iter().zip(dims) {
        let is_auto = !matches!(defs.get(idx), Some(GridLength::Fixed(_)) | Some(GridLength::Star(_)));
        if is_auto {
            sizes[idx] = sizes[idx].max(dim);
        }
    }
    sizes
}

/// Distributes whatever space is left in `total_final` (after every `Fixed`/`Auto` track in
/// `sizes` has already taken its share) across `Star` tracks, proportional to their weights. A
/// no-op if there are no `Star` tracks at all (mirrors WPF: content simply doesn't stretch to fill
/// without at least one `*` track).
fn distribute_star(defs: &[GridLength], sizes: &mut [f32], total_final: f32) {
    let used: f32 = sizes.iter().sum();
    let remaining = (total_final - used).max(0.0);
    let total_weight: f32 = defs
        .iter()
        .filter_map(|d| if let GridLength::Star(w) = d { Some(*w) } else { None })
        .sum();
    if total_weight <= 0.0 {
        return;
    }
    for (i, d) in defs.iter().enumerate() {
        if let GridLength::Star(w) = d {
            sizes[i] = remaining * (w / total_weight);
        }
    }
}

fn prefix_offsets(sizes: &[f32]) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut cursor = 0.0_f32;
    for &s in sizes {
        offsets.push(cursor);
        cursor += s;
    }
    offsets
}

/// The natural (intrinsic) size a `Grid` wants, independent of any `available` space — sum of
/// every column's/row's own natural size (`Star` tracks behave like `Auto` here, see
/// `natural_track_sizes`). `cells`/`child_sizes` are parallel, one entry per child (`Grid`'s own
/// `measure_override`/`arrange_override` build both from its `children`).
pub fn grid_natural_size(rows: &[GridLength], columns: &[GridLength], cells: &[GridCell], child_sizes: &[Size]) -> Size {
    let row_indices: Vec<usize> = cells.iter().map(|c| clamp_track_index(c.row, rows.len())).collect();
    let col_indices: Vec<usize> = cells.iter().map(|c| clamp_track_index(c.column, columns.len())).collect();
    let heights: Vec<f32> = child_sizes.iter().map(|s| s.height).collect();
    let widths: Vec<f32> = child_sizes.iter().map(|s| s.width).collect();

    let row_sizes = natural_track_sizes(rows, &row_indices, &heights);
    let col_sizes = natural_track_sizes(columns, &col_indices, &widths);
    Size { width: col_sizes.iter().sum(), height: row_sizes.iter().sum() }
}

/// Pure `Grid` arrangement math (see `stack_arrange`'s own doc comment on why this is a free
/// function, independent of any backend): resolves `Fixed`/`Auto` track sizes first, distributes
/// any space `final_size` has left over across `Star` tracks (`distribute_star`), then places each
/// child at its (clamped) `GridCell`'s row/column offset. One child per cell — no spanning yet.
pub fn grid_arrange(
    final_size: Size,
    rows: &[GridLength],
    columns: &[GridLength],
    cells: &[GridCell],
    child_sizes: &[Size],
) -> Vec<Rect> {
    let row_indices: Vec<usize> = cells.iter().map(|c| clamp_track_index(c.row, rows.len())).collect();
    let col_indices: Vec<usize> = cells.iter().map(|c| clamp_track_index(c.column, columns.len())).collect();
    let heights: Vec<f32> = child_sizes.iter().map(|s| s.height).collect();
    let widths: Vec<f32> = child_sizes.iter().map(|s| s.width).collect();

    let mut row_sizes = fixed_and_auto_track_sizes(rows, &row_indices, &heights);
    let mut col_sizes = fixed_and_auto_track_sizes(columns, &col_indices, &widths);
    distribute_star(rows, &mut row_sizes, final_size.height);
    distribute_star(columns, &mut col_sizes, final_size.width);

    let row_offsets = prefix_offsets(&row_sizes);
    let col_offsets = prefix_offsets(&col_sizes);

    row_indices
        .iter()
        .zip(col_indices.iter())
        .map(|(&r, &c)| Rect { x: col_offsets[c], y: row_offsets[r], width: col_sizes[c], height: row_sizes[r] })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: f32, height: f32) -> Size {
        Size { width, height }
    }

    #[test]
    fn horizontal_stack_places_children_left_to_right_with_spacing() {
        let sizes = [size(10.0, 20.0), size(30.0, 40.0)];
        let rects = stack_arrange(size(1000.0, 1000.0), Orientation::Horizontal, 5.0, &sizes);

        // Cross-axis (height) is now each child's *slot* spanning the full available height —
        // alignment/sizing within it is applied by `elwindui_core::ui`'s generic wrapper, not
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
    fn apply_size_constraints_clamps_only_the_bounds_that_are_set() {
        let s = size(50.0, 5.0);
        assert_eq!(apply_size_constraints(s, Some(100.0), None, None, Some(4.0)), size(100.0, 4.0));
        assert_eq!(apply_size_constraints(s, None, Some(20.0), None, None), size(20.0, 5.0));
        assert_eq!(apply_size_constraints(s, None, None, None, None), s);
    }

    #[test]
    fn margin_helpers_shrink_and_grow_symmetrically() {
        let s = size(100.0, 50.0);
        assert_eq!(shrink_by_margin(s, 10.0), size(80.0, 30.0));
        assert_eq!(grow_by_margin(shrink_by_margin(s, 10.0), 10.0), s);

        let r = Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 };
        assert_eq!(shrink_rect_by_margin(r, 10.0), Rect { x: 10.0, y: 10.0, width: 80.0, height: 30.0 });
    }

    fn cell(row: i32, column: i32) -> GridCell {
        GridCell { row, column }
    }

    #[test]
    fn grid_arrange_places_fixed_and_auto_tracks_by_their_own_size() {
        // 2 columns (Fixed(100), Auto) x 1 row (Auto). Column 1 (Auto) sizes to its widest child.
        let rows = [GridLength::Auto];
        let columns = [GridLength::Fixed(100.0), GridLength::Auto];
        let cells = [cell(0, 0), cell(0, 1)];
        let sizes = [size(10.0, 20.0), size(30.0, 40.0)];

        let rects = grid_arrange(size(1000.0, 1000.0), &rows, &columns, &cells, &sizes);
        assert_eq!(rects[0], Rect { x: 0.0, y: 0.0, width: 100.0, height: 40.0 });
        assert_eq!(rects[1], Rect { x: 100.0, y: 0.0, width: 30.0, height: 40.0 });
    }

    #[test]
    fn grid_arrange_distributes_remaining_space_across_star_columns_by_weight() {
        // Fixed(100) + Star(1) + Star(2), final width 1000 -> remaining 900 split 1:2 -> 300/600.
        let rows = [GridLength::Auto];
        let columns = [GridLength::Fixed(100.0), GridLength::Star(1.0), GridLength::Star(2.0)];
        let cells = [cell(0, 0), cell(0, 1), cell(0, 2)];
        let sizes = [size(5.0, 5.0), size(5.0, 5.0), size(5.0, 5.0)];

        let rects = grid_arrange(size(1000.0, 50.0), &rows, &columns, &cells, &sizes);
        assert_eq!(rects[0].width, 100.0);
        assert_eq!(rects[1].width, 300.0);
        assert_eq!(rects[2].width, 600.0);
        assert_eq!(rects[1].x, 100.0);
        assert_eq!(rects[2].x, 400.0);
    }

    #[test]
    fn grid_arrange_clamps_out_of_range_cell_indices_to_the_last_track() {
        let rows = [GridLength::Auto, GridLength::Auto];
        let columns = [GridLength::Auto];
        let cells = [cell(99, 0)];
        let sizes = [size(10.0, 10.0)];

        let rects = grid_arrange(size(100.0, 100.0), &rows, &columns, &cells, &sizes);
        assert_eq!(rects[0].y, 0.0, "single non-empty row still lands at offset 0 after clamping");
    }

    #[test]
    fn grid_natural_size_sums_fixed_and_auto_tracks_treating_star_as_auto() {
        let rows = [GridLength::Auto];
        let columns = [GridLength::Fixed(50.0), GridLength::Star(1.0)];
        let cells = [cell(0, 0), cell(0, 1)];
        let sizes = [size(10.0, 20.0), size(30.0, 40.0)];

        // Fixed column reports its literal 50; Star column (no known final size yet) reports its
        // own content's natural width (30), same treatment as an Auto column would get.
        assert_eq!(grid_natural_size(&rows, &columns, &cells, &sizes), size(80.0, 40.0));
    }

    #[test]
    fn grid_empty_rows_and_columns_are_treated_as_a_single_implicit_auto_track() {
        let cells = [cell(0, 0), cell(0, 0)];
        let sizes = [size(10.0, 20.0), size(30.0, 5.0)];
        assert_eq!(grid_natural_size(&[], &[], &cells, &sizes), size(30.0, 20.0));

        let rects = grid_arrange(size(100.0, 100.0), &[], &[], &cells, &sizes);
        assert_eq!(rects[0], Rect { x: 0.0, y: 0.0, width: 30.0, height: 20.0 });
        assert_eq!(rects[1], rects[0], "both children share the single implicit cell");
    }
}
