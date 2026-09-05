//! Context menu, custom popup, and PopupSurface abstractions.
//!
//! See `docs/specs/ui_spec.md` and `docs/design/runtime/popup_context_menu_design.md`.

use crate::base::{Point, Rect, Size};
use crate::environment::{EnvironmentContext, EnvironmentKey};
use crate::focus::FocusTracker;
use crate::ui::{
    HorizontalLayoutExt, IconElementExt, IconSourceElementExt, LayoutExt, MenuExt, TextBlockExt,
    TextStyleOwner, UIElementExt, ViewBuildContext, ViewFactory, unmount_subtree,
};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

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
    pub fn accessibility(
        local_position: Option<Point>,
        screen_anchor: Option<PopupAnchor>,
    ) -> Self {
        Self {
            source: ContextRequestSource::Accessibility,
            local_position,
            screen_anchor,
        }
    }
}

/// A callback that closes the popup that installed it, invoked from within a
/// `context_popup: view! { .. }` subtree. Not part of [`ViewBuildContext`] — installed only into
/// the popup-scoped Environment derived by [`ContextMenuService::open_custom_popup`], via
/// [`PopupDismissActionKey`], so [`ViewFactory`] itself stays popup-agnostic.
///
/// Ownership graph (no strong cycle back to the popup content that holds this value): popup content
/// → its Environment → this `PopupDismissAction` → `open_custom_popup`'s private
/// `Rc<RefCell<PopupDismissState>>` (a fresh allocation per call, owned by nothing the popup content
/// itself reaches) → at most a `Weak<dyn PopupSurfaceHandle>` once shown (`PopupDismissState::Open`)
/// — never a strong `Rc` back to the surface, and the surface itself does not transitively reach back
/// to the popup content it displays (a `PopupSurfaceHandle` only reaches its own backend
/// `InnerPopupSurface`, whose own `content` field is exactly what `close()` releases).
#[derive(Clone)]
pub struct PopupDismissAction {
    dismiss: Rc<dyn Fn()>,
}

impl PopupDismissAction {
    /// Creates a dismiss action from a closure. No-op if the popup is already closed.
    pub fn new(dismiss: impl Fn() + 'static) -> Self {
        Self {
            dismiss: Rc::new(dismiss),
        }
    }

    /// Closes the popup that installed this action.
    pub fn dismiss(&self) {
        (self.dismiss)()
    }
}

/// Private state machine backing a single [`ContextMenuService::open_custom_popup`] call's
/// [`PopupDismissAction`], distinguishing "not shown yet" from "shown" from "already dismissed" so a
/// dismiss request arriving before a native surface exists (during `ViewFactory::build`, i.e. a
/// generated Component's own `on_mount` once #162 lands) is captured rather than silently lost. Not
/// part of any public type — `PopupDismissAction`'s own public shape stays a plain `Fn()` callback;
/// nothing outside `open_custom_popup` ever sees this enum.
enum PopupDismissState {
    /// `template.build(..)` (and any Component `on_mount` it runs) is still in progress; no native
    /// surface exists yet.
    Building,
    /// The popup is on-screen; `close()` on the held handle dismisses it. Only ever a `Weak`
    /// reference — see [`PopupDismissAction`]'s own doc comment for why a strong one would cycle.
    Open(Weak<dyn PopupSurfaceHandle>),
    /// Already dismissed (either during `Building` or after `Open`) — any further dismiss call is a
    /// no-op.
    Dismissed,
}

/// Environment key carrying the active [`PopupDismissAction`] within a popup-scoped Environment.
/// `None` outside a popup (the default, inherited everywhere a popup has not derived and set it).
pub struct PopupDismissActionKey;

impl EnvironmentKey for PopupDismissActionKey {
    type Value = Option<PopupDismissAction>;

