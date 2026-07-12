//! The framework-owned Visual tree, following WinUI3's `UIElement` hierarchy: `Rc<dyn UIElement>`
//! nodes *are* the tree (no separate wrapper/enum type) — `NativeControlImpl<H>` (`Button`/`TextArea`/
//! `MenuBar`/`TabView`, the "NativeControlImpl" family), `TextBlockImpl` (self-drawn primitive text),
//! `ShapeImpl` (`Rectangle`/`Ellipse`), `VerticalLayoutImpl`/`HorizontalLayoutImpl` (each embedding
//! shared `LayoutImpl` fields as their own `base`, but doing their own orientation-specific layout
//! math directly rather than delegating it to that base), and `ControlImpl` (a composable
//! multi-part component) are all peer implementations of the same `UIElement` trait.
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
    align_within, apply_size_constraints, grid_arrange, grid_natural_size, grow_by_margin, shrink_by_margin,
    shrink_rect_by_margin, stack_arrange, stack_natural_size, GridCell, GridLength, HorizontalAlignment, LayoutNode,
    Orientation, Rect, Size, VerticalAlignment, Visibility,
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

/// The backend-agnostic handle to whatever native host (`elwindui-backend-appkit`'s `TreeHostView`,
/// `elwindui-backend-winui3`'s `TreeHostPanel`) currently owns a given tree — the thing
/// `UIElement::invalidate`/`invalidate_arrange`/`invalidate_measure` (see that trait) ultimately
/// call to ask for a fresh `layout_tree` pass. Declared here (not a raw `Rc<dyn Fn()>`) so backends
/// provide an `impl RelayoutHost for XHost` the same way they already provide `impl
/// elwindui_core::ui::Button for ButtonImpl`/etc. — this crate's own established "shared trait in
/// core, impl per backend" convention (see this module's own doc comment on `TextArea`/`Button`/...
/// just below `NativeControl<H>`). Each backend's own `impl` should wrap a *weak* handle back to its
/// host (see e.g. `elwindui-backend-appkit`'s `AppKitRelayoutHost`) — a strong one would create a
/// reference cycle, since the host itself holds the tree that (via `UIElementImpl::invalidate_host`
/// on that tree's root) holds this `Rc<dyn RelayoutHost>` right back.
pub trait RelayoutHost {
    fn request_relayout(&self);
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
/// The common interface every element in the Visual tree implements — `NativeControlImpl<H>`,
/// `TextBlockImpl`, `ShapeImpl`, `VerticalLayoutImpl`/`HorizontalLayoutImpl`, and `ControlImpl` are
/// all peers here, not variants of some enum.
/// New kinds (a future `GridImpl`, say) are added by implementing this trait; nothing here or in
/// `layout_tree` needs to change.
///
/// `UIElementImpl` is the root of the class hierarchy (docs/elwindui_spec.md 付録H.2.1a) —
/// `#[elwindui_macros::class]`'s "root class mode" (no `inherits`): every method on the paired
/// `impl UIElement { .. }` below becomes a *default* method here, embedded body and all, so every
/// other `#[class(inherits = ..)]`-managed subclass inherits all of them for free via Rust's own
/// default-method dispatch — only `base` (synthesized by the macro; its concrete location differs
/// per implementor) is a genuinely required method.
#[elwindui_macros::class(supertrait = AsAny)]
pub struct UIElement {
    pub margin: Cell<f32>,
    pub horizontal_alignment: Cell<HorizontalAlignment>,
    pub vertical_alignment: Cell<VerticalAlignment>,
    /// WinUI3's `UIElement.Visibility` — `Visible` (default) or `Collapsed`. See `Visibility`'s own
    /// doc comment for how `Collapsed` is handled by the layout/render/hit-test traversals.
    pub visibility: Cell<Visibility>,
    /// WinUI3's `FrameworkElement.Width`/`Height`/`MinWidth`/`MinHeight`/`MaxWidth`/`MaxHeight` —
    /// `None` is WinUI3's `NaN` sentinel ("unset", i.e. auto-sized). Applied generically by this
    /// module's `measure`/`measure_and_align` (`crate::layout::apply_size_constraints`), the same
    /// way margin/alignment already are — see those functions' own doc comments.
    pub width: Cell<Option<f32>>,
    pub height: Cell<Option<f32>>,
    pub min_width: Cell<Option<f32>>,
    pub min_height: Cell<Option<f32>>,
    pub max_width: Cell<Option<f32>>,
    pub max_height: Cell<Option<f32>>,
    /// WinUI3's `UIElement.ActualWidth`/`ActualHeight`/`ActualOffset` — the *result* of the most
    /// recent `arrange` pass, not an input to it. `actual_width`/`actual_height` are set on `elem`
    /// itself once its own `final_rect` is known; `actual_offset` (relative to this element's
    /// *parent*, matching WinUI3's own `ActualOffset`) is set on each *child* by its parent, right
    /// before that child's own absolute rect is computed. Both default to `0.0`/`Point{0,0}` before
    /// any layout pass has run, and for the root of whatever tree is being laid out (no parent to
    /// set its offset — the same reasoning as WinUI3's root `Window.Content`).
    pub actual_width: Cell<f32>,
    pub actual_height: Cell<f32>,
    pub actual_offset: Cell<Point>,
    pub data_context: RefCell<Option<Rc<dyn Any>>>,
    /// `#[routed]`-tagged callback fields (`on_click`, and any future one — see
    /// `docs/elwindui_spec.md` 4章), keyed by field name. Each value is a
    /// `Box<dyn Fn(&T, &RoutedEventArgs)>` erased to `Box<dyn Any>` (`T` is that field's own
    /// payload type — `()` for `on_click`, `usize` for a hypothetical routed `on_select`, ...);
    /// generated call sites know `T` statically from the `.elwind` declaration, so the downcast in
    /// `dispatch_routed` always succeeds (same erasure pattern as `data_context`/
    /// `elwindui-builtins::appkit::tab_view`'s `items_source`).
    pub routed_handlers: RoutedHandlers,
    /// Generic, type-erased attached-property bag (docs/elwindui_spec.md §3の添付プロパティ), keyed
    /// by `(owner, field)` — e.g. `("Grid", "row")` — and populated right after construction from
    /// whatever `Owner::field: value` setters the `.elwind` source wrote on this specific element
    /// (`elwindui-codegen`'s `plan_element`/`emit_construction`/`emit_attached_setters`). Absent for
    /// any element that didn't set a given `(owner, field)` — the owner's own reader (e.g.
    /// `GridImpl`'s `grid_cell_of`) supplies the default in that case, since only the owner knows
    /// its own attached fields' declared defaults. Harmless, unconsulted data on any element that
    /// isn't actually a child of the matching owner, exactly like WPF's own attached properties. A
    /// future attached-property owner needs no changes here at all — it just calls
    /// `set_attached`/`get_attached` with its own `(owner, field)` keys.
    pub attached: RefCell<HashMap<(&'static str, &'static str), Box<dyn Any>>>,
    /// WinUI3's `_parent` — set once by `new_element` for every child of the element being
    /// constructed. `Weak` (not `Rc`) since the parent already owns its children via `Rc` in its
    /// own `children()` list; a strong back-reference would create a cycle nothing could ever
    /// drop. `None` for the root of whatever tree this element is currently part of (there's no
    /// `Weak<dyn UIElement>::new()` — an unsizing coercion needs a concrete `Sized` source — so
    /// this is `Option`-wrapped rather than a permanently-empty `Weak`).
    pub parent: RefCell<Option<Weak<dyn UIElement>>>,
    /// The Visual tree's actual child storage (WinUI3's own `VisualCollection`). Every
    /// `UIElement`'s `visual_children()` reads this generically (`UIElement`'s own default trait
    /// method), so no concrete type implements that method itself anymore. Empty (and never
    /// populated) for a leaf like `NativeControlImpl`/`ShapeImpl`/`TextBlockImpl`. A container
    /// (`LayoutImpl`/`ControlImpl`/`GridImpl`) shares this same storage with its own
    /// `UIElementCollection` (WinUI3's `Panel.Children`) via `children_collection` below - adding
    /// or removing through that Logical-tree-facing handle is what actually mutates this field.
    pub visual_children: VisualCollection,
    /// A weak handle to this same element's own `Rc<dyn UIElement>`, populated by `new_element`
    /// right after construction (the same moment `parent` is wired for this element's children -
    /// see that function). Exists so a `UIElementCollection` handed out by `children_collection`
    /// can set a newly (post-construction) added child's `parent` to the right value, even though
    /// `UIElementCollection` itself has no other way to reach "the Rc that owns me".
    pub self_handle: Rc<RefCell<Option<Weak<dyn UIElement>>>>,
    /// Set only on whichever element a backend host currently owns as the root of a hosted tree
    /// (`elwindui-backend-appkit`'s `TreeHostView::set_tree`/`elwindui-backend-winui3`'s
    /// `TreeHostPanel::set_tree`) — `None` on every other element, including every one of that
    /// root's own descendants. `UIElement::invalidate`/`invalidate_arrange`/`invalidate_measure`
    /// (see that trait) reach this by walking `parent()` up to the root, not by reading this field
    /// on `self` directly. See `RelayoutHost`'s own doc comment for why this is a trait object
    /// rather than a raw closure.
    pub invalidate_host: RefCell<Option<Rc<dyn RelayoutHost>>>,
}

impl std::fmt::Debug for UIElementImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UIElementImpl")
            .field("margin", &self.margin.get())
            .field("horizontal_alignment", &self.horizontal_alignment.get())
            .field("vertical_alignment", &self.vertical_alignment.get())
            .field("visibility", &self.visibility.get())
            .field("width", &self.width.get())
            .field("height", &self.height.get())
            .field("min_width", &self.min_width.get())
            .field("min_height", &self.min_height.get())
            .field("max_width", &self.max_width.get())
            .field("max_height", &self.max_height.get())
            .field("actual_width", &self.actual_width.get())
            .field("actual_height", &self.actual_height.get())
            .field("actual_offset", &self.actual_offset.get())
            .field("data_context", &self.data_context.borrow().is_some())
            .field("routed_handlers", &self.routed_handlers.borrow().keys().collect::<Vec<_>>())
            .field("attached_keys", &self.attached.borrow().keys().cloned().collect::<Vec<_>>())
            .field("has_parent", &self.parent.borrow().as_ref().is_some_and(|p| p.upgrade().is_some()))
            .field("visual_children_len", &self.visual_children.len())
            .field("invalidate_host", &self.invalidate_host.borrow().is_some())
            .finish()
    }
}

