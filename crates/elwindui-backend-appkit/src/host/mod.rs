//! The tree host: one `NSView` subclass that reflects an `elwindui_core` element tree into real
//! `NSView` subviews and `CALayer` sublayers, and feeds native events back into core's
//! pointer/keyboard/focus dispatchers.
//!
//! `InnerWindow`'s content view and `InnerTabView`'s per-tab content area are each one of these.
//! Depends downward on `render` for all drawing; `replay` below is the pass that consumes this
//! view's own layer caches, which is why it lives here rather than under `render`.

use crate::ffi::{AnyView, mtm};
use elwindui_core::base::{Point, Rect};
use elwindui_core::input::{
    FocusState, KeyModifiers, KeyboardDispatcher, MouseButton, PointerDispatcher, RawKeyEvent,
    RawKeyEventKind, RawPointerEvent, RawPointerEventKind, RawTextInputEvent,
};
use elwindui_core::ui::popup::PopupSurfaceHandle;
use elwindui_core::ui::{
    ContextMenuPresentation, ContextMenuService, ContextRequest, FocusHost, InvalidationKind,
    RelayoutHost, ResolvedContextDefinition, UIElementExt, layout_root,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSMenu, NSScreen, NSTrackingArea, NSTrackingAreaOptions, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

mod event;
mod replay;

use event::*;
use replay::*;

/// The single reusable "reflect an `Rc<dyn elwindui_core::ui::UIElement>` into real `NSView`
/// subviews/`CAShapeLayer`/`CATextLayer` sublayers" host — `InnerWindow`'s content view and
/// `InnerTabView`'s per-tab content area both are one of these.
pub struct TreeHostIvars {
    pub(crate) tree: RefCell<Option<Rc<dyn UIElementExt>>>,
    /// The retained core-side rendering description for the currently hosted Visual tree.
    pub(crate) render_tree: RefCell<Option<elwindui_core::graphics::RenderTree>>,
    /// Native compositor islands, keyed by `AnyView` identity. They must survive ordinary
    /// relayouts so the first responder is not detached from the view hierarchy.
    pub(crate) native_containers: RefCell<HashMap<usize, Retained<NSView>>>,
    /// `AnyView` identity -> the owning `UIElement`'s `render_group_id`
    /// (`RenderCommand::NativeControl::owner_id`) — populated/pruned in lockstep with
    /// `native_containers`. Lets `ElwinduiWindow::makeFirstResponder:` resolve "which elwindui
    /// element does this native container belong to" without a second registry of its own; see
    /// `resolve_native_owner_id`.
    pub(crate) native_owner_ids: RefCell<HashMap<usize, u64>>,
    /// Everything a replay pass reads or writes besides the live `CALayer`/`NSView` tree itself
    /// (per-group container cache, image/vector-raster caches) — see `replay::ReplayState`'s own
    /// doc comment. Held as a single `RefCell` so a pass takes one borrow across its whole
    /// recursion instead of several small ones.
    pub(crate) replay_state: RefCell<ReplayState>,
    /// Set once, right after construction — lets `set_tree` hand out an `AppKitRelayoutHost`
    /// wrapping a weak reference back to this same view, without needing a `Retained<Self>` in
    /// hand at that point.
    pub(crate) weak_self: RefCell<objc2::rc::Weak<TreeHostView>>,
    /// Turns this view's own raw `NSEvent`s into `elwindui_core::ui::hit_test`/`dispatch_routed`
    /// calls against `tree` — see `elwindui_core::input::PointerDispatcher`'s own doc comment.
    /// The current backend range recorded in `docs/status/backend_status.md`: self-drawn
    /// elements only, since a native subview (`Button`/`TextArea`/`TabView`, laid out as its own
    /// `native_containers` island) receives the OS mouse event directly via ordinary AppKit
    /// hit-testing, never reaching this view's own overrides below at all.
    pub(crate) pointer: PointerDispatcher,
    /// Turns this view's own raw key/text events into `elwindui_core::ui::dispatch_routed` calls
    /// against whichever element currently has focus, and owns the `FocusTracker`/
    /// `ShortcutRegistry` for whatever tree this view hosts — see
    /// `elwindui_core::input::KeyboardDispatcher`'s own doc comment.
    /// The current backend range recorded in `docs/status/backend_status.md` mirrors
    /// `pointer`'s own: self-drawn
    /// elements' virtual focus is real (`KeyboardDispatcher::focus` is the single source of truth),
    /// but a native leaf (`Button`/`TextArea`/`TabView`) receives real OS keyboard focus/events
    /// directly and needs its own individual wiring (see `native_ui.rs`'s `Button`/`TextArea`) —
    /// this view's own `keyDown:`/`keyUp:` overrides below never even fire while one is focused.
    pub(crate) keyboard: KeyboardDispatcher,
    /// The single `NSTrackingArea` this view keeps registered for itself, so `updateTrackingAreas`
    /// can remove the previous one before installing a freshly-sized replacement rather than
    /// accumulating a new one on every resize.
    pub(crate) tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    /// `(width_unconstrained, height_unconstrained)` — `false`/`false` (the default, every existing
    /// host) means `relayout` measures against `self.frame()`'s own size, same as always. `true` on
    /// an axis means `relayout` measures that axis as `f32::INFINITY` instead, then grows `self`'s
    /// own frame to the resulting natural size on that axis — `InnerScrollView`'s content host uses
    /// this so its content reports/gets arranged at its true natural size, letting the enclosing
    /// `NSScrollView`'s native scroll physics do the rest, rather than being clamped to whatever
    /// viewport size happens to be available. See `set_unconstrained_axes`.
    pub(crate) unconstrained_axes: Cell<(bool, bool)>,
    /// `true` for the duration of a `relayout()` call — lets `AppKitRelayoutHost::request_relayout`
    /// detect a reentrant call (e.g. a `NativeControl`'s focus change synchronously running user
    /// code that calls `invalidate()`) and defer touching `render_tree` instead of trying to
    /// `borrow_mut()` it while `relayout()`'s own replay pass still holds it borrowed — see
    /// `pending_dirty_ids`/`needs_another_pass` and `relayout`'s own doc comment.
    pub(crate) relaying_out: Cell<bool>,
    /// Group ids `request_relayout` couldn't `mark_dirty` immediately because `relaying_out` was
    /// set — drained and applied right after the in-progress `relayout()` call finishes.
    pub(crate) pending_dirty_ids: RefCell<Vec<u64>>,
    /// Set by a reentrant `request_relayout` (alongside `pending_dirty_ids`) to tell `relayout()`
    /// it must schedule another pass once the current one finishes, since `setNeedsLayout(true)`
    /// called mid-pass is not guaranteed to still be honored once AppKit's own layout pass (which
    /// is what got this `relayout()` running in the first place) completes.
    pub(crate) needs_another_pass: Cell<bool>,
    /// The strongest `InvalidationKind` any `request_relayout` call has asked for since the last
    /// `relayout_inner` pass consumed it — `None` means no explicit request arrived (a `layout`
    /// callback can still fire for other reasons, e.g. a window resize; see `relayout_inner`'s own
    /// handling of `frame_changed`). Coalesced by `max` rather than overwritten, mirroring the
    /// WinUI3 backend's own `pending: Cell<bool>` coalescing (`WinUI3RelayoutHost`) — several
    /// `request_relayout` calls within one runloop turn collapse into a single pass at the
    /// strongest kind any of them needed.
    pub(crate) pending_invalidation: Cell<Option<InvalidationKind>>,
    /// `self.frame().size` as of the last `relayout_inner` pass — compared against the current
    /// frame at the top of the next pass so a resize (which `layout` can trigger with no
    /// `request_relayout` call at all) is never treated as a `Render`-only pass.
    pub(crate) last_layout_size: Cell<objc2_foundation::NSSize>,
    /// Whether this host currently participates in layout/render at all — the AppKit-side half of
    /// `docs/design/runtime/layout_design.md`'s "container participation" (a `Visible`-vs-
    /// `Collapsed` element's own `UIElementExt::participates_in_layout()` is the *other* half, and
    /// is orthogonal to this one: neither overwrites the other). `true` for every host by default
    /// (matches every existing host's behavior — only `InnerTabView`'s non-selected-tab hosts ever
    /// set this `false`, immediately after construction). See `set_active`'s own doc comment.
    pub(crate) active: Cell<bool>,
    /// Retained handle to any currently active custom popup or context menu surface.
    pub(crate) active_popup: RefCell<Option<Rc<dyn PopupSurfaceHandle>>>,
}

/// `elwindui_core::ui::RelayoutHost` for `TreeHostView` — wraps a *weak* reference back to the view
/// (not the view itself) since a strong one would create a reference cycle. `request_relayout`
/// silently does nothing if the view has since been deallocated (`load()` returns `None`).
pub(crate) struct AppKitRelayoutHost(objc2::rc::Weak<TreeHostView>);

impl RelayoutHost for AppKitRelayoutHost {
    fn request_relayout(&self, dirty_group_id: u64, kind: InvalidationKind) {
        let Some(view) = self.0.load() else { return };
        let previous = view.ivars().pending_invalidation.get();
        view.ivars()
            .pending_invalidation
            .set(Some(previous.map_or(kind, |p| p.max(kind))));
        if view.ivars().relaying_out.get() {
            // Reentrant call from inside `relayout()`'s own replay pass (see `relaying_out`'s own
            // doc comment) — `render_tree` is already borrowed by that pass, so defer the mark
            // instead of panicking on a double `borrow_mut()`.
            view.ivars()
                .pending_dirty_ids
                .borrow_mut()
                .push(dirty_group_id);
            view.ivars().needs_another_pass.set(true);
        } else if let Some(render_tree) = view.ivars().render_tree.borrow_mut().as_mut() {
            // Every kind still marks the group dirty — `Render` especially, since re-recording
            // this group's commands is the entire point of a paint-only invalidation.
            render_tree.mark_dirty(dirty_group_id);
        }
        view.setNeedsLayout(true);
    }
}

/// `elwindui_core::ui::FocusHost` for `TreeHostView` — the `FocusHost` counterpart to
/// `AppKitRelayoutHost`, same weak-back-reference shape. Delegates straight to
/// `TreeHostIvars::keyboard.focus`, the single source of truth for this view's own hosted tree.
pub(crate) struct AppKitFocusHost(objc2::rc::Weak<TreeHostView>);

impl FocusHost for AppKitFocusHost {
    fn request_focus(&self, target: &Rc<dyn UIElementExt>) -> bool {
        match self.0.load() {
            Some(view) => view
                .ivars()
                .keyboard
                .focus
                .set_focus(target, FocusState::Programmatic),
            None => false,
        }
    }
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = TreeHostIvars]
    pub struct TreeHostView;

    unsafe impl NSObjectProtocol for TreeHostView {}

    impl TreeHostView {
        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe {
                let _: () = msg_send![super(self), layout];
            }
            self.relayout();
        }

        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> objc2_foundation::NSSize {
            let size = self
                .ivars()
                .tree
                .borrow()
                .as_ref()
                .map(|tree| elwindui_core::ui::natural_size(&**tree))
                .unwrap_or(elwindui_core::base::Size { width: 0.0, height: 0.0 });
            objc2_foundation::NSSize::new(size.width as f64, size.height as f64)
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Fires when this view's backing store resolution changes — most commonly a window
        /// dragged between displays with different `backingScaleFactor`s, but also the first
        /// `viewDidMoveToWindow` after construction. `backing_scale_factor` (and therefore every
        /// hand-built `CALayer` this view paints, via `render::add_sublayer_scaled`) is derived
        /// from `NSWindow.backingScaleFactor`, so it must be re-applied here rather than assumed
        /// to update on its own.
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            unsafe {
                let _: () = msg_send![super(self), viewDidChangeBackingProperties];
            }
            // Rasterized bitmaps in this cache are keyed by pixel size, not by scale — a bitmap
            // rasterized at 1x stays a 1x bitmap after the window moves to a Retina display.
            // `GroupCacheKey::scale` (see `replay::replay_group`) already forces every group's own
            // `CALayer` tree to rebuild when the scale changes; this cache needs an explicit drop
            // since nothing else invalidates it.
            self.ivars()
                .replay_state
                .borrow_mut()
                .vector_raster_cache
                .clear();
            self.setNeedsLayout(true);
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
            }
            if let Some(old) = self.ivars().tracking_area.borrow_mut().take() {
                self.removeTrackingArea(&old);
            }
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    self.bounds(),
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::MouseMoved
                        | NSTrackingAreaOptions::ActiveInKeyWindow
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(self as &AnyObject),
                    None,
                )
            };
            self.addTrackingArea(&area);
            *self.ivars().tracking_area.borrow_mut() = Some(area);
        }

        /// `NSResponder`'s own gate on receiving `keyDown:`/`keyUp:` at all — `NSView`'s default is
        /// `false`, which is why this view never saw a single key event before this override.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// AppKit normally consumes the first click on an inactive window only to activate it.
        /// ElwindUI's self-drawn controls must receive that same click so pointer gestures (most
        /// visibly dragging a mascot window) can begin without a separate activation click.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: &NSEvent) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.dispatch_key(event, true);
            self.dispatch_text_input(event);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            self.dispatch_key(event, false);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(prev) = self.ivars().active_popup.borrow_mut().take() {
                prev.close();
            }
            self.dispatch_pointer(event, RawPointerEventKind::Pressed(MouseButton::Left));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Released(MouseButton::Left));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Pressed(MouseButton::Right));
            let menu = self.menu_for_event_inner(event);
            if !menu.is_null() {
                let m = unsafe { Retained::retain(menu).unwrap() };
                NSMenu::popUpContextMenu_withEvent_forView(&m, event, self);
            }
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Released(MouseButton::Right));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Moved);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Moved);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Moved);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Moved);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            // A plain `Moved` re-hit-tests from `event`'s own (by now outside this view's bounds)
            // position, which naturally misses everything — `PointerDispatcher`'s hover diffing
            // then exits every element in the last-known hover chain on its own.
            self.dispatch_pointer(event, RawPointerEventKind::Moved);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // Unlike `mouseDown:`/`mouseUp:`/etc. above, this one must call `super` — `dispatch_pointer`
            // is elwindui's own internal wheel-event path (for a self-drawn element that wants raw wheel
            // deltas), but AppKit's own default `NSView.scrollWheel:` is what walks the view hierarchy
            // to find and scroll an *enclosing* `NSScrollView` (see `InnerScrollView`'s own doc comment:
            // "letting the enclosing NSScrollView's native scroll physics do the rest"). This view is the
            // one that actually receives the event first — as `Window`'s own content host, as a `TabView`
            // tab's content host, and as `InnerScrollView`'s `content_host` (its `NSScrollView`'s document
            // view) alike — so without forwarding to `super` here, a `ScrollView`'s content never scrolls.
            unsafe {
                let _: () = msg_send![super(self), scrollWheel: event];
            }
            self.dispatch_pointer(
                event,
                RawPointerEventKind::WheelChanged {
                    delta_x: event.scrollingDeltaX() as f32,
                    delta_y: event.scrollingDeltaY() as f32,
                },
            );
        }

        #[unsafe(method(menuForEvent:))]
        fn menu_for_event(&self, event: &NSEvent) -> *mut NSMenu {
            self.menu_for_event_inner(event)
        }
    }
);

