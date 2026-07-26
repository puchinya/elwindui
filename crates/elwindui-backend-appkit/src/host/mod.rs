//! The tree host: one `NSView` subclass that reflects an `elwindui_core` element tree into real
//! `NSView` subviews and `CALayer` sublayers, and feeds native events back into core's
//! pointer/keyboard/focus dispatchers.
//!
//! `InnerWindow`'s content view and `InnerTabView`'s per-tab content area are each one of these.
//! Depends downward on `render` for all drawing; `replay` below is the pass that consumes this
//! view's own layer caches, which is why it lives here rather than under `render`.


use crate::ffi::{AnyView, mtm};
use elwindui_core::base::Point;
use elwindui_core::input::{
    FocusState, KeyModifiers, KeyboardDispatcher, MouseButton, PointerDispatcher, RawKeyEvent,
    RawKeyEventKind, RawPointerEvent, RawPointerEventKind, RawTextInputEvent,
};
use elwindui_core::ui::{FocusHost, RelayoutHost, UIElementExt, layout_root};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSAppearanceCustomization, NSEvent, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSObjectProtocol, NSRect};
use objc2_quartz_core::CALayer;
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
    /// Decoded-image cache (`RenderCommand::DrawImage`'s `elwindui_core::graphics::Image` -> real
    /// `CGImage`), keyed by the `Image`'s own pointer identity — see `resolve_cgimage`'s own doc
    /// comment. Never cleared piecemeal (unlike `native_containers`): a stale entry for an
    /// `Image` no longer referenced by the current tree is simply harmless dead weight, not
    /// incorrect, and pruning it would need the same kind of `retain`-by-liveness bookkeeping
    /// `native_containers` has for comparatively little benefit (a decoded `CGImage` is far
    /// cheaper to keep around than a live `NSView` island).
    pub(crate) image_cache: RefCell<HashMap<usize, CFRetained<CGImage>>>,
    /// `RenderCommand::DrawVectorImage`'s `VectorRasterizeMode::Auto`/`Fixed` cache — the
    /// rasterized-bitmap counterpart to `image_cache` above, keyed by `VectorImageId` rather than
    /// pointer identity since the *same* `VectorImage` may legitimately need re-rasterizing at a
    /// different pixel size (unlike a decoded raster `Image`, which has one fixed native size).
    /// At most one entry per id — `Auto` mode simply overwrites the entry when the requested size
    /// changes (see `VectorRasterizeMode::Auto`'s own doc comment); `Fixed` mode never changes
    /// size so its entry never gets overwritten after the first rasterization. Never pruned, same
    /// reasoning as `image_cache` above.
    pub(crate) vector_raster_cache: RefCell<HashMap<elwindui_core::graphics::VectorImageId, (u32, u32, CFRetained<CGImage>)>>,
    /// Per-`RenderGroup` id, the persistent container `CALayer` holding that group's own painted
    /// sublayers — a flat sibling of the root paint layer (`frame` always exactly matches the
    /// root's own `bounds()`, a zero-offset "namespace" rather than a real nested coordinate
    /// space) so every existing absolute-canvas-coordinate drawing helper
    /// (`replay_paint_command`/`try_add_gradient_fill_layer`/`clip_mask_layer`/`DrawImage`'s own
    /// container) keeps working completely unchanged. Reused across `relayout` passes — see
    /// `group_layer_cache_keys`'s own doc comment for when its contents get rebuilt vs left alone
    /// (painter design doc §15's renderer cache, acceptance criterion 14).
    pub(crate) group_layers: RefCell<HashMap<u64, Retained<CALayer>>>,
    /// What `group_layers[id]`'s sublayers were last rebuilt from. A `RenderGroup`'s own
    /// `generation` alone can't tell `replay_group` whether a rebuild is needed: this backend
    /// bakes the *full accumulated* origin/clip/transform/opacity directly into each leaf's
    /// `CGPath`/frame (not a live nested `CALayer` transform, by deliberate design — see
    /// `replay_group`'s own doc comment), so a group whose own `commands` are byte-for-byte
    /// unchanged still needs rebuilding if an ancestor's offset moved (the group's own relative
    /// `offset` stays the same, so its `generation` never bumps, even though the *absolute*
    /// geometry baked into its cached sublayers is now stale). Comparing the full
    /// `(generation, origin, clip, transform, opacity)` tuple each pass catches both cases.
    pub(crate) group_layer_cache_keys: RefCell<HashMap<u64, GroupCacheKey>>,
    /// Which `native_containers` identities were discovered inside each group's own `commands` the
    /// last time it was actually rebuilt — replayed back into `live_native_controls` on a cache hit
    /// (where `replay_commands` doesn't run and so can't rediscover them itself), so
    /// `native_containers`' own liveness-based pruning at the end of `relayout` doesn't tear down a
    /// native control just because its owning group happened to be skipped this pass.
    pub(crate) group_native_controls: RefCell<HashMap<u64, Vec<usize>>>,
    /// Set once, right after construction — lets `set_tree` hand out an `AppKitRelayoutHost`
    /// wrapping a weak reference back to this same view, without needing a `Retained<Self>` in
    /// hand at that point.
    pub(crate) weak_self: RefCell<objc2::rc::Weak<TreeHostView>>,
    /// Turns this view's own raw `NSEvent`s into `elwindui_core::ui::hit_test`/`dispatch_routed`
    /// calls against `tree` — see `elwindui_core::input::PointerDispatcher`'s own doc comment.
    /// `docs/elwindui_gui_framework_design.md` §5.10's currently-implemented range: self-drawn
    /// elements only, since a native subview (`Button`/`TextArea`/`TabView`, laid out as its own
    /// `native_containers` island) receives the OS mouse event directly via ordinary AppKit
    /// hit-testing, never reaching this view's own overrides below at all.
    pub(crate) pointer: PointerDispatcher,
    /// Turns this view's own raw key/text events into `elwindui_core::ui::dispatch_routed` calls
    /// against whichever element currently has focus, and owns the `FocusTracker`/
    /// `ShortcutRegistry` for whatever tree this view hosts — see
    /// `elwindui_core::input::KeyboardDispatcher`'s own doc comment. `docs/elwindui_gui_framework_
    /// design.md` §5.5/§8.1's currently-implemented range mirrors `pointer`'s own: self-drawn
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
}

