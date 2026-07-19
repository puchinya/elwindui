//! Native-side AppKit plumbing — every type here is `Inner`-prefixed and, except for `AnyView`
//! itself (re-exported at the crate root; see `lib.rs`'s own doc comment), private to this crate.
//! `native_ui.rs` composes these as plain fields and calls into them; this module owns every bit
//! of genuinely AppKit-specific complexity (NSTextView delegates, tab strip bookkeeping, ...) so
//! `native_ui.rs` stays a thin, uniform "implement the core-side trait by delegating" layer.

use elwindui_core::base::{AsAny, Point};
use elwindui_core::input::{
    FocusState, Key, KeyModifiers, KeyboardDispatcher, MouseButton, PointerDispatcher, RawKeyEvent,
    RawKeyEventKind, RawPointerEvent, RawPointerEventKind, RawTextInputEvent, ShortcutRegistry,
};
use elwindui_core::painter::{RenderCommand, RenderGroup};
use elwindui_core::ui::{FocusHost, RelayoutHost, UIElementExt, layout_root};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSButton, NSEvent,
    NSEventModifierFlags, NSImage, NSMenu, NSMenuItem, NSScreen, NSScrollView, NSStackView,
    NSTextDelegate, NSTextView, NSTextViewDelegate, NSTrackingArea, NSTrackingAreaOptions,
    NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGColor, CGColorSpace, CGDataProvider, CGImage, CGMutablePath};
use objc2_foundation::{NSArray, NSNotification, NSNumber, NSObjectProtocol, NSRect, NSString};
use objc2_quartz_core::{
    CAGradientLayer, CALayer, CAShapeLayer, CAShapeLayerLineCap, CAShapeLayerLineJoin, CATextLayer,
    CATextLayerAlignmentMode, kCAAlignmentCenter, kCAAlignmentLeft, kCAAlignmentRight,
    kCAFillRuleEvenOdd, kCAFillRuleNonZero, kCAFilterLinear, kCAFilterNearest,
    kCAGradientLayerAxial, kCAGradientLayerRadial, kCALineCapButt, kCALineCapRound,
    kCALineCapSquare, kCALineJoinBevel, kCALineJoinMiter, kCALineJoinRound,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// `NSEvent.modifierFlags()` -> `elwindui_core::input::KeyModifiers`.
fn nsevent_modifiers(event: &NSEvent) -> KeyModifiers {
    let flags = event.modifierFlags();
    KeyModifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        meta: flags.contains(NSEventModifierFlags::Command),
    }
}

/// `NSEvent.keyCode()` (a fixed physical-key code, not layout-remapped) -> `elwindui_core::input::
/// Key` for the named keys `Key` distinguishes; every other key falls back to
/// `charactersIgnoringModifiers()`'s first character (`Key::Character`, layout-dependent —
/// see that variant's own doc comment). The named-key codes below are macOS's standard (and
/// long-stable) virtual keycodes for the US keyboard's physical key positions.
fn nsevent_key(event: &NSEvent) -> Option<Key> {
    let key = match event.keyCode() {
        36 => Some(Key::Enter),
        48 => Some(Key::Tab),
        49 => Some(Key::Space),
        51 => Some(Key::Backspace),
        53 => Some(Key::Escape),
        117 => Some(Key::Delete),
        115 => Some(Key::Home),
        119 => Some(Key::End),
        116 => Some(Key::PageUp),
        121 => Some(Key::PageDown),
        123 => Some(Key::Left),
        124 => Some(Key::Right),
        125 => Some(Key::Down),
        126 => Some(Key::Up),
        122 => Some(Key::F1),
        120 => Some(Key::F2),
        99 => Some(Key::F3),
        118 => Some(Key::F4),
        96 => Some(Key::F5),
        97 => Some(Key::F6),
        98 => Some(Key::F7),
        100 => Some(Key::F8),
        101 => Some(Key::F9),
        109 => Some(Key::F10),
        103 => Some(Key::F11),
        111 => Some(Key::F12),
        _ => None,
    };
    key.or_else(|| {
        event
            .charactersIgnoringModifiers()
            .and_then(|s| s.to_string().chars().next())
            .map(Key::Character)
    })
}

/// Depth-first, `visual_children()`-based walk feeding every element's own
/// `UIElementExt::declared_shortcuts()` into `registry` — see `crate::input::ShortcutDecl`'s own
/// doc comment for why this can't happen at construction time.
fn collect_shortcuts_into(tree: &Rc<dyn UIElementExt>, registry: &ShortcutRegistry) {
    for decl in tree.declared_shortcuts() {
        registry.register(decl.chord, decl.scope, tree.clone(), decl.event_name);
    }
    for child in tree.visual_children() {
        collect_shortcuts_into(&child, registry);
    }
}

pub(crate) fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("elwindui-backend-appkit must run on the main thread")
}

/// The capability a type needs to be usable as an `AnyView` — implemented once per raw native view
/// type (`Retained<NSScrollView>`/`Retained<NSButton>`/`Retained<NSStackView>`) instead of matched
/// on centrally, so a future native leaf only needs its own `impl AppKitHandle`, never a change to
/// `AnyView` itself or to any `match` over it.
trait AppKitHandle: AsAny {
    fn as_nsview(&self) -> Retained<NSView>;
}

impl AppKitHandle for Retained<NSScrollView> {
    fn as_nsview(&self) -> Retained<NSView> {
        Retained::into_super(self.clone())
    }
}
impl AppKitHandle for Retained<NSButton> {
    fn as_nsview(&self) -> Retained<NSView> {
        let control: Retained<objc2_app_kit::NSControl> = Retained::into_super(self.clone());
        Retained::into_super(control)
    }
}
impl AppKitHandle for Retained<NSStackView> {
    fn as_nsview(&self) -> Retained<NSView> {
        Retained::into_super(self.clone())
    }
}

/// Everything the generated code can pass as a `Window`/`TabView` child. An `Rc<dyn AppKitHandle>`
/// (not a closed `enum`) so adding a new native leaf never requires touching this type — see
/// `AppKitHandle`'s own doc comment. Re-exported at the crate root (`lib.rs`) since
/// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly.
#[derive(Clone)]
pub struct AnyView(Rc<dyn AppKitHandle>);

impl AnyView {
    /// Stable identity of the retained native handle. Reusing its container across relayouts is
    /// essential: AppKit resigns a control that is temporarily removed from its superview.
    fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as *const () as usize
    }

    fn as_nsview(&self) -> Retained<NSView> {
        self.0.as_nsview()
    }

    /// Lets every native leaf's `measure_override` (in `native_ui.rs::NativeControl`) measure any
    /// wrapped widget uniformly through the base `NSView` API (`fittingSize`) regardless of which
    /// concrete widget it wraps.
    pub(crate) fn measure(
        &self,
        _available: elwindui_core::base::Size,
    ) -> elwindui_core::base::Size {
        let fitting = self.as_nsview().fittingSize();
        elwindui_core::base::Size {
            width: fitting.width as f32,
            height: fitting.height as f32,
        }
    }

    /// Positions this native leaf via plain `NSView.setFrame` — called directly by `TreeHostView`'s
    /// own render loop below, after `layout_root` and RenderTree reconciliation have produced its
    /// retained native command.
    fn arrange(&mut self, final_rect: elwindui_core::base::Rect) {
        self.as_nsview().setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(final_rect.x as f64, final_rect.y as f64),
            objc2_foundation::NSSize::new(final_rect.width as f64, final_rect.height as f64),
        ));
    }
}

impl<T: AppKitHandle + 'static> From<T> for AnyView {
    fn from(v: T) -> Self {
        AnyView(Rc::new(v))
    }
}

fn new_stack(
    children: Vec<AnyView>,
    orientation: NSUserInterfaceLayoutOrientation,
) -> Retained<NSStackView> {
    let m = mtm();
    let views: Vec<Retained<NSView>> = children.iter().map(AnyView::as_nsview).collect();
    let ns =
        NSStackView::stackViewWithViews(&objc2_foundation::NSArray::from_retained_slice(&views), m);
    ns.setOrientation(orientation);
    ns
}

/// Parses a `"#RRGGBB"`/`"#RRGGBBAA"` hex color (the only form `Rectangle`/`Ellipse`'s `fill`/
/// `stroke` params accept — see docs/elwindui_builtins_spec.md 付録N/G) into a `CGColor`. An
/// unparseable string falls back to opaque black rather than panicking, since this runs during
/// layout, not construction.
fn parse_color(hex: &str) -> objc2_core_foundation::CFRetained<CGColor> {
    let hex = hex.trim_start_matches('#');
    let (r, g, b, a) = match (hex.len(), u32::from_str_radix(hex, 16)) {
        (6, Ok(v)) => (
            ((v >> 16) & 0xFF) as f64,
            ((v >> 8) & 0xFF) as f64,
            (v & 0xFF) as f64,
            255.0,
        ),
        (8, Ok(v)) => (
            ((v >> 24) & 0xFF) as f64,
            ((v >> 16) & 0xFF) as f64,
            ((v >> 8) & 0xFF) as f64,
            (v & 0xFF) as f64,
        ),
        _ => (0.0, 0.0, 0.0, 255.0),
    };
    CGColor::new_generic_rgb(r / 255.0, g / 255.0, b / 255.0, a / 255.0)
}

fn intersect_rect(
    a: elwindui_core::base::Rect,
    b: elwindui_core::base::Rect,
) -> Option<elwindui_core::base::Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then_some(elwindui_core::base::Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

/// `elwindui_core::ui::TextAlignment` -> `CATextLayer.alignmentMode` — the `kCAAlignment*` values
/// are `extern "C"` globals (`&'static NSString`), hence the `unsafe` read.
fn ca_alignment_mode(
    alignment: elwindui_core::ui::TextAlignment,
) -> &'static CATextLayerAlignmentMode {
    use elwindui_core::ui::TextAlignment;
    unsafe {
        match alignment {
            TextAlignment::Left => kCAAlignmentLeft,
            TextAlignment::Center => kCAAlignmentCenter,
            TextAlignment::Right => kCAAlignmentRight,
        }
    }
}

