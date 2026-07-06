//! AppKit implementation of the widget surface `elwindui-codegen` targets for the `notepad`
//! example. See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! Only genuinely native leaf widgets (`Window`/`TextArea`/`Button`/`MenuBar`/`TabView`, the
//! "NativeComponent" family — see docs/elwindui_spec.md 付録E) have a Rust struct here at all.
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have none: they're
//! `elwindui_core::tree::UIElement` values that `elwindui-codegen` builds directly, reflected into
//! real `NSView`s/`CAShapeLayer`s/`CATextLayer`s by `TreeHostView` below (used by both `Window`'s
//! content view and `TabView`'s per-tab content area).

#![cfg(target_os = "macos")]

use elwindui_core::tree::{layout_tree, PaintKind, ShapeKind, UIElement};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSButton, NSMenu, NSMenuItem, NSScrollView, NSStackView, NSTextDelegate, NSTextView,
    NSTextViewDelegate, NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSRect, NSString};
use objc2_quartz_core::{CALayer, CAShapeLayer, CATextLayer};
use std::cell::RefCell;
use std::rc::Rc;

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("elwindui-backend-appkit must run on the main thread")
}

/// Everything the generated code can pass as a `Window`/`TabView` child.
/// `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse` have no variant here at all — they're
/// purely `elwindui_core::tree::Node::Virtual` values now (see `TreeHostView` below), never a real
/// widget of their own.
#[derive(Clone)]
pub enum AnyView {
    TextArea(TextArea),
    Button(Button),
    TabView(TabView),
}

impl AnyView {
    fn as_nsview(&self) -> Retained<NSView> {
        match self {
            AnyView::TextArea(v) => Retained::into_super(v.scroll.clone()),
            AnyView::Button(v) => {
                let control: Retained<objc2_app_kit::NSControl> = Retained::into_super(v.ns.clone());
                let view: Retained<NSView> = Retained::into_super(control);
                view
            }
            AnyView::TabView(v) => Retained::into_super(v.root.clone()),
        }
    }
}

/// Lets `TreeHostView` (below) measure/arrange any native leaf uniformly through the base `NSView`
/// API (`fittingSize`/`setFrame`) regardless of which concrete widget it wraps — no per-widget
/// (`Text`, `Button`, ...) `LayoutNode` impl needed. See docs/elwindui_spec.md 付録H.2.
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

impl From<TextArea> for AnyView {
    fn from(v: TextArea) -> Self {
        AnyView::TextArea(v)
    }
}
impl From<Button> for AnyView {
    fn from(v: Button) -> Self {
        AnyView::Button(v)
    }
}
impl From<TabView> for AnyView {
    fn from(v: TabView) -> Self {
        AnyView::TabView(v)
    }
}

#[derive(Clone)]
pub struct Window {
    ns: Retained<NSWindow>,
    content_host: Retained<TreeHostView>,
}

impl Window {
    pub fn new(title: &str) -> Self {
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
        ns.setTitle(&NSString::from_str(title));
        let content_host = TreeHostView::new();
        ns.setContentView(Some(&content_host));
        Self { ns, content_host }
    }

    /// Replaces the window's whole content tree — see `TreeHostView` for how a `Box<dyn
    /// UIElement>` (layouts/shapes/text mixed freely with native controls, at any nesting depth)
    /// gets reflected into real `NSView` subviews and `CAShapeLayer`/`CATextLayer` sublayers.
    pub fn set_content(&self, content: Box<dyn UIElement>) {
        self.content_host.set_tree(content);
    }

    pub fn set_title(&self, title: &str) {
        self.ns.setTitle(&NSString::from_str(title));
    }

