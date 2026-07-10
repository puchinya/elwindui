//! The framework-owned Visual tree, following WinUI3's `UIElement` hierarchy: `Rc<dyn UIElement>`
//! nodes *are* the tree (no separate wrapper/enum type) — `NativeControlImpl<H>` (`Button`/`TextArea`/
//! `MenuBar`/`TabView`, the "NativeControlImpl" family), `TextBlockImpl` (self-drawn primitive text),
//! `ShapeImpl` (`Rectangle`/`Ellipse`), `VerticalLayoutImpl`/`HorizontalLayoutImpl` (sharing
//! `StackImpl` as their own common `base`, the same way `Button`/`TextArea`/`TabView` share
//! `NativeControlImpl<H>`), and `ControlImpl` (a composable multi-part component) are all peer
//! implementations of the same `UIElement` trait.
//! `Margin`/`HorizontalAlignment`/`VerticalAlignment` (`UIElementImpl`) are common to every one of
//! them, applied generically by this module's `measure`/`arrange` (WinUI3's
//! `UIElement.Measure`/`Arrange` wrapping each type's own `MeasureOverride`/`ArrangeOverride`) —
//! see docs/elwindui_spec.md 付録H.2.
//!
//! `H` (whatever a backend uses as its native widget handle, e.g. `elwindui-backend-appkit`'s
//! `AnyView`) appears only on `NativeControlImpl<H>` itself and on the functions that walk a tree
//! looking for one (`layout_tree`) — the `UIElement` trait and every other concrete type
//! (`VerticalLayoutImpl`/`HorizontalLayoutImpl`/`ShapeImpl`/`TextBlockImpl`/`ControlImpl`) are
//! handle-agnostic, since they never hold one.
//!
//! `Window` is deliberately *not* a `UIElement` — like WinUI3's `Window`, it's a separate
//! top-level host that owns a `Rc<dyn UIElement>` (its content) and drives `layout_tree` against
//! its own client area (see `elwindui-backend-appkit`'s `TreeHostView`).
//!
//! **Ownership: `Rc`, not `Box`.** Every node holds a real parent back-reference
//! (`UIElementImpl::parent`, WinUI3's `_parent`) so `dispatch_routed` can bubble a routed event
//! from any element up to the root by simply following `parent()` — no tree search needed, and
//! critically, no dependence on the tree having been built by a single static `.elwind` traversal.
//! A back-reference requires shared (`Rc`) ownership: `Box<dyn UIElement>`'s old parent-owns-child-
//! outright model had no room for a child to point back. See `new_element`, the single choke point
//! that wires a freshly-built element's children's parent pointers — every construction site
//! (`elwindui-codegen`'s generated code, and any hand-written builtin) goes through it instead of
//! calling `Rc::new` directly.

use crate::input::RoutedEventArgs;
use crate::layout::{
    align_within, grid_arrange, grid_natural_size, grow_by_margin, shrink_by_margin, shrink_rect_by_margin,
    stack_arrange, stack_natural_size, GridCell, GridLength, HorizontalAlignment, LayoutNode, Orientation, Rect,
    Size, VerticalAlignment,
};
use crate::painter::Point;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// Lets the generic tree-walker (`layout_tree`) downcast a `&dyn UIElement` to a concrete
/// `NativeControlImpl<H>` to pull out its handle — the *only* place `native_handle`-style access
/// exists (deliberately not a method on `UIElement` itself: every other implementor would have to
/// carry a meaningless default for a concept that doesn't apply to it). Blanket-implemented for
/// every `'static` type, so no concrete `UIElement` impl needs its own boilerplate.
pub trait AsAny: Any {
    fn as_any(&self) -> &dyn Any;
}
impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The fields every `UIElement` carries (WinUI3's `FrameworkElement` base class, via composition
/// since Rust has no class inheritance — each concrete type embeds one of these and delegates
/// `UIElement::base`).
///
/// Every field here is interior-mutable (`Cell`/`RefCell`, matching `routed_handlers`/`parent`,
/// which already were) — every `create_xxx(...)` factory in this crate (and every hand-written
/// backend's `create_button`/etc.) builds its own `UIElementImpl::default()` internally, taking no
/// `base` parameter at all; `elwindui-codegen`'s generated code instead calls `set_margin`/
/// `set_horizontal_alignment`/`set_vertical_alignment`/`set_data_context`/`set_grid_cell` (and
/// `register_routed_handler`, already `Rc<RefCell<..>>`-based) through `&self` right after
/// construction, for whichever of these this specific use site actually specified. This is what
/// lets a native leaf (`Button`/`TextArea`/`TabView`, whose own `Type::new(..)` signature is fixed
/// by `elwindui-codegen`'s `Type::new(args)` calling convention) still have its use-site margin/
/// alignment/data_context applied, without threading them through every factory's constructor
/// argument list.
///
/// `data_context` (WinUI3's `FrameworkElement.DataContext`) is `Rc<dyn Any>`-erased like every
/// other cross-type-parameter value in this crate (see e.g. `elwindui_backend_appkit::builtins::tab_view`'s
/// `erase_items`/`erase_render`).
pub struct UIElementImpl {
    pub margin: Cell<f32>,
    pub horizontal_alignment: Cell<HorizontalAlignment>,
    pub vertical_alignment: Cell<VerticalAlignment>,
    pub data_context: RefCell<Option<Rc<dyn Any>>>,
    /// `#[routed]`-tagged callback fields (`on_click`, and any future one — see
    /// `docs/elwindui_spec.md` 4章), keyed by field name. Each value is a
    /// `Box<dyn Fn(&T, &RoutedEventArgs)>` erased to `Box<dyn Any>` (`T` is that field's own
    /// payload type — `()` for `on_click`, `usize` for a hypothetical routed `on_select`, ...);
    /// generated call sites know `T` statically from the `.elwind` declaration, so the downcast in
    /// `dispatch_routed` always succeeds (same erasure pattern as `data_context`/
    /// `elwindui-builtins::appkit::tab_view`'s `items_source`).
    pub routed_handlers: RoutedHandlers,
    /// This element's `GridImpl::row`/`GridImpl::column` attached-property values (docs/elwindui_spec.md
    /// §3の添付プロパティ), set right after construction from whatever the `.elwind` source wrote on
    /// this specific element (`elwindui-codegen`'s `plan_element`/`emit_construction`) —
    /// `GridCell::default()` (0, 0) for any element that set neither, which happens to coincide
    /// with `GridImpl`'s own declared attached-field defaults so no evaluation of those defaults is
    /// ever needed here. Consulted only by `GridImpl::arrange_override`/`measure_override`
    /// (`grid_arrange`/`grid_natural_size`) — harmless, unconsulted data on any element that isn't
    /// actually a child of a `GridImpl`, exactly like WPF's own attached properties. A future attached
    /// property from a different owner component would get its own field here, the same way this
    /// one was added — see this struct's own doc comment.
    pub grid_cell: Cell<GridCell>,
    /// WinUI3's `_parent` — set once by `new_element` for every child of the element being
    /// constructed. `Weak` (not `Rc`) since the parent already owns its children via `Rc` in its
    /// own `children()` list; a strong back-reference would create a cycle nothing could ever
    /// drop. `None` for the root of whatever tree this element is currently part of (there's no
    /// `Weak<dyn UIElement>::new()` — an unsizing coercion needs a concrete `Sized` source — so
    /// this is `Option`-wrapped rather than a permanently-empty `Weak`).
    pub parent: RefCell<Option<Weak<dyn UIElement>>>,
}

impl std::fmt::Debug for UIElementImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UIElementImpl")
            .field("margin", &self.margin.get())
            .field("horizontal_alignment", &self.horizontal_alignment.get())
            .field("vertical_alignment", &self.vertical_alignment.get())
            .field("data_context", &self.data_context.borrow().is_some())
            .field("routed_handlers", &self.routed_handlers.borrow().keys().collect::<Vec<_>>())
            .field("grid_cell", &self.grid_cell.get())
            .field("has_parent", &self.parent.borrow().as_ref().is_some_and(|p| p.upgrade().is_some()))
            .finish()
    }
}