impl Default for UIElementImpl {
    fn default() -> Self {
        UIElementImpl {
            margin: Cell::new(0.0),
            horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            vertical_alignment: Cell::new(VerticalAlignment::Stretch),
            visibility: Cell::new(Visibility::Visible),
            width: Cell::new(None),
            height: Cell::new(None),
            min_width: Cell::new(None),
            min_height: Cell::new(None),
            max_width: Cell::new(None),
            max_height: Cell::new(None),
            actual_width: Cell::new(0.0),
            actual_height: Cell::new(0.0),
            actual_offset: Cell::new(Point { x: 0.0, y: 0.0 }),
            data_context: RefCell::new(None),
            routed_handlers: Rc::new(RefCell::new(HashMap::new())),
            attached: RefCell::new(HashMap::new()),
            parent: RefCell::new(None),
            visual_children: VisualCollection::new(),
            self_handle: Rc::new(RefCell::new(None)),
            invalidate_host: RefCell::new(None),
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
        self.invalidate();
    }
    pub fn set_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.horizontal_alignment.set(alignment);
        self.invalidate();
    }
    pub fn set_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.vertical_alignment.set(alignment);
        self.invalidate();
    }
    pub fn set_visibility(&self, visibility: Visibility) {
        self.visibility.set(visibility);
        self.invalidate();
    }
    pub fn set_data_context(&self, data_context: Option<Rc<dyn Any>>) {
        *self.data_context.borrow_mut() = data_context;
    }
    /// Stores an attached-property value under `(owner, field)` — e.g. `("Grid", "row")` — type-
    /// erased into the shared `attached` bag (see that field's own doc comment). `owner`/`field` are
    /// always compile-time-known string literals from `elwindui-codegen`'s `emit_attached_setters`,
    /// which also picks `T` via an explicit turbofish matching the `#[attached]` field's declared
    /// type — never inferred from `value` alone, since a mismatched inferred type here would make
    /// `get_attached`'s `downcast_ref` silently miss and fall back to its caller's default.
    pub fn set_attached<T: 'static>(&self, owner: &'static str, field: &'static str, value: T) {
        self.attached.borrow_mut().insert((owner, field), Box::new(value));
        self.invalidate();
    }
    /// Reads an attached-property value previously stored under `(owner, field)`, or `default` if
    /// absent (never set on this element, or set with a different `T` — the same `downcast_ref`
    /// miss as an absent key). Callers are the *owner* component's own layout code (e.g. `GridImpl`'s
    /// `grid_cell_of`), which knows its own attached field's concrete type — see `set_attached`'s
    /// own doc comment for why the type must agree between writer and reader.
    pub fn get_attached<T: Clone + 'static>(&self, owner: &'static str, field: &'static str, default: T) -> T {
        self.attached
            .borrow()
            .get(&(owner, field))
            .and_then(|v| v.downcast_ref::<T>())
            .cloned()
            .unwrap_or(default)
    }
    pub fn set_width(&self, width: Option<f32>) {
        self.width.set(width);
        self.invalidate();
    }
    pub fn set_height(&self, height: Option<f32>) {
        self.height.set(height);
        self.invalidate();
    }
    pub fn set_min_width(&self, min_width: Option<f32>) {
        self.min_width.set(min_width);
        self.invalidate();
    }
    pub fn set_min_height(&self, min_height: Option<f32>) {
        self.min_height.set(min_height);
        self.invalidate();
    }
    pub fn set_max_width(&self, max_width: Option<f32>) {
        self.max_width.set(max_width);
        self.invalidate();
    }
    pub fn set_max_height(&self, max_height: Option<f32>) {
        self.max_height.set(max_height);
        self.invalidate();
    }
    /// Called by whatever backend host (`TreeHostView::set_tree`/`TreeHostPanel::set_tree`) is
    /// about to own this element as the root of a hosted tree — see `invalidate_host`'s own doc
    /// comment. `None` un-registers (e.g. a host discarding a tree it no longer owns).
    pub fn set_invalidate_host(&self, host: Option<Rc<dyn RelayoutHost>>) {
        *self.invalidate_host.borrow_mut() = host;
    }
    /// Hands out a `UIElementCollection` (WinUI3's `Panel.Children`) sharing this same
    /// `UIElementImpl`'s `visual_children`/`self_handle` — a container (`LayoutImpl`/`ControlImpl`/
    /// `GridImpl`) calls this once, at construction time, to build its own Logical-tree-facing
    /// `children` field. See `UIElementCollection`'s own doc comment.
    pub fn children_collection(&self) -> UIElementCollection {
        UIElementCollection { visual: self.visual_children.clone(), owner: self.self_handle.clone() }
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

#[elwindui_macros::class]
impl UIElement {
    fn margin(&self) -> f32 {
        self.as_ui_element().margin.get()
    }
    fn horizontal_alignment(&self) -> HorizontalAlignment {
        self.as_ui_element().horizontal_alignment.get()
    }
    fn vertical_alignment(&self) -> VerticalAlignment {
        self.as_ui_element().vertical_alignment.get()
    }
    /// WinUI3's `UIElement.Visibility` — see `Visibility`'s own doc comment.
    fn visibility(&self) -> Visibility {
        self.as_ui_element().visibility.get()
    }
    /// WinUI3's `FrameworkElement.Width`/`Height`/`MinWidth`/`MinHeight`/`MaxWidth`/`MaxHeight` —
    /// see `UIElementImpl`'s own doc comment for these six fields.
    fn width(&self) -> Option<f32> {
        self.as_ui_element().width.get()
    }
    fn height(&self) -> Option<f32> {
        self.as_ui_element().height.get()
    }
    fn min_width(&self) -> Option<f32> {
        self.as_ui_element().min_width.get()
    }
    fn min_height(&self) -> Option<f32> {
        self.as_ui_element().min_height.get()
    }
    fn max_width(&self) -> Option<f32> {
        self.as_ui_element().max_width.get()
    }
    fn max_height(&self) -> Option<f32> {
        self.as_ui_element().max_height.get()
    }
    /// WinUI3's `UIElement.ActualWidth`/`ActualHeight`/`ActualOffset` — the result of the most
    /// recent `arrange` pass. See `UIElementImpl`'s own doc comment.
    fn actual_width(&self) -> f32 {
        self.as_ui_element().actual_width.get()
    }
    fn actual_height(&self) -> f32 {
        self.as_ui_element().actual_height.get()
    }
    fn actual_offset(&self) -> Point {
        self.as_ui_element().actual_offset.get()
    }
    /// Post-construction setters (docs/elwindui_spec.md 付録H.2.1a) for every field this trait
    /// already exposes a getter for — declared here (not just as `UIElementImpl`'s own inherent
    /// methods) so they're reachable generically through `dyn UIElement`/any bound on this trait,
    /// not only through the concrete backing struct.
    fn set_margin(&self, margin: f32) {
        self.as_ui_element().set_margin(margin);
    }
    fn set_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.as_ui_element().set_horizontal_alignment(alignment);
    }
    fn set_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.as_ui_element().set_vertical_alignment(alignment);
    }
    fn set_visibility(&self, visibility: Visibility) {
        self.as_ui_element().set_visibility(visibility);
    }
    fn set_width(&self, width: Option<f32>) {
        self.as_ui_element().set_width(width);
    }
    fn set_height(&self, height: Option<f32>) {
        self.as_ui_element().set_height(height);
    }
    fn set_min_width(&self, min_width: Option<f32>) {
        self.as_ui_element().set_min_width(min_width);
    }
    fn set_min_height(&self, min_height: Option<f32>) {
        self.as_ui_element().set_min_height(min_height);
    }
    fn set_max_width(&self, max_width: Option<f32>) {
        self.as_ui_element().set_max_width(max_width);
    }
    fn set_max_height(&self, max_height: Option<f32>) {
        self.as_ui_element().set_max_height(max_height);
    }
    fn set_data_context(&self, data_context: Option<Rc<dyn Any>>) {
        self.as_ui_element().set_data_context(data_context);
    }
    /// WinUI3's `FrameworkElement.DataContext` — an ambient, type-erased data value an element
    /// carries (set explicitly via the `data_context:` common attribute, or populated internally by
    /// `TabView`'s `items_source` mode for each generated `TabViewItem`). `None` when unset.
    fn data_context(&self) -> Option<Rc<dyn Any>> {
        self.as_ui_element().data_context.borrow().clone()
    }
    /// WinUI3's `VisualTreeHelper.GetParent` — `None` for the root of whatever tree this element
    /// is currently part of. See `UIElementImpl::parent`'s doc comment.
    fn parent(&self) -> Option<Rc<dyn UIElement>> {
        self.as_ui_element().parent.borrow().as_ref().and_then(|p| p.upgrade())
    }
    /// This element's own children in the **Visual tree** (WinUI3's own Visual-tree children,
    /// docs/elwindui_spec.md 付録H.2.2) — the only tree any code ever actually walks (there is no
    /// separate, generically-traversable Logical tree data structure; some components merely *have*
    /// Logical-tree-shaped children of their own — see `UIElementCollection`). A default method,
    /// not overridden by any concrete type: it reads `self.as_ui_element().visual_children` directly, which
    /// is empty for a leaf like `NativeControlImpl`/`TextBlockImpl`/`ShapeImpl` and populated for a
    /// container (`LayoutImpl`/`ControlImpl`/`GridImpl`) via that same `UIElementImpl`'s
    /// `children_collection()`-derived `UIElementCollection`. Returns an owned `Vec` (each
    /// `Rc<dyn UIElement>` cheaply cloned, a refcount bump), not `&[..]`: the underlying storage is
    /// `RefCell`-backed (mutable at any time via `UIElementCollection`'s `add`/`remove`/etc.), and a
    /// `std::cell::Ref` guard can't be smuggled out through a bare reference tied to `&self`.
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.as_ui_element().visual_children.to_vec()
    }
    /// WinUI3's `GetType().Name` (via `.NET` reflection), commonly paired with `VisualTreeHelper`
    /// when dumping/debugging a tree — see `crate::visual_tree`. A default method, not overridden by
    /// any concrete type: `std::any::type_name::<Self>()` is monomorphized per implementor, so this
    /// resolves to the real concrete type (`ButtonImpl`/`TextBlockImpl`/...) even when called through
    /// `dyn UIElement`.
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    /// This element's own desired size, given `available` (margin already excluded by the caller)
    /// and its children's already-measured sizes (WinUI3's `MeasureOverride`). Defaults to taking
    /// no space at all — every concrete leaf/container overrides this with real logic; nothing
    /// currently relies on this default actually being invoked.
    fn measure_override(&self, _available: Size, _child_sizes: &[Size]) -> Size {
        Size { width: 0.0, height: 0.0 }
    }
    /// The rect to assign each child (in this element's own local coordinate space), given the
    /// final size this element itself was assigned (WinUI3's `ArrangeOverride`). Defaults to no
    /// children — see `measure_override`'s own doc comment.
    fn arrange_override(&self, _final_size: Size, _child_sizes: &[Size]) -> Vec<Rect> {
        Vec::new()
    }
    /// Content this element paints for itself, if any (`None` for pure layout containers like
    /// `LayoutImpl`, which only position children and draw nothing on their own account).
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
    fn try_as_native_control(&self) -> Option<&dyn Any> {
        None
    }
    /// WinUI3's `UIElement.InvalidateVisual`-equivalent — asks whatever host owns this element's
    /// tree to redraw. `invalidate`/`invalidate_arrange`/`invalidate_measure` are kept as three
    /// separate WinUI3-shaped entry points, but this crate's layout engine has no per-element
    /// measure/arrange cache yet (`layout_tree` always recomputes the whole tree from scratch — see
    /// its own doc comment), so all three currently resolve to the exact same action: a full
    /// re-`layout_tree` pass via `request_relayout`. Splitting them for real (e.g. skipping
    /// `measure_override` when only `arrange` is dirty) is future work once such a cache exists.
    /// A no-op if this element isn't (yet, or anymore) part of a hosted tree.
    fn invalidate(&self) {
        request_relayout(self.as_ui_element());
    }
    /// WinUI3's `UIElement.InvalidateArrange` — see `invalidate`'s own doc comment for why this
    /// currently triggers the same full relayout as the other two methods here.
    fn invalidate_arrange(&self) {
        request_relayout(self.as_ui_element());
    }
    /// WinUI3's `UIElement.InvalidateMeasure` — see `invalidate`'s own doc comment for why this
    /// currently triggers the same full relayout as the other two methods here.
    fn invalidate_measure(&self) {
        request_relayout(self.as_ui_element());
    }
}