impl TreeHostView {
    fn menu_for_event_inner(&self, event: &NSEvent) -> *mut NSMenu {
        let Some(tree) = self.ivars().tree.borrow().clone() else {
            return std::ptr::null_mut();
        };
        let location = self.convertPoint_fromView(event.locationInWindow(), None);
        let local_point = Point {
            x: location.x as f32,
            y: location.y as f32,
        };
        let (screen_anchor_pt, work_area) = self.query_screen_and_work_area(location);
        let request = ContextRequest::pointer(local_point, screen_anchor_pt);
        let Some((resolved, anchor)) = ContextMenuService::process_request(
            &tree,
            &self.ivars().keyboard.focus,
            &request,
        ) else {
            return std::ptr::null_mut();
        };
        if let Some(prev) = self.ivars().active_popup.borrow_mut().take() {
            prev.close();
        }
        match resolved.definition {
            ResolvedContextDefinition::Menu { menu, presentation } => {
                match presentation {
                    ContextMenuPresentation::Native => {
                        let appkit_menu = menu
                            .as_any()
                            .downcast_ref::<crate::native_ui::Menu>()
                            .expect("AppKit MenuExt: menu must be this backend's Menu");
                        Retained::autorelease_return(appkit_menu.inner_ns())
                    }
                    ContextMenuPresentation::Custom => {
                        let host = crate::inner::AppKitPopupHost::new(self.window());
                        let handle = ContextMenuService::open_custom_menu(
                            &host,
                            &*menu,
                            &anchor,
                            work_area,
                        );
                        *self.ivars().active_popup.borrow_mut() = handle;
                        std::ptr::null_mut()
                    }
                }
            }
            ResolvedContextDefinition::Popup { template } => {
                let host = crate::inner::AppKitPopupHost::new(self.window());
                let handle = ContextMenuService::open_custom_popup(
                    &host,
                    &resolved.owner,
                    &template,
                    &anchor,
                    resolved.owner.effective_environment(),
                    work_area,
                );
                *self.ivars().active_popup.borrow_mut() = handle;
                std::ptr::null_mut()
            }
        }
    }

