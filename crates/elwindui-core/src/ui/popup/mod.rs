//! Context menu, custom popup, and PopupSurface abstractions.
//!
//! See `docs/specs/ui_spec.md` and `docs/design/runtime/popup_context_menu_design.md`.

use crate::base::{Point, Rect, Size};
use crate::environment::EnvironmentContext;
use crate::focus::FocusTracker;
use crate::ui::{LayoutExt, MenuExt, TextBlockExt, TextStyleOwner, UIElementExt};
use std::cell::RefCell;
use std::rc::Rc;

/// The presentation mode for a standard [`crate::ui::Menu`]-backed context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextMenuPresentation {
    /// Platform-native popup menu (e.g. `NSMenu` on macOS, `MenuFlyout` on Windows).
    #[default]
    Native,
    /// ElwindUI custom-rendered popup menu on a [`PopupSurfaceHandle`].
    Custom,
}

/// The origin of a platform-neutral [`ContextRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextRequestSource {
    /// A pointer-based context action (e.g. secondary click, Control+click).
    Pointer,
    /// A keyboard-based context action (e.g. Shift+F10, Menu/Application key).
    Keyboard,
    /// An accessibility/assistive-technology context action.
    Accessibility,
    /// Programmatic or other context action.
    Other,
}

/// A platform-neutral description of a user or system request to open a context menu.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextRequest {
    /// The input source that generated the request.
    pub source: ContextRequestSource,
    /// TreeHost-local logical coordinate used for hit-testing the target element.
    pub local_position: Option<Point>,
    /// Desktop screen anchor (Point or Rect in desktop logical coordinates) used for popup placement.
    pub screen_anchor: Option<PopupAnchor>,
}

impl ContextRequest {
    /// Creates a pointer-driven context request with local hit-test position and screen anchor position.
    pub fn pointer(local_position: Point, screen_position: Point) -> Self {
        Self {
            source: ContextRequestSource::Pointer,
            local_position: Some(local_position),
            screen_anchor: Some(PopupAnchor::Point(screen_position)),
        }
    }

    /// Creates a keyboard-driven context request (targeting the focused element) with optional screen anchor.
    pub fn keyboard(screen_anchor: Option<PopupAnchor>) -> Self {
        Self {
            source: ContextRequestSource::Keyboard,
            local_position: None,
            screen_anchor,
        }
    }

    /// Creates an accessibility-driven context request with optional screen anchor.
    pub fn accessibility(local_position: Option<Point>, screen_anchor: Option<PopupAnchor>) -> Self {
        Self {
            source: ContextRequestSource::Accessibility,
            local_position,
            screen_anchor,
        }
    }
}

/// Context supplied to a [`PopupContentTemplate`] factory upon building the popup content.
#[derive(Clone)]
pub struct PopupContentContext {
    /// The effective [`EnvironmentContext`] captured from the target element.
    pub environment: EnvironmentContext,
}

/// A lightweight factory for building custom popup content with inherited Environment.
#[derive(Clone)]
pub struct PopupContentTemplate {
    factory: Rc<dyn Fn(PopupContentContext) -> Rc<dyn UIElementExt>>,
}

impl PopupContentTemplate {
    /// Creates a new popup content template from a factory closure.
    pub fn new(factory: impl Fn(PopupContentContext) -> Rc<dyn UIElementExt> + 'static) -> Self {
        Self {
            factory: Rc::new(factory),
        }
    }

    /// Builds the popup UIElement tree using the provided context.
    pub fn build(&self, context: PopupContentContext) -> Rc<dyn UIElementExt> {
        (self.factory)(context)
    }
}

impl<F> From<F> for PopupContentTemplate
where
    F: Fn(PopupContentContext) -> Rc<dyn UIElementExt> + 'static,
{
    fn from(factory: F) -> Self {
        Self::new(factory)
    }
}

/// The resolved context definition attached to an element.
pub enum ResolvedContextDefinition {
    /// A standard `Menu` with an associated presentation mode.
    Menu {
        menu: Rc<dyn MenuExt>,
        presentation: ContextMenuPresentation,
    },
    /// A custom UIElement popup template.
    Popup {
        template: PopupContentTemplate,
    },
}

/// An element and its resolved context menu/popup definition.
pub struct ResolvedContextTarget {
    /// The element that owns the `context_menu` or `context_popup` (either the hit target or an ancestor).
    pub owner: Rc<dyn UIElementExt>,
    /// The resolved definition.
    pub definition: ResolvedContextDefinition,
}

