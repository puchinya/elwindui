//! Reusable templated custom controls shared by Docking and application code.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

extern crate self as elwindui;

pub use elwindui_core as core;
pub use elwindui_core::ui;
pub use elwindui_macros::{class, component};

use core::base::{Point, Rect, Size};
use core::graphics::IconSource;
use core::input::{MouseButton, PointerEventArgs};
pub use core::layout::Orientation;
use core::layout::Visibility;
use core::reactive::Subscription;
use core::ui::{
    ContentControlExt, ControlExt, IconSourceElementExt, LayoutExt, ListExt, UIElementExt,
};
use std::rc::Rc;

const TAB_STRIP_HEIGHT: f32 = 32.0;
const TAB_DRAG_THRESHOLD: f32 = 4.0;

/// The edge on which a tab strip is authored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabStripPosition {
    #[default]
    /// Place tabs above the content.
    Top,
    /// Place tabs below the content.
    Bottom,
}

/// Controls when an item's close affordance is presented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloseButtonPresentation {
    #[default]
    /// Always show a close affordance for closeable items.
    Always,
    /// Show a close affordance while the pointer is over the item.
    OnPointerOver,
    /// Do not show a close affordance.
    Never,
}

/// Payload emitted when a tab drag starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragStartedEventArgs {
    /// The child index at the start of the gesture.
    pub index: usize,
    /// The root-relative pointer position.
    pub position: Point,
    /// The normalized logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted while a tab drag is active.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragMovedEventArgs {
    /// The child index being dragged.
    pub index: usize,
    /// The root-relative pointer position.
    pub position: Point,
    /// The normalized logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted when a tab drag completes or is canceled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragCompletedEventArgs {
    /// The child index being dragged.
    pub index: usize,
    /// The final root-relative pointer position.
    pub position: Point,
    /// The normalized logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
    /// Whether the gesture was canceled rather than committed.
    pub canceled: bool,
}

/// Payload emitted when a closeable tab requests closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabCloseRequestedEventArgs {
    /// The child index requesting closure.
    pub index: usize,
}

/// Backwards-compatible short name for [`TabDragStartedEventArgs`].
pub type TabDragStarted = TabDragStartedEventArgs;
/// Backwards-compatible short name for [`TabDragMovedEventArgs`].
pub type TabDragMoved = TabDragMovedEventArgs;
/// Backwards-compatible short name for [`TabDragCompletedEventArgs`].
pub type TabDragCompleted = TabDragCompletedEventArgs;
/// Backwards-compatible short name for [`TabCloseRequestedEventArgs`].
pub type TabCloseRequested = TabCloseRequestedEventArgs;

#[derive(Clone, Debug)]
enum TabGestureKind {
    Pressed,
    Dragging,
}

#[derive(Clone, Debug)]
pub struct TabGesture {
    item: std::rc::Weak<CustomTabViewItem>,
    press_position: Point,
    press_screen_position: Option<Point>,
    last_position: Point,
    last_screen_position: Option<Point>,
    kind: TabGestureKind,
}

pub enum TabItemPointerEvent {
    Pressed(PointerEventArgs),
    Moved(PointerEventArgs),
    Released(PointerEventArgs),
    Canceled(PointerEventArgs),
    Entered,
    Exited,
}

pub struct ContentEntry {
    item: std::rc::Weak<CustomTabViewItem>,
    content: Option<Rc<dyn UIElementExt>>,
    #[allow(dead_code)]
    subscription: Subscription,
}

#[derive(Clone, Debug)]
pub struct SplitterGesture {
    orientation: Orientation,
    position: Point,
    screen_position: Option<Point>,
    cumulative_delta: f32,
}

/// Payload emitted when splitter dragging starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterDragStartedEventArgs {
    /// The root-relative pointer position.
    pub position: Point,
    /// The logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted for an incremental splitter movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterDragDeltaEventArgs {
    /// Movement along the splitter's logical axis since the previous event.
    pub delta: f32,
    /// Total movement along the logical axis since the gesture began.
    pub cumulative_delta: f32,
    /// The root-relative pointer position.
    pub position: Point,
    /// The logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted when splitter dragging completes or is canceled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterDragCompletedEventArgs {
    /// Total movement along the logical axis.
    pub cumulative_delta: f32,
    /// The final root-relative pointer position.
    pub position: Point,
    /// The final normalized logical desktop position, when supplied by the host.
    pub screen_position: Option<Point>,
    /// Whether the gesture was canceled rather than committed.
    pub canceled: bool,
}

/// Backwards-compatible short name for [`SplitterDragStartedEventArgs`].
pub type SplitterDragStarted = SplitterDragStartedEventArgs;
/// Backwards-compatible short name for [`SplitterDragDeltaEventArgs`].
pub type SplitterDragDelta = SplitterDragDeltaEventArgs;
/// Backwards-compatible short name for [`SplitterDragCompletedEventArgs`].
pub type SplitterDragCompleted = SplitterDragCompletedEventArgs;

/// Private close-slot control used by [`CustomTabViewItem`]'s authored header template.
#[elwindui::component(inherits Control)]
struct CustomTabCloseButton {
    #[prop(default = true)]
    slot_visible: bool,
    #[prop(default = false)]
    glyph_visible: bool,
    #[state(default = None)]
    close_callback: Option<Rc<dyn Fn()>>,
    #[state(default = false)]
    pressed: bool,
    #[state(default = false)]
    handlers_bound: bool,
    #[computed(expr = if slot_visible { Visibility::Visible } else { Visibility::Collapsed })]
    slot_visibility: Visibility,
    #[computed(expr = if glyph_visible { "×".to_string() } else { String::new() })]
    glyph_text: String,
    template: template_view! {
        on_mount {
            this.bind_pointer_handlers();
        }
        Grid {
            width: 20.0
            height: 32.0
            visibility: slot_visibility
            TextBlock {
                text: glyph_text
                text_alignment: elwindui::core::ui::TextAlignment::Center
            }
        }
    },
}