    fn query_screen_and_work_area(&self, location: NSPoint) -> (Point, Rect) {
        let m = mtm();
        let primary_screen = NSScreen::screens(m)
            .firstObject()
            .or_else(|| NSScreen::mainScreen(m));
        let primary_screen_height = primary_screen
            .as_ref()
            .map(|s| s.frame().size.height)
            .or_else(|| self.window().map(|w| w.frame().size.height))
            .unwrap_or_else(|| self.frame().size.height);

        let screen_pt = if let Some(w) = self.window() {
            let view_pt = self.convertPoint_toView(location, None);
            w.convertPointToScreen(view_pt)
        } else {
            location
        };

        let core_screen_pt = Point {
            x: screen_pt.x as f32,
            y: (primary_screen_height - screen_pt.y) as f32,
        };

        let target_screen = NSScreen::screens(m)
            .iter()
            .find(|s| {
                let f = s.frame();
                screen_pt.x >= f.origin.x
                    && screen_pt.x <= f.origin.x + f.size.width
                    && screen_pt.y >= f.origin.y
                    && screen_pt.y <= f.origin.y + f.size.height
            })
            .or_else(|| self.window().and_then(|w| w.screen()))
            .or_else(|| primary_screen);

        let visible_frame = target_screen
            .map(|s| s.visibleFrame())
            .or_else(|| self.window().map(|w| w.frame()))
            .unwrap_or_else(|| self.frame());

        let work_area = Rect {
            x: visible_frame.origin.x as f32,
            y: (primary_screen_height - (visible_frame.origin.y + visible_frame.size.height)) as f32,
            width: visible_frame.size.width as f32,
            height: visible_frame.size.height as f32,
        };

        (core_screen_pt, work_area)
    }

