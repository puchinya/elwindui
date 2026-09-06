use super::core::base::Point;
use super::core::graphics::{Brush, Color};
use super::core::input::{Key, KeyEventArgs, MouseButton, PointerEventArgs};
use super::core::layout::{
    GridLength, GridTrackConstraint, HorizontalAlignment, VerticalAlignment,
};
use super::core::ui::{ControlExt, Grid, GridExt, Rectangle, ShapeExt, UIElementExt};
use super::{
    GridResizeBehavior, GridResizeDirection, GridSplitterInputKind,
    GridSplitterResizeCompletedEventArgs, GridSplitterResizeDeltaEventArgs,
    GridSplitterResizeStartedEventArgs, weak_self_from_visual_owner,
};
use std::rc::{Rc, Weak};

#[derive(Clone)]
pub struct ResizeSession {
    direction: GridResizeDirection,
    target_index: usize,
    sibling_index: usize,
    input_kind: GridSplitterInputKind,
    grid: Weak<dyn UIElementExt>,
    original_tracks: Vec<GridLength>,
    baseline_sizes: Vec<f32>,
    target_min: f32,
    target_max: f32,
    sibling_min: f32,
    sibling_max: f32,
    start_position: Option<Point>,
    position: Option<Point>,
    screen_position: Option<Point>,
    last_effective_delta: f32,
    increment: f32,
}

