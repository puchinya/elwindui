//! WinUI 3 (Windows App SDK) implementation of the widget surface `elwindui-codegen` targets,
//! mirroring `elwindui-backend-appkit`'s shape (see that crate's doc comment for the overall
//! native-vs-virtual design this implements: `VerticalLayout`/`HorizontalLayout`/
//! `Rectangle`/`Ellipse`/`TextBlock` have no widget here at all, just `elwindui_core::ui::UIElement`
//! values `elwindui-codegen` builds directly (`TextBlock` is self-drawn, using the real XAML
//! `TextBlock` class only as a paint primitive inside `TreeHostPanel::relayout_static`, never as a
//! wrapped builtin widget — see `elwindui-backend-appkit`'s `CATextLayer` use for the same role);
//! only `Window`/`ButtonImpl`/`TextAreaImpl`/`MenuBarImpl`/`MenuBarItemImpl`/`MenuImpl`/`MenuItemImpl`/`NativeTabViewImpl` are real
//! native widgets).
//!
//! # UNVERIFIED — read before touching
//!
//! Written entirely without a Windows machine available in this environment to build or run it
//! against. The `elwindui_backend_appkit`-mirroring *structure* (which types exist, what methods
//! they expose, how `TreeHostPanel` reflects an `Rc<dyn UIElement>`) is deliberate and should be
//! sound;
//! the *exact* WinRT/`windows-rs` call shapes (event-handler registration syntax, exact property/
//! method names on `Microsoft.UI.Xaml` types, `build.rs`'s bindgen invocation) are written from
//! memory of the general `windows-rs` WinRT-projection pattern and are the most likely things to
//! need correction once this is actually compiled on Windows with the Windows App SDK installed.

#![cfg(target_os = "windows")]

#[allow(non_snake_case, non_camel_case_types, dead_code, clippy::all)]
mod bindings {
    include!(env!("ELWINDUI_WINUI3_BINDINGS"));
}

use bindings::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueHandler};
use bindings::Microsoft::UI::Windowing::AppWindow;
use bindings::Microsoft::UI::Xaml::Controls::{
    Button as XamlButton, Canvas, MenuBar as XamlMenuBar, MenuFlyoutItem, MenuFlyoutItemBase, TabView as XamlTabView,
    TabViewCloseButtonOverlayMode, TabViewItem, TabViewTabCloseRequestedEventArgs, TextBlock, TextBox,
};
use bindings::Microsoft::UI::Xaml::Input::KeyboardAccelerator;
use bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
use bindings::Microsoft::UI::Xaml::Shapes::{Ellipse as XamlEllipse, Rectangle as XamlRectangle};
use bindings::Microsoft::UI::Xaml::{
    FrameworkElement, RoutedEventHandler, SelectionChangedEventArgs, TextChangedEventArgs, UIElement, Window as XamlWindow,
};
use bindings::Windows::Foundation::{Size, TypedEventHandler};
use bindings::Windows::Graphics::{PointInt32, SizeInt32};
use bindings::Windows::UI::Color;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use windows::core::{Interface, Result, HSTRING};

// `elwindui_core::ui::UIElement` (not `use`d by name — `UIElement` above is XAML's own native type)
// brought into scope anonymously so `.as_ui_element()` dot-call syntax resolves on `Rc<dyn
// elwindui_core::ui::UIElement>`/composed values, matching the `Button as _`/`TextArea as _`-style
// anonymous imports `elwindui-backend-appkit` already uses for this exact purpose.
use elwindui_core::ui::UIElement as _;

/// The capability a type needs to be usable as an `AnyView` — implemented once per raw XAML element
/// type (`TextBox`/`XamlButton`/`XamlTabView`) instead of matched on centrally, so a future native
/// leaf (`Dialog`, `VirtualList`, ...) only needs its own `impl WinUiHandle`, never a change to
/// `AnyView` itself or to any `match` over it — mirrors `elwindui-backend-appkit`'s `AppKitHandle`
/// (see that trait's own doc comment for the rationale).
///
/// Implemented on the raw XAML element type itself (a foreign type — allowed since `WinUiHandle` is
/// a local trait) rather than on `TextAreaImpl`/`ButtonImpl`/`NativeTabViewImpl`, since those now each
/// compose an `elwindui_core::ui::NativeControlImpl<AnyView>` as their own `base` field
/// (docs/elwindui_spec.md 付録H.2.1a) — an `AnyView` wrapping the not-yet-fully-constructed widget
/// itself would be a self-reference. Wrapping just the raw element instead lets `base.handle` be
/// built (`AnyView::from(xaml.clone())`) before the rest of the widget struct exists.
trait WinUiHandle: elwindui_core::ui::AsAny {
    fn as_element(&self) -> FrameworkElement;
}

impl WinUiHandle for TextBox {
    fn as_element(&self) -> FrameworkElement {
        self.clone().into()
    }
}
impl WinUiHandle for XamlButton {
    fn as_element(&self) -> FrameworkElement {
        self.clone().into()
    }
}
impl WinUiHandle for XamlTabView {
    fn as_element(&self) -> FrameworkElement {
        self.clone().into()
    }
}

/// Everything the generated code can pass as a `Window`/`NativeTabViewImpl` child.
/// `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no variant here —
/// they're purely `elwindui_core::ui::UIElement` values (see `TreeHostPanel` below). An
/// `Rc<dyn WinUiHandle>` (not a closed `enum`) so adding a new native leaf never requires touching
/// this type — see `WinUiHandle`'s own doc comment.
#[derive(Clone)]
pub struct AnyView(Rc<dyn WinUiHandle>);

impl AnyView {
    fn as_element(&self) -> FrameworkElement {
        self.0.as_element()
    }
}

