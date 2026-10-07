use super::core;
use super::core::base::Size;
use super::core::graphics::IconSource;
use super::core::input::PointerEventArgs;
use super::core::layout::Visibility;
use super::core::theme::BrushStyle;
use super::core::ui::{ControlExt, Grid, GridExt, IconSourceElementExt, UIElementExt};
use super::custom_tab_view::TabItemPointerEvent;
use super::{
    CloseButtonPresentation, CustomTabCloseButton, CustomTabCloseButtonExt, TabStripPosition,
    weak_self_from_visual_owner,
};
#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::collections::HashMap;
use std::rc::Rc;

const TAB_HEADER_HEIGHT: f32 = 30.0;
/// Generic headers are arranged this much wider on each side so the selected outline's bottom
/// feet reach past the logical tab edge, as in the native TabView; header content keeps its
/// logical position.
pub(crate) const GENERIC_HEADER_OVERHANG: f32 = 4.0;

fn header_overhang(connected: bool) -> f32 {
    if connected {
        0.0
    } else {
        GENERIC_HEADER_OVERHANG
    }
}

fn item_tracks(position: TabStripPosition, connected: bool) -> Vec<core::layout::GridLength> {
    use core::layout::GridLength::Fixed;
    let height = if connected { 30.0 } else { 32.0 };
    let seam = if connected { 2.0 } else { 0.0 };
    if position == TabStripPosition::Top {
        vec![Fixed(height), Fixed(seam)]
    } else {
        vec![Fixed(seam), Fixed(height)]
    }
}

fn header_tracks(height: f32) -> Vec<core::layout::GridLength> {
    vec![core::layout::GridLength::Fixed(height)]
}

fn header_columns(leading: f32, trailing: f32) -> Vec<core::layout::GridLength> {
    use core::layout::GridLength::{Auto, Fixed, Star};
    vec![
        Fixed(leading),
        Auto,
        Auto,
        Auto,
        Auto,
        Star(1.0),
        Auto,
        Auto,
        Auto,
        Auto,
        Fixed(trailing),
    ]
}

fn outline_tracks(connected: bool) -> Vec<core::layout::GridLength> {
    use core::layout::GridLength::{Fixed, Star};
    let edge = if connected { 8.0 } else { 12.0 };
    vec![Fixed(edge), Star(1.0), Fixed(edge)]
}

#[cfg(test)]
thread_local! {
    static PRESENTATION_UPDATE_COUNTS: RefCell<HashMap<usize, usize>> =
        RefCell::new(HashMap::new());
}