#[elwindui::component]
impl CustomTabCloseButton {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        self.slot_visible()
    }
}

/// One item displayed by [`CustomTabView`]. Its visual template is the tab header; its inherited
/// `ContentControl` content remains the logical page presented by the private content presenter.
#[elwindui::component(inherits ContentControl)]
pub struct CustomTabViewItem {
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
    is_pointer_over: bool,
    #[state(default = TabStripPosition::Top)]
    tab_strip_position: TabStripPosition,
    #[state(default = CloseButtonPresentation::Always)]
    close_button_presentation: CloseButtonPresentation,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 0 } else { 1 })]
    header_row: i32,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 1 } else { 0 })]
    indicator_row: i32,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top {
        vec![
            elwindui::core::layout::GridLength::Fixed(30.0),
            elwindui::core::layout::GridLength::Fixed(2.0),
        ]
    } else {
        vec![
            elwindui::core::layout::GridLength::Fixed(2.0),
            elwindui::core::layout::GridLength::Fixed(30.0),
        ]
    })]
    header_grid_rows: Vec<elwindui::core::layout::GridLength>,
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
    #[computed(expr = if is_selected { Visibility::Visible } else { Visibility::Collapsed })]
    indicator_visibility: Visibility,
    template: template_view! {
        on_mount {
            this.bind_header_handlers();
            this.sync_close_button();
        }
        on_update(header, icon, closable, is_selected, is_pointer_over, tab_strip_position, close_button_presentation) {
            this.sync_close_button();
        }
        let close_button = CustomTabCloseButton {
            slot_visible: close_slot_visible
            glyph_visible: close_glyph_visible
        };
        Grid {
            rows: header_grid_rows
            columns: [
                elwindui::core::layout::GridLength::Fixed(10.0),
                elwindui::core::layout::GridLength::Auto,
                elwindui::core::layout::GridLength::Fixed(10.0),
            ]
            HorizontalLayout {
                Grid::row: header_row
                Grid::column: 1
                height: 30.0
                spacing: 6.0
                IconSourceElement {
                    width: 16.0
                    height: 16.0
                    icon_source: icon
                    visibility: icon_visibility
                }
                TextBlock {
                    text: header
                    text_alignment: elwindui::core::ui::TextAlignment::Center
                }
                close_button
            }
            Rectangle {
                Grid::row: indicator_row
                Grid::column: 1
                fill: "#0078d4"
                visibility: indicator_visibility
            }
        }
    },
}

#[elwindui::component]
impl CustomTabViewItem {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        true
    }
}

/// Private presenter that owns the ordered tab-header controls and delegates layout to
/// `HorizontalLayout`.
#[elwindui::component(inherits HorizontalLayout)]
struct CustomTabStripPresenter {
    #[prop(default = Vec::new())]
    items: Vec<Rc<CustomTabViewItem>>,
    #[prop(default = 0)]
    selected_index: usize,
    #[prop(default = TabStripPosition::Top)]
    tab_strip_position: TabStripPosition,
    #[prop(default = CloseButtonPresentation::Always)]
    close_button_presentation: CloseButtonPresentation,
    #[state(default = Vec::new())]
    bound_items: Vec<std::rc::Weak<CustomTabViewItem>>,
    body: view! {
        on_mount {
            this.reconcile_items();
        }
        on_update(items, selected_index, tab_strip_position, close_button_presentation) {
            this.reconcile_items();
        }
    },
}

impl CustomTabStripPresenter {
    fn reconcile_items(&self) {
        let items = self.items();
        let unchanged = self.bound_items().len() == items.len()
            && self
                .bound_items()
                .iter()
                .zip(items.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)));
        if !unchanged {
            LayoutExt::children(self).clear();
            for item in &items {
                let visual: Rc<dyn UIElementExt> = item.clone();
                LayoutExt::children(self).add(visual);
            }
            self.set_bound_items(items.iter().map(Rc::downgrade).collect());
        }
        self.sync_items(&items);
    }

    fn sync_items(&self, items: &[Rc<CustomTabViewItem>]) {
        let selected = self.selected_index();
        let position = self.tab_strip_position();
        let presentation = self.close_button_presentation();
        for (index, item) in items.iter().enumerate() {
            item.set_presentation(
                index == selected,
                item.is_pointer_over(),
                position,
                presentation,
            );
        }
    }
}

#[elwindui::component]
impl CustomTabStripPresenter {}

/// Private presenter that keeps every tab page content visually attached while arranging only the
/// selected page into the available content rectangle.
#[elwindui::component(inherits Control)]
struct CustomTabContentPresenter {
    #[prop(default = Vec::new())]
    items: Vec<Rc<CustomTabViewItem>>,
    #[prop(default = 0)]
    selected_index: usize,
    #[state(default = Vec::new())]
    bound_items: Vec<std::rc::Weak<CustomTabViewItem>>,
    #[state(default = None)]
    presentation_state: Option<Rc<std::cell::RefCell<Vec<ContentEntry>>>>,
    template: template_view! {
        on_mount {
            this.reconcile_contents();
        }
        on_update(items, selected_index) {
            this.reconcile_contents();
            this.invalidate_measure();
        }
        Grid {}
    },
}

impl CustomTabContentPresenter {
    fn state(&self) -> Rc<std::cell::RefCell<Vec<ContentEntry>>> {
        if let Some(state) = self.presentation_state() {
            return state;
        }
        let state = Rc::new(std::cell::RefCell::new(Vec::new()));
        self.set_presentation_state(Some(state.clone()));
        state
    }

