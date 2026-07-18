//! Native-side AppKit plumbing — every type here is `Inner`-prefixed and, except for `AnyView`
//! itself (re-exported at the crate root; see `lib.rs`'s own doc comment), private to this crate.
//! `native_ui.rs` composes these as plain fields and calls into them; this module owns every bit
//! of genuinely AppKit-specific complexity (NSTextView delegates, tab strip bookkeeping, ...) so
//! `native_ui.rs` stays a thin, uniform "implement the core-side trait by delegating" layer.

use elwindui_core::base::{AsAny, Point};
use elwindui_core::input::{
    FocusState, Key, KeyModifiers, KeyboardDispatcher, MouseButton, PointerDispatcher,
    RawKeyEvent, RawKeyEventKind, RawPointerEvent, RawPointerEventKind, RawTextInputEvent,
    ShortcutRegistry,
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
    NSEventModifierFlags, NSMenu, NSMenuItem, NSScreen, NSScrollView, NSStackView, NSTextDelegate,
    NSTextView, NSTextViewDelegate, NSTrackingArea, NSTrackingAreaOptions,
    NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSRect, NSString};
use objc2_quartz_core::{
    CALayer, CAShapeLayer, CATextLayer, CATextLayerAlignmentMode, kCAAlignmentCenter,
    kCAAlignmentLeft, kCAAlignmentRight,
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
        fn replay_group(
            host: &TreeHostView,
            layer: &Retained<CALayer>,
            group: &RenderGroup,
            origin: elwindui_core::base::Point,
            inherited_clip: Option<elwindui_core::base::Rect>,
            live_native_controls: &mut HashSet<usize>,
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
            for command in &group.commands {
                match command {
                    RenderCommand::NativeControl { handle, rect, .. } => {
                        let Some(mut view) = handle.downcast_ref::<AnyView>().cloned() else {
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
                        let visible_rect = effective_clip
                            .and_then(|clip| intersect_rect(rect, clip))
                            .unwrap_or(rect);
                        if visible_rect.width <= 0.0 || visible_rect.height <= 0.0 {
                            continue;
                        }
                        // This is deliberately a native island only around an actual native
                        // command; ordinary RenderGroups continue to replay to `layer` above.
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
                            objc2_foundation::NSPoint::new(
                                visible_rect.x as f64,
                                visible_rect.y as f64,
                            ),
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
                    }
                    RenderCommand::Rectangle {
                        rect,
                        corner_radius,
                        fill,
                        stroke,
                        stroke_width,
                    } => {
                        let cg_rect = NSRect::new(
                            objc2_foundation::NSPoint::new(
                                (origin.x + rect.x) as f64,
                                (origin.y + rect.y) as f64,
                            ),
                            objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
                        );
                        let shape_layer = CAShapeLayer::new();
                        shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
                        let path = unsafe {
                            CGPath::with_rounded_rect(
                                cg_rect,
                                *corner_radius as f64,
                                *corner_radius as f64,
                                std::ptr::null(),
                            )
                        };
                        shape_layer.setPath(Some(&path));
                        match fill {
                            Some(fill) => shape_layer.setFillColor(Some(&parse_color(fill))),
                            None => shape_layer.setFillColor(None),
                        }
                        if let Some(stroke) = stroke {
                            shape_layer.setStrokeColor(Some(&parse_color(stroke)));
                        }
                        shape_layer.setLineWidth(*stroke_width as f64);
                        let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
                        layer.addSublayer(&shape_layer);
                    }
                    RenderCommand::Ellipse {
                        rect,
                        fill,
                        stroke,
                        stroke_width,
                    } => {
                        let cg_rect = NSRect::new(
                            objc2_foundation::NSPoint::new(
                                (origin.x + rect.x) as f64,
                                (origin.y + rect.y) as f64,
                            ),
                            objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
                        );
                        let shape_layer = CAShapeLayer::new();
                        shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
                        let path =
                            unsafe { CGPath::with_ellipse_in_rect(cg_rect, std::ptr::null()) };
                        shape_layer.setPath(Some(&path));
                        match fill {
                            Some(fill) => shape_layer.setFillColor(Some(&parse_color(fill))),
                            None => shape_layer.setFillColor(None),
                        }
                        if let Some(stroke) = stroke {
                            shape_layer.setStrokeColor(Some(&parse_color(stroke)));
                        }
                        shape_layer.setLineWidth(*stroke_width as f64);
                        let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
                        layer.addSublayer(&shape_layer);
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
                            objc2_foundation::NSPoint::new(
                                (origin.x + rect.x) as f64,
                                (origin.y + rect.y) as f64,
                            ),
                            objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
                        ));
                        text_layer.setFontSize(14.0);
                        text_layer.setForegroundColor(Some(&parse_color(
                            color.as_deref().unwrap_or("#000000"),
                        )));
                        text_layer.setAlignmentMode(ca_alignment_mode(*alignment));
                        unsafe {
                            text_layer.setString(Some(&NSString::from_str(content)));
                        }
                        let text_layer: Retained<CALayer> = Retained::into_super(text_layer);
                        layer.addSublayer(&text_layer);
                    }
                    RenderCommand::Line { .. }
                    | RenderCommand::Path { .. }
                    | RenderCommand::Image { .. } => {}
                }
            }
            for child in &group.children {
                replay_group(
                    host,
                    layer,
                    child,
                    origin,
                    effective_clip,
                    live_native_controls,
                );
            }
        }
        replay_group(
            self,
            &layer,
            &render_tree.root,
            elwindui_core::base::Point { x: 0.0, y: 0.0 },
            None,
            &mut live_native_controls,
        );
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
        /*for item in items {
            match item {
                RenderItem::Native(mut view, rect, _node) => {
                    let nsview = view.as_nsview();
                    self.addSubview(&nsview);
                    nsview.setTranslatesAutoresizingMaskIntoConstraints(true);
                    view.arrange(rect);
                }
                RenderItem::Paint(paint, rect) => {
                    let cg_rect = NSRect::new(
                        objc2_foundation::NSPoint::new(rect.x as f64, rect.y as f64),
                        objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
                    );
                    match paint {
                        PaintKind::ShapeExt {
                            kind,
                            fill,
                            stroke,
                            stroke_width,
                        } => {
                            let shape_layer = CAShapeLayer::new();
                            shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
                            let path = unsafe {
                                match kind {
                                    ShapeKind::RoundedRect { corner_radius } => {
                                        CGPath::with_rounded_rect(
                                            cg_rect,
                                            corner_radius as f64,
                                            corner_radius as f64,
                                            std::ptr::null(),
                                        )
                                    }
                                    ShapeKind::Oval => {
                                        CGPath::with_ellipse_in_rect(cg_rect, std::ptr::null())
                                    }
                                }
                            };
                            shape_layer.setPath(Some(&path));
                            match &fill {
                                Some(fill) => shape_layer.setFillColor(Some(&parse_color(fill))),
                                None => shape_layer.setFillColor(None),
                            }
                            if let Some(stroke) = &stroke {
                                shape_layer.setStrokeColor(Some(&parse_color(stroke)));
                            }
                            shape_layer.setLineWidth(stroke_width as f64);
                            let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
                            layer.addSublayer(&shape_layer);
                        }
                        PaintKind::Text {
                            content,
                            color,
                            alignment,
                        } => {
                            let text_layer = CATextLayer::new();
                            text_layer.setName(Some(&NSString::from_str("elwindui-paint")));
                            text_layer.setFrame(cg_rect);
                            text_layer.setFontSize(14.0);
                            text_layer.setForegroundColor(Some(&parse_color(
                                color.as_deref().unwrap_or("#000000"),
                            )));
                            text_layer.setAlignmentMode(ca_alignment_mode(alignment));
                            unsafe {
                                text_layer.setString(Some(&NSString::from_str(&content)));
                            }
                            let text_layer: Retained<CALayer> = Retained::into_super(text_layer);
                            layer.addSublayer(&text_layer);
                        }
                    }
                }
            }
        }*/
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