    /// Sets `NSApplication.mainMenu` (macOS has one global top menu bar, not a per-window one).
    pub fn set_menu_bar(&self, menu_bar: &MenuBar) {
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

/// The single reusable "reflect a `Box<dyn elwindui_core::tree::UIElement>` into real `NSView`
/// subviews/`CAShapeLayer`/`CATextLayer` sublayers" host, replacing the old per-container
/// `StackLayoutView` (`VerticalLayout`/`HorizontalLayout`) — since `VerticalLayout`/
/// `HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` are now all just `UIElement` values with
/// no backend struct of their own (docs/elwindui_spec.md 付録H.2), one host type is all any native
/// container needs to accept arbitrary content: `Window`'s content view and `TabView`'s per-tab
/// content area both are one of these.
struct TreeHostIvars {
    tree: RefCell<Option<Box<dyn UIElement>>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = TreeHostIvars]
    struct TreeHostView;

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
        /// outer `NSStackView` (`TabView`'s content area sits in one) — sees something meaningful
        /// instead of AppKit's zero-size default for a plain, constraint-free `NSView`.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> objc2_foundation::NSSize {
            let size = self
                .ivars()
                .tree
                .borrow()
                .as_ref()
                .map(|tree| elwindui_core::tree::natural_size(&**tree))
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
        let ivars = TreeHostIvars { tree: RefCell::new(None) };
        let this = Self::alloc(m).set_ivars(ivars);
        unsafe { msg_send![super(this), initWithFrame: NSRect::default()] }
    }