/// The single reusable "reflect an `Rc<dyn elwindui_core::ui::UIElement>` into real `NSView`
/// subviews/`CAShapeLayer`/`CATextLayer` sublayers" host — `InnerWindow`'s content view and
/// `InnerTabView`'s per-tab content area both are one of these.
pub struct TreeHostIvars {
    tree: RefCell<Option<Rc<dyn UIElementExt>>>,
    /// The retained core-side rendering description for the currently hosted Visual tree.
    render_tree: RefCell<Option<elwindui_core::painter::RenderTree>>,
    /// Native compositor islands, keyed by `AnyView` identity. They must survive ordinary
    /// relayouts so the first responder is not detached from the view hierarchy.
    native_containers: RefCell<HashMap<usize, Retained<NSView>>>,
    /// Decoded-image cache (`RenderCommand::DrawImage`'s `elwindui_core::painter::Image` -> real
    /// `CGImage`), keyed by the `Image`'s own pointer identity — see `resolve_cgimage`'s own doc
    /// comment. Never cleared piecemeal (unlike `native_containers`): a stale entry for an
    /// `Image` no longer referenced by the current tree is simply harmless dead weight, not
    /// incorrect, and pruning it would need the same kind of `retain`-by-liveness bookkeeping
    /// `native_containers` has for comparatively little benefit (a decoded `CGImage` is far
    /// cheaper to keep around than a live `NSView` island).
    image_cache: RefCell<HashMap<usize, CFRetained<CGImage>>>,
    /// Set once, right after construction — lets `set_tree` hand out an `AppKitRelayoutHost`
    /// wrapping a weak reference back to this same view, without needing a `Retained<Self>` in
    /// hand at that point.
    weak_self: RefCell<objc2::rc::Weak<TreeHostView>>,
    /// Turns this view's own raw `NSEvent`s into `elwindui_core::ui::hit_test`/`dispatch_routed`
    /// calls against `tree` — see `elwindui_core::input::PointerDispatcher`'s own doc comment.
    /// `docs/elwindui_gui_framework_design.md` §5.10's currently-implemented range: self-drawn
    /// elements only, since a native subview (`Button`/`TextArea`/`TabView`, laid out as its own
    /// `native_containers` island) receives the OS mouse event directly via ordinary AppKit
    /// hit-testing, never reaching this view's own overrides below at all.
    pointer: PointerDispatcher,
    /// Turns this view's own raw key/text events into `elwindui_core::ui::dispatch_routed` calls
    /// against whichever element currently has focus, and owns the `FocusTracker`/
    /// `ShortcutRegistry` for whatever tree this view hosts — see
    /// `elwindui_core::input::KeyboardDispatcher`'s own doc comment. `docs/elwindui_gui_framework_
    /// design.md` §5.5/§8.1's currently-implemented range mirrors `pointer`'s own: self-drawn
    /// elements' virtual focus is real (`KeyboardDispatcher::focus` is the single source of truth),
    /// but a native leaf (`Button`/`TextArea`/`TabView`) receives real OS keyboard focus/events
    /// directly and needs its own individual wiring (see `native_ui.rs`'s `Button`/`TextArea`) —
    /// this view's own `keyDown:`/`keyUp:` overrides below never even fire while one is focused.
    keyboard: KeyboardDispatcher,
    /// The single `NSTrackingArea` this view keeps registered for itself, so `updateTrackingAreas`
    /// can remove the previous one before installing a freshly-sized replacement rather than
    /// accumulating a new one on every resize.
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
}

/// `elwindui_core::ui::RelayoutHost` for `TreeHostView` — wraps a *weak* reference back to the view
/// (not the view itself) since a strong one would create a reference cycle. `request_relayout`
/// silently does nothing if the view has since been deallocated (`load()` returns `None`).
struct AppKitRelayoutHost(objc2::rc::Weak<TreeHostView>);

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
struct AppKitFocusHost(objc2::rc::Weak<TreeHostView>);

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
            image_cache: RefCell::new(HashMap::new()),
            weak_self: RefCell::new(objc2::rc::Weak::default()),
            pointer: PointerDispatcher::new(),
            keyboard: KeyboardDispatcher::new(),
            tracking_area: RefCell::new(None),
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
        let tree = self.ivars().tree.borrow();
        let Some(tree) = tree.as_ref() else { return };
        self.ivars().pointer.handle(
            tree,
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
        let weak_self = self.ivars().weak_self.borrow().clone();
        tree.as_ui_element()
            .set_invalidate_host(Some(Rc::new(AppKitRelayoutHost(weak_self.clone()))));
        tree.as_ui_element()
            .set_focus_host(Some(Rc::new(AppKitFocusHost(weak_self))));
        self.ivars().keyboard.focus.clear_focus();
        self.ivars().keyboard.shortcuts().clear();
        collect_shortcuts_into(&tree, self.ivars().keyboard.shortcuts());
        *self.ivars().tree.borrow_mut() = Some(tree);
        *self.ivars().render_tree.borrow_mut() = None;
        self.invalidateIntrinsicContentSize();
        self.relayout();
    }

    fn relayout(&self) {
        use elwindui_core::base::Size;

        let frame = self.frame();
        let available = Size {
            width: frame.size.width as f32,
            height: frame.size.height as f32,
        };
        let tree = self.ivars().tree.borrow();
        let Some(tree) = tree.as_ref() else { return };
        layout_root(tree, available);
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
                *retained_tree = Some(elwindui_core::painter::RenderTree::new::<AnyView>(tree));
            }
        }
        let render_tree = self.ivars().render_tree.borrow();
        let Some(render_tree) = render_tree.as_ref() else {
            return;
        };

        self.setWantsLayer(true);
        let layer = self.layer().expect("wantsLayer(true) implies a layer");
        if let Some(existing) = unsafe { layer.sublayers() } {
            let stale: Vec<_> = existing
                .iter()
                .filter(|sub| {
                    sub.name().map(|n| n.to_string()).as_deref() == Some("elwindui-paint")
                })
                .collect();
            for sub in stale {
                sub.removeFromSuperlayer();
            }
        }

        let mut live_native_controls = HashSet::new();
        let mut image_cache = self.ivars().image_cache.borrow_mut();
        replay_group(
            self,
            &layer,
            &render_tree.root,
            elwindui_core::base::Point { x: 0.0, y: 0.0 },
            None,
            elwindui_core::base::AffineTransform::identity(),
            1.0,
            &mut live_native_controls,
            &mut image_cache,
        );
        drop(image_cache);
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
    }
}

/// One retained-render replay pass over a `RenderGroup` tree, appending real `CALayer`s to
/// `layer` (ordinary painted content) and real `NSView` islands to `host` (native controls),
/// in traversal order so both interleave in the correct Z order (painter design doc §14.2's
/// "single custom drawing surface" intent, adapted to AppKit's native layer-composition model
/// rather than a `NSView.draw(_:)`/`CGContext` replay — `CAShapeLayer`/`CAGradientLayer` already
/// cover fill/stroke/dash/cap/join/miter/gradient natively, so a full `CGContext`-based rewrite
/// would only add complexity without adding capability here). `transform`/`opacity` are plain
/// accumulators (composed/multiplied down the recursion, applied when building each leaf's own
/// geometry/`opacity` — not modeled as extra nested `CALayer`s, which would need fighting
/// `CALayer`'s anchor-point-relative transform semantics for no benefit) — `clip` is the one
/// state that genuinely needs geometry-level handling, done here as a simple bounding-box
/// intersection test (skip a leaf whose rect doesn't overlap `clip` at all) rather than true
/// per-pixel masking, mirroring `Shape::hit_test_content`'s own "whole bounding rect, not
/// per-pixel" simplification elsewhere in this codebase.
fn replay_group(
    host: &TreeHostView,
    layer: &Retained<CALayer>,
    group: &RenderGroup,
    origin: elwindui_core::base::Point,
    inherited_clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    live_native_controls: &mut HashSet<usize>,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let origin = elwindui_core::base::Point {
        x: origin.x + group.offset.x,
        y: origin.y + group.offset.y,
    };
    let group_clip = group.clip.map(|clip| elwindui_core::base::Rect {
        x: origin.x + clip.x,
        y: origin.y + clip.y,
        width: clip.width,
        height: clip.height,
    });
    let effective_clip = match (inherited_clip, group_clip) {
        (Some(a), Some(b)) => intersect_rect(a, b),
        (Some(clip), None) | (None, Some(clip)) => Some(clip),
        (None, None) => None,
    };
    replay_commands(
        host,
        layer,
        &group.commands,
        0,
        origin,
        effective_clip,
        transform,
        opacity,
        live_native_controls,
        image_cache,
    );
    for child in &group.children {
        replay_group(
            host,
            layer,
            child,
            origin,
            effective_clip,
            transform,
            opacity,
            live_native_controls,
            image_cache,
        );
    }
}

/// Replays one `RenderGroup`'s own (flat) command list, starting at `commands[start]`. A `Push*`
/// command recurses with the updated accumulator (`transform`/`opacity`) or (for `PushClip`, the
/// one state needing real geometry) an intersected `clip`; the matching `Pop*` — always the first
/// `Pop*` this recursive call sees, since `RenderContext`'s own `push_*`/`pop_*` pair 1:1 in LIFO
/// order regardless of *kind* (see `elwindui_core::painter::context`'s `stack_depth` counter) —
/// ends that call and returns control to the caller's own loop. Returns the index just past the
/// consumed slice.
#[allow(clippy::too_many_arguments)]
fn replay_commands(
    host: &TreeHostView,
    layer: &Retained<CALayer>,
    commands: &[RenderCommand],
    start: usize,
    origin: elwindui_core::base::Point,
    clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    live_native_controls: &mut HashSet<usize>,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) -> usize {
    let mut idx = start;
    while idx < commands.len() {
        match &commands[idx] {
            RenderCommand::PopClip | RenderCommand::PopTransform | RenderCommand::PopOpacity => {
                return idx + 1;
            }
            RenderCommand::PushClip { clip: pushed } => {
                let pushed_rect = clip_bounds(pushed, origin);
                let new_clip = match (clip, pushed_rect) {
                    (Some(a), Some(b)) => intersect_rect(a, b),
                    (Some(c), None) | (None, Some(c)) => Some(c),
                    (None, None) => None,
                };
                // Real per-pixel clipping (rounded corners, path shapes), not just `new_clip`'s
                // bounding-box culling test above: a masked container layer, sized to exactly
                // overlay `layer` (`frame = layer.bounds()`, so its local coordinate space stays
                // the same shared canvas-absolute space every other sublayer here already uses —
                // no re-anchoring needed, unlike `try_add_gradient_fill_layer`'s own mask). Nested
                // `PushClip`s recurse into their own container-of-a-container, so ancestor masks
                // compose via ordinary `CALayer.mask` nesting.
                let world = elwindui_core::base::AffineTransform::translation(origin.x, origin.y)
                    .concat(&transform);
                let container = CALayer::new();
                container.setName(Some(&NSString::from_str("elwindui-paint")));
                container.setFrame(layer.bounds());
                let mask_layer = clip_mask_layer(&world, pushed);
                unsafe { container.setMask(Some(&mask_layer)) };
                layer.addSublayer(&container);
                idx = replay_commands(
                    host,
                    &container,
                    commands,
                    idx + 1,
                    origin,
                    new_clip,
                    transform,
                    opacity,
                    live_native_controls,
                    image_cache,
                );
            }
            RenderCommand::PushTransform { transform: pushed } => {
                idx = replay_commands(
                    host,
                    layer,
                    commands,
                    idx + 1,
                    origin,
                    clip,
                    transform.concat(pushed),
                    opacity,
                    live_native_controls,
                    image_cache,
                );
            }
            RenderCommand::PushOpacity { opacity: pushed } => {
                idx = replay_commands(
                    host,
                    layer,
                    commands,
                    idx + 1,
                    origin,
                    clip,
                    transform,
                    opacity * *pushed,
                    live_native_controls,
                    image_cache,
                );
            }
            RenderCommand::NativeControl { handle, rect, .. } => {
                let Some(mut view) = handle.downcast_ref::<AnyView>().cloned() else {
                    idx += 1;
                    continue;
                };
                let identity = view.identity();
                live_native_controls.insert(identity);
                let rect = elwindui_core::base::Rect {
                    x: origin.x + rect.x,
                    y: origin.y + rect.y,
                    width: rect.width,
                    height: rect.height,
                };
                let visible_rect = clip
                    .and_then(|clip| intersect_rect(rect, clip))
                    .unwrap_or(rect);
                if visible_rect.width <= 0.0 || visible_rect.height <= 0.0 {
                    idx += 1;
                    continue;
                }
                // This is deliberately a native island only around an actual native command;
                // ordinary painted content continues to replay to `layer` above.
                let (container, is_new) = {
                    let mut containers = host.ivars().native_containers.borrow_mut();
                    if let Some(container) = containers.get(&identity) {
                        (container.clone(), false)
                    } else {
                        let container = NSView::new(mtm());
                        containers.insert(identity, container.clone());
                        (container, true)
                    }
                };
                container.setFrame(NSRect::new(
                    objc2_foundation::NSPoint::new(visible_rect.x as f64, visible_rect.y as f64),
                    objc2_foundation::NSSize::new(
                        visible_rect.width as f64,
                        visible_rect.height as f64,
                    ),
                ));
                container.setClipsToBounds(true);
                let nsview = view.as_nsview();
                if is_new {
                    host.addSubview(&container);
                    container.addSubview(&nsview);
                }
                nsview.setTranslatesAutoresizingMaskIntoConstraints(true);
                view.arrange(elwindui_core::base::Rect {
                    x: rect.x - visible_rect.x,
                    y: rect.y - visible_rect.y,
                    width: rect.width,
                    height: rect.height,
                });
                idx += 1;
            }
            command => {
                if geometry_bounds(command, origin).is_none_or(|bounds| {
                    clip.is_none_or(|clip| intersect_rect(bounds, clip).is_some())
                }) {
                    replay_paint_command(
                        host,
                        layer,
                        command,
                        origin,
                        transform,
                        opacity,
                        image_cache,
                    );
                }
                idx += 1;
            }
        }
    }
    idx
}