/// One item displayed by [`CustomTabView`]. Its visual template is the tab header; its inherited
/// `ContentControl` content remains the logical page presented by the private content presenter.
#[elwindui::component(inherits ContentControl)]
pub struct CustomTabViewItem {
    #[environment(foreground)]
    palette_foreground: BrushStyle,
    #[state(default = false)]
    palette_dark: bool,
    #[prop(default = String::new())]
    header: String,
    #[prop(default = None)]
    icon: Option<IconSource>,
    #[prop(default = true)]
    closable: bool,
    #[state(default = None)]
    owner_pointer_callback: Option<Rc<dyn Fn(TabItemPointerEvent)>>,
    #[state(default = None)]
    owner_close_callback: Option<Rc<dyn Fn()>>,
    #[state(default = false)]
    header_handlers_bound: bool,
    #[state(default = false)]
    is_selected: bool,
    #[state(default = false)]
    active_document_marker: bool,
    #[state(default = false)]
    connected_chrome: bool,
    #[state(default = None)]
    document_pin_callback: Option<Rc<dyn Fn()>>,
    #[state(default = false)]
    document_pin_enabled: bool,
    #[state(default = false)]
    is_pointer_over: bool,
    #[state(default = TabStripPosition::Top)]
    tab_strip_position: TabStripPosition,
    #[state(default = CloseButtonPresentation::Always)]
    close_button_presentation: CloseButtonPresentation,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 0 } else { 1 })]
    header_row: i32,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 1 } else { 0 })]
    indicator_row: i32,
    #[computed(expr = item_tracks(tab_strip_position, connected_chrome))]
    header_grid_rows: Vec<elwindui::core::layout::GridLength>,
    #[computed(expr = if connected_chrome { TAB_HEADER_HEIGHT } else { 32.0 })]
    header_height: f32,
    #[computed(expr = if connected_chrome { 12.0 } else { 8.0 + GENERIC_HEADER_OVERHANG })]
    leading_inset: f32,
    #[computed(expr = header_overhang(connected_chrome) + if connected_chrome || !closable || close_button_presentation == CloseButtonPresentation::Never { 8.0 } else { 4.0 })]
    trailing_inset: f32,
    #[computed(expr = header_tracks(if connected_chrome { TAB_HEADER_HEIGHT } else { 32.0 }))]
    inner_header_rows: Vec<core::layout::GridLength>,
    #[computed(expr = header_columns(if connected_chrome { 12.0 } else { 8.0 + GENERIC_HEADER_OVERHANG }, header_overhang(connected_chrome) + if connected_chrome || !closable || close_button_presentation == CloseButtonPresentation::Never { 8.0 } else { 4.0 }))]
    inner_header_columns: Vec<core::layout::GridLength>,
    #[computed(expr = if connected_chrome { 6.0 } else { 10.0 })]
    icon_gap: f32,
    #[computed(expr = if connected_chrome { 8.0 } else if closable && close_button_presentation != CloseButtonPresentation::Never { 4.0 } else { 0.0 })]
    action_gap: f32,
    #[computed(expr = if !connected_chrome && is_selected { core::graphics::FontWeight::SEMI_BOLD } else { core::graphics::FontWeight::NORMAL })]
    header_weight: core::graphics::FontWeight,
    #[computed(expr = if icon.is_some() { Visibility::Visible } else { Visibility::Collapsed })]
    icon_visibility: Visibility,
    #[computed(expr = closable && close_button_presentation != CloseButtonPresentation::Never)]
    close_slot_visible: bool,
    #[computed(expr = closable && match close_button_presentation {
        CloseButtonPresentation::Always => true,
        CloseButtonPresentation::OnPointerOver => is_pointer_over,
        CloseButtonPresentation::Never => false,
    })]
    close_glyph_visible: bool,
    #[computed(expr = closable && close_button_presentation == CloseButtonPresentation::Always)]
    initial_close_glyph_visible: bool,
    #[computed(expr = connected_chrome && document_pin_enabled && tab_strip_position == TabStripPosition::Top)]
    pin_slot_visible: bool,
    #[computed(expr = pin_slot_visible && is_pointer_over)]
    pin_glyph_visible: bool,
    #[computed(expr = if is_selected { Visibility::Visible } else { Visibility::Collapsed })]
    indicator_visibility: Visibility,
    #[computed(expr = if connected_chrome && active_document_marker { Visibility::Visible } else { Visibility::Collapsed })]
    active_document_marker_visibility: Visibility,
    #[computed(expr = if active_document_marker { crate::support::accent_style() } else { BrushStyle::Primary })]
    active_document_marker_fill: BrushStyle,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { core::layout::VerticalAlignment::Top } else { core::layout::VerticalAlignment::Bottom })]
    baseline_alignment: core::layout::VerticalAlignment,
    #[computed(expr = if is_selected { Visibility::Visible } else { Visibility::Collapsed })]
    native_outline_visibility: Visibility,
    #[computed(expr = outline_tracks(connected_chrome))]
    outline_columns: Vec<core::layout::GridLength>,
    #[computed(expr = { let overhang = header_overhang(connected_chrome); vec![core::layout::GridLength::Fixed(overhang), core::layout::GridLength::Star(1.0), core::layout::GridLength::Fixed(overhang)] })]
    background_columns: Vec<core::layout::GridLength>,
    #[computed(expr = if !is_selected && is_pointer_over { Visibility::Visible } else { Visibility::Collapsed })]
    simple_background_visibility: Visibility,
    #[computed(expr = if !connected_chrome {
        super::support::native_tab_background(is_selected, palette_dark)
    } else if is_selected {
        elwindui::core::theme::BrushStyle::Background
    } else {
        elwindui::core::theme::BrushStyle::Secondary
    })]
    chrome_background: elwindui::core::theme::BrushStyle,
    #[computed(expr = elwindui::core::theme::BrushStyle::Value(core::graphics::Color::TRANSPARENT.into()))]
    transparent_brush: elwindui::core::theme::BrushStyle,
    #[computed(expr = super::support::selected_tab_stroke(is_selected, connected_chrome, active_document_marker, palette_dark))]
    chrome_stroke: elwindui::core::theme::BrushStyle,
    #[computed(expr = super::support::selected_tab_edge(false, tab_strip_position == TabStripPosition::Bottom, connected_chrome, palette_dark, super::support::selected_tab_stroke(is_selected, connected_chrome, active_document_marker, palette_dark)))]
    native_left_edge: Option<core::graphics::ImageSource>,
    #[computed(expr = super::support::selected_tab_edge(true, tab_strip_position == TabStripPosition::Bottom, connected_chrome, palette_dark, super::support::selected_tab_stroke(is_selected, connected_chrome, active_document_marker, palette_dark)))]
    native_right_edge: Option<core::graphics::ImageSource>,
    #[computed(expr = if is_selected { 1.0 } else { 0.0 })]
    chrome_stroke_width: f32,
    #[computed(expr = if is_selected {
        elwindui::core::theme::BrushStyle::Background
    } else {
        elwindui::core::theme::BrushStyle::Separator
    })]
    indicator_fill: elwindui::core::theme::BrushStyle,
    #[computed(expr = if is_selected { Visibility::Collapsed } else { Visibility::Visible })]
    separator_visibility: Visibility,
    template: template_view!(|this: Self| {
        on_mount {
            this.sync_palette();
            // Compact docking groups can assign a width smaller than the title's natural
            // width. Clip each header to its tab bounds so it cannot paint over its neighbor.
            this.set_clip_to_bounds(Some(true));
            this.bind_header_handlers();
            this.sync_close_button();
        }
        on_update(header, icon, closable, is_selected, tab_strip_position, close_button_presentation, palette_foreground) {
            this.sync_palette();
            this.sync_close_button();
        }
        let close_button = CustomTabCloseButton {
            Grid::column: 8
            slot_visible: close_slot_visible
            glyph_visible: initial_close_glyph_visible
            connected_chrome: connected_chrome
        };
        Grid {
            rows: header_grid_rows
            columns: [elwindui::core::layout::GridLength::Star(1.0)]
            Grid {
                Grid::row: header_row
                rows: [elwindui::core::layout::GridLength::Star(1.0)]
                columns: [elwindui::core::layout::GridLength::Star(1.0)]
                hit_test_visible: false
                Grid {
                    columns: background_columns
                    rows: [elwindui::core::layout::GridLength::Star(1.0)]
                    visibility: simple_background_visibility
                    hit_test_visible: false
                    Rectangle {
                        Grid::column: 1
                        fill: chrome_background
                        stroke: chrome_stroke
                        stroke_width: chrome_stroke_width
                        corner_radius: 4.0
                        hit_test_visible: false
                    }
                }
                Grid {
                    columns: outline_columns
                    rows: [elwindui::core::layout::GridLength::Star(1.0)]
                    visibility: native_outline_visibility
                    hit_test_visible: false
                    Image { source: native_left_edge hit_test_visible: false }
                    Rectangle { Grid::column: 1 fill: chrome_background hit_test_visible: false }
                    Rectangle { Grid::column: 1 height: 1.0 vertical_alignment: baseline_alignment fill: chrome_stroke hit_test_visible: false }
                    Image { Grid::column: 2 source: native_right_edge hit_test_visible: false }
                }
            }
            Grid {
                Grid::row: header_row
                height: header_height
                rows: inner_header_rows
                columns: inner_header_columns
                Rectangle {
                    width: leading_inset
                    height: header_height
                    fill: transparent_brush
                    hit_test_visible: false
                }
                Rectangle {
                    Grid::column: 1
                    width: 4.0
                    height: 16.0
                    fill: active_document_marker_fill
                    corner_radius: 2.0
                    vertical_alignment: elwindui::core::layout::VerticalAlignment::Center
                    visibility: active_document_marker_visibility
                    hit_test_visible: false
                }
                Rectangle {
                    Grid::column: 2
                    width: 6.0
                    height: header_height
                    fill: transparent_brush
                    visibility: active_document_marker_visibility
                    hit_test_visible: false
                }
                IconSourceElement {
                    Grid::column: 3
                    width: 16.0
                    height: 16.0
                    icon_source: icon
                    visibility: icon_visibility
                }
                Rectangle {
                    Grid::column: 4
                    width: icon_gap
                    height: header_height
                    fill: transparent_brush
                    visibility: icon_visibility
                    hit_test_visible: false
                }
                TextBlock {
                    Grid::column: 5
                    text: header
                    font_size: 12.0
                    font_weight: header_weight
                    vertical_alignment: core::layout::VerticalAlignment::Center
                    foreground: BrushStyle::Foreground
                    text_alignment: elwindui::core::ui::TextAlignment::Left
                }
                Rectangle { Grid::column: 6 width: action_gap fill: transparent_brush hit_test_visible: false }
                CustomTabCloseButton {
                    Grid::column: 7
                    connected_chrome: true
                    glyph_kind: super::ChromeIcon::Pin
                    slot_visible: pin_slot_visible
                    glyph_visible: pin_glyph_visible
                }
                close_button
                Rectangle {
                    Grid::column: 9
                    width: 1.0
                    height: 16.0
                    vertical_alignment: elwindui::core::layout::VerticalAlignment::Center
                    fill: elwindui::core::theme::BrushStyle::Separator
                    visibility: separator_visibility
                    hit_test_visible: false
                }
                Rectangle {
                    Grid::column: 10
                    width: trailing_inset
                    height: header_height
                    fill: transparent_brush
                    hit_test_visible: false
                }
            }
            Rectangle {
                Grid::row: indicator_row
                fill: indicator_fill
                visibility: indicator_visibility
                hit_test_visible: false
            }
        }
    }),
}

