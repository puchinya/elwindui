//! Retained four-sided auto-hide strip and Document popup.

use crate::DockItemId;
use crate::DockingControl;
use crate::core::base::{Point, Size, Vector};
use crate::core::graphics::Color;
use crate::core::input::{Key, KeyEventArgs, MouseButton, PointerEventArgs};
use crate::core::layout::{GridLength, HorizontalAlignment, VerticalAlignment, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    ContentControlExt, ControlExt, Grid, GridExt, HorizontalLayout, HorizontalLayoutExt, LayoutExt,
    Rectangle, RectangleExt, ShapeExt, TextBlock, TextBlockExt, TextStyleOwner, UIElementExt,
    UnitPoint, VerticalLayout, VerticalLayoutExt, VisualTransform,
};
use crate::model::RootKind;
use crate::placement::DockSide;
use crate::runtime::metrics::{
    AUTO_HIDE_ENTRY_HEIGHT, AUTO_HIDE_ENTRY_SPACING, AUTO_HIDE_MARKER_SIZE,
    AUTO_HIDE_PANEL_HEADER_HEIGHT, AUTO_HIDE_RESIZE_GRIP_SIZE, AUTO_HIDE_STRIP_SIZE,
};
use crate::runtime::{accent_brush, dock_fill_brush, popup_base_brush, themed_brush};
use elwindui_custom_controls::{ChromeIcon, chrome_icon};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::f32::consts::FRAC_PI_2;
use std::rc::{Rc, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ExtentAxis {
    Horizontal,
    Vertical,
}

pub(crate) type AutoHideExtentCache = Rc<RefCell<BTreeMap<(DockItemId, ExtentAxis), f32>>>;

enum StripPanel {
    Horizontal(Rc<HorizontalLayout>),
    Vertical(Rc<VerticalLayout>),
}

impl StripPanel {
    fn element(&self) -> Rc<dyn UIElementExt> {
        match self {
            Self::Horizontal(panel) => panel.clone(),
            Self::Vertical(panel) => panel.clone(),
        }
    }

    fn clear(&self) {
        match self {
            Self::Horizontal(panel) => panel.children().clear(),
            Self::Vertical(panel) => panel.children().clear(),
        }
    }

    fn add(&self, entry: Rc<Grid>) {
        match self {
            Self::Horizontal(panel) => panel.children().add(entry),
            Self::Vertical(panel) => panel.children().add(entry),
        }
    }
}

#[derive(Clone)]
struct HeaderDragGesture {
    item: DockItemId,
    press_position: Point,
    press_screen_position: Option<Point>,
    last_position: Point,
    last_screen_position: Option<Point>,
    dragging: bool,
}

/// Keeps the unrotated text rectangle intact inside a narrow side tab. A normal Grid cell
/// would clamp the title to the rail width before its presentation rotation is applied.
#[elwindui::component(inherits Control)]
pub(crate) struct RotatedStripLabel {
    #[state(default = None)]
    text: Option<Rc<TextBlock>>,
    #[state(default = AUTO_HIDE_ENTRY_HEIGHT)]
    label_width: f32,
    template: template_view!(|_this: Self| {
        Grid {
            rows: [GridLength::Star(1.0)]
            columns: [GridLength::Star(1.0)]
        }
    }),
}

#[elwindui::component]
impl RotatedStripLabel {
    #[overrides]
    fn on_apply_template(&self) {
        if let Some(root) = self.__template_root()
            && let Some(grid) = root.as_any().downcast_ref::<Grid>()
            && let Some(text) = self.text()
        {
            grid.children().add(text);
        }
    }

    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        if let Some(text) = self.text() {
            text.measure(Size {
                width: self.label_width(),
                height: AUTO_HIDE_ENTRY_HEIGHT,
            });
        }
        Size {
            width: AUTO_HIDE_STRIP_SIZE,
            height: self.label_width(),
        }
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        if let Some(root) = self.__template_root() {
            root.arrange(crate::core::base::Rect {
                x: 0.0,
                y: 0.0,
                width: final_size.width,
                height: final_size.height,
            });
        }
        if let Some(text) = self.text() {
            text.arrange(crate::core::base::Rect {
                x: (final_size.width - self.label_width()) * 0.5,
                y: (final_size.height - AUTO_HIDE_ENTRY_HEIGHT) * 0.5,
                width: self.label_width(),
                height: AUTO_HIDE_ENTRY_HEIGHT,
            });
        }
        final_size
    }
}

impl RotatedStripLabel {
    fn new_label(text: Rc<TextBlock>, width: f32) -> Rc<Self> {
        let label = Self::new();
        label.set_text(Some(text));
        label.set_label_width(width);
        label
    }
}

/// Only one auto-hide Document owns the open popup on a surface. Pane extent memory is deliberately
/// runtime-only and keyed by Document plus resize axis; it never enters Snapshot V2.
pub(crate) struct AutoHideOverlay {
    open: Rc<RefCell<Option<DockItemId>>>,
    open_context: Rc<RefCell<Option<(DockItemId, DockSide)>>>,
    dismissed: Rc<RefCell<Option<DockItemId>>>,
    remembered_extents: AutoHideExtentCache,
    current_extent: Rc<Cell<f32>>,
    extent_limit: Rc<Cell<f32>>,
    resize_gesture: Rc<Cell<Option<(Point, f32, DockSide)>>>,
    header_gesture: Rc<RefCell<Option<HeaderDragGesture>>>,
    markers: Rc<RefCell<BTreeMap<DockItemId, (Rc<Rectangle>, Rc<Cell<bool>>, Rc<TextBlock>)>>>,
    visual: Rc<Grid>,
    strips: [Rc<Grid>; 4],
    panels: [StripPanel; 4],
    pane: Rc<Grid>,
    backplate: Rc<Rectangle>,
    /// Transparent, hit-testable layer under the open pane over the main area. Like the light
    /// dismiss of WinUI.Dock's popup, a press outside the pane only dismisses it and never
    /// reaches the content underneath.
    dismiss_layer: Rc<Grid>,
    frame: Rc<Rectangle>,
    header: Rc<Grid>,
    page_host: Rc<Grid>,
    title: Rc<TextBlock>,
    pin_button: Rc<Grid>,
    close_button: Rc<Grid>,
    resize_grip: Rc<Grid>,
    pin_available: Rc<Cell<bool>>,
    close_available: Rc<Cell<bool>>,
    root_context: Rc<RefCell<RootKind>>,
}