/// Shared implementation for `UIElement::invalidate`/`invalidate_arrange`/`invalidate_measure` —
/// walks from `base`'s own element up to the root of whatever tree it's currently part of
/// (`UIElement::parent`, repeated until `None`) and, if that root has a `RelayoutHost` registered
/// (see `UIElementImpl::invalidate_host`), asks it for a fresh layout pass. Takes `&UIElementImpl`
/// (not `&dyn UIElement`) so the caller — a default trait method, where `Self` isn't known to be
/// `Sized` — never needs to unsize-coerce `self` itself; `base.self_handle` already stores a
/// pre-erased `Weak<dyn UIElement>` for exactly this reason. A no-op if `base`'s owner hasn't gone
/// through `new_element` yet (no `self_handle` to start the walk from) or if the root it finds has
/// no registered host (e.g. a standalone tree built for a test, never handed to a real backend).
fn request_relayout(base: &UIElementImpl) {
    let Some(mut current) = base.self_handle.borrow().as_ref().and_then(|w| w.upgrade()) else {
        return;
    };
    while let Some(parent) = current.parent() {
        current = parent;
    }
    if let Some(host) = current.as_ui_element().invalidate_host.borrow().as_ref() {
        host.request_relayout();
    }
}

/// The single choke point every construction site (`elwindui-codegen`'s generated code, and any
/// hand-written builtin) goes through instead of calling `Rc::new` directly — wires each of
/// `value`'s own children's `UIElementImpl::parent` back-reference to the freshly-created `Rc`
/// before handing it back. See this module's own top doc comment.
pub fn new_element<T: UIElement + 'static>(value: T) -> Rc<dyn UIElement> {
    new_element_concrete(value)
}

/// Same wiring as [`new_element`] (`self_handle`/each child's `parent` back-reference), but keeps
/// the returned `Rc` at its own concrete type `T` instead of erasing it to `Rc<dyn UIElement>` —
/// needed wherever the caller still wants to call `T`'s own inherent/trait methods on the result
/// (e.g. a virtual builtin kept on `Self` as a `stored` resync target, docs/elwindui_spec.md
/// 付録H.2.1a, rather than immediately erased into a parent's child list). **Every** construction
/// site must go through this (or [`new_element`]) rather than a bare `Rc::new` — skipping it leaves
/// `self_handle` unset, which makes `invalidate`/`invalidate_measure`/`invalidate_arrange` silent
/// no-ops on that element (`request_relayout`'s own doc comment) and leaves every child added to it
/// *before* this call with no `parent` back-reference either.
pub fn new_element_concrete<T: UIElement + 'static>(value: T) -> Rc<T> {
    let this = Rc::new(value);
    let erased: Rc<dyn UIElement> = this.clone();
    *erased.as_ui_element().self_handle.borrow_mut() = Some(Rc::downgrade(&erased));
    for child in erased.visual_children() {
        *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&erased));
    }
    this
}

/// The Visual tree's actual child storage (WinUI3's own `VisualCollection`, the low-level
/// counterpart to `Panel.Children`'s `UIElementCollection` below) — a plain, runtime-mutable
/// `add`/`insert`/`remove`/`remove_at`/`clear` collection. `UIElementImpl::visual_children` holds
/// one of these directly; `UIElement::visual_children()` (the default trait method) just reads it.
/// Doesn't itself know about parent-wiring (`UIElementCollection` below adds that) — on its own,
/// this is nothing more than a shared, interior-mutable `Vec<Rc<dyn UIElement>>`.
#[derive(Clone)]
pub struct VisualCollection {
    storage: Rc<RefCell<Vec<Rc<dyn UIElement>>>>,
}

impl VisualCollection {
    pub fn new() -> Self {
        VisualCollection { storage: Rc::new(RefCell::new(Vec::new())) }
    }
    pub fn add(&self, child: Rc<dyn UIElement>) {
        self.storage.borrow_mut().push(child);
    }
    pub fn insert(&self, index: usize, child: Rc<dyn UIElement>) {
        self.storage.borrow_mut().insert(index, child);
    }
    /// Removes the first entry pointer-equal to `child`, if any — returns whether one was found.
    pub fn remove(&self, child: &Rc<dyn UIElement>) -> bool {
        let mut storage = self.storage.borrow_mut();
        match storage.iter().position(|c| Rc::ptr_eq(c, child)) {
            Some(index) => {
                storage.remove(index);
                true
            }
            None => false,
        }
    }
    pub fn remove_at(&self, index: usize) -> Rc<dyn UIElement> {
        self.storage.borrow_mut().remove(index)
    }
    pub fn clear(&self) {
        self.storage.borrow_mut().clear();
    }
    pub fn len(&self) -> usize {
        self.storage.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.storage.borrow().is_empty()
    }
    pub fn to_vec(&self) -> Vec<Rc<dyn UIElement>> {
        self.storage.borrow().clone()
    }
}

impl Default for VisualCollection {
    fn default() -> Self {
        Self::new()
    }
}