    /// Replaces this host's entire content, discarding whatever native subviews were there before
    /// — a full swap rather than a diff, matching how `TabView` swaps its content area wholesale
    /// between tabs (see `TabView::set_content`) and how `Window::set_content` is only ever called
    /// once (a `Box<dyn UIElement>` isn't `Clone`, so it can only be handed over once anyway).
    fn set_tree(&self, tree: Box<dyn UIElement>) {
        for old in self.subviews().iter() {
            old.removeFromSuperview();
        }
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
        let (natives, paints): (Vec<(AnyView, elwindui_core::layout::Rect)>, _) =
            layout_tree(&**tree, available);

        for (mut view, rect) in natives {
            let nsview = view.as_nsview();
            self.addSubview(&nsview);
            // Every native leaf here is positioned purely by manual `setFrame` (via `arrange`
            // below), never by Auto Layout constraints — but a plain `addSubview:` leaves
            // `translatesAutoresizingMaskIntoConstraints` at its *class* default, which for an
            // Auto-Layout-authored view like `NSStackView` (`TabView`'s root) is `NO`. With no
            // explicit size constraints of its own, such a view gets silently resized back down
            // to its intrinsic content size on the next layout pass — undoing `arrange`'s
            // `setFrame` entirely. Forcing this back to `YES` opts every leaf out of the
            // constraint system so our manual frame actually sticks.
            nsview.setTranslatesAutoresizingMaskIntoConstraints(true);
            view.arrange(rect);
        }

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
        // `addSublayer` (append, not `insertSublayer_atIndex(_, 0)`/prepend): `paints` is in
        // parent-before-children traversal order (see `tree::arrange`), so appending keeps a
        // parent's own background behind any child content painted after it (e.g. a `Rectangle`'s
        // fill staying behind its `TextBlock` child) instead of the two swapping z-order.
        for (paint, rect) in paints {
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
                PaintKind::Text { content, color } => {
                    let text_layer = CATextLayer::new();
                    text_layer.setName(Some(&NSString::from_str("elwindui-paint")));
                    text_layer.setFrame(cg_rect);
                    text_layer.setFontSize(14.0);
                    text_layer.setForegroundColor(Some(&parse_color(color.as_deref().unwrap_or("#000000"))));
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

#[derive(Clone)]
pub struct TextArea {
    scroll: Retained<NSScrollView>,
    text_view: Retained<NSTextView>,
    delegate_storage: Rc<RefCell<Option<Retained<TextViewDelegate>>>>,
}

impl TextArea {
    pub fn new(initial_text: &str) -> Self {
        let m = mtm();
        let scroll = NSTextView::scrollableTextView(m);
        let text_view = scroll
            .documentView()
            .expect("scrollableTextView always has a document view")
            .downcast::<NSTextView>()
            .expect("scrollableTextView's document view is an NSTextView");
        text_view.setString(&NSString::from_str(initial_text));
        Self { scroll, text_view, delegate_storage: Rc::new(RefCell::new(None)) }
    }

    pub fn set_text(&self, text: &str) {
        self.text_view.setString(&NSString::from_str(text));
    }

    /// `NSTextView.delegate` is an unretained (weak) reference, so the delegate this creates is
    /// only kept alive by `self.delegate_storage`. Since `TextArea` derives `Clone` by sharing
    /// that `Rc`, the delegate survives as long as *any* clone of this `TextArea` value does —
    /// but if every clone is dropped (e.g. a caller extracts just the raw `NSView` and discards
    /// the `TextArea` struct itself), the delegate is deallocated and `on_change` silently stops
    /// firing, even though typed characters keep appearing on screen (native `NSTextView`
    /// rendering needs no delegate). Found via `TabView`'s content pane, which used to do exactly
    /// that — see `TabView::set_content`'s doc comment.
    pub fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        let m = mtm();
        let ivars = TextDelegateIvars { text_view: self.text_view.clone(), callback };
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

#[derive(Clone)]
pub struct Button {
    ns: Retained<NSButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

impl Button {
    pub fn new(title: &str) -> Self {
        let m = mtm();
        let ns = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, m)
        };
        Self { ns, target_storage: Rc::new(RefCell::new(None)) }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    pub fn set_on_click(&self, callback: Box<dyn Fn()>) {
        let target = ButtonTarget::new(ButtonTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }

    /// Used by `TabChip` to rename a tab's title button when its document's file name changes.
    pub fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
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
/// select) plus a small close button, packed into one row so `TabStrip` can insert/remove it as
/// one unit.
#[derive(Clone)]
pub struct TabChip {
    ns: Retained<NSStackView>,
    pub title_button: Button,
    pub close_button: Button,
}

impl TabChip {
    fn new(title: &str) -> Self {
        let title_button = Button::new(title);
        let close_button = Button::new("×");
        let ns = new_stack(
            vec![AnyView::Button(title_button.clone()), AnyView::Button(close_button.clone())],
            NSUserInterfaceLayoutOrientation::Horizontal,
        );
        Self { ns, title_button, close_button }
    }

    pub fn set_title(&self, title: &str) {
        self.title_button.set_text(title);
    }
}

/// The row of `TabChip`s plus a trailing "+" button. `TabView` owns one of these and the content
/// area below it; kept as a separate type since 付録Y's backend table describes it as its own
/// piece (a custom `NSStackView`-based strip, not `NSTabViewController`).
#[derive(Clone)]
pub struct TabStrip {
    ns: Retained<NSStackView>,
    pub new_tab_button: Button,
}

impl TabStrip {
    fn new() -> Self {
        let new_tab_button = Button::new("+");
        let ns = new_stack(
            vec![AnyView::Button(new_tab_button.clone())],
            NSUserInterfaceLayoutOrientation::Horizontal,
        );
        Self { ns, new_tab_button }
    }

    /// Inserts a chip before the "+" button, at arranged-subview position `index`.
    fn insert_tab(&self, index: usize, title: &str) -> TabChip {
        let chip = TabChip::new(title);
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.insertArrangedSubview_atIndex(&view, index as isize);
        chip
    }

    fn remove_tab(&self, chip: &TabChip) {
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.removeArrangedSubview(&view);
        view.removeFromSuperview();
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y. Vertical stack of `[TabStrip, content_area]`; the
/// generated code (see `elwindui-codegen`'s specialized `TabView` codegen path) owns the mapping
/// from the observable tab list to `TabChip`s and calls `set_content` whenever the active tab
/// changes. This type only holds the two widget areas — it has no notion of "the list of tabs" on
/// its own, matching how `VerticalLayout`/`HorizontalLayout` (purely `elwindui_core::tree::Node::Virtual`
/// values) don't know about the data their children came from either.
#[derive(Clone)]
pub struct TabView {
    root: Retained<NSStackView>,
    pub strip: TabStrip,
    // A `TreeHostView`, not a plain `NSStackView`, since a tab's content (e.g. `DocumentView`,
    // whose root is virtual) is a `Box<dyn UIElement>`, not a single `AnyView` — see
    // `set_content`. Its
    // own `tree` ivar is what now keeps any native leaf's retention concern alive (a `TextArea`'s
    // change-notification delegate only stays alive as long as *some* clone of that `TextArea`
    // value does, see `TextArea::set_on_change`'s doc comment) for as long as it's the visible tab.
    content_area: Retained<TreeHostView>,
}

impl TabView {
    pub fn new() -> Self {
        let strip = TabStrip::new();
        let content_area = TreeHostView::new();
        let strip_view: Retained<NSView> = Retained::into_super(strip.ns.clone());
        let content_view: Retained<NSView> = Retained::into_super(content_area.clone());
        let root = NSStackView::stackViewWithViews(
            &objc2_foundation::NSArray::from_retained_slice(&[strip_view, content_view]),
            mtm(),
        );
        root.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        // `NSStackView`'s default `distribution` (`GravityAreas`) leaves each arranged subview at
        // its own intrinsic size unless hugging priorities say otherwise — `.Fill` makes the stack
        // actually consume its *entire* stacking-axis extent, matching `TabView`'s expected "chips
        // row at natural height, content area fills the rest" shape. `content_area`'s own vertical
        // hugging priority is dropped to (near-)zero so it — not the also-low-priority-by-default
        // `strip` — is the one that absorbs whatever space `Fill` distributes, since a
        // `TreeHostView`'s `intrinsicContentSize` (its hosted tree's *natural*, often tiny, size —
        // e.g. an empty `TextArea`'s `fittingSize()`) is never a meaningful hint for how much room
        // it should actually get.
        content_area.setContentHuggingPriority_forOrientation(1.0, objc2_app_kit::NSLayoutConstraintOrientation::Vertical);
        root.setDistribution(objc2_app_kit::NSStackViewDistribution::Fill);
        Self { root, strip, content_area }
    }

    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.strip.new_tab_button.set_on_click(callback);
    }

    /// Inserts a new tab chip at `index`, wiring `on_select`/`on_close` to the given callbacks.
    pub fn insert_tab(
        &self,
        index: usize,
        title: &str,
        on_select: Box<dyn Fn()>,
        on_close: Box<dyn Fn()>,
    ) -> TabChip {
        let chip = self.strip.insert_tab(index, title);
        chip.title_button.set_on_click(on_select);
        chip.close_button.set_on_click(on_close);
        chip
    }

    pub fn remove_tab(&self, chip: &TabChip) {
        self.strip.remove_tab(chip);
    }

    /// Swaps the single visible document pane for the currently selected tab.
    pub fn set_content(&self, content: Box<dyn UIElement>) {
        self.content_area.set_tree(content);
    }
}

/// See docs/elwindui_builtins_spec.md 付録X. A single application-wide `NSMenu` (top menu bar
/// item / `File`, `Edit`, ...), reusing `MenuItem` for its leaf entries.
#[derive(Clone)]
pub struct MenuItem {
    ns: Retained<NSMenuItem>,
    target_storage: Rc<RefCell<Option<Retained<MenuItemTarget>>>>,
}

impl MenuItem {
    pub fn new(title: &str) -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        Self { ns, target_storage: Rc::new(RefCell::new(None)) }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`); macOS defaults a menu item's modifier mask to Cmd,
    /// which matches the common `Cmd+<letter>` shortcuts notepad needs (付録K.2's platform
    /// conversion rule already reads "Ctrl" as "Cmd" on macOS at the DSL level).
    pub fn set_shortcut(&self, key_equivalent: &str) {
        self.ns.setKeyEquivalent(&NSString::from_str(key_equivalent));
    }

    pub fn set_on_select(&self, callback: Box<dyn Fn()>) {
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
/// that way here, but the same type covers both).
#[derive(Clone)]
pub struct Menu {
    ns: Retained<NSMenu>,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        for item in &items {
            ns.addItem(&item.ns);
        }
        Self { ns }
    }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `Menu`.
#[derive(Clone)]
pub struct MenuBarItem {
    ns: Retained<NSMenuItem>,
}

impl MenuBarItem {
    pub fn new(title: &str, submenu: Menu) -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        ns.setSubmenu(Some(&submenu.ns));
        Self { ns }
    }
}

/// The whole top menu bar, installed via `Window::set_menu_bar`.
#[derive(Clone)]
pub struct MenuBar {
    ns: Retained<NSMenu>,
}

impl MenuBar {
    pub fn new(items: Vec<MenuBarItem>) -> Self {
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

        for item in &items {
            ns.addItem(&item.ns);
        }
        Self { ns }
    }
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
    /// `TextArea::delegate_storage`), so this keeps it alive for the process's lifetime.
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
