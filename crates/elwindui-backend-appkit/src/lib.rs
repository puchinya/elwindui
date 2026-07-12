//! AppKit implementation of the widget surface `elwindui-codegen` targets for the `notepad`
//! example. See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! Only genuinely native leaf widgets (`Window`/`TextAreaImpl`/`ButtonImpl`/`MenuBarImpl`/`TabViewImpl`, the
//! "NativeControl" family — see docs/elwindui_spec.md 付録E) have a Rust struct here at all.
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have none: they're
//! `elwindui_core::ui::UIElement` values that `elwindui-codegen` builds directly, reflected into
//! real `NSView`s/`CAShapeLayer`s/`CATextLayer`s by `TreeHostView` below (used by both `Window`'s
//! content view and `TabViewImpl`'s per-tab content area).

#![cfg(target_os = "macos")]

/// DSL-facing `Window`/`TextArea`/`Button`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabView`/
/// `TabViewItem` wrappers that `elwindui-codegen`'s generated code actually constructs — see this
/// module's own doc comment for why it's split out from the raw types above instead of sharing
/// their names at this crate's root.
pub mod builtins;

use elwindui_core::ui::{layout_tree, AsAny, Button as _, PaintKind, RelayoutHost, RenderItem, ShapeKind, UIElement};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSButton, NSMenu, NSMenuItem, NSScreen, NSScrollView, NSStackView, NSTextDelegate, NSTextView,
    NSTextViewDelegate, NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSRect, NSString};
use objc2_quartz_core::{kCAAlignmentCenter, kCAAlignmentLeft, kCAAlignmentRight, CALayer, CAShapeLayer, CATextLayer, CATextLayerAlignmentMode};
use std::cell::RefCell;
use std::rc::Rc;

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("elwindui-backend-appkit must run on the main thread")
}

/// The capability a type needs to be usable as an `AnyView` — implemented once per raw native view
/// type (`Retained<NSScrollView>`/`Retained<NSButton>`/`Retained<NSStackView>`) instead of matched
/// on centrally, so a future native leaf (`Dialog`, `VirtualList`, ...) only needs its own `impl
/// AppKitHandle`, never a change to `AnyView` itself or to any `match` over it — the same open-set
/// extensibility `elwindui_core::ui::UIElement` already has over `NativeControl`/`Stack`/`Shape`/
/// etc. (see that trait's own doc comment), now applied to the handle side too.
///
/// Implemented on the raw `Retained<T>` view type itself (a foreign type — allowed since
/// `AppKitHandle` is a local trait) rather than on `TextAreaImpl`/`ButtonImpl`/`TabViewImpl`, since
/// those now each compose an `elwindui_core::ui::NativeControlImpl<AnyView>` as their own `base`
/// field (docs/elwindui_spec.md 付録H.2.1a) — an `AnyView` wrapping the not-yet-fully-constructed
/// widget itself would be a self-reference. Wrapping just the raw view instead lets `base.handle`
/// be built (`AnyView::from(ns.clone())`) before the rest of the widget struct exists.
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

/// Everything the generated code can pass as a `Window`/`TabViewImpl` child.
/// `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse` have no variant here at all — they're
/// purely `elwindui_core::ui::Node::Virtual` values now (see `TreeHostView` below), never a real
/// widget of their own. An `Rc<dyn AppKitHandle>` (not a closed `enum`) so adding a new native leaf
/// never requires touching this type — see `AppKitHandle`'s own doc comment.
#[derive(Clone)]
pub struct AnyView(Rc<dyn AppKitHandle>);

impl AnyView {
    fn as_nsview(&self) -> Retained<NSView> {
        self.0.as_nsview()
    }
}

/// Lets `TreeHostView` (below) measure/arrange any native leaf uniformly through the base `NSView`
/// API (`fittingSize`/`setFrame`) regardless of which concrete widget it wraps — no per-widget
/// (`Text`, `ButtonImpl`, ...) `LayoutNode` impl needed. See docs/elwindui_spec.md 付録H.2.
impl elwindui_core::layout::LayoutNode for AnyView {
    fn measure(&self, _available: elwindui_core::layout::Size) -> elwindui_core::layout::Size {
        let fitting = self.as_nsview().fittingSize();
        elwindui_core::layout::Size { width: fitting.width as f32, height: fitting.height as f32 }
    }