fn effective_increment(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

fn constraint_at(constraints: &[GridTrackConstraint], index: usize) -> (f32, f32) {
    constraints
        .get(index)
        .copied()
        .unwrap_or_default()
        .effective()
}

fn resolve_pair(
    direction: GridResizeDirection,
    behavior: GridResizeBehavior,
    index: usize,
    horizontal_alignment: HorizontalAlignment,
    vertical_alignment: VerticalAlignment,
) -> Option<(usize, usize)> {
    let behavior = match behavior {
        GridResizeBehavior::BasedOnAlignment => match direction {
            GridResizeDirection::Columns => match horizontal_alignment {
                HorizontalAlignment::Left => GridResizeBehavior::PreviousAndCurrent,
                HorizontalAlignment::Right => GridResizeBehavior::CurrentAndNext,
                HorizontalAlignment::Center | HorizontalAlignment::Stretch => {
                    GridResizeBehavior::PreviousAndNext
                }
            },
            GridResizeDirection::Rows => match vertical_alignment {
                VerticalAlignment::Top => GridResizeBehavior::PreviousAndCurrent,
                VerticalAlignment::Bottom => GridResizeBehavior::CurrentAndNext,
                VerticalAlignment::Center | VerticalAlignment::Stretch => {
                    GridResizeBehavior::PreviousAndNext
                }
            },
            GridResizeDirection::Auto => return None,
        },
        value => value,
    };

    match behavior {
        GridResizeBehavior::CurrentAndNext => Some((index, index.checked_add(1)?)),
        GridResizeBehavior::PreviousAndCurrent => Some((index.checked_sub(1)?, index)),
        GridResizeBehavior::PreviousAndNext => Some((index.checked_sub(1)?, index.checked_add(1)?)),
        GridResizeBehavior::BasedOnAlignment => None,
    }
}

fn resize_definitions(
    original_tracks: &[GridLength],
    baseline_sizes: &[f32],
    target_index: usize,
    sibling_index: usize,
    delta: f32,
    target_min: f32,
    target_max: f32,
    sibling_min: f32,
    sibling_max: f32,
) -> Option<(Vec<GridLength>, f32)> {
    let target_baseline = *baseline_sizes.get(target_index)?;
    let sibling_baseline = *baseline_sizes.get(sibling_index)?;
    if !target_baseline.is_finite()
        || !sibling_baseline.is_finite()
        || !delta.is_finite()
        || !target_min.is_finite()
        || target_max.is_nan()
        || !sibling_min.is_finite()
        || sibling_max.is_nan()
    {
        return None;
    }

    let lower = (target_min - target_baseline).max(sibling_baseline - sibling_max);
    let upper = (target_max - target_baseline).min(sibling_baseline - sibling_min);
    let (lower, upper) = if lower <= upper {
        (lower, upper)
    } else {
        (upper, lower)
    };
    let effective_delta = delta.clamp(lower, upper);
    if !effective_delta.is_finite() {
        return None;
    }

    let target_definition = *original_tracks.get(target_index)?;
    let sibling_definition = *original_tracks.get(sibling_index)?;
    let target_star = matches!(target_definition, GridLength::Star(_));
    let sibling_star = matches!(sibling_definition, GridLength::Star(_));
    let mut tracks = original_tracks.to_vec();

    match (target_star, sibling_star) {
        (false, false) => {
            tracks[target_index] = GridLength::Fixed(target_baseline + effective_delta);
            tracks[sibling_index] = GridLength::Fixed(sibling_baseline - effective_delta);
        }
        (false, true) => {
            tracks[target_index] = GridLength::Fixed(target_baseline + effective_delta);
        }
        (true, false) => {
            tracks[sibling_index] = GridLength::Fixed(sibling_baseline - effective_delta);
        }
        (true, true) => {
            const EPSILON: f32 = 0.0001;
            for (index, definition) in original_tracks.iter().enumerate() {
                if let GridLength::Star(_) = definition {
                    let mut basis = baseline_sizes.get(index).copied()?;
                    if index == target_index {
                        basis += effective_delta;
                    } else if index == sibling_index {
                        basis -= effective_delta;
                    }
                    if !basis.is_finite() {
                        return None;
                    }
                    tracks[index] = GridLength::Star(basis.max(EPSILON));
                }
            }
        }
    }

    if tracks.iter().any(|track| match track {
        GridLength::Fixed(value) | GridLength::Star(value) => !value.is_finite(),
        GridLength::Auto => false,
    }) {
        return None;
    }
    Some((tracks, effective_delta))
}

fn snap_delta(raw_delta: f32, increment: f32) -> Option<f32> {
    if !raw_delta.is_finite() || !increment.is_finite() || increment <= 0.0 {
        return None;
    }
    let snapped = (raw_delta / increment).trunc() * increment;
    snapped.is_finite().then_some(snapped)
}

#[elwindui::component(inherits Control)]
pub struct CustomGridSplitter {
    #[prop(default = GridResizeDirection::Auto)]
    resize_direction: GridResizeDirection,
    #[prop(default = GridResizeBehavior::BasedOnAlignment)]
    resize_behavior: GridResizeBehavior,
    #[prop(default = 0)]
    parent_level: usize,
    #[prop(default = 1.0)]
    drag_increment: f32,
    #[prop(default = 8.0)]
    keyboard_increment: f32,
    #[state(default = None)]
    resize_started_callback: Option<Rc<dyn Fn(GridSplitterResizeStartedEventArgs)>>,
    #[state(default = None)]
    resize_delta_callback: Option<Rc<dyn Fn(GridSplitterResizeDeltaEventArgs)>>,
    #[state(default = None)]
    resize_completed_callback: Option<Rc<dyn Fn(GridSplitterResizeCompletedEventArgs)>>,
    #[state(default = None)]
    resize_session: Option<ResizeSession>,
    #[state(default = false)]
    pointer_over: bool,
    #[state(default = false)]
    pressed: bool,
    #[state(default = false)]
    focused: bool,
    template: template_view!(|this: Self| {
        on_mount {
            this.set_tab_stop(true);
            this.bind_input_handlers();
            this.sync_visual(true);
            let weak_self = weak_self_from_visual_owner(this.as_ref());
            this.add_unmount_hook(Box::new(move || {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.set_resize_session(None);
                }
            }));
        }
        on_update(resize_direction) {
            this.sync_visual(true);
        }
        Rectangle {
            width: 6.0
            height: 6.0
            fill: "#7a7f87"
            corner_radius: 2.0
        }
    }),
}

#[elwindui::component]
impl CustomGridSplitter {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        true
    }

    #[overrides]
    fn on_apply_template(&self) {
        self.sync_visual(true);
    }
}

impl CustomGridSplitter {
    pub fn new_splitter() -> Rc<Self> {
        Self::new()
    }