impl AutoHideOverlay {
    pub(crate) fn new() -> Self {
        Self::with_extent_cache(AutoHideExtentCache::default())
    }

    pub(crate) fn with_extent_cache(remembered_extents: AutoHideExtentCache) -> Self {
        let visual = Grid::new();
        visual.set_rows(vec![
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
        ]);
        visual.set_columns(vec![
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
        ]);

        let strips: [Rc<Grid>; 4] = std::array::from_fn(|index| {
            let strip = Grid::new();
            // WinUI.Dock's `Sidebar` has no background of its own.
            strip.set_background(Some(Color::TRANSPARENT.into()));
            strip.set_visibility(Visibility::Collapsed);
            strip.set_attached("DockSurface", "side", index as i32);
            match index {
                0 => {
                    strip.set_width(AUTO_HIDE_STRIP_SIZE);
                    strip.set_attached("Grid", "row", 1i32);
                    strip.set_attached("Grid", "column", 0i32);
                }
                1 => {
                    strip.set_height(AUTO_HIDE_STRIP_SIZE);
                    strip.set_attached("Grid", "row", 0i32);
                    strip.set_attached("Grid", "column", 1i32);
                }
                2 => {
                    strip.set_width(AUTO_HIDE_STRIP_SIZE);
                    strip.set_attached("Grid", "row", 1i32);
                    strip.set_attached("Grid", "column", 2i32);
                }
                _ => {
                    strip.set_height(AUTO_HIDE_STRIP_SIZE);
                    strip.set_attached("Grid", "row", 2i32);
                    strip.set_attached("Grid", "column", 1i32);
                }
            }
            visual.children().add(strip.clone());
            strip
        });

        let panels: [StripPanel; 4] = std::array::from_fn(|index| {
            let panel = if matches!(index, 0 | 2) {
                let panel = VerticalLayout::new();
                panel.set_spacing(AUTO_HIDE_ENTRY_SPACING);
                StripPanel::Vertical(panel)
            } else {
                let panel = HorizontalLayout::new();
                panel.set_spacing(AUTO_HIDE_ENTRY_SPACING);
                StripPanel::Horizontal(panel)
            };
            strips[index].children().add(panel.element());
            panel
        });

        let pane = Grid::new();
        pane.set_rows(vec![GridLength::Star(1.0)]);
        pane.set_columns(vec![GridLength::Star(1.0)]);
        let body = Grid::new();
        body.set_rows(vec![
            GridLength::Fixed(AUTO_HIDE_PANEL_HEADER_HEIGHT),
            GridLength::Star(1.0),
        ]);
        body.set_columns(vec![GridLength::Star(1.0)]);
        pane.children().add(body.clone());
        // The reference pane is an opaque base (`SolidBackgroundFillColorBase`, the backplate)
        // under the translucent dock fill (`DockFillDefaultBrush`) drawn here.
        pane.set_background(Some(dock_fill_brush()));
        pane.set_visibility(Visibility::Collapsed);
        pane.set_attached("Grid", "row", 1i32);
        pane.set_attached("Grid", "column", 1i32);

        let dismiss_layer = Grid::new();
        dismiss_layer.set_background(Some(Color::TRANSPARENT.into()));
        dismiss_layer.set_hit_test_visible(true);
        dismiss_layer.set_visibility(Visibility::Collapsed);
        dismiss_layer.set_attached("Grid", "row", 1i32);
        dismiss_layer.set_attached("Grid", "column", 1i32);
        visual.children().add(dismiss_layer.clone());

        let backplate = Rectangle::new();
        backplate.set_fill(Some(popup_base_brush()));
        backplate.set_corner_radius(4.0);
        backplate.set_hit_test_visible(false);
        backplate.set_visibility(Visibility::Collapsed);
        backplate.set_attached("Grid", "row", 1i32);
        backplate.set_attached("Grid", "column", 1i32);
        visual.children().add(backplate.clone());
        visual.children().add(pane.clone());

        let frame = Rectangle::new();
        frame.set_fill(None);
        frame.set_stroke(Some(accent_brush()));
        frame.set_stroke_width(1.0);
        frame.set_corner_radius(4.0);
        frame.set_hit_test_visible(false);
        frame.set_visibility(Visibility::Collapsed);
        frame.set_attached("Grid", "row", 1i32);
        frame.set_attached("Grid", "column", 1i32);
        visual.children().add(frame.clone());

        let header = Grid::new();
        header.set_height(AUTO_HIDE_PANEL_HEADER_HEIGHT);
        header.set_rows(vec![GridLength::Star(1.0)]);
        // Reference popup header: 4 px border padding plus an 8 px content margin on each side.
        header.set_columns(vec![
            GridLength::Fixed(12.0),
            GridLength::Star(1.0),
            GridLength::Auto,
            GridLength::Auto,
            GridLength::Fixed(12.0),
        ]);
        header.set_background(Some(dock_fill_brush()));
        header.set_attached("Grid", "row", 0i32);
        header.set_attached("Grid", "column", 0i32);
        body.children().add(header.clone());

        let title = TextBlock::new();
        title.set_foreground(themed_brush(BrushStyle::Foreground));
        // Same title treatment as the group content header and the reference popup header.
        title.set_font_weight(crate::core::graphics::FontWeight(600));
        title.set_font_size(14.0);
        title.set_vertical_alignment(crate::core::layout::VerticalAlignment::Center);
        title.set_attached("Grid", "row", 0i32);
        title.set_attached("Grid", "column", 1i32);
        header.children().add(title.clone());

        let pin_button = action_button(ChromeIcon::Pinned, 2);
        let close_button = action_button(ChromeIcon::Close, 3);
        close_button.set_visibility(Visibility::Collapsed);
        header.children().add(pin_button.clone());
        header.children().add(close_button.clone());

        let page_host = Grid::new();
        page_host.set_attached("Grid", "row", 1i32);
        page_host.set_attached("Grid", "column", 0i32);
        body.children().add(page_host.clone());

        let resize_grip = Grid::new();
        resize_grip.set_background(Some(Color::TRANSPARENT.into()));
        resize_grip.set_hit_test_visible(true);
        resize_grip.set_visibility(Visibility::Collapsed);
        pane.children().add(resize_grip.clone());

        let open = Rc::new(RefCell::new(None));
        let open_context = Rc::new(RefCell::new(None));
        let dismissed = Rc::new(RefCell::new(None));
        let current_extent = Rc::new(Cell::new(0.0));
        let extent_limit = Rc::new(Cell::new(0.0));
        let resize_gesture = Rc::new(Cell::new(None));
        let header_gesture = Rc::new(RefCell::new(None));
        let markers = Rc::new(RefCell::new(BTreeMap::new()));
        let pin_available = Rc::new(Cell::new(false));
        let close_available = Rc::new(Cell::new(false));
        let root_context = Rc::new(RefCell::new(RootKind::Main));

        bind_resize_handlers(
            &resize_grip,
            pane.clone(),
            backplate.clone(),
            frame.clone(),
            open_context.clone(),
            remembered_extents.clone(),
            current_extent.clone(),
            extent_limit.clone(),
            resize_gesture.clone(),
        );
        Self {
            open,
            open_context,
            dismissed,
            remembered_extents,
            current_extent,
            extent_limit,
            resize_gesture,
            header_gesture,
            markers,
            visual,
            strips,
            panels,
            pane,
            backplate,
            dismiss_layer,
            frame,
            header,
            page_host,
            title,
            pin_button,
            close_button,
            resize_grip,
            pin_available,
            close_available,
            root_context,
        }
    }