impl CustomTabViewItem {
    fn sync_palette(&self) {
        let dark = super::support::native_tab_dark(self.palette_foreground());
        if self.palette_dark() != dark {
            self.set_palette_dark(dark);
        }
    }
    /// Docking installs a weak owner callback; generic hosts never expose this action.
    #[doc(hidden)]
    pub fn set_document_pin_action(&self, enabled: bool, callback: Rc<dyn Fn()>) {
        if self.document_pin_callback().is_none() {
            self.set_document_pin_callback(Some(callback));
        }
        if self.document_pin_enabled() != enabled {
            self.set_document_pin_enabled(enabled);
        }
        self.sync_close_button();
    }
    pub(crate) fn apply_connected_chrome(&self, connected: bool) {
        if self.connected_chrome() != connected {
            self.set_connected_chrome(connected);
            self.sync_header_layout();
            self.sync_close_button();
        }
    }
}

#[elwindui::component]
impl CustomTabViewItem {
    #[overrides]
    fn on_apply_template(&self) {
        // The framework creates the visual template lazily, after Docking configures the item.
        // Replay the private style input with the template bindings installed.
        if self.connected_chrome() {
            self.set_connected_chrome(false);
            self.set_connected_chrome(true);
        }
        let selected = self.is_selected();
        self.set_is_selected(!selected);
        self.set_is_selected(selected);
        let dark = self.palette_dark();
        self.set_palette_dark(!dark);
        self.set_palette_dark(dark);
        self.sync_header_layout();
        self.sync_close_button();
    }