    pub fn set_on_resize_started(&self, callback: Box<dyn Fn(GridSplitterResizeStartedEventArgs)>) {
        self.set_resize_started_callback(Some(Rc::from(callback)));
    }

    pub fn set_on_resize_delta(&self, callback: Box<dyn Fn(GridSplitterResizeDeltaEventArgs)>) {
        self.set_resize_delta_callback(Some(Rc::from(callback)));
    }

    pub fn set_on_resize_completed(
        &self,
        callback: Box<dyn Fn(GridSplitterResizeCompletedEventArgs)>,
    ) {
        self.set_resize_completed_callback(Some(Rc::from(callback)));
    }

    fn visual_fill(&self) -> Brush {
        let color = if self.pressed() {
            Color::rgb(96, 205, 255)
        } else if self.pointer_over() || self.focused() {
            Color::rgb(141, 200, 255)
        } else {
            Color::rgb(122, 127, 135)
        };
        Brush::Solid(color)
    }

    fn sync_visual(&self, invalidate_layout: bool) {
        let Some(root) = self.visual_children().into_iter().next() else {
            return;
        };
        let Some(rectangle) = root.as_any().downcast_ref::<Rectangle>() else {
            return;
        };
        let element = rectangle.as_ui_element();
        if invalidate_layout {
            match self.resize_direction() {
                GridResizeDirection::Columns => {
                    element.width.set(Some(6.0));
                    element.height.set(None);
                    element.min_width.set(None);
                    element.min_height.set(Some(6.0));
                    element
                        .horizontal_alignment
                        .set(HorizontalAlignment::Stretch);
                    element.vertical_alignment.set(VerticalAlignment::Stretch);
                }
                GridResizeDirection::Rows => {
                    element.width.set(None);
                    element.height.set(Some(6.0));
                    element.min_width.set(Some(6.0));
                    element.min_height.set(None);
                    element
                        .horizontal_alignment
                        .set(HorizontalAlignment::Stretch);
                    element.vertical_alignment.set(VerticalAlignment::Stretch);
                }
                GridResizeDirection::Auto => {
                    element.width.set(Some(6.0));
                    element.height.set(Some(6.0));
                    element.min_width.set(None);
                    element.min_height.set(None);
                    element
                        .horizontal_alignment
                        .set(HorizontalAlignment::Center);
                    element.vertical_alignment.set(VerticalAlignment::Center);
                }
            }
        }
        rectangle.set_fill_render_only(Some(self.visual_fill()));
    }

    fn target_control(&self) -> Option<Rc<dyn UIElementExt>> {
        if self.parent_level() == 0 {
            let owner = weak_self_from_visual_owner(self).upgrade()?;
            let target: Rc<dyn UIElementExt> = owner;
            return Some(target);
        }

        let mut target = self.visual_parent()?;
        for _ in 1..self.parent_level() {
            target = target.visual_parent()?;
        }
        Some(target)
    }

    fn resolve_direction(&self, target: &Rc<dyn UIElementExt>) -> Option<GridResizeDirection> {
        match self.resize_direction() {
            GridResizeDirection::Columns | GridResizeDirection::Rows => {
                Some(self.resize_direction())
            }
            GridResizeDirection::Auto => {
                if target.horizontal_alignment() != HorizontalAlignment::Stretch {
                    Some(GridResizeDirection::Columns)
                } else if target.vertical_alignment() != VerticalAlignment::Stretch {
                    Some(GridResizeDirection::Rows)
                } else if target.arranged_width() <= target.arranged_height() {
                    Some(GridResizeDirection::Columns)
                } else {
                    Some(GridResizeDirection::Rows)
                }
            }
        }
    }