/// The Logical-tree-shaped child list a container (`Layout`/`Control` family) declares in
/// `.elwind` — WinUI3's own `UIElementCollection` (docs/elwindui_spec.md 付録H.2.2), e.g.
/// `Panel.Children`. There is no separate, generically-traversable Logical tree: this is simply the
/// convenience API a *particular* component exposes for its own children, which automatically stays
/// in sync with the real Visual tree — `add`/`insert`/`remove`/`remove_at`/`clear` all mutate the
/// exact same storage `UIElement::visual_children()` reads (`self.visual`, shared with the owning
/// `UIElementImpl` via `UIElementImpl::children_collection`), and additionally keep each affected
/// child's own `parent` pointer correct: `add`/`insert` set it to the owner (if the owner has
/// already been `Rc`-wrapped by `new_element` — otherwise `new_element`'s own initial wiring pass
/// handles it once construction finishes), `remove`/`remove_at`/`clear` clear it back to `None`.
/// Deliberately has no way to replace its storage wholesale (no `set_children`) — every mutation
/// goes through one of these add/remove operations, so the Visual tree can never silently drift out
/// of sync with whatever a container thinks its own children are.
#[derive(Clone)]
pub struct UIElementCollection {
    visual: VisualCollection,
    owner: Rc<RefCell<Option<Weak<dyn UIElement>>>>,
}

impl UIElementCollection {
    fn owner_rc(&self) -> Option<Rc<dyn UIElement>> {
        self.owner.borrow().as_ref().and_then(|w| w.upgrade())
    }
    pub fn add(&self, child: Rc<dyn UIElement>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        self.visual.add(child);
    }
    pub fn insert(&self, index: usize, child: Rc<dyn UIElement>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        self.visual.insert(index, child);
    }
    pub fn remove(&self, child: &Rc<dyn UIElement>) -> bool {
        let removed = self.visual.remove(child);
        if removed {
            *child.as_ui_element().parent.borrow_mut() = None;
        }
        removed
    }
    pub fn remove_at(&self, index: usize) -> Rc<dyn UIElement> {
        let child = self.visual.remove_at(index);
        *child.as_ui_element().parent.borrow_mut() = None;
        child
    }
    pub fn clear(&self) {
        for child in self.visual.to_vec() {
            *child.as_ui_element().parent.borrow_mut() = None;
        }
        self.visual.clear();
    }
    pub fn len(&self) -> usize {
        self.visual.len()
    }
    pub fn is_empty(&self) -> bool {
        self.visual.is_empty()
    }
    pub fn to_vec(&self) -> Vec<Rc<dyn UIElement>> {
        self.visual.to_vec()
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
    Text { content: String, color: Option<String>, alignment: TextAlignment },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeKind {
    RoundedRect { corner_radius: f32 },
    Oval,
}

/// WinUI3's `TextBlock.TextAlignment` — how `TextBlockImpl`'s own content is aligned *within its
/// own drawn bounds*, independent of `UIElement::horizontal_alignment` (which positions the
/// `TextBlock` element itself within whatever slot its parent allotted it). Deliberately a separate
/// enum from `crate::layout::HorizontalAlignment` rather than reused: WinUI3 itself keeps these as
/// two distinct types (`Microsoft.UI.Xaml.TextAlignment` vs `HorizontalAlignment`), and
/// `HorizontalAlignment::Stretch` has no meaningful counterpart for text alignment. Only `Left`/
/// `Center`/`Right` are modeled — WinUI3's `Justify`/`DetectFromContent` are out of scope for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}

/// `Button`/`TextArea`/`MenuBar`/`TabView` (the "NativeControlImpl" family) — the only `UIElement`
/// with a real backend handle. Always a leaf as far as this tree is concerned: whatever lives
/// beneath it in its own backend-managed hierarchy (e.g. `TabView`'s tab-switching) is opaque here.
/// `NativeControl<H>`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) has no methods beyond
/// `UIElement` today — declared via `#[elwindui_macros::class]` purely so the type participates in
/// the trait+`Impl`+`base` convention like every other class in this hierarchy.
#[elwindui_macros::class(inherits = UIElement)]
pub struct NativeControl<H> {
    pub handle: H,
}

#[elwindui_macros::class]
impl<H: LayoutNode + 'static> NativeControl<H> {
    fn measure_override(&self, available: Size, _child_sizes: &[Size]) -> Size {
        self.handle.measure(available)
    }
    fn arrange_override(&self, _final_size: Size, _child_sizes: &[Size]) -> Vec<Rect> {
        Vec::new()
    }
    fn try_as_native_control(&self) -> Option<&dyn Any> {
        Some(self)
    }
    pub fn new(handle: H) -> Self {
        Self { base: UIElementImpl::default(), handle }
    }
}

pub fn create_native_control<H: LayoutNode + 'static>(handle: H) -> NativeControlImpl<H> {
    NativeControlImpl::new(handle)
}

/// The property-setter traits below (`TextArea`/`Button`/`MenuItem`/`Menu`/`MenuBar`/`MenuBarItem`/
/// `Window`) are declared once here rather than duplicated per backend crate — every backend's own
/// hand-written `XImpl` (`elwindui-backend-appkit`/`elwindui-backend-winui3`) had been independently
/// declaring an identically-shaped trait (same method signatures, same doc-comment rationale)
/// purely because Rust has no cross-crate trait sharing without a common home for it; this is that
/// home. Each backend crate now just provides `impl Xxx for BackendXImpl { .. }` — the property
/// *shape* (what setters exist, what they take) is common to every backend, only the method
/// *bodies* (the actual platform API calls) differ, exactly the same split
/// `NativeControl<H>`/`Layout`/`Shape`/`Control`/etc. above already model for the virtual builtins.
///
/// `Menu`/`MenuBar`/`MenuBarItem`/`Window` are *not* generic over the backend's own concrete
/// menu-entry/menu-bar-entry/menu/menu-bar type the way `NativeControlImpl<H>`'s `H` is — instead
/// each such argument is `&dyn` (or `Rc<dyn>`) the matching leaf trait itself (`MenuItem`/`Menu`/
/// `MenuBarItem`/`MenuBar`), and each backend's own `impl Xxx for BackendXImpl` downcasts it back to
/// its own concrete type via `AsAny::as_any` (see that trait's own doc comment; already the
/// established pattern for `UIElement::try_as_native_control`/`visual_tree::find_all`) before
/// delegating to its real native handle.
///
/// `TabView`/`TabViewItem` are deliberately **not** included here: their own methods
/// (`insert_tab`/`remove_tab`/`set_tab_content_visible`, an owned content host handle per platform)
/// are genuinely different in shape per backend (AppKit's `Retained<TreeHostView>`/`TabChipImpl` vs
/// WinUI3's own equivalents have no common signature to share without associated types this crate
/// doesn't need yet) — each backend keeps declaring its own local `TabView` trait.
pub trait TextArea: UIElement {
    fn set_text(&self, text: &str);
    fn set_on_change(&self, callback: Box<dyn Fn(String)>);
}

pub trait Button: UIElement {
    fn set_enabled(&self, enabled: bool);
    fn set_on_click(&self, callback: Box<dyn Fn()>);
    fn set_text(&self, text: &str);
}

pub trait MenuItem: AsAny {
    fn set_text(&self, text: &str);
    fn set_enabled(&self, enabled: bool);
    fn set_shortcut(&self, key_equivalent: &str);
    fn set_on_select(&self, callback: Box<dyn Fn()>);
}

pub trait Menu: AsAny {
    fn add_item(&self, item: &dyn MenuItem);
    fn remove_item(&self, item: &dyn MenuItem);
}

pub trait MenuBarItem: AsAny {
    fn set_text(&self, text: &str);
    fn set_submenu(&self, submenu: &dyn Menu);
}

pub trait MenuBar: AsAny {
    fn add_item(&self, item: &dyn MenuBarItem);
    fn remove_item(&self, item: &dyn MenuBarItem);
}

/// `Window`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — also the `component X inherits
/// Window` (host-composition) bare name every backend's own `WindowImpl` implements.
/// `set_menu_bar`'s `Rc<dyn MenuBar>` follows the same trait-object-argument convention as
/// `Menu`/`MenuBar`/`MenuBarItem` just above (see this module's own doc comment on that group) —
/// `impl Window for WindowImpl` downcasts it back to its own concrete `MenuBarImpl` internally.
pub trait Window {
    fn set_title(&self, title: &str);
    fn set_menu_bar(&self, menu_bar: Rc<dyn MenuBar>);
    fn set_content(&self, content: Rc<dyn UIElement>);
    fn show(&self);
    fn left(&self) -> f32;
    fn set_left(&self, left: f32);
    fn top(&self) -> f32;
    fn set_top(&self, top: f32);
    fn width(&self) -> f32;
    fn set_width(&self, width: f32);
    fn height(&self) -> f32;
    fn set_height(&self, height: f32);
}