    #[overrides]
    fn hit_test_content(&self) -> bool {
        true
    }
}

impl CustomTabViewItem {
    pub(crate) fn intrinsic_header_width(&self, maximum: f32, height: f32) -> f32 {
        if let Some(width) = self.width() {
            return width.clamp(0.0, maximum);
        }
        let Some(header) = self
            .__template_root()
            .and_then(|root| root.visual_children().get(1).cloned())
        else {
            return 0.0;
        };
        let header = header.as_ui_element();
        let children = header.visual_children();
        let mut width = 0.0;
        for child in children {
            child.measure(Size {
                width: maximum,
                height,
            });
            if !child.participates_in_layout() {
                continue;
            }
            width += child.measured_size().map(|size| size.width).unwrap_or(0.0);
        }
        // The overhang is arranged outside the logical width (`header_overhang`).
        (width - 2.0 * self.header_overhang()).clamp(0.0, maximum)
    }

    /// Extra width the strip arranges on each side of this header beyond its logical width.
    pub(crate) fn header_overhang(&self) -> f32 {
        header_overhang(self.connected_chrome())
    }

    /// Creates a tab item with its default presentation properties.
    pub fn new_item() -> Rc<Self> {
        Self::new()
    }

    #[doc(hidden)]
    pub fn set_active_document_marker_visible(&self, visible: bool) {
        if self.active_document_marker() != visible {
            self.set_active_document_marker(visible);
        }
    }