/// The (already origin-adjusted, pre-transform) bounding rect a paint command occupies — used
/// only for the clip bounding-box overlap test in `replay_commands`, so a command with no
/// meaningful rect (nothing today) can return `None` to always pass.
fn geometry_bounds(
    command: &RenderCommand,
    origin: elwindui_core::base::Point,
) -> Option<elwindui_core::base::Rect> {
    let offset = |r: &elwindui_core::base::Rect| elwindui_core::base::Rect {
        x: origin.x + r.x,
        y: origin.y + r.y,
        width: r.width,
        height: r.height,
    };
    match command {
        RenderCommand::FillRect { rect, .. }
        | RenderCommand::StrokeRect { rect, .. }
        | RenderCommand::FillRoundedRect { rect, .. }
        | RenderCommand::StrokeRoundedRect { rect, .. }
        | RenderCommand::FillEllipse { rect, .. }
        | RenderCommand::StrokeEllipse { rect, .. }
        | RenderCommand::Text { rect, .. } => Some(offset(rect)),
        RenderCommand::DrawImage { dest, .. } => Some(offset(dest)),
        RenderCommand::DrawLine { .. }
        | RenderCommand::FillPath { .. }
        | RenderCommand::StrokePath { .. } => None,
        RenderCommand::NativeControl { .. }
        | RenderCommand::PushClip { .. }
        | RenderCommand::PopClip
        | RenderCommand::PushTransform { .. }
        | RenderCommand::PopTransform
        | RenderCommand::PushOpacity { .. }
        | RenderCommand::PopOpacity => None,
    }
}

/// Absolute (origin-adjusted) bounds of a `Clip` value, for `replay_commands`'s own clip-stack
/// intersection — `Clip::Path`'s bounds are used (a bounding-box approximation, consistent with
/// this whole replay pass never doing true per-pixel clipping).
fn clip_bounds(
    clip: &elwindui_core::painter::Clip,
    origin: elwindui_core::base::Point,
) -> Option<elwindui_core::base::Rect> {
    let offset = |r: elwindui_core::base::Rect| elwindui_core::base::Rect {
        x: origin.x + r.x,
        y: origin.y + r.y,
        width: r.width,
        height: r.height,
    };
    match clip {
        elwindui_core::painter::Clip::Rect(r) => Some(offset(*r)),
        elwindui_core::painter::Clip::RoundedRect { rect, .. } => Some(offset(*rect)),
        elwindui_core::painter::Clip::Path { path, .. } => Some(offset(path.bounds())),
    }
}

/// Builds the `CAShapeLayer` mask that gives `PushClip`/`PopClip` (`replay_commands`) real
/// per-pixel clipping — `world` is already `translation(origin) * transform` at the `PushClip`
/// site, keeping the mask path in the same canvas-absolute coordinate space the masked container
/// layer occupies (its `frame` is set to exactly overlay its parent, so no re-anchoring is needed).
fn clip_mask_layer(
    world: &elwindui_core::base::AffineTransform,
    clip: &elwindui_core::painter::Clip,
) -> Retained<CALayer> {
    let mask_layer = CAShapeLayer::new();
    let (path, rule) = match clip {
        elwindui_core::painter::Clip::Rect(rect) => (
            rounded_rect_cgpath(world, *rect, elwindui_core::base::CornerRadius::default()),
            elwindui_core::painter::FillRule::NonZero,
        ),
        elwindui_core::painter::Clip::RoundedRect { rect, radii } => {
            (rounded_rect_cgpath(world, *rect, *radii), elwindui_core::painter::FillRule::NonZero)
        }
        elwindui_core::painter::Clip::Path { path, rule } => (path_to_cgpath(world, path), *rule),
    };
    mask_layer.setPath(Some(&path));
    mask_layer.setFillRule(match rule {
        elwindui_core::painter::FillRule::NonZero => unsafe { kCAFillRuleNonZero },
        elwindui_core::painter::FillRule::EvenOdd => unsafe { kCAFillRuleEvenOdd },
    });
    mask_layer.setFillColor(Some(&color_to_cgcolor(elwindui_core::painter::Color::black())));
    Retained::into_super(mask_layer)
}

fn transform_point(
    t: &elwindui_core::base::AffineTransform,
    p: elwindui_core::base::Point,
) -> objc2_foundation::NSPoint {
    let p = t.transform_point(p);
    objc2_foundation::NSPoint::new(p.x as f64, p.y as f64)
}

