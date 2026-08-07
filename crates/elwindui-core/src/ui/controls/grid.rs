//! `builtin::Grid` — row/column layout, plus the attached-property read-back its cell placement uses.

use super::*;

/// WPF/WinUI3-style row/column layout (`builtin::Grid`, docs/specs/dsl_spec.md §3). Each child's
/// cell placement comes from its own `UIElement::attached` bag (the `Grid::row`/`Grid::column`
/// attached properties it was constructed with, read back via `grid_cell_of` since only `Grid`
/// itself knows those two fields are `i32`), not a field on `Grid` itself — see `attached`'s
/// own doc comment. A child whose cell falls outside `row_definitions`/`column_definitions`'
/// bounds is clamped to the last row/column, mirroring `grid_arrange`'s own clamping. Row/column
/// spanning is out of scope for this pass (one child per cell) — a future `#[attached]
/// row_span`/`column_span` pair on `builtin::Grid` would extend this the same way `row`/`column`
/// were added, with no changes needed here beyond consulting the extra fields.
/// `rows`/`columns` (not `row_definitions`/`column_definitions`) to match `builtin::Grid`'s own
/// `#[param] rows`/`#[param] columns` names — `elwindui-codegen`'s setter-based construction calls
/// `.set_{param name}(..)` generically, so the Rust field/setter name must agree with the DSL's.
/// `Grid`'s own class trait (docs/design/gui_framework_design.md §5.1) — inherits `Layout` (like
/// `VerticalLayout`/`HorizontalLayout`), so `children` comes from that shared base rather than
/// being declared on `Grid` itself (docs/specs/builtins_spec.md 付録F.11).
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
#[content(children)]
#[prop(rows: Vec<crate::layout::GridLength>)]
#[prop(columns: Vec<crate::layout::GridLength>)]
#[prop(attached, row: i32 = 0)]
#[prop(attached, column: i32 = 0)]
pub struct Grid {
    pub rows: RefCell<Vec<GridLength>>,
    pub columns: RefCell<Vec<GridLength>>,
}

#[elwindui_macros::class]
impl Grid {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let children = self.children().to_vec();
        let cells: Vec<GridCell> = children.iter().map(grid_cell_of).collect();
        let rows = self.rows.borrow();
        let columns = self.columns.borrow();

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

        let (row_sizes, col_sizes) =
            grid_resolve_track_sizes(&rows, &columns, &cells, &pass1_sizes, available);

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
            width: col_sizes.iter().sum(),
            height: row_sizes.iter().sum(),
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
        let child_rects = grid_arrange(
            final_size,
            &self.rows.borrow(),
            &self.columns.borrow(),
            &cells,
            &child_sizes,
        );
        for (child, rect) in children.iter().zip(child_rects) {
            child.arrange(rect);
        }
        final_size
    }
    fn set_rows(&self, rows: Vec<GridLength>) {
        *self.rows.borrow_mut() = rows;
        self.invalidate_measure();
    }
    fn set_columns(&self, columns: Vec<GridLength>) {
        *self.columns.borrow_mut() = columns;
        self.invalidate_measure();
    }
    fn construct() -> Self {
        Self {
            base: Layout::construct(),
            rows: RefCell::new(Vec::new()),
            columns: RefCell::new(Vec::new()),
        }
    }
}

