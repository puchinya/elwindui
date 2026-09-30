//! `elwindui::ui::Grid` — row/column layout, plus the attached-property read-back its cell placement uses.

use super::*;
use crate::layout::{
    GridTrackConstraint, grid_arrange_track_sizes, grid_arrange_with_constraints,
    grid_resolve_track_sizes_with_constraints,
};

/// WPF/WinUI3-style row/column layout (`elwindui::ui::Grid`, docs/specs/dsl_spec.md §3). Each child's
/// cell placement comes from its own `UIElement::attached` bag (the `Grid::row`/`Grid::column`
/// attached properties it was constructed with, read back via `grid_cell_of` since only `Grid`
/// itself knows those two fields are `i32`), not a field on `Grid` itself — see `attached`'s
/// own doc comment. A child whose cell falls outside `row_definitions`/`column_definitions`'
/// bounds is clamped to the last row/column, mirroring `grid_arrange`'s own clamping. Row/column
/// spanning is out of scope for this pass (one child per cell) — a future `#[attached]
/// row_span`/`column_span` pair on `elwindui::ui::Grid` would extend this the same way `row`/`column`
/// were added, with no changes needed here beyond consulting the extra fields.
/// `rows`/`columns` (not `row_definitions`/`column_definitions`) to match `elwindui::ui::Grid`'s own
/// `#[param] rows`/`#[param] columns` names — `elwindui-codegen`'s setter-based construction calls
/// `.set_{param name}(..)` generically, so the Rust field/setter name must agree with the DSL's.
/// `Grid`'s own class trait (docs/design/runtime/ui_tree_design.md) — inherits `Layout` (like
/// `VerticalLayout`/`HorizontalLayout`), so `children` comes from that shared base rather than
/// being declared on `Grid` itself (`docs/specs/ui_spec.md#grid`).
/// Reads a child's `Grid::row`/`Grid::column` attached-property values back out of its
/// `UIElement::attached` bag — `Grid` is the only thing that knows those two fields are `i32`
/// and default to `0`, so it (not `UIElement`) owns this downcast, mirroring how
/// `elwindui-codegen`'s `emit_attached_setters` also resolves the field's declared type from the
/// owner (`Grid`) itself, never `UIElement`.
pub(crate) fn grid_cell_of(child: &Rc<dyn UIElementExt>) -> GridCell {
    GridCell {
        row: child.as_ui_element().get_attached("Grid", "row", 0i32),
        column: child.as_ui_element().get_attached("Grid", "column", 0i32),
    }
}

#[elwindui_macros::class(inherits = crate::ui::Layout)]
#[prop(rows: Vec<crate::layout::GridLength>)]
#[prop(columns: Vec<crate::layout::GridLength>)]
#[prop(row_spacing: Option<f32>)]
#[prop(column_spacing: Option<f32>)]
#[prop(row_constraints: Vec<crate::layout::GridTrackConstraint>)]
#[prop(column_constraints: Vec<crate::layout::GridTrackConstraint>)]
#[prop(attached, row: i32 = 0)]
#[prop(attached, column: i32 = 0)]
pub struct Grid {
    pub rows: RefCell<Vec<GridLength>>,
    pub columns: RefCell<Vec<GridLength>>,
    row_spacing: Cell<f32>,
    column_spacing: Cell<f32>,
    pub row_constraints: RefCell<Vec<GridTrackConstraint>>,
    pub column_constraints: RefCell<Vec<GridTrackConstraint>>,
    resolved_row_sizes: RefCell<Vec<f32>>,
    resolved_column_sizes: RefCell<Vec<f32>>,
}

#[elwindui_macros::class]
impl Grid {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let children = self.children().to_vec();
        let cells: Vec<GridCell> = children.iter().map(grid_cell_of).collect();
        let rows = self.rows.borrow();
        let columns = self.columns.borrow();
        let row_constraints = self.row_constraints.borrow();
        let column_constraints = self.column_constraints.borrow();
        let row_spacing = self.row_spacing.get();
        let column_spacing = self.column_spacing.get();
        let track_available = grid_track_available(
            available,
            rows.len().max(1),
            columns.len().max(1),
            row_spacing,
            column_spacing,
        );

        // Pass 1: each child's own natural size, constrained only by its own track where that
        // track already has a known size (`Fixed`) — see `grid_measure_pass1_available`'s own doc
        // comment.
        let pass1_available = grid_measure_pass1_available(&rows, &columns, &cells);
        for (child, avail) in children.iter().zip(&pass1_available) {
            child.measure(*avail);
        }
        let pass1_sizes: Vec<Size> = children
            .iter()
            .map(|c| c.measured_size().unwrap_or_default())
            .collect();