/// Builds and appends the one `CALayer` (`CAShapeLayer`/`CAGradientLayer`+mask/`CATextLayer`/
/// image-`CALayer`) a single ordinary paint `RenderCommand` needs, applying `transform` to its
/// geometry directly (each corner point individually — see `replay_group`'s own doc comment for
/// why this is simpler/more robust here than a nested `CALayer.affineTransform`) and `opacity` to
/// the resulting layer.
fn replay_paint_command(
    _host: &TreeHostView,
    layer: &Retained<CALayer>,
    command: &RenderCommand,
    origin: elwindui_core::base::Point,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let world =
        elwindui_core::base::AffineTransform::translation(origin.x, origin.y).concat(&transform);
    let rounded_rect_path = |rect: &elwindui_core::base::Rect,
                             radii: elwindui_core::base::CornerRadius| {
        rounded_rect_cgpath(&world, *rect, radii)
    };
    match command {
        RenderCommand::FillRect { rect, brush } => {
            if !try_add_gradient_fill_layer(layer, brush, *rect, GradientMaskShape::RoundedRect(elwindui_core::base::CornerRadius::default()), &world, opacity) {
                let path = rounded_rect_path(rect, elwindui_core::base::CornerRadius::default());
                add_shape_layer(layer, &path, Some(brush), None, opacity, *rect);
            }
        }
        RenderCommand::StrokeRect {
            rect,
            brush,
            stroke,
        } => {
            let path = rounded_rect_path(rect, elwindui_core::base::CornerRadius::default());
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, *rect);
        }
        RenderCommand::FillRoundedRect { rect, radii, brush } => {
            if !try_add_gradient_fill_layer(layer, brush, *rect, GradientMaskShape::RoundedRect(*radii), &world, opacity) {
                let path = rounded_rect_path(rect, *radii);
                add_shape_layer(layer, &path, Some(brush), None, opacity, *rect);
            }
        }
        RenderCommand::StrokeRoundedRect {
            rect,
            radii,
            brush,
            stroke,
        } => {
            let path = rounded_rect_path(rect, *radii);
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, *rect);
        }
        RenderCommand::FillEllipse { rect, brush } => {
            if !try_add_gradient_fill_layer(layer, brush, *rect, GradientMaskShape::Ellipse, &world, opacity) {
                let path = ellipse_cgpath(&world, *rect);
                add_shape_layer(layer, &path, Some(brush), None, opacity, *rect);
            }
        }
        RenderCommand::StrokeEllipse {
            rect,
            brush,
            stroke,
        } => {
            let path = ellipse_cgpath(&world, *rect);
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, *rect);
        }
        RenderCommand::DrawLine {
            from,
            to,
            brush,
            stroke,
        } => {
            let path = CGMutablePath::new();
            unsafe {
                CGMutablePath::move_to_point(
                    Some(&path),
                    std::ptr::null(),
                    transform_point(&world, *from).x,
                    transform_point(&world, *from).y,
                );
            }
            unsafe {
                CGMutablePath::add_line_to_point(
                    Some(&path),
                    std::ptr::null(),
                    transform_point(&world, *to).x,
                    transform_point(&world, *to).y,
                );
            }
            let bounds = elwindui_core::base::Rect {
                x: from.x.min(to.x),
                y: from.y.min(to.y),
                width: (to.x - from.x).abs(),
                height: (to.y - from.y).abs(),
            };
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, bounds);
        }
        RenderCommand::FillPath { path, brush, rule } => {
            let cg_path = path_to_cgpath(&world, path);
            let shape_layer = CAShapeLayer::new();
            shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
            shape_layer.setPath(Some(&cg_path));
            shape_layer.setFillRule(match rule {
                elwindui_core::painter::FillRule::NonZero => unsafe { kCAFillRuleNonZero },
                elwindui_core::painter::FillRule::EvenOdd => unsafe { kCAFillRuleEvenOdd },
            });
            apply_fill(&shape_layer, Some(brush), path.bounds());
            shape_layer.setOpacity(opacity);
            let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
            layer.addSublayer(&shape_layer);
        }
        RenderCommand::StrokePath {
            path,
            brush,
            stroke,
        } => {
            let cg_path = path_to_cgpath(&world, path);
            let shape_layer = CAShapeLayer::new();
            shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
            shape_layer.setPath(Some(&cg_path));
            // `CAShapeLayer.fillColor` defaults to opaque black — must be explicitly nilled for a
            // stroke-only shape, same reasoning as `add_shape_layer`'s own doc comment.
            shape_layer.setFillColor(None);
            apply_stroke(&shape_layer, brush, stroke, path.bounds());
            shape_layer.setOpacity(opacity);
            let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
            layer.addSublayer(&shape_layer);
        }
        RenderCommand::DrawImage {
            image,
            dest,
            source,
            options,
        } => {
            // `options.repeat` (`TileMode::Tile`/`FlipX`/`FlipY`/`FlipXY`) has no direct
            // `CALayer.contents` equivalent — tiling would need multiple image sublayers stamped
            // across `dest` — and isn't attempted here; every `TileMode` draws as `None` (single
            // placement per `fitted_image_rect`) instead of silently ignoring the field outright.
            let Some(cg_image) = resolve_cgimage(image, image_cache) else {
                return;
            };
            let Some(cg_image) = crop_cgimage(&cg_image, *source) else {
                return;
            };
            let image_size = (
                CGImage::width(Some(&cg_image)) as f32,
                CGImage::height(Some(&cg_image)) as f32,
            );
            let placed =
                fitted_image_rect(*dest, image_size, options.fit, options.alignment_x, options.alignment_y);

            // A `dest`-sized, `masksToBounds` container keeps `Cover`/`None` overflow (the placed
            // image can be larger than `dest`) from bleeding into neighboring content — `placed` is
            // already expressed in this container's own local (dest-relative) coordinate space, the
            // same re-anchoring `try_add_gradient_fill_layer`'s mask path uses for the same reason.
            let container = CALayer::new();
            container.setName(Some(&NSString::from_str("elwindui-paint")));
            container.setMasksToBounds(true);
            container.setFrame(NSRect::new(
                transform_point(
                    &world,
                    elwindui_core::base::Point {
                        x: dest.x,
                        y: dest.y,
                    },
                ),
                objc2_foundation::NSSize::new(dest.width as f64, dest.height as f64),
            ));

            let image_layer = CALayer::new();
            image_layer.setFrame(NSRect::new(
                objc2_foundation::NSPoint::new(placed.x as f64, placed.y as f64),
                objc2_foundation::NSSize::new(placed.width as f64, placed.height as f64),
            ));
            unsafe {
                image_layer.setContents(Some(cg_image.as_ref() as &objc2::runtime::AnyObject))
            };
            let filter = match options.sampling {
                elwindui_core::painter::ImageSampling::Nearest => unsafe { kCAFilterNearest },
                elwindui_core::painter::ImageSampling::Linear
                | elwindui_core::painter::ImageSampling::Cubic => unsafe { kCAFilterLinear },
            };
            image_layer.setMagnificationFilter(filter);
            image_layer.setMinificationFilter(filter);
            container.addSublayer(&image_layer);
            container.setOpacity(opacity);
            layer.addSublayer(&container);
        }
        RenderCommand::Text {
            content,
            rect,
            color,
            alignment,
            ..
        } => {
            let text_layer = CATextLayer::new();
            text_layer.setName(Some(&NSString::from_str("elwindui-paint")));
            text_layer.setFrame(NSRect::new(
                transform_point(
                    &world,
                    elwindui_core::base::Point {
                        x: rect.x,
                        y: rect.y,
                    },
                ),
                objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
            ));
            text_layer.setFontSize(14.0);
            text_layer.setForegroundColor(Some(&color_to_cgcolor(
                color.unwrap_or(elwindui_core::painter::Color::black()),
            )));
            text_layer.setAlignmentMode(ca_alignment_mode(*alignment));
            unsafe {
                text_layer.setString(Some(&NSString::from_str(content)));
            }
            text_layer.setOpacity(opacity);
            let text_layer: Retained<CALayer> = Retained::into_super(text_layer);
            layer.addSublayer(&text_layer);
        }
        RenderCommand::NativeControl { .. }
        | RenderCommand::PushClip { .. }
        | RenderCommand::PopClip
        | RenderCommand::PushTransform { .. }
        | RenderCommand::PopTransform
        | RenderCommand::PushOpacity { .. }
        | RenderCommand::PopOpacity => {}
    }
}

/// Crops `cg_image` to `source` (image-pixel coordinates, top-left origin — `CGImage::
/// with_image_in_rect`'s own convention for a raster image), clamped to the image's own bounds
/// first (painter design doc §13.2: "source が画像外にはみ出した場合は交差領域にクリップする").
/// `None` means "draw the image unchanged"; a `source` that clamps to an empty intersection means
/// "draw nothing", surfaced the same way (`None`) since both are indistinguishable to the caller
/// once resolved — `RenderCommand::DrawImage`'s handler treats either as "skip this command".
fn crop_cgimage(
    cg_image: &CFRetained<CGImage>,
    source: Option<elwindui_core::base::Rect>,
) -> Option<CFRetained<CGImage>> {
    let Some(source) = source else {
        return Some(cg_image.clone());
    };
    let image_bounds = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: CGImage::width(Some(cg_image)) as f32,
        height: CGImage::height(Some(cg_image)) as f32,
    };
    let clamped = intersect_rect(source, image_bounds)?;
    CGImage::with_image_in_rect(
        Some(cg_image),
        objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(clamped.x as f64, clamped.y as f64),
            objc2_core_foundation::CGSize::new(clamped.width as f64, clamped.height as f64),
        ),
    )
}