    pub(crate) fn new() -> Retained<Self> {
        let m = mtm();
        let ivars = TreeHostIvars {
            tree: RefCell::new(None),
            render_tree: RefCell::new(None),
            native_containers: RefCell::new(HashMap::new()),
            native_owner_ids: RefCell::new(HashMap::new()),
            replay_state: RefCell::new(ReplayState::default()),
            weak_self: RefCell::new(objc2::rc::Weak::default()),
            pointer: PointerDispatcher::new(),
            keyboard: KeyboardDispatcher::new(),
            tracking_area: RefCell::new(None),
            unconstrained_axes: Cell::new((false, false)),
            relaying_out: Cell::new(false),
            pending_dirty_ids: RefCell::new(Vec::new()),
            needs_another_pass: Cell::new(false),
            pending_invalidation: Cell::new(None),
            last_layout_size: Cell::new(objc2_foundation::NSSize::new(-1.0, -1.0)),
            active: Cell::new(true),
            active_popup: RefCell::new(None),
        };
        let this = Self::alloc(m).set_ivars(ivars);
        let this: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::default()] };
        *this.ivars().weak_self.borrow_mut() = objc2::rc::Weak::from_retained(&this);
        this
    }

    /// Converts `event`'s own position/modifiers/timestamp and feeds it, together with `kind`, to
    /// `PointerDispatcher::handle` against whatever tree this view currently hosts — the single
    /// entry point every `mouseDown:`/`mouseUp:`/`mouseMoved:`/... override above funnels through.
    /// A no-op if no tree is hosted yet.
    fn dispatch_pointer(&self, event: &NSEvent, kind: RawPointerEventKind) {
        // `isFlipped` is `true` (see that override above), so this is already this view's own
        // top-left-origin local space — the same space `elwindui_core::ui::hit_test`'s `at`
        // expects, matching `elwindui_core::ui::layout_root`'s own coordinate convention.
        let location = self.convertPoint_fromView(event.locationInWindow(), None);
        self.dispatch_pointer_at(
            Point {
                x: location.x as f32,
                y: location.y as f32,
            },
            nsevent_modifiers(event),
            kind,
            event.timestamp(),
        );
    }

    fn dispatch_pointer_at(
        &self,
        position: Point,
        modifiers: KeyModifiers,
        kind: RawPointerEventKind,
        timestamp: f64,
    ) {
        let tree = self.ivars().tree.borrow().clone();
        let Some(tree) = tree else { return };
        self.ivars().pointer.handle(
            &tree,
            &self.ivars().keyboard.focus,
            RawPointerEvent {
                kind,
                position,
                modifiers,
                timestamp_ms: timestamp * 1000.0,
            },
        );
    }

    /// Converts `event`'s own key/modifiers/repeat and feeds it, together with `is_down`, to
    /// `KeyboardDispatcher::handle_key` against whatever tree this view currently hosts. A no-op if
    /// no tree is hosted yet, or if `event` maps to no `Key` at all (`nsevent_key` returning `None`
    /// — practically never, since it always falls back to the raw character).
    fn dispatch_key(&self, event: &NSEvent, is_down: bool) {
        let tree = self.ivars().tree.borrow().clone();
        let Some(tree) = tree else { return };
        let Some(key) = nsevent_key(event) else {
            return;
        };
        self.ivars().keyboard.handle_key(
            &tree,
            RawKeyEvent {
                kind: if is_down {
                    RawKeyEventKind::Down {
                        is_repeat: event.isARepeat(),
                    }
                } else {
                    RawKeyEventKind::Up
                },
                key,
                modifiers: nsevent_modifiers(event),
                timestamp_ms: event.timestamp() * 1000.0,
            },
        );
    }

    /// `event.characters()` (post-modifier, pre-IME — see `nsevent_key`'s own doc comment on the
    /// same "no full `NSTextInputClient`" limitation) fed to `KeyboardDispatcher::handle_text_input`
    /// as `on_text_input`, filtered to a single non-control character. Control keys (arrows, Tab,
    /// Enter, Escape, function keys, ...) also produce a non-empty `characters()` string on macOS —
    /// excluding `Unicode` control-category characters keeps those from misfiring as text input.
    fn dispatch_text_input(&self, event: &NSEvent) {
        let tree = self.ivars().tree.borrow().clone();
        let Some(tree) = tree else { return };
        let Some(text) = event.characters().map(|s| s.to_string()) else {
            return;
        };
        if text.is_empty() || text.chars().any(|c| c.is_control()) {
            return;
        }
        self.ivars()
            .keyboard
            .handle_text_input(&tree, RawTextInputEvent { text });
    }

    /// Replaces this host's entire content, discarding whatever native subviews were there before.
    pub(crate) fn set_tree(&self, tree: Rc<dyn UIElementExt>) {
        for old in self.subviews().iter() {
            old.removeFromSuperview();
        }
        self.ivars().native_containers.borrow_mut().clear();
        self.ivars().native_owner_ids.borrow_mut().clear();
        *self.ivars().replay_state.borrow_mut() = ReplayState::default();
        let weak_self = self.ivars().weak_self.borrow().clone();
        tree.as_ui_element()
            .set_invalidate_host(Some(Rc::new(AppKitRelayoutHost(weak_self.clone()))));
        tree.as_ui_element()
            .set_focus_host(Some(Rc::new(AppKitFocusHost(weak_self))));
        self.ivars().keyboard.focus.clear_focus();
        self.ivars().keyboard.shortcuts().clear();
        self.ivars().keyboard.shortcuts().collect_from_tree(&tree);
        *self.ivars().tree.borrow_mut() = Some(tree);
        *self.ivars().render_tree.borrow_mut() = None;
        self.invalidateIntrinsicContentSize();
        self.relayout();
    }

    /// Clears this host's tree and releases native compositor islands and focus.
    pub(crate) fn clear_tree(&self) {
        for old in self.subviews().iter() {
            old.removeFromSuperview();
        }
        self.ivars().native_containers.borrow_mut().clear();
        self.ivars().native_owner_ids.borrow_mut().clear();
        *self.ivars().replay_state.borrow_mut() = ReplayState::default();
        self.ivars().keyboard.focus.clear_focus();
        self.ivars().keyboard.shortcuts().clear();
        *self.ivars().tree.borrow_mut() = None;
        *self.ivars().render_tree.borrow_mut() = None;
    }

    /// Focuses the specified element within this host's focus tracker.
    pub(crate) fn focus_element(&self, element: &Rc<dyn UIElementExt>) {
        self.ivars().keyboard.focus.set_focus(element, FocusState::Programmatic);
    }

    /// Opts this host's own `relayout` into measuring `width`/`height` unconstrained (`f32::
    /// INFINITY`) rather than against `self.frame()`'s current size — see `TreeHostIvars::
    /// unconstrained_axes`'s own doc comment. `InnerScrollView` calls this once, at construction,
    /// on its nested content host; every other host leaves both `false` (the default).
    pub(crate) fn set_unconstrained_axes(&self, width: bool, height: bool) {
        self.ivars().unconstrained_axes.set((width, height));
    }

    /// The authoritative pixels-per-point scale for everything this host paints — the value every
    /// hand-built `CALayer` in `replay_group`/`render::*` must be stamped with (see
    /// `render::add_sublayer_scaled`) since Core Animation does not inherit `contentsScale` from a
    /// superlayer the way it does for a layer-backed view's own backing layer.
    ///
    /// Sourced from `self.window().backingScaleFactor()` rather than `self.layer().
    /// contentsScale()`: the latter is itself derived by AppKit *from* `backingScaleFactor`, so
    /// depending on it would make this value sensitive to whether AppKit has already refreshed the
    /// backing layer before `layout` runs on a given pass. Falls back to `NSScreen::mainScreen`'s
    /// scale, then `1.0`, for a host laid out before it is attached to a window — e.g.
    /// `InnerScrollView`'s nested content host and `InnerTabView`'s per-tab hosts, both of which
    /// call `relayout` during construction.
    pub(crate) fn backing_scale_factor(&self) -> objc2_core_foundation::CGFloat {
        if let Some(window) = self.window() {
            return window.backingScaleFactor();
        }
        if let Some(screen) = objc2_app_kit::NSScreen::mainScreen(mtm()) {
            return screen.backingScaleFactor();
        }
        1.0
    }

    /// Activates or suppresses this host's own layout/render participation — the mechanism behind
    /// `InnerTabView`'s non-selected tabs (docs/design/runtime/layout_design.md): a
    /// suppressed host keeps its `tree` (so a previously-shown-then-hidden tab doesn't lose any
    /// state) but discards `render_tree` and every retained backend resource that tree produced
    /// (`CALayer`s, native control islands, image/vector-raster caches), and `relayout_inner`
    /// (called below) refuses to do any measure/arrange/render work while suppressed — including
    /// the `layout()` calls AppKit's own autoresizing mask machinery keeps firing on every
    /// `content_container` resize for every tab host, not just the selected one.
    ///
    /// Reactivating forces a full `Measure` pass at this host's *current* frame size (not
    /// whatever size it had when last active) — a suppressed host still gets resized by its
    /// superview's autoresizing mask, so its `frame` may well have changed while suppressed, and
    /// no `relayout_inner` pass ran to notice.
    pub(crate) fn set_active(&self, active: bool) {
        if self.ivars().active.get() == active {
            return;
        }
        self.ivars().active.set(active);
        if active {
            self.ivars()
                .last_layout_size
                .set(objc2_foundation::NSSize::new(-1.0, -1.0));
            self.relayout();
        } else {
            // `relayout_inner`'s own GC (the `retain` calls below) only runs during a relayout
            // pass, which a suppressed host by definition no longer gets — so every currently-
            // attached CALayer/NSView must be detached here, explicitly, before the caches that
            // own them are dropped.
            for container in self.ivars().replay_state.borrow().group_layers.values() {
                container.removeFromSuperlayer();
            }
            for (_, container) in self.ivars().native_containers.borrow_mut().drain() {
                container.removeFromSuperview();
            }
            self.ivars().native_owner_ids.borrow_mut().clear();
            *self.ivars().render_tree.borrow_mut() = None;
            *self.ivars().replay_state.borrow_mut() = ReplayState::default();
        }
    }

    /// Reflects the current `tree`'s layout and paint state into real `NSView`/`CALayer` state.
    /// Wraps `relayout_inner` with the reentrancy guard `AppKitRelayoutHost::request_relayout`
    /// relies on (`relaying_out`/`pending_dirty_ids`/`needs_another_pass` — see those fields' own
    /// doc comments): a reentrant `request_relayout` during `relayout_inner` (e.g. a
    /// `NativeControl` focus change synchronously running user code that calls `invalidate()`)
    /// cannot safely `render_tree.borrow_mut()` while `relayout_inner`'s own replay pass still
    /// holds `render_tree` borrowed, so it defers its `mark_dirty` instead — this is what applies
    /// that deferred work once `relayout_inner` returns.
    fn relayout(&self) {
        self.ivars().relaying_out.set(true);
        self.relayout_inner();
        self.ivars().relaying_out.set(false);

        let pending: Vec<u64> = self
            .ivars()
            .pending_dirty_ids
            .borrow_mut()
            .drain(..)
            .collect();
        if !pending.is_empty() {
            if let Some(render_tree) = self.ivars().render_tree.borrow_mut().as_mut() {
                for id in pending {
                    render_tree.mark_dirty(id);
                }
            }
        }
        if self.ivars().needs_another_pass.take() {
            self.setNeedsLayout(true);
        }
    }

    fn relayout_inner(&self) {
        use elwindui_core::base::Size;

        // A suppressed host (`set_active(false)` — e.g. a `TabView`'s non-selected tab) does no
        // measure/arrange/render work at all, including for a pass `layout()` triggered for
        // reasons that have nothing to do with this specific host (a window resize propagating
        // through AppKit's own autoresizing mask machinery hits every tab's content host, not just
        // the selected one). `render_tree`/`replay_state` were already torn down by `set_active`
        // itself, so there is nothing here to reconcile against even if this didn't return early.
        if !self.ivars().active.get() {
            return;
        }

        // Suppresses Core Animation's implicit ~0.25s property animations for this whole pass —
        // see `ImplicitAnimationGuard`'s own doc comment. `_animation_guard` is never read; its
        // `Drop` (running whichever of this function's several early `return`s is taken) is the
        // entire point.
        let _animation_guard = crate::render::ImplicitAnimationGuard::begin();

        let frame = self.frame();
        let (unconstrained_width, unconstrained_height) = self.ivars().unconstrained_axes.get();
        let available = Size {
            width: if unconstrained_width {
                f32::INFINITY
            } else {
                frame.size.width as f32
            },
            height: if unconstrained_height {
                f32::INFINITY
            } else {
                frame.size.height as f32
            },
        };
        let tree = self.ivars().tree.borrow();
        let Some(tree) = tree.as_ref() else { return };

        // `layout` (the NSView override that calls `relayout`) fires for reasons other than our
        // own `request_relayout` — a window resize chief among them — so a frame change always
        // forces `Measure` regardless of what (if anything) was actually requested. Several
        // `request_relayout` calls since the last pass already coalesced to their strongest kind
        // (`AppKitRelayoutHost::request_relayout`'s own `max`); `None` (nothing was explicitly
        // requested — e.g. the very first pass from `set_tree`) also defaults to `Measure`, the
        // only kind safe to assume with no other information.
        let requested = self.ivars().pending_invalidation.take();
        let frame_changed = self.ivars().last_layout_size.get() != frame.size;
        let kind = if frame_changed {
            InvalidationKind::Measure
        } else {
            requested.unwrap_or(InvalidationKind::Measure)
        };
        self.ivars().last_layout_size.set(frame.size);

        if kind != InvalidationKind::Render {
            layout_root(tree, available);
            // `InnerScrollView`'s content host (`unconstrained_axes` set via
            // `set_unconstrained_axes`) grows to its own content's natural size on whichever axis is
            // unconstrained, rather than staying clamped to `frame`'s (possibly stale, possibly
            // zero-on-first-layout) size — every other host has both axes `false` and this is a no-op.
            if unconstrained_width || unconstrained_height {
                let natural_width = tree.arranged_width().unwrap_or(0.0) as f64;
                let natural_height = tree.arranged_height().unwrap_or(0.0) as f64;
                let new_width = if unconstrained_width {
                    natural_width
                } else {
                    frame.size.width
                };
                let new_height = if unconstrained_height {
                    natural_height
                } else {
                    frame.size.height
                };
                if new_width != frame.size.width || new_height != frame.size.height {
                    self.setFrame(NSRect::new(
                        frame.origin,
                        objc2_foundation::NSSize::new(new_width, new_height),
                    ));
                }
            }
        }
        // `Render` skips `layout_root` above, but `reconcile` below still runs — since Render
        // invalidation never clears `arranged_*`, `reconcile_render_group` recomputes the exact
        // same `offset`/`size`/`clip` it always would, so this reconcile is provably equivalent to
        // what today's always-runs-`layout_root` path already produced; only the (redundant)
        // measure/arrange work itself is skipped.
        {
            let mut retained_tree = self.ivars().render_tree.borrow_mut();
            if retained_tree
                .as_ref()
                .is_some_and(|render_tree| render_tree.root_id() == tree.render_group_id())
            {
                retained_tree
                    .as_mut()
                    .expect("checked above")
                    .reconcile::<AnyView>(tree);
            } else {
                *retained_tree = Some(elwindui_core::graphics::RenderTree::new::<AnyView>(tree));
            }
        }
        let render_tree = self.ivars().render_tree.borrow();
        let Some(render_tree) = render_tree.as_ref() else {
            return;
        };

        // `setWantsLayer` is idempotent in AppKit itself, but calling it every pass is still an
        // avoidable message send — only the very first pass needs it.
        if self.layer().is_none() {
            self.setWantsLayer(true);
        }
        let layer = self.layer().expect("wantsLayer(true) implies a layer");

        let mut live_native_controls = HashSet::new();
        let mut live_group_ids = HashSet::new();
        let mut live_image_ids = HashSet::new();
        let mut live_vector_image_ids = HashSet::new();
        let mut new_group_order = Vec::new();
        let mut new_native_order = Vec::new();
        let mut state = self.ivars().replay_state.borrow_mut();
        replay_group(
            self,
            &layer,
            &render_tree.root,
            elwindui_core::base::Point { x: 0.0, y: 0.0 },
            None,
            elwindui_core::base::AffineTransform::identity(),
            1.0,
            self.backing_scale_factor(),
            &mut live_native_controls,
            &mut live_group_ids,
            &mut live_image_ids,
            &mut live_vector_image_ids,
            &mut new_group_order,
            &mut new_native_order,
            &mut state,
        );
        state
            .image_cache
            .retain(|id, _| live_image_ids.contains(id));
        state
            .vector_raster_cache
            .retain(|id, _| live_vector_image_ids.contains(id));
        state.group_layers.retain(|id, container| {
            if live_group_ids.contains(id) {
                true
            } else {
                container.removeFromSuperlayer();
                false
            }
        });
        state
            .group_cache
            .retain(|id, _| live_group_ids.contains(id));

        // Z-order repair: `replay_group` only `addSublayer`s a container the first time it's ever
        // created (see that function's own doc comment) — a group whose position in the traversal
        // order moved (a list reorder, a tab switch, an item insert/delete elsewhere in the tree)
        // needs its container moved too. Comparing traversal orders makes "nothing moved" (by far
        // the common case — a static UI's steady-state relayout) provably free: this whole block,
        // and therefore any `addSublayer` call, is skipped entirely.
        //
        // Re-`addSublayer`ing every entry in `new_group_order`, in order, rather than computing a
        // minimal set of moves: `root_layer.setSublayers(...)` can't be used here since AppKit
        // interleaves each native control's own backing layer into this same array (see the
        // native-control z-order comment below) — replacing the array wholesale would detach every
        // one of them. `insertSublayer:atIndex:` has the same problem from the other direction: an
        // index computed from `new_group_order` alone is an index into a *paint-only* sequence, not
        // into the real (paint + native) array. Re-adding in order sidesteps both — AppKit moves an
        // already-attached sublayer to the top of the array rather than duplicating it, so a plain
        // ordered pass reproduces `new_group_order`'s relative order at the top of whatever native
        // layers are already interleaved below.
        let group_order_changed = state.group_order != new_group_order;
        if group_order_changed {
            for id in &new_group_order {
                if let Some(container) = state.group_layers.get(id) {
                    crate::render::stats::bump(|s| s.add_sublayer_calls += 1);
                    layer.addSublayer(container);
                }
            }
            state.group_order = new_group_order;
        }
        let native_order_changed = state.native_order != new_native_order;
        if native_order_changed {
            state.native_order = new_native_order;
        }
        drop(state);
        self.ivars()
            .native_containers
            .borrow_mut()
            .retain(|identity, container| {
                if live_native_controls.contains(identity) {
                    true
                } else {
                    container.removeFromSuperview();
                    false
                }
            });
        self.ivars()
            .native_owner_ids
            .borrow_mut()
            .retain(|identity, _| live_native_controls.contains(identity));
        // A repainted `RenderGroup` container whose *order* moved above (`group_order_changed`)
        // just moved back to the front of `root_layer`'s sublayers (see the Z-order repair comment
        // above). A native leaf's own island, though, is only ever `host.addSubview`ed once, the
        // first time it appears (`NativeIslandHost::attach_island`'s `is_new` guard in
        // `replay_commands`) — so a paint reorder would otherwise stack a paint layer right back on
        // top of a native control that didn't move, hiding it, even though the control itself is
        // still correctly laid out and attached. Run this restore loop only when paint topology
        // could plausibly have shifted (`group_order_changed`) or the native controls themselves
        // did (`native_order_changed`, e.g. one was added/removed/reordered this pass) — a static
        // UI's steady-state relayout takes neither branch, so `addSubview` is called zero times.
        // Iterating `new_native_order` (rather than the old `native_containers.values()`, a
        // `HashMap` whose iteration order was never actually deterministic) also fixes a latent
        // bug: the relative Z-order among multiple native controls used to be randomized every
        // pass instead of following paint traversal order like every other leaf here does.
        if group_order_changed || native_order_changed {
            let containers = self.ivars().native_containers.borrow();
            for identity in &self.ivars().replay_state.borrow().native_order {
                if let Some(container) = containers.get(identity) {
                    crate::render::stats::bump(|s| s.subview_added += 1);
                    self.addSubview(container);
                }
            }
        }

        // Cheap only relative to a `debug_assertions`/`render-stats` build that already pays for
        // `render::stats` bumps throughout this pass — a release build with neither never takes
        // this branch, so the `task_info` syscall and cache walk `record_memory_stats` does never
        // run there. See that method's own doc comment for why it isn't unconditional.
        #[cfg(any(test, debug_assertions, feature = "render-stats"))]
        {
            self.record_memory_stats();
            // Manual, opt-in observation point for the numbers `record_memory_stats` populates —
            // there is otherwise no way to read `render::stats::snapshot()` from outside the
            // process. `ELWINDUI_RENDER_STATS=1 cargo run -p <example>` prints one JSON-ish line
            // per relayout pass; see `docs/status/implementation_status.md` for how this feeds
            // the AppKit render-optimization work's per-step measurement table.
            if std::env::var_os("ELWINDUI_RENDER_STATS").is_some() {
                let s = crate::render::stats::snapshot();
                eprintln!(
                    "elwindui-render-stats groups_visited={} groups_rebuilt={} groups_cache_hit={} \
                     groups_updated_in_place={} layers_created={} layers_removed={} \
                     add_sublayer_calls={} subview_added={} cgpaths_created={} cgcolors_created={} \
                     text_layers_created={} attributed_strings_created={} setter_calls={} \
                     setter_calls_skipped={} image_cache_bytes={} vector_raster_cache_bytes={} \
                     process_footprint_bytes={} process_resident_bytes={}",
                    s.groups_visited,
                    s.groups_rebuilt,
                    s.groups_cache_hit,
                    s.groups_updated_in_place,
                    s.layers_created,
                    s.layers_removed,
                    s.add_sublayer_calls,
                    s.subview_added,
                    s.cgpaths_created,
                    s.cgcolors_created,
                    s.text_layers_created,
                    s.attributed_strings_created,
                    s.setter_calls,
                    s.setter_calls_skipped,
                    s.image_cache_bytes,
                    s.vector_raster_cache_bytes,
                    s.process_footprint_bytes,
                    s.process_resident_bytes,
                );
            }
        }
    }

    /// Looks up which live native leaf `container` (one of `native_containers`' own values — the
    /// per-widget island `host.addSubview`ed directly, not the leaf's own inner `NSView`) belongs
    /// to, returning that leaf's owning element's `render_group_id`. Used by
    /// `ElwinduiWindow::makeFirstResponder:` (see that method's own doc comment) to bridge a native
    /// OS focus change back into `elwindui_core::focus`. A linear scan is fine here — a single
    /// window typically hosts at most a handful of native controls at once.
    pub(crate) fn resolve_native_owner_id(&self, container: &NSView) -> Option<u64> {
        let identity = self
            .ivars()
            .native_containers
            .borrow()
            .iter()
            .find(|(_, v)| std::ptr::eq(&***v, container))
            .map(|(identity, _)| *identity)?;
        self.ivars()
            .native_owner_ids
            .borrow()
            .get(&identity)
            .copied()
    }
}

