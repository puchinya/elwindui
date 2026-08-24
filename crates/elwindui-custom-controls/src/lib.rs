//! Reusable custom-rendered controls shared by Docking and application code.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

extern crate self as elwindui;

pub use elwindui_core as core;
pub use elwindui_core::ui;
pub use elwindui_macros::{class, component};

use core::base::{Point, Rect, Size};
use core::graphics::{Brush, IconSource, RenderContext, StrokeStyle};
use core::input::{MouseButton, PointerEventArgs};
pub use core::layout::Orientation;
use core::ui::{ControlExt, IconSourceElementExt, ListExt, TextBlockExt, UIElementExt};
use std::rc::Rc;

const TAB_STRIP_HEIGHT: f32 = 32.0;
const TAB_HORIZONTAL_PADDING: f32 = 10.0;
const TAB_ELEMENT_GAP: f32 = 6.0;
const TAB_ICON_SIZE: f32 = 16.0;
const TAB_CLOSE_SLOT: f32 = 20.0;
const TAB_CLOSE_GLYPH: f32 = 10.0;
const TAB_DRAG_THRESHOLD: f32 = 4.0;
const SELECTED_INDICATOR_THICKNESS: f32 = 2.0;
const SPLITTER_THICKNESS: f32 = 6.0;

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
    Close { rect: Rect },
}

#[derive(Clone, Debug)]
struct TabGesture {
    item: std::rc::Weak<CustomTabViewItem>,
    press_position: Point,
    press_screen_position: Option<Point>,
    last_position: Point,
    last_screen_position: Option<Point>,
    kind: TabGestureKind,
}

#[derive(Clone)]
struct TabChrome {
    item: std::rc::Weak<CustomTabViewItem>,
    header: Rc<core::ui::TextBlock>,
    icon: Option<Rc<core::ui::IconSourceElement>>,
    header_rect: Rect,
    close_rect: Option<Rect>,
}

#[derive(Clone, Debug)]
struct SplitterGesture {
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

/// One item displayed by [`CustomTabView`].
#[elwindui::component(inherits ContentControl)]
pub struct CustomTabViewItem {
    #[prop(default = String::new())]
    header: String,
    #[prop(default = None)]
    icon: Option<IconSource>,
    #[prop(default = true)]
    closable: bool,
    #[state(default = None)]
    owner_chrome_callback: Option<Rc<dyn Fn()>>,
    body: view! {
        on_update(header, icon, closable) {
            this.notify_owner_chrome_changed();
        }
    },
}

#[elwindui::component]
impl CustomTabViewItem {}

/// A self-drawn tab strip and selected-content host.
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
    #[state(default = Vec::new())]
    chrome: Vec<TabChrome>,
    #[state(default = None)]
    tab_gesture: Option<TabGesture>,
    #[state(default = false)]
    root_handlers_bound: bool,
    #[state(default = None)]
    hovered_index: Option<usize>,
    body: view! {
        on_mount {
            this.set_clip_to_bounds(Some(true));
            this.reconcile_children();
            this.bind_root_handlers();
        }
        on_update(children, selected_index, tab_strip_position, close_button_presentation) {
            this.reconcile_children();
            this.invalidate_measure();
            this.invalidate_render();
        }
        // The transparent shape keeps the self-drawn surface hit-testable even though the
        // inherited Control hit-test hook cannot currently be overridden by a composed
        // `#[component]` (see the C-class limitation in the status document).  It is renderer
        // chrome only; real item/content ownership remains below this node.
        Rectangle {
            fill: "#00000000"
        }
    },
}

