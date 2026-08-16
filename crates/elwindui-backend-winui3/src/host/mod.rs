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
    CharacterReceivedRoutedEventArgs, KeyEventHandler,
};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, SizeChangedEventHandler, UIElement};
use crate::render::composition::{
    CompositionClipSpec, CompositionPrimitive, CompositionRenderer, DesiredCompositionIsland,
    DesiredCompositionNode, IslandId,
};
use elwindui_core::input::{
    FocusState, KeyboardDispatcher, RawKeyEvent, RawKeyEventKind, RawTextInputEvent,
};
use elwindui_core::ui::{FocusHost, UIElementExt as _};
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
                            let request = elwindui_core::ui::ContextRequest::keyboard();
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
                                let screen_pt = Self::canvas_to_screen_point(&canvas_for_context, local_pt);
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
                            let screen_pt = TreeHostPanel::canvas_to_screen_point(&canvas_for_ctx, local_pt);
                            elwindui_core::ui::ContextRequest::pointer(local_pt, screen_pt)
                        } else {
                            elwindui_core::ui::ContextRequest::keyboard()
                        };
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
                        Ok(())
                    },
                ),
            );
        }
        this
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

    /// Converts canvas-local logical DIPs to screen logical coordinates.
    pub(crate) fn canvas_to_screen_point(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        canvas_point: Point,
    ) -> Point {
        if let Ok(xaml_root) = canvas.XamlRoot() {
            if let Ok(content) = xaml_root.Content() {
                let uie: crate::bindings::Microsoft::UI::Xaml::UIElement =
                    canvas.cast().expect("Canvas as UIElement");
                if let Ok(transform) = uie.TransformToVisual(&content) {
                    let pt = windows::Foundation::Point {
                        X: canvas_point.x,
                        Y: canvas_point.y,
                    };
                    if let Ok(transformed) = transform.TransformPoint(pt) {
                        return Point {
                            x: transformed.X,
                            y: transformed.Y,
                        };
                    }
                }
            }
        }
        canvas_point
    }

    /// Queries the real monitor work area in screen logical coordinates for the given canvas and optional anchor point.
    pub(crate) fn query_work_area_for_canvas(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        anchor_pt: Option<Point>,
    ) -> elwindui_core::base::Rect {
        let scale = if let Ok(xaml_root) = canvas.XamlRoot() {
            xaml_root.RasterizationScale().unwrap_or(1.0)
        } else {
            1.0
        };

        if let Some(anchor) = anchor_pt {
            let pt_int = windows::Graphics::PointInt32 {
                X: (anchor.x as f64 * scale) as i32,
                Y: (anchor.y as f64 * scale) as i32,
            };
            if let Ok(display_area) = crate::bindings::Microsoft::UI::Windowing::DisplayArea::GetFromPoint(
                pt_int,
                crate::bindings::Microsoft::UI::Windowing::DisplayAreaFallback::Nearest,
            ) {
                if let Ok(work_area) = display_area.WorkArea() {
                    return elwindui_core::base::Rect {
                        x: (work_area.X as f64 / scale) as f32,
                        y: (work_area.Y as f64 / scale) as f32,
                        width: (work_area.Width as f64 / scale) as f32,
                        height: (work_area.Height as f64 / scale) as f32,
                    };
                }
            }
        }

        if let Ok(xaml_root) = canvas.XamlRoot() {
            if let Ok(size) = xaml_root.Size() {
                return elwindui_core::base::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: size.Width as f32,
                    height: size.Height as f32,
                };
            }
        }
        let fe: crate::bindings::Microsoft::UI::Xaml::FrameworkElement =
            canvas.cast().expect("Canvas as FrameworkElement");
        let w = fe.ActualWidth().unwrap_or(800.0) as f32;
        let h = fe.ActualHeight().unwrap_or(600.0) as f32;
        elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: w.max(100.0),
            height: h.max(100.0),
        }
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
                        if let Some(old) = active_popup.borrow_mut().take() {
                            old.close();
                        }
                        let host = crate::inner::WinUI3PopupHost;
                        let anchor_pt = match &anchor {
                            elwindui_core::ui::popup::PopupAnchor::Point(pt) => Some(*pt),
                            elwindui_core::ui::popup::PopupAnchor::Rect(r) => Some(elwindui_core::base::Point { x: r.x, y: r.y }),
                        };
                        let work_area = Self::query_work_area_for_canvas(canvas, anchor_pt);
                        let handle = elwindui_core::ui::ContextMenuService::open_custom_menu(
                            &host,
                            &*menu,
                            &anchor,
                            work_area,
                        );
                        *active_popup.borrow_mut() = Some(handle);
                        return true;
                    }
                }
            }
            elwindui_core::ui::popup::ResolvedContextDefinition::Popup { template } => {
                if let Some(old) = active_popup.borrow_mut().take() {
                    old.close();
                }
                let host = crate::inner::WinUI3PopupHost;
                let anchor_pt = match &anchor {
                    elwindui_core::ui::popup::PopupAnchor::Point(pt) => Some(*pt),
                    elwindui_core::ui::popup::PopupAnchor::Rect(r) => Some(elwindui_core::base::Point { x: r.x, y: r.y }),
                };
                let work_area = Self::query_work_area_for_canvas(canvas, anchor_pt);
                let handle = elwindui_core::ui::ContextMenuService::open_custom_popup(
                    &host,
                    &template,
                    &anchor,
                    resolved.owner.effective_environment(),
                    work_area,
                );
                *active_popup.borrow_mut() = Some(handle);
                return true;
            }
        }
        false
    }
}