/// Resolves the nearest context menu or context popup definition for `target` by walking up
/// the `visual_parent` ancestry chain.
///
/// Returns `None` if neither `target` nor any of its visual ancestors have a context menu or popup configured.
pub fn resolve_context_target(target: &Rc<dyn UIElementExt>) -> Option<ResolvedContextTarget> {
    let mut current = Some(Rc::clone(target));
    while let Some(elem) = current {
        let has_popup = elem.context_popup();
        let has_menu = elem.context_menu();

        if let Some(template) = has_popup {
            return Some(ResolvedContextTarget {
                owner: elem,
                definition: ResolvedContextDefinition::Popup { template },
            });
        }
        if let Some(menu) = has_menu {
            let presentation = elem.context_menu_presentation();
            return Some(ResolvedContextTarget {
                owner: elem,
                definition: ResolvedContextDefinition::Menu { menu, presentation },
            });
        }
        current = elem.visual_parent();
    }
    None
}

/// The anchor reference geometry for positioning a popup.
#[derive(Debug, Clone, PartialEq)]
pub enum PopupAnchor {
    /// A specific point (e.g. mouse cursor position).
    Point(Point),
    /// A bounding rectangle (e.g. focused element arranged bounds).
    Rect(Rect),
}

/// Placement direction policy for a popup relative to its anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupPlacement {
    /// Automatically flips if overflowing the monitor work area bounds (default).
    #[default]
    AutoFlip,
    /// Place below the anchor.
    Below,
    /// Place above the anchor.
    Above,
    /// Place to the right of the anchor.
    Right,
    /// Place to the left of the anchor.
    Left,
}

/// Pure calculation of a popup's top-left origin given its anchor, desired size, monitor work area, and placement mode in Core top-left (0, 0), Y-down coordinate space.
pub fn calculate_popup_placement(
    anchor: &PopupAnchor,
    popup_size: Size,
    work_area: Rect,
    placement: PopupPlacement,
) -> Point {
    let work_area_right = work_area.x + work_area.width;
    let work_area_bottom = work_area.y + work_area.height;

    match anchor {
        PopupAnchor::Point(p) => {
            let mut x = p.x;
            let mut y = p.y;

            if placement == PopupPlacement::AutoFlip || placement == PopupPlacement::Below {
                if y + popup_size.height > work_area_bottom && p.y - popup_size.height >= work_area.y {
                    y = p.y - popup_size.height;
                }
            } else if placement == PopupPlacement::Above {
                y = p.y - popup_size.height;
                if y < work_area.y && p.y + popup_size.height <= work_area_bottom {
                    y = p.y;
                }
            }

            if placement == PopupPlacement::AutoFlip || placement == PopupPlacement::Right {
                if x + popup_size.width > work_area_right && p.x - popup_size.width >= work_area.x {
                    x = p.x - popup_size.width;
                }
            } else if placement == PopupPlacement::Left {
                x = p.x - popup_size.width;
                if x < work_area.x && p.x + popup_size.width <= work_area_right {
                    x = p.x;
                }
            }

            let max_x = (work_area_right - popup_size.width).max(work_area.x);
            let max_y = (work_area_bottom - popup_size.height).max(work_area.y);
            x = x.clamp(work_area.x, max_x);
            y = y.clamp(work_area.y, max_y);

            Point { x, y }
        }
        PopupAnchor::Rect(r) => {
            let mut x = r.x;
            let mut y = r.y + r.height;

            if placement == PopupPlacement::Above {
                y = r.y - popup_size.height;
                if y < work_area.y && r.y + r.height + popup_size.height <= work_area_bottom {
                    y = r.y + r.height;
                }
            } else {
                // AutoFlip / Below
                if y + popup_size.height > work_area_bottom && r.y - popup_size.height >= work_area.y {
                    y = r.y - popup_size.height;
                }
            }

            if placement == PopupPlacement::Left {
                x = r.x - popup_size.width;
                if x < work_area.x && r.x + r.width + popup_size.width <= work_area_right {
                    x = r.x + r.width;
                }
            } else if placement == PopupPlacement::Right {
                x = r.x + r.width;
                if x + popup_size.width > work_area_right && r.x - popup_size.width >= work_area.x {
                    x = r.x - popup_size.width;
                }
            } else {
                if x + popup_size.width > work_area_right {
                    x = (r.x + r.width - popup_size.width).max(work_area.x);
                }
            }

            let max_x = (work_area_right - popup_size.width).max(work_area.x);
            let max_y = (work_area_bottom - popup_size.height).max(work_area.y);
            x = x.clamp(work_area.x, max_x);
            y = y.clamp(work_area.y, max_y);

            Point { x, y }
        }
    }
}