#[elwindui::component]
impl CustomTabView {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        true
    }

    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        self.reconcile_children();
        let content_available = Size {
            width: available.width,
            height: (available.height - TAB_STRIP_HEIGHT).max(0.0),
        };
        let mut header_width = 0.0;
        let mut header_height = TAB_STRIP_HEIGHT;
        let presentation = self.close_button_presentation();
        for record in self.chrome() {
            record.header.measure(Size {
                width: available.width.max(0.0),
                height: TAB_STRIP_HEIGHT,
            });
            let label_size = record.header.measured_size().unwrap_or_default();
            let icon_size = record
                .icon
                .as_ref()
                .map(|icon| {
                    icon.measure(Size {
                        width: TAB_ICON_SIZE,
                        height: TAB_ICON_SIZE,
                    });
                    icon.measured_size().unwrap_or_default()
                })
                .unwrap_or_default();
            let item = record.item.upgrade();
            let has_icon = item.as_ref().and_then(|item| item.icon()).is_some();
            let close_width = item
                .as_ref()
                .is_some_and(|item| {
                    item.closable() && presentation != CloseButtonPresentation::Never
                })
                .then_some(TAB_CLOSE_SLOT)
                .unwrap_or(0.0);
            let icon_width = if has_icon {
                TAB_ICON_SIZE.max(icon_size.width)
            } else {
                0.0
            };
            let gap = if has_icon { TAB_ELEMENT_GAP } else { 0.0 }
                + if close_width > 0.0 {
                    TAB_ELEMENT_GAP
                } else {
                    0.0
                };
            header_width +=
                TAB_HORIZONTAL_PADDING * 2.0 + icon_width + gap + label_size.width + close_width;
            header_height = header_height.max(label_size.height.max(icon_size.height));
        }

        let children = self.children_values();
        for child in &children {
            child.measure(content_available);
        }
        let content_size = self
            .selected_item()
            .and_then(|item| item.measured_size())
            .unwrap_or_default();
        Size {
            width: header_width.max(content_size.width),
            height: header_height.max(TAB_STRIP_HEIGHT) + content_size.height,
        }
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        self.reconcile_children();
        let strip_y = match self.tab_strip_position() {
            TabStripPosition::Top => 0.0,
            TabStripPosition::Bottom => (final_size.height - TAB_STRIP_HEIGHT).max(0.0),
        };
        let content_y = match self.tab_strip_position() {
            TabStripPosition::Top => TAB_STRIP_HEIGHT,
            TabStripPosition::Bottom => 0.0,
        };
        let content_rect = Rect {
            x: 0.0,
            y: content_y,
            width: final_size.width.max(0.0),
            height: (final_size.height - TAB_STRIP_HEIGHT).max(0.0),
        };

        let presentation = self.close_button_presentation();
        let mut x = 0.0;
        let mut chrome = self.chrome();
        for record in &mut chrome {
            let item = record.item.upgrade();
            let label_size = record.header.measured_size().unwrap_or_default();
            let icon_size = record
                .icon
                .as_ref()
                .and_then(|icon| icon.measured_size())
                .unwrap_or_default();
            let has_icon = item.as_ref().and_then(|item| item.icon()).is_some();
            let close_visible = item.as_ref().is_some_and(|item| {
                item.closable() && presentation != CloseButtonPresentation::Never
            });
            let icon_width = if has_icon {
                TAB_ICON_SIZE.max(icon_size.width)
            } else {
                0.0
            };
            let close_width = close_visible.then_some(TAB_CLOSE_SLOT).unwrap_or(0.0);
            let gap_before_label = if has_icon { TAB_ELEMENT_GAP } else { 0.0 };
            let gap_before_close = if close_width > 0.0 {
                TAB_ELEMENT_GAP
            } else {
                0.0
            };
            let width = TAB_HORIZONTAL_PADDING * 2.0
                + icon_width
                + gap_before_label
                + label_size.width
                + gap_before_close
                + close_width;
            let header_rect = Rect {
                x,
                y: strip_y,
                width,
                height: TAB_STRIP_HEIGHT.min(final_size.height.max(0.0)),
            };
            record.header_rect = header_rect;
            let mut child_x = x + TAB_HORIZONTAL_PADDING;
            if let Some(icon) = &record.icon {
                if has_icon {
                    icon.arrange(Rect {
                        x: child_x,
                        y: strip_y + (TAB_STRIP_HEIGHT - TAB_ICON_SIZE).max(0.0) * 0.5,
                        width: TAB_ICON_SIZE,
                        height: TAB_ICON_SIZE,
                    });
                    child_x += icon_width + TAB_ELEMENT_GAP;
                } else {
                    icon.arrange(Rect {
                        x: child_x,
                        y: strip_y,
                        width: 0.0,
                        height: 0.0,
                    });
                }
            }
            record.header.arrange(Rect {
                x: child_x,
                y: strip_y + (TAB_STRIP_HEIGHT - label_size.height).max(0.0) * 0.5,
                width: label_size.width,
                height: label_size.height,
            });
            record.close_rect = close_visible.then_some(Rect {
                x: x + width - TAB_HORIZONTAL_PADDING - TAB_CLOSE_SLOT,
                y: strip_y,
                width: TAB_CLOSE_SLOT,
                height: TAB_STRIP_HEIGHT.min(final_size.height.max(0.0)),
            });
            x += width;
        }
        self.set_chrome(chrome);

        let selected = self.selected_index();
        for (index, item) in self.children_values().into_iter().enumerate() {
            let rect = (index == selected).then_some(content_rect).unwrap_or(Rect {
                x: 0.0,
                y: content_y,
                width: 0.0,
                height: 0.0,
            });
            item.set_clip_to_bounds(Some(true));
            item.arrange(rect);
        }
        if let Some(root) = self.__template_root() {
            root.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width: final_size.width.max(0.0),
                height: final_size.height.max(0.0),
            });
        }
        final_size
    }

    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let width = self.arranged_width().unwrap_or(0.0).max(0.0);
        let height = self.arranged_height().unwrap_or(0.0).max(0.0);
        let count = self.children_values().len();
        let strip_height = TAB_STRIP_HEIGHT.min(height);
        let strip_y = match self.tab_strip_position() {
            TabStripPosition::Top => 0.0,
            TabStripPosition::Bottom => (height - strip_height).max(0.0),
        };
        let strip_brush: Brush = "#f2f2f2".into();
        context.fill_rect(
            Rect {
                x: 0.0,
                y: strip_y,
                width,
                height: strip_height,
            },
            &strip_brush,
        );
        if count == 0 {
            return;
        }
        let selected = self.selected_index();
        let selected_brush = self
            .as_ui_element()
            .as_text_style_owner()
            .map(|owner| owner.resolved_text_style().foreground)
            .unwrap_or_else(|| core::ui::inherited_text_style(self.as_ui_element()).foreground);
        let selected_x = self
            .chrome()
            .get(selected)
            .map(|record| record.header_rect.x)
            .unwrap_or(0.0);
        let selected_width = self
            .chrome()
            .get(selected)
            .map(|record| record.header_rect.width)
            .unwrap_or(0.0);
        let underline_y = match self.tab_strip_position() {
            TabStripPosition::Top => strip_height - SELECTED_INDICATOR_THICKNESS,
            TabStripPosition::Bottom => strip_y,
        };
        context.fill_rect(
            Rect {
                x: selected_x,
                y: underline_y,
                width: selected_width,
                height: SELECTED_INDICATOR_THICKNESS,
            },
            &selected_brush,
        );

        let stroke = StrokeStyle {
            width: 1.0,
            ..Default::default()
        };
        let close_brush = self
            .as_ui_element()
            .as_text_style_owner()
            .map(|owner| owner.resolved_text_style().foreground)
            .unwrap_or_else(|| core::ui::inherited_text_style(self.as_ui_element()).foreground);
        for (index, record) in self.chrome().into_iter().enumerate() {
            let Some(close_rect) = record.close_rect else {
                continue;
            };
            let active_close = self.hovered_index() == Some(index)
                || matches!(self.tab_gesture(), Some(TabGesture { kind: TabGestureKind::Close { .. }, ref item, .. }) if item.upgrade().is_some_and(|active| record.item.upgrade().is_some_and(|record_item| Rc::ptr_eq(&active, &record_item))));
            let visible = match self.close_button_presentation() {
                CloseButtonPresentation::Always => true,
                CloseButtonPresentation::OnPointerOver => active_close,
                CloseButtonPresentation::Never => false,
            };
            if !visible {
                continue;
            }
            let center = Point {
                x: close_rect.x + close_rect.width * 0.5,
                y: close_rect.y + close_rect.height * 0.5,
            };
            let half = TAB_CLOSE_GLYPH * 0.5;
            context.draw_line(
                Point {
                    x: center.x - half,
                    y: center.y - half,
                },
                Point {
                    x: center.x + half,
                    y: center.y + half,
                },
                &close_brush,
                &stroke,
            );
            context.draw_line(
                Point {
                    x: center.x + half,
                    y: center.y - half,
                },
                Point {
                    x: center.x - half,
                    y: center.y + half,
                },
                &close_brush,
                &stroke,
            );
        }
    }
}

