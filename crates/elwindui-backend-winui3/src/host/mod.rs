//! The tree host: one XAML `Panel` that reflects an `elwindui_core` element tree into real XAML
//! children and Composition visuals, and feeds native events back into core's
//! pointer/keyboard/focus dispatchers.
//!
//! Depends downward on `render` for all drawing.

mod event;
mod replay;

use crate::ffi::{
    AnyView, UiCallbackRegistryOwner, invoke_ui_event_callback, invoke_ui_key_event_callback,
    invoke_ui_text_event_callback,
};
use event::*;
use replay::*;

use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
use crate::bindings::Microsoft::UI::Xaml::Input::{
    CharacterReceivedRoutedEventArgs, KeyEventHandler, PointerEventHandler, PointerRoutedEventArgs,
};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, SizeChangedEventHandler, UIElement};
use crate::render::composition::{
    CompositionClipSpec, CompositionPrimitive, CompositionRenderer, DesiredCompositionIsland,
    DesiredCompositionNode, IslandId,
};
use elwindui_core::input::{
    FocusState, KeyboardDispatcher, MouseButton, PointerDispatcher, RawKeyEvent, RawKeyEventKind,
    RawPointerEvent, RawPointerEventKind, RawTextInputEvent,
};
use elwindui_core::ui::{CoordinateHost, FocusHost, PointerGestureHost, UIElementExt as _};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use windows::Foundation::TypedEventHandler;
use windows::core::Interface;

/// The single reusable "reflect an `Rc<dyn elwindui_core::ui::UIElement>` into real XAML
/// elements" host — the WinUI3 counterpart of `elwindui-backend-appkit`'s `TreeHostView`. A
/// `Canvas` needs no custom `MeasureOverride`/`ArrangeOverride` subclass (unlike `TreeHostView`'s
/// `NSView` subclass) since `Canvas`'s own built-in layout already just measures every child with
/// an unconstrained size and positions it from the `Canvas.Left`/`Canvas.Top` attached properties —
/// exactly the "trust `elwindui_core::ui::layout_root`'s own absolute-rect computation, don't
/// let the native layout system second-guess it" behavior this needs. `Rectangle`/`Ellipse`/
/// `TextBlock` paint nodes become real `Shapes::Rectangle`/`Shapes::Ellipse`/`Controls::TextBlock`
/// elements appended to `Canvas.Children` in traversal order (`Canvas` z-orders by collection
/// order — a parent's own paint is appended before its children's, so it stays behind them),
/// rather than AppKit's separate `CAShapeLayer`/`CATextLayer` sublayer mechanism.
#[derive(Clone)]
pub struct TreeHostPanel {
    canvas: Canvas,
    composition: Rc<RefCell<CompositionRenderer>>,
    tree: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
    render_tree: Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
    /// The `Text`/`NativeControl` children currently reflected into `canvas.Children()` — see
    /// `reconcile_native_children`'s own doc comment for why this exists (in short: so relayout
    /// never has to `Clear()`/rebuild `canvas.Children()` wholesale, which is what broke Win2D's
    /// device creation for whichever tab started out selected).
    native_children: Rc<RefCell<NativeChildMap>>,
    /// Turns `canvas`'s own raw `KeyDown`/`KeyUp`/`CharacterReceived` events into
    /// `elwindui_core::ui::dispatch_routed` calls against whichever element currently has focus,
    /// and owns the `FocusTracker`/`ShortcutRegistry` for whatever tree this panel hosts — mirrors
    /// `elwindui_backend_appkit::inner::TreeHostIvars::keyboard`'s own doc comment, including its
    /// caveat: self-drawn elements' virtual focus is real, but a native leaf (`Button`/`TextArea`/
    /// `TabView`) receives real OS keyboard focus/events directly and needs its own individual
    /// wiring (see `native_ui.rs`'s `Button`/`TextArea`) — `canvas`'s own `KeyDown`/`KeyUp` below
    /// never even fire while one is focused.
    keyboard: Rc<KeyboardDispatcher>,
    /// Core routed-pointer dispatcher for self-drawn content hosted directly by `canvas`.
    pointer: Rc<PointerDispatcher>,
    /// `(width_unconstrained, height_unconstrained)` — mirrors
    /// `elwindui_backend_appkit::inner::TreeHostIvars::unconstrained_axes` (see that field's own
    /// doc comment for the full rationale). `false`/`false` (the default, every existing host) means
    /// `relayout_static` measures against `canvas`'s current explicit `Width`/`Height` (or
    /// `ActualWidth`/`ActualHeight` if unset), same as always. `true` on an axis measures that axis
    /// as unconstrained instead, growing `canvas` to the resulting natural size on that axis —
    /// `InnerScrollView`'s content host uses this. `Rc<Cell<..>>`, not a plain `Cell<..>` field on
    /// `TreeHostPanel` itself, because `relayout_static`'s own closures (`SizeChanged`, ...) need
    /// their own weak-captured handle to read it at fire time, the same pattern `render_tree`/
    /// `native_children` already use.
    unconstrained_axes: Rc<Cell<(bool, bool)>>,
    /// Gates all layout and rendering work while this host belongs to a non-selected tab.
    active: Rc<Cell<bool>>,
    /// Keeps track of an active standalone custom popup surface, if any, ensuring single-popup ownership.
    pub(crate) active_popup: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>>,
    /// Keeps every FFI callback registered by this host alive for exactly this host's lifetime.
    callback_owner: UiCallbackRegistryOwner,
}

/// `elwindui_core::ui::RelayoutHost` for `TreeHostPanel` — wraps a *weak* reference back to the
/// panel's own tree storage (not a full owned `TreeHostPanel` clone) since a strong one would
/// create a reference cycle: this panel's own `tree` strongly holds the hosted tree's root, and
/// that root's own `UIElementImpl::invalidate_host` would then strongly hold this, right back to
/// the panel. `canvas` is captured strongly, matching `TreeHostPanel::new`'s own `SizeChanged`
/// handler below, which uses the exact same capture split (strong `canvas`, weak `tree`).
///
/// Unlike AppKit's `AppKitRelayoutHost` (where `NSView.setNeedsLayout(true)` is itself already
/// coalesced by AppKit into a single pass per display cycle, no matter how many times it's called),
/// `relayout_static` here rebuilds `Canvas.Children` synchronously and from scratch — so
/// `request_relayout` debounces via `pending` + this thread's `DispatcherQueue`, matching
/// docs/design/runtime/layout_design.md's `RelayoutHost` coalescing contract: repeated calls within the same
/// synchronous burst (e.g. several property setters inside one `resync()`) collapse into a single
/// deferred relayout pass, not one synchronous pass per call.
pub(crate) struct WinUI3RelayoutHost {
    canvas: Canvas,
    composition: Weak<RefCell<CompositionRenderer>>,
    tree: Weak<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
    render_tree: Weak<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
    native_children: Weak<RefCell<NativeChildMap>>,
    /// Threaded through to `relayout_static`/`reconcile_native_children` so a genuinely new native
    /// child discovered during this relayout pass can wire its own `GotFocus`/`LostFocus` — see
    /// `reconcile_native_children`'s own doc comment on that wiring.
    keyboard: Weak<KeyboardDispatcher>,
    /// See `TreeHostPanel::unconstrained_axes`'s own doc comment.
    unconstrained_axes: Weak<Cell<(bool, bool)>>,
    /// See `TreeHostPanel::active`.
    active: Weak<Cell<bool>>,
    /// `true` while a relayout pass is already enqueued on the `DispatcherQueue` and hasn't run
    /// yet — makes `request_relayout` a no-op for any further call until that pass actually runs
    /// (and clears it right before doing so).
    pending: Cell<bool>,
    /// Lets `request_relayout` (which only ever sees `&self`) hand an owned `Rc<Self>` to the
    /// `DispatcherQueueHandler` closure — set once, right after this host is `Rc`-wrapped (see
    /// `TreeHostPanel::set_tree`), the same self-referential-`Weak` pattern
    /// `InnerTabView`'s own event wiring uses for the same reason.
    weak_self: RefCell<Weak<WinUI3RelayoutHost>>,
}