impl AnyView {
    /// Lets each `elwindui_core::ui::NativeControl<AnyView>`-derived widget's own `measure_override`
    /// (`TextArea`/`Button`/`TabView` below) measure itself uniformly through the base
    /// `FrameworkElement`/`UIElement` API regardless of which concrete widget it wraps — no per-widget
    /// re-implementation of the actual `Measure`/`DesiredSize` calls needed, just a one-line
    /// `measure_override` per widget that calls this. A plain inherent method, not a shared
    /// `elwindui-core`-defined trait — measuring a native handle is entirely backend-specific, so
    /// `elwindui_core::ui::NativeControl<H>` itself doesn't know how to do it (see that type's own doc
    /// comment).
    fn measure(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        let element = self.as_element();
        let _ = element.Measure(Size { Width: available.width as f32, Height: available.height as f32 });
        let desired = element.DesiredSize().unwrap_or(Size { Width: 0.0, Height: 0.0 });
        elwindui_core::base::Size { width: desired.Width, height: desired.Height }
    }

    /// Positions this native leaf — like `measure` above, a plain inherent method (elwindui-core's
    /// generic layout code never calls either) — called directly by `TreeHostPanel`'s own render loop
    /// below, once `layout_tree` has already handed back a concrete `RenderItem::Native(AnyView, ..)`.
    /// Unlike AppKit (where `arrange` calls `setFrame` directly), a `Canvas`'s children are still
    /// measured/arranged by the real XAML layout system on every layout pass — this only needs to set
    /// the `Width`/`Height` and `Canvas.Left`/`Canvas.Top` attached properties once; `Canvas`'s own
    /// (built-in) `ArrangeOverride` does the rest, unlike AppKit's plain `NSView` which has no
    /// attached-property positioning at all.
    fn arrange(&mut self, final_rect: elwindui_core::base::Rect) {
        let element = self.as_element();
        let _ = element.SetWidth(final_rect.width as f64);
        let _ = element.SetHeight(final_rect.height as f64);
        let _ = Canvas::SetLeft(&element, final_rect.x as f64);
        let _ = Canvas::SetTop(&element, final_rect.y as f64);
    }
}

impl<T: WinUiHandle + 'static> From<T> for AnyView {
    fn from(v: T) -> Self {
        AnyView(Rc::new(v))
    }
}

/// The single reusable "reflect an `Rc<dyn elwindui_core::ui::UIElement>` into real XAML
/// elements" host — the WinUI3 counterpart of `elwindui-backend-appkit`'s `TreeHostView`. A
/// `Canvas` needs no custom `MeasureOverride`/`ArrangeOverride` subclass (unlike `TreeHostView`'s
/// `NSView` subclass) since `Canvas`'s own built-in layout already just measures every child with
/// an unconstrained size and positions it from the `Canvas.Left`/`Canvas.Top` attached properties —
/// exactly the "trust `elwindui_core::ui::layout_tree`'s own absolute-rect computation, don't
/// let the native layout system second-guess it" behavior this needs. `Rectangle`/`Ellipse`/
/// `TextBlock` paint nodes become real `Shapes::Rectangle`/`Shapes::Ellipse`/`Controls::TextBlock`
/// elements appended to `Canvas.Children` in traversal order (`Canvas` z-orders by collection
/// order — a parent's own paint is appended before its children's, so it stays behind them),
/// rather than AppKit's separate `CAShapeLayer`/`CATextLayer` sublayer mechanism.
#[derive(Clone)]
pub struct TreeHostPanel {
    canvas: Canvas,
    tree: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElement>>>>,
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
/// docs/elwindui_spec.md H.2.3's `RelayoutHost` coalescing contract: repeated calls within the same
/// synchronous burst (e.g. several property setters inside one `resync()`) collapse into a single
/// deferred relayout pass, not one synchronous pass per call.
struct WinUI3RelayoutHost {
    canvas: Canvas,
    tree: Weak<RefCell<Option<Rc<dyn elwindui_core::ui::UIElement>>>>,
    /// `true` while a relayout pass is already enqueued on the `DispatcherQueue` and hasn't run
    /// yet — makes `request_relayout` a no-op for any further call until that pass actually runs
    /// (and clears it right before doing so).
    pending: Cell<bool>,
    /// Lets `request_relayout` (which only ever sees `&self`) hand an owned `Rc<Self>` to the
    /// `DispatcherQueueHandler` closure — set once, right after this host is `Rc`-wrapped (see
    /// `TreeHostPanel::set_tree`), the same self-referential-`Weak` pattern
    /// `elwindui_backend_appkit::builtins::tab_view::NativeTabViewImpl` uses for the same reason.
    weak_self: RefCell<Weak<WinUI3RelayoutHost>>,
}

impl elwindui_core::ui::RelayoutHost for WinUI3RelayoutHost {
    fn request_relayout(&self) {
        if self.pending.replace(true) {
            return; // already scheduled — the pending pass will pick up this call's changes too
        }
        let Some(this) = self.weak_self.borrow().upgrade() else {
            self.pending.set(false);
            return;
        };
        let Ok(queue) = DispatcherQueue::GetForCurrentThread() else {
            self.pending.set(false);
            return;
        };
        let _ = queue.TryEnqueue(&DispatcherQueueHandler::new(move || {
            this.pending.set(false);
            if let Some(tree) = this.tree.upgrade() {
                TreeHostPanel::relayout_static(&this.canvas, &tree);
            }
            Ok(())
        }));
    }
}

impl TreeHostPanel {
    pub fn new() -> Self {
        let canvas = Canvas::new().expect("Canvas::new");
        let this = Self { canvas, tree: Rc::new(RefCell::new(None)) };
        let weak = Rc::downgrade(&this.tree);
        let canvas_for_handler = this.canvas.clone();
        // `SizeChanged` fires whenever this panel's own allotted space changes (window resize,
        // or — for a `NativeTabViewImpl`'s per-tab content area — the tab strip/window resizing together)
        // — the same role `layout()` plays for AppKit's `TreeHostView`.
        let _ = this.canvas.SizeChanged(&TypedEventHandler::new(move |_, _| {
            if let Some(tree) = weak.upgrade() {
                Self::relayout_static(&canvas_for_handler, &tree);
            }
            Ok(())
        }));
        this
    }