    fn reconcile_contents(&self) {
        let items = self.items();
        let unchanged = self.bound_items().len() == items.len()
            && self
                .bound_items()
                .iter()
                .zip(items.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)));
        if unchanged {
            return;
        }

        let state = self.state();
        let old_entries = std::mem::take(&mut *state.borrow_mut());
        for entry in old_entries {
            if let Some(old) = entry.content {
                self.as_ui_element().visual_collection.remove(&old);
            }
        }
        let mut entries = Vec::with_capacity(items.len());
        for item in &items {
            let content = item.__content_opt();
            if let Some(content) = content.as_ref() {
                if let Some(parent) = content.visual_parent() {
                    let owner = self.as_ui_element().visual_collection.owner_rc();
                    assert!(
                        owner
                            .as_ref()
                            .is_some_and(|owner| Rc::ptr_eq(&parent, owner)),
                        "CustomTabContentPresenter cannot steal content owned by another visual parent"
                    );
                }
                self.as_ui_element().visual_collection.add(content.clone());
            }
            let weak_presenter = self.weak_self();
            let weak_item = Rc::downgrade(item);
            let subscription = item.__subscribe_content_changed(Rc::new(move |replacement| {
                if let (Some(presenter), Some(item)) =
                    (weak_presenter.upgrade(), weak_item.upgrade())
                {
                    presenter.replace_item_content(&item, replacement);
                }
            }));
            entries.push(ContentEntry {
                item: Rc::downgrade(item),
                content,
                subscription,
            });
        }
        *state.borrow_mut() = entries;
        self.set_bound_items(items.iter().map(Rc::downgrade).collect());
    }

    fn replace_item_content(
        &self,
        item: &CustomTabViewItem,
        replacement: Option<Rc<dyn UIElementExt>>,
    ) {
        let state = self.state();
        let mut entries = state.borrow_mut();
        let Some(entry) = entries.iter_mut().find(|entry| {
            entry
                .item
                .upgrade()
                .is_some_and(|candidate| std::ptr::eq(candidate.as_ref(), item))
        }) else {
            return;
        };
        if let Some(old) = entry.content.take() {
            self.as_ui_element().visual_collection.remove(&old);
        }
        if let Some(content) = replacement {
            if let Some(parent) = content.visual_parent() {
                let owner = self.as_ui_element().visual_collection.owner_rc();
                assert!(
                    owner
                        .as_ref()
                        .is_some_and(|owner| Rc::ptr_eq(&parent, owner)),
                    "CustomTabContentPresenter cannot steal replacement content"
                );
            }
            self.as_ui_element().visual_collection.add(content.clone());
            entry.content = Some(content);
        }
        drop(entries);
        self.invalidate_measure();
    }

    fn entries(&self) -> Vec<(usize, Option<Rc<dyn UIElementExt>>)> {
        self.state()
            .borrow()
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, entry.content.clone()))
            .collect()
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        self.__self_weak
            .borrow()
            .clone()
            .upgrade()
            .and_then(|rc| rc.downcast::<Self>().ok())
            .map(|rc| Rc::downgrade(&rc))
            .unwrap_or_default()
    }
}

#[elwindui::component]
impl CustomTabContentPresenter {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        self.reconcile_contents();
        if let Some(root) = self.__template_root() {
            root.measure(available);
        }
        let entries = self.entries();
        for (_, content) in &entries {
            if let Some(content) = content {
                content.measure(available);
            }
        }
        entries
            .iter()
            .find(|(index, _)| *index == self.selected_index())
            .and_then(|(_, content)| content.as_ref()?.measured_size())
            .unwrap_or_default()
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        self.reconcile_contents();
        if let Some(root) = self.__template_root() {
            root.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width: final_size.width.max(0.0),
                height: final_size.height.max(0.0),
            });
        }
        for (index, content) in self.entries() {
            let Some(content) = content else {
                continue;
            };
            let rect = if index == self.selected_index() {
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: final_size.width.max(0.0),
                    height: final_size.height.max(0.0),
                }
            } else {
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                }
            };
            content.set_clip_to_bounds(Some(true));
            content.arrange(rect);
        }
        final_size
    }
}

/// A templated tab strip and selected-content host.
#[elwindui::component(inherits Control)]
#[content(tab_children)]
pub struct CustomTabView {
    #[prop(default = Vec::new())]
    tab_children: Vec<Rc<CustomTabViewItem>>,
    #[prop(default = 0)]
    #[two_way]
    selected_index: usize,
    #[prop(default = TabStripPosition::Top)]
    tab_strip_position: TabStripPosition,
    #[prop(default = CloseButtonPresentation::Always)]
    close_button_presentation: CloseButtonPresentation,
    #[state(default = None)]
    selected_index_callback: Option<Rc<dyn Fn(usize)>>,
    #[state(default = None)]
    close_requested_callback: Option<Rc<dyn Fn(usize)>>,
    #[state(default = None)]
    tab_drag_started_callback: Option<Rc<dyn Fn(TabDragStartedEventArgs)>>,
    #[state(default = None)]
    tab_drag_moved_callback: Option<Rc<dyn Fn(TabDragMovedEventArgs)>>,
    #[state(default = None)]
    tab_drag_completed_callback: Option<Rc<dyn Fn(TabDragCompletedEventArgs)>>,
    #[state(default = Vec::new())]
    bound_items: Vec<std::rc::Weak<CustomTabViewItem>>,
    #[state(default = None)]
    tab_gesture: Option<TabGesture>,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 0 } else { 1 })]
    tab_strip_row: i32,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 1 } else { 0 })]
    content_row: i32,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top {
        vec![
            elwindui::core::layout::GridLength::Fixed(TAB_STRIP_HEIGHT),
            elwindui::core::layout::GridLength::Star(1.0),
        ]
    } else {
        vec![
            elwindui::core::layout::GridLength::Star(1.0),
            elwindui::core::layout::GridLength::Fixed(TAB_STRIP_HEIGHT),
        ]
    })]
    grid_rows: Vec<elwindui::core::layout::GridLength>,
    #[state(default = Vec::new())]
    template_items: Vec<Rc<CustomTabViewItem>>,
    #[computed(expr = template_items.clone())]
    tab_items: Vec<Rc<CustomTabViewItem>>,
    #[computed(expr = template_items.clone())]
    content_items: Vec<Rc<CustomTabViewItem>>,
    template: template_view! {
        on_mount {
            this.set_clip_to_bounds(Some(true));
            this.reconcile_children();
        }
        on_update(tab_children, template_items, selected_index, tab_strip_position, close_button_presentation) {
            this.reconcile_children();
        }
        let tab_strip = CustomTabStripPresenter {
            items: tab_items
            selected_index: selected_index
            tab_strip_position: tab_strip_position
            close_button_presentation: close_button_presentation
            Grid::row: tab_strip_row
        };
        let content_presenter = CustomTabContentPresenter {
            items: content_items
            selected_index: selected_index
            Grid::row: content_row
        };
        Grid {
            rows: grid_rows
            columns: [elwindui::core::layout::GridLength::Star(1.0)]
            tab_strip
            content_presenter
        }
    },
}