    /// Opens a popup in response to a new user activation, clearing any prior light-dismiss state.
    pub(crate) fn open(
        &mut self,
        item: DockItemId,
        side: DockSide,
        surface_size: Size,
    ) -> Option<DockItemId> {
        self.open_with_policy(item, side, surface_size, true)
    }

    /// Replays the model's open flag unless the same Document was light-dismissed during this
    /// runtime lifetime. This prevents unrelated model updates from immediately reopening it.
    pub(crate) fn open_from_model(
        &mut self,
        item: DockItemId,
        side: DockSide,
        surface_size: Size,
    ) -> Option<DockItemId> {
        self.open_with_policy(item, side, surface_size, false)
    }

    fn open_with_policy(
        &mut self,
        item: DockItemId,
        side: DockSide,
        surface_size: Size,
        explicit: bool,
    ) -> Option<DockItemId> {
        if !explicit && self.dismissed.borrow().as_ref() == Some(&item) {
            return None;
        }
        if explicit {
            *self.dismissed.borrow_mut() = None;
        }
        if self
            .open_context
            .borrow()
            .as_ref()
            .is_some_and(|(open, open_side)| open != &item || *open_side != side)
        {
            remember_pane_extent(
                &self.open_context,
                &self.remembered_extents,
                &self.current_extent,
            );
        }
        self.configure_pane(&item, side, surface_size);
        let previous = self.open.replace(Some(item.clone()));
        *self.open_context.borrow_mut() = Some((item.clone(), side));
        self.pane.set_visibility(Visibility::Visible);
        self.backplate.set_visibility(Visibility::Visible);
        self.dismiss_layer.set_visibility(Visibility::Visible);
        self.frame.set_visibility(Visibility::Visible);
        self.resize_grip.set_visibility(Visibility::Visible);
        previous
    }

    /// Layout extent each visible strip reserves, in `DockSide::ALL` order (Left, Top, Right,
    /// Bottom). Main content is arranged inside these insets so strips never cover it.
    pub(crate) fn strip_extents(&self) -> [f32; 4] {
        std::array::from_fn(|index| {
            if self.strips[index].visibility() == Visibility::Visible {
                AUTO_HIDE_STRIP_SIZE
            } else {
                0.0
            }
        })
    }

    fn configure_pane(&self, item: &DockItemId, side: DockSide, surface_size: Size) {
        let width = finite_extent(surface_size.width);
        let height = finite_extent(surface_size.height);
        let left_visible = self.strips[DockSide::Left.index()].visibility() == Visibility::Visible;
        let right_visible =
            self.strips[DockSide::Right.index()].visibility() == Visibility::Visible;
        let top_visible = self.strips[DockSide::Top.index()].visibility() == Visibility::Visible;
        let bottom_visible =
            self.strips[DockSide::Bottom.index()].visibility() == Visibility::Visible;
        let center_width = (width
            - if left_visible {
                AUTO_HIDE_STRIP_SIZE
            } else {
                0.0
            }
            - if right_visible {
                AUTO_HIDE_STRIP_SIZE
            } else {
                0.0
            })
        .max(1.0);
        let center_height = (height
            - if top_visible {
                AUTO_HIDE_STRIP_SIZE
            } else {
                0.0
            }
            - if bottom_visible {
                AUTO_HIDE_STRIP_SIZE
            } else {
                0.0
            })
        .max(1.0);
        let axis = axis_for_side(side);
        let available_extent = match axis {
            ExtentAxis::Horizontal => center_width,
            ExtentAxis::Vertical => center_height,
        };
        let limit = (available_extent - 160.0).max(1.0);
        let default_extent = (available_extent / 3.0).clamp(1.0, limit);
        let extent = self
            .remembered_extents
            .borrow()
            .get(&(item.clone(), axis))
            .copied()
            .unwrap_or(default_extent)
            .clamp(1.0, limit);
        self.current_extent.set(extent);
        self.extent_limit.set(limit);
        let pane_width = match axis {
            ExtentAxis::Horizontal => extent,
            ExtentAxis::Vertical => center_width,
        };
        let pane_height = match axis {
            ExtentAxis::Horizontal => center_height,
            ExtentAxis::Vertical => extent,
        };
        self.pane.set_min_width(pane_width);
        self.pane.set_max_width(pane_width);
        self.pane.set_min_height(pane_height);
        self.pane.set_max_height(pane_height);
        self.pane.set_horizontal_alignment(match side {
            DockSide::Left => HorizontalAlignment::Left,
            DockSide::Right => HorizontalAlignment::Right,
            DockSide::Top | DockSide::Bottom => HorizontalAlignment::Stretch,
        });
        self.pane.set_vertical_alignment(match side {
            DockSide::Top => VerticalAlignment::Top,
            DockSide::Bottom => VerticalAlignment::Bottom,
            DockSide::Left | DockSide::Right => VerticalAlignment::Stretch,
        });
        self.frame.set_min_width(pane_width);
        self.frame.set_max_width(pane_width);
        self.backplate.set_min_width(pane_width);
        self.backplate.set_max_width(pane_width);
        self.frame.set_min_height(pane_height);
        self.frame.set_max_height(pane_height);
        self.backplate.set_min_height(pane_height);
        self.backplate.set_max_height(pane_height);
        self.frame.set_horizontal_alignment(match side {
            DockSide::Left => HorizontalAlignment::Left,
            DockSide::Right => HorizontalAlignment::Right,
            DockSide::Top | DockSide::Bottom => HorizontalAlignment::Stretch,
        });
        self.frame.set_vertical_alignment(match side {
            DockSide::Top => VerticalAlignment::Top,
            DockSide::Bottom => VerticalAlignment::Bottom,
            DockSide::Left | DockSide::Right => VerticalAlignment::Stretch,
        });
        self.backplate.set_horizontal_alignment(match side {
            DockSide::Left => HorizontalAlignment::Left,
            DockSide::Right => HorizontalAlignment::Right,
            DockSide::Top | DockSide::Bottom => HorizontalAlignment::Stretch,
        });
        self.backplate.set_vertical_alignment(match side {
            DockSide::Top => VerticalAlignment::Top,
            DockSide::Bottom => VerticalAlignment::Bottom,
            DockSide::Left | DockSide::Right => VerticalAlignment::Stretch,
        });
        self.configure_resize_grip(side, center_width, center_height);
    }