/// The `Rect` — in `dest`-relative local coordinates, i.e. `(0, 0)` is `dest`'s own top-left, the
/// coordinate space `RenderCommand::DrawImage`'s `masksToBounds` container layer uses for its
/// image sublayer — that `image_size` (the already-cropped image's own pixel dimensions) should
/// actually be drawn at once `fit`/`alignment_x`/`alignment_y` are applied. `Fill` always returns
/// `dest` reduced to `(0, 0)`-origin (unchanged from this command's pre-`fit` behavior);
/// `Contain`/`Cover` scale `image_size` to fit inside/cover `dest` while preserving its aspect
/// ratio; `None` draws at intrinsic size. Any leftover space (`Contain`/`None`) or overflow
/// (`Cover`/`None`) is distributed per `alignment_x`/`alignment_y` — overflow is why the caller
/// needs its own `masksToBounds` container rather than just handing this rect straight to `dest`'s
/// own layer.
fn fitted_image_rect(
    dest: elwindui_core::base::Rect,
    image_size: (f32, f32),
    fit: elwindui_core::painter::ImageFit,
    alignment_x: elwindui_core::painter::AlignmentX,
    alignment_y: elwindui_core::painter::AlignmentY,
) -> elwindui_core::base::Rect {
    use elwindui_core::painter::{AlignmentX, AlignmentY, ImageFit};
    let (iw, ih) = image_size;
    let (w, h) = if iw <= 0.0 || ih <= 0.0 {
        (dest.width, dest.height)
    } else {
        match fit {
            ImageFit::Fill => (dest.width, dest.height),
            ImageFit::None => (iw, ih),
            ImageFit::Contain => {
                let scale = (dest.width / iw).min(dest.height / ih);
                (iw * scale, ih * scale)
            }
            ImageFit::Cover => {
                let scale = (dest.width / iw).max(dest.height / ih);
                (iw * scale, ih * scale)
            }
        }
    };
    let x = match alignment_x {
        AlignmentX::Left => 0.0,
        AlignmentX::Center => (dest.width - w) / 2.0,
        AlignmentX::Right => dest.width - w,
    };
    let y = match alignment_y {
        AlignmentY::Top => 0.0,
        AlignmentY::Center => (dest.height - h) / 2.0,
        AlignmentY::Bottom => dest.height - h,
    };
    elwindui_core::base::Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn add_shape_layer(
    layer: &Retained<CALayer>,
    path: &CFRetained<CGMutablePath>,
    fill: Option<&elwindui_core::painter::Brush>,
    stroke: Option<(
        &elwindui_core::painter::Brush,
        &elwindui_core::painter::StrokeStyle,
    )>,
    opacity: f32,
    bounds: elwindui_core::base::Rect,
) {
    let shape_layer = CAShapeLayer::new();
    shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
    shape_layer.setPath(Some(path));
    // `CAShapeLayer.fillColor` defaults to opaque black, not nil — `apply_fill`'s own `None` arm
    // (`setFillColor(None)`) must always run for a stroke-only shape, or the shape silently paints
    // as if solid-black-filled underneath its stroke.
    apply_fill(&shape_layer, fill, bounds);
    if let Some((brush, style)) = stroke {
        apply_stroke(&shape_layer, brush, style, bounds);
    }
    shape_layer.setOpacity(opacity);
    let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
    layer.addSublayer(&shape_layer);
}

/// Which built-in shape a gradient's clip mask should take — mirrors `replay_paint_command`'s own
/// `FillRect`/`FillRoundedRect`/`FillEllipse` distinction, since a gradient fill needs a *local*
/// (mask-space, not canvas-absolute) path rebuilt for the mask layer (see
/// `try_add_gradient_fill_layer`'s own doc comment).
enum GradientMaskShape {
    RoundedRect(elwindui_core::base::CornerRadius),
    Ellipse,
}

fn is_pure_translation(t: &elwindui_core::base::AffineTransform) -> bool {
    (t.m11 - 1.0).abs() < 1e-4
        && t.m12.abs() < 1e-4
        && t.m21.abs() < 1e-4
        && (t.m22 - 1.0).abs() < 1e-4
}

/// Realizes a `LinearGradient`/`RadialGradient` fill as a real `CAGradientLayer` (rather than
/// `apply_fill`'s flat first-stop-color fallback), masked to `shape`'s outline. Returns `false`
/// (does nothing) for anything else — a solid brush, an unsupported `Image` brush, or a gradient
/// under a non-translation `world` (rotated/scaled group) — so the caller falls back to
/// `add_shape_layer`'s existing solid-color path in those cases.
///
/// The mask needs its own path expressed in the *gradient layer's local* coordinate space (origin
/// at the gradient layer's own top-left, not the canvas-absolute space `path_to_cgpath`/
/// `rounded_rect_cgpath` normally build in) — `CALayer.mask` interprets its mask layer exactly
/// like an ordinary sublayer of the layer being masked. `bounds` (already the pre-`world` local
/// rect every `replay_paint_command` call site already has on hand) rebuilt through
/// `AffineTransform::translation(-bounds.x, -bounds.y)` produces exactly that.
///
/// `GradientStop`'s own `offset` aside, `LinearGradientBrush`/`RadialGradientBrush::spread`
/// (`GradientSpreadMethod::{Pad,Reflect,Repeat}`) is never read here: `CAGradientLayer` has no
/// native notion of a spread method beyond clamping at the first/last stop (`Pad`'s own behavior),
/// so every brush renders as `Pad` regardless of what `spread` is actually set to — `Reflect`/
/// `Repeat` would need tiling multiple `CAGradientLayer`s across the fill region, not attempted
/// here (painter design doc §9.4 accepts a documented-but-unimplemented gap in the same spirit).
fn try_add_gradient_fill_layer(
    layer: &Retained<CALayer>,
    brush: &elwindui_core::painter::Brush,
    bounds: elwindui_core::base::Rect,
    mask_shape: GradientMaskShape,
    world: &elwindui_core::base::AffineTransform,
    opacity: f32,
) -> bool {
    use elwindui_core::painter::Brush;
    if !is_pure_translation(world) {
        return false;
    }
    let absolute_origin = world.transform_point(elwindui_core::base::Point { x: bounds.x, y: bounds.y });
    let gradient_layer = CAGradientLayer::new();
    gradient_layer.setName(Some(&NSString::from_str("elwindui-paint")));
    let ca_layer: &CALayer = &gradient_layer;
    ca_layer.setFrame(NSRect::new(
        objc2_foundation::NSPoint::new(absolute_origin.x as f64, absolute_origin.y as f64),
        objc2_foundation::NSSize::new(bounds.width as f64, bounds.height as f64),
    ));
    ca_layer.setOpacity(opacity);

    let stops: &[elwindui_core::painter::GradientStop] = match brush {
        Brush::LinearGradient(g) => {
            unsafe { gradient_layer.setType(kCAGradientLayerAxial) };
            gradient_layer.setStartPoint(gradient_unit_point(g.start, g.mapping, bounds));
            gradient_layer.setEndPoint(gradient_unit_point(g.end, g.mapping, bounds));
            &g.stops
        }
        Brush::RadialGradient(g) => {
            unsafe { gradient_layer.setType(kCAGradientLayerRadial) };
            let center = gradient_unit_point(g.center, g.mapping, bounds);
            gradient_layer.setStartPoint(center);
            let (rx, ry) = match g.mapping {
                elwindui_core::painter::BrushMappingMode::RelativeToBounds => (g.radius_x, g.radius_y),
                elwindui_core::painter::BrushMappingMode::Absolute => (
                    g.radius_x / bounds.width.max(1e-6),
                    g.radius_y / bounds.height.max(1e-6),
                ),
            };
            // `CAGradientLayer`'s radial `endPoint` encodes *both* radii at once, as the vector
            // from `startPoint` (the center) to this point — an endpoint level with the center on
            // one axis (e.g. `(center.x + rx, center.y)`) collapses that axis's radius to zero
            // instead of leaving it at `rx`, making the gradient degenerate/invisible.
            gradient_layer.setEndPoint(objc2_core_foundation::CGPoint::new(
                center.x + rx as f64,
                center.y + ry as f64,
            ));
            &g.stops
        }
        _ => return false,
    };
    if stops.is_empty() {
        return false;
    }

    let colors: Vec<CFRetained<CGColor>> = stops.iter().map(|s| color_to_cgcolor(s.color)).collect();
    let color_refs: Vec<&objc2::runtime::AnyObject> = colors
        .iter()
        .map(|c| c.as_ref() as &objc2_core_foundation::CFType)
        .map(|c| c.as_ref())
        .collect();
    let colors_array = NSArray::from_slice(&color_refs);
    unsafe { gradient_layer.setColors(Some(&colors_array)) };

    let locations: Vec<Retained<NSNumber>> = stops.iter().map(|s| NSNumber::new_f64(s.offset as f64)).collect();
    let location_refs: Vec<&NSNumber> = locations.iter().map(|n| n.as_ref()).collect();
    gradient_layer.setLocations(Some(&NSArray::from_slice(&location_refs)));

    // `local_bounds` is already `bounds` re-anchored at (0, 0) — the identity transform (not
    // another `translation(-bounds.x, -bounds.y)`) is what belongs alongside it; applying both
    // shifts the mask a second time; for a `bounds` far from the canvas origin (any cell but the
    // very first) that moves the mask entirely outside `gradient_layer`'s own local bounds,
    // leaving nothing visible at all (an *empty* intersection, not just a misaligned one).
    let mask_layer = CAShapeLayer::new();
    let identity = elwindui_core::base::AffineTransform::identity();
    let local_bounds = elwindui_core::base::Rect { x: 0.0, y: 0.0, ..bounds };
    let mask_path = match mask_shape {
        GradientMaskShape::RoundedRect(radii) => rounded_rect_cgpath(&identity, local_bounds, radii),
        GradientMaskShape::Ellipse => ellipse_cgpath(&identity, local_bounds),
    };
    mask_layer.setPath(Some(&mask_path));
    mask_layer.setFillColor(Some(&color_to_cgcolor(elwindui_core::painter::Color::black())));
    let mask_layer: Retained<CALayer> = Retained::into_super(mask_layer);
    unsafe { ca_layer.setMask(Some(&mask_layer)) };

    let gradient_layer: Retained<CALayer> = Retained::into_super(gradient_layer);
    layer.addSublayer(&gradient_layer);
    true
}

fn gradient_unit_point(
    p: elwindui_core::base::Point,
    mapping: elwindui_core::painter::BrushMappingMode,
    bounds: elwindui_core::base::Rect,
) -> objc2_core_foundation::CGPoint {
    match mapping {
        elwindui_core::painter::BrushMappingMode::RelativeToBounds => {
            objc2_core_foundation::CGPoint::new(p.x as f64, p.y as f64)
        }
        elwindui_core::painter::BrushMappingMode::Absolute => objc2_core_foundation::CGPoint::new(
            ((p.x - bounds.x) / bounds.width.max(1e-6)) as f64,
            ((p.y - bounds.y) / bounds.height.max(1e-6)) as f64,
        ),
    }
}

/// Applies `brush` as `shape_layer`'s fill. A gradient brush is realized as a masked
/// `CAGradientLayer` sibling rather than `CAShapeLayer.fillColor` (which only accepts a solid
/// color) — `shape_layer` itself is left with no fill color (transparent interior) and the
/// gradient layer, masked by a copy of the same shape, is added alongside it in `shape_layer`'s
/// own superlayer once `shape_layer` itself has been added (see call sites).
fn apply_fill(
    shape_layer: &CAShapeLayer,
    brush: Option<&elwindui_core::painter::Brush>,
    bounds: elwindui_core::base::Rect,
) {
    match brush {
        None => shape_layer.setFillColor(None),
        Some(elwindui_core::painter::Brush::Solid(color)) => {
            shape_layer.setFillColor(Some(&color_to_cgcolor(*color)));
        }
        Some(
            brush @ (elwindui_core::painter::Brush::LinearGradient(_)
            | elwindui_core::painter::Brush::RadialGradient(_)),
        ) => {
            // No direct sibling-insertion point here (that needs the *superlayer*, only known
            // once `shape_layer` itself is added) — approximate with the gradient's first stop as
            // a flat fill instead. A `CAGradientLayer`+mask upgrade is real future work (painter
            // design doc §6), not a silent capability gap: this is the one brush combination this
            // backend doesn't fully realize yet, and it degrades to *a* reasonable solid color
            // rather than nothing.
            if let Some(color) = first_gradient_stop_color(brush) {
                shape_layer.setFillColor(Some(&color_to_cgcolor(color)));
            }
        }
        Some(elwindui_core::painter::Brush::Image(_)) => {
            // Same limitation as the gradient case above — image-brush fills aren't realized yet.
        }
    }
    let _ = bounds;
}

fn apply_stroke(
    shape_layer: &CAShapeLayer,
    brush: &elwindui_core::painter::Brush,
    style: &elwindui_core::painter::StrokeStyle,
    _bounds: elwindui_core::base::Rect,
) {
    let color = match brush {
        elwindui_core::painter::Brush::Solid(color) => *color,
        other => first_gradient_stop_color(other).unwrap_or(elwindui_core::painter::Color::black()),
    };
    shape_layer.setStrokeColor(Some(&color_to_cgcolor(color)));
    shape_layer.setLineWidth(style.width as f64);
    shape_layer.setMiterLimit(style.miter_limit as f64);
    shape_layer.setLineCap(ca_line_cap(style.end_cap));
    shape_layer.setLineJoin(ca_line_join(style.line_join));
    if !style.dash_pattern.is_empty() {
        let numbers: Vec<Retained<NSNumber>> = style
            .dash_pattern
            .iter()
            .map(|&d| NSNumber::new_f64(d as f64))
            .collect();
        let refs: Vec<&NSNumber> = numbers.iter().map(|n| n.as_ref()).collect();
        let array = NSArray::from_slice(&refs);
        shape_layer.setLineDashPattern(Some(&array));
        shape_layer.setLineDashPhase(style.dash_offset as f64);
    } else {
        shape_layer.setLineDashPattern(None);
    }
}

fn first_gradient_stop_color(
    brush: &elwindui_core::painter::Brush,
) -> Option<elwindui_core::painter::Color> {
    match brush {
        elwindui_core::painter::Brush::LinearGradient(g) => g.stops.first().map(|s| s.color),
        elwindui_core::painter::Brush::RadialGradient(g) => g.stops.first().map(|s| s.color),
        _ => None,
    }
}

fn ca_line_cap(cap: elwindui_core::painter::LineCap) -> &'static CAShapeLayerLineCap {
    unsafe {
        match cap {
            elwindui_core::painter::LineCap::Butt => kCALineCapButt,
            elwindui_core::painter::LineCap::Round => kCALineCapRound,
            elwindui_core::painter::LineCap::Square => kCALineCapSquare,
        }
    }
}

fn ca_line_join(join: elwindui_core::painter::LineJoin) -> &'static CAShapeLayerLineJoin {
    unsafe {
        match join {
            elwindui_core::painter::LineJoin::Miter => kCALineJoinMiter,
            elwindui_core::painter::LineJoin::Round => kCALineJoinRound,
            elwindui_core::painter::LineJoin::Bevel => kCALineJoinBevel,
        }
    }
}

fn color_to_cgcolor(
    color: elwindui_core::painter::Color,
) -> objc2_core_foundation::CFRetained<CGColor> {
    CGColor::new_generic_rgb(
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
        color.a as f64 / 255.0,
    )
}

/// Builds via the general `PathBuilder`/`path_to_cgpath` route uniformly (rather than special-
/// casing `CGPath::with_rounded_rect` for the common uniform-radius/identity-transform case) —
/// `CGPath::with_rounded_rect` returns an *immutable* `CGPath`, whereas every other path this
/// backend builds is a `CGMutablePath` (so `transform`/dash/gradient-mask code can treat all of
/// them uniformly); bridging between the two isn't worth it for what's a one-time-per-repaint
/// path construction, not a hot loop.
fn rounded_rect_cgpath(
    world: &elwindui_core::base::AffineTransform,
    rect: elwindui_core::base::Rect,
    radii: elwindui_core::base::CornerRadius,
) -> CFRetained<CGMutablePath> {
    let mut builder = elwindui_core::painter::PathBuilder::new();
    builder.add_rounded_rect(rect, radii);
    path_to_cgpath(
        world,
        &builder.build().expect("rounded rect path is never empty"),
    )
}