#[elwindui::component]
impl CustomTabView {}

/// A templated splitter that reports logical-axis drag deltas.
#[elwindui::component(inherits Control)]
pub struct CustomSplitter {
    #[prop(default = Orientation::Horizontal)]
    orientation: Orientation,
    #[state(default = None)]
    drag_started_callback: Option<Rc<dyn Fn(SplitterDragStartedEventArgs)>>,
    #[state(default = None)]
    drag_delta_callback: Option<Rc<dyn Fn(SplitterDragDeltaEventArgs)>>,
    #[state(default = None)]
    drag_completed_callback: Option<Rc<dyn Fn(SplitterDragCompletedEventArgs)>>,
    #[state(default = None)]
    gesture: Option<SplitterGesture>,
    template: template_view! {
        on_mount {
            this.bind_pointer_handlers();
        }
        match orientation {
            Orientation::Horizontal => {
                Rectangle {
                    width: 6.0
                    fill: "#d0d0d0"
                }
            }
            Orientation::Vertical => {
                Rectangle {
                    height: 6.0
                    fill: "#d0d0d0"
                }
            }
        }
    },
}

#[elwindui::component]
impl CustomSplitter {}

impl CustomTabCloseButton {
    fn set_on_close(&self, callback: Option<Rc<dyn Fn()>>) {
        self.set_close_callback(callback);
    }

    fn bind_pointer_handlers(&self) {
        if self.handlers_bound() {
            return;
        }
        let weak_self = self.weak_self();
        if weak_self.upgrade().is_none() {
            return;
        }
        self.set_handlers_bound(true);

        let weak_self = weak_self.clone();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, args| {
                if args.handled.get()
                    || event.button != Some(MouseButton::Left)
                    || weak_self.upgrade().is_none()
                {
                    return;
                }
                let button = weak_self.upgrade().expect("close button alive");
                if !button.slot_visible() {
                    return;
                }
                button.set_pressed(true);
                args.handled.set(true);
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                let Some(button) = weak_self.upgrade() else {
                    return;
                };
                if !button.pressed() {
                    return;
                }
                button.set_pressed(false);
                args.handled.set(true);
                if button.slot_visible()
                    && button.contains_root_point(event.position)
                    && let Some(callback) = button.close_callback()
                {
                    callback();
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |_, args| {
                if let Some(button) = weak_self.upgrade() {
                    if button.pressed() {
                        args.handled.set(true);
                    }
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |_, args| {
                if let Some(button) = weak_self.upgrade() {
                    button.set_pressed(false);
                    args.handled.set(true);
                }
            }),
        );
    }

    fn contains_root_point(&self, point: Point) -> bool {
        let mut offset = self.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let mut parent = self.visual_parent();
        while let Some(element) = parent {
            let child_offset = element
                .arranged_offset()
                .unwrap_or(Point { x: 0.0, y: 0.0 });
            offset.x += child_offset.x;
            offset.y += child_offset.y;
            parent = element.visual_parent();
        }
        let width = self.arranged_width().unwrap_or(20.0);
        let height = self.arranged_height().unwrap_or(32.0);
        point.x >= offset.x
            && point.y >= offset.y
            && point.x < offset.x + width
            && point.y < offset.y + height
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        self.__self_weak
            .borrow()
            .clone()
            .upgrade()
            .and_then(|rc| rc.downcast::<Self>().ok())
            .map(|rc| Rc::downgrade(&rc))
            .unwrap_or_default()
    }
}

impl CustomTabViewItem {
    /// Creates a tab item with its default presentation properties.
    pub fn new_item() -> Rc<Self> {
        Self::new()
    }

    /// Returns whether this item may be closed by a user gesture.
    pub fn is_closable(&self) -> bool {
        self.closable()
    }

    /// Updates the tab label only when its value changes.
    pub fn set_header(&self, header: String) {
        if self.header() == header {
            return;
        }
        <Self as CustomTabViewItemExt>::set_header(self, header);
    }

    /// Updates the close capability only when its value changes.
    pub fn set_closable(&self, closable: bool) {
        if self.closable() == closable {
            return;
        }
        <Self as CustomTabViewItemExt>::set_closable(self, closable);
    }

    fn set_owner_pointer_handler(&self, callback: Option<Box<dyn Fn(TabItemPointerEvent)>>) {
        self.set_owner_pointer_callback(callback.map(Rc::from));
    }

    fn set_owner_close_handler(&self, callback: Option<Box<dyn Fn()>>) {
        self.set_owner_close_callback(callback.map(Rc::from));
        self.sync_close_button();
    }

    fn update_pointer_over(&self, value: bool) {
        if self.is_pointer_over() == value {
            return;
        }
        let old_glyph_visible = self.close_glyph_visible();
        self.is_pointer_over.set(value);
        let new_glyph_visible = self.closable()
            && match self.close_button_presentation() {
                CloseButtonPresentation::Always => true,
                CloseButtonPresentation::OnPointerOver => value,
                CloseButtonPresentation::Never => false,
            };
        self.close_glyph_visible.set(new_glyph_visible);
        if old_glyph_visible != new_glyph_visible {
            self.on_property_changed(CustomTabViewItemProperty::close_glyph_visible);
        }
        self.on_property_changed(CustomTabViewItemProperty::is_pointer_over);
    }

    fn set_presentation(
        &self,
        is_selected: bool,
        is_pointer_over: bool,
        tab_strip_position: TabStripPosition,
        close_button_presentation: CloseButtonPresentation,
    ) {
        if self.is_selected() != is_selected {
            self.set_is_selected(is_selected);
        }
        if self.is_pointer_over() != is_pointer_over {
            self.set_is_pointer_over(is_pointer_over);
        }
        let position_changed = self.tab_strip_position() != tab_strip_position;
        if position_changed {
            self.set_tab_strip_position(tab_strip_position);
            self.sync_header_rows();
        }
        if self.close_button_presentation() != close_button_presentation {
            self.set_close_button_presentation(close_button_presentation);
        }
        self.sync_close_button();
    }

    fn sync_header_rows(&self) {
        let Some(root) = self.__template_root() else {
            return;
        };
        let children = root.visual_children();
        if let Some(header) = children.first() {
            header
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.header_row());
        }
        if let Some(indicator) = children.get(1) {
            indicator
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.indicator_row());
        }
    }