    fn configure_resize_grip(&self, side: DockSide, width: f32, height: f32) {
        self.resize_grip.set_attached("Grid", "row", 0i32);
        self.resize_grip.set_attached("Grid", "column", 0i32);
        match side {
            DockSide::Left => {
                self.resize_grip.set_width(AUTO_HIDE_RESIZE_GRIP_SIZE);
                self.resize_grip.set_height(height);
                self.resize_grip
                    .set_horizontal_alignment(HorizontalAlignment::Right);
                self.resize_grip
                    .set_vertical_alignment(VerticalAlignment::Stretch);
            }
            DockSide::Right => {
                self.resize_grip.set_width(AUTO_HIDE_RESIZE_GRIP_SIZE);
                self.resize_grip.set_height(height);
                self.resize_grip
                    .set_horizontal_alignment(HorizontalAlignment::Left);
                self.resize_grip
                    .set_vertical_alignment(VerticalAlignment::Stretch);
            }
            DockSide::Top => {
                self.resize_grip.set_width(width);
                self.resize_grip.set_height(AUTO_HIDE_RESIZE_GRIP_SIZE);
                self.resize_grip
                    .set_horizontal_alignment(HorizontalAlignment::Stretch);
                self.resize_grip
                    .set_vertical_alignment(VerticalAlignment::Bottom);
            }
            DockSide::Bottom => {
                self.resize_grip.set_width(width);
                self.resize_grip.set_height(AUTO_HIDE_RESIZE_GRIP_SIZE);
                self.resize_grip
                    .set_horizontal_alignment(HorizontalAlignment::Stretch);
                self.resize_grip
                    .set_vertical_alignment(VerticalAlignment::Top);
            }
        }
    }

    pub(crate) fn close(&mut self) -> Option<DockItemId> {
        remember_pane_extent(
            &self.open_context,
            &self.remembered_extents,
            &self.current_extent,
        );
        let previous = self.open.replace(None);
        *self.open_context.borrow_mut() = None;
        self.pane.set_visibility(Visibility::Collapsed);
        self.backplate.set_visibility(Visibility::Collapsed);
        self.dismiss_layer.set_visibility(Visibility::Collapsed);
        self.frame.set_visibility(Visibility::Collapsed);
        self.pin_button.set_visibility(Visibility::Collapsed);
        self.close_button.set_visibility(Visibility::Collapsed);
        self.resize_grip.set_visibility(Visibility::Collapsed);
        self.page_host.children().clear();
        self.title.set_text("");
        self.resize_gesture.set(None);
        self.header_gesture.borrow_mut().take();
        previous
    }

    /// Closes only the current popup presentation while retaining the active header gesture so
    /// its implicit pointer capture can finish the selected Document drag.
    pub(crate) fn dismiss_for_drag(&self, item: &DockItemId) {
        if self.open.borrow().as_ref() != Some(item) {
            return;
        }
        remember_pane_extent(
            &self.open_context,
            &self.remembered_extents,
            &self.current_extent,
        );
        *self.dismissed.borrow_mut() = Some(item.clone());
        self.open.borrow_mut().take();
        *self.open_context.borrow_mut() = None;
        self.pane.set_visibility(Visibility::Collapsed);
        self.backplate.set_visibility(Visibility::Collapsed);
        self.dismiss_layer.set_visibility(Visibility::Collapsed);
        self.frame.set_visibility(Visibility::Collapsed);
        self.pin_button.set_visibility(Visibility::Collapsed);
        self.close_button.set_visibility(Visibility::Collapsed);
        self.resize_grip.set_visibility(Visibility::Collapsed);
        self.page_host.children().clear();
        self.resize_gesture.set(None);
    }

    pub(crate) fn current(&self) -> Option<DockItemId> {
        self.open.borrow().clone()
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.visual.clone()
    }

    pub(crate) fn drag_source_element(&self) -> Rc<dyn UIElementExt> {
        self.pane.clone()
    }

    #[cfg(test)]
    pub(crate) fn pane_for_test(&self) -> Rc<Grid> {
        self.pane.clone()
    }

    #[cfg(test)]
    pub(crate) fn page_host_for_test(&self) -> Rc<Grid> {
        self.page_host.clone()
    }

    #[cfg(test)]
    pub(crate) fn pin_button_for_test(&self) -> Rc<Grid> {
        self.pin_button.clone()
    }

    #[cfg(test)]
    pub(crate) fn resize_grip_for_test(&self) -> Rc<Grid> {
        self.resize_grip.clone()
    }

    #[cfg(test)]
    pub(crate) fn marker_count_for_test(&self) -> usize {
        self.markers.borrow().len()
    }

    #[cfg(test)]
    pub(crate) fn marker_fill_for_test(
        &self,
        item: &DockItemId,
    ) -> Option<crate::core::graphics::Brush> {
        self.markers.borrow().get(item)?.0.fill()
    }