/// Service that processes [`ContextRequest`]s and resolves the target element and anchor.
pub struct ContextMenuService;

impl ContextMenuService {
    /// Resolves a context request against a root element and focus tracker.
    pub fn process_request(
        root: &Rc<dyn UIElementExt>,
        focus: &FocusTracker,
        request: &ContextRequest,
    ) -> Option<(ResolvedContextTarget, PopupAnchor)> {
        let target = match request.source {
            ContextRequestSource::Pointer => {
                let local_pos = request.local_position?;
                crate::ui::hit_test(root, local_pos)?
            }
            ContextRequestSource::Keyboard => {
                focus.focused()?
            }
            ContextRequestSource::Accessibility | ContextRequestSource::Other => {
                focus.focused().unwrap_or_else(|| Rc::clone(root))
            }
        };

        let resolved = resolve_context_target(&target)?;
        let anchor = request.screen_anchor.clone().unwrap_or_else(|| {
            let offset = target.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
            PopupAnchor::Point(offset)
        });

        Some((resolved, anchor))
    }

    /// Resolves a context request for an explicitly known target (e.g. from a NativeControl context event).
    pub fn process_request_for_target(
        target: &Rc<dyn UIElementExt>,
        request: &ContextRequest,
    ) -> Option<(ResolvedContextTarget, PopupAnchor)> {
        let resolved = resolve_context_target(target)?;
        let anchor = request.screen_anchor.clone().unwrap_or_else(|| {
            let offset = target.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
            PopupAnchor::Point(offset)
        });
        Some((resolved, anchor))
    }

    /// Opens a custom popup definition using the provided popup host, environment context, and work area.
    pub fn open_custom_popup(
        host: &dyn PopupHost,
        template: &PopupContentTemplate,
        anchor: &PopupAnchor,
        environment: EnvironmentContext,
        work_area: Rect,
    ) -> Rc<dyn PopupSurfaceHandle> {
        let content = template.build(PopupContentContext {
            environment: environment.clone(),
        });
        content.set_environment_context(environment);
        content.measure(Size {
            width: work_area.width,
            height: work_area.height,
        });
        let measured = content.measured_size().unwrap_or(Size {
            width: 200.0,
            height: 200.0,
        });
        let popup_size = Size {
            width: measured.width.max(1.0),
            height: measured.height.max(1.0),
        };
        let position = calculate_popup_placement(anchor, popup_size, work_area, PopupPlacement::AutoFlip);
        host.show_popup(PopupRequest {
            content,
            position,
            size: popup_size,
            focus_policy: PopupFocusPolicy::None,
            dismiss_policy: PopupDismissPolicy::LightDismiss,
        })
    }

    /// Opens a custom-rendered standard menu on the provided popup host.
    pub fn open_custom_menu(
        host: &dyn PopupHost,
        menu: &dyn MenuExt,
        anchor: &PopupAnchor,
        work_area: Rect,
    ) -> Rc<dyn PopupSurfaceHandle> {
        let handle_slot: Rc<RefCell<Option<std::rc::Weak<dyn PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        let slot_clone = Rc::clone(&handle_slot);

        let on_close: Rc<dyn Fn()> = Rc::new(move || {
            let weak_opt = slot_clone.borrow_mut().take();
            if let Some(weak_handle) = weak_opt {
                if let Some(handle) = weak_handle.upgrade() {
                    handle.close();
                }
            }
        });

        let content = ContextMenuPresenter::build_menu_view(menu, on_close);
        content.measure(Size {
            width: work_area.width,
            height: work_area.height,
        });
        let measured = content.measured_size().unwrap_or(Size {
            width: 180.0,
            height: 100.0,
        });
        let popup_size = Size {
            width: measured.width.max(1.0),
            height: measured.height.max(1.0),
        };
        let position = calculate_popup_placement(anchor, popup_size, work_area, PopupPlacement::AutoFlip);
        let inner_handle = host.show_popup(PopupRequest {
            content,
            position,
            size: popup_size,
            focus_policy: PopupFocusPolicy::Root,
            dismiss_policy: PopupDismissPolicy::LightDismiss,
        });

        let wrapped_handle: Rc<dyn PopupSurfaceHandle> = Rc::new(CustomMenuPopupHandle {
            inner: inner_handle,
            closed: std::cell::Cell::new(false),
        });
        *handle_slot.borrow_mut() = Some(Rc::downgrade(&wrapped_handle));
        wrapped_handle
    }
}