impl elwindui_core::ui::RelayoutHost for WinUI3RelayoutHost {
    // `_kind` is unused: this backend has no `InvalidationKind::Render` fast path yet (every
    // relayout is a full `relayout_static` rebuild regardless of what was invalidated) — see this
    // crate's own top-level doc comment on why it can't be built or type-checked on this machine,
    // so this mechanical signature update is deliberately kept behavior-identical rather than
    // guessed at.
    fn request_relayout(&self, dirty_group_id: u64, _kind: elwindui_core::ui::InvalidationKind) {
        let Some(active) = self.active.upgrade() else {
            return;
        };
        if !active.get() {
            return;
        }
        if let Some(render_tree) = self.render_tree.upgrade() {
            if let Some(render_tree) = render_tree.borrow_mut().as_mut() {
                render_tree.mark_dirty(dirty_group_id);
            }
        }
        if self.pending.replace(true) {
            return; // already scheduled — the pending pass will pick up this call's changes too
        }
        let Some(this) = self.weak_self.borrow().upgrade() else {
            self.pending.set(false);
            return;
        };
        this.pending.set(false);
        if let (
            Some(tree),
            Some(render_tree),
            Some(native_children),
            Some(composition),
            Some(keyboard),
            Some(unconstrained_axes),
            Some(active),
        ) = (
            this.tree.upgrade(),
            this.render_tree.upgrade(),
            this.native_children.upgrade(),
            this.composition.upgrade(),
            this.keyboard.upgrade(),
            this.unconstrained_axes.upgrade(),
            this.active.upgrade(),
        ) {
            TreeHostPanel::relayout_static(
                &this.canvas,
                &composition,
                &tree,
                &render_tree,
                &native_children,
                &keyboard,
                unconstrained_axes.get(),
                &active,
            );
        }
    }
}

/// `elwindui_core::ui::FocusHost` for `TreeHostPanel` — the `FocusHost` counterpart to
/// `WinUI3RelayoutHost`, same weak-back-reference shape (a strong one would create the same
/// `tree` -> `focus_host` -> panel reference cycle `WinUI3RelayoutHost`'s own doc comment
/// describes). Delegates straight to `keyboard.focus`, the single source of truth for this panel's
/// own hosted tree — mirrors `elwindui_backend_appkit::inner::AppKitFocusHost`.
pub(crate) struct WinUI3FocusHost {
    keyboard: Weak<KeyboardDispatcher>,
}

impl FocusHost for WinUI3FocusHost {
    fn request_focus(&self, target: &Rc<dyn elwindui_core::ui::UIElementExt>) -> bool {
        match self.keyboard.upgrade() {
            Some(keyboard) => keyboard.focus.set_focus(target, FocusState::Programmatic),
            None => false,
        }
    }
}

/// Root/screen conversion for one WinUI3 hosted tree. XAML objects support WinRT weak references,
/// so the tree does not retain its owning Canvas through this capability.
pub(crate) struct WinUI3CoordinateHost {
    canvas: windows::core::Weak<Canvas>,
}

impl CoordinateHost for WinUI3CoordinateHost {
    fn root_to_screen(&self, point: Point) -> Option<Point> {
        TreeHostPanel::canvas_to_screen_point(&self.canvas.upgrade()?, point)
    }

    fn screen_to_root(&self, point: Point) -> Option<Point> {
        TreeHostPanel::screen_to_canvas_point(&self.canvas.upgrade()?, point)
    }
}

/// Pointer-cancellation bridge for one WinUI3 hosted tree. Both references are weak so the Core
/// root cannot retain either its dispatcher or native Canvas owner.
pub(crate) struct WinUI3PointerGestureHost {
    pointer: Weak<PointerDispatcher>,
    canvas: windows::core::Weak<Canvas>,
}

impl PointerGestureHost for WinUI3PointerGestureHost {
    fn cancel_pointer_gesture_in_subtree(
        &self,
        subtree: &Rc<dyn elwindui_core::ui::UIElementExt>,
    ) -> bool {
        let canceled = self
            .pointer
            .upgrade()
            .is_some_and(|pointer| pointer.cancel_for_subtree(subtree));
        if canceled {
            if let Some(canvas) = self.canvas.upgrade() {
                let _ = canvas.ReleasePointerCaptures();
            }
        }
        canceled
    }
}

