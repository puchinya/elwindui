use super::core::base::Point;
use super::core::input::{MouseButton, PointerEventArgs};
use super::core::ui::{ControlExt, ListExt, UIElementExt};
use super::{
    CloseButtonPresentation, CustomTabContentPresenter, CustomTabContentPresenterExt,
    CustomTabStripPresenter, CustomTabStripPresenterExt, CustomTabViewItem, CustomTabViewItemExt,
    TabCloseRequested, TabDragCompletedEventArgs, TabDragMovedEventArgs, TabDragStartedEventArgs,
    TabStripPosition, weak_self_from_visual_owner,
};
use std::rc::Rc;

const TAB_STRIP_HEIGHT: f32 = 32.0;
const TAB_DRAG_THRESHOLD: f32 = 4.0;

#[derive(Clone, Debug)]
// This module is private; `pub` is required only so the component macro can
// name the state type in generated methods. The type remains unreachable from
// outside this crate because `custom_tab_view` is not exported.
pub enum TabGestureKind {
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

/// A templated tab strip and selected-content host.
#[elwindui::component(inherits Control)]
#[content(children)]
pub struct CustomTabView {
    #[prop(default = Vec::new())]
    children: Vec<Rc<CustomTabViewItem>>,
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
    template: template_view!(|this: Self| {
        on_update(children, template_items, selected_index, tab_strip_position, close_button_presentation) {
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
    }),
}

#[elwindui::component]
impl CustomTabView {
    #[overrides]
    fn on_apply_template(&self) {
        self.set_clip_to_bounds(Some(true));
        self.reconcile_children();
    }
}

/// A templated splitter that reports logical-axis drag deltas.

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
        self.set_children_internal(children);
    }

    /// Returns the established typed ordered-list surface for tab items.
    #[cfg(not(rust_analyzer))]
    pub fn children(&self) -> &dyn ListExt<dyn CustomTabViewItemExt> {
        self
    }

    /// Replaces the concrete list used by declarative and programmatic callers.
    #[cfg(not(rust_analyzer))]
    pub fn set_children(&self, children: Vec<Rc<CustomTabViewItem>>) {
        self.set_children_internal(children);
    }

    fn set_children_internal(&self, children: Vec<Rc<CustomTabViewItem>>) {
        #[cfg(rust_analyzer)]
        self.set_children(children);
        #[cfg(not(rust_analyzer))]
        <Self as CustomTabViewExt>::set_children(self, children);
    }

    /// Appends one item to the ordered tab list.
    pub fn append_child(&self, item: Rc<CustomTabViewItem>) {
        let mut children = self.children_values();
        children.push(item);
        self.set_children_internal(children);
    }

    /// Removes and returns an item by index, if present.
    pub fn remove_child(&self, index: usize) -> Option<Rc<CustomTabViewItem>> {
        let mut children = self.children_values();
        if index >= children.len() {
            return None;
        }
        let removed = children.remove(index);
        self.set_children_internal(children);
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
        #[cfg(rust_analyzer)]
        {
            self.children()
        }
        #[cfg(not(rust_analyzer))]
        {
            <Self as CustomTabViewExt>::children(self)
        }
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
                .all(|(old, new)| {
                    let old: Option<Rc<CustomTabViewItem>> = old.upgrade();
                    old.is_some_and(|old| Rc::ptr_eq(&old, new))
                });
        if unchanged {
            self.sync_presenters(&children);
            return;
        }

        if self.cancel_removed_gesture(&children) {
            self.reconcile_children();
            return;
        }

        let old_items: Vec<Rc<CustomTabViewItem>> = self
            .bound_items()
            .into_iter()
            .filter_map(|item: std::rc::Weak<CustomTabViewItem>| item.upgrade())
            .collect();
        for old in old_items {
            old.set_owner_pointer_handler(None);
            old.set_owner_close_handler(None);
        }

        let weak_view: std::rc::Weak<CustomTabView> = self.weak_self();
        for item in &children {
            let weak_item: std::rc::Weak<CustomTabViewItem> = Rc::downgrade(item);
            let weak_view_for_pointer = weak_view.clone();
            item.set_owner_pointer_handler(Some(Box::new(move |event| {
                let view: Option<Rc<CustomTabView>> = weak_view_for_pointer.upgrade();
                let item: Option<Rc<CustomTabViewItem>> = weak_item.upgrade();
                if let (Some(view), Some(item)) = (view, item) {
                    view.handle_item_pointer(&item, event);
                }
            })));
            let weak_item: std::rc::Weak<CustomTabViewItem> = Rc::downgrade(item);
            let weak_view_for_close = weak_view.clone();
            item.set_owner_close_handler(Some(Box::new(move || {
                let view: Option<Rc<CustomTabView>> = weak_view_for_close.upgrade();
                let item: Option<Rc<CustomTabViewItem>> = weak_item.upgrade();
                if let (Some(view), Some(item)) = (view, item) {
                    if let Some(index) = view.index_of(&item) {
                        view.request_close(index);
                    }
                }
            })));
        }

        self.set_bound_items(
            children
                .iter()
                .map(|item: &Rc<CustomTabViewItem>| Rc::downgrade(item))
                .collect(),
        );
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

        for node in super::core::visual_tree::find_all::<CustomTabStripPresenter>(self) {
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
            presenter.reconcile_items();
            break;
        }
        for node in super::core::visual_tree::find_all::<CustomTabContentPresenter>(self) {
            let Some(presenter) = node.as_any().downcast_ref::<CustomTabContentPresenter>() else {
                continue;
            };
            presenter.set_items(children.to_vec());
            presenter.set_selected_index(selected);
            presenter
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.content_row());
            presenter.reconcile_contents();
            break;
        }
        for (index, item) in children.iter().enumerate() {
            item.set_presentation(index == selected, item.pointer_over(), position, close);
        }
    }

    fn validate_children(&self, children: &[Rc<CustomTabViewItem>]) {
        let owner = super::core::visual_tree::find_all::<CustomTabStripPresenter>(self)
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
        let gesture_item: Option<Rc<CustomTabViewItem>> = gesture.item.upgrade();
        let still_present = gesture_item.is_some_and(|item| {
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
                let current_item: Option<Rc<CustomTabViewItem>> = current.item.upgrade();
                let original_item: Option<Rc<CustomTabViewItem>> = gesture_item.upgrade();
                let same_item = current_item
                    .zip(original_item)
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
        weak_self_from_visual_owner(self)
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
        self.set_children_internal(children);
    }

    fn insert(&self, index: usize, item: Rc<dyn CustomTabViewItemExt>) {
        let mut children = self.children_values();
        let index = index.min(children.len());
        children.insert(index, concrete_tab_item(item));
        self.set_children_internal(children);
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
        self.set_children_internal(children);
        true
    }

    fn remove_at(&self, index: usize) -> Rc<dyn CustomTabViewItemExt> {
        let mut children = self.children_values();
        let item = children.remove(index);
        self.set_children_internal(children);
        item
    }

    fn clear(&self) {
        self.set_children_internal(Vec::new());
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

fn gesture_index(gesture: &TabGesture, view: &CustomTabView) -> usize {
    view.item_index_from_weak(&gesture.item).unwrap_or_else(|| {
        view.bound_items()
            .iter()
            .position(|item| {
                item.upgrade()
                    .is_some_and(|current: Rc<CustomTabViewItem>| {
                        let target: Option<Rc<CustomTabViewItem>> = gesture.item.upgrade();
                        target.is_some_and(|target| Rc::ptr_eq(&current, &target))
                    })
            })
            .unwrap_or(0)
    })
}