struct CustomMenuPopupHandle {
    inner: Rc<dyn PopupSurfaceHandle>,
    closed: std::cell::Cell<bool>,
}

impl PopupSurfaceHandle for CustomMenuPopupHandle {
    fn close(&self) {
        if !self.closed.get() {
            self.closed.set(true);
            self.inner.close();
        }
    }
}

impl Drop for CustomMenuPopupHandle {
    fn drop(&mut self) {
        self.close();
    }
}

/// Default presenter that builds an ElwindUI custom-rendered `UIElement` tree from a [`MenuExt`].
pub struct ContextMenuPresenter;

impl ContextMenuPresenter {
    /// Builds a custom-rendered context menu UIElement tree for a menu.
    /// When an item is selected or dismissed, `on_close` is invoked.
    pub fn build_menu_view(
        menu: &dyn MenuExt,
        on_close: Rc<dyn Fn()>,
    ) -> Rc<dyn UIElementExt> {
        let layout = crate::ui::VerticalLayout::new();
        layout.set_margin(4.0);
        layout.set_background(Some(crate::graphics::Color::rgb(45, 45, 48).into()));
        layout.set_tab_stop(true);

        let items = menu.items().to_vec();
        let rows: Rc<RefCell<Vec<Rc<crate::ui::HorizontalLayout>>>> =
            Rc::new(RefCell::new(Vec::new()));
        let selected_index: Rc<std::cell::Cell<Option<usize>>> =
            Rc::new(std::cell::Cell::new(None));

        let rows_for_highlight = Rc::clone(&rows);
        let items_for_highlight = items.clone();
        let selected_for_highlight = Rc::clone(&selected_index);
        let update_highlight = Rc::new(move || {
            let sel = selected_for_highlight.get();
            let rows_borrow = rows_for_highlight.borrow();
            for (i, row) in rows_borrow.iter().enumerate() {
                let enabled = items_for_highlight.get(i).map(|it| it.enabled()).unwrap_or(true);
                if enabled && sel == Some(i) {
                    row.set_background(Some(crate::graphics::Color::rgb(0, 120, 215).into()));
                } else {
                    row.set_background(Some(crate::graphics::Color::transparent().into()));
                }
            }
        });

        for (i, item) in items.iter().enumerate() {
            let row = crate::ui::HorizontalLayout::new();
            row.set_margin(4.0);

            let enabled = item.enabled();

            let label = crate::ui::TextBlock::new();
            label.set_text(&item.text());
            let text_color = if enabled {
                crate::graphics::Color::rgb(240, 240, 240)
            } else {
                crate::graphics::Color::rgb(128, 128, 128)
            };
            label.set_foreground(Some(text_color.into()));
            crate::ui::LayoutExt::children(&*row).add(Rc::clone(&label) as Rc<dyn UIElementExt>);

            if let Some(shortcut_str) = item.shortcut() {
                let shortcut_label = crate::ui::TextBlock::new();
                shortcut_label.set_text(&format!("   {}", shortcut_str));
                let sc_color = if enabled {
                    crate::graphics::Color::rgb(160, 160, 160)
                } else {
                    crate::graphics::Color::rgb(100, 100, 100)
                };
                shortcut_label.set_foreground(Some(sc_color.into()));
                crate::ui::LayoutExt::children(&*row).add(Rc::clone(&shortcut_label) as Rc<dyn UIElementExt>);
            }

            let item_clone = Rc::clone(item);
            let close_cb = Rc::clone(&on_close);
            let sel_cell = Rc::clone(&selected_index);
            let highlight_cb = Rc::clone(&update_highlight);

            if enabled {
                let sel_enter = Rc::clone(&sel_cell);
                let hl_enter = Rc::clone(&highlight_cb);
                row.register_routed_handler::<crate::input::PointerEventArgs>(
                    "on_pointer_entered",
                    Box::new(move |_args, _routed| {
                        sel_enter.set(Some(i));
                        hl_enter();
                    }),
                );

                let sel_exit = Rc::clone(&sel_cell);
                let hl_exit = Rc::clone(&highlight_cb);
                row.register_routed_handler::<crate::input::PointerEventArgs>(
                    "on_pointer_exited",
                    Box::new(move |_args, _routed| {
                        if sel_exit.get() == Some(i) {
                            sel_exit.set(None);
                            hl_exit();
                        }
                    }),
                );

                row.register_routed_handler::<crate::input::TappedEventArgs>(
                    "on_tapped",
                    Box::new(move |_args, _routed| {
                        item_clone.select();
                        close_cb();
                    }),
                );
            }

            rows.borrow_mut().push(Rc::clone(&row));
            crate::ui::LayoutExt::children(&*layout).add(row as Rc<dyn UIElementExt>);
        }

        // Keyboard navigation on custom menu:
        let items_len = items.len();
        let items_for_key = items.clone();
        let sel_key = Rc::clone(&selected_index);
        let hl_key = Rc::clone(&update_highlight);
        let close_key = Rc::clone(&on_close);

        layout.register_routed_handler::<crate::input::KeyEventArgs>(
            "on_key_down",
            Box::new(move |args, _routed| {
                match args.key {
                    crate::input::Key::Down => {
                        let current = sel_key.get().unwrap_or(usize::MAX);
                        let mut next = (current.wrapping_add(1)) % items_len.max(1);
                        // Skip disabled items
                        for _ in 0..items_len {
                            if items_for_key.get(next).map(|it| it.enabled()).unwrap_or(false) {
                                sel_key.set(Some(next));
                                hl_key();
                                break;
                            }
                            next = (next + 1) % items_len;
                        }
                    }
                    crate::input::Key::Up => {
                        let current = sel_key.get().unwrap_or(0);
                        let mut prev = if current == 0 { items_len.saturating_sub(1) } else { current - 1 };
                        for _ in 0..items_len {
                            if items_for_key.get(prev).map(|it| it.enabled()).unwrap_or(false) {
                                sel_key.set(Some(prev));
                                hl_key();
                                break;
                            }
                            prev = if prev == 0 { items_len.saturating_sub(1) } else { prev - 1 };
                        }
                    }
                    crate::input::Key::Home => {
                        for idx in 0..items_len {
                            if items_for_key.get(idx).map(|it| it.enabled()).unwrap_or(false) {
                                sel_key.set(Some(idx));
                                hl_key();
                                break;
                            }
                        }
                    }
                    crate::input::Key::End => {
                        for idx in (0..items_len).rev() {
                            if items_for_key.get(idx).map(|it| it.enabled()).unwrap_or(false) {
                                sel_key.set(Some(idx));
                                hl_key();
                                break;
                            }
                        }
                    }
                    crate::input::Key::Enter | crate::input::Key::Space => {
                        if let Some(idx) = sel_key.get() {
                            if let Some(it) = items_for_key.get(idx) {
                                if it.enabled() {
                                    it.select();
                                    close_key();
                                }
                            }
                        }
                    }
                    crate::input::Key::Escape => {
                        close_key();
                    }
                    _ => {}
                }
            }),
        );

        layout as Rc<dyn UIElementExt>
    }
}