fn ellipse_cgpath(
    world: &elwindui_core::base::AffineTransform,
    rect: elwindui_core::base::Rect,
) -> CFRetained<CGMutablePath> {
    let mut builder = elwindui_core::painter::PathBuilder::new();
    builder.add_ellipse(rect);
    path_to_cgpath(
        world,
        &builder.build().expect("ellipse path is never empty"),
    )
}

/// Converts one of our own `Path`s into a `CGMutablePath`, applying `world` to every point —
/// arcs/quads are already normalized to cubics by `Path`'s own internal representation, so this
/// only ever has to emit `moveTo`/`lineTo`/`curveTo`/`closePath`.
fn path_to_cgpath(
    world: &elwindui_core::base::AffineTransform,
    path: &elwindui_core::painter::Path,
) -> CFRetained<CGMutablePath> {
    let cg_path = CGMutablePath::new();
    for command in path.commands() {
        match *command {
            elwindui_core::painter::PathCommand::MoveTo(p) => {
                let p = transform_point(world, p);
                unsafe {
                    CGMutablePath::move_to_point(Some(&cg_path), std::ptr::null(), p.x, p.y);
                }
            }
            elwindui_core::painter::PathCommand::LineTo(p) => {
                let p = transform_point(world, p);
                unsafe {
                    CGMutablePath::add_line_to_point(Some(&cg_path), std::ptr::null(), p.x, p.y);
                }
            }
            elwindui_core::painter::PathCommand::QuadTo { control, to } => {
                let c = transform_point(world, control);
                let p = transform_point(world, to);
                unsafe {
                    CGMutablePath::add_quad_curve_to_point(
                        Some(&cg_path),
                        std::ptr::null(),
                        c.x,
                        c.y,
                        p.x,
                        p.y,
                    );
                }
            }
            elwindui_core::painter::PathCommand::CubicTo {
                control1,
                control2,
                to,
            } => {
                let c1 = transform_point(world, control1);
                let c2 = transform_point(world, control2);
                let p = transform_point(world, to);
                unsafe {
                    CGMutablePath::add_curve_to_point(
                        Some(&cg_path),
                        std::ptr::null(),
                        c1.x,
                        c1.y,
                        c2.x,
                        c2.y,
                        p.x,
                        p.y,
                    );
                }
            }
            elwindui_core::painter::PathCommand::ArcTo(_) => {
                // `Path` normalizes every `ArcTo` to cubics internally for bounds/flattening
                // purposes, but `PathCommand::ArcTo` itself (this raw command list) is the
                // author's original, un-normalized form — reachable here directly. Converting it
                // would duplicate `path.rs`'s own (private) `arc_to_cubics`; skipping it is a
                // known gap (an arc segment drawn via `PathBuilder::arc_to`/`arc_center` won't
                // render on this backend yet) rather than a silent geometry corruption.
            }
            elwindui_core::painter::PathCommand::Close => {
                CGMutablePath::close_subpath(Some(&cg_path));
            }
        }
    }
    cg_path
}

/// Resolves an `Image` to a `CGImage`, decoding at most once per distinct `Image` (`image_cache`,
/// keyed by the `Image`'s own `Arc` pointer identity — cheap and stable since `Image` is
/// `Arc`-backed and the same logical image reuses the same `Arc` across relayouts unless the
/// application constructs a fresh one).
fn resolve_cgimage(
    image: &elwindui_core::painter::Image,
    cache: &mut HashMap<usize, CFRetained<CGImage>>,
) -> Option<CFRetained<CGImage>> {
    let key = image as *const _ as usize;
    if let Some(cached) = cache.get(&key) {
        return Some(cached.clone());
    }
    let decoded = decode_cgimage(image)?;
    cache.insert(key, decoded.clone());
    Some(decoded)
}

/// Releases the boxed pixel buffer `with_data` was given ownership of — `CGDataProvider::with_data`
/// takes raw `(info, data, size)` with no built-in ownership story of its own, so this callback is
/// what actually frees it once Core Graphics is done (as opposed to going through `NSData`/`CFData`
/// bridging, which would need a toll-free-bridging guarantee this crate version doesn't expose a
/// convenient safe path for).
unsafe extern "C-unwind" fn release_boxed_pixels(
    _info: *mut std::ffi::c_void,
    data: std::ptr::NonNull<std::ffi::c_void>,
    size: usize,
) {
    unsafe {
        drop(Vec::from_raw_parts(data.as_ptr() as *mut u8, size, size));
    }
}

fn decode_cgimage(image: &elwindui_core::painter::Image) -> Option<CFRetained<CGImage>> {
    match image.data() {
        elwindui_core::painter::ImageData::Rgba8 {
            width,
            height,
            stride,
            pixels,
            alpha,
        } => {
            let mut owned = pixels.to_vec().into_boxed_slice();
            let len = owned.len();
            let ptr = owned.as_mut_ptr();
            std::mem::forget(owned);
            let provider = unsafe {
                CGDataProvider::with_data(
                    std::ptr::null_mut(),
                    ptr as *const _,
                    len,
                    Some(release_boxed_pixels),
                )
            }?;
            let color_space = CGColorSpace::new_device_rgb()?;
            let alpha_info = match alpha {
                elwindui_core::painter::AlphaMode::Opaque => {
                    objc2_core_graphics::CGImageAlphaInfo::NoneSkipLast
                }
                _ => objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast,
            };
            unsafe {
                CGImage::new(
                    *width as usize,
                    *height as usize,
                    8,
                    32,
                    *stride as usize,
                    Some(&color_space),
                    objc2_core_graphics::CGBitmapInfo(alpha_info.0 as _),
                    Some(&provider),
                    std::ptr::null(),
                    false,
                    objc2_core_graphics::CGColorRenderingIntent::RenderingIntentDefault,
                )
            }
        }
        elwindui_core::painter::ImageData::Encoded { bytes, .. } => {
            let data = objc2_foundation::NSData::with_bytes(bytes);
            let ns_image = NSImage::initWithData(NSImage::alloc(), &data)?;
            let mut rect = NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), ns_image.size());
            let cg_image = unsafe {
                ns_image.CGImageForProposedRect_context_hints(&mut rect as *mut NSRect, None, None)
            }?;
            // `NSImage.CGImageForProposedRect:context:hints:` returns an Objective-C-managed
            // `Retained<CGImage>` even though every other `CGImage` this backend produces is a
            // Core-Foundation-managed `CFRetained<CGImage>` — `CGImageRef` is toll-free bridged
            // with `id`, so the two retain/release mechanisms are the same underlying operation,
            // and handing the raw pointer straight from one wrapper to the other is sound.
            let ptr = std::ptr::NonNull::new(Retained::into_raw(cg_image))
                .expect("Retained is never null");
            Some(unsafe { CFRetained::from_raw(ptr) })
        }
        elwindui_core::painter::ImageData::Backend(handle) => {
            handle.0.downcast_ref::<CFRetained<CGImage>>().cloned()
        }
    }
}

/// Raw `NSWindow` + content host — composed by `native_ui::Window`.
#[derive(Clone)]
pub(crate) struct InnerWindow {
    ns: Retained<NSWindow>,
    content_host: Retained<TreeHostView>,
}

impl InnerWindow {
    pub(crate) fn new() -> Self {
        let mtm = mtm();
        let content_rect = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(480.0, 360.0),
        );
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let ns = unsafe {
            let alloc = mtm.alloc::<NSWindow>();
            NSWindow::initWithContentRect_styleMask_backing_defer(
                alloc,
                content_rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        let content_host = TreeHostView::new();
        // `Window` property setters can resize the NSWindow after this content view has been
        // installed (the notepad starts at 640×480 although InnerWindow's construction rect is
        // 480×360). Keep the host synchronized with the client area just like per-tab hosts do.
        content_host.setTranslatesAutoresizingMaskIntoConstraints(true);
        content_host.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        ns.setContentView(Some(&content_host));
        Self { ns, content_host }
    }

    pub(crate) fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.content_host.set_tree(content);
    }

    fn sync_content_host_frame(&self) {
        let client = self.ns.contentRectForFrameRect(self.ns.frame());
        self.content_host.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            client.size,
        ));
        self.content_host.setNeedsLayout(true);
    }

    pub(crate) fn set_title(&self, title: &str) {
        self.ns.setTitle(&NSString::from_str(title));
    }

    /// Sets `NSApplication.mainMenu` (macOS has one global top menu bar, not a per-window one).
    pub(crate) fn set_menu_bar(&self, menu_bar: &InnerMenuBar) {
        NSApplication::sharedApplication(mtm()).setMainMenu(Some(&menu_bar.ns));
    }

    pub(crate) fn show(&self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        self.ns.makeKeyAndOrderFront(None);
        app.activate();
    }

    fn screen_height(&self) -> f64 {
        self.ns
            .screen()
            .or_else(|| NSScreen::mainScreen(mtm()))
            .map(|screen| screen.frame().size.height)
            .unwrap_or(0.0)
    }

    pub(crate) fn left(&self) -> f32 {
        self.ns.frame().origin.x as f32
    }

    pub(crate) fn set_left(&self, left: f32) {
        let mut frame = self.ns.frame();
        frame.origin.x = left as f64;
        self.ns.setFrame_display(frame, true);
    }

    pub(crate) fn top(&self) -> f32 {
        let frame = self.ns.frame();
        (self.screen_height() - (frame.origin.y + frame.size.height)) as f32
    }

    pub(crate) fn set_top(&self, top: f32) {
        let screen_height = self.screen_height();
        let mut frame = self.ns.frame();
        frame.origin.y = screen_height - top as f64 - frame.size.height;
        self.ns.setFrame_display(frame, true);
    }

    pub(crate) fn width(&self) -> f32 {
        self.ns.frame().size.width as f32
    }

    pub(crate) fn set_width(&self, width: f32) {
        let mut frame = self.ns.frame();
        frame.size.width = width as f64;
        self.ns.setFrame_display(frame, true);
        self.sync_content_host_frame();
    }

    pub(crate) fn height(&self) -> f32 {
        self.ns.frame().size.height as f32
    }

    pub(crate) fn set_height(&self, height: f32) {
        let mut frame = self.ns.frame();
        let old_height = frame.size.height;
        frame.size.height = height as f64;
        frame.origin.y -= height as f64 - old_height;
        self.ns.setFrame_display(frame, true);
        self.sync_content_host_frame();
    }
}

/// Raw `NSTextView` + change-notification delegate — composed by `native_ui::TextArea`.
pub(crate) struct InnerTextArea {
    handle: AnyView,
    text_view: Retained<NSTextView>,
    delegate_storage: Rc<RefCell<Option<Retained<TextViewDelegate>>>>,
}