    #[cfg(test)]
    pub(crate) fn marker_size_for_test(&self, item: &DockItemId) -> Option<Size> {
        let marker = self.markers.borrow().get(item)?.0.clone();
        Some(Size {
            width: marker.width()?,
            height: marker.height()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn remembered_extent_for_test(
        &self,
        item: &DockItemId,
        side: DockSide,
    ) -> Option<f32> {
        self.remembered_extents
            .borrow()
            .get(&(item.clone(), axis_for_side(side)))
            .copied()
    }

    pub(crate) fn render_strips(
        &self,
        titles: impl Iterator<
            Item = (
                usize,
                DockItemId,
                String,
                Option<crate::core::graphics::IconSource>,
            ),
        >,
        owner: &Weak<DockingControl>,
        root: RootKind,
    ) {
        for (index, panel) in self.panels.iter().enumerate() {
            panel.clear();
            self.strips[index].set_visibility(Visibility::Collapsed);
        }
        self.markers.borrow_mut().clear();
        for (side, item, title, _icon) in titles {
            let Some(strip) = self.strips.get(side) else {
                continue;
            };
            let Some(panel) = self.panels.get(side) else {
                continue;
            };
            strip.set_visibility(Visibility::Visible);
            let side = DockSide::ALL[side];
            let (entry, marker, hovered, label) = self.make_strip_entry(&title, side);
            panel.add(entry.clone());
            self.markers.borrow_mut().insert(
                item.clone(),
                (marker.clone(), hovered.clone(), label.clone()),
            );

            let weak_owner = owner.clone();
            let entry_root = root.clone();
            let entry_item = item.clone();
            entry.register_routed_handler::<PointerEventArgs>(
                "on_pointer_pressed",
                Box::new(move |event, args| {
                    if event.button == Some(MouseButton::Left) {
                        args.handled.set(true);
                    }
                }),
            );
            // WinUI.Dock's strip button: the mark and title turn to the accent color only while
            // the pointer is over (or pressing) the entry; otherwise the mark stays gray.
            let marker_enter = marker.clone();
            let label_enter = label.clone();
            let hovered_enter = hovered.clone();
            entry.register_routed_handler::<PointerEventArgs>(
                "on_pointer_entered",
                Box::new(move |_, args| {
                    if !args.handled.get() {
                        hovered_enter.set(true);
                        marker_enter.set_fill(Some(accent_brush()));
                        label_enter.set_foreground(Some(accent_brush()));
                    }
                }),
            );
            let marker_exit = marker.clone();
            let label_exit = label.clone();
            let hovered_exit = hovered.clone();
            entry.register_routed_handler::<PointerEventArgs>(
                "on_pointer_exited",
                Box::new(move |_, args| {
                    if args.handled.get() {
                        return;
                    }
                    hovered_exit.set(false);
                    marker_exit.set_fill(themed_brush(BrushStyle::Separator));
                    label_exit.set_foreground(themed_brush(BrushStyle::Foreground));
                }),
            );
            entry.register_routed_handler::<PointerEventArgs>(
                "on_pointer_released",
                Box::new(move |_, args| {
                    args.handled.set(true);
                    let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
                    if let Some(owner) = owner {
                        owner.handle_auto_hide_open(entry_root.clone(), entry_item.clone());
                    }
                }),
            );
        }
    }

    fn make_strip_entry(
        &self,
        label: &str,
        side: DockSide,
    ) -> (Rc<Grid>, Rc<Rectangle>, Rc<Cell<bool>>, Rc<TextBlock>) {
        let text = TextBlock::new();
        text.set_text(label);
        text.set_foreground(themed_brush(BrushStyle::Foreground));
        text.measure(Size {
            width: f32::INFINITY,
            height: AUTO_HIDE_ENTRY_HEIGHT,
        });
        let natural = text.measured_size().unwrap_or_default();
        let text_width = natural.width.max(AUTO_HIDE_ENTRY_HEIGHT);
        text.set_width(text_width);
        text.set_height(AUTO_HIDE_ENTRY_HEIGHT);
        text.set_horizontal_alignment(HorizontalAlignment::Center);
        text.set_vertical_alignment(VerticalAlignment::Center);
        let entry = Grid::new();
        entry.set_background(Some(Color::TRANSPARENT.into()));
        entry.set_hit_test_visible(true);
        if matches!(side, DockSide::Left | DockSide::Right) {
            entry.set_width(AUTO_HIDE_STRIP_SIZE);
            entry.set_height(text_width);
            text.set_visual_transform(VisualTransform::new(
                Vector { x: 0.0, y: 0.0 },
                1.0,
                // Both side rails read top to bottom, like WinUI.Dock's vertical strips.
                FRAC_PI_2,
            ));
            text.set_transform_origin(UnitPoint::CENTER);
        } else {
            entry.set_width(text_width);
            entry.set_height(AUTO_HIDE_STRIP_SIZE);
        }
        let label = text.clone();
        if matches!(side, DockSide::Left | DockSide::Right) {
            entry
                .children()
                .add(RotatedStripLabel::new_label(text, text_width));
        } else {
            entry.children().add(text);
        }
        // The mark sits on the strip's outer edge and is always shown, like the reference.
        let marker = Rectangle::new();
        marker.set_fill(themed_brush(BrushStyle::Separator));
        marker.set_hit_test_visible(false);
        match side {
            DockSide::Left => {
                marker.set_width(AUTO_HIDE_MARKER_SIZE);
                marker.set_height(text_width);
                marker.set_horizontal_alignment(HorizontalAlignment::Left);
                marker.set_vertical_alignment(VerticalAlignment::Top);
            }
            DockSide::Right => {
                marker.set_width(AUTO_HIDE_MARKER_SIZE);
                marker.set_height(text_width);
                marker.set_horizontal_alignment(HorizontalAlignment::Right);
                marker.set_vertical_alignment(VerticalAlignment::Top);
            }
            DockSide::Top => {
                marker.set_width(text_width);
                marker.set_height(AUTO_HIDE_MARKER_SIZE);
                marker.set_horizontal_alignment(HorizontalAlignment::Center);
                marker.set_vertical_alignment(VerticalAlignment::Top);
            }
            DockSide::Bottom => {
                marker.set_width(text_width);
                marker.set_height(AUTO_HIDE_MARKER_SIZE);
                marker.set_horizontal_alignment(HorizontalAlignment::Center);
                marker.set_vertical_alignment(VerticalAlignment::Bottom);
            }
        }
        entry.children().add(marker.clone());
        (entry, marker, Rc::new(Cell::new(false)), label)
    }

    pub(crate) fn present_open_item(
        &self,
        wrapper: Option<Rc<elwindui_custom_controls::CustomTabViewItem>>,
        title: &str,
        can_pin: bool,
        can_close: bool,
    ) {
        self.page_host.children().clear();
        if let Some(wrapper) = wrapper {
            // Auto-hide items are not represented by a tab presenter while hidden. Adopt only the
            // retained page content; the popup's 40 px header remains a Document-level operation
            // area and routes drag gestures through the same tab threshold/coordinator.
            wrapper.__prepare_template_presentation();
            if let Some(content) = wrapper.__content_opt() {
                self.page_host.children().add(content);
            }
        }
        self.title.set_text(title);
        self.pin_available.set(can_pin);
        self.close_available.set(can_close);
        self.pin_button.set_visibility(if can_pin {
            Visibility::Visible
        } else {
            Visibility::Collapsed
        });
        self.close_button.set_visibility(if can_close {
            Visibility::Visible
        } else {
            Visibility::Collapsed
        });
        self.show_pane();
    }

    pub(crate) fn bind_handlers(&self, owner: &Weak<DockingControl>, root: RootKind) {
        *self.root_context.borrow_mut() = root.clone();
        bind_document_header_drag(
            &self.header,
            &self.header_gesture,
            &self.open_context,
            owner,
        );
        let weak_owner = owner.clone();
        let root_context = self.root_context.clone();
        self.pin_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                if event.button != Some(MouseButton::Left) {
                    return;
                }
                args.handled.set(true);
                let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_pin_gesture(root_context.borrow().clone());
                }
            }),
        );
        let weak_owner = owner.clone();
        let open_context = self.open_context.clone();
        self.close_button
            .register_routed_handler::<PointerEventArgs>(
                "on_pointer_released",
                Box::new(move |event, args| {
                    if event.button != Some(MouseButton::Left) {
                        return;
                    }
                    args.handled.set(true);
                    let Some(item) = open_context.borrow().as_ref().map(|(item, _)| item.clone())
                    else {
                        return;
                    };
                    let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
                    if let Some(owner) = owner {
                        owner.handle_auto_hide_close(item);
                    }
                }),
            );
    }

    pub(crate) fn bind_light_dismiss(&self, surface_root: &Rc<Grid>, owner: &Weak<DockingControl>) {
        let weak_owner = owner.clone();
        let open = self.open.clone();
        let dismissed = self.dismissed.clone();
        let open_context = self.open_context.clone();
        let pane = self.pane.clone();
        let backplate = self.backplate.clone();
        let dismiss_layer = self.dismiss_layer.clone();
        let frame = self.frame.clone();
        let page_host = self.page_host.clone();
        let pin_button = self.pin_button.clone();
        let close_button = self.close_button.clone();
        let resize_grip = self.resize_grip.clone();
        let markers = self.markers.clone();
        let resize_gesture = self.resize_gesture.clone();
        let header_gesture = self.header_gesture.clone();
        let remembered = self.remembered_extents.clone();
        let current = self.current_extent.clone();
        surface_root.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, args| {
                if args.handled.get()
                    || open.borrow().is_none()
                    || element_contains(pane.as_ref(), event.position)
                {
                    return;
                }
                remember_pane_extent(&open_context, &remembered, &current);
                let dismissed_item = dismiss_shared(
                    &open,
                    &dismissed,
                    &open_context,
                    &pane,
                    &backplate,
                    &dismiss_layer,
                    &frame,
                    &page_host,
                    &pin_button,
                    &close_button,
                    &resize_grip,
                    &markers,
                    &resize_gesture,
                    &header_gesture,
                );
                notify_dismissed(&weak_owner, dismissed_item);
            }),
        );

        let weak_owner = owner.clone();
        let open = self.open.clone();
        let dismissed = self.dismissed.clone();
        let open_context = self.open_context.clone();
        let pane = self.pane.clone();
        let backplate = self.backplate.clone();
        let dismiss_layer = self.dismiss_layer.clone();
        let frame = self.frame.clone();
        let page_host = self.page_host.clone();
        let pin_button = self.pin_button.clone();
        let close_button = self.close_button.clone();
        let resize_grip = self.resize_grip.clone();
        let markers = self.markers.clone();
        let resize_gesture = self.resize_gesture.clone();
        let header_gesture = self.header_gesture.clone();
        let remembered = self.remembered_extents.clone();
        let current = self.current_extent.clone();
        surface_root.register_routed_handler::<KeyEventArgs>(
            "on_key_down",
            Box::new(move |event, args| {
                if args.handled.get() || event.key != Key::Escape || open.borrow().is_none() {
                    return;
                }
                remember_pane_extent(&open_context, &remembered, &current);
                let dismissed_item = dismiss_shared(
                    &open,
                    &dismissed,
                    &open_context,
                    &pane,
                    &backplate,
                    &dismiss_layer,
                    &frame,
                    &page_host,
                    &pin_button,
                    &close_button,
                    &resize_grip,
                    &markers,
                    &resize_gesture,
                    &header_gesture,
                );
                notify_dismissed(&weak_owner, dismissed_item);
                args.handled.set(true);
            }),
        );
    }

    pub(crate) fn set_root(&self, root: RootKind) {
        *self.root_context.borrow_mut() = root;
    }

    pub(crate) fn show_pane(&self) {
        self.pane.set_visibility(Visibility::Visible);
        self.backplate.set_visibility(Visibility::Visible);
        self.dismiss_layer.set_visibility(Visibility::Visible);
        self.frame.set_visibility(Visibility::Visible);
        self.resize_grip.set_visibility(Visibility::Visible);
        self.pin_button.set_visibility(if self.pin_available.get() {
            Visibility::Visible
        } else {
            Visibility::Collapsed
        });
        self.close_button
            .set_visibility(if self.close_available.get() {
                Visibility::Visible
            } else {
                Visibility::Collapsed
            });
    }

    pub(crate) fn refresh_theme(&self) {
        self.backplate.set_fill(Some(popup_base_brush()));
        self.pane.set_background(Some(dock_fill_brush()));
        self.frame.set_stroke(Some(accent_brush()));
        self.header.set_background(Some(dock_fill_brush()));
        self.title
            .set_foreground(themed_brush(BrushStyle::Foreground));
        self.pin_button.children().clear();
        self.pin_button.children().add(chrome_icon(
            ChromeIcon::Pinned,
            themed_brush(BrushStyle::Foreground),
        ));
        self.close_button.children().clear();
        self.close_button.children().add(chrome_icon(
            ChromeIcon::Close,
            themed_brush(BrushStyle::Foreground),
        ));
        for (marker, hovered, label) in self.markers.borrow().values() {
            marker.set_fill(if hovered.get() {
                Some(accent_brush())
            } else {
                themed_brush(BrushStyle::Separator)
            });
            label.set_foreground(if hovered.get() {
                Some(accent_brush())
            } else {
                themed_brush(BrushStyle::Foreground)
            });
        }
    }
}