impl Default for UIElementImpl {
    fn default() -> Self {
        UIElementImpl {
            margin: Cell::new(0.0),
            horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            vertical_alignment: Cell::new(VerticalAlignment::Stretch),
            data_context: RefCell::new(None),
            routed_handlers: Rc::new(RefCell::new(HashMap::new())),
            grid_cell: Cell::new(GridCell::default()),
            parent: RefCell::new(None),
        }
    }
}

impl UIElementImpl {
    /// Registers a handler for a `#[routed]`-tagged field named `name` on this element — see this
    /// struct's own `routed_handlers` doc comment for the erasure convention.
    pub fn register_routed_handler<T: 'static>(&self, name: &'static str, handler: Box<dyn Fn(&T, &RoutedEventArgs)>) {
        register_routed_handler(&self.routed_handlers, name, handler);
    }
    /// See this struct's own doc comment for why every one of these is a post-construction `&self`
    /// setter rather than a constructor argument.
    pub fn set_margin(&self, margin: f32) {
        self.margin.set(margin);
    }
    pub fn set_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.horizontal_alignment.set(alignment);
    }
    pub fn set_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.vertical_alignment.set(alignment);
    }
    pub fn set_data_context(&self, data_context: Option<Rc<dyn Any>>) {
        *self.data_context.borrow_mut() = data_context;
    }
    pub fn set_grid_cell(&self, grid_cell: GridCell) {
        self.grid_cell.set(grid_cell);
    }
}

/// The type every widget wrapper wanting `#[routed]` support (not just `UIElementImpl`, which
/// every `UIElement` already carries one of — a hand-written builtin like
/// `elwindui-builtins::appkit::Button` needs its *own* copy too, registered into at its own
/// construction time and later shared into the `NativeControlImpl` wrapping it, since that wrapper
/// doesn't exist yet when the widget itself is constructed and wired — see
/// `elwindui-codegen`'s `into_node_if_needed`) stores its handlers as.
pub type RoutedHandlers = Rc<RefCell<HashMap<&'static str, Vec<Box<dyn Any>>>>>;

/// Shared registration logic for anything holding a [`RoutedHandlers`] — `UIElementImpl`'s own
/// `register_routed_handler` method delegates here, and any widget wrapper exposing its own
/// `register_routed_handler` (see this module's own doc comment) should too, rather than
/// reimplementing the erasure.
pub fn register_routed_handler<T: 'static>(handlers: &RoutedHandlers, name: &'static str, handler: Box<dyn Fn(&T, &RoutedEventArgs)>) {
    handlers.borrow_mut().entry(name).or_default().push(Box::new(handler));
}

/// The common interface every element in the Visual tree implements — `NativeControlImpl<H>`,
/// `TextBlockImpl`, `ShapeImpl`, `VerticalLayoutImpl`/`HorizontalLayoutImpl`, and `ControlImpl` are
/// all peers here, not variants of some enum.
/// New kinds (a future `GridImpl`, say) are added by implementing this trait; nothing here or in
/// `layout_tree` needs to change.
pub trait UIElement: AsAny {
    fn base(&self) -> &UIElementImpl;
    fn margin(&self) -> f32 {
        self.base().margin.get()
    }
    fn horizontal_alignment(&self) -> HorizontalAlignment {
        self.base().horizontal_alignment.get()
    }
    fn vertical_alignment(&self) -> VerticalAlignment {
        self.base().vertical_alignment.get()
    }
    /// WinUI3's `FrameworkElement.DataContext` — an ambient, type-erased data value an element
    /// carries (set explicitly via the `data_context:` common attribute, or populated internally by
    /// `TabView`'s `items_source` mode for each generated `TabViewItem`). `None` when unset.
    fn data_context(&self) -> Option<Rc<dyn Any>> {
        self.base().data_context.borrow().clone()
    }
    /// WinUI3's `VisualTreeHelper.GetParent` — `None` for the root of whatever tree this element
    /// is currently part of. See `UIElementImpl::parent`'s doc comment.
    fn parent(&self) -> Option<Rc<dyn UIElement>> {
        self.base().parent.borrow().as_ref().and_then(|p| p.upgrade())
    }
    /// This element's own children in the **Visual tree** (WinUI3's own Visual-tree children,
    /// docs/elwindui_spec.md 付録H.2.2) — empty for a leaf like `NativeControlImpl`/
    /// `TextBlockImpl`/`ShapeImpl`. For every container type today (`StackImpl`/`ControlImpl`/
    /// `GridImpl`) this is derived directly from that type's own `UIElementCollection` (the
    /// Logical-tree child list declared in `.elwind`), since no templating exists yet to make the
    /// two trees diverge. Returns an owned `Vec` (each `Rc<dyn UIElement>` cheaply cloned, a
    /// refcount bump) rather than `&[..]`: `children` is now `RefCell`-backed (settable after
    /// construction via `set_children` — docs/elwindui_spec.md 付録H.2.1a's post-construction
    /// setter convention, extended to every builtin property), and a `std::cell::Ref` guard can't
    /// be smuggled out through a bare reference tied to `&self`.
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>>;
    /// This element's own desired size, given `available` (margin already excluded by the caller)
    /// and its children's already-measured sizes (WinUI3's `MeasureOverride`).
    fn measure_override(&self, available: Size, child_sizes: &[Size]) -> Size;
    /// The rect to assign each child (in this element's own local coordinate space), given the
    /// final size this element itself was assigned (WinUI3's `ArrangeOverride`).
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect>;
    /// Content this element paints for itself, if any (`None` for pure layout containers like
    /// `StackImpl`, which only position children and draw nothing on their own account).
    fn paint(&self) -> Option<PaintKind> {
        None
    }
    /// `Some(self)` for `NativeControlImpl<H>` itself, and for any type that composes one as its own
    /// `base` field (docs/elwindui_spec.md 付録H.2.1a — e.g. a backend's `ButtonImpl { base:
    /// NativeControlImpl<AnyView>, .. }` overrides this to return `Some(&self.base)`); `None` for
    /// every other `UIElement` (the default). `arrange`'s own downcast goes through this instead of
    /// downcasting `self` directly, since `Any::downcast_ref` only ever succeeds against the exact
    /// concrete type placed in the tree — without this indirection, a composing type's *own*
    /// concrete type (not `NativeControlImpl<H>` itself) would never be recognized as carrying a
    /// real native handle.
    fn as_native_control(&self) -> Option<&dyn Any> {
        None
    }
}