/// The real, production `NativeIslandHost` — see that trait's own doc comment for why the replay
/// pass needs nothing else from a live view.
impl NativeIslandHost for TreeHostView {
    fn island(&self, identity: usize, owner_id: u64) -> (Retained<NSView>, bool) {
        let mut containers = self.ivars().native_containers.borrow_mut();
        if let Some(container) = containers.get(&identity) {
            (container.clone(), false)
        } else {
            let container = NSView::new(mtm());
            containers.insert(identity, container.clone());
            self.ivars()
                .native_owner_ids
                .borrow_mut()
                .insert(identity, owner_id);
            (container, true)
        }
    }

    fn attach_island(&self, container: &NSView, nsview: &NSView) {
        self.addSubview(container);
        container.addSubview(nsview);
    }
}

impl TreeHostView {
    /// Populates `render::stats::RenderStats`'s memory fields (`image_cache_bytes`/
    /// `vector_raster_cache_bytes`/`process_footprint_bytes`/`process_resident_bytes`) from this
    /// host's current caches and the process's own task VM counters. Called from `relayout_inner`
    /// under the same `cfg(any(test, debug_assertions, feature = "render-stats"))` gate as every other
    /// `render::stats` counter, so a plain release build (no feature) never pays for the
    /// `task_info` syscall or the `O(cache size)` walk this does — see that call site.
    pub(crate) fn record_memory_stats(&self) {
        let state = self.ivars().replay_state.borrow();
        let image_cache_bytes: u64 = state
            .image_cache
            .values()
            .map(|image| crate::render::cgimage_bytes(image))
            .sum();
        let vector_raster_cache_bytes: u64 = state
            .vector_raster_cache
            .values()
            .map(|(_, _, _, image)| crate::render::cgimage_bytes(image))
            .sum();
        drop(state);
        let process_memory = crate::render::stats::process_memory();
        crate::render::stats::bump(|s| {
            s.image_cache_bytes = image_cache_bytes;
            s.vector_raster_cache_bytes = vector_raster_cache_bytes;
            s.process_footprint_bytes = process_memory.physical_footprint_bytes;
            s.process_resident_bytes = process_memory.resident_bytes;
        });
    }
}