    fn as_element(&self) -> FrameworkElement {
        self.canvas.clone().into()
    }

    /// Replaces this host's entire content, discarding whatever native children were there before
    /// — a full swap rather than a diff, matching `NativeTabViewImpl`'s wholesale content swap between tabs
    /// and `Window::set_content` only ever being called once (see `TreeHostView::set_tree`'s doc
    /// comment on the AppKit side for the same reasoning).
    pub fn set_tree(&self, tree: Rc<dyn elwindui_core::ui::UIElement>) {
        if let Ok(children) = self.canvas.Children() {
            let _ = children.Clear();
        }
        let host = Rc::new(WinUI3RelayoutHost {
            canvas: self.canvas.clone(),
            tree: Rc::downgrade(&self.tree),
            pending: Cell::new(false),
            weak_self: RefCell::new(Weak::new()),
        });
        *host.weak_self.borrow_mut() = Rc::downgrade(&host);
        tree.as_ui_element().set_invalidate_host(Some(host));
        *self.tree.borrow_mut() = Some(tree);
        Self::relayout_static(&self.canvas, &self.tree);
    }

    fn relayout_static(canvas: &Canvas, tree: &Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElement>>>>) {
        use elwindui_core::base::Size as LSize;

        let width = canvas.ActualWidth().unwrap_or(0.0) as f32;
        let height = canvas.ActualHeight().unwrap_or(0.0) as f32;
        let available = LSize { width, height };

        let tree_ref = tree.borrow();
        let Some(tree) = tree_ref.as_ref() else { return };
        let items: Vec<elwindui_core::ui::RenderItem<AnyView>> = elwindui_core::ui::layout_tree(tree, available);

        let Ok(children) = canvas.Children() else { return };

        // `items` is `layout_tree`'s single interleaved list, in `arrange`'s own parent-before-
        // children traversal order (see `RenderItem`'s doc comment) — replayed here in one pass so
        // `Children.Append` happens in the exact order encountered. `Canvas` z-orders by `Children`
        // collection order, so this makes document order the z-order for native leaves and
        // self-painted content alike (e.g. a `Rectangle`'s fill staying behind a `ButtonImpl` child
        // placed after it), instead of appending "all paints" then "all natives" as two separate
        // batches, which threw the relative ordering between the two away.
        for item in items {
            match item {
                elwindui_core::ui::RenderItem::Paint(paint, rect) => match paint {
                    elwindui_core::ui::PaintKind::Shape { kind, fill, stroke, stroke_width } => {
                        let element: UIElement = match kind {
                            elwindui_core::ui::ShapeKind::RoundedRect { corner_radius } => {
                                let r = XamlRectangle::new().expect("Rectangle::new");
                                let _ = r.SetRadiusX(corner_radius as f64);
                                let _ = r.SetRadiusY(corner_radius as f64);
                                r.into()
                            }
                            elwindui_core::ui::ShapeKind::Oval => XamlEllipse::new().expect("Ellipse::new").into(),
                        };
                        let fe: FrameworkElement = element.clone().into();
                        let _ = fe.SetWidth(rect.width as f64);
                        let _ = fe.SetHeight(rect.height as f64);
                        let _ = Canvas::SetLeft(&fe, rect.x as f64);
                        let _ = Canvas::SetTop(&fe, rect.y as f64);
                        if let Some(fill) = fill {
                            if let Ok(brush) = SolidColorBrush::CreateInstance(parse_color(&fill)) {
                                let _ = set_shape_fill(&element, &brush);
                            }
                        }
                        if let Some(stroke) = stroke {
                            if let Ok(brush) = SolidColorBrush::CreateInstance(parse_color(&stroke)) {
                                let _ = set_shape_stroke(&element, &brush, stroke_width as f64);
                            }
                        }
                        let _ = children.Append(&element);
                    }
                    elwindui_core::ui::PaintKind::Text { content, color, alignment } => {
                        // Uses the real XAML `TextBlock` class purely as a paint primitive
                        // (positioned manually via the same `Canvas.Left`/`Canvas.Top`/`Width`/
                        // `Height` convention as every shape above), never wrapped as a builtin
                        // widget with its own getter/setter surface — the WinUI3 counterpart of
                        // `elwindui-backend-appkit`'s `CATextLayer` use.
                        let text_block = TextBlock::new().expect("TextBlock::new");
                        let _ = text_block.SetText(&HSTRING::from(content));
                        if let Ok(brush) = SolidColorBrush::CreateInstance(parse_color(color.as_deref().unwrap_or("#000000"))) {
                            let _ = text_block.SetForeground(&brush);
                        }
                        let _ = text_block.SetTextAlignment(xaml_text_alignment(alignment));
                        let fe: FrameworkElement = text_block.into();
                        let _ = fe.SetWidth(rect.width as f64);
                        let _ = fe.SetHeight(rect.height as f64);
                        let _ = Canvas::SetLeft(&fe, rect.x as f64);
                        let _ = Canvas::SetTop(&fe, rect.y as f64);
                        let _ = children.Append(&fe);
                    }
                },
                // The third element (each native's own `Rc<dyn UIElement>` tree node) is what
                // AppKit's `TreeHostView::relayout` uses to wire routed-event dispatch
                // (docs/elwindui_spec.md 4章, `#[routed]`) — not done here, since this WinUI3
                // backend is spec-only/best-effort and unverified (see this crate's own top doc
                // comment); real click wiring is AppKit-only for now.
                elwindui_core::ui::RenderItem::Native(mut view, rect, _node) => {
                    view.arrange(elwindui_core::base::Rect { x: rect.x, y: rect.y, width: rect.width, height: rect.height });
                    let _ = children.Append(&view.as_element());
                }
            }
        }
    }
}

/// `Rectangle`/`Ellipse` share `Fill`/`Stroke`/`StrokeThickness` (declared on their common
/// `Shape` base class), but the generated bindings likely expose them per-concrete-type rather
/// than through a `Shape` handle directly usable here — dispatching on the `UIElement` avoids
/// needing a third enum just for this.
fn set_shape_fill(element: &UIElement, brush: &SolidColorBrush) -> Result<()> {
    if let Ok(r) = element.cast::<XamlRectangle>() {
        return r.SetFill(brush);
    }
    element.cast::<XamlEllipse>()?.SetFill(brush)
}

fn set_shape_stroke(element: &UIElement, brush: &SolidColorBrush, thickness: f64) -> Result<()> {
    if let Ok(r) = element.cast::<XamlRectangle>() {
        r.SetStroke(brush)?;
        return r.SetStrokeThickness(thickness);
    }
    let e = element.cast::<XamlEllipse>()?;
    e.SetStroke(brush)?;
    e.SetStrokeThickness(thickness)
}

/// Parses a `"#RRGGBB"`/`"#RRGGBBAA"` hex color (the only form `Rectangle`/`Ellipse`'s `fill`/
/// `stroke` params accept) into a `Windows::UI::Color`. An unparseable string falls back to opaque
/// black rather than panicking, since this runs during layout, not construction.
fn parse_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    let (r, g, b, a) = match (hex.len(), u32::from_str_radix(hex, 16)) {
        (6, Ok(v)) => (((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8, 255u8),
        (8, Ok(v)) => (((v >> 24) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8),
        _ => (0, 0, 0, 255),
    };
    Color { A: a, R: r, G: g, B: b }
}

/// `elwindui_core::ui::TextAlignment` -> `Microsoft.UI.Xaml.TextAlignment`.
fn xaml_text_alignment(alignment: elwindui_core::ui::TextAlignment) -> bindings::Microsoft::UI::Xaml::TextAlignment {
    match alignment {
        elwindui_core::ui::TextAlignment::Left => bindings::Microsoft::UI::Xaml::TextAlignment::Left,
        elwindui_core::ui::TextAlignment::Center => bindings::Microsoft::UI::Xaml::TextAlignment::Center,
        elwindui_core::ui::TextAlignment::Right => bindings::Microsoft::UI::Xaml::TextAlignment::Right,
    }
}

#[derive(Clone)]
pub struct Window {
    xaml: XamlWindow,
    content_host: TreeHostPanel,
}

impl Window {
    pub fn new() -> Self {
        let xaml = XamlWindow::new().expect("Window::new");
        let content_host = TreeHostPanel::new();
        let _ = xaml.SetContent(&content_host.as_element());
        Self { xaml, content_host }
    }

    /// Replaces the window's whole content tree — see `TreeHostPanel` for how an `Rc<dyn
    /// UIElement>` (layouts/shapes/text mixed freely with native controls, at any nesting depth)
    /// gets reflected into real XAML elements.
    pub fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElement>) {
        self.content_host.set_tree(content);
    }

    pub fn set_title(&self, title: &str) {
        let _ = self.xaml.SetTitle(&HSTRING::from(title));
    }

    /// `Microsoft.UI.Xaml.Controls.MenuBar` is placed as a real element *above* the content host,
    /// unlike AppKit's single global `NSApplication.mainMenu` — this repacks `Window`'s content
    /// into a two-row layout (`MenuBarImpl`, then the existing content host) the first time a menu bar
    /// is set. `VerticalLayout`/`HorizontalLayout` aren't available here (no backend struct — see
    /// the module doc comment), so this uses a plain `Canvas`-less stack: a small dedicated host `Grid` with two
    /// rows would be the idiomatic XAML way to do this; simplified here to stacking two elements
    /// inside a fresh outer `Canvas` sized/positioned manually, mirroring `TreeHostPanel`'s own
    /// "don't trust native auto-layout, position everything explicitly" approach.
    pub fn set_menu_bar(&self, menu_bar: &MenuBarImpl) {
        let outer = Canvas::new().expect("Canvas::new");
        if let Ok(children) = outer.Children() {
            let _ = children.Append(&menu_bar.xaml);
            let _ = children.Append(&self.content_host.as_element());
            let _ = Canvas::SetTop(&self.content_host.as_element(), 32.0);
        }
        let _ = self.xaml.SetContent(&outer);
    }

    /// Shows the window. Does not block — call `application::run()` afterward to actually enter
    /// the platform message loop (see that module's doc comment for why the two are separate).
    pub fn show(&self) {
        let _ = self.xaml.Activate();
    }

    /// `Window.AppWindow` (Windows App SDK 1.3+) already handles the `WinRT.Interop.WindowNative`/
    /// `Win32Interop.GetWindowIdFromWindow` dance internally, so no manual interop is needed here.
    fn app_window(&self) -> Option<AppWindow> {
        self.xaml.AppWindow().ok()
    }

    /// WinUI3's `AppWindow.Position.X`/`.Y` and `AppWindow.Size.Width`/`.Height` are already
    /// top-left-origin, Y increasing downward — unlike `elwindui-backend-appkit`'s `Window`, no
    /// coordinate conversion is needed here. `None` (no `AppWindow` yet, e.g. before the window has
    /// ever been shown) reads back as `0.0`.
    pub fn left(&self) -> f32 {
        self.app_window().and_then(|w| w.Position().ok()).map(|p| p.X as f32).unwrap_or(0.0)
    }

    pub fn set_left(&self, left: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(position) = app_window.Position() {
                let _ = app_window.Move(PointInt32 { X: left as i32, Y: position.Y });
            }
        }
    }

    pub fn top(&self) -> f32 {
        self.app_window().and_then(|w| w.Position().ok()).map(|p| p.Y as f32).unwrap_or(0.0)
    }

    pub fn set_top(&self, top: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(position) = app_window.Position() {
                let _ = app_window.Move(PointInt32 { X: position.X, Y: top as i32 });
            }
        }
    }