    fn arrange(&mut self, final_rect: elwindui_core::layout::Rect) {
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

#[derive(Clone)]
pub struct Window {
    ns: Retained<NSWindow>,
    content_host: Retained<TreeHostView>,
}

impl Window {
    pub fn new() -> Self {
        let mtm = mtm();
        let content_rect = NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(480.0, 360.0));
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
        ns.setContentView(Some(&content_host));
        Self { ns, content_host }
    }

    /// Replaces the window's whole content tree — see `TreeHostView` for how an `Rc<dyn
    /// UIElement>` (layouts/shapes/text mixed freely with native controls, at any nesting depth)
    /// gets reflected into real `NSView` subviews and `CAShapeLayer`/`CATextLayer` sublayers.
    pub fn set_content(&self, content: Rc<dyn UIElement>) {
        self.content_host.set_tree(content);
    }

    pub fn set_title(&self, title: &str) {
        self.ns.setTitle(&NSString::from_str(title));
    }

    /// Sets `NSApplication.mainMenu` (macOS has one global top menu bar, not a per-window one).
    pub fn set_menu_bar(&self, menu_bar: &MenuBarImpl) {
        NSApplication::sharedApplication(mtm()).setMainMenu(Some(&menu_bar.ns));
    }

    /// Shows the window and activates the app. Does not block — call `application::run()`
    /// afterward to actually enter the platform event loop (see that module's doc comment for
    /// why the two are separate).
    pub fn show(&self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        self.ns.makeKeyAndOrderFront(None);
        app.activate();
    }

    /// The height (in points) of the screen `top`/`set_top`/`set_height` convert against — the
    /// window's own current screen if it has one (it may not yet, before the first `show`), else
    /// the main screen, else `0.0` (no screen at all, e.g. headless test environment).
    fn screen_height(&self) -> f64 {
        self.ns
            .screen()
            .or_else(|| NSScreen::mainScreen(mtm()))
            .map(|screen| screen.frame().size.height)
            .unwrap_or(0.0)
    }

    /// WinUI3's `AppWindow.Position.X` — AppKit's own `NSWindow.frame.origin.x` needs no
    /// conversion (both are left-origin).
    pub fn left(&self) -> f32 {
        self.ns.frame().origin.x as f32
    }

    pub fn set_left(&self, left: f32) {
        let mut frame = self.ns.frame();
        frame.origin.x = left as f64;
        self.ns.setFrame_display(frame, true);
    }

    /// WinUI3's `AppWindow.Position.Y` — top-left-origin, Y increasing downward, unlike AppKit's
    /// own bottom-left-origin `NSWindow.frame.origin.y` (Y increasing upward), so this converts
    /// via the window's own screen height (see `screen_height`).
    pub fn top(&self) -> f32 {
        let frame = self.ns.frame();
        (self.screen_height() - (frame.origin.y + frame.size.height)) as f32
    }

    pub fn set_top(&self, top: f32) {
        let screen_height = self.screen_height();
        let mut frame = self.ns.frame();
        frame.origin.y = screen_height - top as f64 - frame.size.height;
        self.ns.setFrame_display(frame, true);
    }

    pub fn width(&self) -> f32 {
        self.ns.frame().size.width as f32
    }

    pub fn set_width(&self, width: f32) {
        let mut frame = self.ns.frame();
        frame.size.width = width as f64;
        self.ns.setFrame_display(frame, true);
    }

    pub fn height(&self) -> f32 {
        self.ns.frame().size.height as f32
    }

    /// Keeps `top` (not AppKit's own bottom-left `origin.y`) fixed as the height changes — matching
    /// WinUI3's `AppWindow.Resize`, which grows/shrinks a window downward from its current `Top`.
    pub fn set_height(&self, height: f32) {
        let mut frame = self.ns.frame();
        let old_height = frame.size.height;
        frame.size.height = height as f64;
        frame.origin.y -= height as f64 - old_height;
        self.ns.setFrame_display(frame, true);
    }
}

fn new_stack(children: Vec<AnyView>, orientation: NSUserInterfaceLayoutOrientation) -> Retained<NSStackView> {
    let m = mtm();
    let views: Vec<Retained<NSView>> = children.iter().map(AnyView::as_nsview).collect();
    let ns = NSStackView::stackViewWithViews(&objc2_foundation::NSArray::from_retained_slice(&views), m);
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
        (6, Ok(v)) => (((v >> 16) & 0xFF) as f64, ((v >> 8) & 0xFF) as f64, (v & 0xFF) as f64, 255.0),
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

/// `elwindui_core::ui::TextAlignment` -> `CATextLayer.alignmentMode` — the `kCAAlignment*` values
/// are `extern "C"` globals (`&'static NSString`), hence the `unsafe` read.
fn ca_alignment_mode(alignment: elwindui_core::ui::TextAlignment) -> &'static CATextLayerAlignmentMode {
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
/// subviews/`CAShapeLayer`/`CATextLayer` sublayers" host, replacing the old per-container
/// `StackLayoutView` (`VerticalLayout`/`HorizontalLayout`) — since `VerticalLayout`/
/// `HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` are now all just `UIElement` values with
/// no backend struct of their own (docs/elwindui_spec.md 付録H.2), one host type is all any native
/// container needs to accept arbitrary content: `Window`'s content view and `TabViewImpl`'s per-tab
/// content area both are one of these.
pub struct TreeHostIvars {
    tree: RefCell<Option<Rc<dyn UIElement>>>,
    /// Set once, right after construction (see `TreeHostView::new`) — lets `set_tree` hand out an
    /// `AppKitRelayoutHost` wrapping a weak reference back to this same view, without needing a
    /// `Retained<Self>` in hand at that point. See `AppKitRelayoutHost`'s own doc comment for why
    /// weak, not strong (retain cycle).
    weak_self: RefCell<objc2::rc::Weak<TreeHostView>>,
}

/// `elwindui_core::ui::RelayoutHost` for `TreeHostView` — wraps a *weak* reference back to the view
/// (not the view itself) since a strong one would create a reference cycle: this view's own
/// `ivars().tree` strongly holds the hosted tree's root, and that root's own `UIElementImpl::
/// invalidate_host` would then strongly hold this, right back to the view. `request_relayout`
/// silently does nothing if the view has since been deallocated (`load()` returns `None`).
struct AppKitRelayoutHost(objc2::rc::Weak<TreeHostView>);

impl RelayoutHost for AppKitRelayoutHost {
    fn request_relayout(&self) {
        if let Some(view) = self.0.load() {
            // Schedules AppKit's normal deferred layout pass (this view's own `layout()` override,
            // above, runs at the next display cycle) rather than calling `relayout()` synchronously
            // — the same standard AppKit idiom already driving window-resize-triggered relayout.
            view.setNeedsLayout(true);
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
        /// Overrides `NSView`'s own layout pass instead of adding Auto Layout constraints — the
        /// hook that makes re-hosting genuinely dynamic (re-run on window resize / tab-strip
        /// changes), same role `StackLayoutView::layout` used to play for just `VerticalLayout`/
        /// `HorizontalLayout`.
        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe {
                let _: () = msg_send![super(self), layout];
            }
            self.relayout();
        }

        /// Reports this host's current tree's natural size so `fittingSize()` — and therefore an
        /// *outer* `AnyView::measure()` (this host nested inside another virtual container) or an
        /// outer `NSStackView` (`TabViewImpl`'s content area sits in one) — sees something meaningful
        /// instead of AppKit's zero-size default for a plain, constraint-free `NSView`.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> objc2_foundation::NSSize {
            let size = self
                .ivars()
                .tree
                .borrow()
                .as_ref()
                .map(|tree| elwindui_core::ui::natural_size(&**tree))
                .unwrap_or(elwindui_core::layout::Size { width: 0.0, height: 0.0 });
            objc2_foundation::NSSize::new(size.width as f64, size.height as f64)
        }

        /// `elwindui_core::layout::stack_arrange` (and every other `VirtualNode` arrange
        /// implementation) computes top-down coordinates (Y increasing downward, origin at the
        /// top — the same convention WinUI3's `Canvas` uses) — but a plain `NSView`'s default
        /// coordinate system has its origin at the bottom-left with Y increasing *upward*.
        /// Without this override, `relayout`'s `setFrame`/`CAShapeLayer` positioning would be
        /// interpreted bottom-up, effectively flipping every child's vertical position.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl TreeHostView {
    fn new() -> Retained<Self> {
        let m = mtm();
        let ivars = TreeHostIvars { tree: RefCell::new(None), weak_self: RefCell::new(objc2::rc::Weak::default()) };
        let this = Self::alloc(m).set_ivars(ivars);
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::default()] };
        *this.ivars().weak_self.borrow_mut() = objc2::rc::Weak::from_retained(&this);
        this
    }