/// `Layout`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — empty marker over `UIElement`,
/// implemented by every layout-container virtual builtin (`VerticalLayoutImpl`/
/// `HorizontalLayoutImpl`/`GridImpl`), the same way `NativeControl<H>` groups every native leaf.
///
/// Shared fields behind `VerticalLayout`/`HorizontalLayout`. `VerticalLayoutImpl`/
/// `HorizontalLayoutImpl` each embed one as `base` and implement `UIElement`/`Layout` themselves,
/// doing their own layout math directly against `elwindui_core::layout`'s `stack_arrange`/
/// `stack_natural_size` free functions with their own fixed `Orientation` literal — neither
/// delegates its `measure_override`/`arrange_override` to this struct's own (trivial, "take no
/// space" — see `UIElement::measure_override`'s own doc comment) default, since the orientation
/// (and so the entire layout algorithm) is a property of *which concrete type this is*, not of
/// shared state a common base could hold.
///
/// `abstract_class`: `Layout` itself is never instantiated — only its concrete subclasses
/// (`VerticalLayout`/`HorizontalLayout`) are, each building its own `LayoutImpl` base inline in its
/// own `new()`, the same way every other leaf/container builds its own `UIElementImpl::default()`
/// inline rather than through a shared factory (see e.g. `Shape::new`/`Control::new`).
#[elwindui_macros::class(inherits = UIElement, abstract_class)]
pub struct Layout {
    pub spacing: Cell<f32>,
    /// Shares its storage with `base.visual_children` (`UIElementImpl::children_collection`) —
    /// `UIElement::visual_children()`'s default implementation already reads that storage directly,
    /// so no override is needed here.
    pub children: UIElementCollection,
}

#[elwindui_macros::class]
impl Layout {
    /// Kept off the `Layout` trait itself (`#[inherent]`) — `VerticalLayout`/`HorizontalLayout`
    /// already expose their own `set_spacing` that forwards to this one inherent method, so making
    /// it a `Layout`-trait-required method too would just duplicate the same signature on both
    /// levels of the hierarchy.
    #[inherent]
    pub fn set_spacing(&self, spacing: f32) {
        self.spacing.set(spacing);
        self.invalidate();
    }
}

/// `VerticalLayoutImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a).
#[elwindui_macros::class(inherits = Layout)]
pub struct VerticalLayout {}

#[elwindui_macros::class]
impl VerticalLayout {
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        stack_natural_size(Orientation::Vertical, self.base.spacing.get(), child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        stack_arrange(final_size, Orientation::Vertical, self.base.spacing.get(), child_sizes)
    }
    fn set_spacing(&self, spacing: f32) {
        self.base.set_spacing(spacing);
    }
    pub fn children(&self) -> &UIElementCollection {
        &self.base.children
    }
    fn new() -> Self {
        let base = UIElementImpl::default();
        let children = base.children_collection();
        Self { base: LayoutImpl { base, spacing: Cell::new(0.0), children } }
    }
}

pub fn create_vertical_layout() -> VerticalLayoutImpl {
    VerticalLayoutImpl::new()
}

/// `HorizontalLayoutImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a).
#[elwindui_macros::class(inherits = Layout)]
pub struct HorizontalLayout {}

#[elwindui_macros::class]
impl HorizontalLayout {
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        stack_natural_size(Orientation::Horizontal, self.base.spacing.get(), child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        stack_arrange(final_size, Orientation::Horizontal, self.base.spacing.get(), child_sizes)
    }
    fn set_spacing(&self, spacing: f32) {
        self.base.set_spacing(spacing);
    }
    pub fn children(&self) -> &UIElementCollection {
        &self.base.children
    }
    fn new() -> Self {
        let base = UIElementImpl::default();
        let children = base.children_collection();
        Self { base: LayoutImpl { base, spacing: Cell::new(0.0), children } }
    }
}

pub fn create_horizontal_layout() -> HorizontalLayoutImpl {
    HorizontalLayoutImpl::new()
}

/// `Rectangle`/`Ellipse`. A pure leaf, like `TextBlockImpl` — no children of its own (matching real
/// WinUI3's `Shape`, which likewise has no `Children`/content property; see docs/elwindui_spec.md
/// 付録H.2.2), so its natural size is just its own drawn bounds.
/// `ShapeImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a); `Shape` has no further
/// DSL-level subclass today.
#[elwindui_macros::class(inherits = UIElement)]
pub struct Shape {
    pub kind: Cell<ShapeKind>,
    pub fill: RefCell<Option<String>>,
    pub stroke: RefCell<Option<String>>,
    pub stroke_width: Cell<f32>,
}

#[elwindui_macros::class]
impl Shape {
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
    fn set_kind(&self, kind: ShapeKind) {
        self.kind.set(kind);
        self.invalidate();
    }
    fn set_fill(&self, fill: Option<String>) {
        *self.fill.borrow_mut() = fill;
        self.invalidate();
    }
    fn set_stroke(&self, stroke: Option<String>) {
        *self.stroke.borrow_mut() = stroke;
        self.invalidate();
    }
    fn set_stroke_width(&self, stroke_width: f32) {
        self.stroke_width.set(stroke_width);
        self.invalidate();
    }
    fn new() -> Self {
        Self {
            base: UIElementImpl::default(),
            kind: Cell::new(ShapeKind::RoundedRect { corner_radius: 0.0 }),
            fill: RefCell::new(None),
            stroke: RefCell::new(None),
            stroke_width: Cell::new(0.0),
        }
    }
}

pub fn create_shape() -> ShapeImpl {
    ShapeImpl::new()
}

/// `builtin::Rectangle`(docs/elwindui_builtins_spec.md 付録G/N)— `ShapeKind::RoundedRect` に固定
/// した `Shape` の薄いラッパー。かつては `elwindui-codegen`(`builtins.elwind`の`view Rectangle`)が
/// 消費クレートごとに再生成していたが、バックエンド非依存な合成 builtin はここに一度だけ手書きする
/// 方が二重管理にならない。`#[ancestor]`(`elwindui_macros::class`の doc comment 参照)で`Shape`
/// 自身の必須メソッド(`set_kind`等)を`base`委譲として登録している。
#[elwindui_macros::class(inherits = Shape)]
pub struct Rectangle {
    stroke_width: Option<f32>,
    corner_radius: Option<f32>,
}