    pub fn width(&self) -> f32 {
        self.app_window().and_then(|w| w.Size().ok()).map(|s| s.Width as f32).unwrap_or(0.0)
    }

    pub fn set_width(&self, width: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(size) = app_window.Size() {
                let _ = app_window.Resize(SizeInt32 { Width: width as i32, Height: size.Height });
            }
        }
    }

    pub fn height(&self) -> f32 {
        self.app_window().and_then(|w| w.Size().ok()).map(|s| s.Height as f32).unwrap_or(0.0)
    }

    pub fn set_height(&self, height: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(size) = app_window.Size() {
                let _ = app_window.Resize(SizeInt32 { Width: size.Width, Height: height as i32 });
            }
        }
    }
}

#[elwindui_macros::class(implements = elwindui_core::ui::TextArea, inherits = elwindui_core::ui::NativeControl<AnyView>)]
pub struct TextArea {
    text_box: TextBox,
    on_change: Rc<RefCell<Option<Box<dyn Fn(String)>>>>,
}

/// `elwindui_core::ui::TextArea`'s shape is common to every backend (docs/elwindui_spec.md
/// 付録H.2.1a) — see that trait's own doc comment; only these method bodies are WinUI3-specific.
#[elwindui_macros::class]
impl TextArea {
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.base.handle.measure(available)
    }

    /// `TextBox.Text` assigned programmatically resets the caret/selection to the start, even when
    /// the text given is identical to what's already there — same issue as AppKit's
    /// `NSTextView.setString:` (see that backend's own `TextAreaImpl::set_text` doc comment for the
    /// full rationale). The two-way `#[two_way] text` binding re-syncs *every* bound field on
    /// *every* model change, including the one this exact edit just caused, so without this guard
    /// typing a single character would immediately call this with that same character already
    /// applied, yanking the caret away mid-keystroke.
    fn set_text(&self, text: &str) {
        let current = self.text_box.Text().map(|s| s.to_string_lossy()).unwrap_or_default();
        if current == text {
            return;
        }
        let _ = self.text_box.SetText(&HSTRING::from(text));
    }

    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }

    fn new() -> Self {
        let text_box = TextBox::new().expect("TextBox::new");
        let _ = text_box.SetAcceptsReturn(true);
        let _ = text_box.SetTextWrapping(bindings::Microsoft::UI::Xaml::TextWrapping::Wrap);
        let handle = AnyView::from(text_box.clone());
        let this = Self {
            base: elwindui_core::ui::create_native_control(handle),
            text_box,
            on_change: Rc::new(RefCell::new(None)),
        };
        let callback = this.on_change.clone();
        let text_box_for_handler = this.text_box.clone();
        let _ = this.text_box.TextChanged(&TypedEventHandler::<TextBox, TextChangedEventArgs>::new(move |_, _| {
            if let Some(cb) = callback.borrow().as_ref() {
                let text = text_box_for_handler.Text().map(|s| s.to_string_lossy()).unwrap_or_default();
                cb(text);
            }
            Ok(())
        }));
        this
    }
}