    /// Returns whether this item may be closed by a user gesture.
    pub fn is_closable(&self) -> bool {
        self.closable()
    }

    /// Updates the tab label only when its value changes.
    #[cfg(not(rust_analyzer))]
    pub fn set_header(&self, header: String) {
        if self.header() == header {
            return;
        }
        <Self as CustomTabViewItemExt>::set_header(self, header);
    }

    /// Updates the close capability only when its value changes.
    #[cfg(not(rust_analyzer))]
    pub fn set_closable(&self, closable: bool) {
        if self.closable() == closable {
            return;
        }
        <Self as CustomTabViewItemExt>::set_closable(self, closable);
    }

    pub(crate) fn set_owner_pointer_handler(
        &self,
        callback: Option<Box<dyn Fn(TabItemPointerEvent)>>,
    ) {
        self.set_owner_pointer_callback(callback.map(Rc::from));
    }

    pub(crate) fn set_owner_close_handler(&self, callback: Option<Box<dyn Fn()>>) {
        self.set_owner_close_callback(callback.map(Rc::from));
        self.sync_close_button();
    }

    pub(crate) fn update_pointer_over(&self, value: bool) {
        if self.is_pointer_over() == value {
            return;
        }
        self.set_is_pointer_over(value);
        self.sync_close_button();
    }

    pub(crate) fn set_presentation(
        &self,
        is_selected: bool,
        is_pointer_over: bool,
        tab_strip_position: TabStripPosition,
        close_button_presentation: CloseButtonPresentation,
    ) {
        #[cfg(test)]
        self.note_presentation_update();
        if self.is_selected() != is_selected {
            self.set_is_selected(is_selected);
        }
        if self.is_pointer_over() != is_pointer_over {
            self.set_is_pointer_over(is_pointer_over);
        }
        let position_changed = self.tab_strip_position() != tab_strip_position;
        if position_changed {
            self.set_tab_strip_position(tab_strip_position);
            self.sync_header_layout();
        }
        if self.close_button_presentation() != close_button_presentation {
            self.set_close_button_presentation(close_button_presentation);
        }
        self.sync_close_button();
    }

    pub(crate) fn pointer_over(&self) -> bool {
        self.is_pointer_over()
    }

    pub(crate) fn refresh_theme(&self) {
        // The tab edge images bake the current Background/stroke brushes; replay the palette so
        // a runtime theme change rebuilds them even when the light/dark classification is equal.
        let dark = super::support::native_tab_dark(self.palette_foreground());
        self.set_palette_dark(!dark);
        self.set_palette_dark(dark);
        self.sync_close_button();
    }

    #[cfg(test)]
    pub(crate) fn presentation_update_count_for_test(&self) -> usize {
        let key = self as *const Self as usize;
        PRESENTATION_UPDATE_COUNTS.with(|counts| counts.borrow().get(&key).copied().unwrap_or(0))
    }