/// `elwindui_core::ui::RelayoutHost` for `TreeHostView` — wraps a *weak* reference back to the view
/// (not the view itself) since a strong one would create a reference cycle. `request_relayout`
/// silently does nothing if the view has since been deallocated (`load()` returns `None`).
pub(crate) struct AppKitRelayoutHost(objc2::rc::Weak<TreeHostView>);

impl RelayoutHost for AppKitRelayoutHost {
    fn request_relayout(&self, dirty_group_id: u64) {
        if let Some(view) = self.0.load() {
            if let Some(render_tree) = view.ivars().render_tree.borrow_mut().as_mut() {
                render_tree.mark_dirty(dirty_group_id);
            }
            view.setNeedsLayout(true);
        }
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

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            unsafe {
                let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance];
            }
            let name = self.effectiveAppearance().name().to_string();
            let appearance = if name.contains("Dark") {
                elwindui_core::theme::ThemeAppearance::Dark
            } else {
                elwindui_core::theme::ThemeAppearance::Light
            };
            self.theme_handle().set_appearance(appearance);
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
            self.dispatch_pointer(event, RawPointerEventKind::Pressed(MouseButton::Left));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Released(MouseButton::Left));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.dispatch_pointer(event, RawPointerEventKind::Pressed(MouseButton::Right));
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
    }
);