pub fn create_text_area() -> TextAreaImpl {
    TextAreaImpl::new()
}

#[elwindui_macros::class(implements = elwindui_core::ui::Button, inherits = elwindui_core::ui::NativeControl<AnyView>)]
pub struct Button {
    xaml: XamlButton,
    on_click: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

/// `elwindui_core::ui::Button`'s shape is common to every backend — see that trait's own doc
/// comment; only these method bodies are WinUI3-specific.
#[elwindui_macros::class]
impl Button {
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.base.handle.measure(available)
    }

    fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    fn set_on_click(&self, callback: Box<dyn Fn()>) {
        *self.on_click.borrow_mut() = Some(callback);
    }

    /// Used by generic resync when a `ButtonImpl`'s `text` attribute is a dynamic expression.
    fn set_text(&self, text: &str) {
        let _ = self.xaml.SetContent(&HSTRING::from(text));
    }

    fn new() -> Self {
        let xaml = XamlButton::new().expect("ButtonImpl::new");
        let handle = AnyView::from(xaml.clone());
        let this = Self { base: elwindui_core::ui::create_native_control(handle), xaml, on_click: Rc::new(RefCell::new(None)) };
        let callback = this.on_click.clone();
        let _ = this.xaml.Click(&RoutedEventHandler::new(move |_, _| {
            if let Some(cb) = callback.borrow().as_ref() {
                cb();
            }
            Ok(())
        }));
        this
    }
}

pub fn create_button() -> ButtonImpl {
    ButtonImpl::new()
}

/// `Microsoft.UI.Xaml.Controls.TabView` is a real native tabbed-document control (unlike AppKit,
/// which has no built-in equivalent — `elwindui-backend-appkit`'s `TabStrip`/`TabChip` hand-roll
/// one from `ButtonImpl`s), so this wraps it directly instead of assembling a strip from scratch. Each
/// tab's `TabViewItem.Content` is a `TreeHostPanel` (see that type) holding that tab's
/// `Rc<dyn UIElement>` — `elwindui-builtins`'s generic wrapper (the `Rc<dyn Any>`-erased per-item type,
/// mirroring `elwindui-builtins::appkit::tab_view`) owns the tab list and calls the methods below;
/// this type only knows about "N tabs, each with a title and a content host", the same division
/// AppKit's `NativeTabViewImpl` keeps.
/// `NativeTabViewImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — mirrors
/// `elwindui-backend-appkit::TabView`, extending `NativeControl<AnyView>` since a real `AnyView`
/// handle (`self.base.handle`, wrapping `self.xaml`) is what makes this leaf embeddable in the
/// visual tree at all.
pub trait TabView: elwindui_core::ui::NativeControl<AnyView> {
    fn set_on_select(&self, callback: Box<dyn Fn(usize)>);
    fn set_on_close(&self, callback: Box<dyn Fn(usize)>);
    fn set_on_new_tab(&self, callback: Box<dyn Fn()>);
    /// Inserts a new tab at `index` with an empty content host, returning that host so the caller
    /// (`elwindui-builtins`'s generic wrapper) can `set_tree` it — the WinUI3 counterpart of
    /// AppKit's `insert_tab`, minus the per-chip `on_select`/`on_close` callbacks (WinUI3's
    /// `NativeTabViewImpl` fires those once for the whole control, wired in `new` above, not per item).
    fn insert_tab(&self, index: usize, title: &str, closable: bool) -> TreeHostPanel;
    fn remove_tab_at(&self, index: usize);
    fn set_tab_title(&self, index: usize, title: &str);
    fn set_selected_index(&self, index: usize);
}