impl Drop for AutoHideOverlay {
    fn drop(&mut self) {
        // A floating surface may disappear while its Document moves to another surface.
        remember_pane_extent(
            &self.open_context,
            &self.remembered_extents,
            &self.current_extent,
        );
    }
}

fn action_button(icon: ChromeIcon, column: i32) -> Rc<Grid> {
    let button = Grid::new();
    button.set_background(Some(Color::TRANSPARENT.into()));
    // Reference pane buttons are 28x28 (padding 8 around a 12-pixel glyph); star tracks center
    // the 16-pixel glyph box inside.
    button.set_min_width(28.0);
    button.set_max_width(28.0);
    button.set_min_height(28.0);
    button.set_max_height(28.0);
    button.set_rows(vec![GridLength::Star(1.0)]);
    button.set_columns(vec![GridLength::Star(1.0)]);
    button.set_horizontal_alignment(HorizontalAlignment::Center);
    button.set_vertical_alignment(VerticalAlignment::Center);
    button.set_attached("Grid", "row", 0i32);
    button.set_attached("Grid", "column", column);
    button
        .children()
        .add(chrome_icon(icon, themed_brush(BrushStyle::Foreground)));
    button.register_routed_handler::<PointerEventArgs>(
        "on_pointer_pressed",
        Box::new(|event, args| {
            if event.button == Some(MouseButton::Left) {
                args.handled.set(true);
            }
        }),
    );
    button
}