    #[cfg(test)]
    pub(crate) fn close_button_glyph_rebuild_count_for_test(&self) -> usize {
        core::visual_tree::find_all::<CustomTabCloseButton>(self)
            .into_iter()
            .find(|node| {
                node.as_any()
                    .downcast_ref::<CustomTabCloseButton>()
                    .is_some_and(|button| button.glyph_kind() == super::ChromeIcon::Close)
            })
            .and_then(|button| {
                button
                    .as_any()
                    .downcast_ref::<CustomTabCloseButton>()
                    .map(|button| button.glyph_rebuild_count_for_test())
            })
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn note_presentation_update(&self) {
        let key = self as *const Self as usize;
        PRESENTATION_UPDATE_COUNTS.with(|counts| {
            let mut counts = counts.borrow_mut();
            *counts.entry(key).or_default() += 1;
        });
    }

    fn sync_header_layout(&self) {
        let Some(root) = self.__template_root() else {
            return;
        };
        let root = if root.as_any().is::<Grid>() {
            root
        } else {
            let Some(root) = core::visual_tree::find_all::<Grid>(root.as_ref())
                .into_iter()
                .next()
            else {
                return;
            };
            root
        };
        if let Some(grid) = root.as_any().downcast_ref::<Grid>() {
            let indicator_height = if self.connected_chrome() { 2.0 } else { 0.0 };
            let rows = if self.tab_strip_position() == TabStripPosition::Top {
                vec![
                    elwindui::core::layout::GridLength::Fixed(self.header_height()),
                    elwindui::core::layout::GridLength::Fixed(indicator_height),
                ]
            } else {
                vec![
                    elwindui::core::layout::GridLength::Fixed(indicator_height),
                    elwindui::core::layout::GridLength::Fixed(self.header_height()),
                ]
            };
            grid.set_rows(rows);
        }
        let children = root.visual_children();
        if let Some(background) = children.first() {
            background
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.header_row());
        }
        if let Some(header) = children.get(1) {
            if let Some(grid) = header.as_any().downcast_ref::<Grid>() {
                grid.set_rows(header_tracks(self.header_height()));
                grid.set_columns(header_columns(self.leading_inset(), self.trailing_inset()));
                let parts = grid.visual_children();
                if let Some(leading) = parts.first() {
                    leading.set_width(self.leading_inset());
                    leading.set_height(self.header_height());
                }
                if let Some(trailing) = parts.last() {
                    trailing.set_width(self.trailing_inset());
                    trailing.set_height(self.header_height());
                }
            }
            let header = header.as_ui_element();
            header.set_attached::<i32>("Grid", "row", self.header_row());
            header.set_height(self.header_height());
        }
        if let Some(indicator) = children.get(2) {
            indicator
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.indicator_row());
        }
    }

    fn bind_header_handlers(&self) {
        if self.header_handlers_bound() {
            return;
        }
        let weak_self: std::rc::Weak<CustomTabViewItem> = self.weak_self();
        let self_handle: Option<Rc<CustomTabViewItem>> = weak_self.upgrade();
        if self_handle.is_none() {
            return;
        }
        self.set_header_handlers_bound(true);

        let weak_self = weak_self.clone();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    let item: Option<Rc<CustomTabViewItem>> = weak_self.upgrade();
                    if let Some(item) = item {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Pressed(*event));
                        }
                    }
                }
            }),
        );
        let weak_self: std::rc::Weak<CustomTabViewItem> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    let item: Option<Rc<CustomTabViewItem>> = weak_self.upgrade();
                    if let Some(item) = item {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Moved(*event));
                        }
                    }
                }
            }),
        );
        let weak_self: std::rc::Weak<CustomTabViewItem> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    let item: Option<Rc<CustomTabViewItem>> = weak_self.upgrade();
                    if let Some(item) = item {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Released(*event));
                        }
                    }
                }
            }),
        );
        let weak_self: std::rc::Weak<CustomTabViewItem> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    let item: Option<Rc<CustomTabViewItem>> = weak_self.upgrade();
                    if let Some(item) = item {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Canceled(*event));
                        }
                    }
                }
            }),
        );
        let weak_self: std::rc::Weak<CustomTabViewItem> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_entered",
            Box::new(move |_, args| {
                if !args.handled.get() {
                    let item: Option<Rc<CustomTabViewItem>> = weak_self.upgrade();
                    if let Some(item) = item {
                        item.update_pointer_over(true);
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Entered);
                        }
                    }
                }
            }),
        );
        let weak_self: std::rc::Weak<CustomTabViewItem> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_exited",
            Box::new(move |_, args| {
                if !args.handled.get() {
                    let item: Option<Rc<CustomTabViewItem>> = weak_self.upgrade();
                    if let Some(item) = item {
                        item.update_pointer_over(false);
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Exited);
                        }
                    }
                }
            }),
        );
    }

    fn sync_close_button(&self) {
        for node in core::visual_tree::find_all::<CustomTabCloseButton>(self) {
            let Some(button) = node.as_any().downcast_ref::<CustomTabCloseButton>() else {
                continue;
            };
            if button.glyph_kind() == super::ChromeIcon::Pin {
                button.set_on_close(self.document_pin_callback());
                button.sync_glyph_paint();
                continue;
            }
            if button.connected_chrome() != self.connected_chrome() {
                button.set_connected_chrome(self.connected_chrome());
            }
            let slot_visible = self.closable()
                && self.close_button_presentation() != CloseButtonPresentation::Never;
            if button.slot_visible() != slot_visible {
                button.set_slot_visible(slot_visible);
            }
            let glyph_visible = self.closable()
                && match self.close_button_presentation() {
                    CloseButtonPresentation::Always => true,
                    CloseButtonPresentation::OnPointerOver => self.is_pointer_over(),
                    CloseButtonPresentation::Never => false,
                };
            if button.glyph_visible() != glyph_visible {
                button.set_glyph_visible(glyph_visible);
            }
            button.set_on_close(self.owner_close_callback());
            button.sync_glyph_paint();
        }
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        weak_self_from_visual_owner(self)
    }

    /// Resolves the icon into the Core `IconSourceElement` realization used by callers that need a
    /// standalone icon element. The authored header template itself owns its icon element.
    pub fn realize_icon(&self) -> Option<Rc<dyn UIElementExt>> {
        self.icon().map(|icon_source| {
            let icon = core::ui::IconSourceElement::new();
            icon.set_icon_source(Some(icon_source));
            icon as Rc<dyn UIElementExt>
        })
    }

    /// Returns the close affordance from the mounted header template.
    pub fn close_button(&self) -> Rc<dyn UIElementExt> {
        let button = core::visual_tree::find_all::<CustomTabCloseButton>(self)
            .into_iter()
            .find(|node| {
                node.as_any()
                    .downcast_ref::<CustomTabCloseButton>()
                    .is_some_and(|button| button.glyph_kind() == super::ChromeIcon::Close)
            })
            .expect("CustomTabViewItem close button is not mounted");
        button
    }
}