    fn default_value() -> Self::Value {
        None
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
    Popup { template: ViewFactory },
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
                if y + popup_size.height > work_area_bottom
                    && p.y - popup_size.height >= work_area.y
                {
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
                if y + popup_size.height > work_area_bottom
                    && r.y - popup_size.height >= work_area.y
                {
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
            ContextRequestSource::Keyboard => focus.focused()?,
            ContextRequestSource::Accessibility | ContextRequestSource::Other => {
                focus.focused().unwrap_or_else(|| Rc::clone(root))
            }
        };

        let resolved = resolve_context_target(&target)?;
        let anchor = request.screen_anchor.clone()?;

        Some((resolved, anchor))
    }

    /// Resolves a context request for an explicitly known target (e.g. from a NativeControl context event).
    pub fn process_request_for_target(
        target: &Rc<dyn UIElementExt>,
        request: &ContextRequest,
    ) -> Option<(ResolvedContextTarget, PopupAnchor)> {
        let resolved = resolve_context_target(target)?;
        let anchor = request.screen_anchor.clone()?;
        Some((resolved, anchor))
    }

    /// Opens a custom popup definition using the provided popup host, owner, environment context,
    /// and work area.
    ///
    /// Derives a popup-scoped `EnvironmentContext` from `environment` (never mutating `environment`
    /// itself), installs a [`PopupDismissAction`] into it, then builds `template` against that
    /// derived Environment with `owner` captured only as `Weak`. Returns `None` without showing
    /// anything in any of these cases:
    ///
    /// - `template` declines to build (e.g. `owner` has already been dropped by the time this is
    ///   called — enforced by `ViewFactory::build` itself);
    /// - the built content calls the installed `PopupDismissAction` during its own build/mount
    ///   (e.g. from a Component's `on_mount`, before a native surface exists) — the built content is
    ///   unmounted and the popup is never shown, rather than the dismiss request being silently lost;
    /// - `host.show_popup` itself fails (backend-specific — e.g. WinUI3 coordinate conversion) — the
    ///   already-built content is unmounted rather than leaked mounted-but-never-shown.
    pub fn open_custom_popup(
        host: &dyn PopupHost,
        owner: &Rc<dyn UIElementExt>,
        template: &ViewFactory,
        anchor: &PopupAnchor,
        environment: EnvironmentContext,
        work_area: Rect,
    ) -> Option<Rc<dyn PopupSurfaceHandle>> {
        let popup_environment = environment.derive();

        // `PopupDismissState` distinguishes "not shown yet" from "shown" from "already dismissed",
        // so a dismiss request arriving during `template.build` (i.e. during a generated
        // Component's own `on_mount`, once #162 lands) is captured rather than lost — see
        // `PopupDismissState`'s own doc comment for the full state machine. Private to this
        // function; `PopupDismissAction`'s own public shape stays a plain callback.
        let dismiss_state = Rc::new(RefCell::new(PopupDismissState::Building));
        let state_for_dismiss = Rc::clone(&dismiss_state);
        let dismiss_action = PopupDismissAction::new(move || {
            let mut state = state_for_dismiss.borrow_mut();
            match std::mem::replace(&mut *state, PopupDismissState::Dismissed) {
                PopupDismissState::Building | PopupDismissState::Dismissed => {}
                PopupDismissState::Open(weak_handle) => {
                    drop(state);
                    let handle: Option<Rc<dyn PopupSurfaceHandle>> = weak_handle.upgrade();
                    if let Some(handle) = handle {
                        handle.close();
                    }
                }
            }
        });
        popup_environment.set::<PopupDismissActionKey>(Some(dismiss_action));

        let content = template.build(ViewBuildContext {
            owner: Rc::downgrade(owner),
            environment: popup_environment.clone(),
        })?;

        if matches!(*dismiss_state.borrow(), PopupDismissState::Dismissed) {
            // Dismissed synchronously during build/mount, before any native surface exists — the
            // popup must never be shown at all, not shown-then-immediately-closed.
            unmount_subtree(&content);
            return None;
        }

        content.set_environment_context(popup_environment);
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
        let position =
            calculate_popup_placement(anchor, popup_size, work_area, PopupPlacement::AutoFlip);
        // `dismiss()` may run synchronously from *inside* `host.show_popup` (a backend's native
        // "show" call can itself dispatch events reentrantly) — the `Building` state above only
        // covers up to this call, not through it. `dismiss_state` is checked again, atomically,
        // immediately after `show_popup` returns, so a dismiss during the call is never silently
        // overwritten by the unconditional `Open` transition that used to follow.
        let Some(handle) = host.show_popup(PopupRequest {
            content: Rc::clone(&content),
            position,
            size: popup_size,
            focus_policy: PopupFocusPolicy::None,
            dismiss_policy: PopupDismissPolicy::LightDismiss,
        }) else {
            // Backend show failed (e.g. WinUI3 coordinate conversion / Popup construction) — the
            // content above may already carry mounted Component state; tear it down rather than
            // leaking a mounted-but-never-shown subtree. Finalize to `Dismissed` first so a
            // reentrant dismiss() call racing this path (or arriving from within the unmount below)
            // observes a terminal state, never a stale `Building`.
            *dismiss_state.borrow_mut() = PopupDismissState::Dismissed;
            unmount_subtree(&content);
            return None;
        };
        // Single atomic transition out of `Building`: either publish the real handle (the common
        // case) or discover that `dismiss()` already ran *during* `show_popup` and fired against an
        // empty slot — in which case this freshly-created `handle` must still be closed (the
        // dismiss request is honored against this handle instead of being lost), and the popup must
        // not be returned as open. `Open` is otherwise unreachable here: nothing outside this
        // function can observe or mutate `dismiss_state` before this point.
        let dismissed_during_show = {
            let mut state = dismiss_state.borrow_mut();
            match std::mem::replace(&mut *state, PopupDismissState::Dismissed) {
                PopupDismissState::Building => {
                    *state = PopupDismissState::Open(Rc::downgrade(&handle));
                    false
                }
                PopupDismissState::Dismissed => true,
                PopupDismissState::Open(_) => {
                    unreachable!(
                        "PopupDismissState cannot already be Open before this function ever \
                         publishes a handle into it"
                    )
                }
            }
        };
        if dismissed_during_show {
            handle.close();
            return None;
        }
        Some(handle)
    }

    /// Opens a custom-rendered standard menu on the provided popup host. Returns `None` (after
    /// unmounting the already-built menu content) if `host.show_popup` itself fails, mirroring
    /// [`Self::open_custom_popup`]'s failure handling.
    pub fn open_custom_menu(
        host: &dyn PopupHost,
        menu: &dyn MenuExt,
        anchor: &PopupAnchor,
        work_area: Rect,
    ) -> Option<Rc<dyn PopupSurfaceHandle>> {
        let handle_slot: Rc<RefCell<Option<std::rc::Weak<dyn PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        let slot_clone = Rc::clone(&handle_slot);

        let on_close: Rc<dyn Fn()> = Rc::new(move || {
            let weak_opt = slot_clone.borrow_mut().take();
            if let Some(weak_handle) = weak_opt {
                let handle: Option<Rc<dyn PopupSurfaceHandle>> = weak_handle.upgrade();
                if let Some(handle) = handle {
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
        let position =
            calculate_popup_placement(anchor, popup_size, work_area, PopupPlacement::AutoFlip);
        let Some(inner_handle) = host.show_popup(PopupRequest {
            content: Rc::clone(&content),
            position,
            size: popup_size,
            focus_policy: PopupFocusPolicy::Root,
            dismiss_policy: PopupDismissPolicy::LightDismiss,
        }) else {
            unmount_subtree(&content);
            return None;
        };

        let wrapped_handle: Rc<dyn PopupSurfaceHandle> = Rc::new(CustomMenuPopupHandle {
            inner: inner_handle,
            closed: std::cell::Cell::new(false),
        });
        *handle_slot.borrow_mut() = Some(Rc::downgrade(&wrapped_handle));
        Some(wrapped_handle)
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
    pub fn build_menu_view(menu: &dyn MenuExt, on_close: Rc<dyn Fn()>) -> Rc<dyn UIElementExt> {
        let layout = crate::ui::VerticalLayout::new();
        layout.set_margin(4.0);
        layout.set_background(Some(crate::graphics::Color::rgb(45, 45, 48).into()));
        layout.set_tab_stop(true);

        let items = menu.items().to_vec();
        // §2.9 icon column: reserved on every row once any item in the menu has an icon, so
        // labels stay aligned across icon and icon-less rows; otherwise the layout is unchanged
        // from before icon support existed.
        let any_icon = items.iter().any(|item| item.icon().is_some());
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
                let enabled = items_for_highlight
                    .get(i)
                    .map(|it| it.enabled())
                    .unwrap_or(true);
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
            if any_icon {
                row.set_spacing(6.0);
            }

            let enabled = item.enabled();
            let text_color = if enabled {
                crate::graphics::Color::rgb(240, 240, 240)
            } else {
                crate::graphics::Color::rgb(128, 128, 128)
            };

            if any_icon {
                let icon_slot = crate::ui::IconSourceElement::new();
                icon_slot.set_width(16.0);
                icon_slot.set_height(16.0);
                icon_slot.set_foreground(Some(text_color.into()));
                // Failure/unknown-icon semantics (§2.11): an absent `icon` (or, in the backend
                // layer, a failed user-image decode) simply leaves this slot's source unset —
                // never removes the row, never touches text/enabled/shortcut/on_select.
                if let Some(icon) = item.icon() {
                    icon_slot.set_icon_source(Some(icon));
                }
                crate::ui::LayoutExt::children(&*row)
                    .add(Rc::clone(&icon_slot) as Rc<dyn UIElementExt>);
            }

            let label = crate::ui::TextBlock::new();
            label.set_text(&item.text());
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
                crate::ui::LayoutExt::children(&*row)
                    .add(Rc::clone(&shortcut_label) as Rc<dyn UIElementExt>);
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
                            if items_for_key
                                .get(next)
                                .map(|it| it.enabled())
                                .unwrap_or(false)
                            {
                                sel_key.set(Some(next));
                                hl_key();
                                break;
                            }
                            next = (next + 1) % items_len;
                        }
                    }
                    crate::input::Key::Up => {
                        let current = sel_key.get().unwrap_or(0);
                        let mut prev = if current == 0 {
                            items_len.saturating_sub(1)
                        } else {
                            current - 1
                        };
                        for _ in 0..items_len {
                            if items_for_key
                                .get(prev)
                                .map(|it| it.enabled())
                                .unwrap_or(false)
                            {
                                sel_key.set(Some(prev));
                                hl_key();
                                break;
                            }
                            prev = if prev == 0 {
                                items_len.saturating_sub(1)
                            } else {
                                prev - 1
                            };
                        }
                    }
                    crate::input::Key::Home => {
                        for idx in 0..items_len {
                            if items_for_key
                                .get(idx)
                                .map(|it| it.enabled())
                                .unwrap_or(false)
                            {
                                sel_key.set(Some(idx));
                                hl_key();
                                break;
                            }
                        }
                    }
                    crate::input::Key::End => {
                        for idx in (0..items_len).rev() {
                            if items_for_key
                                .get(idx)
                                .map(|it| it.enabled())
                                .unwrap_or(false)
                            {
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
    /// Displays a popup surface according to `request`, or `None` if the backend could not show it
    /// (e.g. WinUI3 coordinate conversion or `Popup` construction failure). Callers
    /// (`ContextMenuService::open_custom_popup`/`open_custom_menu`) are responsible for unmounting
    /// `request.content` when this returns `None` — a backend must never return a handle that
    /// wraps a nonexistent native surface merely to satisfy an infallible-looking call site.
    fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{Brush, IconSource, ImageSource, SystemIcon, VectorNode, VectorPaint};
    use crate::ui::testsupport::FakeMenu;
    use crate::ui::{
        HorizontalLayout, IconSourceElement, LayoutExt, ListExt, MenuItemExt, TextBlock,
        VerticalLayout,
    };
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
        /// When `true`, `show_popup` returns `None` without recording anything in `shown` —
        /// simulates a backend show failure (e.g. WinUI3 coordinate conversion), §8-§12.
        fail_show: Cell<bool>,
    }

    impl FakePopupHost {
        fn new() -> Self {
            Self {
                shown: RefCell::new(Vec::new()),
                closed: Rc::new(Cell::new(false)),
                fail_show: Cell::new(false),
            }
        }

        fn new_failing() -> Self {
            let host = Self::new();
            host.fail_show.set(true);
            host
        }
    }

    impl PopupHost for FakePopupHost {
        fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>> {
            if self.fail_show.get() {
                return None;
            }
            self.shown.borrow_mut().push((
                Rc::clone(&request.content),
                request.position,
                request.size,
            ));
            Some(Rc::new(FakePopupHandle {
                closed: Rc::clone(&self.closed),
            }))
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
            ResolvedContextDefinition::Menu {
                menu: m,
                presentation,
            } => {
                assert!(Rc::ptr_eq(&m, &(menu as Rc<dyn MenuExt>)));
                assert_eq!(presentation, ContextMenuPresentation::Native);
            }
            _ => panic!("expected Menu definition"),
        }
    }

    #[test]
    fn resolve_target_own_context_popup() {
        let node = VerticalLayout::new();
        let template = ViewFactory::new(|_ctx| {
            let layout = VerticalLayout::new();
            let label = TextBlock::new();
            layout
                .children()
                .add(Rc::clone(&label) as Rc<dyn UIElementExt>);
            Some(layout as Rc<dyn UIElementExt>)
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
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();
        let template = ViewFactory::new(|_ctx| {
            let block = TextBlock::new();
            block.set_width(120.0);
            block.set_height(80.0);
            Some(block as Rc<dyn UIElementExt>)
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
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        )
        .expect("owner is alive, template should build");

        assert_eq!(host.shown.borrow().len(), 1);
        let (_, pos, size) = &host.shown.borrow()[0];
        assert_eq!(*pos, Point { x: 50.0, y: 50.0 });
        assert_eq!(
            *size,
            Size {
                width: 120.0,
                height: 80.0
            }
        );

        assert!(!host.closed.get());
        handle.close();
        assert!(host.closed.get());
    }

    #[test]
    fn open_custom_popup_returns_none_when_template_declines_to_build() {
        // Simulates the "owner already dropped" case: the factory itself decides not to build
        // (mirroring what a codegen-generated factory does once `ViewBuildContext::owner` fails to
        // upgrade), independent of which `Rc<dyn UIElementExt>` is passed as `owner` here.
        let host = FakePopupHost::new();
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();
        let template = ViewFactory::new(|_ctx| None);

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let handle = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        );
        assert!(handle.is_none());
        assert_eq!(host.shown.borrow().len(), 0);
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

        let handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area)
            .expect("host should show successfully");

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
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let _handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

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
        assert!(
            selected.get(),
            "menu item on_select callback should be invoked"
        );
        assert!(
            host.closed.get(),
            "popup surface handle close() should be invoked on item selection"
        );
    }

    #[test]
    fn resolve_target_ancestor_fallback() {
        let root = VerticalLayout::new();
        let menu = FakeMenu::new();
        root.set_context_menu(Some(Rc::clone(&menu) as Rc<dyn MenuExt>));

        let child = TextBlock::new();
        root.children()
            .add(Rc::clone(&child) as Rc<dyn UIElementExt>);

        let root_dyn: Rc<dyn UIElementExt> = root;
        let child_dyn: Rc<dyn UIElementExt> = child;

        // child has no context menu, so lookup finds root
        let resolved =
            resolve_context_target(&child_dyn).expect("should resolve ancestor context target");
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

        let resolved =
            resolve_context_target(&leaf_dyn).expect("should resolve nearest context target");
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
        root.children()
            .add(Rc::clone(&leaf) as Rc<dyn UIElementExt>);

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
        root.children()
            .add(Rc::clone(&child) as Rc<dyn UIElementExt>);

        let leaf = TextBlock::new();
        child
            .children()
            .add(Rc::clone(&leaf) as Rc<dyn UIElementExt>);

        let leaf_dyn: Rc<dyn UIElementExt> = leaf;
        let effective = leaf_dyn.effective_environment();
        assert_eq!(effective.get::<TestKey>(), 42);

        let captured_env: Rc<RefCell<Option<EnvironmentContext>>> = Rc::new(RefCell::new(None));
        let captured_clone = Rc::clone(&captured_env);

        let template = ViewFactory::new(move |ctx| {
            *captured_clone.borrow_mut() = Some(ctx.environment.clone());
            let b = TextBlock::new();
            Some(b as Rc<dyn UIElementExt>)
        });

        let host = FakePopupHost::new();
        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        ContextMenuService::open_custom_popup(
            &host,
            &leaf_dyn,
            &template,
            &anchor,
            leaf_dyn.effective_environment(),
            work_area,
        );

        let resolved_env = captured_env
            .borrow()
            .clone()
            .expect("popup should capture environment");
        assert_eq!(resolved_env.get::<TestKey>(), 42);
    }

    #[test]
    fn open_custom_popup_derives_environment_without_mutating_owner() {
        let owner = VerticalLayout::new();
        let owner_env = EnvironmentContext::root();
        owner_env.set::<TestKey>(7);
        owner.set_environment_context(owner_env.clone());
        let owner_dyn: Rc<dyn UIElementExt> = owner;

        let captured_env: Rc<RefCell<Option<EnvironmentContext>>> = Rc::new(RefCell::new(None));
        let captured_clone = Rc::clone(&captured_env);
        let template = ViewFactory::new(move |ctx| {
            *captured_clone.borrow_mut() = Some(ctx.environment.clone());
            Some(TextBlock::new() as Rc<dyn UIElementExt>)
        });

        let host = FakePopupHost::new();
        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        ContextMenuService::open_custom_popup(
            &host,
            &owner_dyn,
            &template,
            &anchor,
            owner_dyn.effective_environment(),
            work_area,
        );

        let popup_env = captured_env
            .borrow()
            .clone()
            .expect("popup should capture environment");
        // The derived popup Environment inherits the owner's value...
        assert_eq!(popup_env.get::<TestKey>(), 7);
        // ...but is a distinct derived context, and a popup-side override never leaks back to the
        // owner's own Environment.
        popup_env.set::<TestKey>(99);
        assert_eq!(popup_env.get::<TestKey>(), 99);
        assert_eq!(owner_env.get::<TestKey>(), 7);
    }

    #[test]
    fn open_custom_popup_installs_dismiss_action_that_closes_popup() {
        let host = FakePopupHost::new();
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();

        let captured_dismiss: Rc<RefCell<Option<PopupDismissAction>>> = Rc::new(RefCell::new(None));
        let captured_clone = Rc::clone(&captured_dismiss);
        let template = ViewFactory::new(move |ctx| {
            *captured_clone.borrow_mut() = ctx.environment.get::<PopupDismissActionKey>();
            Some(TextBlock::new() as Rc<dyn UIElementExt>)
        });

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let handle = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        )
        .expect("owner is alive, template should build");

        let dismiss = captured_dismiss
            .borrow()
            .clone()
            .expect("PopupDismissAction should be installed in the popup-scoped Environment");
        assert!(!host.closed.get());
        dismiss.dismiss();
        assert!(
            host.closed.get(),
            "dismiss action should close the popup surface"
        );

        // Idempotent: dismissing again after the handle is already dropped/closed must not panic.
        dismiss.dismiss();
        let _ = handle;
    }

    #[test]
    fn unmount_subtree_reentrant_from_within_own_event_dispatch_does_not_panic() {
        // Simulates the AppKit/WinUI3 backend's `close()` sequence: `unmount_subtree` runs
        // synchronously on the popup content root from *inside* a handler currently being
        // dispatched on that very tree (a `Button`'s own `on_tapped`, wired the same way a
        // declarative `context_popup` dismiss action's target would be) — the reentrancy shape
        // teardown-before-detach requires be safe. `unmount_subtree` walks/mutates only the
        // `UIElementExt` tree's own `visual_collection`/lifecycle state, never a backend host's own
        // `RefCell`s, so this must not panic regardless of which handler triggered it.
        let host = FakePopupHost::new();
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();

        let template = ViewFactory::new(|_ctx| {
            let layout = VerticalLayout::new();
            let button = TextBlock::new();
            let button_dyn: Rc<dyn UIElementExt> = Rc::clone(&button) as Rc<dyn UIElementExt>;
            let weak_root: Rc<RefCell<Option<std::rc::Weak<dyn UIElementExt>>>> =
                Rc::new(RefCell::new(None));
            let weak_root_for_handler = Rc::clone(&weak_root);
            button.register_routed_handler::<crate::input::TappedEventArgs>(
                "on_tapped",
                Box::new(move |_args, _routed| {
                    if let Some(root) = weak_root_for_handler
                        .borrow()
                        .as_ref()
                        .and_then(|w| w.upgrade())
                    {
                        crate::ui::unmount_subtree(&root);
                    }
                }),
            );
            layout.children().add(button_dyn);
            let root: Rc<dyn UIElementExt> = layout;
            *weak_root.borrow_mut() = Some(Rc::downgrade(&root));
            Some(root)
        });

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let _handle = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        )
        .expect("owner is alive, template should build");

        let content = Rc::clone(&host.shown.borrow()[0].0);
        let button_child = Rc::clone(&content.visual_children()[0]);

        let routed_args = crate::input::RoutedEventArgs::default();
        crate::ui::dispatch_routed(
            &button_child,
            "on_tapped",
            &crate::input::TappedEventArgs {
                position: Point { x: 0.0, y: 0.0 },
                modifiers: crate::input::KeyModifiers::default(),
            },
            &routed_args,
        );
        // Reaching here without panicking is the assertion.
    }

    #[test]
    fn unmount_hook_observes_intact_environment_before_backend_would_detach() {
        // Regression for teardown-before-detach, distinguishing it from detach-before-unmount (not
        // just "did not panic"): a `close()` that got the order backwards would let a backend
        // clear/detach native state before `unmount_subtree`'s hooks ever run, so by the time an
        // `on_unmount`-equivalent hook executes, `effective_environment()` would already be gone.
        // This proves the opposite: content's own Environment is still fully resolvable *during* the
        // unmount hook, and becomes unset only once `unmount()` itself (called by `unmount_subtree`)
        // actually runs — see `InnerPopupSurface::close()`'s own doc comment on both backends for the
        // native-call-order half of this contract (not independently testable here without a live
        // window; enforced by code review + this ordering invariant).
        let host = FakePopupHost::new();
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();

        let observed_during_hook: Rc<RefCell<Option<u32>>> = Rc::new(RefCell::new(None));
        let observed_clone = Rc::clone(&observed_during_hook);

        let template = ViewFactory::new(move |_ctx| {
            let root: Rc<dyn UIElementExt> = TextBlock::new();
            let observed_for_hook = Rc::clone(&observed_clone);
            let root_for_hook = Rc::clone(&root);
            root.add_unmount_hook(Box::new(move || {
                *observed_for_hook.borrow_mut() =
                    Some(root_for_hook.effective_environment().get::<TestKey>());
            }));
            Some(root)
        });

        let popup_env = EnvironmentContext::root();
        popup_env.set::<TestKey>(99);

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let _handle = ContextMenuService::open_custom_popup(
            &host, &owner, &template, &anchor, popup_env, work_area,
        )
        .expect("owner is alive, template should build");

        let content = Rc::clone(&host.shown.borrow()[0].0);
        assert!(
            observed_during_hook.borrow().is_none(),
            "the unmount hook must not have run yet before close/unmount"
        );

        // Simulate exactly what both backends' `close()` now correctly do: unmount before any
        // native/host detach.
        crate::ui::unmount_subtree(&content);

        assert_eq!(
            *observed_during_hook.borrow(),
            Some(99),
            "on_unmount must observe the popup-scoped Environment while it is still intact, \
             before any detach"
        );
        assert!(
            content.environment_context().is_none(),
            "environment_context must be cleared only as part of unmount() itself, after the \
             hook already ran — proves hook-then-clear, not clear-then-hook"
        );
    }

    #[test]
    fn open_custom_popup_dismiss_during_show_popup_is_not_lost_or_reopened() {
        // Distinct from `open_custom_popup_dismiss_during_build_prevents_the_popup_from_showing`:
        // here `dismiss()` fires from *inside* `host.show_popup` itself (a backend's native "show"
        // call can dispatch reentrantly), i.e. after `ViewFactory::build` already returned and
        // `PopupDismissState` is still `Building` (the state doesn't move to `Open` until
        // `open_custom_popup` gets control back with the real handle). Before the fix this window
        // let the dismiss request get silently overwritten by the unconditional post-show `Open`
        // assignment, since the handle didn't exist yet when dismiss() ran.
        struct DismissDuringShowHandle {
            content: RefCell<Option<Rc<dyn UIElementExt>>>,
            close_count: Rc<Cell<u32>>,
        }
        impl PopupSurfaceHandle for DismissDuringShowHandle {
            fn close(&self) {
                self.close_count.set(self.close_count.get() + 1);
                if let Some(content) = self.content.borrow_mut().take() {
                    unmount_subtree(&content);
                }
            }
        }
        struct DismissDuringShowHost {
            close_count: Rc<Cell<u32>>,
        }
        impl PopupHost for DismissDuringShowHost {
            fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>> {
                // Obtains the installed PopupDismissAction from the content's own effective popup
                // Environment and calls it *before* returning the handle — simulating a backend
                // whose native "show" call reenters synchronously.
                let dismiss = request
                    .content
                    .effective_environment()
                    .get::<PopupDismissActionKey>();
                let handle: Rc<dyn PopupSurfaceHandle> = Rc::new(DismissDuringShowHandle {
                    content: RefCell::new(Some(Rc::clone(&request.content))),
                    close_count: Rc::clone(&self.close_count),
                });
                if let Some(dismiss) = dismiss {
                    dismiss.dismiss();
                }
                Some(handle)
            }
        }

        let close_count = Rc::new(Cell::new(0));
        let host = DismissDuringShowHost {
            close_count: Rc::clone(&close_count),
        };
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();

        let weak_content: Rc<RefCell<Option<Weak<dyn UIElementExt>>>> = Rc::new(RefCell::new(None));
        let weak_clone = Rc::clone(&weak_content);
        let template = ViewFactory::new(move |_ctx| {
            let root: Rc<dyn UIElementExt> = TextBlock::new();
            *weak_clone.borrow_mut() = Some(Rc::downgrade(&root));
            Some(root)
        });

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let result = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        );

        assert!(
            result.is_none(),
            "a popup dismissed during host.show_popup itself must not be returned as open"
        );
        assert_eq!(
            close_count.get(),
            1,
            "the handle created during show_popup must be closed exactly once by open_custom_popup \
             (not left open, and not closed twice)"
        );
        let weak = weak_content
            .borrow()
            .clone()
            .expect("template captured its content");
        assert!(
            weak.upgrade().is_none(),
            "content built before a dismiss-during-show must be unmounted and released, not \
             retained via the never-returned handle"
        );
    }

    #[test]
    fn open_custom_popup_dismiss_during_build_prevents_the_popup_from_showing() {
        // Simulates a generated declarative Component's `on_mount { dismiss(); }` (#162) calling the
        // popup_dismiss action synchronously from *inside* `ViewFactory::build` — before any native
        // surface exists. The popup must never be shown at all (not shown-then-immediately-closed),
        // and the already-built content must still be unmounted exactly once.
        let host = FakePopupHost::new();
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();

        let unmount_count = Rc::new(Cell::new(0));
        let unmount_count_for_hook = Rc::clone(&unmount_count);

        let template = ViewFactory::new(move |ctx| {
            let root: Rc<dyn UIElementExt> = TextBlock::new();
            let count_for_hook = Rc::clone(&unmount_count_for_hook);
            root.add_unmount_hook(Box::new(move || {
                count_for_hook.set(count_for_hook.get() + 1);
            }));
            // The pre-show dismiss call itself — mirrors what a generated Component's own
            // `on_mount` will invoke via `#[environment(popup_dismiss)]` once #162 lands.
            if let Some(dismiss) = ctx.environment.get::<PopupDismissActionKey>() {
                dismiss.dismiss();
            }
            Some(root)
        });

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let handle = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        );

        assert!(
            handle.is_none(),
            "a popup dismissed during build/mount must not be shown"
        );
        assert_eq!(
            host.shown.borrow().len(),
            0,
            "the popup host's show_popup must never be called once a pre-show dismiss was requested"
        );
        assert_eq!(
            unmount_count.get(),
            1,
            "content built before the pre-show dismiss must still be unmounted exactly once"
        );
    }

    #[test]
    fn open_custom_popup_unmounts_and_returns_none_when_backend_show_fails() {
        // Simulates a WinUI3-style backend show failure (coordinate conversion, Popup construction)
        // — content may already carry mounted Component state by this point; it must be torn down
        // rather than leaked mounted-but-never-shown, and no active popup handle is ever produced.
        let host = FakePopupHost::new_failing();
        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();

        let unmount_count = Rc::new(Cell::new(0));
        let unmount_count_for_hook = Rc::clone(&unmount_count);
        let template = ViewFactory::new(move |_ctx| {
            let root: Rc<dyn UIElementExt> = TextBlock::new();
            let count_for_hook = Rc::clone(&unmount_count_for_hook);
            root.add_unmount_hook(Box::new(move || {
                count_for_hook.set(count_for_hook.get() + 1);
            }));
            Some(root)
        });

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let handle = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        );

        assert!(
            handle.is_none(),
            "open_custom_popup must return None when the backend show fails"
        );
        assert_eq!(host.shown.borrow().len(), 0);
        assert_eq!(
            unmount_count.get(),
            1,
            "built content must be unmounted exactly once on show failure"
        );
    }

    #[test]
    fn open_custom_menu_unmounts_and_returns_none_when_backend_show_fails() {
        let host = FakePopupHost::new_failing();
        let menu = FakeMenu::new();

        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

        assert!(
            handle.is_none(),
            "open_custom_menu must return None when the backend show fails"
        );
        assert_eq!(host.shown.borrow().len(), 0);
    }

    #[test]
    fn popup_surface_handle_releases_content_after_close_not_just_unmounted() {
        // A `PopupSurfaceHandle` implementation that retains `content: Rc<dyn UIElementExt>` (rather
        // than `RefCell<Option<Rc<..>>>` + `take()` in `close()`) would unmount content correctly but
        // still keep it alive for as long as the handle itself is reachable (e.g. via a host's
        // `active_popup` field, until replaced or dropped). This test distinguishes "unmounted but
        // retained" from "unmounted and released" using a `PopupSurfaceHandle` that follows the fixed
        // ownership pattern both `elwindui-backend-appkit`'s and `elwindui-backend-winui3`'s
        // `InnerPopupSurface` now use (`content: RefCell<Option<Rc<..>>>`, taken in `close()`).
        struct ReleasingPopupHandle {
            content: RefCell<Option<Rc<dyn UIElementExt>>>,
            closed: Cell<bool>,
        }
        impl PopupSurfaceHandle for ReleasingPopupHandle {
            fn close(&self) {
                if !self.closed.get() {
                    self.closed.set(true);
                    if let Some(content) = self.content.borrow_mut().take() {
                        unmount_subtree(&content);
                    }
                }
            }
        }
        impl Drop for ReleasingPopupHandle {
            fn drop(&mut self) {
                self.close();
            }
        }
        struct ReleasingPopupHost;
        impl PopupHost for ReleasingPopupHost {
            fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>> {
                Some(Rc::new(ReleasingPopupHandle {
                    content: RefCell::new(Some(request.content)),
                    closed: Cell::new(false),
                }))
            }
        }

        let owner: Rc<dyn UIElementExt> = VerticalLayout::new();
        let weak_content: Rc<RefCell<Option<Weak<dyn UIElementExt>>>> = Rc::new(RefCell::new(None));
        let weak_clone = Rc::clone(&weak_content);
        let template = ViewFactory::new(move |_ctx| {
            let root: Rc<dyn UIElementExt> = TextBlock::new();
            *weak_clone.borrow_mut() = Some(Rc::downgrade(&root));
            Some(root)
        });

        let host = ReleasingPopupHost;
        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };

        let handle = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            EnvironmentContext::default(),
            work_area,
        )
        .expect("owner is alive, template should build");

        let weak = weak_content
            .borrow()
            .clone()
            .expect("template captured its content");
        assert!(
            weak.upgrade().is_some(),
            "content must be alive while the popup is open"
        );

        handle.close();
        drop(handle);

        assert!(
            weak.upgrade().is_none(),
            "a PopupSurfaceHandle must release its retained content after close(), not merely \
             unmount it in place — a surface keeping a bare `content: Rc<..>` field would fail this"
        );
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
        let menu_view =
            ContextMenuPresenter::build_menu_view(&*menu, Rc::new(move || closed_clone.set(true)));

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

    /// §8.2: `set_icon` replaces then clears, and never touches `text`/`shortcut`/`enabled`.
    #[test]
    fn menu_item_icon_set_replace_clear_preserves_other_state() {
        let item = crate::ui::testsupport::FakeMenuItem::new();
        item.set_text("Item");
        item.set_shortcut("X");
        item.set_enabled(true);

        item.set_icon(Some(IconSource::System(SystemIcon::Copy)));
        match item.icon() {
            Some(IconSource::System(SystemIcon::Copy)) => {}
            other => panic!("expected Some(System(Copy)) after first set_icon, got {other:?}"),
        }

        item.set_icon(Some(IconSource::System(SystemIcon::Delete)));
        match item.icon() {
            Some(IconSource::System(SystemIcon::Delete)) => {}
            other => {
                panic!("expected Some(System(Delete)) after replacing set_icon, got {other:?}")
            }
        }

        item.set_icon(None);
        assert!(item.icon().is_none(), "set_icon(None) must clear the icon");

        assert_eq!(item.text(), "Item");
        assert_eq!(item.shortcut().as_deref(), Some("X"));
        assert!(item.enabled());
    }

    /// The row's leading icon slot — `None` unless the row's first child is actually an
    /// `IconSourceElement`
    /// (as opposed to the label `TextBlock`, which is first when the menu has no icon column at
    /// all). Still type-erased but `Rc`-owned so callers can inspect the concrete icon element.
    fn icon_slot(row: &Rc<dyn UIElementExt>) -> Option<Rc<dyn UIElementExt>> {
        let row_layout = row.as_any().downcast_ref::<HorizontalLayout>()?;
        let first = crate::ui::LayoutExt::children(row_layout)
            .to_vec()
            .into_iter()
            .next()?;
        if first.as_any().downcast_ref::<IconSourceElement>().is_some() {
            Some(first)
        } else {
            None
        }
    }

    /// §8.5/§8.6: a leading 16x16 icon slot is reserved on every row once *any* item in the menu
    /// has an icon (aligning icon-less rows' labels with icon rows'), and no such column exists at
    /// all when no item has an icon (the pre-icon-support layout, unchanged).
    #[test]
    fn build_menu_view_reserves_icon_column_only_when_any_item_has_icon() {
        // No icons anywhere: no row gets a leading icon slot.
        let plain_menu = FakeMenu::new();
        let plain_item = crate::ui::testsupport::FakeMenuItem::new();
        plain_item.set_text("Plain");
        plain_menu.add(Rc::clone(&plain_item) as Rc<dyn MenuItemExt>);
        let plain_view = ContextMenuPresenter::build_menu_view(&*plain_menu, Rc::new(|| {}));
        let plain_layout = plain_view
            .as_any()
            .downcast_ref::<VerticalLayout>()
            .expect("build_menu_view returns a VerticalLayout");
        let plain_rows = crate::ui::LayoutExt::children(plain_layout).to_vec();
        assert_eq!(plain_rows.len(), 1);
        assert!(
            icon_slot(&plain_rows[0]).is_none(),
            "a menu with no icons must not gain an icon column"
        );

        // Mixed menu: SystemIcon item, icon-less item, user ImageSource item — all three rows must
        // reserve the same leading IconSourceElement slot.
        let mixed_menu = FakeMenu::new();
        let with_system_icon = crate::ui::testsupport::FakeMenuItem::new();
        with_system_icon.set_text("System");
        with_system_icon.set_icon(Some(IconSource::System(SystemIcon::Copy)));
        let without_icon = crate::ui::testsupport::FakeMenuItem::new();
        without_icon.set_text("No Icon");
        let with_user_icon = crate::ui::testsupport::FakeMenuItem::new();
        with_user_icon.set_text("User");
        let bitmap = crate::graphics::BitmapImage::from_rgba8(
            1,
            1,
            4,
            vec![255u8, 0, 0, 255],
            crate::graphics::AlphaMode::Straight,
        )
        .expect("1x1 rgba8 buffer is well-formed");
        with_user_icon.set_icon(Some(IconSource::Image(ImageSource::Raster(bitmap))));
        mixed_menu.add(Rc::clone(&with_system_icon) as Rc<dyn MenuItemExt>);
        mixed_menu.add(Rc::clone(&without_icon) as Rc<dyn MenuItemExt>);
        mixed_menu.add(Rc::clone(&with_user_icon) as Rc<dyn MenuItemExt>);
        let mixed_view = ContextMenuPresenter::build_menu_view(&*mixed_menu, Rc::new(|| {}));
        let mixed_layout = mixed_view
            .as_any()
            .downcast_ref::<VerticalLayout>()
            .expect("build_menu_view returns a VerticalLayout");
        let mixed_rows = crate::ui::LayoutExt::children(mixed_layout).to_vec();
        assert_eq!(mixed_rows.len(), 3);
        for (i, row) in mixed_rows.iter().enumerate() {
            assert!(
                icon_slot(row).is_some(),
                "row {i} must reserve the leading icon slot once any item in the menu has an icon"
            );
        }
        // The icon-less row's slot has no source set (empty slot, not a shifted label).
        let empty_slot = icon_slot(&mixed_rows[1]).expect("row 1 has an icon slot");
        let empty_icon = empty_slot
            .as_any()
            .downcast_ref::<IconSourceElement>()
            .expect("icon slot is an IconSourceElement");
        assert!(empty_icon.icon_source().is_none());
    }

    /// §8.7: a disabled item's canonical `SystemIcon` vector uses the disabled foreground color
    /// path (the same color `build_menu_view` gives the disabled label), not the enabled one.
    #[test]
    fn build_menu_view_disabled_system_icon_uses_disabled_color() {
        let menu = FakeMenu::new();
        let disabled_item = crate::ui::testsupport::FakeMenuItem::new();
        disabled_item.set_text("Disabled");
        disabled_item.set_enabled(false);
        disabled_item.set_icon(Some(IconSource::System(SystemIcon::Delete)));
        menu.add(Rc::clone(&disabled_item) as Rc<dyn MenuItemExt>);

        let view = ContextMenuPresenter::build_menu_view(&*menu, Rc::new(|| {}));
        let layout = view
            .as_any()
            .downcast_ref::<VerticalLayout>()
            .expect("build_menu_view returns a VerticalLayout");
        let rows = crate::ui::LayoutExt::children(layout).to_vec();
        let slot = icon_slot(&rows[0]).expect("disabled row has an icon slot");
        let icon_element = slot
            .as_any()
            .downcast_ref::<IconSourceElement>()
            .expect("icon slot is an IconSourceElement");
        let mut commands = Vec::new();
        icon_element.render(&mut crate::graphics::RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        let Some(crate::graphics::RenderCommand::DrawVectorImage { image: vector, .. }) =
            commands.first()
        else {
            panic!("SystemIcon must emit the Core canonical vector fallback");
        };
        let node = vector
            .root()
            .children
            .first()
            .expect("canonical icon geometry has at least one node");
        let color = match node {
            VectorNode::Path(path_node) => match (&path_node.fill, &path_node.stroke) {
                (Some(fill), None) => match &fill.paint {
                    VectorPaint::Brush(Brush::Solid(color)) => *color,
                    other => panic!("unexpected fill paint: {other:?}"),
                },
                (None, Some(stroke)) => match &stroke.paint {
                    VectorPaint::Brush(Brush::Solid(color)) => *color,
                    other => panic!("unexpected stroke paint: {other:?}"),
                },
                other => panic!("expected exactly one of fill/stroke, got {other:?}"),
            },
            other => panic!("expected a path node, got {other:?}"),
        };
        // Matches the disabled label color `build_menu_view` itself uses.
        assert_eq!(color, crate::graphics::Color::rgb(128, 128, 128));
    }

    /// §8.8: keyboard navigation/selection/close-once behavior is unchanged when icon slots are
    /// present (regression against the pre-icon-support behavior already covered by
    /// `custom_menu_keyboard_navigation_and_item_state` above).
    #[test]
    fn custom_menu_keyboard_navigation_still_works_with_icons_present() {
        let menu = FakeMenu::new();

        let item1 = crate::ui::testsupport::FakeMenuItem::new();
        item1.set_text("Item 1");
        item1.set_icon(Some(IconSource::System(SystemIcon::Cut)));
        let item1_selected = Rc::new(Cell::new(false));
        let item1_sel_clone = Rc::clone(&item1_selected);
        item1.set_on_select(Box::new(move || item1_sel_clone.set(true)));

        let item2_disabled = crate::ui::testsupport::FakeMenuItem::new();
        item2_disabled.set_text("Item 2 Disabled");
        item2_disabled.set_enabled(false);
        item2_disabled.set_icon(Some(IconSource::System(SystemIcon::Delete)));
        let item2_selected = Rc::new(Cell::new(false));
        let item2_sel_clone = Rc::clone(&item2_selected);
        item2_disabled.set_on_select(Box::new(move || item2_sel_clone.set(true)));

        let item3 = crate::ui::testsupport::FakeMenuItem::new();
        item3.set_text("Item 3");
        // Deliberately icon-less, to also cover a mixed icon/no-icon row during keyboard nav.
        let item3_selected = Rc::new(Cell::new(false));
        let item3_sel_clone = Rc::clone(&item3_selected);
        item3.set_on_select(Box::new(move || item3_sel_clone.set(true)));

        menu.add(Rc::clone(&item1) as Rc<dyn MenuItemExt>);
        menu.add(Rc::clone(&item2_disabled) as Rc<dyn MenuItemExt>);
        menu.add(Rc::clone(&item3) as Rc<dyn MenuItemExt>);

        let closed = Rc::new(Cell::new(false));
        let close_count = Rc::new(Cell::new(0u32));
        let closed_clone = Rc::clone(&closed);
        let close_count_clone = Rc::clone(&close_count);
        let menu_view = ContextMenuPresenter::build_menu_view(
            &*menu,
            Rc::new(move || {
                closed_clone.set(true);
                close_count_clone.set(close_count_clone.get() + 1);
            }),
        );

        let routed_args = crate::input::RoutedEventArgs::default();
        for key in [
            crate::input::Key::Down,
            crate::input::Key::Down,
            crate::input::Key::Enter,
        ] {
            crate::ui::dispatch_routed(
                &menu_view,
                "on_key_down",
                &crate::input::KeyEventArgs {
                    key,
                    modifiers: crate::input::KeyModifiers::default(),
                    is_repeat: false,
                },
                &routed_args,
            );
        }

        assert!(!item1_selected.get());
        assert!(!item2_selected.get());
        assert!(
            item3_selected.get(),
            "second enabled row (item3) must be selected"
        );
        assert!(closed.get());
        assert_eq!(close_count.get(), 1, "popup must close exactly once");
    }
}