/// The single choke point every construction site (`elwindui-codegen`'s generated code, and any
/// hand-written builtin) goes through instead of calling `Rc::new` directly — wires each of
/// `value`'s own children's `UIElementImpl::parent` back-reference to the freshly-created `Rc`
/// before handing it back. See this module's own top doc comment.
pub fn new_element<T: UIElement + 'static>(value: T) -> Rc<dyn UIElement> {
    let this: Rc<dyn UIElement> = Rc::new(value);
    for child in this.visual_children() {
        *child.base().parent.borrow_mut() = Some(Rc::downgrade(&this));
    }
    this
}

/// The Logical-tree child list a container (`Layout`/`Control` family) declares in `.elwind` —
/// WinUI3's own `UIElementCollection` (docs/elwindui_spec.md 付録H.2.2). For every concrete
/// container type today (`StackImpl`/`ControlImpl`/`GridImpl`) this list *is* also the Visual
/// tree's children — no templating exists yet to make the two diverge, so
/// `UIElement::visual_children` is derived from it directly (`as_slice`).
pub struct UIElementCollection(Vec<Rc<dyn UIElement>>);

impl UIElementCollection {
    pub fn new(children: Vec<Rc<dyn UIElement>>) -> Self {
        UIElementCollection(children)
    }
    pub fn as_slice(&self) -> &[Rc<dyn UIElement>] {
        &self.0
    }
}