    fn begin_session(
        &self,
        input_kind: GridSplitterInputKind,
        start_position: Option<Point>,
        position: Option<Point>,
        screen_position: Option<Point>,
    ) -> Option<ResizeSession> {
        let target = self.target_control()?;
        let grid = target.visual_parent()?;
        grid.as_any().downcast_ref::<Grid>()?;
        let direction = self.resolve_direction(&target)?;
        let placement = match direction {
            GridResizeDirection::Columns => target
                .as_ui_element()
                .get_attached::<i32>("Grid", "column", 0),
            GridResizeDirection::Rows => {
                target.as_ui_element().get_attached::<i32>("Grid", "row", 0)
            }
            GridResizeDirection::Auto => return None,
        };
        if placement < 0 {
            return None;
        }
        let placement = placement as usize;
        let behavior = self.resize_behavior();
        let (target_index, sibling_index) = resolve_pair(
            direction,
            behavior,
            placement,
            target.horizontal_alignment(),
            target.vertical_alignment(),
        )?;

        let (original_tracks, baseline_sizes, constraints) = {
            let grid = grid.as_any().downcast_ref::<Grid>()?;
            match direction {
                GridResizeDirection::Columns => (
                    grid.columns.borrow().clone(),
                    grid.resolved_column_sizes(),
                    grid.column_constraints.borrow().clone(),
                ),
                GridResizeDirection::Rows => (
                    grid.rows.borrow().clone(),
                    grid.resolved_row_sizes(),
                    grid.row_constraints.borrow().clone(),
                ),
                GridResizeDirection::Auto => return None,
            }
        };
        if baseline_sizes.is_empty()
            || baseline_sizes.len() != original_tracks.len().max(1)
            || baseline_sizes.iter().any(|size| !size.is_finite())
        {
            return None;
        }
        let (target_min, target_max) = constraint_at(&constraints, target_index);
        let (sibling_min, sibling_max) = constraint_at(&constraints, sibling_index);
        let increment = match input_kind {
            GridSplitterInputKind::Pointer => effective_increment(self.drag_increment(), 1.0),
            GridSplitterInputKind::Keyboard => effective_increment(self.keyboard_increment(), 8.0),
        };

        Some(ResizeSession {
            direction,
            target_index,
            sibling_index,
            input_kind,
            grid: Rc::downgrade(&grid),
            original_tracks,
            baseline_sizes,
            target_min,
            target_max,
            sibling_min,
            sibling_max,
            start_position,
            position,
            screen_position,
            last_effective_delta: 0.0,
            increment,
        })
    }

    fn apply_tracks(&self, session: &ResizeSession, tracks: Vec<GridLength>) -> bool {
        let Some(grid) = session.grid.upgrade() else {
            return false;
        };
        let Some(grid) = grid.as_any().downcast_ref::<Grid>() else {
            return false;
        };
        match session.direction {
            GridResizeDirection::Columns => grid.set_columns(tracks),
            GridResizeDirection::Rows => grid.set_rows(tracks),
            GridResizeDirection::Auto => return false,
        }
        grid.flush_interactive_relayout();
        true
    }

    fn apply_delta(&self, session: &ResizeSession, requested_delta: f32) -> Option<f32> {
        let (tracks, effective_delta) = resize_definitions(
            &session.original_tracks,
            &session.baseline_sizes,
            session.target_index,
            session.sibling_index,
            requested_delta,
            session.target_min,
            session.target_max,
            session.sibling_min,
            session.sibling_max,
        )?;
        self.apply_tracks(session, tracks)
            .then_some(effective_delta)
    }

    fn started_args(session: &ResizeSession) -> GridSplitterResizeStartedEventArgs {
        GridSplitterResizeStartedEventArgs {
            direction: session.direction,
            target_index: session.target_index,
            sibling_index: session.sibling_index,
            input_kind: session.input_kind,
            position: session.position,
            screen_position: session.screen_position,
        }
    }

    fn emit_started(&self, session: &ResizeSession) {
        if let Some(callback) = self.resize_started_callback() {
            callback(Self::started_args(session));
        }
    }

    fn emit_delta(&self, session: &ResizeSession, delta: f32) {
        if let Some(callback) = self.resize_delta_callback() {
            callback(GridSplitterResizeDeltaEventArgs {
                delta,
                cumulative_delta: session.last_effective_delta,
                direction: session.direction,
                target_index: session.target_index,
                sibling_index: session.sibling_index,
                input_kind: session.input_kind,
                position: session.position,
                screen_position: session.screen_position,
            });
        }
    }