    fn bind_header_handlers(&self) {
        if self.header_handlers_bound() {
            return;
        }
        let weak_self = self.weak_self();
        if weak_self.upgrade().is_none() {
            return;
        }
        self.set_header_handlers_bound(true);

        let weak_self = weak_self.clone();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Pressed(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Moved(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Released(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Canceled(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_entered",
            Box::new(move |_, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        item.update_pointer_over(true);
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Entered);
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_exited",
            Box::new(move |_, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
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
            break;
        }
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        self.__self_weak
            .borrow()
            .clone()
            .upgrade()
            .and_then(|rc| rc.downcast::<Self>().ok())
            .map(|rc| Rc::downgrade(&rc))
            .unwrap_or_default()
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
            .next()
            .expect("CustomTabViewItem close button is not mounted");
        button
    }
}

impl CustomTabView {
    /// Returns a newly constructed tab view.
    pub fn new_view() -> Rc<Self> {
        Self::new()
    }

    /// Compatibility alias for the original short tab-position property name.
    pub fn tab_position(&self) -> TabStripPosition {
        self.tab_strip_position()
    }

    /// Compatibility alias for the original short tab-position property setter.
    pub fn set_tab_position(&self, position: TabStripPosition) {
        self.set_tab_strip_position(position);
    }

    /// Replaces the ordered tab list and reconciles its logical and visual ownership.
    pub fn replace_children(&self, children: Vec<Rc<CustomTabViewItem>>) {
        self.set_children(children);
    }

    /// Returns the established typed ordered-list surface for tab items.
    pub fn children(&self) -> &dyn ListExt<dyn CustomTabViewItemExt> {
        self
    }

    /// Replaces the concrete list used by declarative and programmatic callers.
    pub fn set_children(&self, children: Vec<Rc<CustomTabViewItem>>) {
        <Self as CustomTabViewExt>::set_tab_children(self, children);
    }

    /// Appends one item to the ordered tab list.
    pub fn append_child(&self, item: Rc<CustomTabViewItem>) {
        let mut children = self.children_values();
        children.push(item);
        self.set_children(children);
    }

    /// Removes and returns an item by index, if present.
    pub fn remove_child(&self, index: usize) -> Option<Rc<CustomTabViewItem>> {
        let mut children = self.children_values();
        if index >= children.len() {
            return None;
        }
        let removed = children.remove(index);
        self.set_children(children);
        Some(removed)
    }

    /// Selects a child and emits the TwoWay notification when the value changes.
    pub fn select_index(&self, index: usize) -> bool {
        if index >= self.children_values().len() || self.selected_index() == index {
            return false;
        }
        self.set_selected_index(index);
        if let Some(callback) = self.selected_index_callback() {
            callback(index);
        }
        true
    }

    /// Registers the callback used by user-driven TwoWay selected-index changes.
    pub fn set_on_selected_index_change(&self, callback: Box<dyn Fn(usize)>) {
        self.set_selected_index_callback(Some(Rc::new(callback)));
    }

    /// Registers the callback used by user-driven TwoWay selected-index changes.
    pub fn set_on_selected_index_changed(&self, callback: impl Fn(usize) + 'static) {
        self.set_on_selected_index_change(Box::new(callback));
    }

    /// Removes the selected-index callback.
    pub fn clear_on_selected_index_changed(&self) {
        self.set_selected_index_callback(None);
    }

    /// Registers a close-request callback. A request never removes an item itself.
    pub fn set_on_close_request(&self, callback: Box<dyn Fn(usize)>) {
        self.set_close_requested_callback(Some(Rc::new(callback)));
    }

    /// Registers a close-request callback using the payload alias retained for compatibility.
    pub fn set_on_close_requested(&self, callback: impl Fn(TabCloseRequested) + 'static) {
        self.set_on_close_request(Box::new(move |index| callback(TabCloseRequested { index })));
    }

    /// Requests closure of a tab if its capability permits user closure.
    pub fn request_close(&self, index: usize) -> bool {
        let children = self.children_values();
        let Some(item) = children.get(index) else {
            return false;
        };
        if !item.is_closable() {
            return false;
        }
        if let Some(callback) = self.close_requested_callback() {
            callback(index);
        }
        true
    }

    /// Sets the tab-drag-started callback.
    pub fn set_on_tab_drag_started(&self, callback: Box<dyn Fn(TabDragStartedEventArgs)>) {
        self.set_tab_drag_started_callback(Some(Rc::from(callback)));
    }

    /// Sets the tab-drag-moved callback.
    pub fn set_on_tab_drag_moved(&self, callback: Box<dyn Fn(TabDragMovedEventArgs)>) {
        self.set_tab_drag_moved_callback(Some(Rc::from(callback)));
    }

    /// Sets the tab-drag-completed callback.
    pub fn set_on_tab_drag_completed(&self, callback: Box<dyn Fn(TabDragCompletedEventArgs)>) {
        self.set_tab_drag_completed_callback(Some(Rc::from(callback)));
    }

    fn children_values(&self) -> Vec<Rc<CustomTabViewItem>> {
        <Self as CustomTabViewExt>::tab_children(self)
    }

    /// Returns the typed ordered-list surface used by dynamic content composition.
    pub fn children_list(&self) -> &dyn ListExt<dyn CustomTabViewItemExt> {
        self
    }

    fn reconcile_children(&self) {
        let children = self.children_values();
        self.validate_children(&children);
        let unchanged = self.bound_items().len() == children.len()
            && self
                .bound_items()
                .iter()
                .zip(children.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)));
        if unchanged {
            self.sync_presenters(&children);
            return;
        }

        if self.cancel_removed_gesture(&children) {
            self.reconcile_children();
            return;
        }

        for old in self
            .bound_items()
            .into_iter()
            .filter_map(|item| item.upgrade())
        {
            old.set_owner_pointer_handler(None);
            old.set_owner_close_handler(None);
        }

        let weak_view = self.weak_self();
        for item in &children {
            let weak_item = Rc::downgrade(item);
            let weak_view_for_pointer = weak_view.clone();
            item.set_owner_pointer_handler(Some(Box::new(move |event| {
                if let (Some(view), Some(item)) =
                    (weak_view_for_pointer.upgrade(), weak_item.upgrade())
                {
                    view.handle_item_pointer(&item, event);
                }
            })));
            let weak_item = Rc::downgrade(item);
            let weak_view_for_close = weak_view.clone();
            item.set_owner_close_handler(Some(Box::new(move || {
                if let (Some(view), Some(item)) =
                    (weak_view_for_close.upgrade(), weak_item.upgrade())
                {
                    if let Some(index) = view.index_of(&item) {
                        view.request_close(index);
                    }
                }
            })));
        }

        self.set_bound_items(children.iter().map(Rc::downgrade).collect());
        let template_matches = self.template_items().len() == children.len()
            && self
                .template_items()
                .iter()
                .zip(children.iter())
                .all(|(old, new)| Rc::ptr_eq(old, new));
        if !template_matches {
            self.set_template_items(children.clone());
            self.sync_presenters(&children);
            return;
        }
        self.sync_presenters(&children);
    }

    fn sync_presenters(&self, children: &[Rc<CustomTabViewItem>]) {
        let selected = self.selected_index();
        let position = self.tab_strip_position();
        let close = self.close_button_presentation();

        for node in core::visual_tree::find_all::<CustomTabStripPresenter>(self) {
            let Some(presenter) = node.as_any().downcast_ref::<CustomTabStripPresenter>() else {
                continue;
            };
            presenter.set_items(children.to_vec());
            presenter.set_selected_index(selected);
            presenter.set_tab_strip_position(position);
            presenter.set_close_button_presentation(close);
            presenter
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.tab_strip_row());
            break;
        }
        for node in core::visual_tree::find_all::<CustomTabContentPresenter>(self) {
            let Some(presenter) = node.as_any().downcast_ref::<CustomTabContentPresenter>() else {
                continue;
            };
            presenter.set_items(children.to_vec());
            presenter.set_selected_index(selected);
            presenter
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.content_row());
            break;
        }
        for (index, item) in children.iter().enumerate() {
            item.set_presentation(index == selected, item.is_pointer_over(), position, close);
        }
    }