#[cfg(test)]
mod tests {
    use super::core::graphics::{ImageSource, VectorNode, VectorPaint};
    use super::core::theme::ResolvedValue;
    use super::*;

    #[test]
    fn connected_selected_outline_tracks_activation_and_resets_for_a_generic_host() {
        let item = CustomTabViewItem::new_item();
        item.apply_connected_chrome(true);
        item.set_is_selected(true);
        let root: Rc<dyn UIElementExt> = item.clone();
        core::ui::layout_root(
            &root,
            Size {
                width: 200.0,
                height: 32.0,
            },
        );
        assert_eq!(item.chrome_stroke(), BrushStyle::Separator);

        item.set_active_document_marker_visible(true);
        assert_eq!(item.chrome_stroke(), super::super::support::accent_style());
        let ResolvedValue::Value(accent) = item
            .chrome_stroke()
            .resolve(&core::environment::application_environment())
        else {
            panic!("active outline must resolve to an accent brush");
        };
        for edge in [item.native_left_edge(), item.native_right_edge()] {
            let Some(ImageSource::Vector(image)) = edge else {
                panic!("selected outline must retain its curved vector edge");
            };
            let VectorNode::Path(path) = &image.root().children[1] else {
                panic!("edge must have a stroked contour");
            };
            let VectorPaint::Brush(stroke) = &path.stroke.as_ref().unwrap().paint else {
                panic!("active outline must paint with a brush");
            };
            assert_eq!(*stroke, accent);
        }

        item.set_active_document_marker_visible(false);
        assert_eq!(item.chrome_stroke(), BrushStyle::Separator);
        item.set_active_document_marker_visible(true);
        item.apply_connected_chrome(false);
        assert_eq!(
            item.chrome_stroke(),
            super::super::support::native_tab_stroke(item.palette_dark())
        );
    }
}