#[elwindui_macros::class(implements = TabView, inherits = elwindui_core::ui::NativeControl<AnyView>)]
pub struct NativeTabViewImpl {
    xaml: XamlTabView,
    on_select: Rc<RefCell<Option<Box<dyn Fn(usize)>>>>,
    on_close: Rc<RefCell<Option<Box<dyn Fn(usize)>>>>,
    on_new_tab: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

#[elwindui_macros::class]
impl NativeTabViewImpl {
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.base.handle.measure(available)
    }

    fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_select.borrow_mut() = Some(callback);
    }

    fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_close.borrow_mut() = Some(callback);
    }

    fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        *self.on_new_tab.borrow_mut() = Some(callback);
    }

    fn insert_tab(&self, index: usize, title: &str, closable: bool) -> TreeHostPanel {
        let content_host = TreeHostPanel::new();
        let item = TabViewItem::new().expect("TabViewItem::new");
        let _ = item.SetHeader(&HSTRING::from(title));
        let _ = item.SetIsClosable(closable);
        let _ = item.SetContent(&content_host.as_element());
        if let Ok(items) = self.xaml.TabItems() {
            let _ = items.InsertAt(index as u32, &item.into());
        }
        content_host
    }

    fn remove_tab_at(&self, index: usize) {
        if let Ok(items) = self.xaml.TabItems() {
            let _ = items.RemoveAt(index as u32);
        }
    }

    fn set_tab_title(&self, index: usize, title: &str) {
        if let Ok(items) = self.xaml.TabItems() {
            if let Ok(item) = items.GetAt(index as u32) {
                if let Ok(item) = item.cast::<TabViewItem>() {
                    let _ = item.SetHeader(&HSTRING::from(title));
                }
            }
        }
    }

    fn set_selected_index(&self, index: usize) {
        let _ = self.xaml.SetSelectedIndex(index as i32);
    }

    fn new() -> Self {
        let xaml = XamlTabView::new().expect("NativeTabViewImpl::new");
        let _ = xaml.SetTabWidthMode(bindings::Microsoft::UI::Xaml::Controls::TabViewWidthMode::SizeToContent);
        let _ = xaml.SetCloseButtonOverlayMode(TabViewCloseButtonOverlayMode::Always);
        let _ = xaml.SetIsAddTabButtonVisible(true);

        let handle = AnyView::from(xaml.clone());
        let this = Self {
            base: elwindui_core::ui::create_native_control(handle),
            xaml,
            on_select: Rc::new(RefCell::new(None)),
            on_close: Rc::new(RefCell::new(None)),
            on_new_tab: Rc::new(RefCell::new(None)),
        };

        let on_select = this.on_select.clone();
    let _ = this.xaml.SelectionChanged(&TypedEventHandler::<XamlTabView, SelectionChangedEventArgs>::new(move |sender, _| {
        if let (Some(sender), Some(cb)) = (sender, on_select.borrow().as_ref()) {
            let index = sender.SelectedIndex().unwrap_or(-1);
            if index >= 0 {
                cb(index as usize);
            }
        }
        Ok(())
    }));

    let on_close = this.on_close.clone();
    let _ = this.xaml.TabCloseRequested(&TypedEventHandler::<XamlTabView, TabViewTabCloseRequestedEventArgs>::new(move |sender, args| {
        if let (Some(sender), Some(args), Some(cb)) = (sender, args, on_close.borrow().as_ref()) {
            if let Ok(items) = sender.TabItems() {
                if let Ok(item) = args.Tab() {
                    if let Ok(index) = items.IndexOf(&item.into()) {
                        cb(index as usize);
                    }
                }
            }
        }
        Ok(())
    }));

        let on_new_tab = this.on_new_tab.clone();
        let _ = this.xaml.AddTabButtonClick(&TypedEventHandler::new(move |_, _| {
            if let Some(cb) = on_new_tab.borrow().as_ref() {
                cb();
            }
            Ok(())
        }));

        this
    }
}

pub fn create_tab_view() -> NativeTabViewImpl {
    NativeTabViewImpl::new()
}

/// See `elwindui-backend-appkit::MenuItemImpl`'s doc comment — same role, backed by a
/// `MenuFlyoutItem` (WinUI3's `MenuBarItem.Items` collection holds `MenuFlyoutItemBase`s).
#[derive(Clone)]
pub struct MenuItemImpl {
    xaml: MenuFlyoutItem,
    on_select: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

/// `elwindui_core::ui::MenuItem`'s shape is common to every backend — see that trait's own doc
/// comment; only these method bodies are WinUI3-specific.
impl elwindui_core::ui::MenuItem for MenuItemImpl {
    /// A real title setter — `create_menu_item()` takes no title argument, so this is the only way
    /// a menu item's title is ever actually set.
    fn set_text(&self, text: &str) {
        let _ = self.xaml.SetText(&HSTRING::from(text));
    }

    fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`), matching AppKit's `set_shortcut` convention — mapped to
    /// a `Ctrl`-modifier `KeyboardAccelerator` (WinUI3 has no single-string key-equivalent setter
    /// the way `NSMenuItem.keyEquivalent` does).
    fn set_shortcut(&self, key_equivalent: &str) {
        let Some(key) = key_equivalent.chars().next() else { return };
        let Ok(accelerator) = KeyboardAccelerator::new() else { return };
        let _ = accelerator.SetModifiers(bindings::Microsoft::UI::Xaml::Input::VirtualKeyModifiers::Control);
        let virtual_key = bindings::Windows::System::VirtualKey(key.to_ascii_uppercase() as i32);
        let _ = accelerator.SetKey(virtual_key);
        if let Ok(accelerators) = self.xaml.KeyboardAccelerators() {
            let _ = accelerators.Append(&accelerator);
        }
    }

    fn set_on_select(&self, callback: Box<dyn Fn()>) {
        *self.on_select.borrow_mut() = Some(callback);
    }
}

pub fn create_menu_item() -> MenuItemImpl {
    let xaml = MenuFlyoutItem::new().expect("MenuFlyoutItem::new");
    let this = MenuItemImpl { xaml, on_select: Rc::new(RefCell::new(None)) };
    let callback = this.on_select.clone();
    let _ = this.xaml.Click(&RoutedEventHandler::new(move |_, _| {
        if let Some(cb) = callback.borrow().as_ref() {
            cb();
        }
        Ok(())
    }));
    this
}

/// A dropdown attached to a `MenuBarItem` — see `elwindui-backend-appkit::MenuImpl`'s doc comment.
/// `items` is a plain `Vec` (not the native `MenuFlyoutItemBase` collection directly) since a
/// `MenuImpl` only ever becomes real XAML elements once installed into a `MenuBarItemImpl`
/// (`create_menu_bar_item`/`MenuBarItemImpl::set_submenu`) — `add_item`/`remove_item` mutate this
/// `Vec` and, if already installed, the live XAML collection too.
#[derive(Clone)]
pub struct MenuImpl {
    items: Rc<RefCell<Vec<MenuItemImpl>>>,
    installed_into: Rc<RefCell<Option<bindings::Windows::Foundation::Collections::IVector<MenuFlyoutItemBase>>>>,
}

/// `add_item`/`remove_item` are real `IVector<MenuFlyoutItemBase>.Append`/`.RemoveAtEnd`-style
/// calls once this `Menu` is installed into a `MenuBarItemImpl` (see `installed_into`'s doc
/// comment), reachable post-construction so `elwindui-backend-winui3::builtins::MenuImpl::
/// set_children` can reconcile a changed child list without rebuilding the native menu from
/// scratch. Plain inherent methods (not `elwindui_core::ui::Menu`, unlike the DSL-facing wrapper's
/// own `impl Menu for builtins::MenuImpl`) — this raw layer already knows its own concrete
/// `MenuItemImpl` argument type, so it has no need for that trait's `&dyn MenuItem`/`AsAny`-downcast
/// indirection, the same way `Window` above stays plain inherent methods throughout.
impl MenuImpl {
    fn add_item(&self, item: &MenuItemImpl) {
        self.items.borrow_mut().push(item.clone());
        if let Some(items) = self.installed_into.borrow().as_ref() {
            let base: MenuFlyoutItemBase = item.xaml.clone().into();
            let _ = items.Append(&base);
        }
    }
    fn remove_item(&self, item: &MenuItemImpl) {
        let mut items = self.items.borrow_mut();
        if let Some(pos) = items.iter().position(|i| i.xaml == item.xaml) {
            items.remove(pos);
        }
        if let Some(native_items) = self.installed_into.borrow().as_ref() {
            let base: MenuFlyoutItemBase = item.xaml.clone().into();
            if let Ok(index) = native_items.IndexOf(&base) {
                let _ = native_items.RemoveAt(index);
            }
        }
    }
}

pub fn create_menu() -> MenuImpl {
    MenuImpl { items: Rc::new(RefCell::new(Vec::new())), installed_into: Rc::new(RefCell::new(None)) }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `MenuImpl`.
#[derive(Clone)]
pub struct MenuBarItemImpl {
    xaml: bindings::Microsoft::UI::Xaml::Controls::MenuBarItem,
}

/// `set_text`/`set_submenu` are real post-construction setters (`create_menu_bar_item()` takes
/// neither argument, so these are the only way a menu bar item's title/submenu are ever actually
/// set). Plain inherent methods — see `MenuImpl`'s own `add_item`/`remove_item` doc comment for why
/// this raw layer doesn't implement `elwindui_core::ui::MenuBarItem` itself.
impl MenuBarItemImpl {
    fn set_text(&self, text: &str) {
        let _ = self.xaml.SetTitle(&HSTRING::from(text));
    }
    fn set_submenu(&self, submenu: &MenuImpl) {
        if let Ok(items) = self.xaml.Items() {
            for item in submenu.items.borrow().iter() {
                let base: MenuFlyoutItemBase = item.xaml.clone().into();
                let _ = items.Append(&base);
            }
            *submenu.installed_into.borrow_mut() = Some(items);
        }
    }
}

pub fn create_menu_bar_item() -> MenuBarItemImpl {
    let xaml = bindings::Microsoft::UI::Xaml::Controls::MenuBarItem::new().expect("MenuBarItem::new");
    MenuBarItemImpl { xaml }
}

/// The whole top menu bar. Unlike AppKit (one global `NSApplication.mainMenu`), WinUI3's
/// `MenuBarImpl` is a per-window element — installed by `Window::set_menu_bar` above, not a shared
/// process-wide singleton, so (unlike the AppKit backend) there's no app-menu-slot/Quit-item
/// special-casing needed here.
#[derive(Clone)]
pub struct MenuBarImpl {
    xaml: XamlMenuBar,
}

/// `add_item`/`remove_item` mirror `MenuImpl`'s own: real `IVector<MenuBarItem>.Append`/
/// `.RemoveAt` calls, reachable post-construction for `elwindui-backend-winui3::builtins::
/// MenuBarImpl::set_children`. Plain inherent methods — see `MenuImpl`'s own `add_item`/
/// `remove_item` doc comment for why this raw layer doesn't implement `elwindui_core::ui::MenuBar`
/// itself.
impl MenuBarImpl {
    fn add_item(&self, item: &MenuBarItemImpl) {
        if let Ok(items) = self.xaml.Items() {
            let _ = items.Append(&item.xaml);
        }
    }
    fn remove_item(&self, item: &MenuBarItemImpl) {
        if let Ok(items) = self.xaml.Items() {
            if let Ok(index) = items.IndexOf(&item.xaml) {
                let _ = items.RemoveAt(index);
            }
        }
    }
}

pub fn create_menu_bar() -> MenuBarImpl {
    let xaml = XamlMenuBar::new().expect("MenuBarImpl::new");
    MenuBarImpl { xaml }
}

/// DSL-facing `Window`/`TextArea`/`Button`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabView`/
/// `TabViewItem` wrappers that `elwindui-codegen`'s generated code actually constructs — see
/// `elwindui_backend_appkit::builtins`'s doc comment for why it's split out from the raw types
/// above instead of sharing their names at this crate's root. Declared here (after every native
/// leaf struct above, not near the top of the file) so `#[elwindui_macros::class]`'s
/// `ancestor_registry` already has every native leaf class registered by the time `builtins`' own
/// `#[class(inherits = winui3::XImpl)]` structs expand and try to chain-walk past their immediate
/// parent — see `elwindui-backend-appkit`'s own `lib.rs` for the identical reasoning.
pub mod builtins;

/// See docs/elwindui_spec.md 付録T.2 — same async-shaped-but-synchronous-underneath API as
/// AppKit's `platform::file_dialog` (`IFileOpenDialog`/`IFileSaveDialog::Show` block the calling
/// thread until the user closes the dialog; there's no genuine suspend point). Uses the classic
/// Win32 common file dialog COM interfaces (`Win32_UI_Shell` — present in the mainstream `windows`
/// crate) rather than the WinRT `Windows.Storage.Pickers` pickers, since those need
/// `IInitializeWithWindow` interop to attach to a non-UWP top-level `HWND`, which is extra
/// complexity this skips in favor of a path more likely to actually compile as written.
pub mod platform {
    pub mod file_dialog {
        use std::path::PathBuf;
        use windows::core::Interface;
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::Shell::{FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH};

        fn ensure_com_initialized() {
            unsafe {
                // Ignore the result: `RPC_E_CHANGED_MODE`/`S_FALSE` both mean COM is already
                // initialized on this thread (fine — this only ever runs on the UI thread), and
                // any other failure surfaces later as the dialog itself failing to create.
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
        }

        pub async fn open() -> Option<PathBuf> {
            ensure_com_initialized();
            unsafe {
                let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                dialog.Show(None).ok()?;
                let item = dialog.GetResult().ok()?;
                let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                Some(PathBuf::from(path.to_string().ok()?))
            }
        }

        pub async fn save() -> Option<PathBuf> {
            ensure_com_initialized();
            unsafe {
                let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                dialog.Show(None).ok()?;
                let item = dialog.GetResult().ok()?;
                let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                Some(PathBuf::from(path.to_string().ok()?))
            }
        }
    }
}

/// WinUI3's `Dispatcher` (docs/elwindui_spec.md 付録P.5): hops back to the UI thread via the
/// current thread's `DispatcherQueue` — the WinUI3/WinAppSDK analog of AppKit's
/// `dispatch2::DispatchQueue::main()`. `application::run()` (below) is what pumps this queue as
/// part of its own message loop, so a job enqueued from any thread is guaranteed to run promptly.
pub struct WinUI3Dispatcher {
    queue: DispatcherQueue,
}

impl elwindui_core::task::Dispatcher for WinUI3Dispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        let job = std::cell::RefCell::new(Some(job));
        let _ = self.queue.TryEnqueue(&DispatcherQueueHandler::new(move || {
            if let Some(job) = job.borrow_mut().take() {
                job();
            }
            Ok(())
        }));
    }
}

/// The single entry point that owns "enter the platform message loop" — kept separate from
/// `Window::show()` for the same reason as `elwindui-backend-appkit`'s `application::run()` (see
/// that module's doc comment): it's the one well-defined place to install the task executor before
/// any generated code runs.
pub mod application {
    use super::{DispatcherQueue, WinUI3Dispatcher};
    use elwindui_core::task::LocalExecutor;
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};