    /// Replaces this host's entire content, discarding whatever native subviews were there before
    /// — a full swap rather than a diff. `pub` (unlike most of this type's methods) since
    /// `TabViewImpl::insert_tab` hands a fresh host straight to its caller (`elwindui-builtins`'s
    /// wrapper), which calls this exactly once per tab to populate it — see that type's own doc
    /// comment for why each tab gets its own persistent host instead of sharing one.
    pub fn set_tree(&self, tree: Rc<dyn UIElement>) {
        for old in self.subviews().iter() {
            old.removeFromSuperview();
        }
        let weak_self = self.ivars().weak_self.borrow().clone();
        tree.base().set_invalidate_host(Some(Rc::new(AppKitRelayoutHost(weak_self))));
        *self.ivars().tree.borrow_mut() = Some(tree);
        self.invalidateIntrinsicContentSize();
        self.relayout();
    }

    /// Re-measures and re-arranges every native leaf, and re-syncs every self-painting node's
    /// `CAShapeLayer`, against this view's *current* frame — called from `layout()` (above)
    /// whenever AppKit thinks this view's size may have changed.
    fn relayout(&self) {
        use elwindui_core::layout::{LayoutNode, Size};

        let frame = self.frame();
        let available = Size { width: frame.size.width as f32, height: frame.size.height as f32 };
        let tree = self.ivars().tree.borrow();
        let Some(tree) = tree.as_ref() else { return };
        let items: Vec<RenderItem<AnyView>> = layout_tree(tree, available);

        self.setWantsLayer(true);
        let layer = self.layer().expect("wantsLayer(true) implies a layer");
        // Only ever touches sublayers *this* code added (tagged below) — AppKit manages its own
        // per-subview backing layers alongside these when the view is layer-backed, and those must
        // be left alone.
        if let Some(existing) = unsafe { layer.sublayers() } {
            // Collected into a `Vec` first, then removed in a separate pass: `removeFromSuperlayer`
            // mutates this same array (it's `layer`'s live `sublayers`), and Cocoa's fast
            // enumeration protocol (which `.iter()` uses) raises "mutation detected during
            // enumeration" if the backing array changes mid-iteration.
            let stale: Vec<_> = existing
                .iter()
                .filter(|sub| sub.name().map(|n| n.to_string()).as_deref() == Some("elwindui-paint"))
                .collect();
            for sub in stale {
                sub.removeFromSuperlayer();
            }
        }

        // `items` is `layout_tree`'s single interleaved list, in `arrange`'s own parent-before-
        // children traversal order (see `RenderItem`'s doc comment) — replayed here in one pass, so
        // `addSubview`/`addSublayer` happen in the exact order encountered and document order
        // becomes z-order for native leaves and self-painted content alike (e.g. a `Rectangle`'s
        // fill staying behind a `ButtonImpl` child placed after it), instead of batching "all natives"
        // then "all paints", which threw the relative ordering between the two away.
        for item in items {
            match item {
                // The tree node (third tuple element) needs no handling here anymore — a native
                // leaf's `#[routed]` click is wired directly to `dispatch_routed` once, at its own
                // construction time (see `elwindui_backend_appkit::builtins::Button::new`), since
                // its `base` (and thus `routed_handlers`) is already real by then.
                RenderItem::Native(mut view, rect, _node) => {
                    let nsview = view.as_nsview();
                    self.addSubview(&nsview);
                    // Every native leaf here is positioned purely by manual `setFrame` (via
                    // `arrange` below), never by Auto Layout constraints — but a plain
                    // `addSubview:` leaves `translatesAutoresizingMaskIntoConstraints` at its
                    // *class* default, which for an Auto-Layout-authored view like `NSStackView`
                    // (`TabViewImpl`'s root) is `NO`. With no explicit size constraints of its own,
                    // such a view gets silently resized back down to its intrinsic content size on
                    // the next layout pass — undoing `arrange`'s `setFrame` entirely. Forcing this
                    // back to `YES` opts every leaf out of the constraint system so our manual
                    // frame actually sticks.
                    nsview.setTranslatesAutoresizingMaskIntoConstraints(true);
                    view.arrange(rect);
                }
                RenderItem::Paint(paint, rect) => {
                    let cg_rect = NSRect::new(
                        objc2_foundation::NSPoint::new(rect.x as f64, rect.y as f64),
                        objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
                    );
                    match paint {
                        PaintKind::Shape { kind, fill, stroke, stroke_width } => {
                            let shape_layer = CAShapeLayer::new();
                            shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
                            let path = unsafe {
                                match kind {
                                    ShapeKind::RoundedRect { corner_radius } => CGPath::with_rounded_rect(
                                        cg_rect,
                                        corner_radius as f64,
                                        corner_radius as f64,
                                        std::ptr::null(),
                                    ),
                                    ShapeKind::Oval => CGPath::with_ellipse_in_rect(cg_rect, std::ptr::null()),
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
                        PaintKind::Text { content, color, alignment } => {
                            let text_layer = CATextLayer::new();
                            text_layer.setName(Some(&NSString::from_str("elwindui-paint")));
                            text_layer.setFrame(cg_rect);
                            text_layer.setFontSize(14.0);
                            text_layer.setForegroundColor(Some(&parse_color(color.as_deref().unwrap_or("#000000"))));
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
        }
    }
}


#[elwindui_macros::class(implements = elwindui_core::ui::TextArea, inherits = elwindui_core::ui::NativeControl<AnyView>)]
pub struct TextArea {
    text_view: Retained<NSTextView>,
    delegate_storage: Rc<RefCell<Option<Retained<TextViewDelegate>>>>,
}

/// `elwindui_core::ui::TextArea`'s shape is common to every backend (docs/elwindui_spec.md
/// 付録H.2.1a) — see that trait's own doc comment; only these method bodies are AppKit-specific.
#[elwindui_macros::class(implements = elwindui_core::ui::TextArea, inherits = elwindui_core::ui::NativeControl<AnyView>)]
impl TextArea {
    /// `NSTextView.setString:` unconditionally resets the caret/selection to the start, even when
    /// the text it's given is identical to what's already there. The two-way `#[two_way] text`
    /// binding (`TextArea` in `builtins.elwind`) re-syncs *every* bound field on *every* model
    /// change — including the one this exact edit just caused — so without this guard, typing a
    /// single character would immediately call this with that same character already applied,
    /// yanking the caret away mid-keystroke. Skipping the no-op case leaves the user's own edit
    /// (and the caret AppKit already placed for it) untouched; only a *genuine* external change
    /// (e.g. loading a different document) still goes through `setString:`.
    fn set_text(&self, text: &str) {
        if self.text_view.string().to_string() == text {
            return;
        }
        self.text_view.setString(&NSString::from_str(text));
    }

    /// `NSTextView.delegate` is an unretained (weak) reference, so the delegate this creates is
    /// only kept alive by `self.delegate_storage`.
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        let m = mtm();
        let ivars = TextDelegateIvars { text_view: self.text_view.clone(), callback };
        let delegate = TextViewDelegate::new(m, ivars);
        let protocol_obj: &objc2::runtime::ProtocolObject<dyn NSTextViewDelegate> =
            objc2::runtime::ProtocolObject::from_ref(&*delegate);
        self.text_view.setDelegate(Some(protocol_obj));
        *self.delegate_storage.borrow_mut() = Some(delegate);
    }

    fn new() -> Self {
        let m = mtm();
        let scroll = NSTextView::scrollableTextView(m);
        let text_view = scroll
            .documentView()
            .expect("scrollableTextView always has a document view")
            .downcast::<NSTextView>()
            .expect("scrollableTextView's document view is an NSTextView");
        let handle = AnyView::from(scroll);
        Self {
            base: elwindui_core::ui::create_native_control(handle),
            text_view,
            delegate_storage: Rc::new(RefCell::new(None)),
        }
    }
}

pub fn create_text_area() -> TextAreaImpl {
    TextAreaImpl::new()
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

#[elwindui_macros::class(implements = elwindui_core::ui::Button, inherits = elwindui_core::ui::NativeControl<AnyView>)]
pub struct Button {
    ns: Retained<NSButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

/// `elwindui_core::ui::Button`'s shape is common to every backend — see that trait's own doc
/// comment; only these method bodies are AppKit-specific.
#[elwindui_macros::class(implements = elwindui_core::ui::Button, inherits = elwindui_core::ui::NativeControl<AnyView>)]
impl Button {
    fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    fn set_on_click(&self, callback: Box<dyn Fn()>) {
        let target = ButtonTarget::new(ButtonTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }

    /// Used by `TabChipImpl` to rename a tab's title button when its document's file name changes.
    fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    /// AppKit-only helper (no `elwindui_core::ui::Button` trait member — WinUI3's real `TabView`
    /// highlights its selected tab for free, no borderless-button trick needed there): used by
    /// `create_tab_chip` so `TabChipImpl::set_selected`'s translucent background tint shows through
    /// instead of being hidden behind the button's own opaque default bezel.
    #[inherent]
    fn set_bordered(&self, bordered: bool) {
        self.ns.setBordered(bordered);
    }

    fn new() -> Self {
        let m = mtm();
        let ns = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(""), None, None, m) };
        let handle = AnyView::from(ns.clone());
        Self { base: elwindui_core::ui::create_native_control(handle), ns, target_storage: Rc::new(RefCell::new(None)) }
    }
}

pub fn create_button() -> ButtonImpl {
    ButtonImpl::new()
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
/// its two buttons are built with a plain default `base` — no use-site margin/alignment ever
/// applies to them.
pub struct TabChipImpl {
    ns: Retained<NSStackView>,
    pub title_button: ButtonImpl,
    pub close_button: ButtonImpl,
}

fn create_tab_chip(title: &str) -> TabChipImpl {
    let title_button = create_button();
    title_button.set_text(title);
    // Borderless: an `NSButton`'s default bezel is opaque and would otherwise cover almost the
    // entire chip row, hiding `set_selected`'s translucent background tint underneath it.
    title_button.set_bordered(false);
    let close_button = create_button();
    close_button.set_text("×");
    close_button.set_bordered(false);
    let ns = new_stack(
        vec![title_button.base.handle.clone(), close_button.base.handle.clone()],
        NSUserInterfaceLayoutOrientation::Horizontal,
    );
    TabChipImpl { ns, title_button, close_button }
}

impl TabChipImpl {
    pub fn set_title(&self, title: &str) {
        self.title_button.set_text(title);
    }

    /// Highlights this chip's own row with a translucent background tint when it's the selected
    /// tab. AppKit has no native "selected tab" concept to lean on here (unlike WinUI3's real
    /// `Controls::TabView`, whose `SelectedIndex` gets OS-drawn highlighting for free) — this
    /// backend hand-rolls its tab strip out of a plain `NSStackView` (see this type's own doc
    /// comment), so the highlight is drawn the same way `Rectangle`'s own `fill` is: a
    /// layer-backed background color, applied to `ns` (the chip's whole row) rather than just
    /// `title_button` so it isn't hidden behind that button's own bezel rendering.
    pub fn set_selected(&self, selected: bool) {
        self.ns.setWantsLayer(true);
        let layer = self.ns.layer().expect("wantsLayer(true) implies a layer");
        if selected {
            layer.setBackgroundColor(Some(&parse_color("#7f7f7f40")));
        } else {
            layer.setBackgroundColor(None);
        }
    }
}

/// The row of `TabChipImpl`s plus a trailing "+" button. `TabViewImpl` owns one of these and the content
/// area below it; kept as a separate type since 付録Y's backend table describes it as its own
/// piece (a custom `NSStackView`-based strip, not `NSTabViewController`). Purely an internal
/// composition helper too — see `TabChipImpl`'s own doc comment.
pub struct TabStripImpl {
    ns: Retained<NSStackView>,
    pub new_tab_button: ButtonImpl,
}

fn create_tab_strip() -> TabStripImpl {
    let new_tab_button = create_button();
    new_tab_button.set_text("+");
    let ns = new_stack(vec![new_tab_button.base.handle.clone()], NSUserInterfaceLayoutOrientation::Horizontal);
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

/// See docs/elwindui_builtins_spec.md 付録Y. Vertical stack of `[TabStripImpl, content_container]`;
/// the generated code (via `elwindui-builtins::appkit::tab_view`) owns the mapping from
/// `items_source`/static `TabViewItem`s to `TabChipImpl`s + content hosts. This type only holds the
/// widget areas — it has no notion of "the list of tabs" on its own, matching how
/// `VerticalLayout`/`HorizontalLayout` (purely `elwindui_core::ui::Node::Virtual` values) don't
/// know about the data their children came from either.
///
/// Unlike an earlier version of this type, `content_container` isn't itself a single
/// `TreeHostView` swapped wholesale on every tab switch — each tab gets its *own* persistent
/// `TreeHostView` (created once, in `insert_tab`), added as an overlaid subview of
/// `content_container` and shown/hidden via `set_tab_content_visible` rather than destroyed and
/// rebuilt. A `Box<dyn UIElement>` isn't `Clone`, so a tab's content can only ever be handed over
/// once (`TreeHostView::set_tree`) — a single shared pane would have no way to restore a
/// previously-shown-then-hidden tab's content after switching away from it. Each host tracks its
/// own `elwindui_core::ui` (keeping any native leaf's retention concern alive, e.g. a
/// `TextAreaImpl`'s change-notification delegate — see `TextAreaImpl::set_on_change`'s doc comment — for
/// as long as its tab exists, not just while it's the visible one).
/// `TabViewImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — extends `NativeControl<AnyView>`
/// since a real `AnyView` handle (`self.base.handle`, wrapping the outer `NSStackView`) is what
/// makes this leaf embeddable in the visual tree at all.
pub trait TabView: elwindui_core::ui::NativeControl<AnyView> {
    fn set_on_new_tab(&self, callback: Box<dyn Fn()>);
    /// Inserts a new tab chip at `index` (wiring `on_select`/`on_close` to the given callbacks)
    /// plus a fresh, persistent content host — added to `content_container`, initially hidden.
    /// `elwindui-builtins`'s wrapper calls `TreeHostView::set_tree` on the returned host exactly
    /// once (its content never needs to be handed over again — see this type's own doc comment),
    /// and `set_tab_content_visible` to toggle it on selection.
    fn insert_tab(&self, index: usize, title: &str, on_select: Box<dyn Fn()>, on_close: Box<dyn Fn()>) -> (TabChipImpl, Retained<TreeHostView>);
    /// Removes a tab's chip and its persistent content host together.
    fn remove_tab(&self, chip: &TabChipImpl, host: &TreeHostView);
    /// Shows or hides a tab's content host — selecting a tab means showing its host and hiding
    /// the previously-selected one, never touching either one's actual content.
    fn set_tab_content_visible(&self, host: &TreeHostView, visible: bool);
}

#[elwindui_macros::class(implements = TabView, inherits = elwindui_core::ui::NativeControl<AnyView>)]
pub struct TabViewImpl {
    pub strip: TabStripImpl,
    content_container: Retained<NSView>,
}

#[elwindui_macros::class(implements = TabView, inherits = elwindui_core::ui::NativeControl<AnyView>)]
impl TabViewImpl {
    fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.strip.new_tab_button.set_on_click(callback);
    }

    fn insert_tab(
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
        // constraints of its own — it isn't managed by `NSStackView` — so this is the default
        // anyway, made explicit) plus a `.width | .height` autoresizing mask makes AppKit stretch
        // `host` to match `content_container`'s bounds on every resize, with no custom `NSView`
        // subclass or constraint bookkeeping needed here.
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

    fn remove_tab(&self, chip: &TabChipImpl, host: &TreeHostView) {
        self.strip.remove_tab(chip);
        host.removeFromSuperview();
    }

    fn set_tab_content_visible(&self, host: &TreeHostView, visible: bool) {
        host.setHidden(!visible);
    }

    fn new() -> Self {
        let m = mtm();
        let strip = create_tab_strip();
        let content_container = NSView::initWithFrame(NSView::alloc(m), NSRect::default());
        let strip_view: Retained<NSView> = Retained::into_super(strip.ns.clone());
        let root =
            NSStackView::stackViewWithViews(&objc2_foundation::NSArray::from_retained_slice(&[strip_view, content_container.clone()]), m);
        root.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        // `NSStackView`'s default `distribution` (`GravityAreas`) leaves each arranged subview at
        // its own intrinsic size unless hugging priorities say otherwise — `.Fill` makes the stack
        // actually consume its *entire* stacking-axis extent, matching `TabViewImpl`'s expected "chips
        // row at natural height, content area fills the rest" shape. `content_container`'s own
        // vertical hugging priority is dropped to (near-)zero so it — not the also-low-priority-
        // by-default `strip` — is the one that absorbs whatever space `Fill` distributes (a plain
        // `NSView` with no subviews yet has no intrinsic size hint worth respecting anyway).
        content_container.setContentHuggingPriority_forOrientation(1.0, objc2_app_kit::NSLayoutConstraintOrientation::Vertical);
        root.setDistribution(objc2_app_kit::NSStackViewDistribution::Fill);
        let handle = AnyView::from(root);
        Self { base: elwindui_core::ui::create_native_control(handle), strip, content_container }
    }
}

pub fn create_tab_view() -> TabViewImpl {
    TabViewImpl::new()
}

/// See docs/elwindui_builtins_spec.md 付録X. A single application-wide `NSMenu` (top menu bar
/// item / `File`, `Edit`, ...), reusing `MenuItemImpl` for its leaf entries.
#[derive(Clone)]
pub struct MenuItemImpl {
    ns: Retained<NSMenuItem>,
    target_storage: Rc<RefCell<Option<Retained<MenuItemTarget>>>>,
}

/// `elwindui_core::ui::MenuItem`'s shape is common to every backend — see that trait's own doc
/// comment; only these method bodies are AppKit-specific.
impl elwindui_core::ui::MenuItem for MenuItemImpl {
    /// A real `NSMenuItem.title` setter — `create_menu_item()` takes no title argument, so this is
    /// the only way a menu item's title is ever actually set.
    fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`); macOS defaults a menu item's modifier mask to Cmd,
    /// which matches the common `Cmd+<letter>` shortcuts notepad needs (付録K.2's platform
    /// conversion rule already reads "Ctrl" as "Cmd" on macOS at the DSL level).
    fn set_shortcut(&self, key_equivalent: &str) {
        self.ns.setKeyEquivalent(&NSString::from_str(key_equivalent));
    }

    fn set_on_select(&self, callback: Box<dyn Fn()>) {
        let target = MenuItemTarget::new(MenuItemTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }
}

pub fn create_menu_item() -> MenuItemImpl {
    let m = mtm();
    let ns = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(m.alloc::<NSMenuItem>(), &NSString::from_str(""), None, &NSString::from_str(""))
    };
    MenuItemImpl { ns, target_storage: Rc::new(RefCell::new(None)) }
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

/// A dropdown attached to a `MenuBarItemImpl` (or, per 付録M, a right-click context menu — not used
/// that way here, but the same type covers both).
#[derive(Clone)]
pub struct MenuImpl {
    ns: Retained<NSMenu>,
}

/// `add_item`/`remove_item` are real `NSMenu.addItem(_:)`/`.removeItem(_:)` calls (the same API
/// `create_menu` already used to call in a loop at construction time), now also reachable
/// post-construction so the wrapper's own `set_children` (`elwindui-backend-appkit::builtins`) can
/// reconcile a changed child list without rebuilding the native `NSMenu` from scratch. Plain
/// inherent methods (not `elwindui_core::ui::Menu`, unlike the DSL-facing wrapper's own `impl Menu
/// for builtins::MenuImpl`) — this raw layer already knows its own concrete `MenuItemImpl` argument
/// type, so it has no need for that trait's `&dyn MenuItem`/`AsAny`-downcast indirection, the same
/// way `Window` above stays plain inherent methods throughout.
impl MenuImpl {
    fn add_item(&self, item: &MenuItemImpl) {
        self.ns.addItem(&item.ns);
    }
    fn remove_item(&self, item: &MenuItemImpl) {
        self.ns.removeItem(&item.ns);
    }
}

pub fn create_menu() -> MenuImpl {
    let m = mtm();
    let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
    MenuImpl { ns }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `MenuImpl`.
#[derive(Clone)]
pub struct MenuBarItemImpl {
    ns: Retained<NSMenuItem>,
}

/// `set_text`/`set_submenu` are real post-construction setters (`create_menu_bar_item()` takes
/// neither argument, so these are the only way a menu bar item's title/submenu are ever actually
/// set). Plain inherent methods — see `MenuImpl`'s own `add_item`/`remove_item` doc comment for why
/// this raw layer doesn't implement `elwindui_core::ui::MenuBarItem` itself.
impl MenuBarItemImpl {
    fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }
    fn set_submenu(&self, submenu: &MenuImpl) {
        self.ns.setSubmenu(Some(&submenu.ns));
    }
}

pub fn create_menu_bar_item() -> MenuBarItemImpl {
    let m = mtm();
    let ns = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(m.alloc::<NSMenuItem>(), &NSString::from_str(""), None, &NSString::from_str(""))
    };
    MenuBarItemImpl { ns }
}

/// The whole top menu bar, installed via `Window::set_menu_bar`.
#[derive(Clone)]
pub struct MenuBarImpl {
    ns: Retained<NSMenu>,
}

/// `add_item`/`remove_item` mirror `MenuImpl`'s own: real `NSMenu.addItem(_:)`/`.removeItem(_:)`
/// calls, reachable post-construction for `elwindui-backend-appkit::builtins::MenuBarImpl::
/// set_children`. Plain inherent methods — see `MenuImpl`'s own `add_item`/`remove_item` doc
/// comment for why this raw layer doesn't implement `elwindui_core::ui::MenuBar` itself.
impl MenuBarImpl {
    fn add_item(&self, item: &MenuBarItemImpl) {
        self.ns.addItem(&item.ns);
    }
    fn remove_item(&self, item: &MenuBarItemImpl) {
        self.ns.removeItem(&item.ns);
    }
}

pub fn create_menu_bar() -> MenuBarImpl {
    let m = mtm();
    let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));

    // macOS convention: `mainMenu`'s *first* item is always displayed as the bold app name
    // (whatever title it's given is ignored/overridden by the OS) and its submenu is "the app
    // menu". Without one, the DSL's first real top-level item (e.g. "File") gets silently
    // absorbed into that slot instead of showing up as its own menu — so this app-menu slot,
    // with at minimum a working Quit item, is provided here rather than asked of the DSL
    // author, since it's a platform detail of `NSApp.mainMenu`, not something 付録X's
    // `MenuBarImpl`/`MenuBarItemImpl` DSL shape should need to know about.
    let app_menu_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(m.alloc::<NSMenuItem>(), &NSString::from_str(""), None, &NSString::from_str(""))
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
    MenuBarImpl { ns }
}

/// See docs/elwindui_spec.md 付録T.2. Modal file panels (`runModal`) are themselves synchronous
/// (they block until the user closes the panel), so these `async fn`s never actually suspend —
/// they resolve on the first poll. That's enough for `#[command(async)]` bodies that just need to
/// `.await` a dialog result; it is not a general-purpose async executor (nothing here can yield
/// across a real I/O wait), which is what `elwindui-core`'s planned `Dispatcher`/`spawn`
/// (docs/elwindui_gui_framework_design.md §7.3) is for.
pub mod platform {
    pub mod file_dialog {
        use crate::mtm;
        use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
        use std::path::PathBuf;

        pub async fn open() -> Option<PathBuf> {
            let panel = NSOpenPanel::openPanel(mtm());
            if panel.runModal() != NSModalResponseOK {
                return None;
            }
            panel.URL().and_then(|url| url.path()).map(|p| PathBuf::from(p.to_string()))
        }

        pub async fn save() -> Option<PathBuf> {
            let panel = NSSavePanel::savePanel(mtm());
            if panel.runModal() != NSModalResponseOK {
                return None;
            }
            panel.URL().and_then(|url| url.path()).map(|p| PathBuf::from(p.to_string()))
        }
    }
}

/// AppKit's `Dispatcher` (docs/elwindui_spec.md 付録P.5): hops back to the main thread via GCD's
/// main queue, which `NSApplication.run()` (`application::run()` below) actively services as part
/// of its own event loop — so a job enqueued from any thread (a background `tokio` task
/// completing, say) is guaranteed to run promptly. See `elwindui_core::task` for how this lets a
/// suspended `#[command(async)]` body resume back on the UI thread, the same role C#'s
/// `SynchronizationContext.Post` plays.
pub struct AppKitDispatcher;

impl elwindui_core::task::Dispatcher for AppKitDispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        dispatch2::DispatchQueue::main().exec_async(job);
    }
}

thread_local! {
    /// `NSApplication.delegate` is an unretained (weak) reference (same situation as
    /// `TextAreaImpl::delegate_storage`), so this keeps it alive for the process's lifetime.
    static APP_DELEGATE: RefCell<Option<Retained<AppDelegate>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        /// Without this, AppKit's default behavior leaves the process running after the last
        /// (only, for `notepad`) window is closed via its close button.
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// The single entry point that owns "enter the platform event loop" — kept separate from
/// `Window::show()` so that there's one well-defined place to install the task executor (see
/// `elwindui_core::task::set_current`) and the app delegate before any generated code runs. Call
/// once, after showing the app's window(s).
pub mod application {
    use super::{mtm, AppDelegate, AppKitDispatcher, APP_DELEGATE};
    use elwindui_core::task::LocalExecutor;
    use objc2_app_kit::NSApplication;

    /// Blocking: enters the AppKit main event loop.
    pub fn run() {
        elwindui_core::task::set_current(LocalExecutor::new(AppKitDispatcher));

        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        let delegate = AppDelegate::new(mtm);
        app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
        APP_DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));

        app.run();
    }
}