impl InnerTextArea {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let scroll = NSTextView::scrollableTextView(m);
        let text_view = scroll
            .documentView()
            .expect("scrollableTextView always has a document view")
            .downcast::<NSTextView>()
            .expect("scrollableTextView's document view is an NSTextView");
        let handle = AnyView::from(scroll);
        Self {
            handle,
            text_view,
            delegate_storage: Rc::new(RefCell::new(None)),
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    /// `NSTextView.setString:` resets the caret/selection. In the normal two-way input path the
    /// native buffer has already changed before its delegate calls the model setter, so identical
    /// model→widget updates must be a no-op.
    pub(crate) fn set_text(&self, text: &str) {
        if self.text_view.string().to_string() == text {
            return;
        }
        self.text_view.setString(&NSString::from_str(text));
    }

    /// `NSTextView.delegate` is an unretained (weak) reference, so the delegate this creates is
    /// only kept alive by `self.delegate_storage`.
    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        let m = mtm();
        let ivars = TextDelegateIvars {
            text_view: self.text_view.clone(),
            callback,
        };
        let delegate = TextViewDelegate::new(m, ivars);
        let protocol_obj: &objc2::runtime::ProtocolObject<dyn NSTextViewDelegate> =
            objc2::runtime::ProtocolObject::from_ref(&*delegate);
        self.text_view.setDelegate(Some(protocol_obj));
        *self.delegate_storage.borrow_mut() = Some(delegate);
    }
}

struct TextDelegateIvars {
    text_view: Retained<NSTextView>,
    callback: Box<dyn Fn(String)>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = TextDelegateIvars]
    struct TextViewDelegate;

    unsafe impl NSObjectProtocol for TextViewDelegate {}

    unsafe impl NSTextDelegate for TextViewDelegate {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, _notification: &NSNotification) {
            let s = self.ivars().text_view.string();
            (self.ivars().callback)(s.to_string());
        }
    }

    unsafe impl NSTextViewDelegate for TextViewDelegate {}
);

impl TextViewDelegate {
    fn new(mtm: MainThreadMarker, ivars: TextDelegateIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

/// Raw `NSButton` + click target — composed by `native_ui::Button` (and used directly, not through
/// `native_ui::Button`, by `TabChipImpl`/`TabStripImpl` below for their own internal chip/strip
/// buttons — see those types' own doc comments).
pub(crate) struct InnerButton {
    pub(crate) handle: AnyView,
    ns: Retained<NSButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

impl InnerButton {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(""), None, None, m)
        };
        let handle = AnyView::from(ns.clone());
        Self {
            handle,
            ns,
            target_storage: Rc::new(RefCell::new(None)),
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    pub(crate) fn set_on_click(&self, callback: Box<dyn Fn()>) {
        let target = ButtonTarget::new(ButtonTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }

    /// Used by `TabChipImpl` to rename a tab's title button when its document's file name changes.
    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    /// AppKit-only helper (no `elwindui_core::ui::Button` trait member — WinUI3's real `TabView`
    /// highlights its selected tab for free, no borderless-button trick needed there): used by
    /// `create_tab_chip` so `TabChipImpl::set_selected`'s translucent background tint shows through
    /// instead of being hidden behind the button's own opaque default bezel.
    pub(crate) fn set_bordered(&self, bordered: bool) {
        self.ns.setBordered(bordered);
    }
}

struct ButtonTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = ButtonTargetIvars]
    struct ButtonTarget;

    unsafe impl NSObjectProtocol for ButtonTarget {}

    impl ButtonTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl ButtonTarget {
    fn new(ivars: ButtonTargetIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y. A single tab's header: a title button (click to
/// select) plus a small close button, packed into one row so `TabStripImpl` can insert/remove it as
/// one unit. Purely an internal composition helper (never a real `.elwind`-declared element), so
/// its two buttons are plain `InnerButton`s, not `native_ui::Button` — no use-site margin/alignment
/// ever applies to them.
pub(crate) struct TabChipImpl {
    ns: Retained<NSStackView>,
    pub(crate) title_button: InnerButton,
    pub(crate) close_button: InnerButton,
}

fn create_tab_chip(title: &str) -> TabChipImpl {
    let title_button = InnerButton::new();
    title_button.set_text(title);
    // Borderless: an `NSButton`'s default bezel is opaque and would otherwise cover almost the
    // entire chip row, hiding `set_selected`'s translucent background tint underneath it.
    title_button.set_bordered(false);
    let close_button = InnerButton::new();
    close_button.set_text("×");
    close_button.set_bordered(false);
    let ns = new_stack(
        vec![title_button.handle.clone(), close_button.handle.clone()],
        NSUserInterfaceLayoutOrientation::Horizontal,
    );
    TabChipImpl {
        ns,
        title_button,
        close_button,
    }
}

impl TabChipImpl {
    pub(crate) fn set_title(&self, title: &str) {
        self.title_button.set_text(title);
    }

    /// Highlights this chip's own row with a translucent background tint when it's the selected
    /// tab. AppKit has no native "selected tab" concept to lean on here (unlike WinUI3's real
    /// `Controls::TabView`, whose `SelectedIndex` gets OS-drawn highlighting for free) — this
    /// backend hand-rolls its tab strip out of a plain `NSStackView`, so the highlight is drawn the
    /// same way `Rectangle`'s own `fill` is: a layer-backed background color, applied to `ns` (the
    /// chip's whole row) rather than just `title_button` so it isn't hidden behind that button's
    /// own bezel rendering.
    pub(crate) fn set_selected(&self, selected: bool) {
        self.ns.setWantsLayer(true);
        let layer = self.ns.layer().expect("wantsLayer(true) implies a layer");
        if selected {
            layer.setBackgroundColor(Some(&parse_color("#7f7f7f40")));
        } else {
            layer.setBackgroundColor(None);
        }
    }
}

/// The row of `TabChipImpl`s plus a trailing "+" button. `InnerTabView` owns one of these and the
/// content area below it; kept as a separate type since 付録Y's backend table describes it as its
/// own piece (a custom `NSStackView`-based strip, not `NSTabViewController`).
pub(crate) struct TabStripImpl {
    ns: Retained<NSStackView>,
    pub(crate) new_tab_button: InnerButton,
}

fn create_tab_strip() -> TabStripImpl {
    let new_tab_button = InnerButton::new();
    new_tab_button.set_text("+");
    let ns = new_stack(
        vec![new_tab_button.handle.clone()],
        NSUserInterfaceLayoutOrientation::Horizontal,
    );
    TabStripImpl { ns, new_tab_button }
}

impl TabStripImpl {
    /// Inserts a chip before the "+" button, at arranged-subview position `index`.
    fn insert_tab(&self, index: usize, title: &str) -> TabChipImpl {
        let chip = create_tab_chip(title);
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.insertArrangedSubview_atIndex(&view, index as isize);
        chip
    }

    fn remove_tab(&self, chip: &TabChipImpl) {
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.removeArrangedSubview(&view);
        view.removeFromSuperview();
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y. Vertical stack of `[TabStripImpl, content_container]`
/// — composed by `native_ui::TabView`, which owns the mapping from its `children` collection's
/// `TabViewItem`s to `TabChipImpl`s + content hosts. This type only holds the widget areas — it has
/// no notion of "the list of tabs" on its own.
///
/// Each tab gets its own persistent `TreeHostView` (created once, in `insert_tab`), added as an
/// overlaid subview of `content_container` and shown/hidden via `set_tab_content_visible` rather
/// than destroyed and rebuilt — a single shared pane would have no way to restore a previously-
/// shown-then-hidden tab's content after switching away from it.
pub(crate) struct InnerTabView {
    handle: AnyView,
    pub(crate) strip: TabStripImpl,
    content_container: Retained<NSView>,
}

impl InnerTabView {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let strip = create_tab_strip();
        let content_container = NSView::initWithFrame(NSView::alloc(m), NSRect::default());
        let strip_view: Retained<NSView> = Retained::into_super(strip.ns.clone());
        let root = NSStackView::stackViewWithViews(
            &objc2_foundation::NSArray::from_retained_slice(&[
                strip_view,
                content_container.clone(),
            ]),
            m,
        );
        root.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        // `NSStackView`'s default `distribution` (`GravityAreas`) leaves each arranged subview at
        // its own intrinsic size unless hugging priorities say otherwise — `.Fill` makes the stack
        // actually consume its *entire* stacking-axis extent, matching the expected "chips row at
        // natural height, content area fills the rest" shape. `content_container`'s own vertical
        // hugging priority is dropped to (near-)zero so it — not the also-low-priority-by-default
        // `strip` — is the one that absorbs whatever space `Fill` distributes.
        content_container.setContentHuggingPriority_forOrientation(
            1.0,
            objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
        );
        root.setDistribution(objc2_app_kit::NSStackViewDistribution::Fill);
        let handle = AnyView::from(root);
        Self {
            handle,
            strip,
            content_container,
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.strip.new_tab_button.set_on_click(callback);
    }

    /// Inserts a new tab chip at `index` (wiring `on_select`/`on_close` to the given callbacks)
    /// plus a fresh, persistent content host — added to `content_container`, initially hidden.
    pub(crate) fn insert_tab(
        &self,
        index: usize,
        title: &str,
        on_select: Box<dyn Fn()>,
        on_close: Box<dyn Fn()>,
    ) -> (TabChipImpl, Retained<TreeHostView>) {
        let chip = self.strip.insert_tab(index, title);
        chip.title_button.set_on_click(on_select);
        chip.close_button.set_on_click(on_close);

        let host = TreeHostView::new();
        // Classic pre-Auto-Layout "fill the parent" technique instead of `NSLayoutConstraint`s:
        // `translatesAutoresizingMaskIntoConstraints(true)` (this container has no Auto Layout
        // constraints of its own, so this is the default anyway, made explicit) plus a
        // `.width | .height` autoresizing mask makes AppKit stretch `host` to match
        // `content_container`'s bounds on every resize, with no custom `NSView` subclass or
        // constraint bookkeeping needed here.
        host.setTranslatesAutoresizingMaskIntoConstraints(true);
        host.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        host.setFrame(self.content_container.bounds());
        host.setHidden(true);
        self.content_container.addSubview(&host);
        (chip, host)
    }

    /// Removes a tab's chip and its persistent content host together.
    pub(crate) fn remove_tab(&self, chip: &TabChipImpl, host: &TreeHostView) {
        self.strip.remove_tab(chip);
        host.removeFromSuperview();
    }

    /// Shows or hides a tab's content host — selecting a tab means showing its host and hiding the
    /// previously-selected one, never touching either one's actual content.
    pub(crate) fn set_tab_content_visible(&self, host: &TreeHostView, visible: bool) {
        host.setHidden(!visible);
    }
}

/// See docs/elwindui_builtins_spec.md 付録X. A single application-wide `NSMenu` (top menu bar
/// item / `File`, `Edit`, ...) entry — composed by `native_ui::MenuItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuItem {
    ns: Retained<NSMenuItem>,
    target_storage: Rc<RefCell<Option<Retained<MenuItemTarget>>>>,
}

impl InnerMenuItem {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        Self {
            ns,
            target_storage: Rc::new(RefCell::new(None)),
        }
    }

    /// A real `NSMenuItem.title` setter — construction takes no title argument, so this is the
    /// only way a menu item's title is ever actually set.
    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`); macOS defaults a menu item's modifier mask to Cmd,
    /// which matches the common `Cmd+<letter>` shortcuts notepad needs.
    pub(crate) fn set_shortcut(&self, key_equivalent: &str) {
        self.ns
            .setKeyEquivalent(&NSString::from_str(key_equivalent));
    }

    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn()>) {
        let target = MenuItemTarget::new(MenuItemTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }
}

struct MenuItemTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = MenuItemTargetIvars]
    struct MenuItemTarget;

    unsafe impl NSObjectProtocol for MenuItemTarget {}

    impl MenuItemTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl MenuItemTarget {
    fn new(ivars: MenuItemTargetIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

/// A dropdown attached to a `MenuBarItem` (or, per 付録M, a right-click context menu — not used
/// that way here, but the same type covers both) — composed by `native_ui::Menu`.
#[derive(Clone)]
pub(crate) struct InnerMenu {
    ns: Retained<NSMenu>,
}

impl InnerMenu {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        Self { ns }
    }

    pub(crate) fn add_item(&self, item: &InnerMenuItem) {
        self.ns.addItem(&item.ns);
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuItem) {
        self.ns.removeItem(&item.ns);
    }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `InnerMenu` — composed
/// by `native_ui::MenuBarItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuBarItem {
    ns: Retained<NSMenuItem>,
}

impl InnerMenuBarItem {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        Self { ns }
    }

    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }
    pub(crate) fn set_submenu(&self, submenu: &InnerMenu) {
        self.ns.setSubmenu(Some(&submenu.ns));
    }
}

/// The whole top menu bar, installed via `native_ui::Window::set_menu_bar` — composed by
/// `native_ui::MenuBar`.
#[derive(Clone)]
pub(crate) struct InnerMenuBar {
    ns: Retained<NSMenu>,
}

impl InnerMenuBar {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));

        // macOS convention: `mainMenu`'s *first* item is always displayed as the bold app name
        // (whatever title it's given is ignored/overridden by the OS) and its submenu is "the app
        // menu". Without one, the DSL's first real top-level item (e.g. "File") gets silently
        // absorbed into that slot instead of showing up as its own menu — so this app-menu slot,
        // with at minimum a working Quit item, is provided here rather than asked of the DSL
        // author, since it's a platform detail of `NSApp.mainMenu`, not something 付録X's
        // `MenuBar`/`MenuBarItem` DSL shape should need to know about.
        let app_menu_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        let app_menu = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        let quit_item = unsafe {
            // No target: leaving it nil dispatches through the responder chain to
            // `NSApplication`, which implements `terminate:` itself — the standard way to wire a
            // Quit item without the app needing to be its own `NSApplicationDelegate`.
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str("Quit"),
                Some(sel!(terminate:)),
                &NSString::from_str("q"),
            )
        };
        app_menu.addItem(&quit_item);
        app_menu_item.setSubmenu(Some(&app_menu));
        ns.addItem(&app_menu_item);
        Self { ns }
    }

    pub(crate) fn add_item(&self, item: &InnerMenuBarItem) {
        self.ns.addItem(&item.ns);
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuBarItem) {
        self.ns.removeItem(&item.ns);
    }
}