#[elwindui_macros::class]
impl Rectangle {
    fn fill(&self) -> Option<String> {
        self.base.fill.borrow().clone()
    }
    fn stroke(&self) -> Option<String> {
        self.base.stroke.borrow().clone()
    }
    fn stroke_width(&self) -> Option<f32> {
        self.stroke_width.clone()
    }
    fn corner_radius(&self) -> Option<f32> {
        self.corner_radius.clone()
    }
    #[ancestor]
    fn set_kind(&self, kind: ShapeKind) {
        self.base.set_kind(kind)
    }
    #[ancestor]
    fn set_fill(&self, fill: Option<String>) {
        self.base.set_fill(fill)
    }
    #[ancestor]
    fn set_stroke(&self, stroke: Option<String>) {
        self.base.set_stroke(stroke)
    }
    #[ancestor]
    fn set_stroke_width(&self, stroke_width: f32) {
        self.base.set_stroke_width(stroke_width)
    }
    fn paint(&self) -> Option<PaintKind> {
        self.base.paint()
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElement> {
        self
    }
    fn new(fill: Option<String>, stroke: Option<String>, stroke_width: Option<f32>, corner_radius: Option<f32>) -> Rc<Self> {
        Rc::new(create_rectangle(fill, stroke, stroke_width, corner_radius))
    }
}

/// The plain (not `Rc`-wrapped) value `RectangleImpl::new` itself wraps — also what a future
/// `component X inherits Rectangle` would embed unwrapped as its own `base` field, mirroring
/// `create_control`/`create_shape`'s own role for `Control`/`Shape` (`Rectangle` is `#[sealed]`
/// today, so nothing actually reaches this via that path yet, but the shape stays consistent with
/// every other builtin's `create_xxx`/`XxxImpl::new` split).
pub fn create_rectangle(
    fill: Option<String>,
    stroke: Option<String>,
    stroke_width: Option<f32>,
    corner_radius: Option<f32>,
) -> RectangleImpl {
    let shape = create_shape();
    shape.set_kind(ShapeKind::RoundedRect { corner_radius: corner_radius.unwrap_or(0.0) });
    shape.set_fill(fill);
    shape.set_stroke(stroke);
    shape.set_stroke_width(stroke_width.unwrap_or(0.0));
    RectangleImpl { base: shape, stroke_width, corner_radius }
}

/// `builtin::Ellipse`(docs/elwindui_builtins_spec.md 付録G/N)— `ShapeKind::Oval` に固定した
/// `Shape` の薄いラッパー。`RectangleImpl`の doc comment 参照。
#[elwindui_macros::class(inherits = Shape)]
pub struct Ellipse {
    stroke_width: Option<f32>,
}

#[elwindui_macros::class]
impl Ellipse {
    fn fill(&self) -> Option<String> {
        self.base.fill.borrow().clone()
    }
    fn stroke(&self) -> Option<String> {
        self.base.stroke.borrow().clone()
    }
    fn stroke_width(&self) -> Option<f32> {
        self.stroke_width.clone()
    }
    #[ancestor]
    fn set_kind(&self, kind: ShapeKind) {
        self.base.set_kind(kind)
    }
    #[ancestor]
    fn set_fill(&self, fill: Option<String>) {
        self.base.set_fill(fill)
    }
    #[ancestor]
    fn set_stroke(&self, stroke: Option<String>) {
        self.base.set_stroke(stroke)
    }
    #[ancestor]
    fn set_stroke_width(&self, stroke_width: f32) {
        self.base.set_stroke_width(stroke_width)
    }
    fn paint(&self) -> Option<PaintKind> {
        self.base.paint()
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElement> {
        self
    }
    fn new(fill: Option<String>, stroke: Option<String>, stroke_width: Option<f32>) -> Rc<Self> {
        Rc::new(create_ellipse(fill, stroke, stroke_width))
    }
}

/// The plain (not `Rc`-wrapped) value `EllipseImpl::new` itself wraps — see `create_rectangle`'s
/// own doc comment for why this split exists.
pub fn create_ellipse(fill: Option<String>, stroke: Option<String>, stroke_width: Option<f32>) -> EllipseImpl {
    let shape = create_shape();
    shape.set_kind(ShapeKind::Oval);
    shape.set_fill(fill);
    shape.set_stroke(stroke);
    shape.set_stroke_width(stroke_width.unwrap_or(0.0));
    EllipseImpl { base: shape, stroke_width }
}

/// Self-drawn primitive text (WinUI3's `TextBlockImpl`) — no native widget, unlike the native `Text`
/// this replaces. A leaf, like `NativeControlImpl`. Field named `text` (not `content`, unlike
/// `PaintKind::Text`'s own field of the same meaning) to match `builtin::TextBlock`'s own `#[param]
/// text` name — `elwindui-codegen`'s setter-based construction calls `.set_{param name}(..)`
/// generically, so the Rust field/setter name must agree with the DSL's own field name.
/// `TextBlockImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a); `TextBlock` has no
/// further DSL-level subclass today.
#[elwindui_macros::class(inherits = UIElement)]
pub struct TextBlock {
    pub text: RefCell<String>,
    pub color: RefCell<Option<String>>,
    pub alignment: Cell<TextAlignment>,
}

#[elwindui_macros::class]
impl TextBlock {
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
        Some(PaintKind::Text { content: self.text.borrow().clone(), color: self.color.borrow().clone(), alignment: self.alignment.get() })
    }
    fn set_text(&self, text: String) {
        *self.text.borrow_mut() = text;
        self.invalidate();
    }
    fn set_color(&self, color: Option<String>) {
        *self.color.borrow_mut() = color;
        self.invalidate();
    }
    fn set_text_alignment(&self, alignment: TextAlignment) {
        self.alignment.set(alignment);
        self.invalidate();
    }
    fn new() -> Self {
        Self {
            base: UIElementImpl::default(),
            text: RefCell::new(String::new()),
            color: RefCell::new(None),
            alignment: Cell::new(TextAlignment::Left),
        }
    }
}

pub fn create_text_block() -> TextBlockImpl {
    TextBlockImpl::new()
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
/// replacement is future work.
/// `ControlImpl`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — exposes the fields a
/// DSL-level subclass composed via `base: ControlImpl` (e.g. `builtin::ContentControl`,
/// `crates/elwindui-builtins/src/builtins.elwind`) delegates to.
#[elwindui_macros::class(inherits = UIElement)]
pub struct Control {
    pub padding: Cell<f32>,
    pub content_horizontal_alignment: Cell<HorizontalAlignment>,
    pub content_vertical_alignment: Cell<VerticalAlignment>,
    /// Shares its storage with `base.visual_children` (`UIElementImpl::children_collection`) — see
    /// `LayoutImpl::children`'s own doc comment.
    pub children: UIElementCollection,
}

#[elwindui_macros::class]
impl Control {
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
    fn padding(&self) -> f32 {
        self.padding.get()
    }
    fn content_horizontal_alignment(&self) -> HorizontalAlignment {
        self.content_horizontal_alignment.get()
    }
    fn content_vertical_alignment(&self) -> VerticalAlignment {
        self.content_vertical_alignment.get()
    }
    fn set_padding(&self, padding: f32) {
        self.padding.set(padding);
        self.invalidate();
    }
    fn set_content_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.content_horizontal_alignment.set(alignment);
        self.invalidate();
    }
    fn set_content_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.content_vertical_alignment.set(alignment);
        self.invalidate();
    }
    fn new() -> Self {
        let base = UIElementImpl::default();
        let children = base.children_collection();
        Self {
            base,
            padding: Cell::new(0.0),
            content_horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            content_vertical_alignment: Cell::new(VerticalAlignment::Stretch),
            children,
        }
    }
}

pub fn create_control() -> ControlImpl {
    ControlImpl::new()
}

/// `builtin::ContentControl`(docs/elwindui_spec.md 付録H.2.1a)— 単一の子(`content`)を持つ
/// `Control`の薄いラッパー。`RectangleImpl`の doc comment 参照(同じ理由でここに直接手書きする)。
/// `Control.children`は`UIElementImpl.visual_children`と同一のストレージを共有する
/// (`Control`構造体の`children`フィールドの doc comment 参照)ため、`Rectangle`/`Ellipse`と違い
/// `new()`は追加した子の親ポインタを実際に張り直す必要がある。
#[elwindui_macros::class(inherits = Control)]
pub struct ContentControl {
    padding: Option<f32>,
    content: Rc<dyn UIElement>,
}

#[elwindui_macros::class]
impl ContentControl {
    fn padding(&self) -> Option<f32> {
        self.padding.clone()
    }
    fn content(&self) -> Rc<dyn UIElement> {
        self.content.clone()
    }
    #[ancestor]
    fn padding(&self) -> f32 {
        self.base.padding()
    }
    #[ancestor]
    fn content_horizontal_alignment(&self) -> HorizontalAlignment {
        self.base.content_horizontal_alignment()
    }
    #[ancestor]
    fn content_vertical_alignment(&self) -> VerticalAlignment {
        self.base.content_vertical_alignment()
    }
    #[ancestor]
    fn set_padding(&self, padding: f32) {
        self.base.set_padding(padding)
    }
    #[ancestor]
    fn set_content_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.base.set_content_horizontal_alignment(alignment)
    }
    #[ancestor]
    fn set_content_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.base.set_content_vertical_alignment(alignment)
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElement> {
        self
    }
    fn new(padding: Option<f32>, content: Rc<dyn UIElement>) -> Rc<Self> {
        let this = Rc::new(create_content_control(padding, content));
        let erased: Rc<dyn UIElement> = this.clone();
        for child in this.visual_children() {
            *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&erased));
        }
        this
    }
}

/// The plain (not `Rc`-wrapped) value `ContentControlImpl::new` itself wraps — also what
/// `component X inherits ContentControl` (`RoundedPanel`/`DocumentView` in `examples/notepad`)
/// embeds unwrapped as its own `base` field, mirroring `create_control`/`create_shape`'s own role
/// for `Control`/`Shape`. Unlike `create_rectangle`/`create_ellipse`, this one genuinely is called
/// that way today (`ContentControl` isn't `#[sealed]`), by generated code that never goes through
/// `ContentControlImpl::new` at all — the parent-pointer wiring `new` does on top of this is only
/// needed when `content` is embedded directly as *this* value's own child; a shape-composing
/// subclass rewires it again itself once `content` becomes one of *its own* visual children.
pub fn create_content_control(padding: Option<f32>, content: Rc<dyn UIElement>) -> ContentControlImpl {
    let control = create_control();
    control.set_padding(padding.unwrap_or(0.0));
    control.children.add(content.clone());
    ContentControlImpl { base: control, padding, content }
}

/// WPF/WinUI3-style row/column layout (`builtin::Grid`, docs/elwindui_spec.md §3). Each child's
/// cell placement comes from its own `UIElementImpl::attached` bag (the `Grid::row`/`Grid::column`
/// attached properties it was constructed with, read back via `grid_cell_of` since only `Grid`
/// itself knows those two fields are `i32`), not a field on `GridImpl` itself — see `attached`'s
/// own doc comment. A child whose cell falls outside `row_definitions`/`column_definitions`'
/// bounds is clamped to the last row/column, mirroring `grid_arrange`'s own clamping. Row/column
/// spanning is out of scope for this pass (one child per cell) — a future `#[attached]
/// row_span`/`column_span` pair on `builtin::Grid` would extend this the same way `row`/`column`
/// were added, with no changes needed here beyond consulting the extra fields.
/// `rows`/`columns` (not `row_definitions`/`column_definitions`) to match `builtin::Grid`'s own
/// `#[param] rows`/`#[param] columns` names — `elwindui-codegen`'s setter-based construction calls
/// `.set_{param name}(..)` generically, so the Rust field/setter name must agree with the DSL's.
/// `GridImpl`'s own class trait — empty marker (docs/elwindui_spec.md 付録H.2.1a); `Grid` has no
/// further DSL-level subclass today.
/// Reads a child's `Grid::row`/`Grid::column` attached-property values back out of its
/// `UIElementImpl::attached` bag — `Grid` is the only thing that knows those two fields are `i32`
/// and default to `0`, so it (not `UIElementImpl`) owns this downcast, mirroring how
/// `elwindui-codegen`'s `emit_attached_setters` also resolves the field's declared type from the
/// owner (`Grid`) itself, never `UIElementImpl`.
fn grid_cell_of(child: &Rc<dyn UIElement>) -> GridCell {
    GridCell {
        row: child.as_ui_element().get_attached("Grid", "row", 0i32),
        column: child.as_ui_element().get_attached("Grid", "column", 0i32),
    }
}

#[elwindui_macros::class(inherits = UIElement)]
pub struct Grid {
    pub rows: RefCell<Vec<GridLength>>,
    pub columns: RefCell<Vec<GridLength>>,
    /// Shares its storage with `base.visual_children` (`UIElementImpl::children_collection`) — see
    /// `LayoutImpl::children`'s own doc comment.
    pub children: UIElementCollection,
}