/// A self-drawn splitter that reports logical-axis drag deltas.
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
    body: view! {
        on_mount {
            this.bind_pointer_handlers();
        }
        on_update(orientation) {
            this.invalidate_measure();
            this.invalidate_render();
        }
        Rectangle {
            fill: "#d0d0d0"
        }
    },
}

#[elwindui::component]
impl CustomSplitter {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        true
    }

    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let desired = match self.orientation() {
            Orientation::Horizontal => Size {
                width: SPLITTER_THICKNESS,
                height: 0.0,
            },
            Orientation::Vertical => Size {
                width: 0.0,
                height: SPLITTER_THICKNESS,
            },
        };
        for child in self.visual_children() {
            child.measure(available);
        }
        desired
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        for child in self.visual_children() {
            child.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width: final_size.width.max(0.0),
                height: final_size.height.max(0.0),
            });
        }
        final_size
    }

    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let foreground = self
            .as_ui_element()
            .as_text_style_owner()
            .map(|owner| owner.resolved_text_style().foreground)
            .unwrap_or_else(|| core::ui::inherited_text_style(self.as_ui_element()).foreground);
        let stroke = StrokeStyle {
            width: 1.0,
            ..Default::default()
        };
        let width = self.arranged_width().unwrap_or(0.0).max(0.0);
        let height = self.arranged_height().unwrap_or(0.0).max(0.0);
        match self.orientation() {
            Orientation::Horizontal => {
                let x = width * 0.5;
                context.draw_line(
                    Point { x, y: 0.0 },
                    Point { x, y: height },
                    &foreground,
                    &stroke,
                );
            }
            Orientation::Vertical => {
                let y = height * 0.5;
                context.draw_line(
                    Point { x: 0.0, y },
                    Point { x: width, y },
                    &foreground,
                    &stroke,
                );
            }
        }
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

    /// Updates the tab label only when its value changes.  The generated property setter remains
    /// the storage/notification path; this guard keeps equal assignments from rebuilding private
    /// chrome or invalidating the owning tab.
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

    /// Installs the tab-view-owned metadata callback. This is crate-private because the callback
    /// is only a weak presentation back-link used to refresh private tab chrome.
    fn set_owner_chrome_changed(&self, callback: Option<Box<dyn Fn()>>) {
        self.set_owner_chrome_callback(callback.map(Rc::from));
    }

    fn notify_owner_chrome_changed(&self) {
        if let Some(callback) = self.owner_chrome_callback() {
            callback();
        }
    }

    /// Resolves the icon into the Core `IconSourceElement` realization used by custom chrome.
    pub fn realize_icon(&self) -> Option<Rc<dyn UIElementExt>> {
        self.icon().map(|icon_source| {
            let icon = core::ui::IconSourceElement::new();
            icon.set_icon_source(Some(icon_source));
            icon as Rc<dyn UIElementExt>
        })
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

    /// Replaces the ordered tab list and reconciles its Visual ownership.
    pub fn replace_children(&self, children: Vec<Rc<CustomTabViewItem>>) {
        self.set_children(children);
    }

    /// Returns the established typed ordered-list surface for tab items.
    pub fn children(&self) -> &dyn ListExt<dyn CustomTabViewItemExt> {
        self
    }

    /// Replaces the concrete list used by declarative and programmatic callers.
    pub fn set_children(&self, children: Vec<Rc<CustomTabViewItem>>) {
        <Self as CustomTabViewExt>::set_children(self, children);
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
    /// Returns the concrete list used internally by the component implementation.
    fn children_values(&self) -> Vec<Rc<CustomTabViewItem>> {
        <Self as CustomTabViewExt>::children(self)
    }

    /// Returns the typed ordered-list surface used by dynamic content composition.
    pub fn children_list(&self) -> &dyn ListExt<CustomTabViewItem> {
        self
    }
    fn selected_item(&self) -> Option<Rc<CustomTabViewItem>> {
        self.children_values().get(self.selected_index()).cloned()
    }

    fn reconcile_children(&self) {
        // Standalone Core tests can mutate a newly constructed component before a host sends its
        // normal mount notification. Binding here is idempotent and keeps the control's routed
        // behavior independent of native host timing.
        self.bind_root_handlers();
        let children = self.children_values();
        self.validate_children(&children);
        let unchanged = self.bound_items().len() == children.len()
            && self
                .bound_items()
                .iter()
                .zip(children.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)));
        if unchanged {
            return;
        }

        if self.cancel_removed_gesture(&children) {
            // The completion callback is external and may have replaced `children` reentrantly.
            // Do not continue with this stale snapshot: restart from the current authoritative
            // property state after the gesture has already been cleared.
            self.reconcile_children();
            return;
        }
        let old_items = self.bound_items();
        for old in old_items.into_iter().filter_map(|item| item.upgrade()) {
            old.set_owner_chrome_changed(None);
            let old: Rc<dyn UIElementExt> = old;
            self.as_ui_element().visual_collection.remove(&old);
        }
        for old in self.chrome() {
            let header: Rc<dyn UIElementExt> = old.header;
            self.as_ui_element().visual_collection.remove(&header);
            if let Some(icon) = old.icon {
                let icon: Rc<dyn UIElementExt> = icon;
                self.as_ui_element().visual_collection.remove(&icon);
            }
        }

        let weak_view = self.weak_self();
        let mut chrome = Vec::with_capacity(children.len());
        for item in &children {
            item.set_clip_to_bounds(Some(true));
            let weak_item = Rc::downgrade(item);
            let weak_callback_view = weak_view.clone();
            item.set_owner_chrome_changed(Some(Box::new(move || {
                if let (Some(view), Some(item)) =
                    (weak_callback_view.upgrade(), weak_item.upgrade())
                {
                    view.refresh_item_chrome(&item);
                }
            })));
            let item_visual: Rc<dyn UIElementExt> = item.clone();
            self.as_ui_element().visual_collection.add(item_visual);

            let header = core::ui::TextBlock::new();
            header.set_text(&item.header());
            header.set_hit_test_visible(false);
            self.as_ui_element()
                .visual_collection
                .add(header.clone() as Rc<dyn UIElementExt>);

            let icon = item.icon().map(|source| {
                let icon = core::ui::IconSourceElement::new();
                icon.set_icon_source(Some(source));
                icon.set_hit_test_visible(false);
                self.as_ui_element()
                    .visual_collection
                    .add(icon.clone() as Rc<dyn UIElementExt>);
                icon
            });
            chrome.push(TabChrome {
                item: Rc::downgrade(item),
                header,
                icon,
                header_rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                },
                close_rect: None,
            });
        }
        self.set_bound_items(children.iter().map(Rc::downgrade).collect());
        self.set_chrome(chrome);
        self.invalidate_measure();
    }

    fn validate_children(&self, children: &[Rc<CustomTabViewItem>]) {
        let owner = self.weak_self().upgrade().map(|owner| {
            let owner: Rc<dyn UIElementExt> = owner;
            owner
        });
        for (index, child) in children.iter().enumerate() {
            assert!(
                !children[..index]
                    .iter()
                    .any(|previous| Rc::ptr_eq(previous, child)),
                "CustomTabView cannot attach the same CustomTabViewItem twice; detach the duplicate first"
            );
            if let Some(parent) = child.visual_parent() {
                let owned_by_self = owner
                    .as_ref()
                    .is_some_and(|owner| Rc::ptr_eq(&parent, owner));
                assert!(
                    owned_by_self,
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

    fn refresh_item_chrome(&self, item: &CustomTabViewItem) {
        let Some(index) = self.index_of(item) else {
            return;
        };
        let mut chrome = self.chrome();
        let Some(record) = chrome.get_mut(index) else {
            return;
        };
        record.header.set_text(&item.header());
        if let Some(old_icon) = record.icon.take() {
            let old_icon: Rc<dyn UIElementExt> = old_icon;
            self.as_ui_element().visual_collection.remove(&old_icon);
        }
        record.icon = item.icon().map(|source| {
            let icon = core::ui::IconSourceElement::new();
            icon.set_icon_source(Some(source));
            icon.set_hit_test_visible(false);
            self.as_ui_element()
                .visual_collection
                .add(icon.clone() as Rc<dyn UIElementExt>);
            icon
        });
        self.set_chrome(chrome);
        self.invalidate_measure();
        self.invalidate_render();
    }

    fn bind_root_handlers(&self) {
        if self.root_handlers_bound() {
            return;
        }
        let weak_view = self.weak_self();
        if weak_view.upgrade().is_none() {
            // Component property subscriptions can run while `Rc::new_cyclic` is still building
            // the object. Defer registration until the first post-construction reconciliation.
            return;
        }
        self.set_root_handlers_bound(true);
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, _| {
                if let Some(view) = weak_view.upgrade() {
                    view.handle_pointer_pressed(event);
                }
            }),
        );
        let weak_view = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |event, _| {
                if let Some(view) = weak_view.upgrade() {
                    view.handle_pointer_moved(event);
                }
            }),
        );
        let weak_view = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, _| {
                if let Some(view) = weak_view.upgrade() {
                    view.handle_pointer_released(event);
                }
            }),
        );
        let weak_view = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |event, _| {
                if let Some(view) = weak_view.upgrade() {
                    view.handle_pointer_canceled(event);
                }
            }),
        );
        let weak_view = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_entered",
            Box::new(move |event, _| {
                if let Some(view) = weak_view.upgrade() {
                    view.update_hover(event.position);
                }
            }),
        );
        let weak_view = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_exited",
            Box::new(move |_, _| {
                if let Some(view) = weak_view.upgrade() {
                    view.update_hovered_index(None);
                }
            }),
        );
    }

    fn handle_pointer_pressed(&self, event: &PointerEventArgs) {
        if event.button != Some(MouseButton::Left) {
            return;
        }
        let point = self.root_local_position(event.position);
        self.update_hover(event.position);
        if let Some((index, rect)) = self.close_at(point) {
            let children = self.children_values();
            let Some(item) = children.get(index) else {
                return;
            };
            self.set_tab_gesture(Some(TabGesture {
                item: Rc::downgrade(item),
                press_position: event.position,
                press_screen_position: event.screen_position,
                last_position: event.position,
                last_screen_position: event.screen_position,
                kind: TabGestureKind::Close { rect },
            }));
            return;
        }
        let Some(index) = self.header_at(point) else {
            return;
        };
        let children = self.children_values();
        let Some(item) = children.get(index) else {
            return;
        };
        self.set_tab_gesture(Some(TabGesture {
            item: Rc::downgrade(item),
            press_position: event.position,
            press_screen_position: event.screen_position,
            last_position: event.position,
            last_screen_position: event.screen_position,
            kind: TabGestureKind::Pressed,
        }));
        let _ = self.select_index(index);
    }

    fn handle_pointer_moved(&self, event: &PointerEventArgs) {
        self.update_hover(event.position);
        let Some(mut gesture) = self.tab_gesture() else {
            return;
        };
        gesture.last_position = event.position;
        gesture.last_screen_position = event.screen_position;
        match gesture.kind.clone() {
            TabGestureKind::Close { .. } => self.set_tab_gesture(Some(gesture)),
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

                // The started callback is external and may remove or reorder the item. Re-read
                // the gesture and resolve its index by identity before emitting the first move.
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

    fn handle_pointer_released(&self, event: &PointerEventArgs) {
        let Some(gesture) = self.tab_gesture() else {
            return;
        };
        self.set_tab_gesture(None);
        let point = self.root_local_position(event.position);
        match gesture.kind {
            TabGestureKind::Close { rect } => {
                let Some(index) = self.item_index_from_weak(&gesture.item) else {
                    return;
                };
                let visible = self.is_close_visible(index);
                if visible
                    && rect_contains(rect, point)
                    && self
                        .children_values()
                        .get(index)
                        .is_some_and(|item| item.closable())
                    && let Some(callback) = self.close_requested_callback()
                {
                    callback(index);
                }
            }
            TabGestureKind::Pressed => {}
            TabGestureKind::Dragging => {
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
    }

    fn handle_pointer_canceled(&self, event: &PointerEventArgs) {
        let Some(gesture) = self.tab_gesture() else {
            return;
        };
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

    fn update_hover(&self, position: Point) {
        let point = self.root_local_position(position);
        self.update_hovered_index(self.header_at(point));
    }

    fn update_hovered_index(&self, hovered: Option<usize>) {
        if self.hovered_index() == hovered {
            return;
        }
        self.set_hovered_index(hovered);
        self.invalidate_render();
    }

    fn root_local_position(&self, position: Point) -> Point {
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
        Point {
            x: position.x - offset.x,
            y: position.y - offset.y,
        }
    }

    fn header_at(&self, point: Point) -> Option<usize> {
        let chrome = self.chrome();
        if let Some(index) = chrome
            .iter()
            .position(|record| rect_contains(record.header_rect, point))
        {
            return Some(index);
        }
        // Before the first host layout pass there are no cached header rectangles. A small,
        // deterministic fallback keeps Core-only routed-event tests useful and is replaced by
        // the measured rectangles as soon as arrange runs.
        if chrome.iter().all(|record| record.header_rect.width == 0.0)
            && point.x >= 0.0
            && point.y >= 0.0
            && self.children_values().len() == 1
        {
            return Some(0);
        }
        None
    }

    fn close_at(&self, point: Point) -> Option<(usize, Rect)> {
        self.chrome()
            .iter()
            .enumerate()
            .find_map(|(index, record)| {
                let rect = record.close_rect?;
                (self.is_close_visible(index) && rect_contains(rect, point))
                    .then_some((index, rect))
            })
    }

    fn is_close_visible(&self, index: usize) -> bool {
        match self.close_button_presentation() {
            CloseButtonPresentation::Always => true,
            CloseButtonPresentation::OnPointerOver => self.hovered_index() == Some(index),
            CloseButtonPresentation::Never => false,
        }
    }

    fn index_of(&self, item: &CustomTabViewItem) -> Option<usize> {
        self.children_values().iter().position(|candidate| {
            let candidate: &CustomTabViewItem = candidate;
            std::ptr::eq(candidate, item)
        })
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

fn rect_contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
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