        let (row_sizes, col_sizes) = grid_resolve_track_sizes_with_constraints(
            &rows,
            &columns,
            &cells,
            &pass1_sizes,
            track_available,
            &row_constraints,
            &column_constraints,
        );

        // Pass 2: re-measure every child against its now-fully-resolved cell size, so
        // `measured_size()` afterward — read back by `arrange_override`'s own track resolution
        // below, and by whatever measured this `Grid` for its own desired size returned here —
        // reflects the size each child will actually occupy, not pass 1's Auto/Star-unconstrained
        // probe size.
        let pass2_available = grid_pass2_available(&rows, &columns, &cells, &row_sizes, &col_sizes);
        for (child, avail) in children.iter().zip(&pass2_available) {
            child.measure(*avail);
        }

        Size {
            width: col_sizes.iter().sum::<f32>()
                + grid_spacing_total(columns.len().max(1), column_spacing),
            height: row_sizes.iter().sum::<f32>()
                + grid_spacing_total(rows.len().max(1), row_spacing),
        }
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let children = self.children().to_vec();
        let cells: Vec<GridCell> = children.iter().map(grid_cell_of).collect();
        let child_sizes: Vec<Size> = children
            .iter()
            .map(|c| c.measured_size().unwrap_or_default())
            .collect();
        let rows = self.rows.borrow();
        let columns = self.columns.borrow();
        let row_constraints = self.row_constraints.borrow();
        let column_constraints = self.column_constraints.borrow();
        let row_spacing = self.row_spacing.get();
        let column_spacing = self.column_spacing.get();
        let track_area = grid_track_available(
            final_size,
            rows.len().max(1),
            columns.len().max(1),
            row_spacing,
            column_spacing,
        );
        let (row_sizes, col_sizes) = grid_arrange_track_sizes(
            track_area,
            &rows,
            &columns,
            &cells,
            &child_sizes,
            &row_constraints,
            &column_constraints,
        );
        let child_rects = grid_arrange_with_constraints(
            track_area,
            &rows,
            &columns,
            &cells,
            &child_sizes,
            &row_constraints,
            &column_constraints,
        );
        let row_track_count = row_sizes.len();
        let column_track_count = col_sizes.len();
        *self.resolved_row_sizes.borrow_mut() = row_sizes;
        *self.resolved_column_sizes.borrow_mut() = col_sizes;
        for ((child, mut rect), cell) in children.iter().zip(child_rects).zip(&cells) {
            let row = (cell.row.max(0) as usize).min(row_track_count.saturating_sub(1));
            let column = (cell.column.max(0) as usize).min(column_track_count.saturating_sub(1));
            rect.x += column as f32 * column_spacing;
            rect.y += row as f32 * row_spacing;
            child.arrange(rect);
        }
        final_size
    }
    fn set_rows(&self, rows: Vec<GridLength>) {
        let changed = *self.rows.borrow() != rows;
        if std::env::var_os("ELWINDUI_PERF_TRACE").is_some() {
            eprintln!(
                "[perf] grid_writer method=set_rows effective_changed={changed} actual_invalidate={}",
                changed
            );
        }
        if !changed {
            return;
        }
        *self.rows.borrow_mut() = rows;
        self.resolved_row_sizes.borrow_mut().clear();
        self.resolved_column_sizes.borrow_mut().clear();
        self.invalidate_measure();
    }
    fn set_columns(&self, columns: Vec<GridLength>) {
        let changed = *self.columns.borrow() != columns;
        if std::env::var_os("ELWINDUI_PERF_TRACE").is_some() {
            eprintln!(
                "[perf] grid_writer method=set_columns effective_changed={changed} actual_invalidate={}",
                changed
            );
        }
        if !changed {
            return;
        }
        *self.columns.borrow_mut() = columns;
        self.resolved_column_sizes.borrow_mut().clear();
        self.resolved_row_sizes.borrow_mut().clear();
        self.invalidate_measure();
    }
    fn set_row_spacing(&self, spacing: f32) {
        let spacing = effective_grid_spacing(spacing);
        if self.row_spacing.get() == spacing {
            return;
        }
        self.row_spacing.set(spacing);
        self.resolved_row_sizes.borrow_mut().clear();
        self.resolved_column_sizes.borrow_mut().clear();
        self.invalidate_measure();
    }
    fn set_column_spacing(&self, spacing: f32) {
        let spacing = effective_grid_spacing(spacing);
        if self.column_spacing.get() == spacing {
            return;
        }
        self.column_spacing.set(spacing);
        self.resolved_row_sizes.borrow_mut().clear();
        self.resolved_column_sizes.borrow_mut().clear();
        self.invalidate_measure();
    }
    fn set_row_constraints(&self, constraints: Vec<GridTrackConstraint>) {
        if *self.row_constraints.borrow() == constraints {
            return;
        }
        *self.row_constraints.borrow_mut() = constraints;
        self.resolved_row_sizes.borrow_mut().clear();
        self.resolved_column_sizes.borrow_mut().clear();
        self.invalidate_measure();
    }
    fn set_column_constraints(&self, constraints: Vec<GridTrackConstraint>) {
        if *self.column_constraints.borrow() == constraints {
            return;
        }
        *self.column_constraints.borrow_mut() = constraints;
        self.resolved_column_sizes.borrow_mut().clear();
        self.resolved_row_sizes.borrow_mut().clear();
        self.invalidate_measure();
    }
    pub fn resolved_row_sizes(&self) -> Vec<f32> {
        self.resolved_row_sizes.borrow().clone()
    }
    pub fn resolved_column_sizes(&self) -> Vec<f32> {
        self.resolved_column_sizes.borrow().clone()
    }
    fn construct() -> Self {
        Self {
            base: Layout::construct(),
            rows: RefCell::new(Vec::new()),
            columns: RefCell::new(Vec::new()),
            row_spacing: Cell::new(0.0),
            column_spacing: Cell::new(0.0),
            row_constraints: RefCell::new(Vec::new()),
            column_constraints: RefCell::new(Vec::new()),
            resolved_row_sizes: RefCell::new(Vec::new()),
            resolved_column_sizes: RefCell::new(Vec::new()),
        }
    }
}