    /// Blocking: enters the classic Win32 message loop. A `DispatcherQueueController` is created
    /// first (needed so `#[command(async)]` bodies have somewhere to post continuations back to —
    /// see `WinUI3Dispatcher`), but for an unpackaged Win32 app hosting WinUI3 content (as opposed
    /// to a packaged UWP-style app whose `Application::Start` owns the whole loop), the actual
    /// "keep the app alive and pump input/paint messages" loop is still the plain
    /// `GetMessageW`/`DispatchMessageW` pattern every Win32 app uses — `GetMessageW` returns `0`
    /// (loop exit) once `PostQuitMessage` has been called, which every top-level `Window` here is
    /// expected to do when closed (not yet wired — see the module's UNVERIFIED note; a real
    /// implementation needs a `Window.Closed` handler calling `PostQuitMessage(0)` once the last
    /// window closes, mirroring AppKit's `applicationShouldTerminateAfterLastWindowClosed`).
    pub fn run() {
        let controller = crate::bindings::Microsoft::UI::Dispatching::DispatcherQueueController::CreateOnCurrentThread()
            .expect("DispatcherQueueController::CreateOnCurrentThread");
        let queue: DispatcherQueue = controller.DispatcherQueue().expect("DispatcherQueue");
        elwindui_core::task::set_current(LocalExecutor::new(WinUI3Dispatcher { queue }));

        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