/// A handle to an open popup surface for dismissal or updates.
pub trait PopupSurfaceHandle {
    /// Closes the popup surface and cleans up native resources.
    fn close(&self);
}

/// Focus policy for newly opened popup surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupFocusPolicy {
    /// Do not automatically transfer focus.
    #[default]
    None,
    /// Transfer focus to the root UIElement of the popup tree.
    Root,
    /// Transfer focus to the first focusable element inside the popup tree.
    FirstFocusable,
}

/// Dismissal policy for standalone popup surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupDismissPolicy {
    /// Dismiss automatically on outside click or Escape.
    #[default]
    LightDismiss,
    /// Dismiss only upon explicit programmatic close.
    Explicit,
}

/// A request to display a popup surface.
#[derive(Clone)]
pub struct PopupRequest {
    pub content: Rc<dyn UIElementExt>,
    pub position: Point,
    pub size: Size,
    pub focus_policy: PopupFocusPolicy,
    pub dismiss_policy: PopupDismissPolicy,
}

/// Capability trait implemented by backend window hosts to display standalone popup surfaces.
pub trait PopupHost {
    /// Displays a popup surface according to `request`.
    fn show_popup(&self, request: PopupRequest) -> Rc<dyn PopupSurfaceHandle>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::FakeMenu;
    use crate::ui::{LayoutExt, ListExt, MenuItemExt, TextBlock, VerticalLayout};
    use std::cell::{Cell, RefCell};

    struct FakePopupHandle {
        closed: Rc<Cell<bool>>,
    }

    impl PopupSurfaceHandle for FakePopupHandle {
        fn close(&self) {
            self.closed.set(true);
        }
    }

    struct FakePopupHost {
        shown: RefCell<Vec<(Rc<dyn UIElementExt>, Point, Size)>>,
        closed: Rc<Cell<bool>>,
    }

    impl FakePopupHost {
        fn new() -> Self {
            Self {
                shown: RefCell::new(Vec::new()),
                closed: Rc::new(Cell::new(false)),
            }
        }
    }