/// One entry of `layout_tree`'s output, in `arrange`'s own parent-before-children traversal
/// order — the single ordering a backend's host must replay verbatim (`addSubview`/`addSublayer`,
/// or WinUI3's `Children.Append`, in this exact sequence) for a native leaf and a self-painted
/// element to end up in the same relative front-to-back position they have in the source `.elwind`
/// tree. Splitting `natives`/`paints` into two separately-ordered lists (the old shape) throws this
/// relative ordering away — a `Rectangle` painted after a `Button` in traversal order could still
/// end up either above or below it depending only on which backend happens to process its own two
/// lists in which order, which is exactly the bug this single interleaved list avoids.
pub enum RenderItem<H> {
    Native(H, Rect, Rc<dyn UIElement>),
    Paint(PaintKind, Rect),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintKind {
    Shape { kind: ShapeKind, fill: Option<String>, stroke: Option<String>, stroke_width: f32 },
    /// `TextBlockImpl`'s self-drawn content. No font/size here yet (kept minimal for this pass) — a
    /// backend measures/renders the string itself (e.g. AppKit via `NSAttributedString`/
    /// `CATextLayer`), the same "elwindui-core doesn't know how to actually draw" split `ShapeImpl`
    /// already has with `CAShapeLayer`.
    Text { content: String, color: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeKind {
    RoundedRect { corner_radius: f32 },
    Oval,
}

/// `Button`/`TextArea`/`MenuBar`/`TabView` (the "NativeControlImpl" family) — the only `UIElement`
/// with a real backend handle. Always a leaf as far as this tree is concerned: whatever lives
/// beneath it in its own backend-managed hierarchy (e.g. `TabView`'s tab-switching) is opaque here.
pub struct NativeControlImpl<H> {
    pub base: UIElementImpl,
    pub handle: H,
}

impl<H: LayoutNode + 'static> UIElement for NativeControlImpl<H> {
    fn base(&self) -> &UIElementImpl {
        &self.base
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        Vec::new()
    }
    fn measure_override(&self, available: Size, _child_sizes: &[Size]) -> Size {
        self.handle.measure(available)
    }
    fn arrange_override(&self, _final_size: Size, _child_sizes: &[Size]) -> Vec<Rect> {
        Vec::new()
    }
    fn as_native_control(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

/// `NativeControlImpl<H>`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — no methods beyond
/// `UIElement` today; exists so the type participates in the trait+`Impl`+`base` convention like
/// every other class in this hierarchy.
pub trait NativeControl<H>: UIElement {}
impl<H: LayoutNode + 'static> NativeControl<H> for NativeControlImpl<H> {}

pub fn create_native_control<H>(handle: H) -> NativeControlImpl<H> {
    NativeControlImpl { base: UIElementImpl::default(), handle }
}

/// `Layout`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — empty marker over `UIElement`,
/// implemented by every layout-container virtual builtin (`VerticalLayoutImpl`/
/// `HorizontalLayoutImpl`/`GridImpl`), the same way `NativeControl<H>` groups every native leaf.
pub trait Layout: UIElement {}

/// Shared implementation behind `VerticalLayout`/`HorizontalLayout` — a thin wrapper around
/// `elwindui_core::layout`'s `stack_arrange`/`stack_natural_size` free functions. Not itself a
/// DSL-facing leaf type (mirrors `NativeControlImpl<H>`'s role for `Button`/`TextArea`/`TabView`):
/// `VerticalLayoutImpl`/`HorizontalLayoutImpl` each hold one as `base` and delegate `UIElement` to
/// it, the same trait+Impl+base composition every other builtin follows (docs/elwindui_spec.md
/// 付録H.2.1a) — `VerticalLayout`/`HorizontalLayout` used to share this struct directly with no
/// per-orientation type of their own; that was the one remaining exception to the convention.
pub struct StackImpl {
    pub base: UIElementImpl,
    /// Fixed for the lifetime of the value (which concrete factory built it — `create_stack`'s own
    /// caller — not a `.elwind`-settable `#[param]`), so plain, not `Cell`-wrapped.
    pub orientation: Orientation,
    pub spacing: Cell<f32>,
    pub children: RefCell<UIElementCollection>,
}

impl UIElement for StackImpl {
    fn base(&self) -> &UIElementImpl {
        &self.base
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.children.borrow().as_slice().to_vec()
    }
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        stack_natural_size(self.orientation, self.spacing.get(), child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        stack_arrange(final_size, self.orientation, self.spacing.get(), child_sizes)
    }
}

impl Layout for StackImpl {}

impl StackImpl {
    pub fn set_spacing(&self, spacing: f32) {
        self.spacing.set(spacing);
    }
    pub fn set_children(&self, children: UIElementCollection) {
        *self.children.borrow_mut() = children;
    }
}

fn create_stack(orientation: Orientation) -> StackImpl {
    StackImpl { base: UIElementImpl::default(), orientation, spacing: Cell::new(0.0), children: RefCell::new(UIElementCollection::new(Vec::new())) }
}

/// `VerticalLayoutImpl`'s own class trait — empty marker (docs/elwindui_spec.md 付録H.2.1a).
pub trait VerticalLayout: Layout {}
impl VerticalLayout for VerticalLayoutImpl {}

pub struct VerticalLayoutImpl {
    pub base: StackImpl,
}

impl UIElement for VerticalLayoutImpl {
    fn base(&self) -> &UIElementImpl {
        self.base.base()
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.base.visual_children()
    }
    fn measure_override(&self, available: Size, child_sizes: &[Size]) -> Size {
        self.base.measure_override(available, child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        self.base.arrange_override(final_size, child_sizes)
    }
}

impl Layout for VerticalLayoutImpl {}

impl VerticalLayoutImpl {
    pub fn set_spacing(&self, spacing: f32) {
        self.base.set_spacing(spacing);
    }
    pub fn set_children(&self, children: UIElementCollection) {
        self.base.set_children(children);
    }
}

pub fn create_vertical_layout() -> VerticalLayoutImpl {
    VerticalLayoutImpl { base: create_stack(Orientation::Vertical) }
}

/// `HorizontalLayoutImpl`'s own class trait — empty marker (docs/elwindui_spec.md 付録H.2.1a).
pub trait HorizontalLayout: Layout {}
impl HorizontalLayout for HorizontalLayoutImpl {}

pub struct HorizontalLayoutImpl {
    pub base: StackImpl,
}

impl UIElement for HorizontalLayoutImpl {
    fn base(&self) -> &UIElementImpl {
        self.base.base()
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.base.visual_children()
    }
    fn measure_override(&self, available: Size, child_sizes: &[Size]) -> Size {
        self.base.measure_override(available, child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        self.base.arrange_override(final_size, child_sizes)
    }
}

impl Layout for HorizontalLayoutImpl {}

impl HorizontalLayoutImpl {
    pub fn set_spacing(&self, spacing: f32) {
        self.base.set_spacing(spacing);
    }
    pub fn set_children(&self, children: UIElementCollection) {
        self.base.set_children(children);
    }
}

pub fn create_horizontal_layout() -> HorizontalLayoutImpl {
    HorizontalLayoutImpl { base: create_stack(Orientation::Horizontal) }
}

/// `Rectangle`/`Ellipse`. A pure leaf, like `TextBlockImpl` — no children of its own (matching real
/// WinUI3's `Shape`, which likewise has no `Children`/content property; see docs/elwindui_spec.md
/// 付録H.2.2), so its natural size is just its own drawn bounds.
pub struct ShapeImpl {
    pub base: UIElementImpl,
    pub kind: Cell<ShapeKind>,
    pub fill: RefCell<Option<String>>,
    pub stroke: RefCell<Option<String>>,
    pub stroke_width: Cell<f32>,
}

impl UIElement for ShapeImpl {
    fn base(&self) -> &UIElementImpl {
        &self.base
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        Vec::new()
    }
    fn measure_override(&self, _available: Size, _child_sizes: &[Size]) -> Size {
        Size { width: 0.0, height: 0.0 }
    }
    fn arrange_override(&self, _final_size: Size, _child_sizes: &[Size]) -> Vec<Rect> {
        Vec::new()
    }
    fn paint(&self) -> Option<PaintKind> {
        Some(PaintKind::Shape {
            kind: self.kind.get(),
            fill: self.fill.borrow().clone(),
            stroke: self.stroke.borrow().clone(),
            stroke_width: self.stroke_width.get(),
        })
    }
}

/// `ShapeImpl`'s own class trait — empty marker (docs/elwindui_spec.md 付録H.2.1a); `Shape` has no
/// further DSL-level subclass today.
pub trait Shape: UIElement {}
impl Shape for ShapeImpl {}

impl ShapeImpl {
    pub fn set_kind(&self, kind: ShapeKind) {
        self.kind.set(kind);
    }
    pub fn set_fill(&self, fill: Option<String>) {
        *self.fill.borrow_mut() = fill;
    }
    pub fn set_stroke(&self, stroke: Option<String>) {
        *self.stroke.borrow_mut() = stroke;
    }
    pub fn set_stroke_width(&self, stroke_width: f32) {
        self.stroke_width.set(stroke_width);
    }
}

pub fn create_shape() -> ShapeImpl {
    ShapeImpl {
        base: UIElementImpl::default(),
        kind: Cell::new(ShapeKind::RoundedRect { corner_radius: 0.0 }),
        fill: RefCell::new(None),
        stroke: RefCell::new(None),
        stroke_width: Cell::new(0.0),
    }
}

/// Self-drawn primitive text (WinUI3's `TextBlockImpl`) — no native widget, unlike the native `Text`
/// this replaces. A leaf, like `NativeControlImpl`. Field named `text` (not `content`, unlike
/// `PaintKind::Text`'s own field of the same meaning) to match `builtin::TextBlock`'s own `#[param]
/// text` name — `elwindui-codegen`'s setter-based construction calls `.set_{param name}(..)`
/// generically, so the Rust field/setter name must agree with the DSL's own field name.
pub struct TextBlockImpl {
    pub base: UIElementImpl,
    pub text: RefCell<String>,
    pub color: RefCell<Option<String>>,
}

impl UIElement for TextBlockImpl {
    fn base(&self) -> &UIElementImpl {
        &self.base
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        Vec::new()
    }
    fn measure_override(&self, _available: Size, _child_sizes: &[Size]) -> Size {
        // `elwindui-core` has no font metrics of its own (measurement, like painting, is a
        // backend concern for self-drawn content — see `ShapeImpl`'s same split) — a rough per-
        // character estimate is enough to avoid collapsing to zero size; a backend may still
        // render a string that overflows this estimate.
        Size { width: self.text.borrow().chars().count() as f32 * 8.0, height: 16.0 }
    }
    fn arrange_override(&self, _final_size: Size, _child_sizes: &[Size]) -> Vec<Rect> {
        Vec::new()
    }
    fn paint(&self) -> Option<PaintKind> {
        Some(PaintKind::Text { content: self.text.borrow().clone(), color: self.color.borrow().clone() })
    }
}

/// `TextBlockImpl`'s own class trait — empty marker (docs/elwindui_spec.md 付録H.2.1a); `TextBlock`
/// has no further DSL-level subclass today.
pub trait TextBlock: UIElement {}
impl TextBlock for TextBlockImpl {}

impl TextBlockImpl {
    pub fn set_text(&self, text: String) {
        *self.text.borrow_mut() = text;
    }
    pub fn set_color(&self, color: Option<String>) {
        *self.color.borrow_mut() = color;
    }
}

pub fn create_text_block() -> TextBlockImpl {
    TextBlockImpl { base: UIElementImpl::default(), text: RefCell::new(String::new()), color: RefCell::new(None) }
}

/// A composable, multi-part component (WinUI3's `ControlImpl`) — Visually built from any number of
/// other `UIElement`s (`VerticalLayoutImpl`/`HorizontalLayoutImpl`/`ShapeImpl`/`TextBlockImpl`/
/// `NativeControlImpl`/other `ControlImpl`s), stored as its own `UIElementCollection` (the Logical
/// tree this component declares, docs/elwindui_spec.md 付録H.2.2) — unlike `ShapeImpl`, which has
/// no children at all. `padding` shrinks the area its children are overlaid into, the
/// `ControlImpl`-level analog of `margin` on an individual element.
///
/// Scope note: this is intentionally minimal for now — `content_horizontal_alignment`/
/// `content_vertical_alignment` are stored but not yet consulted by `arrange_override` (each
/// child's *own* `horizontal_alignment`/`vertical_alignment`, applied generically by `arrange`
/// below, already governs its placement within the padded content area); template
/// replacement/Logical-tree wiring is future work (see `LogicalNode`).
pub struct ControlImpl {
    pub base: UIElementImpl,
    pub padding: Cell<f32>,
    pub content_horizontal_alignment: Cell<HorizontalAlignment>,
    pub content_vertical_alignment: Cell<VerticalAlignment>,
    pub children: RefCell<UIElementCollection>,
}

impl UIElement for ControlImpl {
    fn base(&self) -> &UIElementImpl {
        &self.base
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.children.borrow().as_slice().to_vec()
    }
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        let inner = child_sizes
            .iter()
            .fold(Size { width: 0.0, height: 0.0 }, |acc, s| Size { width: acc.width.max(s.width), height: acc.height.max(s.height) });
        grow_by_margin(inner, self.padding.get())
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        let full = Rect { x: 0.0, y: 0.0, width: final_size.width, height: final_size.height };
        vec![shrink_rect_by_margin(full, self.padding.get()); child_sizes.len()]
    }
}

/// `ControlImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — exposes the fields a
/// DSL-level subclass composed via `base: ControlImpl` (e.g. `builtin::ContentControl`,
/// `crates/elwindui-builtins/src/builtins.elwind`) delegates to.
pub trait Control: UIElement {
    fn padding(&self) -> f32;
    fn content_horizontal_alignment(&self) -> HorizontalAlignment;
    fn content_vertical_alignment(&self) -> VerticalAlignment;
}
impl Control for ControlImpl {
    fn padding(&self) -> f32 {
        self.padding.get()
    }
    fn content_horizontal_alignment(&self) -> HorizontalAlignment {
        self.content_horizontal_alignment.get()
    }
    fn content_vertical_alignment(&self) -> VerticalAlignment {
        self.content_vertical_alignment.get()
    }
}

impl ControlImpl {
    pub fn set_padding(&self, padding: f32) {
        self.padding.set(padding);
    }
    pub fn set_content_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.content_horizontal_alignment.set(alignment);
    }
    pub fn set_content_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.content_vertical_alignment.set(alignment);
    }
    pub fn set_children(&self, children: UIElementCollection) {
        *self.children.borrow_mut() = children;
    }
}

pub fn create_control() -> ControlImpl {
    ControlImpl {
        base: UIElementImpl::default(),
        padding: Cell::new(0.0),
        content_horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
        content_vertical_alignment: Cell::new(VerticalAlignment::Stretch),
        children: RefCell::new(UIElementCollection::new(Vec::new())),
    }
}

/// WPF/WinUI3-style row/column layout (`builtin::Grid`, docs/elwindui_spec.md §3). Each child's
/// cell placement comes from its own `UIElementImpl::grid_cell` (the `Grid::row`/`Grid::column`
/// attached properties it was constructed with), not a field on `GridImpl` itself — see `GridCell`'s
/// doc comment. A child whose `grid_cell` falls outside `row_definitions`/`column_definitions`'
/// bounds is clamped to the last row/column, mirroring `grid_arrange`'s own clamping. Row/column
/// spanning is out of scope for this pass (one child per cell) — a future `#[attached]
/// row_span`/`column_span` pair on `builtin::Grid` would extend this the same way `row`/`column`
/// were added, with no changes needed here beyond consulting the extra fields.
/// `rows`/`columns` (not `row_definitions`/`column_definitions`) to match `builtin::Grid`'s own
/// `#[param] rows`/`#[param] columns` names — `elwindui-codegen`'s setter-based construction calls
/// `.set_{param name}(..)` generically, so the Rust field/setter name must agree with the DSL's.
pub struct GridImpl {
    pub base: UIElementImpl,
    pub rows: RefCell<Vec<GridLength>>,
    pub columns: RefCell<Vec<GridLength>>,
    pub children: RefCell<UIElementCollection>,
}

impl UIElement for GridImpl {
    fn base(&self) -> &UIElementImpl {
        &self.base
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.children.borrow().as_slice().to_vec()
    }
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        let cells: Vec<GridCell> = self.children.borrow().as_slice().iter().map(|c| c.base().grid_cell.get()).collect();
        grid_natural_size(&self.rows.borrow(), &self.columns.borrow(), &cells, child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        let cells: Vec<GridCell> = self.children.borrow().as_slice().iter().map(|c| c.base().grid_cell.get()).collect();
        grid_arrange(final_size, &self.rows.borrow(), &self.columns.borrow(), &cells, child_sizes)
    }
}

impl Layout for GridImpl {}

/// `GridImpl`'s own class trait — empty marker (docs/elwindui_spec.md 付録H.2.1a); `Grid` has no
/// further DSL-level subclass today.
pub trait Grid: Layout {}
impl Grid for GridImpl {}

impl GridImpl {
    pub fn set_rows(&self, rows: Vec<GridLength>) {
        *self.rows.borrow_mut() = rows;
    }
    pub fn set_columns(&self, columns: Vec<GridLength>) {
        *self.columns.borrow_mut() = columns;
    }
    pub fn set_children(&self, children: UIElementCollection) {
        *self.children.borrow_mut() = children;
    }
}

pub fn create_grid() -> GridImpl {
    GridImpl {
        base: UIElementImpl::default(),
        rows: RefCell::new(Vec::new()),
        columns: RefCell::new(Vec::new()),
        children: RefCell::new(UIElementCollection::new(Vec::new())),
    }
}

/// The tree of component *references* as authored in `.elwind` (WinUI3's Logical tree) — distinct
/// from the Visual tree (`Rc<dyn UIElement>`) `layout_tree` actually walks. A `ControlImpl` (or any
/// user-defined component) is a single `LogicalNode` here even though its Visual representation
/// may expand into many `UIElement`s. Reserved for future use by `elwindui_core::element`'s
/// `find_by_id`/`find_all` and template support — not yet produced by `elwindui-codegen`.
pub struct LogicalNode {
    pub type_name: String,
    pub children: Vec<LogicalNode>,
}

fn measure(elem: &dyn UIElement, available: Size) -> Size {
    let inner_available = shrink_by_margin(available, elem.margin());
    let child_sizes: Vec<Size> = elem.visual_children().iter().map(|c| measure(c.as_ref(), inner_available)).collect();
    let desired = elem.measure_override(inner_available, &child_sizes);
    grow_by_margin(desired, elem.margin())
}

/// `elem`'s own absolute rect and its children's, threaded through `natives`/`paints` just like
/// `arrange` below — factored out so `hit_test_at` (coordinate-only, no native handle) and
/// `arrange` (handle-collecting) can share the measure/align math without duplicating it.
fn measure_and_align(elem: &dyn UIElement, allotted: Rect) -> Rect {
    let slot = shrink_rect_by_margin(allotted, elem.margin());
    let slot_size = Size { width: slot.width, height: slot.height };
    let child_sizes_for_measure: Vec<Size> = elem.visual_children().iter().map(|c| measure(c.as_ref(), slot_size)).collect();
    let desired = elem.measure_override(slot_size, &child_sizes_for_measure);
    align_within(slot, desired, elem.horizontal_alignment(), elem.vertical_alignment())
}

fn arrange<H: Clone + 'static>(elem: &Rc<dyn UIElement>, allotted: Rect, out: &mut Vec<RenderItem<H>>) {
    let final_rect = measure_and_align(elem.as_ref(), allotted);
    let final_size = Size { width: final_rect.width, height: final_rect.height };

    // `as_native_control` (not a direct `as_any().downcast_ref` on `elem` itself) so a type that
    // *composes* a `NativeControlImpl<H>` as its own `base` field (e.g. a backend's `ButtonImpl`)
    // is recognized too — `Any::downcast_ref` only succeeds against the exact concrete type placed
    // in the tree, which for such a type is never literally `NativeControlImpl<H>` itself. See
    // `UIElement::as_native_control`'s own doc comment.
    if let Some(native) = elem.as_ref().as_native_control().and_then(|a| a.downcast_ref::<NativeControlImpl<H>>()) {
        out.push(RenderItem::Native(native.handle.clone(), final_rect, Rc::clone(elem)));
    }
    if let Some(paint) = elem.paint() {
        out.push(RenderItem::Paint(paint, final_rect));
    }

    let child_sizes: Vec<Size> = elem.visual_children().iter().map(|c| measure(c.as_ref(), final_size)).collect();
    let child_rects = elem.arrange_override(final_size, &child_sizes);
    for (child, child_rect) in elem.visual_children().iter().zip(child_rects) {
        let absolute_child_rect =
            Rect { x: final_rect.x + child_rect.x, y: final_rect.y + child_rect.y, width: child_rect.width, height: child_rect.height };
        arrange::<H>(child, absolute_child_rect, out);
    }
}

/// This element's natural (unconstrained) size — e.g. for a container that must report an
/// `intrinsicContentSize` to an Auto-Layout-managed ancestor (see `elwindui-backend-appkit`'s
/// `TreeHostView`) before it has ever actually been given a frame to lay out into.
pub fn natural_size(elem: &dyn UIElement) -> Size {
    measure(elem, Size { width: 0.0, height: 0.0 })
}

/// Recursively measures and arranges `root` against `available`, returning every `NativeControlImpl<H>`
/// leaf (its handle cloned — cheap for a thin `Retained<NSView>`-style handle) paired with its
/// **absolute** rect and the `Rc<dyn UIElement>` tree node that owns it, and every self-painting
/// element's content paired with its own absolute rect — interleaved into a single `Vec<RenderItem<H>>`
/// in `arrange`'s own traversal order (see that type's doc comment for why this must stay one list,
/// not two). A backend's host (see `elwindui-backend-appkit`'s `TreeHostView`) replays this list in
/// order: a `RenderItem::Native` gets placed as a native subview and positioned via its handle's own
/// `LayoutNode::arrange` (a real `#[routed]` click/etc. is wired once, at the widget's own
/// construction time — see e.g. `elwindui_backend_appkit::builtins::Button::new` — not here), a
/// `RenderItem::Paint` gets added as a paint layer (e.g. a `CAShapeLayer`) — `elwindui-core` itself
/// knows nothing about `NSView`/`addSubview`/`CALayer`.
///
/// `H` only needs to be named here (and on `NativeControlImpl<H>` itself) — every other `UIElement` is
/// handle-agnostic. The root's own `horizontal_alignment`/`vertical_alignment` default to
/// `Stretch` (`UIElementImpl::default`), so it fills `available` unless a caller explicitly
/// overrides them — the same default every mainstream UI framework gives a top-level content
/// element (`Window.Content`, an HTML `<body>`).
pub fn layout_tree<H: Clone + 'static>(root: &Rc<dyn UIElement>, available: Size) -> Vec<RenderItem<H>> {
    let mut out = Vec::new();
    let allotted = Rect { x: 0.0, y: 0.0, width: available.width, height: available.height };
    arrange::<H>(root, allotted, &mut out);
    out
}

fn rect_contains(rect: Rect, at: Point) -> bool {
    at.x >= rect.x && at.x <= rect.x + rect.width && at.y >= rect.y && at.y <= rect.y + rect.height
}

/// Re-runs the same measure/arrange traversal `arrange` (above) does, without needing to know any
/// backend's native handle type (`H`) — hit-testing only needs each element's own computed rect,
/// never its handle. Returns the deepest (topmost) element whose rect contains `at`, or `None` if
/// `at` falls outside `elem`'s own bounds entirely. See `elwindui_core::input::InputRouter`'s doc
/// comment (modeled on WinUI3's routed events) — bubbling from the returned element is then just
/// `dispatch_routed` following `parent()`, no path/ancestor computation needed here.
fn hit_test_at(elem: &Rc<dyn UIElement>, allotted: Rect, at: Point) -> Option<Rc<dyn UIElement>> {
    let final_rect = measure_and_align(elem.as_ref(), allotted);
    if !rect_contains(final_rect, at) {
        return None;
    }

    let final_size = Size { width: final_rect.width, height: final_rect.height };
    let child_sizes: Vec<Size> = elem.visual_children().iter().map(|c| measure(c.as_ref(), final_size)).collect();
    let child_rects = elem.arrange_override(final_size, &child_sizes);

    // Children are searched last-to-first: `arrange`'s own traversal order paints later children
    // on top of earlier ones (see 付録N's z-order note), so the *last* child whose own rect
    // contains `at` is the topmost, correctly-hit one.
    for (child, child_rect) in elem.visual_children().iter().zip(child_rects.iter()).rev() {
        let absolute_child_rect =
            Rect { x: final_rect.x + child_rect.x, y: final_rect.y + child_rect.y, width: child_rect.width, height: child_rect.height };
        if let Some(hit) = hit_test_at(child, absolute_child_rect, at) {
            return Some(hit);
        }
    }

    Some(Rc::clone(elem))
}

/// Hit-tests `root` at `at` (in `root`'s own available-space coordinates, e.g. the hosting
/// `TreeHostView`'s current bounds). Returns the deepest (topmost) hit element, or `None` if `at`
/// falls outside `root`'s own bounds entirely.
pub fn hit_test(root: &Rc<dyn UIElement>, available: Size, at: Point) -> Option<Rc<dyn UIElement>> {
    let allotted = Rect { x: 0.0, y: 0.0, width: available.width, height: available.height };
    hit_test_at(root, allotted, at)
}

/// Bubbles a routed event starting at `target` (e.g. `hit_test`'s return value, or a native leaf's
/// own tree node — see `elwindui-backend-appkit`'s `TreeHostView`): calls `target`'s own handlers
/// registered under `name` (via `UIElementImpl::register_routed_handler::<T>`), then its parent's,
/// and so on up to the root (`UIElement::parent`), stopping as soon as one sets `args.handled`.
/// Works identically whether `target`'s tree was built by a single static `.elwind` traversal or
/// assembled at runtime (e.g. `TabView`'s `items_source`/`item_template`) — `parent()` only cares
/// that `new_element` wired it, not how or when. `T` must match the type every handler for `name`
/// was registered with — see `UIElementImpl::routed_handlers`'s doc comment for why the downcast
/// this performs always succeeds in practice (both sides come from the same `.elwind` field
/// declaration).
pub fn dispatch_routed<T: 'static>(target: &Rc<dyn UIElement>, name: &str, payload: &T, args: &RoutedEventArgs) {
    let mut current = Some(Rc::clone(target));
    while let Some(elem) = current {
        let handlers = elem.base().routed_handlers.borrow();
        if let Some(handlers) = handlers.get(name) {
            for handler in handlers {
                let handler = handler
                    .downcast_ref::<Box<dyn Fn(&T, &RoutedEventArgs)>>()
                    .expect("elwindui: routed handler registered under a mismatched payload type");
                handler(payload, args);
                if args.handled.get() {
                    return;
                }
            }
        }
        drop(handlers);
        current = elem.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug)]
    struct FakeHandle(&'static str, Size);

    impl LayoutNode for FakeHandle {
        fn measure(&self, _available: Size) -> Size {
            self.1
        }
        fn arrange(&mut self, _final_rect: Rect) {}
    }

    fn size(width: f32, height: f32) -> Size {
        Size { width, height }
    }

    fn native(name: &'static str, size: Size) -> Rc<dyn UIElement> {
        new_element(create_native_control(FakeHandle(name, size)))
    }

    fn stack(orientation: Orientation, spacing: f32, children: Vec<Rc<dyn UIElement>>) -> Rc<dyn UIElement> {
        let node = create_stack(orientation);
        node.set_spacing(spacing);
        node.set_children(UIElementCollection::new(children));
        new_element(node)
    }

    // Splits `layout_tree`'s single interleaved `Vec<RenderItem<H>>` back into the pre-`RenderItem`
    // `(natives, paints)` shape these tests were originally written against (dropping each native's
    // `Rc<dyn UIElement>` tree-node component too) — a test asserting on native/paint *content*
    // doesn't care about their relative ordering against each other, only `render_item_ordering_*`
    // below (which asserts on the combined list directly) tests that.
    fn split<H: Clone>(items: Vec<RenderItem<H>>) -> (Vec<(H, Rect)>, Vec<(PaintKind, Rect)>) {
        let mut natives = Vec::new();
        let mut paints = Vec::new();
        for item in items {
            match item {
                RenderItem::Native(h, r, _) => natives.push((h, r)),
                RenderItem::Paint(p, r) => paints.push((p, r)),
            }
        }
        (natives, paints)
    }

    #[test]
    fn single_native_leaf_as_root_fills_available_space() {
        // The root's default alignment is `Stretch`, so it fills `available` regardless of its
        // own measured size — this matters for e.g. `TabView` (a native leaf) as `Window`'s
        // content: it must fill the window, not shrink to its own `fittingSize()`.
        let tree = native("a", size(10.0, 20.0));
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 100.0)));
        assert_eq!(natives, vec![(FakeHandle("a", size(10.0, 20.0)), Rect { x: 0.0, y: 0.0, width: 200.0, height: 100.0 })]);
        assert!(paints.is_empty());
    }

    #[test]
    fn nested_stack_accumulates_absolute_offsets() {
        // Vertical outer stack containing a native leaf, then a horizontal inner stack of two
        // native leaves — checks that the inner stack's children get *absolute* coordinates, not
        // coordinates relative to the inner stack alone. Every element here uses `Left`/`Top`
        // alignment explicitly (not the `Stretch` default) so each child keeps its own measured
        // size instead of filling its stack-allocated cross-axis slot — matching the old
        // `CrossAlign::Start` behavior this test used to exercise.
        fn leaf(name: &'static str, s: Size) -> Rc<dyn UIElement> {
            let node = new_element(create_native_control(FakeHandle(name, s)));
            node.base().set_horizontal_alignment(HorizontalAlignment::Left);
            node.base().set_vertical_alignment(VerticalAlignment::Top);
            node
        }
        fn start_stack(orientation: Orientation, spacing: f32, children: Vec<Rc<dyn UIElement>>) -> Rc<dyn UIElement> {
            let stack = create_stack(orientation);
            stack.set_spacing(spacing);
            stack.set_children(UIElementCollection::new(children));
            let node = new_element(stack);
            node.base().set_horizontal_alignment(HorizontalAlignment::Left);
            node.base().set_vertical_alignment(VerticalAlignment::Top);
            node
        }

        let tree = start_stack(
            Orientation::Vertical,
            5.0,
            vec![
                leaf("top", size(50.0, 10.0)),
                start_stack(Orientation::Horizontal, 2.0, vec![leaf("left", size(20.0, 20.0)), leaf("right", size(30.0, 20.0))]),
            ],
        );

        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert!(paints.is_empty());
        assert_eq!(natives.len(), 3);
        assert_eq!(natives[0], (FakeHandle("top", size(50.0, 10.0)), Rect { x: 0.0, y: 0.0, width: 50.0, height: 10.0 }));
        // inner stack starts at y = 10 (top's height) + 5 (spacing) = 15
        assert_eq!(natives[1], (FakeHandle("left", size(20.0, 20.0)), Rect { x: 0.0, y: 15.0, width: 20.0, height: 20.0 }));
        assert_eq!(natives[2], (FakeHandle("right", size(30.0, 20.0)), Rect { x: 22.0, y: 15.0, width: 30.0, height: 20.0 }));
    }

    #[test]
    fn stretch_default_fills_the_cross_axis_slot() {
        // Unlike the previous test, this one leaves alignment at its `Stretch` default — each
        // leaf should fill the *entire* stack width (the cross axis, for a vertical stack), not
        // just its own measured width.
        let tree = stack(Orientation::Vertical, 0.0, vec![native("a", size(10.0, 20.0))]);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 100.0)));
        assert_eq!(natives[0].1, Rect { x: 0.0, y: 0.0, width: 200.0, height: 20.0 });
    }

    #[test]
    fn shape_reports_paint_and_has_no_children() {
        // `Shape` (matching real WinUI3's `Shape`) is a pure leaf: no `Children`/content property
        // of its own — see `ShapeImpl`'s own doc comment.
        let shape = create_shape();
        shape.set_kind(ShapeKind::RoundedRect { corner_radius: 8.0 });
        shape.set_fill(Some("#3498db".to_string()));
        let tree: Rc<dyn UIElement> = new_element(shape);

        assert!(tree.visual_children().is_empty());
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 50.0)));
        assert_eq!(paints.len(), 1);
        // As the root, the shape fills `available` (default `Stretch`, not its own zero-sized
        // natural size).
        assert_eq!(paints[0].1, Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 });
        assert!(natives.is_empty());
    }

    #[test]
    fn control_padding_shrinks_the_slot_its_children_are_arranged_into() {
        let control = create_control();
        control.set_padding(10.0);
        control.set_children(UIElementCollection::new(vec![native("a", size(10.0, 20.0))]));
        let tree: Rc<dyn UIElement> = new_element(control);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(natives[0].1, Rect { x: 10.0, y: 10.0, width: 80.0, height: 80.0 });
    }

    #[test]
    fn empty_virtual_node_has_zero_size_and_no_leaves() {
        let tree = stack(Orientation::Vertical, 0.0, vec![]);
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
    }

    #[test]
    fn margin_shrinks_the_slot_an_element_is_arranged_into() {
        let tree: Rc<dyn UIElement> = new_element(create_native_control(FakeHandle("a", size(10.0, 20.0))));
        tree.base().set_margin(10.0);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(natives[0].1, Rect { x: 10.0, y: 10.0, width: 80.0, height: 80.0 });
    }

    #[test]
    fn non_stretch_alignment_keeps_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElement> = new_element(create_native_control(FakeHandle("a", size(10.0, 20.0))));
        tree.base().set_horizontal_alignment(HorizontalAlignment::Center);
        tree.base().set_vertical_alignment(VerticalAlignment::Center);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(natives[0].1, Rect { x: 45.0, y: 40.0, width: 10.0, height: 20.0 });
    }

    /// A minimal test-only fixture that both paints itself *and* has children — no real builtin
    /// combines the two today (`ShapeImpl` is a childless leaf; `StackImpl`/`ControlImpl`/`GridImpl`
    /// never paint), so `render_item_ordering_preserves_traversal_order_across_native_and_paint`
    /// (below) needs its own local type to exercise the paint-then-child traversal order.
    struct PaintingContainer {
        base: UIElementImpl,
        children: UIElementCollection,
    }

    impl UIElement for PaintingContainer {
        fn base(&self) -> &UIElementImpl {
            &self.base
        }
        fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
            self.children.as_slice().to_vec()
        }
        fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
            child_sizes.iter().fold(Size { width: 0.0, height: 0.0 }, |acc, s| Size { width: acc.width.max(s.width), height: acc.height.max(s.height) })
        }
        fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
            vec![Rect { x: 0.0, y: 0.0, width: final_size.width, height: final_size.height }; child_sizes.len()]
        }
        fn paint(&self) -> Option<PaintKind> {
            Some(PaintKind::Shape { kind: ShapeKind::RoundedRect { corner_radius: 4.0 }, fill: Some("#000000".to_string()), stroke: None, stroke_width: 0.0 })
        }
    }

    #[test]
    fn render_item_ordering_preserves_traversal_order_across_native_and_paint() {
        // A painting container containing a native leaf child: traversal visits the container
        // itself (pushing its `Paint`) before recursing into its child (pushing the child's
        // `Native`), so the combined list must come back `[Paint, Native]` — a backend replaying
        // this list in order therefore places the native leaf *in front of* the container's own
        // paint, matching the source tree's parent-then-child nesting instead of an accidental
        // "all natives first" or "all paints first" batching.
        let tree: Rc<dyn UIElement> = new_element(PaintingContainer {
            base: UIElementImpl::default(),
            children: UIElementCollection::new(vec![native("child", size(10.0, 10.0))]),
        });
        let items = layout_tree::<FakeHandle>(&tree, size(50.0, 50.0));
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], RenderItem::Paint(..)));
        assert!(matches!(items[1], RenderItem::Native(..)));
    }

    #[test]
    fn child_parent_pointer_is_set_by_new_element() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        assert!(Rc::ptr_eq(&leaf.parent().expect("leaf should have a parent"), &root));
        assert!(root.parent().is_none());
    }

    #[test]
    fn dispatch_routed_bubbles_and_stops_at_handled() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);

        let leaf_calls = Rc::new(RefCell::new(0));
        let root_calls = Rc::new(RefCell::new(0));
        {
            let leaf_calls = Rc::clone(&leaf_calls);
            leaf.base().register_routed_handler::<()>("on_click", Box::new(move |_, _| *leaf_calls.borrow_mut() += 1));
        }
        {
            let root_calls = Rc::clone(&root_calls);
            root.base().register_routed_handler::<()>("on_click", Box::new(move |_, args| {
                *root_calls.borrow_mut() += 1;
                args.handled.set(true);
            }));
        }

        let args = RoutedEventArgs::default();
        dispatch_routed(&leaf, "on_click", &(), &args);
        assert_eq!(*leaf_calls.borrow(), 1);
        assert_eq!(*root_calls.borrow(), 1);
        assert!(args.handled.get());
    }
}