    fn emit_completed(&self, session: &ResizeSession, canceled: bool) {
        if let Some(callback) = self.resize_completed_callback() {
            callback(GridSplitterResizeCompletedEventArgs {
                cumulative_delta: session.last_effective_delta,
                direction: session.direction,
                target_index: session.target_index,
                sibling_index: session.sibling_index,
                input_kind: session.input_kind,
                position: session.position,
                screen_position: session.screen_position,
                canceled,
            });
        }
    }

    fn pointer_pressed(&self, event: PointerEventArgs) {
        if event.button != Some(MouseButton::Left) || self.resize_session().is_some() {
            return;
        }
        let Some(session) = self.begin_session(
            GridSplitterInputKind::Pointer,
            Some(event.position),
            Some(event.position),
            event.screen_position,
        ) else {
            return;
        };
        self.set_pressed(true);
        self.sync_visual(false);
        self.set_resize_session(Some(session.clone()));
        self.emit_started(&session);
    }

    fn pointer_moved(&self, event: PointerEventArgs) {
        let Some(mut session) = self.resize_session() else {
            return;
        };
        if session.input_kind != GridSplitterInputKind::Pointer {
            return;
        }
        let Some(start) = session.start_position else {
            return;
        };
        let raw_delta = match session.direction {
            GridResizeDirection::Columns => event.position.x - start.x,
            GridResizeDirection::Rows => event.position.y - start.y,
            GridResizeDirection::Auto => return,
        };
        let Some(snapped_delta) = snap_delta(raw_delta, session.increment) else {
            return;
        };
        if snapped_delta == session.last_effective_delta {
            session.position = Some(event.position);
            session.screen_position = event.screen_position;
            self.set_resize_session(Some(session));
            return;
        }
        let old_delta = session.last_effective_delta;
        let Some(effective_delta) = self.apply_delta(&session, snapped_delta) else {
            return;
        };
        session.position = Some(event.position);
        session.screen_position = event.screen_position;
        session.last_effective_delta = effective_delta;
        self.set_resize_session(Some(session.clone()));
        if effective_delta != old_delta {
            self.emit_delta(&session, effective_delta - old_delta);
        }
    }

    fn pointer_released(&self, event: PointerEventArgs) {
        let Some(mut session) = self.resize_session() else {
            return;
        };
        if session.input_kind != GridSplitterInputKind::Pointer {
            return;
        }
        if let Some(start) = session.start_position {
            let raw_delta = match session.direction {
                GridResizeDirection::Columns => event.position.x - start.x,
                GridResizeDirection::Rows => event.position.y - start.y,
                GridResizeDirection::Auto => return,
            };
            if let Some(snapped_delta) = snap_delta(raw_delta, session.increment) {
                if snapped_delta != session.last_effective_delta {
                    if let Some(effective_delta) = self.apply_delta(&session, snapped_delta) {
                        session.last_effective_delta = effective_delta;
                    }
                }
            }
        }
        session.position = Some(event.position);
        session.screen_position = event.screen_position;
        self.set_pressed(false);
        self.sync_visual(false);
        self.set_resize_session(None);
        self.emit_completed(&session, false);
    }

    fn pointer_canceled(&self) {
        let Some(session) = self.resize_session() else {
            return;
        };
        if session.input_kind != GridSplitterInputKind::Pointer {
            return;
        }
        self.set_pressed(false);
        self.sync_visual(false);
        self.set_resize_session(None);
        self.apply_tracks(&session, session.original_tracks.clone());
        self.emit_completed(&session, true);
    }

    fn keyboard_delta(&self, key: Key, direction: GridResizeDirection) -> Option<f32> {
        let increment = effective_increment(self.keyboard_increment(), 8.0);
        match (direction, key) {
            (GridResizeDirection::Columns, Key::Left) => Some(-increment),
            (GridResizeDirection::Columns, Key::Right) => Some(increment),
            (GridResizeDirection::Rows, Key::Up) => Some(-increment),
            (GridResizeDirection::Rows, Key::Down) => Some(increment),
            _ => None,
        }
    }