fn bind_resize_handlers(
    grip: &Rc<Grid>,
    pane: Rc<Grid>,
    backplate: Rc<Rectangle>,
    frame: Rc<Rectangle>,
    context: Rc<RefCell<Option<(DockItemId, DockSide)>>>,
    remembered: Rc<RefCell<BTreeMap<(DockItemId, ExtentAxis), f32>>>,
    current: Rc<Cell<f32>>,
    limit: Rc<Cell<f32>>,
    gesture: Rc<Cell<Option<(Point, f32, DockSide)>>>,
) {
    let context_press = context.clone();
    let gesture_press = gesture.clone();
    let current_press = current.clone();
    grip.register_routed_handler::<PointerEventArgs>(
        "on_pointer_pressed",
        Box::new(move |event, args| {
            if args.handled.get() || event.button != Some(MouseButton::Left) {
                return;
            }
            let Some(side) = context_press.borrow().as_ref().map(|(_, side)| *side) else {
                return;
            };
            gesture_press.set(Some((event.position, current_press.get(), side)));
            args.handled.set(true);
        }),
    );

    let gesture_move = gesture.clone();
    let current_move = current.clone();
    let limit_move = limit.clone();
    // The grip belongs to the pane. Holding its parent strongly from a grip callback
    // would keep the entire popup and its remembered extents alive after teardown.
    let pane_move = Rc::downgrade(&pane);
    let backplate_move = backplate.clone();
    let frame_move = frame.clone();
    grip.register_routed_handler::<PointerEventArgs>(
        "on_pointer_moved",
        Box::new(move |event, args| {
            if args.handled.get() {
                return;
            }
            let Some((start, initial, side)) = gesture_move.get() else {
                return;
            };
            let delta = match side {
                DockSide::Left => event.position.x - start.x,
                DockSide::Right => start.x - event.position.x,
                DockSide::Top => event.position.y - start.y,
                DockSide::Bottom => start.y - event.position.y,
            };
            let extent = (initial + delta).clamp(1.0, limit_move.get().max(1.0));
            current_move.set(extent);
            if let Some(pane) = pane_move.upgrade() {
                set_pane_extent(&pane, &backplate_move, &frame_move, side, extent);
            }
            args.handled.set(true);
        }),
    );

    let gesture_release = gesture.clone();
    let context_release = context.clone();
    let remembered_release = remembered.clone();
    let current_release = current.clone();
    grip.register_routed_handler::<PointerEventArgs>(
        "on_pointer_released",
        Box::new(move |_, args| {
            if let Some((_, _, side)) = gesture_release.take() {
                if let Some((item, _)) = context_release.borrow().as_ref() {
                    remembered_release
                        .borrow_mut()
                        .insert((item.clone(), axis_for_side(side)), current_release.get());
                }
                args.handled.set(true);
            }
        }),
    );

    let gesture_cancel = gesture.clone();
    let current_cancel = current;
    let pane_cancel = Rc::downgrade(&pane);
    let backplate_cancel = backplate;
    let frame_cancel = frame;
    grip.register_routed_handler::<PointerEventArgs>(
        "on_pointer_canceled",
        Box::new(move |_, args| {
            if let Some((_, initial, side)) = gesture_cancel.take() {
                current_cancel.set(initial);
                if let Some(pane) = pane_cancel.upgrade() {
                    set_pane_extent(&pane, &backplate_cancel, &frame_cancel, side, initial);
                }
                args.handled.set(true);
            }
        }),
    );
}

fn set_pane_extent(
    pane: &Grid,
    backplate: &Rectangle,
    frame: &Rectangle,
    side: DockSide,
    extent: f32,
) {
    match axis_for_side(side) {
        ExtentAxis::Horizontal => {
            pane.set_min_width(extent);
            pane.set_max_width(extent);
            backplate.set_min_width(extent);
            backplate.set_max_width(extent);
            frame.set_min_width(extent);
            frame.set_max_width(extent);
        }
        ExtentAxis::Vertical => {
            pane.set_min_height(extent);
            pane.set_max_height(extent);
            backplate.set_min_height(extent);
            backplate.set_max_height(extent);
            frame.set_min_height(extent);
            frame.set_max_height(extent);
        }
    }
}