impl TreeHostView {
    pub(crate) fn new() -> Retained<Self> {
        let m = mtm();
        let ivars = TreeHostIvars {
            tree: RefCell::new(None),
            render_tree: RefCell::new(None),
            native_containers: RefCell::new(HashMap::new()),
            native_owner_ids: RefCell::new(HashMap::new()),
            image_cache: RefCell::new(HashMap::new()),
            vector_raster_cache: RefCell::new(HashMap::new()),
            group_layers: RefCell::new(HashMap::new()),
            group_layer_cache_keys: RefCell::new(HashMap::new()),
            group_native_controls: RefCell::new(HashMap::new()),
            weak_self: RefCell::new(objc2::rc::Weak::default()),
            pointer: PointerDispatcher::new(),
            keyboard: KeyboardDispatcher::new(),
            tracking_area: RefCell::new(None),
            unconstrained_axes: Cell::new((false, false)),
        };
        let this = Self::alloc(m).set_ivars(ivars);
        let this: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::default()] };
        *this.ivars().weak_self.borrow_mut() = objc2::rc::Weak::from_retained(&this);
        this
    }

    /// Returns the Window-inherited theme for the hosted tree, or the application theme before a
    /// tree is attached.
    pub(crate) fn theme_handle(&self) -> elwindui_core::theme::ThemeHandle {
        self.ivars()
            .tree
            .borrow()
            .as_ref()
            .map_or_else(elwindui_core::theme::application_theme, |tree| {
                tree.theme_handle()
            })
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
        let tree = self.ivars().tree.borrow();
        let Some(tree) = tree.as_ref() else { return };
        self.ivars().pointer.handle(
            tree,
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
        let tree = self.ivars().tree.borrow();
        let Some(tree) = tree.as_ref() else { return };
        let Some(key) = nsevent_key(event) else {
            return;
        };
        self.ivars().keyboard.handle_key(
            tree,
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
        let tree = self.ivars().tree.borrow();
        let Some(tree) = tree.as_ref() else { return };
        let Some(text) = event.characters().map(|s| s.to_string()) else {
            return;
        };
        if text.is_empty() || text.chars().any(|c| c.is_control()) {
            return;
        }
        self.ivars()
            .keyboard
            .handle_text_input(tree, RawTextInputEvent { text });
    }

    /// Replaces this host's entire content, discarding whatever native subviews were there before.
    pub(crate) fn set_tree(&self, tree: Rc<dyn UIElementExt>) {
        for old in self.subviews().iter() {
            old.removeFromSuperview();
        }
        self.ivars().native_containers.borrow_mut().clear();
        self.ivars().native_owner_ids.borrow_mut().clear();
        self.ivars().group_layers.borrow_mut().clear();
        self.ivars().group_layer_cache_keys.borrow_mut().clear();
        self.ivars().group_native_controls.borrow_mut().clear();
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

    /// Opts this host's own `relayout` into measuring `width`/`height` unconstrained (`f32::
    /// INFINITY`) rather than against `self.frame()`'s current size — see `TreeHostIvars::
    /// unconstrained_axes`'s own doc comment. `InnerScrollView` calls this once, at construction,
    /// on its nested content host; every other host leaves both `false` (the default).
    pub(crate) fn set_unconstrained_axes(&self, width: bool, height: bool) {
        self.ivars().unconstrained_axes.set((width, height));
    }

    fn relayout(&self) {
        use elwindui_core::base::Size;

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

        self.setWantsLayer(true);
        let layer = self.layer().expect("wantsLayer(true) implies a layer");

        let mut live_native_controls = HashSet::new();
        let mut live_group_ids = HashSet::new();
        let mut image_cache = self.ivars().image_cache.borrow_mut();
        let mut vector_raster_cache = self.ivars().vector_raster_cache.borrow_mut();
        replay_group(
            self,
            &layer,
            &render_tree.root,
            elwindui_core::base::Point { x: 0.0, y: 0.0 },
            None,
            elwindui_core::base::AffineTransform::identity(),
            1.0,
            &mut live_native_controls,
            &mut live_group_ids,
            &mut image_cache,
            &mut vector_raster_cache,
        );
        drop(image_cache);
        drop(vector_raster_cache);
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
        self.ivars().group_layers.borrow_mut().retain(|id, container| {
            if live_group_ids.contains(id) {
                true
            } else {
                container.removeFromSuperlayer();
                false
            }
        });
        self.ivars()
            .group_layer_cache_keys
            .borrow_mut()
            .retain(|id, _| live_group_ids.contains(id));
        self.ivars()
            .group_native_controls
            .borrow_mut()
            .retain(|id, _| live_group_ids.contains(id));
        // Every repainted `RenderGroup` container above just moved back to the front of
        // `root_layer`'s sublayers (`replay_group`'s own doc comment on why: re-`addSublayer`ing
        // an already-attached container is what keeps *paint* z-order correct across a mix of
        // rebuilt and cache-hit groups). A native leaf's own island, though, is only ever
        // `host.addSubview`ed once, the first time it appears (`replay_commands`' `is_new` guard)
        // — so after any later pass repaints a sibling paint layer (e.g. a themed, now-opaque
        // `window_background`/`layout_background`), that paint layer ends up stacked back on top
        // of every native control, hiding it, even though the control itself is still correctly
        // laid out and attached. Re-adding every still-live native container here brings it back
        // to the front of `self`'s subviews (AppKit moves an already-attached subview to the top
        // of the z-order rather than duplicating it), keeping native controls visually above all
        // painted content on every pass, not just the first.
        for container in self.ivars().native_containers.borrow().values() {
            self.addSubview(container);
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
        self.ivars().native_owner_ids.borrow().get(&identity).copied()
    }
}