fn effective_grid_spacing(spacing: f32) -> f32 {
    if spacing.is_finite() && spacing > 0.0 {
        spacing
    } else {
        0.0
    }
}

fn grid_spacing_total(track_count: usize, spacing: f32) -> f32 {
    track_count.saturating_sub(1) as f32 * effective_grid_spacing(spacing)
}

fn grid_track_available(
    available: Size,
    row_count: usize,
    column_count: usize,
    row_spacing: f32,
    column_spacing: f32,
) -> Size {
    let subtract_spacing = |axis: f32, count: usize, spacing: f32| {
        if axis.is_finite() {
            (axis - grid_spacing_total(count, spacing)).max(0.0)
        } else {
            axis
        }
    };
    Size {
        width: subtract_spacing(available.width, column_count, column_spacing),
        height: subtract_spacing(available.height, row_count, row_spacing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn identical_track_definitions_do_not_invalidate_measure() {
        struct CountingHost {
            calls: RefCell<usize>,
        }

        impl RelayoutHost for CountingHost {
            fn request_relayout(&self, _dirty_group_id: u64, _kind: InvalidationKind) {
                *self.calls.borrow_mut() += 1;
            }
        }

        let root = Grid::new();
        let host = Rc::new(CountingHost {
            calls: RefCell::new(0),
        });
        root.set_invalidate_host(Some(host.clone()));
        let rows = vec![GridLength::Fixed(10.0)];
        let columns = vec![GridLength::Star(1.0)];

        root.set_rows(rows.clone());
        root.set_columns(columns.clone());
        assert_eq!(*host.calls.borrow(), 2);

        root.set_rows(rows);
        root.set_columns(columns);
        assert_eq!(*host.calls.borrow(), 2);

        root.set_row_spacing(8.0);
        root.set_column_spacing(12.0);
        assert_eq!(*host.calls.borrow(), 4);
        root.set_row_spacing(8.0);
        root.set_column_spacing(12.0);
        assert_eq!(*host.calls.borrow(), 4);
    }

    #[test]
    fn grid_measures_children_in_two_passes_per_track_kind() {
        // Single Auto row; Fixed(50)/Auto/Star(1.0) columns, one child per column.
        let root = Grid::new();
        root.set_rows(vec![GridLength::Auto]);
        root.set_columns(vec![
            GridLength::Fixed(50.0),
            GridLength::Auto,
            GridLength::Star(1.0),
        ]);

        let fixed_child = MeasureProbe::new(size(10.0, 10.0));
        fixed_child.set_attached("Grid", "column", 0i32);
        let auto_child = MeasureProbe::new(size(30.0, 10.0));
        auto_child.set_attached("Grid", "column", 1i32);
        let star_child = MeasureProbe::new(size(20.0, 10.0));
        star_child.set_attached("Grid", "column", 2i32);

        root.children().add(fixed_child.clone());
        root.children().add(auto_child.clone());
        root.children().add(star_child.clone());

        root.measure(size(300.0, 100.0));

        // Pass 1: `Fixed` measures at its own literal size; `Auto`/`Star` measure unconstrained
        // (on both axes -- the row is `Auto` too) so each child's own natural size is exactly what
        // comes back to resolve its track.
        assert_eq!(
            fixed_child.calls.borrow()[0],
            Size {
                width: 50.0,
                height: f32::INFINITY
            }
        );
        assert!(auto_child.calls.borrow()[0].width.is_infinite());
        assert!(star_child.calls.borrow()[0].width.is_infinite());

        // Pass 2: every child is re-measured at its now-fully-resolved cell size — `Fixed` column
        // stays 50, `Auto` column becomes its own natural width (30), `Star` column gets whatever's
        // left (300 - 50 - 30 = 220). The `Auto` row resolves to 10 (every child's own natural
        // height) and every child is re-measured at that height too.
        assert_eq!(
            fixed_child.last_available(),
            Size {
                width: 50.0,
                height: 10.0
            }
        );
        assert_eq!(
            auto_child.last_available(),
            Size {
                width: 30.0,
                height: 10.0
            }
        );
        assert_eq!(
            star_child.last_available(),
            Size {
                width: 220.0,
                height: 10.0
            }
        );
    }

    #[test]
    fn grid_resolved_track_sizes_are_empty_until_arrange_and_clear_on_definition_change() {
        let root = Grid::new();
        assert!(root.resolved_row_sizes().is_empty());
        assert!(root.resolved_column_sizes().is_empty());

        root.set_rows(vec![GridLength::Fixed(10.0)]);
        root.set_columns(vec![GridLength::Fixed(40.0), GridLength::Star(1.0)]);
        root.measure(Size {
            width: 200.0,
            height: 100.0,
        });
        root.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 100.0,
        });
        assert_eq!(root.resolved_row_sizes(), vec![10.0]);
        assert_eq!(root.resolved_column_sizes(), vec![40.0, 160.0]);

        root.set_columns(vec![GridLength::Fixed(80.0), GridLength::Star(1.0)]);
        assert!(root.resolved_row_sizes().is_empty());
        assert!(root.resolved_column_sizes().is_empty());
        root.measure(Size {
            width: 200.0,
            height: 100.0,
        });
        root.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 100.0,
        });
        assert_eq!(root.resolved_column_sizes(), vec![80.0, 120.0]);
    }

    #[test]
    fn grid_spacing_is_included_in_measure_and_arrange_but_not_track_sizes() {
        let root = Grid::new();
        root.set_rows(vec![GridLength::Star(1.0), GridLength::Star(1.0)]);
        root.set_columns(vec![GridLength::Star(1.0), GridLength::Star(1.0)]);
        root.set_row_spacing(12.0);
        root.set_column_spacing(12.0);

        let mut children = Vec::new();
        for row in 0..2 {
            for column in 0..2 {
                let child = MeasureProbe::new(size(10.0, 10.0));
                child.set_attached("Grid", "row", row);
                child.set_attached("Grid", "column", column);
                root.children().add(child.clone());
                children.push(child);
            }
        }

        root.measure(size(212.0, 112.0));
        assert_eq!(
            root.measured_size(),
            Some(size(212.0, 112.0)),
            "desired size includes one row and column gap"
        );

        root.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 212.0,
            height: 112.0,
        });

        assert_eq!(root.resolved_column_sizes(), vec![100.0, 100.0]);
        assert_eq!(root.resolved_row_sizes(), vec![50.0, 50.0]);
        let lower_right = &children[3];
        assert_eq!(
            lower_right.arranged_offset(),
            Some(Point { x: 112.0, y: 62.0 })
        );
        assert_eq!(lower_right.arranged_width(), Some(100.0));
        assert_eq!(lower_right.arranged_height(), Some(50.0));

        root.set_column_spacing(f32::INFINITY);
        assert!(root.resolved_column_sizes().is_empty());
        assert!(root.resolved_row_sizes().is_empty());
        assert_eq!(root.column_spacing.get(), 0.0);
    }
}