#[elwindui_macros::class]
impl Grid {
    fn measure_override(&self, _available: Size, child_sizes: &[Size]) -> Size {
        let cells: Vec<GridCell> = self.children.to_vec().iter().map(grid_cell_of).collect();
        grid_natural_size(&self.rows.borrow(), &self.columns.borrow(), &cells, child_sizes)
    }
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect> {
        let cells: Vec<GridCell> = self.children.to_vec().iter().map(grid_cell_of).collect();
        grid_arrange(final_size, &self.rows.borrow(), &self.columns.borrow(), &cells, child_sizes)
    }
    fn set_rows(&self, rows: Vec<GridLength>) {
        *self.rows.borrow_mut() = rows;
        self.invalidate();
    }
    fn set_columns(&self, columns: Vec<GridLength>) {
        *self.columns.borrow_mut() = columns;
        self.invalidate();
    }
    fn new() -> Self {
        let base = UIElementImpl::default();
        let children = base.children_collection();
        Self { base, rows: RefCell::new(Vec::new()), columns: RefCell::new(Vec::new()), children }
    }
}

// `Grid`'s `base` is `UIElementImpl` directly (it has no `spacing`, so it skips `LayoutImpl`
// entirely, unlike `VerticalLayout`/`HorizontalLayout`) — `#[class(inherits = UIElement)]` above
// doesn't know to also mark it as a `Layout`, so that one-line empty marker stays hand-written.
impl Layout for GridImpl {}

pub fn create_grid() -> GridImpl {
    GridImpl::new()
}

/// WinUI3's `FrameworkElement.MeasureCore`-style constraint step, shared by `measure`/
/// `measure_and_align`: an explicit `width`/`height` overrides that axis outright, then both axes
/// are clamped to `min_width..max_width`/`min_height..max_height` (`crate::layout::
/// apply_size_constraints`). Applied twice per element per the same WinUI3 algorithm — once to the
/// space handed down to `measure_override` (a fixed `Width` shouldn't let a container measure
/// against the parent's *actual* available space), once to `measure_override`'s own returned size
/// (a container's natural content size shouldn't override an explicit `Width`/`Height`/`Max*`).
fn constrain(elem: &dyn UIElement, size: Size) -> Size {
    let overridden = Size { width: elem.width().unwrap_or(size.width), height: elem.height().unwrap_or(size.height) };
    apply_size_constraints(overridden, elem.min_width(), elem.max_width(), elem.min_height(), elem.max_height())
}

fn measure(elem: &dyn UIElement, available: Size) -> Size {
    // WinUI3's `Collapsed`: `DesiredSize` is unconditionally `(0, 0)`, ignoring margin/children/
    // explicit `Width`/`Height` entirely — see `Visibility`'s own doc comment. Known limitation:
    // `stack_arrange` (`elwindui_core::layout`) still reserves a full `spacing` gap on either side
    // of a zero-sized `Collapsed` child, unlike WinUI3's `StackPanel.Spacing`, which only counts
    // visible children — out of scope for now.
    if elem.visibility() == Visibility::Collapsed {
        return Size { width: 0.0, height: 0.0 };
    }
    let inner_available = constrain(elem, shrink_by_margin(available, elem.margin()));
    let child_sizes: Vec<Size> = elem.visual_children().iter().map(|c| measure(c.as_ref(), inner_available)).collect();
    let desired = constrain(elem, elem.measure_override(inner_available, &child_sizes));
    grow_by_margin(desired, elem.margin())
}

/// `elem`'s own absolute rect and its children's, threaded through `natives`/`paints` just like
/// `arrange` below — factored out so `hit_test_at` (coordinate-only, no native handle) and
/// `arrange` (handle-collecting) can share the measure/align math without duplicating it.
fn measure_and_align(elem: &dyn UIElement, allotted: Rect) -> Rect {
    // See `measure`'s own `Collapsed` comment — same zero-size treatment, kept at `allotted`'s
    // own origin since there's no `desired`/alignment computation to run at all.
    if elem.visibility() == Visibility::Collapsed {
        return Rect { x: allotted.x, y: allotted.y, width: 0.0, height: 0.0 };
    }
    let slot = shrink_rect_by_margin(allotted, elem.margin());
    let slot_size = constrain(elem, Size { width: slot.width, height: slot.height });
    let child_sizes_for_measure: Vec<Size> = elem.visual_children().iter().map(|c| measure(c.as_ref(), slot_size)).collect();
    let desired = constrain(elem, elem.measure_override(slot_size, &child_sizes_for_measure));
    align_within(slot, desired, elem.horizontal_alignment(), elem.vertical_alignment())
}