impl TreeHostPanel {
    pub(crate) fn new() -> Self {
        let canvas = Canvas::new().expect("Canvas::new");
        let composition = CompositionRenderer::new(&canvas).expect("CompositionRenderer::new");
        let this = Self {
            canvas,
            composition: Rc::new(RefCell::new(composition)),
            tree: Rc::new(RefCell::new(None)),
            render_tree: Rc::new(RefCell::new(None)),
            native_children: Rc::new(RefCell::new(NativeChildMap::new())),
            keyboard: Rc::new(KeyboardDispatcher::new()),
            pointer: Rc::new(PointerDispatcher::new()),
            unconstrained_axes: Rc::new(Cell::new((false, false))),
            active: Rc::new(Cell::new(true)),
            active_popup: Rc::new(RefCell::new(None)),
            callback_owner: UiCallbackRegistryOwner::default(),
        };
        // WinUI3's `Control.IsTabStop` gate. Once the WinRT event projection is restored this
        // allows the host to receive OS keyboard focus, mirroring AppKit's TreeHostView.
        let _ = this.canvas.SetIsTabStop(true);
        {
            let tree_for_key = Rc::downgrade(&this.tree);
            let keyboard_for_key = Rc::downgrade(&this.keyboard);
            let active_for_key = Rc::downgrade(&this.active_popup);
            let callback_id = this.callback_owner.register_key(Rc::new(move |event| {
                if let (Some(tree), Some(keyboard)) =
                    (tree_for_key.upgrade(), keyboard_for_key.upgrade())
                {
                    if let Some(tree) = tree.borrow().clone() {
                        keyboard.handle_key(&tree, event);
                    }
                }
            }));
            let _ = this
                .canvas
                .KeyDown(&KeyEventHandler::new(move |_sender, args| {
                    let Some(args) = args.cloned() else {
                        return Ok(());
                    };
                    let Ok(virtual_key) = args.Key() else {
                        return Ok(());
                    };
                    let Some(key) = winui_key(virtual_key) else {
                        return Ok(());
                    };
                    let is_repeat = args
                        .KeyStatus()
                        .map(|status| status.RepeatCount > 1)
                        .unwrap_or(false);
                    let modifiers = winui_modifiers();
                    if crate::host::event::is_context_menu_key(virtual_key, modifiers) {
                        let tree_ref = tree_for_key.upgrade().and_then(|t| t.borrow().clone());
                        let kb_ref = keyboard_for_key.upgrade();
                        let active_ref = active_for_key.upgrade();
                        if let (Some(tree), Some(kb), Some(active)) = (tree_ref, kb_ref, active_ref) {
                            let screen_anchor = if let Some(focused) = kb.focus.focused() {
                                let offset = focused.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
                                let w = focused.arranged_width().unwrap_or(0.0);
                                let h = focused.arranged_height().unwrap_or(0.0);
                                Self::canvas_to_screen_point(&this.canvas, offset).map(|screen_pt| {
                                    elwindui_core::ui::popup::PopupAnchor::Rect(elwindui_core::base::Rect {
                                        x: screen_pt.x,
                                        y: screen_pt.y,
                                        width: w,
                                        height: h,
                                    })
                                })
                            } else {
                                None
                            };
                            if let Some(screen_anchor) = screen_anchor {
                                let request = elwindui_core::ui::ContextRequest::keyboard(Some(screen_anchor));
                                if Self::dispatch_context_request(
                                    &Some(tree),
                                    &kb,
                                    &this.canvas,
                                    &active,
                                    &request,
                                ) {
                                    return Ok(());
                                }
                            }
                        }
                    }
                    invoke_ui_key_event_callback(
                        callback_id,
                        RawKeyEvent {
                            kind: RawKeyEventKind::Down { is_repeat },
                            key,
                            modifiers,
                            timestamp_ms: 0.0,
                        },
                    );
                    Ok(())
                }));
        }
        {
            let tree_for_key = Rc::downgrade(&this.tree);
            let keyboard_for_key = Rc::downgrade(&this.keyboard);
            let callback_id = this.callback_owner.register_key(Rc::new(move |event| {
                if let (Some(tree), Some(keyboard)) =
                    (tree_for_key.upgrade(), keyboard_for_key.upgrade())
                {
                    if let Some(tree) = tree.borrow().clone() {
                        keyboard.handle_key(&tree, event);
                    }
                }
            }));
            let _ = this
                .canvas
                .KeyUp(&KeyEventHandler::new(move |_sender, args| {
                    let Some(args) = args.cloned() else {
                        return Ok(());
                    };
                    let Ok(virtual_key) = args.Key() else {
                        return Ok(());
                    };
                    let Some(key) = winui_key(virtual_key) else {
                        return Ok(());
                    };
                    invoke_ui_key_event_callback(
                        callback_id,
                        RawKeyEvent {
                            kind: RawKeyEventKind::Up,
                            key,
                            modifiers: winui_modifiers(),
                            timestamp_ms: 0.0,
                        },
                    );
                    Ok(())
                }));
        }
        {
            let tree_for_text = Rc::downgrade(&this.tree);
            let keyboard_for_text = Rc::downgrade(&this.keyboard);
            let callback_id = this.callback_owner.register_text(Rc::new(move |text| {
                if let (Some(tree), Some(keyboard)) =
                    (tree_for_text.upgrade(), keyboard_for_text.upgrade())
                {
                    if let Some(tree) = tree.borrow().clone() {
                        keyboard.handle_text_input(&tree, RawTextInputEvent { text });
                    }
                }
            }));
            let _ = this.canvas.CharacterReceived(&TypedEventHandler::<
                UIElement,
                CharacterReceivedRoutedEventArgs,
            >::new(move |_sender, args| {
                let Some(args) = args.cloned() else {
                    return Ok(());
                };
                let Ok(code_unit) = args.Character() else {
                    return Ok(());
                };
                let Some(ch) = char::from_u32(code_unit as u32) else {
                    return Ok(());
                };
                if !ch.is_control() {
                    invoke_ui_text_event_callback(callback_id, ch.to_string());
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_handler = canvas.clone();
            let _ = canvas.PointerPressed(&PointerEventHandler::new(move |_sender, args| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                let Some(kind) = Self::pointer_button_kind(args, &canvas_for_handler, true) else {
                    return Ok(());
                };
                if Self::dispatch_pointer_routed(
                    &tree,
                    &pointer,
                    &keyboard,
                    &canvas_for_handler,
                    args,
                    kind,
                ) {
                    if let Ok(native_pointer) = args.Pointer() {
                        let _ = canvas_for_handler.CapturePointer(&native_pointer);
                    }
                    let _ = args.SetHandled(true);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_handler = canvas.clone();
            let _ = canvas.PointerMoved(&PointerEventHandler::new(move |_sender, args| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                if Self::dispatch_pointer_routed(
                    &tree,
                    &pointer,
                    &keyboard,
                    &canvas_for_handler,
                    args,
                    RawPointerEventKind::Moved,
                ) {
                    let _ = args.SetHandled(true);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_handler = canvas.clone();
            let _ = canvas.PointerReleased(&PointerEventHandler::new(move |_sender, args| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                let Some(kind) = Self::pointer_button_kind(args, &canvas_for_handler, false) else {
                    return Ok(());
                };
                if Self::dispatch_pointer_routed(
                    &tree,
                    &pointer,
                    &keyboard,
                    &canvas_for_handler,
                    args,
                    kind,
                ) {
                    if let Ok(native_pointer) = args.Pointer() {
                        let _ = canvas_for_handler.ReleasePointerCapture(&native_pointer);
                    }
                    let _ = args.SetHandled(true);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_handler = canvas.clone();
            let _ = canvas.PointerCanceled(&PointerEventHandler::new(move |_sender, args| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                if Self::dispatch_pointer_routed(
                    &tree,
                    &pointer,
                    &keyboard,
                    &canvas_for_handler,
                    args,
                    RawPointerEventKind::Canceled,
                ) {
                    let _ = canvas_for_handler.ReleasePointerCaptures();
                    let _ = args.SetHandled(true);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_handler = canvas.clone();
            let _ = canvas.PointerCaptureLost(&PointerEventHandler::new(move |_sender, args| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                if Self::dispatch_pointer_routed(
                    &tree,
                    &pointer,
                    &keyboard,
                    &canvas_for_handler,
                    args,
                    RawPointerEventKind::Canceled,
                ) {
                    let _ = args.SetHandled(true);
                }
                Ok(())
            }));
        }
        {
            let weak = Rc::downgrade(&this.tree);
            let weak_render_tree = Rc::downgrade(&this.render_tree);
            let weak_native_children = Rc::downgrade(&this.native_children);
            let weak_composition = Rc::downgrade(&this.composition);
            let weak_keyboard = Rc::downgrade(&this.keyboard);
            let weak_unconstrained_axes = Rc::downgrade(&this.unconstrained_axes);
            let weak_active = Rc::downgrade(&this.active);
            let canvas_for_handler = this.canvas.clone();
            let callback_id = this.callback_owner.register_event(Rc::new(move || {
                if let (
                    Some(tree),
                    Some(render_tree),
                    Some(native_children),
                    Some(composition),
                    Some(keyboard),
                    Some(unconstrained_axes),
                    Some(active),
                ) = (
                    weak.upgrade(),
                    weak_render_tree.upgrade(),
                    weak_native_children.upgrade(),
                    weak_composition.upgrade(),
                    weak_keyboard.upgrade(),
                    weak_unconstrained_axes.upgrade(),
                    weak_active.upgrade(),
                ) {
                    Self::relayout_static(
                        &canvas_for_handler,
                        &composition,
                        &tree,
                        &render_tree,
                        &native_children,
                        &keyboard,
                        unconstrained_axes.get(),
                        &active,
                    );
                }
            }));
            // `SizeChanged` fires whenever this panel's own allotted space changes (window resize,
            // or — for a `NativeTabView`'s per-tab content area — the tab strip/window resizing together)
            // — the same role `layout()` plays for AppKit's `TreeHostView`.
            let _ = this
                .canvas
                .SizeChanged(&SizeChangedEventHandler::new(move |_, _| {
                    invoke_ui_event_callback(callback_id);
                    Ok(())
                }));
        }
        {
            let tree_for_context = Rc::downgrade(&this.tree);
            let keyboard_for_context = Rc::downgrade(&this.keyboard);
            let canvas_for_context = this.canvas.clone();
            let active_for_context = Rc::downgrade(&this.active_popup);
            let _ = this.canvas.RightTapped(
                &crate::bindings::Microsoft::UI::Xaml::Input::RightTappedEventHandler::new(
                    move |_sender, args| {
                        let Some(args) = args.cloned() else {
                            return Ok(());
                        };
                        if args.Handled().unwrap_or(false) {
                            return Ok(());
                        }
                        if let (Some(tree), Some(keyboard), Some(active)) = (
                            tree_for_context.upgrade(),
                            keyboard_for_context.upgrade(),
                            active_for_context.upgrade(),
                        ) {
                            if let Some(tree) = tree.borrow().clone() {
                                let Ok(point) = args.GetPosition(&canvas_for_context) else {
                                    return Ok(());
                                };
                                let local_pt = elwindui_core::base::Point {
                                     x: point.X,
                                     y: point.Y,
                                 };
                                 if let Some(screen_pt) = Self::canvas_to_screen_point(&canvas_for_context, local_pt) {
                                     let request = elwindui_core::ui::ContextRequest::pointer(local_pt, screen_pt);
                                     if Self::dispatch_context_request(
                                         &Some(tree),
                                         &keyboard,
                                         &canvas_for_context,
                                         &active,
                                         &request,
                                     ) {
                                         let _ = args.SetHandled(true);
                                     }
                                 }
                            }
                        }
                        Ok(())
                    },
                ),
            );
        }
        {
            let tree_for_ctx = Rc::downgrade(&this.tree);
            let keyboard_for_ctx = Rc::downgrade(&this.keyboard);
            let canvas_for_ctx = this.canvas.clone();
            let active_for_ctx = Rc::downgrade(&this.active_popup);
            let _ = this.canvas.ContextRequested(
                &TypedEventHandler::new(
                    move |_sender, args: &Option<Microsoft::UI::Xaml::Input::ContextRequestedEventArgs>| {
                        let Some(args) = args.as_ref() else {
                            return Ok(());
                        };
                        let mut pt = windows::Foundation::Point::default();
                        let is_pointer = args.TryGetPosition(&canvas_for_ctx, &mut pt).unwrap_or(false);
                        let request = if is_pointer {
                            let local_pt = elwindui_core::base::Point { x: pt.X, y: pt.Y };
                            TreeHostPanel::canvas_to_screen_point(&canvas_for_ctx, local_pt)
                                .map(|screen_pt| elwindui_core::ui::ContextRequest::pointer(local_pt, screen_pt))
                        } else {
                            let screen_anchor = if let Some(keyboard) = keyboard_for_ctx.upgrade() {
                                if let Some(focused) = keyboard.focus.focused() {
                                    let offset = focused.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
                                    let w = focused.arranged_width().unwrap_or(0.0);
                                    let h = focused.arranged_height().unwrap_or(0.0);
                                    TreeHostPanel::canvas_to_screen_point(&canvas_for_ctx, offset).map(|screen_pt| {
                                        elwindui_core::ui::popup::PopupAnchor::Rect(elwindui_core::base::Rect {
                                            x: screen_pt.x,
                                            y: screen_pt.y,
                                            width: w,
                                            height: h,
                                        })
                                    })
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            screen_anchor.map(|anchor| elwindui_core::ui::ContextRequest::keyboard(Some(anchor)))
                        };
                        if let Some(request) = request {
                            if let (Some(tree), Some(keyboard), Some(active)) = (
                                tree_for_ctx.upgrade(),
                                keyboard_for_ctx.upgrade(),
                                active_for_ctx.upgrade(),
                            ) {
                                if let Some(tree) = tree.borrow().clone() {
                                    if Self::dispatch_context_request(
                                        &Some(tree),
                                        &keyboard,
                                        &canvas_for_ctx,
                                        &active,
                                        &request,
                                    ) {
                                        let _ = args.SetHandled(true);
                                    }
                                }
                            }
                        }
                        Ok(())
                    },
                ),
            );
        }
        this
    }

    /// `Canvas` receives bubbled events from native XAML children too. Only events whose original
    /// XAML source is the Canvas itself belong to the self-drawn core tree.
    fn pointer_originates_from_canvas(canvas: &Canvas, args: &PointerRoutedEventArgs) -> bool {
        args.OriginalSource()
            .ok()
            .and_then(|source| source.cast::<Canvas>().ok())
            .is_some_and(|source| source == *canvas)
    }

    fn pointer_button_kind(
        args: &PointerRoutedEventArgs,
        canvas: &Canvas,
        pressed: bool,
    ) -> Option<RawPointerEventKind> {
        use windows::UI::Input::PointerUpdateKind;

        let update = args
            .GetCurrentPoint(canvas)
            .ok()?
            .Properties()
            .ok()?
            .PointerUpdateKind()
            .ok()?;
        let button = match (pressed, update) {
            (true, PointerUpdateKind::LeftButtonPressed)
            | (false, PointerUpdateKind::LeftButtonReleased) => MouseButton::Left,
            (true, PointerUpdateKind::RightButtonPressed)
            | (false, PointerUpdateKind::RightButtonReleased) => MouseButton::Right,
            (true, PointerUpdateKind::MiddleButtonPressed)
            | (false, PointerUpdateKind::MiddleButtonReleased) => MouseButton::Middle,
            _ => return None,
        };
        Some(if pressed {
            RawPointerEventKind::Pressed(button)
        } else {
            RawPointerEventKind::Released(button)
        })
    }

    fn dispatch_pointer_routed(
        tree: &Weak<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
        pointer: &Weak<PointerDispatcher>,
        keyboard: &Weak<KeyboardDispatcher>,
        canvas: &Canvas,
        args: &PointerRoutedEventArgs,
        kind: RawPointerEventKind,
    ) -> bool {
        if !Self::pointer_originates_from_canvas(canvas, args) {
            return false;
        }
        let Some(tree) = tree.upgrade().and_then(|tree| tree.borrow().clone()) else {
            return false;
        };
        let (Some(pointer), Some(keyboard)) = (pointer.upgrade(), keyboard.upgrade()) else {
            return false;
        };
        let Ok(point) = args.GetCurrentPoint(canvas) else {
            return false;
        };
        let Ok(position) = point.Position() else {
            return false;
        };
        let local = Point {
            x: position.X,
            y: position.Y,
        };
        pointer.handle(
            &tree,
            &keyboard.focus,
            RawPointerEvent {
                kind,
                position: local,
                screen_position: Self::canvas_to_screen_point(canvas, local),
                modifiers: winui_modifiers(),
                timestamp_ms: point.Timestamp().unwrap_or(0) as f64 / 1000.0,
            },
        );
        true
    }

    pub(crate) fn as_element(&self) -> FrameworkElement {
        self.canvas
            .cast()
            .expect("Canvas must be a FrameworkElement")
    }

    pub(crate) fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    pub(crate) fn set_transparent_background(&self, transparent: bool) {
        use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
        use windows::UI::Color;

        if transparent {
            if let Ok(brush) = SolidColorBrush::new() {
                let _ = brush.SetColor(Color {
                    A: 0,
                    R: 0,
                    G: 0,
                    B: 0,
                });
                let _ = self.canvas.SetBackground(&brush);
            }
        } else if let Ok(property) = Canvas::BackgroundProperty() {
            let _ = self.canvas.ClearValue(&property);
        }
    }

    /// Forces an immediate, synchronous relayout pass against `canvas`'s *current*
    /// `ActualWidth`/`ActualHeight` — for hosts whose size is pushed in explicitly (e.g. a
    /// `TabViewItem`'s own content `Canvas`, sized by `native_ui::TabView`/`InnerTabView` rather
    /// than by native layout) rather than reliably arriving through `canvas`'s own `SizeChanged`.
    /// Confirmed necessary, not just defensive: `SetWidth`/`SetHeight` (even with
    /// `InvalidateMeasure`/`InvalidateArrange`) on such a `Canvas` does not, in practice, make its
    /// `SizeChanged` fire on any later frame either — logged and observed directly, not assumed.
    pub(crate) fn force_relayout(&self) {
        if !self.active.get() {
            if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                eprintln!("[elwindui-winui3] skipped relayout for inactive TreeHostPanel");
            }
            return;
        }
        Self::relayout_static(
            &self.canvas,
            &self.composition,
            &self.tree,
            &self.render_tree,
            &self.native_children,
            &self.keyboard,
            self.unconstrained_axes.get(),
            &self.active,
        );
    }

    /// Activates or suspends this host's layout and rendering lifecycle.
    ///
    /// Suspending removes retained Composition and native children, clears focus, and turns later
    /// invalidations and forced layouts into no-ops. Reactivating performs one full relayout from
    /// the retained logical tree, so callers do not need to replay changes made while inactive.
    pub(crate) fn set_active(&self, active: bool) {
        if self.active.replace(active) == active {
            return;
        }
        if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            eprintln!("[elwindui-winui3] TreeHostPanel active={active}");
        }
        if active {
            self.force_relayout();
            return;
        }

        if self.pointer.cancel() {
            let _ = self.canvas.ReleasePointerCaptures();
        }
        self.keyboard.focus.clear_focus();
        let _ = self
            .composition
            .borrow_mut()
            .reconcile(&self.canvas, Vec::new());
        reconcile_native_children(
            &self.canvas,
            &self.native_children,
            Vec::new(),
            &self.render_tree,
            &self.keyboard,
        );
        *self.render_tree.borrow_mut() = None;
    }

    /// See `TreeHostPanel::unconstrained_axes`'s own doc comment. `InnerScrollView` calls this once,
    /// at construction, on its nested content host; every other host leaves both `false` (the
    /// default). Structurally mirrors
    /// `elwindui_backend_appkit::inner::TreeHostView::set_unconstrained_axes`.
    pub(crate) fn set_unconstrained_axes(&self, width: bool, height: bool) {
        self.unconstrained_axes.set((width, height));
    }

    /// Replaces this host's entire content. Both composition islands and `Text`/`NativeControl`
    /// children are reconciled by `relayout_static` rather than via `Children.Clear()`: a genuinely
    /// new tree's `RenderGroup` ids never match the old tree's, so the diff naturally tears down
    /// every old child and builds every new one on its own.
    pub(crate) fn set_tree(&self, tree: Rc<dyn elwindui_core::ui::UIElementExt>) {
        self.cancel_and_unregister_current_tree();
        let host = Rc::new(WinUI3RelayoutHost {
            canvas: self.canvas.clone(),
            composition: Rc::downgrade(&self.composition),
            tree: Rc::downgrade(&self.tree),
            render_tree: Rc::downgrade(&self.render_tree),
            native_children: Rc::downgrade(&self.native_children),
            keyboard: Rc::downgrade(&self.keyboard),
            unconstrained_axes: Rc::downgrade(&self.unconstrained_axes),
            active: Rc::downgrade(&self.active),
            pending: Cell::new(false),
            weak_self: RefCell::new(Weak::new()),
        });
        *host.weak_self.borrow_mut() = Rc::downgrade(&host);
        tree.as_ui_element().set_invalidate_host(Some(host));
        tree.as_ui_element()
            .set_coordinate_host(Some(Rc::new(WinUI3CoordinateHost {
                canvas: self.canvas.downgrade().unwrap_or_default(),
            })));
        tree.as_ui_element()
            .set_pointer_gesture_host(Some(Rc::new(WinUI3PointerGestureHost {
                pointer: Rc::downgrade(&self.pointer),
                canvas: self.canvas.downgrade().unwrap_or_default(),
            })));
        tree.as_ui_element()
            .set_focus_host(Some(Rc::new(WinUI3FocusHost {
                keyboard: Rc::downgrade(&self.keyboard),
            })));
        self.keyboard.focus.clear_focus();
        self.keyboard.shortcuts().clear();
        self.keyboard.shortcuts().collect_from_tree(&tree);
        *self.tree.borrow_mut() = Some(tree);
        *self.render_tree.borrow_mut() = None;
        self.force_relayout();
    }

    /// Clears this host's tree, cleans up native composition and children, and closes any active popup.
    pub(crate) fn clear_tree(&self) {
        self.cancel_and_unregister_current_tree();
        if let Some(old) = self.active_popup.borrow_mut().take() {
            old.close();
        }
        self.keyboard.focus.clear_focus();
        self.keyboard.shortcuts().clear();
        let _ = self
            .composition
            .borrow_mut()
            .reconcile(&self.canvas, Vec::new());
        reconcile_native_children(
            &self.canvas,
            &self.native_children,
            Vec::new(),
            &self.render_tree,
            &self.keyboard,
        );
        *self.tree.borrow_mut() = None;
        *self.render_tree.borrow_mut() = None;
    }

    fn cancel_and_unregister_current_tree(&self) {
        if self.pointer.cancel() {
            let _ = self.canvas.ReleasePointerCaptures();
        }
        if let Some(old_tree) = self.tree.borrow().as_ref() {
            old_tree.set_invalidate_host(None);
            old_tree.set_coordinate_host(None);
            old_tree.set_pointer_gesture_host(None);
            old_tree.set_focus_host(None);
        }
    }

    /// Issue #162 §3.18: closes this host's own active custom popup/context-menu surface, if any —
    /// see `close_active_popup_slot`'s own doc comment for the reentrancy-safety reasoning. Shared
    /// by the existing request-replacement paths above and the owner `Window::unmount_override`
    /// path (`native_ui::window.rs`).
    pub(crate) fn close_active_popup(&self) {
        close_active_popup_slot(&self.active_popup);
    }

    /// Focuses the specified element within this host's focus tracker.
    pub(crate) fn focus_element(&self, element: &Rc<dyn elwindui_core::ui::UIElementExt>) {
        self.keyboard
            .focus
            .set_focus(element, elwindui_core::input::FocusState::Programmatic);
    }

    fn relayout_static(
        canvas: &Canvas,
        composition: &Rc<RefCell<CompositionRenderer>>,
        tree: &Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
        retained_tree: &Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
        native_children: &Rc<RefCell<NativeChildMap>>,
        keyboard: &Rc<KeyboardDispatcher>,
        unconstrained_axes: (bool, bool),
        active: &Cell<bool>,
    ) {
        if !active.get() {
            return;
        }
        use elwindui_core::base::Size as LSize;

        // `ActualWidth`/`ActualHeight` only update after a real native layout pass runs on this
        // element, which never happens for a panel used as `TabViewItem.Content` (see
        // `force_relayout`'s doc comment). When a caller has explicitly set `Width`/`Height` (not
        // `NaN`, the "unset" sentinel) ahead of calling this — e.g. `InnerTabView`'s resize
        // callback — that value is authoritative and already reflects the real available size, so
        // prefer it over the possibly-stale `ActualWidth`/`ActualHeight`.
        let explicit_width = canvas.Width().unwrap_or(f64::NAN);
        let explicit_height = canvas.Height().unwrap_or(f64::NAN);
        let width = if explicit_width.is_finite() {
            explicit_width as f32
        } else {
            canvas.ActualWidth().unwrap_or(0.0) as f32
        };
        let height = if explicit_height.is_finite() {
            explicit_height as f32
        } else {
            canvas.ActualHeight().unwrap_or(0.0) as f32
        };
        let (unconstrained_width, unconstrained_height) = unconstrained_axes;
        // `InnerScrollView`'s content host (`unconstrained_axes`, set via
        // `TreeHostPanel::set_unconstrained_axes`) measures the scrolling axis/axes as unconstrained
        // instead of clamped to `width`/`height` — mirrors
        // `elwindui_backend_appkit::inner::TreeHostView::relayout`'s own `unconstrained_axes`
        // handling; every other host has both `false` and this is a no-op.
        let available = LSize {
            width: if unconstrained_width {
                f32::INFINITY
            } else {
                width
            },
            height: if unconstrained_height {
                f32::INFINITY
            } else {
                height
            },
        };

        let tree_ref = tree.borrow();
        let Some(tree) = tree_ref.as_ref() else {
            return;
        };
        elwindui_core::ui::layout_root(tree, available);
        // Grows `canvas` to the resulting natural size on any unconstrained axis — the WinUI3-side
        // counterpart of AppKit's own post-`layout_root` `setFrame` in `TreeHostView::relayout`.
        let final_width = if unconstrained_width {
            tree.arranged_width().unwrap_or(0.0)
        } else {
            width
        };
        let final_height = if unconstrained_height {
            tree.arranged_height().unwrap_or(0.0)
        } else {
            height
        };
        if unconstrained_width || unconstrained_height {
            let _ = canvas.SetWidth(final_width as f64);
            let _ = canvas.SetHeight(final_height as f64);
        }
        {
            let mut retained_tree = retained_tree.borrow_mut();
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
        // Named distinctly from the `retained_tree: &Rc<RefCell<Option<RenderTree>>>` parameter
        // (not shadowed, unlike the `borrow_mut()` above whose own shadow already goes out of scope
        // at the end of its block) so that parameter is still reachable below, to pass into
        // `reconcile_native_children`.
        let retained_tree_ref = retained_tree.borrow();
        let Some(render_tree) = retained_tree_ref.as_ref() else {
            return;
        };

        let mut transforms = vec![elwindui_core::base::AffineTransform::identity()];
        let mut opacities = vec![1.0_f32];

        // Keyed by `(group.id, index within that group's own commands)` — see `NativeChildKey`'s
        // doc comment — so `reconcile_native_children` can tell a `Text`/`NativeControl` command
        // that's merely being updated in place apart from one that's genuinely new or gone,
        // without ever needing to `Clear()`/rebuild `canvas.Children()` wholesale.
        fn collect_commands<'a>(
            group: &'a elwindui_core::graphics::RenderGroup,
            origin: elwindui_core::base::Point,
            out: &mut Vec<(
                u64,
                usize,
                &'a elwindui_core::graphics::RenderCommand,
                elwindui_core::base::Point,
            )>,
        ) {
            let origin = elwindui_core::base::Point {
                x: origin.x + group.offset.x,
                y: origin.y + group.offset.y,
            };
            for (index, command) in group.commands.iter().enumerate() {
                out.push((group.id, index, command, origin));
            }
            for child in &group.children {
                collect_commands(child, origin, out);
            }
        }
        let mut commands = Vec::new();
        collect_commands(
            &render_tree.root,
            elwindui_core::base::Point { x: 0.0, y: 0.0 },
            &mut commands,
        );
        let mut native_wanted: Vec<(NativeChildKey, RenderedNativeChild)> = Vec::new();
        let mut composition_islands = Vec::<DesiredCompositionIsland>::new();
        let mut composition_nodes = Vec::<DesiredCompositionNode>::new();
        let mut layer_order = Vec::<RenderLayerKey>::new();
        let mut clip_stack = Vec::<CompositionClipSpec>::new();

        fn flush_composition_island(
            nodes: &mut Vec<DesiredCompositionNode>,
            islands: &mut Vec<DesiredCompositionIsland>,
            order: &mut Vec<RenderLayerKey>,
            clips: &[CompositionClipSpec],
        ) {
            if let Some(island) =
                DesiredCompositionIsland::from_nodes(std::mem::take(nodes), clips.to_vec())
            {
                order.push(RenderLayerKey::Composition(island.id));
                islands.push(island);
            }
        }

        // Composition handles every custom-drawn node. XAML controls and text remain normal
        // children of the host Canvas and are reconciled in place afterward.
        for (group_id, command_index, command, origin) in commands {
            match command {
                elwindui_core::graphics::RenderCommand::PushTransform { transform } => {
                    let next = transforms
                        .last()
                        .expect("transform stack")
                        .concat(transform);
                    transforms.push(next);
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PopTransform => {
                    if transforms.len() > 1 {
                        transforms.pop();
                    }
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PushOpacity { opacity } => {
                    let next = opacities.last().expect("opacity stack") * opacity;
                    opacities.push(next);
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PushClip { clip } => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    let transform = *transforms.last().expect("transform stack");
                    let spec = match clip {
                        elwindui_core::graphics::Clip::Rect(rect) => CompositionClipSpec::Rect {
                            rect: elwindui_core::base::Rect {
                                x: origin.x + rect.x,
                                y: origin.y + rect.y,
                                width: rect.width,
                                height: rect.height,
                            },
                            transform,
                        },
                        elwindui_core::graphics::Clip::RoundedRect { rect, radii } => {
                            CompositionClipSpec::RoundedRect {
                                rect: elwindui_core::base::Rect {
                                    x: origin.x + rect.x,
                                    y: origin.y + rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                                radii: *radii,
                                transform,
                            }
                        }
                        elwindui_core::graphics::Clip::Path { path, rule } => {
                            CompositionClipSpec::Path {
                                commands: path.commands().to_vec(),
                                rule: *rule,
                                origin,
                                transform,
                            }
                        }
                    };
                    clip_stack.push(spec);
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PopClip => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    clip_stack.pop();
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PopOpacity => {
                    if opacities.len() > 1 {
                        opacities.pop();
                    }
                    continue;
                }
                _ => {}
            }

            let node_id = (group_id, command_index);
            let transform = *transforms.last().expect("transform stack");
            let opacity = *opacities.last().expect("opacity stack");
            let absolute_rect = |rect: &elwindui_core::base::Rect| elwindui_core::base::Rect {
                x: origin.x + rect.x,
                y: origin.y + rect.y,
                width: rect.width,
                height: rect.height,
            };
            // The active clip is applied at the island root. Adjacent commands are flushed at
            // every clip boundary, so each node belongs to the innermost active clip island and
            // remains a retained Composition primitive. General intersecting nested clips still
            // need a dedicated nested-container representation before this can preserve arbitrary
            // overlapping clip regions without a surface fallback.
            let fallback_if_clipped = |primitive: CompositionPrimitive| primitive;

            let composition_node = match command {
                elwindui_core::graphics::RenderCommand::FillRect { rect, brush } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::Rectangle {
                            rect: absolute_rect(rect),
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokeRect {
                    rect,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Rectangle {
                        rect: absolute_rect(rect),
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::FillRoundedRect { rect, radii, brush } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::RoundedRectangle {
                            rect: absolute_rect(rect),
                            radii: *radii,
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokeRoundedRect {
                    rect,
                    radii,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::RoundedRectangle {
                        rect: absolute_rect(rect),
                        radii: *radii,
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::FillEllipse { rect, brush } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::Ellipse {
                            rect: absolute_rect(rect),
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokeEllipse {
                    rect,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Ellipse {
                        rect: absolute_rect(rect),
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::DrawLine {
                    from,
                    to,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Line {
                        from: elwindui_core::base::Point {
                            x: origin.x + from.x,
                            y: origin.y + from.y,
                        },
                        to: elwindui_core::base::Point {
                            x: origin.x + to.x,
                            y: origin.y + to.y,
                        },
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::FillPath { path, brush, rule } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::Path {
                            commands: path.commands().to_vec(),
                            rule: *rule,
                            origin,
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokePath {
                    path,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Path {
                        commands: path.commands().to_vec(),
                        rule: elwindui_core::graphics::FillRule::NonZero,
                        origin,
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::DrawImage {
                    image,
                    dest,
                    source,
                    options,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Rectangle {
                        rect: absolute_rect(dest),
                    }),
                    fill: Some(elwindui_core::graphics::Brush::Image(
                        elwindui_core::graphics::ImageBrush {
                            image: image.clone(),
                            source_rect: *source,
                            stretch: match options.fit {
                                elwindui_core::graphics::ImageFit::Fill => {
                                    elwindui_core::graphics::Stretch::Fill
                                }
                                elwindui_core::graphics::ImageFit::Contain => {
                                    elwindui_core::graphics::Stretch::Uniform
                                }
                                elwindui_core::graphics::ImageFit::Cover => {
                                    elwindui_core::graphics::Stretch::UniformToFill
                                }
                                elwindui_core::graphics::ImageFit::None => {
                                    elwindui_core::graphics::Stretch::None
                                }
                            },
                            alignment_x: options.alignment_x,
                            alignment_y: options.alignment_y,
                            tile_mode: options.repeat,
                            opacity: options.opacity,
                            transform: elwindui_core::base::AffineTransform::IDENTITY,
                        },
                    )),
                    stroke: None,
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::DrawVectorImage {
                    image,
                    dest,
                    source,
                    options,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: CompositionPrimitive::VectorImage {
                        image: image.clone(),
                        dest: absolute_rect(dest),
                        source: *source,
                        options: *options,
                    },
                    fill: None,
                    stroke: None,
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::Text { .. } => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    layer_order.push(RenderLayerKey::Native(node_id));
                    None
                }
                elwindui_core::graphics::RenderCommand::NativeControl { handle, .. } => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    if handle.downcast_ref::<AnyView>().is_some() {
                        layer_order.push(RenderLayerKey::Native(node_id));
                    }
                    None
                }
                elwindui_core::graphics::RenderCommand::PushClip { .. }
                | elwindui_core::graphics::RenderCommand::PopClip
                | elwindui_core::graphics::RenderCommand::PushTransform { .. }
                | elwindui_core::graphics::RenderCommand::PopTransform
                | elwindui_core::graphics::RenderCommand::PushOpacity { .. }
                | elwindui_core::graphics::RenderCommand::PopOpacity => None,
            };
            if let Some(node) = composition_node {
                composition_nodes.push(node);
            }

            match command {
                elwindui_core::graphics::RenderCommand::Text {
                    content,
                    rect,
                    style,
                    foreground,
                    alignment,
                } => {
                    native_wanted.push((
                        (group_id, command_index),
                        RenderedNativeChild::Text {
                            content: content.clone(),
                            rect: elwindui_core::base::Rect {
                                x: origin.x + rect.x,
                                y: origin.y + rect.y,
                                width: rect.width,
                                height: rect.height,
                            },
                            style: style.clone(),
                            foreground: foreground.clone(),
                            alignment: *alignment,
                        },
                    ));
                }
                elwindui_core::graphics::RenderCommand::NativeControl { handle, rect, .. } => {
                    if let Some(view) = handle.downcast_ref::<AnyView>().cloned() {
                        native_wanted.push((
                            (group_id, command_index),
                            RenderedNativeChild::Native {
                                view,
                                rect: elwindui_core::base::Rect {
                                    x: origin.x + rect.x,
                                    y: origin.y + rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                            },
                        ));
                    }
                }
                _ => {}
            }
        }
        flush_composition_island(
            &mut composition_nodes,
            &mut composition_islands,
            &mut layer_order,
            &clip_stack,
        );
        let composition_hosts = match composition
            .borrow_mut()
            .reconcile(canvas, composition_islands)
        {
            Ok((hosts, unsupported)) => {
                for unsupported in unsupported {
                    eprintln!(
                        "elwindui-winui3: render node {:?} routed to surface fallback: {}",
                        unsupported.id, unsupported.reason
                    );
                }
                hosts.into_iter().collect::<HashMap<IslandId, UIElement>>()
            }
            Err(error) => {
                eprintln!("elwindui-winui3: Composition reconciliation failed: {error}");
                HashMap::new()
            }
        };
        reconcile_native_children(
            canvas,
            native_children,
            native_wanted,
            retained_tree,
            keyboard,
        );
        {
            let native_children = native_children.borrow();
            for (z, layer) in layer_order.into_iter().enumerate() {
                let element = match layer {
                    RenderLayerKey::Composition(id) => composition_hosts.get(&id).cloned(),
                    RenderLayerKey::Native(id) => native_children
                        .get(&id)
                        .and_then(|child| child.framework_element().cast::<UIElement>().ok()),
                };
                if let Some(element) = element {
                    let _ = Canvas::SetZIndex(&element, z as i32);
                }
            }
        }
    }

    /// Converts canvas-local logical DIPs to desktop screen logical coordinates.
    pub(crate) fn canvas_to_screen_point(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        canvas_point: Point,
    ) -> Option<Point> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let scale = xaml_root.RasterizationScale().unwrap_or(1.0);
        let content = xaml_root.Content().ok()?;
        let uie: crate::bindings::Microsoft::UI::Xaml::UIElement =
            canvas.cast().ok()?;
        let transform = uie.TransformToVisual(&content).ok()?;
        let pt = windows::Foundation::Point {
            X: canvas_point.x,
            Y: canvas_point.y,
        };
        let local_dip = transform.TransformPoint(pt).ok()?;

        // Primary method: ContentCoordinateConverter
        if let Ok(island) = xaml_root.ContentIslandEnvironment() {
            if let Ok(app_window_id) = island.AppWindowId() {
                if let Ok(converter) = crate::bindings::Microsoft::UI::Content::ContentCoordinateConverter::CreateForWindowId(app_window_id) {
                    if let Ok(screen_phys) = converter.ConvertLocalToScreen(local_dip) {
                        let scale = if scale <= 0.0 { 1.0 } else { scale };
                        return Some(Point {
                            x: (screen_phys.X as f64 / scale) as f32,
                            y: (screen_phys.Y as f64 / scale) as f32,
                        });
                    }
                }
                if let Ok(app_window) = crate::bindings::Microsoft::UI::Windowing::AppWindow::GetFromWindowId(app_window_id) {
                    if let Ok(pos) = app_window.Position() {
                        return Some(canvas_local_to_screen_logical_pure(
                            Point { x: local_dip.X, y: local_dip.Y },
                            (pos.X, pos.Y),
                            scale,
                        ));
                    }
                }
            }
        }

        None
    }

    /// Converts normalized screen logical coordinates to this Canvas's own root-local DIPs.
    pub(crate) fn screen_to_canvas_point(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        screen_point: Point,
    ) -> Option<Point> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let content = xaml_root.Content().ok()?;
        let xaml_local = Self::screen_logical_to_xaml_local(canvas, screen_point)?;
        let canvas_element: crate::bindings::Microsoft::UI::Xaml::UIElement = canvas.cast().ok()?;
        let transform = content.TransformToVisual(&canvas_element).ok()?;
        let canvas_local = transform
            .TransformPoint(windows::Foundation::Point {
                X: xaml_local.x,
                Y: xaml_local.y,
            })
            .ok()?;
        Some(Point {
            x: canvas_local.X,
            y: canvas_local.Y,
        })
    }

    /// Converts desktop screen logical coordinates to XAML root local DIPs.
    pub(crate) fn screen_logical_to_xaml_local(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        screen_point: Point,
    ) -> Option<Point> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let scale = xaml_root.RasterizationScale().unwrap_or(1.0);
        let scale_safe = if scale <= 0.0 { 1.0 } else { scale };
        let screen_phys = windows::Graphics::PointInt32 {
            X: (screen_point.x as f64 * scale_safe) as i32,
            Y: (screen_point.y as f64 * scale_safe) as i32,
        };

        if let Ok(island) = xaml_root.ContentIslandEnvironment() {
            if let Ok(app_window_id) = island.AppWindowId() {
                if let Ok(converter) = crate::bindings::Microsoft::UI::Content::ContentCoordinateConverter::CreateForWindowId(app_window_id) {
                    if let Ok(local_dip) = converter.ConvertScreenToLocal(screen_phys) {
                        return Some(Point {
                            x: local_dip.X,
                            y: local_dip.Y,
                        });
                    }
                }
                if let Ok(app_window) = crate::bindings::Microsoft::UI::Windowing::AppWindow::GetFromWindowId(app_window_id) {
                    if let Ok(pos) = app_window.Position() {
                        return Some(screen_logical_to_xaml_local_pure(
                            screen_point,
                            (pos.X, pos.Y),
                            scale,
                        ));
                    }
                }
            }
        }

        None
    }

    /// Queries the real monitor work area in screen logical coordinates for the given canvas and optional anchor point.
    pub(crate) fn query_work_area_for_canvas(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        anchor_pt: Option<Point>,
    ) -> Option<elwindui_core::base::Rect> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let scale = xaml_root.RasterizationScale().unwrap_or(1.0);
        let scale_safe = if scale <= 0.0 { 1.0 } else { scale };

        // 1. Try DisplayArea::GetFromPoint if anchor_pt given
        if let Some(anchor) = anchor_pt {
            let pt_int = windows::Graphics::PointInt32 {
                X: (anchor.x as f64 * scale_safe) as i32,
                Y: (anchor.y as f64 * scale_safe) as i32,
            };
            if let Ok(display_area) = crate::bindings::Microsoft::UI::Windowing::DisplayArea::GetFromPoint(
                pt_int,
                crate::bindings::Microsoft::UI::Windowing::DisplayAreaFallback::Nearest,
            ) {
                let outer = display_area.OuterBounds().unwrap_or_default();
                let work = display_area.WorkArea().unwrap_or_default();
                return Some(display_area_to_core_work_area(
                    outer.X, outer.Y, work.X, work.Y, work.Width, work.Height, scale_safe,
                ));
            }
        }

        // 2. Try DisplayArea::GetFromWindowId
        if let Ok(island) = xaml_root.ContentIslandEnvironment() {
            if let Ok(app_window_id) = island.AppWindowId() {
                if let Ok(display_area) = crate::bindings::Microsoft::UI::Windowing::DisplayArea::GetFromWindowId(
                    app_window_id,
                    crate::bindings::Microsoft::UI::Windowing::DisplayAreaFallback::Nearest,
                ) {
                    let outer = display_area.OuterBounds().unwrap_or_default();
                    let work = display_area.WorkArea().unwrap_or_default();
                    return Some(display_area_to_core_work_area(
                        outer.X, outer.Y, work.X, work.Y, work.Width, work.Height, scale_safe,
                    ));
                }
            }
        }

        // 3. Fallback: Convert XamlRoot bounds (local DIP) explicitly to screen logical coordinates
        if let Ok(size) = xaml_root.Size() {
            if let (Some(p0), Some(p1)) = (
                Self::canvas_to_screen_point(canvas, Point { x: 0.0, y: 0.0 }),
                Self::canvas_to_screen_point(canvas, Point { x: size.Width as f32, y: size.Height as f32 }),
            ) {
                return Some(elwindui_core::base::Rect {
                    x: p0.x.min(p1.x),
                    y: p0.y.min(p1.y),
                    width: (p1.x - p0.x).abs().max(100.0),
                    height: (p1.y - p0.y).abs().max(100.0),
                });
            }
        }

        None
    }

    pub(crate) fn dispatch_context_request(
        tree: &Option<Rc<dyn UIElementExt>>,
        keyboard: &crate::host::KeyboardDispatcher,
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        active_popup: &RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>,
        request: &elwindui_core::ui::ContextRequest,
    ) -> bool {
        let Some(tree) = tree.as_ref() else {
            return false;
        };
        let Some((resolved, anchor)) = elwindui_core::ui::ContextMenuService::process_request(
            tree,
            &keyboard.focus,
            request,
        ) else {
            return false;
        };
        match resolved.definition {
            elwindui_core::ui::popup::ResolvedContextDefinition::Menu { menu, presentation } => {
                match presentation {
                    elwindui_core::ui::ContextMenuPresentation::Native => {
                        if let Some(winui_menu) = menu.as_any().downcast_ref::<crate::native_ui::Menu>() {
                            if let Ok(flyout) = winui_menu.create_flyout() {
                                let uie: crate::bindings::Microsoft::UI::Xaml::UIElement =
                                    canvas.cast().expect("Canvas as UIElement");
                                let _ = flyout.ShowAt(&uie);
                                return true;
                            }
                        }
                    }
                    elwindui_core::ui::ContextMenuPresentation::Custom => {
                        let anchor_pt = match &anchor {
                            elwindui_core::ui::popup::PopupAnchor::Point(pt) => Some(*pt),
                            elwindui_core::ui::popup::PopupAnchor::Rect(r) => Some(elwindui_core::base::Point { x: r.x, y: r.y }),
                        };
                        let Some(work_area) = Self::query_work_area_for_canvas(canvas, anchor_pt) else {
                            return false;
                        };
                        if let Some(old) = active_popup.borrow_mut().take() {
                            old.close();
                        }
                        let host = crate::inner::WinUI3PopupHost::new(canvas.clone());
                        let handle = elwindui_core::ui::ContextMenuService::open_custom_menu(
                            &host,
                            &*menu,
                            &anchor,
                            work_area,
                        );
                        let opened = handle.is_some();
                        *active_popup.borrow_mut() = handle;
                        return opened;
                    }
                }
            }
            elwindui_core::ui::popup::ResolvedContextDefinition::Popup { template } => {
                let anchor_pt = match &anchor {
                    elwindui_core::ui::popup::PopupAnchor::Point(pt) => Some(*pt),
                    elwindui_core::ui::popup::PopupAnchor::Rect(r) => Some(elwindui_core::base::Point { x: r.x, y: r.y }),
                };
                let Some(work_area) = Self::query_work_area_for_canvas(canvas, anchor_pt) else {
                    return false;
                };
                if let Some(old) = active_popup.borrow_mut().take() {
                    old.close();
                }
                let host = crate::inner::WinUI3PopupHost::new(canvas.clone());
                let handle = elwindui_core::ui::ContextMenuService::open_custom_popup(
                    &host,
                    &resolved.owner,
                    &template,
                    &anchor,
                    resolved.owner.effective_environment(),
                    work_area,
                );
                let opened = handle.is_some();
                *active_popup.borrow_mut() = handle;
                return opened;
            }
        }
        false
    }
}

/// Pure helper: converts physical display area bounds and work area offset into a global screen logical Rect.
pub fn display_area_to_core_work_area(
    outer_x: i32,
    outer_y: i32,
    work_x: i32,
    work_y: i32,
    work_width: i32,
    work_height: i32,
    scale: f64,
) -> Rect {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    let global_x = outer_x + work_x;
    let global_y = outer_y + work_y;
    Rect {
        x: (global_x as f64 / scale) as f32,
        y: (global_y as f64 / scale) as f32,
        width: (work_width as f64 / scale) as f32,
        height: (work_height as f64 / scale) as f32,
    }
}

/// Pure helper: converts canvas-to-window local DIP + window origin physical px into desktop screen logical DIP.
pub fn canvas_local_to_screen_logical_pure(
    canvas_to_window_local_dip: Point,
    window_origin_physical: (i32, i32),
    scale: f64,
) -> Point {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    let window_origin_dip_x = window_origin_physical.0 as f64 / scale;
    let window_origin_dip_y = window_origin_physical.1 as f64 / scale;
    Point {
        x: (window_origin_dip_x + canvas_to_window_local_dip.x as f64) as f32,
        y: (window_origin_dip_y + canvas_to_window_local_dip.y as f64) as f32,
    }
}

/// Pure helper: converts desktop screen logical DIP into XAML root local DIP.
pub fn screen_logical_to_xaml_local_pure(
    screen_logical: Point,
    window_origin_physical: (i32, i32),
    scale: f64,
) -> Point {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    let window_origin_dip_x = window_origin_physical.0 as f64 / scale;
    let window_origin_dip_y = window_origin_physical.1 as f64 / scale;
    Point {
        x: (screen_logical.x as f64 - window_origin_dip_x) as f32,
        y: (screen_logical.y as f64 - window_origin_dip_y) as f32,
    }
}

/// PR #165 rereview remediation round 2, A6/T25 (Layer 2): closes `slot`'s own active custom
/// popup/context-menu surface, if any — extracted out of `TreeHostPanel::close_active_popup` as a
/// free function over a bare `&RefCell<..>` (no `TreeHostPanel`/native host construction needed)
/// so it is unit-testable in isolation, mirroring `elwindui-backend-appkit`'s own identical
/// extraction (`host::close_active_popup_slot`). `take()`s the slot *before* calling `close()` so
/// a reentrant close triggered from within `close()` itself (e.g. the popup's own `on_unmount`
/// closing the owner Window again, which reaches `Window::unmount_override` ->
/// `close_active_popup` -> this same function a second time) finds the slot already empty rather
/// than double-closing it or panicking on a nested `RefCell` borrow. This crate is
/// `#![cfg(target_os = "windows")]`-gated in its entirety, so — like every other test in this same
/// module's own `#[cfg(test)] mod tests` below — this function's own unit tests cannot run in this
/// (macOS) environment regardless of how pure the function itself is; NOT VERIFIED here.
pub(crate) fn close_active_popup_slot(
    slot: &RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>,
) {
    let popup = slot.borrow_mut().take();
    if let Some(popup) = popup {
        popup.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct FakePopupSurfaceHandle {
        slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>>,
        close_count: Rc<Cell<u32>>,
        reenter: bool,
    }

    impl elwindui_core::ui::popup::PopupSurfaceHandle for FakePopupSurfaceHandle {
        fn close(&self) {
            assert!(
                self.slot.borrow().is_none(),
                "the slot must already be empty by the time PopupSurfaceHandle::close() runs"
            );
            self.close_count.set(self.close_count.get() + 1);
            if self.reenter {
                close_active_popup_slot(&self.slot);
            }
        }
    }

    /// T25 (Layer 2): a plain close — the slot holds a handle, `close_active_popup_slot` takes it
    /// (leaving the slot empty) before calling `close()`, and `close()` runs exactly once.
    #[test]
    fn close_active_popup_slot_takes_before_close_and_closes_exactly_once() {
        let slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        let close_count = Rc::new(Cell::new(0));
        let handle: Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle> =
            Rc::new(FakePopupSurfaceHandle {
                slot: slot.clone(),
                close_count: close_count.clone(),
                reenter: false,
            });
        *slot.borrow_mut() = Some(handle);

        close_active_popup_slot(&slot);

        assert_eq!(close_count.get(), 1);
        assert!(slot.borrow().is_none());
    }

    /// T25 (Layer 2): an empty slot is a no-op — no panic, nothing closed.
    #[test]
    fn close_active_popup_slot_on_empty_slot_is_a_no_op() {
        let slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        close_active_popup_slot(&slot);
        assert!(slot.borrow().is_none());
    }

    /// T25 (Layer 2): reentrancy safety — `PopupSurfaceHandle::close()` itself calls back into
    /// `close_active_popup_slot` on the *same* slot. Must not panic on a nested `RefCell` borrow,
    /// and the reentrant call must observe an already-empty slot (no second close).
    #[test]
    fn close_active_popup_slot_is_reentrancy_safe() {
        let slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        let close_count = Rc::new(Cell::new(0));
        let handle: Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle> =
            Rc::new(FakePopupSurfaceHandle {
                slot: slot.clone(),
                close_count: close_count.clone(),
                reenter: true,
            });
        *slot.borrow_mut() = Some(handle);

        close_active_popup_slot(&slot);

        assert_eq!(
            close_count.get(),
            1,
            "the reentrant call must find the slot already empty and close nothing a second time"
        );
        assert!(slot.borrow().is_none());
    }

    #[test]
    fn display_area_work_area_primary_monitor_scale_1() {
        let work = display_area_to_core_work_area(0, 0, 0, 0, 1920, 1040, 1.0);
        assert_eq!(work.x, 0.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1040.0);
    }

    #[test]
    fn display_area_work_area_secondary_monitor_right() {
        // Secondary monitor to the right at (1920, 0) with work area offset (0, 0)
        let work = display_area_to_core_work_area(1920, 0, 0, 0, 1920, 1080, 1.0);
        assert_eq!(work.x, 1920.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_secondary_monitor_left_negative_x() {
        // Secondary monitor to the left at (-1920, 0) with work area offset (0, 0)
        let work = display_area_to_core_work_area(-1920, 0, 0, 0, 1920, 1080, 1.0);
        assert_eq!(work.x, -1920.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_secondary_left_with_left_taskbar() {
        // Secondary monitor at (-1920, 0) with 60px left taskbar offset
        let work = display_area_to_core_work_area(-1920, 0, 60, 0, 1860, 1080, 1.0);
        assert_eq!(work.x, -1860.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1860.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_taskbar_top() {
        // Taskbar at top (offset y = 40)
        let work_top = display_area_to_core_work_area(0, 0, 0, 40, 1920, 1040, 1.0);
        assert_eq!(work_top.x, 0.0);
        assert_eq!(work_top.y, 40.0);
        assert_eq!(work_top.width, 1920.0);
        assert_eq!(work_top.height, 1040.0);
    }

    #[test]
    fn display_area_work_area_negative_y_monitor() {
        // Secondary monitor above primary at (0, -1080)
        let work = display_area_to_core_work_area(0, -1080, 0, 0, 1920, 1080, 1.0);
        assert_eq!(work.x, 0.0);
        assert_eq!(work.y, -1080.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_fractional_scale() {
        // High-DPI monitor with scale 1.5 (3840x2160 physical -> 2560x1440 logical)
        let work_15 = display_area_to_core_work_area(0, 0, 0, 0, 3840, 2160, 1.5);
        assert_eq!(work_15.width, 2560.0);
        assert_eq!(work_15.height, 1440.0);

        // Scale 2.0 on secondary right monitor
        let work_20 = display_area_to_core_work_area(1920, 0, 0, 0, 3840, 2160, 2.0);
        assert_eq!(work_20.x, 960.0);
        assert_eq!(work_20.width, 1920.0);
        assert_eq!(work_20.height, 1080.0);
    }

    #[test]
    fn coordinate_round_trip_canvas_to_screen_to_xaml_local() {
        let canvas_local = Point { x: 50.0, y: 75.0 };
        let window_phys = (300, 450); // window at (300, 450) physical px
        let scale = 1.5;

        let screen = canvas_local_to_screen_logical_pure(canvas_local, window_phys, scale);
        // window_phys (300, 450) at scale 1.5 -> (200.0, 300.0) DIP
        // screen -> (200 + 50, 300 + 75) = (250.0, 375.0)
        assert_eq!(screen.x, 250.0);
        assert_eq!(screen.y, 375.0);

        let xaml_local = screen_logical_to_xaml_local_pure(screen, window_phys, scale);
        assert_eq!(xaml_local.x, 50.0);
        assert_eq!(xaml_local.y, 75.0);
    }

    #[test]
    fn coordinate_conversion_negative_window_origin() {
        let canvas_local = Point { x: 10.0, y: 20.0 };
        let window_phys = (-1920, -100);
        let scale = 1.0;

        let screen = canvas_local_to_screen_logical_pure(canvas_local, window_phys, scale);
        assert_eq!(screen.x, -1910.0);
        assert_eq!(screen.y, -80.0);

        let local = screen_logical_to_xaml_local_pure(screen, window_phys, scale);
        assert_eq!(local.x, 10.0);
        assert_eq!(local.y, 20.0);
    }
}