    fn validate_children(&self, children: &[Rc<CustomTabViewItem>]) {
        let owner = core::visual_tree::find_all::<CustomTabStripPresenter>(self)
            .into_iter()
            .next();
        for (index, child) in children.iter().enumerate() {
            assert!(
                !children[..index]
                    .iter()
                    .any(|previous| Rc::ptr_eq(previous, child)),
                "CustomTabView cannot attach the same CustomTabViewItem twice; detach the duplicate first"
            );
            if let Some(parent) = child.visual_parent() {
                assert!(
                    owner
                        .as_ref()
                        .is_some_and(|owner| Rc::ptr_eq(&parent, owner)),
                    "CustomTabViewItem is already owned by another Visual parent; detach it before attaching"
                );
            }
        }
    }

    fn cancel_removed_gesture(&self, children: &[Rc<CustomTabViewItem>]) -> bool {
        let Some(gesture) = self.tab_gesture() else {
            return false;
        };
        let still_present = gesture.item.upgrade().is_some_and(|item| {
            children
                .iter()
                .any(|candidate| Rc::ptr_eq(candidate, &item))
        });
        if still_present {
            return false;
        }
        self.set_tab_gesture(None);
        if matches!(gesture.kind, TabGestureKind::Dragging) {
            if let Some(callback) = self.tab_drag_completed_callback() {
                callback(TabDragCompletedEventArgs {
                    index: gesture_index(&gesture, self),
                    position: gesture.last_position,
                    screen_position: gesture.last_screen_position,
                    canceled: true,
                });
                return true;
            }
        }
        false
    }

    fn handle_item_pointer(&self, item: &Rc<CustomTabViewItem>, event: TabItemPointerEvent) {
        match event {
            TabItemPointerEvent::Pressed(event) => self.handle_pointer_pressed(item, &event),
            TabItemPointerEvent::Moved(event) => self.handle_pointer_moved(item, &event),
            TabItemPointerEvent::Released(event) => self.handle_pointer_released(item, &event),
            TabItemPointerEvent::Canceled(event) => self.handle_pointer_canceled(item, &event),
            TabItemPointerEvent::Entered => item.update_pointer_over(true),
            TabItemPointerEvent::Exited => item.update_pointer_over(false),
        }
    }

    fn handle_pointer_pressed(&self, item: &Rc<CustomTabViewItem>, event: &PointerEventArgs) {
        if event.button != Some(MouseButton::Left) {
            return;
        }
        self.set_tab_gesture(Some(TabGesture {
            item: Rc::downgrade(item),
            press_position: event.position,
            press_screen_position: event.screen_position,
            last_position: event.position,
            last_screen_position: event.screen_position,
            kind: TabGestureKind::Pressed,
        }));
        if let Some(index) = self.index_of(item) {
            let _ = self.select_index(index);
        }
    }