    impl PopupHost for FakePopupHost {
        fn show_popup(
            &self,
            request: PopupRequest,
        ) -> Rc<dyn PopupSurfaceHandle> {
            self.shown.borrow_mut().push((
                Rc::clone(&request.content),
                request.position,
                request.size,
            ));
            Rc::new(FakePopupHandle {
                closed: Rc::clone(&self.closed),
            })
        }
    }

    #[test]
    fn resolve_target_own_context_menu() {
        let node = VerticalLayout::new();
        let menu = FakeMenu::new();
        node.set_context_menu(Some(Rc::clone(&menu) as Rc<dyn MenuExt>));

        let node_dyn: Rc<dyn UIElementExt> = node;
        let resolved = resolve_context_target(&node_dyn).expect("should resolve context target");
        assert!(Rc::ptr_eq(&resolved.owner, &node_dyn));
        match resolved.definition {
            ResolvedContextDefinition::Menu { menu: m, presentation } => {
                assert!(Rc::ptr_eq(&m, &(menu as Rc<dyn MenuExt>)));
                assert_eq!(presentation, ContextMenuPresentation::Native);
            }
            _ => panic!("expected Menu definition"),
        }
    }

    #[test]
    fn resolve_target_own_context_popup() {
        let node = VerticalLayout::new();
        let template = PopupContentTemplate::new(|_ctx| {
            let layout = VerticalLayout::new();
            let label = TextBlock::new();
            layout.children().add(Rc::clone(&label) as Rc<dyn UIElementExt>);
            layout as Rc<dyn UIElementExt>
        });
        node.set_context_popup(Some(template));

        let node_dyn: Rc<dyn UIElementExt> = node;
        let resolved = resolve_context_target(&node_dyn).expect("should resolve context target");
        assert!(Rc::ptr_eq(&resolved.owner, &node_dyn));
        match resolved.definition {
            ResolvedContextDefinition::Popup { template: _ } => {}
            _ => panic!("expected Popup definition"),
        }
    }

    #[test]
    fn open_custom_popup_measures_and_displays_on_host() {
        let host = FakePopupHost::new();
        let template = PopupContentTemplate::new(|_ctx| {
            let block = TextBlock::new();
            block.set_width(120.0);
            block.set_height(80.0);
            block as Rc<dyn UIElementExt>
        });

        let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let handle = ContextMenuService::open_custom_popup(
            &host,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        );

        assert_eq!(host.shown.borrow().len(), 1);
        let (_, pos, size) = &host.shown.borrow()[0];
        assert_eq!(*pos, Point { x: 50.0, y: 50.0 });
        assert_eq!(*size, Size { width: 120.0, height: 80.0 });

        assert!(!host.closed.get());
        handle.close();
        assert!(host.closed.get());
    }