/// Offscreen golden-scene rendering tests (painter design doc §20.2) — renders a handful of
/// representative scenes into an in-memory `CGBitmapContext` via `CALayer.renderInContext`
/// (no window/screen involved, so no Screen Recording permission is needed and these run
/// headlessly in `cargo test`) and asserts specific sample pixels rather than diffing against a
/// checked-in reference PNG — a narrower, self-contained foundation for this class of test rather
/// than the full 24-scene cross-backend suite the design doc describes (WinUI3/GTK4 can't run on
/// this machine at all — see `docs/elwindui_implementation_status.md` — so a true cross-backend
/// image diff isn't achievable here regardless).
#[cfg(test)]
mod golden_tests {
    use super::*;

    struct Bitmap {
        ctx: CFRetained<objc2_core_graphics::CGContext>,
        pixels: Box<[u8]>,
        width: usize,
        height: usize,
        bytes_per_row: usize,
    }

    impl Bitmap {
        fn new(width: usize, height: usize) -> Self {
            let bytes_per_row = width * 4;
            let mut pixels = vec![0u8; bytes_per_row * height].into_boxed_slice();
            let color_space = CGColorSpace::new_device_rgb().expect("device RGB color space");
            let bitmap_info = objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast.0
                | objc2_core_graphics::CGBitmapInfo::ByteOrder32Big.0;
            let ctx = unsafe {
                objc2_core_graphics::CGBitmapContextCreate(
                    pixels.as_mut_ptr() as *mut _,
                    width,
                    height,
                    8,
                    bytes_per_row,
                    Some(&color_space),
                    bitmap_info,
                )
            }
            .expect("CGBitmapContextCreate");
            Self {
                ctx,
                pixels,
                width,
                height,
                bytes_per_row,
            }
        }

        fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
            assert!(x < self.width && y < self.height);
            let offset = y * self.bytes_per_row + x * 4;
            (
                self.pixels[offset],
                self.pixels[offset + 1],
                self.pixels[offset + 2],
                self.pixels[offset + 3],
            )
        }
    }

    fn render_layer(root: &Retained<CALayer>, bitmap: &Bitmap) {
        root.renderInContext(&bitmap.ctx);
    }

    fn approx(actual: (u8, u8, u8, u8), expected: (u8, u8, u8, u8), tolerance: u8) {
        let close = |a: u8, b: u8| a.abs_diff(b) <= tolerance;
        assert!(
            close(actual.0, expected.0)
                && close(actual.1, expected.1)
                && close(actual.2, expected.2)
                && close(actual.3, expected.3),
            "expected {expected:?}, got {actual:?} (tolerance {tolerance})"
        );
    }

    #[test]
    fn solid_filled_rect_paints_the_expected_color_and_nothing_outside_it() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::painter::Brush::Solid(
                elwindui_core::painter::Color::rgb(255, 0, 0),
            )),
            None,
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        approx(bitmap.pixel(32, 32), (255, 0, 0, 255), 50);
        approx(bitmap.pixel(2, 2), (0, 0, 0, 0), 10);
    }

    #[test]
    fn filled_ellipse_is_transparent_at_its_corners() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 8.0,
            y: 8.0,
            width: 48.0,
            height: 48.0,
        };
        let path = ellipse_cgpath(&world, rect);
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::painter::Brush::Solid(
                elwindui_core::painter::Color::rgb(0, 128, 255),
            )),
            None,
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        // Ellipse center: opaque blue.
        approx(bitmap.pixel(32, 32), (0, 128, 255, 255), 50);
        // Bounding-box corner: outside the ellipse's curve, must stay transparent.
        approx(bitmap.pixel(9, 9), (0, 0, 0, 0), 10);
    }

    #[test]
    fn stroked_rect_paints_only_near_its_border() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        let stroke = elwindui_core::painter::StrokeStyle {
            width: 4.0,
            ..Default::default()
        };
        add_shape_layer(
            &root,
            &path,
            None,
            Some((
                &elwindui_core::painter::Brush::Solid(elwindui_core::painter::Color::black()),
                &stroke,
            )),
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        // Interior of the rect (well inside the 4px-wide border): unpainted.
        approx(bitmap.pixel(32, 32), (0, 0, 0, 0), 10);
        // Right on the border: opaque black.
        approx(bitmap.pixel(16, 32), (0, 0, 0, 255), 40);
    }

    #[test]
    fn opacity_accumulator_scales_down_alpha() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::painter::Brush::Solid(
                elwindui_core::painter::Color::rgb(0, 255, 0),
            )),
            None,
            0.5,
            rect,
        );
        render_layer(&root, &bitmap);
        let (_, _, _, a) = bitmap.pixel(32, 32);
        assert!(
            a < 200,
            "half-opacity fill should not be fully opaque, got alpha {a}"
        );
        assert!(
            a > 50,
            "half-opacity fill should still be visibly painted, got alpha {a}"
        );
    }

    #[test]
    fn fitted_image_rect_fill_always_matches_dest_regardless_of_image_size() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let placed = fitted_image_rect(
            dest,
            (20.0, 80.0),
            elwindui_core::painter::ImageFit::Fill,
            elwindui_core::painter::AlignmentX::Center,
            elwindui_core::painter::AlignmentY::Center,
        );
        assert_eq!(placed, elwindui_core::base::Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 });
    }

    #[test]
    fn fitted_image_rect_contain_letterboxes_without_overflowing_dest() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        // A 200x100 (2:1) image `Contain`ed into a 100x100 square must shrink to fit the narrower
        // axis (height), leaving horizontal letterboxing rather than overflowing either axis.
        let placed = fitted_image_rect(
            dest,
            (200.0, 100.0),
            elwindui_core::painter::ImageFit::Contain,
            elwindui_core::painter::AlignmentX::Center,
            elwindui_core::painter::AlignmentY::Center,
        );
        assert_eq!(placed.width, 100.0);
        assert_eq!(placed.height, 50.0);
        assert_eq!(placed.x, 0.0);
        assert_eq!(placed.y, 25.0);
    }

    #[test]
    fn fitted_image_rect_cover_fills_dest_and_overflows_the_wider_axis() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        // The same 2:1 image `Cover`ing a 100x100 square must grow to fill the *shorter* axis
        // (height), overflowing width — the opposite of `Contain`'s letterboxing.
        let placed = fitted_image_rect(
            dest,
            (200.0, 100.0),
            elwindui_core::painter::ImageFit::Cover,
            elwindui_core::painter::AlignmentX::Center,
            elwindui_core::painter::AlignmentY::Center,
        );
        assert_eq!(placed.width, 200.0);
        assert_eq!(placed.height, 100.0);
        assert_eq!(placed.x, -50.0);
        assert_eq!(placed.y, 0.0);
    }

    #[test]
    fn fitted_image_rect_none_draws_at_intrinsic_size_and_honors_alignment() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let placed = fitted_image_rect(
            dest,
            (30.0, 20.0),
            elwindui_core::painter::ImageFit::None,
            elwindui_core::painter::AlignmentX::Right,
            elwindui_core::painter::AlignmentY::Bottom,
        );
        assert_eq!(placed.width, 30.0);
        assert_eq!(placed.height, 20.0);
        assert_eq!(placed.x, 70.0);
        assert_eq!(placed.y, 80.0);
    }
}