fn arrange<H: Clone + 'static>(elem: &Rc<dyn UIElement>, allotted: Rect, out: &mut Vec<RenderItem<H>>) {
    // A `Collapsed` element neither renders itself nor recurses into its children — its whole
    // subtree is skipped, matching WinUI3 (a `Collapsed` parent hides its descendants too). See
    // `Visibility`'s own doc comment. `actual_offset` was already set by the parent's own loop
    // (below) before this call, so only the size needs zeroing here.
    if elem.visibility() == Visibility::Collapsed {
        elem.as_ui_element().actual_width.set(0.0);
        elem.as_ui_element().actual_height.set(0.0);
        return;
    }
    let final_rect = measure_and_align(elem.as_ref(), allotted);
    let final_size = Size { width: final_rect.width, height: final_rect.height };
    // WinUI3's `ActualWidth`/`ActualHeight` — the result of this element's own just-completed
    // Arrange, readable afterward via `UIElement::actual_width`/`actual_height`.
    elem.as_ui_element().actual_width.set(final_size.width);
    elem.as_ui_element().actual_height.set(final_size.height);

    // `try_as_native_control` (not a direct `as_any().downcast_ref` on `elem` itself) so a type that
    // *composes* a `NativeControlImpl<H>` as its own `base` field (e.g. a backend's `ButtonImpl`)
    // is recognized too — `Any::downcast_ref` only succeeds against the exact concrete type placed
    // in the tree, which for such a type is never literally `NativeControlImpl<H>` itself. See
    // `UIElement::try_as_native_control`'s own doc comment.
    if let Some(native) = elem.as_ref().try_as_native_control().and_then(|a| a.downcast_ref::<NativeControlImpl<H>>()) {
        out.push(RenderItem::Native(native.handle.clone(), final_rect, Rc::clone(elem)));
    }
    if let Some(paint) = elem.paint() {
        out.push(RenderItem::Paint(paint, final_rect));
    }

    let child_sizes: Vec<Size> = elem.visual_children().iter().map(|c| measure(c.as_ref(), final_size)).collect();
    let child_rects = elem.arrange_override(final_size, &child_sizes);
    for (child, child_rect) in elem.visual_children().iter().zip(child_rects) {
        // WinUI3's `ActualOffset` — this child's own position relative to `elem` (its parent),
        // set here before its absolute rect (below) is computed for the recursive call.
        child.as_ui_element().actual_offset.set(Point { x: child_rect.x, y: child_rect.y });
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
    // A `Collapsed` element (and its whole subtree) is excluded from hit-testing, matching
    // `arrange`'s own treatment — see `Visibility`'s own doc comment.
    if elem.visibility() == Visibility::Collapsed {
        return None;
    }
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
        let handlers = elem.as_ui_element().routed_handlers.borrow();
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
        match orientation {
            Orientation::Vertical => {
                let node = create_vertical_layout();
                node.set_spacing(spacing);
                for child in children {
                    node.children().add(child);
                }
                new_element(node)
            }
            Orientation::Horizontal => {
                let node = create_horizontal_layout();
                node.set_spacing(spacing);
                for child in children {
                    node.children().add(child);
                }
                new_element(node)
            }
        }
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
            node.as_ui_element().set_horizontal_alignment(HorizontalAlignment::Left);
            node.as_ui_element().set_vertical_alignment(VerticalAlignment::Top);
            node
        }
        fn start_stack(orientation: Orientation, spacing: f32, children: Vec<Rc<dyn UIElement>>) -> Rc<dyn UIElement> {
            let node = match orientation {
                Orientation::Vertical => {
                    let stack = create_vertical_layout();
                    stack.set_spacing(spacing);
                    for child in children {
                        stack.children().add(child);
                    }
                    new_element(stack)
                }
                Orientation::Horizontal => {
                    let stack = create_horizontal_layout();
                    stack.set_spacing(spacing);
                    for child in children {
                        stack.children().add(child);
                    }
                    new_element(stack)
                }
            };
            node.as_ui_element().set_horizontal_alignment(HorizontalAlignment::Left);
            node.as_ui_element().set_vertical_alignment(VerticalAlignment::Top);
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
        control.children.add(native("a", size(10.0, 20.0)));
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
        tree.as_ui_element().set_margin(10.0);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(natives[0].1, Rect { x: 10.0, y: 10.0, width: 80.0, height: 80.0 });
    }

    #[test]
    fn explicit_width_and_height_override_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElement> = new_element(create_native_control(FakeHandle("a", size(10.0, 20.0))));
        tree.as_ui_element().set_width(Some(50.0));
        tree.as_ui_element().set_height(Some(5.0));
        // `Stretch` (the default) still governs slot placement; the explicit width/height above
        // constrains what `measure_override`'s own `available`/`desired` see, not the final
        // stretch-to-slot size — a non-`Stretch` alignment (below) is what actually surfaces the
        // explicit size in the arranged rect.
        tree.as_ui_element().set_horizontal_alignment(HorizontalAlignment::Left);
        tree.as_ui_element().set_vertical_alignment(VerticalAlignment::Top);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert_eq!(natives[0].1, Rect { x: 0.0, y: 0.0, width: 50.0, height: 5.0 });
    }

    #[test]
    fn min_and_max_clamp_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElement> = new_element(create_native_control(FakeHandle("a", size(10.0, 20.0))));
        tree.as_ui_element().set_min_width(Some(30.0));
        tree.as_ui_element().set_max_height(Some(8.0));
        tree.as_ui_element().set_horizontal_alignment(HorizontalAlignment::Left);
        tree.as_ui_element().set_vertical_alignment(VerticalAlignment::Top);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert_eq!(natives[0].1, Rect { x: 0.0, y: 0.0, width: 30.0, height: 8.0 });
    }

    #[test]
    fn actual_width_height_and_offset_are_populated_after_layout() {
        let leaf = native("a", size(10.0, 20.0));
        leaf.as_ui_element().set_horizontal_alignment(HorizontalAlignment::Left);
        leaf.as_ui_element().set_vertical_alignment(VerticalAlignment::Top);
        let root = stack(Orientation::Vertical, 5.0, vec![native("top", size(50.0, 10.0)), Rc::clone(&leaf)]);
        layout_tree::<FakeHandle>(&root, size(200.0, 200.0));

        assert_eq!(root.actual_width(), 200.0);
        assert_eq!(root.actual_height(), 200.0);
        assert_eq!(root.actual_offset(), Point { x: 0.0, y: 0.0 }, "root has no parent to set its own offset");
        // second stack child ("top" is 10 tall, spacing is 5) starts at y = 15, relative to the stack
        assert_eq!(leaf.actual_offset(), Point { x: 0.0, y: 15.0 });
        assert_eq!(leaf.actual_width(), 10.0);
        assert_eq!(leaf.actual_height(), 20.0);
    }

    #[test]
    fn non_stretch_alignment_keeps_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElement> = new_element(create_native_control(FakeHandle("a", size(10.0, 20.0))));
        tree.as_ui_element().set_horizontal_alignment(HorizontalAlignment::Center);
        tree.as_ui_element().set_vertical_alignment(VerticalAlignment::Center);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(natives[0].1, Rect { x: 45.0, y: 40.0, width: 10.0, height: 20.0 });
    }

    /// A minimal test-only fixture that both paints itself *and* has children — no real builtin
    /// combines the two today (`ShapeImpl` is a childless leaf; `LayoutImpl`/`ControlImpl`/`GridImpl`
    /// never paint), so `render_item_ordering_preserves_traversal_order_across_native_and_paint`
    /// (below) needs its own local type to exercise the paint-then-child traversal order.
    struct PaintingContainer {
        base: UIElementImpl,
    }

    impl UIElement for PaintingContainer {
        fn as_ui_element(&self) -> &UIElementImpl {
            &self.base
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
        let base = UIElementImpl::default();
        base.children_collection().add(native("child", size(10.0, 10.0)));
        let tree: Rc<dyn UIElement> = new_element(PaintingContainer { base });
        let items = layout_tree::<FakeHandle>(&tree, size(50.0, 50.0));
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], RenderItem::Paint(..)));
        assert!(matches!(items[1], RenderItem::Native(..)));
    }

    #[test]
    fn text_block_defaults_to_left_alignment_and_set_text_alignment_updates_paint() {
        let text_block = create_text_block();
        assert_eq!(text_block.alignment.get(), TextAlignment::Left);
        match text_block.paint() {
            Some(PaintKind::Text { alignment, .. }) => assert_eq!(alignment, TextAlignment::Left),
            other => panic!("expected PaintKind::Text, got {other:?}"),
        }

        text_block.set_text_alignment(TextAlignment::Center);
        match text_block.paint() {
            Some(PaintKind::Text { alignment, .. }) => assert_eq!(alignment, TextAlignment::Center),
            other => panic!("expected PaintKind::Text, got {other:?}"),
        }
    }

    #[test]
    fn child_parent_pointer_is_set_by_new_element() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        assert!(Rc::ptr_eq(&leaf.parent().expect("leaf should have a parent"), &root));
        assert!(root.parent().is_none());
    }

    #[test]
    fn runtime_add_and_remove_after_construction_wire_parent_and_visual_children() {
        // `UIElementCollection::add`/`remove` must work *after* the owner is already `Rc`-wrapped
        // (not just at construction time, when `new_element`'s own initial wiring pass would have
        // covered it) — this is the whole point of not having a wholesale `set_children` anymore.
        // `children` is cloned out *before* `new_element` erases the concrete `ControlImpl` into
        // `Rc<dyn UIElement>` — a cheap clone (two shared `Rc`s), and it keeps sharing the exact
        // same underlying storage as the erased value's own `base.visual_children` afterward.
        let control = create_control();
        let children = control.children.clone();
        let root: Rc<dyn UIElement> = new_element(control);
        assert!(root.visual_children().is_empty());

        let child = native("a", size(10.0, 20.0));
        children.add(Rc::clone(&child));

        assert_eq!(root.visual_children().len(), 1);
        assert!(Rc::ptr_eq(&child.parent().expect("add should wire the child's parent"), &root));

        assert!(children.remove(&child));
        assert!(root.visual_children().is_empty());
        assert!(child.parent().is_none(), "remove should clear the child's parent");
    }

    #[test]
    fn invalidate_family_reaches_a_relayout_host_registered_on_the_root() {
        struct CountingHost {
            calls: Rc<RefCell<usize>>,
        }
        impl RelayoutHost for CountingHost {
            fn request_relayout(&self) {
                *self.calls.borrow_mut() += 1;
            }
        }

        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);

        let calls = Rc::new(RefCell::new(0));
        root.as_ui_element().set_invalidate_host(Some(Rc::new(CountingHost { calls: Rc::clone(&calls) })));

        // Called from the *leaf*, not the root — must walk `parent()` up to find the registered host.
        leaf.invalidate();
        leaf.invalidate_arrange();
        leaf.invalidate_measure();
        assert_eq!(*calls.borrow(), 3);

        root.as_ui_element().set_invalidate_host(None);
        leaf.invalidate();
        assert_eq!(*calls.borrow(), 3, "un-registering the host should make invalidate a no-op again");
    }

    #[test]
    fn invalidate_on_an_unhosted_tree_is_a_no_op() {
        // No `RelayoutHost` registered anywhere on this tree — must not panic.
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        leaf.invalidate();
        root.invalidate_arrange();
    }

    #[test]
    fn dispatch_routed_bubbles_and_stops_at_handled() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);

        let leaf_calls = Rc::new(RefCell::new(0));
        let root_calls = Rc::new(RefCell::new(0));
        {
            let leaf_calls = Rc::clone(&leaf_calls);
            leaf.as_ui_element().register_routed_handler::<()>("on_click", Box::new(move |_, _| *leaf_calls.borrow_mut() += 1));
        }
        {
            let root_calls = Rc::clone(&root_calls);
            root.as_ui_element().register_routed_handler::<()>("on_click", Box::new(move |_, args| {
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

    #[test]
    fn collapsed_leaf_has_zero_size_and_produces_no_render_item() {
        let tree = native("a", size(10.0, 20.0));
        tree.as_ui_element().set_visibility(Visibility::Collapsed);
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
        assert_eq!(tree.actual_width(), 0.0);
        assert_eq!(tree.actual_height(), 0.0);
    }

    #[test]
    fn collapsed_child_is_excluded_from_stack_layout() {
        let collapsed = native("collapsed", size(50.0, 50.0));
        collapsed.as_ui_element().set_visibility(Visibility::Collapsed);
        let visible = native("visible", size(30.0, 10.0));
        visible.as_ui_element().set_horizontal_alignment(HorizontalAlignment::Left);
        visible.as_ui_element().set_vertical_alignment(VerticalAlignment::Top);
        let tree = stack(Orientation::Vertical, 5.0, vec![Rc::clone(&collapsed), Rc::clone(&visible)]);

        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        // Known limitation (see `Visibility`'s own doc comment / the layout engine's own comment
        // above `measure`): `stack_arrange` still reserves the 5.0 `spacing` gap around the
        // zero-sized collapsed child, so `visible` starts at y = 5.0, not y = 0.0.
        assert_eq!(natives, vec![(FakeHandle("visible", size(30.0, 10.0)), Rect { x: 0.0, y: 5.0, width: 30.0, height: 10.0 })]);
    }

    #[test]
    fn collapsed_containers_subtree_is_entirely_excluded() {
        let leaf = native("child", size(10.0, 10.0));
        let container = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        container.as_ui_element().set_visibility(Visibility::Collapsed);

        let (natives, paints) = split(layout_tree::<FakeHandle>(&container, size(100.0, 100.0)));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
        assert_eq!(leaf.visibility(), Visibility::Visible, "the child itself was never made Collapsed");
    }

    #[test]
    fn collapsed_element_is_excluded_from_hit_test() {
        let tree = native("a", size(10.0, 20.0));
        tree.as_ui_element().set_visibility(Visibility::Collapsed);
        assert!(hit_test(&tree, size(100.0, 100.0), Point { x: 5.0, y: 5.0 }).is_none());
    }
}