    fn key_down(&self, event: KeyEventArgs) -> bool {
        if self.resize_session().is_some() {
            return false;
        }
        let Some(target) = self.target_control() else {
            return false;
        };
        let Some(direction) = self.resolve_direction(&target) else {
            return false;
        };
        let Some(requested_delta) = self.keyboard_delta(event.key, direction) else {
            return false;
        };
        let Some(mut session) =
            self.begin_session(GridSplitterInputKind::Keyboard, None, None, None)
        else {
            return false;
        };
        self.set_resize_session(Some(session.clone()));
        self.emit_started(&session);
        if let Some(effective_delta) = self.apply_delta(&session, requested_delta) {
            session.last_effective_delta = effective_delta;
            self.set_resize_session(Some(session.clone()));
            if effective_delta != 0.0 {
                self.emit_delta(&session, effective_delta);
            }
        }
        self.set_resize_session(None);
        self.emit_completed(&session, false);
        true
    }

    fn bind_input_handlers(&self) {
        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.pointer_pressed(*event);
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |event, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.pointer_moved(*event);
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.pointer_released(*event);
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |_, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.pointer_canceled();
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_entered",
            Box::new(move |_, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.set_pointer_over(true);
                    splitter.sync_visual(false);
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_exited",
            Box::new(move |_, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.set_pointer_over(false);
                    splitter.sync_visual(false);
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<()>(
            "on_got_focus",
            Box::new(move |_, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.set_focused(true);
                    splitter.sync_visual(false);
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<()>(
            "on_lost_focus",
            Box::new(move |_, _| {
                if let Some(splitter) = weak_self.upgrade() {
                    splitter.set_focused(false);
                    splitter.sync_visual(false);
                }
            }),
        );

        let weak_self = weak_self_from_visual_owner(self);
        self.register_routed_handler::<KeyEventArgs>(
            "on_key_down",
            Box::new(move |event, routed| {
                if let Some(splitter) = weak_self.upgrade() {
                    if splitter.key_down(*event) {
                        routed.handled.set(true);
                    }
                }
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_uses_truncation_for_negative_cumulative_delta() {
        assert_eq!(snap_delta(-15.0, 16.0), Some(0.0));
        assert_eq!(snap_delta(-17.0, 16.0), Some(-16.0));
    }

    #[test]
    fn resize_pair_preserves_star_semantics() {
        let original = [
            GridLength::Star(1.0),
            GridLength::Star(1.0),
            GridLength::Star(1.0),
        ];
        let baseline = [100.0, 200.0, 300.0];
        let (tracks, delta) =
            resize_definitions(&original, &baseline, 0, 1, 40.0, 0.0, 500.0, 0.0, 500.0)
                .expect("valid pair");
        assert_eq!(delta, 40.0);
        assert_eq!(tracks[0], GridLength::Star(140.0));
        assert_eq!(tracks[1], GridLength::Star(160.0));
        assert_eq!(tracks[2], GridLength::Star(300.0));
    }

    #[test]
    fn auto_tracks_become_fixed_when_resized() {
        let original = [GridLength::Auto, GridLength::Fixed(6.0)];
        let baseline = [40.0, 6.0];
        let (tracks, _) =
            resize_definitions(&original, &baseline, 0, 1, 8.0, 0.0, 100.0, 0.0, 100.0)
                .expect("valid pair");
        assert_eq!(tracks, [GridLength::Fixed(46.0), GridLength::Fixed(0.0)]);
    }

    #[test]
    fn resize_engine_covers_all_grid_length_pair_shapes() {
        let cases = [
            (
                [GridLength::Fixed(100.0), GridLength::Fixed(80.0)],
                [GridLength::Fixed(120.0), GridLength::Fixed(60.0)],
            ),
            (
                [GridLength::Fixed(100.0), GridLength::Star(1.0)],
                [GridLength::Fixed(120.0), GridLength::Star(1.0)],
            ),
            (
                [GridLength::Star(1.0), GridLength::Fixed(80.0)],
                [GridLength::Star(1.0), GridLength::Fixed(60.0)],
            ),
            (
                [GridLength::Auto, GridLength::Fixed(80.0)],
                [GridLength::Fixed(120.0), GridLength::Fixed(60.0)],
            ),
            (
                [GridLength::Fixed(100.0), GridLength::Auto],
                [GridLength::Fixed(120.0), GridLength::Fixed(60.0)],
            ),
            (
                [GridLength::Auto, GridLength::Star(1.0)],
                [GridLength::Fixed(120.0), GridLength::Star(1.0)],
            ),
            (
                [GridLength::Star(1.0), GridLength::Auto],
                [GridLength::Star(1.0), GridLength::Fixed(60.0)],
            ),
            (
                [GridLength::Auto, GridLength::Auto],
                [GridLength::Fixed(120.0), GridLength::Fixed(60.0)],
            ),
        ];

        for (original, expected) in cases {
            let (tracks, effective_delta) = resize_definitions(
                &original,
                &[100.0, 80.0],
                0,
                1,
                20.0,
                0.0,
                500.0,
                0.0,
                500.0,
            )
            .expect("valid GridLength pair");
            assert_eq!(effective_delta, 20.0);
            assert_eq!(tracks, expected);
        }
    }

    #[test]
    fn resize_engine_derives_each_clamped_preview_from_the_baseline() {
        let original = [GridLength::Fixed(100.0), GridLength::Fixed(80.0)];
        let baseline = [100.0, 80.0];

        let (at_upper, _) =
            resize_definitions(&original, &baseline, 0, 1, 200.0, 0.0, 140.0, 0.0, 100.0)
                .expect("upper clamp");
        let (back_inside, _) =
            resize_definitions(&original, &baseline, 0, 1, 20.0, 0.0, 140.0, 0.0, 100.0)
                .expect("baseline-derived preview");

        assert_eq!(
            at_upper,
            [GridLength::Fixed(140.0), GridLength::Fixed(40.0)]
        );
        assert_eq!(
            back_inside,
            [GridLength::Fixed(120.0), GridLength::Fixed(60.0)]
        );
    }

    #[test]
    fn resize_behavior_rejects_pairs_outside_the_track_axis() {
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Columns,
                GridResizeBehavior::PreviousAndCurrent,
                0,
                HorizontalAlignment::Stretch,
                VerticalAlignment::Stretch,
            ),
            None
        );
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Rows,
                GridResizeBehavior::CurrentAndNext,
                usize::MAX,
                HorizontalAlignment::Stretch,
                VerticalAlignment::Stretch,
            ),
            None
        );
    }

    #[test]
    fn resize_behavior_resolves_the_contract_pairs() {
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Columns,
                GridResizeBehavior::CurrentAndNext,
                2,
                HorizontalAlignment::Stretch,
                VerticalAlignment::Stretch,
            ),
            Some((2, 3))
        );
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Columns,
                GridResizeBehavior::PreviousAndCurrent,
                2,
                HorizontalAlignment::Stretch,
                VerticalAlignment::Stretch,
            ),
            Some((1, 2))
        );
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Columns,
                GridResizeBehavior::PreviousAndNext,
                2,
                HorizontalAlignment::Stretch,
                VerticalAlignment::Stretch,
            ),
            Some((1, 3))
        );
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Columns,
                GridResizeBehavior::BasedOnAlignment,
                2,
                HorizontalAlignment::Left,
                VerticalAlignment::Stretch,
            ),
            Some((1, 2))
        );
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Columns,
                GridResizeBehavior::BasedOnAlignment,
                2,
                HorizontalAlignment::Right,
                VerticalAlignment::Stretch,
            ),
            Some((2, 3))
        );
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Rows,
                GridResizeBehavior::BasedOnAlignment,
                2,
                HorizontalAlignment::Stretch,
                VerticalAlignment::Top,
            ),
            Some((1, 2))
        );
        assert_eq!(
            resolve_pair(
                GridResizeDirection::Rows,
                GridResizeBehavior::BasedOnAlignment,
                2,
                HorizontalAlignment::Stretch,
                VerticalAlignment::Bottom,
            ),
            Some((2, 3))
        );
    }

    #[test]
    fn invalid_increments_use_finite_defaults() {
        assert_eq!(effective_increment(0.0, 1.0), 1.0);
        assert_eq!(effective_increment(-1.0, 1.0), 1.0);
        assert_eq!(effective_increment(f32::NAN, 1.0), 1.0);
        assert_eq!(effective_increment(f32::INFINITY, 8.0), 8.0);
    }
}