    #[test]
    fn open_custom_menu_measures_and_displays_on_host() {
        let host = FakePopupHost::new();
        let menu = FakeMenu::new();

        let anchor = PopupAnchor::Point(Point { x: 100.0, y: 100.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 1024.0,
            height: 768.0,
        };

        let handle = ContextMenuService::open_custom_menu(
            &host,
            &*menu,
            &anchor,
            work_area,
        );

        assert_eq!(host.shown.borrow().len(), 1);
        let (_, pos, size) = &host.shown.borrow()[0];
        assert_eq!(*pos, Point { x: 100.0, y: 100.0 });
        assert!(size.width > 0.0);
        assert!(size.height > 0.0);

        assert!(!host.closed.get());
        handle.close();
        assert!(host.closed.get());
    }

    #[test]
    fn open_custom_menu_item_selection_triggers_on_close_and_dismisses_popup_surface() {
        let host = FakePopupHost::new();
        let menu = FakeMenu::new();

        let item = crate::ui::testsupport::FakeMenuItem::new();
        item.set_text("Selectable Item");
        let selected = Rc::new(Cell::new(false));
        let sel_clone = Rc::clone(&selected);
        item.set_on_select(Box::new(move || sel_clone.set(true)));
        menu.add(Rc::clone(&item) as Rc<dyn MenuItemExt>);

        let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
        let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

        let _handle = ContextMenuService::open_custom_menu(
            &host,
            &*menu,
            &anchor,
            work_area,
        );

        assert_eq!(host.shown.borrow().len(), 1);
        let (content, _pos, _size) = &host.shown.borrow()[0];
        assert!(!host.closed.get());

        // Find the first child row in custom menu view and trigger on_tapped
        let layout_children = content.visual_children();
        assert!(!layout_children.is_empty());
        let row = &layout_children[0];

        let routed_args = crate::input::RoutedEventArgs::default();
        crate::ui::dispatch_routed(
            row,
            "on_tapped",
            &crate::input::TappedEventArgs {
                position: Point { x: 10.0, y: 10.0 },
                modifiers: crate::input::KeyModifiers::default(),
            },
            &routed_args,
        );

        // Verification: item was selected and popup surface was closed!
        assert!(selected.get(), "menu item on_select callback should be invoked");
        assert!(host.closed.get(), "popup surface handle close() should be invoked on item selection");
    }

    #[test]
    fn resolve_target_ancestor_fallback() {
        let root = VerticalLayout::new();
        let menu = FakeMenu::new();
        root.set_context_menu(Some(Rc::clone(&menu) as Rc<dyn MenuExt>));

        let child = TextBlock::new();
        root.children().add(Rc::clone(&child) as Rc<dyn UIElementExt>);

        let root_dyn: Rc<dyn UIElementExt> = root;
        let child_dyn: Rc<dyn UIElementExt> = child;

        // child has no context menu, so lookup finds root
        let resolved = resolve_context_target(&child_dyn).expect("should resolve ancestor context target");
        assert!(Rc::ptr_eq(&resolved.owner, &root_dyn));
    }

    #[test]
    fn resolve_target_nearest_ancestor_wins() {
        let root = VerticalLayout::new();
        let root_menu = FakeMenu::new();
        root.set_context_menu(Some(Rc::clone(&root_menu) as Rc<dyn MenuExt>));

        let mid = VerticalLayout::new();
        let mid_menu = FakeMenu::new();
        mid.set_context_menu(Some(Rc::clone(&mid_menu) as Rc<dyn MenuExt>));
        root.children().add(Rc::clone(&mid) as Rc<dyn UIElementExt>);

        let leaf = TextBlock::new();
        mid.children().add(Rc::clone(&leaf) as Rc<dyn UIElementExt>);

        let mid_dyn: Rc<dyn UIElementExt> = mid;
        let leaf_dyn: Rc<dyn UIElementExt> = leaf;

        let resolved = resolve_context_target(&leaf_dyn).expect("should resolve nearest context target");
        assert!(Rc::ptr_eq(&resolved.owner, &mid_dyn));
        match resolved.definition {
            ResolvedContextDefinition::Menu { menu: m, .. } => {
                assert!(Rc::ptr_eq(&m, &(mid_menu as Rc<dyn MenuExt>)));
            }
            _ => panic!("expected Menu definition"),
        }
    }

    #[test]
    fn resolve_target_no_context_returns_none() {
        let root = VerticalLayout::new();
        let leaf = TextBlock::new();
        root.children().add(Rc::clone(&leaf) as Rc<dyn UIElementExt>);

        let leaf_dyn: Rc<dyn UIElementExt> = leaf;
        assert!(resolve_context_target(&leaf_dyn).is_none());
    }

    #[test]
    fn calculate_placement_flips_on_work_area_boundary() {
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        };
        let popup_size = Size {
            width: 200.0,
            height: 300.0,
        };

        // Middle of screen: extends downward normally (y = 100.0)
        let pos1 = calculate_popup_placement(
            &PopupAnchor::Point(Point { x: 100.0, y: 100.0 }),
            popup_size,
            work_area,
            PopupPlacement::AutoFlip,
        );
        assert_eq!(pos1, Point { x: 100.0, y: 100.0 });

        // Near bottom: flips upward (y = 700 - 300 = 400.0)
        let pos_bottom = calculate_popup_placement(
            &PopupAnchor::Point(Point { x: 100.0, y: 700.0 }),
            popup_size,
            work_area,
            PopupPlacement::AutoFlip,
        );
        assert_eq!(pos_bottom, Point { x: 100.0, y: 400.0 });

        // Near right: flips leftward (x = 950 - 200 = 750.0, y = 100.0)
        let pos2 = calculate_popup_placement(
            &PopupAnchor::Point(Point { x: 950.0, y: 100.0 }),
            popup_size,
            work_area,
            PopupPlacement::AutoFlip,
        );
        assert_eq!(pos2, Point { x: 750.0, y: 100.0 });

        // Near bottom-right: flips both leftward and upward (x = 750.0, y = 400.0)
        let pos_br = calculate_popup_placement(
            &PopupAnchor::Point(Point { x: 950.0, y: 700.0 }),
            popup_size,
            work_area,
            PopupPlacement::AutoFlip,
        );
        assert_eq!(pos_br, Point { x: 750.0, y: 400.0 });
    }