fn bind_document_header_drag(
    header: &Rc<Grid>,
    state: &Rc<RefCell<Option<HeaderDragGesture>>>,
    context: &Rc<RefCell<Option<(DockItemId, DockSide)>>>,
    owner: &Weak<DockingControl>,
) {
    let state_press = state.clone();
    let open_context = context.clone();
    header.register_routed_handler::<PointerEventArgs>(
        "on_pointer_pressed",
        Box::new(move |event, args| {
            if args.handled.get() || event.button != Some(MouseButton::Left) {
                return;
            }
            if let Some((item, _)) = open_context.borrow().as_ref() {
                *state_press.borrow_mut() = Some(HeaderDragGesture {
                    item: item.clone(),
                    press_position: event.position,
                    press_screen_position: event.screen_position,
                    last_position: event.position,
                    last_screen_position: event.screen_position,
                    dragging: false,
                });
            }
        }),
    );
    let state_move = state.clone();
    let weak_owner = owner.clone();
    header.register_routed_handler::<PointerEventArgs>(
        "on_pointer_moved",
        Box::new(move |event, args| {
            if args.handled.get() {
                return;
            }
            let Some(mut gesture) = state_move.borrow().clone() else {
                return;
            };
            gesture.last_position = event.position;
            gesture.last_screen_position = event.screen_position;
            let dx = event.position.x - gesture.press_position.x;
            let dy = event.position.y - gesture.press_position.y;
            if !gesture.dragging && dx.mul_add(dx, dy * dy).sqrt() < 4.0 {
                state_move.replace(Some(gesture));
                return;
            }
            let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
            let Some(owner) = owner else {
                state_move.replace(None);
                return;
            };
            if !gesture.dragging {
                gesture.dragging = true;
                owner.handle_auto_hide_drag_started(
                    gesture.item.clone(),
                    PointerEventArgs {
                        position: gesture.press_position,
                        screen_position: gesture.press_screen_position,
                        button: Some(MouseButton::Left),
                        modifiers: event.modifiers,
                    },
                );
            }
            state_move.replace(Some(gesture.clone()));
            owner.handle_auto_hide_drag_moved(gesture.item, *event);
            args.handled.set(true);
        }),
    );
    let state_release = state.clone();
    let weak_owner = owner.clone();
    header.register_routed_handler::<PointerEventArgs>(
        "on_pointer_released",
        Box::new(move |event, args| {
            let Some(gesture) = state_release.borrow_mut().take() else {
                return;
            };
            if gesture.dragging {
                let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_auto_hide_drag_completed(gesture.item, *event, false);
                }
                args.handled.set(true);
            }
        }),
    );
    let state_cancel = state.clone();
    let weak_owner = owner.clone();
    header.register_routed_handler::<PointerEventArgs>(
        "on_pointer_canceled",
        Box::new(move |event, args| {
            let Some(gesture) = state_cancel.borrow_mut().take() else {
                return;
            };
            if gesture.dragging {
                let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_auto_hide_drag_completed(gesture.item, *event, true);
                }
                args.handled.set(true);
            }
        }),
    );
}

fn remember_pane_extent(
    context: &RefCell<Option<(DockItemId, DockSide)>>,
    remembered: &RefCell<BTreeMap<(DockItemId, ExtentAxis), f32>>,
    current: &Cell<f32>,
) {
    if let Some((item, side)) = context.borrow().as_ref() {
        remembered
            .borrow_mut()
            .insert((item.clone(), axis_for_side(*side)), current.get());
    }
}

fn dismiss_shared(
    open: &Rc<RefCell<Option<DockItemId>>>,
    dismissed: &Rc<RefCell<Option<DockItemId>>>,
    open_context: &Rc<RefCell<Option<(DockItemId, DockSide)>>>,
    pane: &Rc<Grid>,
    backplate: &Rc<Rectangle>,
    dismiss_layer: &Rc<Grid>,
    frame: &Rc<Rectangle>,
    page_host: &Rc<Grid>,
    pin_button: &Rc<Grid>,
    close_button: &Rc<Grid>,
    resize_grip: &Rc<Grid>,
    markers: &Rc<RefCell<BTreeMap<DockItemId, (Rc<Rectangle>, Rc<Cell<bool>>, Rc<TextBlock>)>>>,
    resize_gesture: &Rc<Cell<Option<(Point, f32, DockSide)>>>,
    header_gesture: &Rc<RefCell<Option<HeaderDragGesture>>>,
) -> Option<DockItemId> {
    let previous = open.borrow_mut().take();
    if let Some(item) = &previous {
        *dismissed.borrow_mut() = Some(item.clone());
    }
    *open_context.borrow_mut() = None;
    pane.set_visibility(Visibility::Collapsed);
    backplate.set_visibility(Visibility::Collapsed);
    dismiss_layer.set_visibility(Visibility::Collapsed);
    frame.set_visibility(Visibility::Collapsed);
    pin_button.set_visibility(Visibility::Collapsed);
    close_button.set_visibility(Visibility::Collapsed);
    resize_grip.set_visibility(Visibility::Collapsed);
    page_host.children().clear();
    for (marker, hovered, label) in markers.borrow().values() {
        hovered.set(false);
        marker.set_fill(themed_brush(BrushStyle::Separator));
        label.set_foreground(themed_brush(BrushStyle::Foreground));
    }
    resize_gesture.set(None);
    header_gesture.borrow_mut().take();
    previous
}

/// A light dismissal releases the dismissed item's active state through the model
/// (`docking_spec.md`), committed from the routed handler like the pane's pin/close actions.
fn notify_dismissed(owner: &Weak<DockingControl>, item: Option<DockItemId>) {
    let owner: Option<Rc<DockingControl>> = owner.upgrade();
    let (Some(owner), Some(item)) = (owner, item) else {
        return;
    };
    owner.handle_auto_hide_dismissed(item);
}

fn axis_for_side(side: DockSide) -> ExtentAxis {
    match side {
        DockSide::Left | DockSide::Right => ExtentAxis::Horizontal,
        DockSide::Top | DockSide::Bottom => ExtentAxis::Vertical,
    }
}

fn finite_extent(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

fn element_contains(element: &dyn UIElementExt, point: Point) -> bool {
    let mut offset = element
        .arranged_offset()
        .unwrap_or(Point { x: 0.0, y: 0.0 });
    let mut parent = element.visual_parent();
    while let Some(element) = parent {
        let child_offset = element
            .arranged_offset()
            .unwrap_or(Point { x: 0.0, y: 0.0 });
        offset.x += child_offset.x;
        offset.y += child_offset.y;
        parent = element.visual_parent();
    }
    let width = element.arranged_width().unwrap_or(0.0);
    let height = element.arranged_height().unwrap_or(0.0);
    point.x >= offset.x
        && point.y >= offset.y
        && point.x < offset.x + width
        && point.y < offset.y + height
}

impl Default for AutoHideOverlay {
    fn default() -> Self {
        Self::new()
    }
}