    fn handle_pointer_moved(&self, item: &Rc<CustomTabViewItem>, event: &PointerEventArgs) {
        let Some(mut gesture) = self.tab_gesture() else {
            return;
        };
        if !gesture
            .item
            .upgrade()
            .is_some_and(|active| Rc::ptr_eq(&active, item))
        {
            return;
        }
        gesture.last_position = event.position;
        gesture.last_screen_position = event.screen_position;
        match gesture.kind.clone() {
            TabGestureKind::Pressed => {
                let dx = event.position.x - gesture.press_position.x;
                let dy = event.position.y - gesture.press_position.y;
                if dx.mul_add(dx, dy * dy).sqrt() < TAB_DRAG_THRESHOLD {
                    self.set_tab_gesture(Some(gesture));
                    return;
                }
                gesture.kind = TabGestureKind::Dragging;
                self.set_tab_gesture(Some(gesture.clone()));
                let gesture_item = gesture.item.clone();
                let Some(index) = self.item_index_from_weak(&gesture_item) else {
                    return;
                };
                if let Some(callback) = self.tab_drag_started_callback() {
                    callback(TabDragStartedEventArgs {
                        index,
                        position: gesture.press_position,
                        screen_position: gesture.press_screen_position,
                    });
                }

                let Some(current) = self.tab_gesture() else {
                    return;
                };
                let same_item = current
                    .item
                    .upgrade()
                    .zip(gesture_item.upgrade())
                    .is_some_and(|(current, original)| Rc::ptr_eq(&current, &original));
                if !same_item || !matches!(current.kind, TabGestureKind::Dragging) {
                    return;
                }
                let Some(index) = self.item_index_from_weak(&current.item) else {
                    return;
                };
                if let Some(callback) = self.tab_drag_moved_callback() {
                    callback(TabDragMovedEventArgs {
                        index,
                        position: event.position,
                        screen_position: event.screen_position,
                    });
                }
            }
            TabGestureKind::Dragging => {
                self.set_tab_gesture(Some(gesture.clone()));
                if let Some(index) = self.item_index_from_weak(&gesture.item) {
                    if let Some(callback) = self.tab_drag_moved_callback() {
                        callback(TabDragMovedEventArgs {
                            index,
                            position: event.position,
                            screen_position: event.screen_position,
                        });
                    }
                }
            }
        }
    }

    fn handle_pointer_released(&self, item: &Rc<CustomTabViewItem>, event: &PointerEventArgs) {
        let Some(gesture) = self.tab_gesture() else {
            return;
        };
        if !gesture
            .item
            .upgrade()
            .is_some_and(|active| Rc::ptr_eq(&active, item))
        {
            return;
        }
        self.set_tab_gesture(None);
        if matches!(gesture.kind, TabGestureKind::Dragging) {
            let index = gesture_index(&gesture, self);
            if let Some(callback) = self.tab_drag_completed_callback() {
                callback(TabDragCompletedEventArgs {
                    index,
                    position: event.position,
                    screen_position: event.screen_position,
                    canceled: false,
                });
            }
        }
    }

    fn handle_pointer_canceled(&self, item: &Rc<CustomTabViewItem>, event: &PointerEventArgs) {
        let Some(gesture) = self.tab_gesture() else {
            return;
        };
        if !gesture
            .item
            .upgrade()
            .is_some_and(|active| Rc::ptr_eq(&active, item))
        {
            return;
        }
        self.set_tab_gesture(None);
        if matches!(gesture.kind, TabGestureKind::Dragging) {
            if let Some(callback) = self.tab_drag_completed_callback() {
                callback(TabDragCompletedEventArgs {
                    index: gesture_index(&gesture, self),
                    position: gesture.last_position,
                    screen_position: gesture.last_screen_position,
                    canceled: true,
                });
            }
        }
        let _ = event;
    }

    fn index_of(&self, item: &CustomTabViewItem) -> Option<usize> {
        self.children_values()
            .iter()
            .position(|candidate| std::ptr::eq(candidate.as_ref(), item))
    }

    fn item_index_from_weak(&self, item: &std::rc::Weak<CustomTabViewItem>) -> Option<usize> {
        item.upgrade().and_then(|item| self.index_of(&item))
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        self.__self_weak
            .borrow()
            .clone()
            .upgrade()
            .and_then(|rc| rc.downcast::<Self>().ok())
            .map(|rc| Rc::downgrade(&rc))
            .unwrap_or_default()
    }
}

fn concrete_tab_item(item: Rc<dyn CustomTabViewItemExt>) -> Rc<CustomTabViewItem> {
    assert!(
        item.as_any().is::<CustomTabViewItem>(),
        "CustomTabView accepts only CustomTabViewItem implementations"
    );
    // `as_any` above verifies the concrete type before the data pointer is recovered. The
    // generated extension trait intentionally erases the Rc for the generic ListExt surface.
    let raw = Rc::into_raw(item) as *const ();
    // SAFETY: the checked Any type is exactly CustomTabViewItem, and the Rc strong count is
    // transferred from the erased pointer without changing ownership.
    unsafe { Rc::from_raw(raw as *const CustomTabViewItem) }
}

impl ListExt<dyn CustomTabViewItemExt> for CustomTabView {
    fn add(&self, item: Rc<dyn CustomTabViewItemExt>) {
        let mut children = self.children_values();
        children.push(concrete_tab_item(item));
        self.set_children(children);
    }

    fn insert(&self, index: usize, item: Rc<dyn CustomTabViewItemExt>) {
        let mut children = self.children_values();
        let index = index.min(children.len());
        children.insert(index, concrete_tab_item(item));
        self.set_children(children);
    }

    fn remove(&self, item: &Rc<dyn CustomTabViewItemExt>) -> bool {
        let mut children = self.children_values();
        let Some(index) = children.iter().position(|candidate| {
            let candidate: Rc<dyn CustomTabViewItemExt> = candidate.clone();
            Rc::ptr_eq(&candidate, item)
        }) else {
            return false;
        };
        children.remove(index);
        self.set_children(children);
        true
    }