    struct TestKey;
    impl crate::environment::EnvironmentKey for TestKey {
        type Value = u32;
        fn default_value() -> Self::Value {
            100
        }
    }

    #[test]
    fn open_custom_popup_inherits_effective_environment() {
        let root = VerticalLayout::new();
        let env = EnvironmentContext::root();
        env.set::<TestKey>(42);
        root.set_environment_context(env);

        let child = VerticalLayout::new();
        root.children().add(Rc::clone(&child) as Rc<dyn UIElementExt>);

        let leaf = TextBlock::new();
        child.children().add(Rc::clone(&leaf) as Rc<dyn UIElementExt>);

        let leaf_dyn: Rc<dyn UIElementExt> = leaf;
        let effective = leaf_dyn.effective_environment();
        assert_eq!(effective.get::<TestKey>(), 42);

        let captured_env: Rc<RefCell<Option<EnvironmentContext>>> = Rc::new(RefCell::new(None));
        let captured_clone = Rc::clone(&captured_env);

        let template = PopupContentTemplate::new(move |ctx| {
            *captured_clone.borrow_mut() = Some(ctx.environment.clone());
            let b = TextBlock::new();
            b as Rc<dyn UIElementExt>
        });

        let host = FakePopupHost::new();
        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

        ContextMenuService::open_custom_popup(
            &host,
            &template,
            &anchor,
            leaf_dyn.effective_environment(),
            work_area,
        );

        let resolved_env = captured_env.borrow().clone().expect("popup should capture environment");
        assert_eq!(resolved_env.get::<TestKey>(), 42);
    }

    #[test]
    fn custom_menu_keyboard_navigation_and_item_state() {
        let menu = FakeMenu::new();

        let item1 = crate::ui::testsupport::FakeMenuItem::new();
        item1.set_text("Item 1");
        item1.set_shortcut("1");
        let item1_selected = Rc::new(Cell::new(false));
        let item1_sel_clone = Rc::clone(&item1_selected);
        item1.set_on_select(Box::new(move || item1_sel_clone.set(true)));

        let item2_disabled = crate::ui::testsupport::FakeMenuItem::new();
        item2_disabled.set_text("Item 2 Disabled");
        item2_disabled.set_enabled(false);
        let item2_selected = Rc::new(Cell::new(false));
        let item2_sel_clone = Rc::clone(&item2_selected);
        item2_disabled.set_on_select(Box::new(move || item2_sel_clone.set(true)));

        let item3 = crate::ui::testsupport::FakeMenuItem::new();
        item3.set_text("Item 3");
        item3.set_shortcut("3");
        let item3_selected = Rc::new(Cell::new(false));
        let item3_sel_clone = Rc::clone(&item3_selected);
        item3.set_on_select(Box::new(move || item3_sel_clone.set(true)));

        menu.add(Rc::clone(&item1) as Rc<dyn MenuItemExt>);
        menu.add(Rc::clone(&item2_disabled) as Rc<dyn MenuItemExt>);
        menu.add(Rc::clone(&item3) as Rc<dyn MenuItemExt>);

        let closed = Rc::new(Cell::new(false));
        let closed_clone = Rc::clone(&closed);
        let menu_view = ContextMenuPresenter::build_menu_view(&*menu, Rc::new(move || closed_clone.set(true)));

        let routed_args = crate::input::RoutedEventArgs::default();

        // Down key moves to item1
        crate::ui::dispatch_routed(
            &menu_view,
            "on_key_down",
            &crate::input::KeyEventArgs {
                key: crate::input::Key::Down,
                modifiers: crate::input::KeyModifiers::default(),
                is_repeat: false,
            },
            &routed_args,
        );

        // Next Down key skips disabled item2 and selects item3
        crate::ui::dispatch_routed(
            &menu_view,
            "on_key_down",
            &crate::input::KeyEventArgs {
                key: crate::input::Key::Down,
                modifiers: crate::input::KeyModifiers::default(),
                is_repeat: false,
            },
            &routed_args,
        );

        // Enter key activates item3
        crate::ui::dispatch_routed(
            &menu_view,
            "on_key_down",
            &crate::input::KeyEventArgs {
                key: crate::input::Key::Enter,
                modifiers: crate::input::KeyModifiers::default(),
                is_repeat: false,
            },
            &routed_args,
        );

        assert!(!item1_selected.get());
        assert!(!item2_selected.get());
        assert!(item3_selected.get());
        assert!(closed.get());
    }
}
