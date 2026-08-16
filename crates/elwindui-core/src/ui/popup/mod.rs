//! Context menu, custom popup, and PopupSurface abstractions.
//!
//! See `docs/specs/ui_spec.md` and `docs/design/runtime/popup_context_menu_design.md`.

use crate::base::{Point, Rect, Size};
use crate::environment::EnvironmentContext;
use crate::focus::FocusTracker;
use crate::ui::{LayoutExt, MenuExt, TextBlockExt, TextStyleOwner, UIElementExt};
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
    /// The location of the interaction in window/screen logical coordinates, if pointer-driven.
    pub position: Option<Point>,
}

impl ContextRequest {
    /// Creates a pointer-driven context request at the specified point.
    pub fn pointer(position: Point) -> Self {
        Self {
            source: ContextRequestSource::Pointer,
            position: Some(position),
        }
    }

    /// Creates a keyboard-driven context request (targeting the focused element).
    pub fn keyboard() -> Self {
        Self {
            source: ContextRequestSource::Keyboard,
            position: None,
        }
    }

    /// Creates an accessibility-driven context request.
    pub fn accessibility(position: Option<Point>) -> Self {
        Self {
            source: ContextRequestSource::Accessibility,
            position,
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

/// Pure calculation of a popup's top-left origin given its anchor, desired size, monitor work area, and placement mode.
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
        let (target, anchor) = match request.source {
            ContextRequestSource::Pointer => {
                let position = request.position?;
                let hit = crate::ui::hit_test(root, position)?;
                (hit, PopupAnchor::Point(position))
            }
            ContextRequestSource::Keyboard => {
                let focused = focus.focused()?;
                let offset = focused.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
                let arranged_rect = Rect {
                    x: offset.x,
                    y: offset.y,
                    width: focused.arranged_width().unwrap_or(0.0),
                    height: focused.arranged_height().unwrap_or(0.0),
                };
                (focused, PopupAnchor::Rect(arranged_rect))
            }
            ContextRequestSource::Accessibility | ContextRequestSource::Other => {
                let target = focus.focused().unwrap_or_else(|| Rc::clone(root));
                let anchor = match request.position {
                    Some(pos) => PopupAnchor::Point(pos),
                    None => {
                        let offset = target.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
                        let arranged_rect = Rect {
                            x: offset.x,
                            y: offset.y,
                            width: target.arranged_width().unwrap_or(0.0),
                            height: target.arranged_height().unwrap_or(0.0),
                        };
                        PopupAnchor::Rect(arranged_rect)
                    }
                };
                (target, anchor)
            }
        };

        let resolved = resolve_context_target(&target)?;
        Some((resolved, anchor))
    }

    /// Resolves a context request for an explicitly known target (e.g. from a NativeControl context event).
    pub fn process_request_for_target(
        target: &Rc<dyn UIElementExt>,
        request: &ContextRequest,
    ) -> Option<(ResolvedContextTarget, PopupAnchor)> {
        let anchor = match request.position {
            Some(pos) => PopupAnchor::Point(pos),
            None => {
                let offset = target.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
                let arranged_rect = Rect {
                    x: offset.x,
                    y: offset.y,
                    width: target.arranged_width().unwrap_or(0.0),
                    height: target.arranged_height().unwrap_or(0.0),
                };
                PopupAnchor::Rect(arranged_rect)
            }
        };
        let resolved = resolve_context_target(target)?;
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
        let content = template.build(PopupContentContext { environment });
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
        host.show_popup(content, position, popup_size)
    }

    /// Opens a custom-rendered standard menu on the provided popup host.
    pub fn open_custom_menu(
        host: &dyn PopupHost,
        menu: &dyn MenuExt,
        anchor: &PopupAnchor,
        work_area: Rect,
    ) -> Rc<dyn PopupSurfaceHandle> {
        let handle_cell: Rc<std::cell::RefCell<Option<Rc<dyn PopupSurfaceHandle>>>> =
            Rc::new(std::cell::RefCell::new(None));
        let handle_weak = Rc::downgrade(&handle_cell);

        let on_close: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(cell) = handle_weak.upgrade() {
                if let Some(handle) = cell.borrow().as_ref() {
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
        let handle = host.show_popup(content, position, popup_size);
        *handle_cell.borrow_mut() = Some(Rc::clone(&handle));
        handle
    }
}

/// Default presenter that builds an ElwindUI custom-rendered `UIElement` tree from a [`MenuExt`].
pub struct ContextMenuPresenter;

impl ContextMenuPresenter {
    /// Builds a custom-rendered context menu UIElement tree for a menu.
    /// When an item is clicked, `on_close` is invoked to dismiss the popup surface.
    pub fn build_menu_view(
        menu: &dyn MenuExt,
        on_close: Rc<dyn Fn()>,
    ) -> Rc<dyn UIElementExt> {
        let layout = crate::ui::VerticalLayout::new();
        layout.set_margin(4.0);
        layout.set_background(Some(crate::graphics::Color::rgb(45, 45, 48).into()));

        for item in menu.items().to_vec() {
            let row = crate::ui::HorizontalLayout::new();
            row.set_margin(4.0);

            let label = crate::ui::TextBlock::new();
            label.set_text(&item.text());
            label.set_foreground(Some(crate::graphics::Color::rgb(240, 240, 240).into()));
            crate::ui::LayoutExt::children(&*row).add(Rc::clone(&label) as Rc<dyn UIElementExt>);

            let item_clone = Rc::clone(&item);
            let close_cb = Rc::clone(&on_close);
            row.register_routed_handler::<crate::input::PointerEventArgs>(
                "on_pointer_pressed",
                Box::new(move |_args, _routed| {
                    item_clone.select();
                    close_cb();
                }),
            );

            crate::ui::LayoutExt::children(&*layout).add(row as Rc<dyn UIElementExt>);
        }

        layout as Rc<dyn UIElementExt>
    }
}

/// A handle to an open popup surface for dismissal or updates.
pub trait PopupSurfaceHandle {
    /// Closes the popup surface and cleans up native resources.
    fn close(&self);
}

/// Capability trait implemented by backend window hosts to display standalone popup surfaces.
pub trait PopupHost {
    /// Displays a popup surface containing `content` at `position` with `size`.
    fn show_popup(
        &self,
        content: Rc<dyn UIElementExt>,
        position: Point,
        size: Size,
    ) -> Rc<dyn PopupSurfaceHandle>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::FakeMenu;
    use crate::ui::{LayoutExt, TextBlock, VerticalLayout};
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
            content: Rc<dyn UIElementExt>,
            position: Point,
            size: Size,
        ) -> Rc<dyn PopupSurfaceHandle> {
            self.shown.borrow_mut().push((Rc::clone(&content), position, size));
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

        // Near top-left: fits normally
        let pos1 = calculate_popup_placement(
            &PopupAnchor::Point(Point { x: 100.0, y: 100.0 }),
            popup_size,
            work_area,
            PopupPlacement::AutoFlip,
        );
        assert_eq!(pos1, Point { x: 100.0, y: 100.0 });

        // Near bottom-right: flips left and above
        let pos2 = calculate_popup_placement(
            &PopupAnchor::Point(Point { x: 950.0, y: 750.0 }),
            popup_size,
            work_area,
            PopupPlacement::AutoFlip,
        );
        assert_eq!(pos2, Point { x: 750.0, y: 450.0 });
    }
}