    fn remove_at(&self, index: usize) -> Rc<dyn CustomTabViewItemExt> {
        let mut children = self.children_values();
        let item = children.remove(index);
        self.set_children(children);
        item
    }

    fn clear(&self) {
        self.set_children(Vec::new());
    }

    fn len(&self) -> usize {
        self.children_values().len()
    }

    fn is_empty(&self) -> bool {
        self.children_values().is_empty()
    }

    fn to_vec(&self) -> Vec<Rc<dyn CustomTabViewItemExt>> {
        self.children_values()
            .into_iter()
            .map(|item| item as Rc<dyn CustomTabViewItemExt>)
            .collect()
    }
}

impl ListExt<CustomTabViewItem> for CustomTabView {
    fn add(&self, item: Rc<CustomTabViewItem>) {
        let mut children = self.children_values();
        children.push(item);
        self.set_children(children);
    }

    fn insert(&self, index: usize, item: Rc<CustomTabViewItem>) {
        let mut children = self.children_values();
        let index = index.min(children.len());
        children.insert(index, item);
        self.set_children(children);
    }

    fn remove(&self, item: &Rc<CustomTabViewItem>) -> bool {
        let mut children = self.children_values();
        let Some(index) = children
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, item))
        else {
            return false;
        };
        children.remove(index);
        self.set_children(children);
        true
    }

    fn remove_at(&self, index: usize) -> Rc<CustomTabViewItem> {
        let mut children = self.children_values();
        let item = children.remove(index);
        self.set_children(children);
        item
    }

    fn clear(&self) {
        self.set_children(Vec::new());
    }

    fn len(&self) -> usize {
        self.children_values().len()
    }

    fn is_empty(&self) -> bool {
        self.children_values().is_empty()
    }

    fn to_vec(&self) -> Vec<Rc<CustomTabViewItem>> {
        self.children_values()
    }
}

fn gesture_index(gesture: &TabGesture, view: &CustomTabView) -> usize {
    view.item_index_from_weak(&gesture.item).unwrap_or_else(|| {
        view.bound_items()
            .iter()
            .position(|item| {
                item.upgrade().is_some_and(|current| {
                    gesture
                        .item
                        .upgrade()
                        .is_some_and(|target| Rc::ptr_eq(&current, &target))
                })
            })
            .unwrap_or(0)
    })
}

impl CustomSplitter {
    /// Creates a splitter with the default horizontal orientation.
    pub fn new_splitter() -> Rc<Self> {
        Self::new()
    }

    /// Registers the splitter-start callback.
    pub fn set_on_drag_started(&self, callback: Box<dyn Fn(SplitterDragStartedEventArgs)>) {
        self.set_drag_started_callback(Some(Rc::from(callback)));
    }

    /// Registers the incremental splitter-delta callback.
    pub fn set_on_drag_delta(&self, callback: Box<dyn Fn(SplitterDragDeltaEventArgs)>) {
        self.set_drag_delta_callback(Some(Rc::from(callback)));
    }

    /// Registers the splitter-completed callback.
    pub fn set_on_drag_completed(&self, callback: Box<dyn Fn(SplitterDragCompletedEventArgs)>) {
        self.set_drag_completed_callback(Some(Rc::from(callback)));
    }

    fn bind_pointer_handlers(&self) {
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, _| {
                if event.button != Some(MouseButton::Left) {
                    return;
                }
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                splitter.set_gesture(Some(SplitterGesture {
                    orientation: splitter.orientation(),
                    position: event.position,
                    screen_position: event.screen_position,
                    cumulative_delta: 0.0,
                }));
                if let Some(callback) = splitter.drag_started_callback() {
                    callback(SplitterDragStarted {
                        position: event.position,
                        screen_position: event.screen_position,
                    });
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |event, _| {
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                let Some(mut gesture) = splitter.gesture() else {
                    return;
                };
                let delta = match gesture.orientation {
                    Orientation::Horizontal => event.position.x - gesture.position.x,
                    Orientation::Vertical => event.position.y - gesture.position.y,
                };
                gesture.position = event.position;
                gesture.screen_position = event.screen_position;
                gesture.cumulative_delta += delta;
                splitter.set_gesture(Some(gesture.clone()));
                if delta == 0.0 {
                    return;
                }
                if let Some(callback) = splitter.drag_delta_callback() {
                    callback(SplitterDragDelta {
                        delta,
                        cumulative_delta: gesture.cumulative_delta,
                        position: event.position,
                        screen_position: event.screen_position,
                    });
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, _| {
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                let Some(mut gesture) = splitter.gesture() else {
                    return;
                };
                let final_delta = match gesture.orientation {
                    Orientation::Horizontal => event.position.x - gesture.position.x,
                    Orientation::Vertical => event.position.y - gesture.position.y,
                };
                gesture.position = event.position;
                gesture.screen_position = event.screen_position;
                gesture.cumulative_delta += final_delta;
                splitter.set_gesture(None);
                if let Some(callback) = splitter.drag_completed_callback() {
                    callback(SplitterDragCompletedEventArgs {
                        cumulative_delta: gesture.cumulative_delta,
                        position: gesture.position,
                        screen_position: gesture.screen_position,
                        canceled: false,
                    });
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |_, _| {
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                let Some(gesture) = splitter.gesture() else {
                    return;
                };
                splitter.set_gesture(None);
                if let Some(callback) = splitter.drag_completed_callback() {
                    callback(SplitterDragCompletedEventArgs {
                        cumulative_delta: gesture.cumulative_delta,
                        position: gesture.position,
                        screen_position: gesture.screen_position,
                        canceled: true,
                    });
                }
            }),
        );
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        self.__self_weak
            .borrow()
            .clone()
            .upgrade()
            .and_then(|rc| rc.downcast::<Self>().ok())
            .map(|rc| Rc::downgrade(&rc))
            .unwrap_or_default()
    }
}
