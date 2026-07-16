//! The framework-owned Visual tree, following WinUI3's `UIElement` hierarchy: `Rc<dyn UIElement>`
//! nodes *are* the tree (no separate wrapper/enum type) — a backend's own `NativeControlImpl`
//! (`Button`/`TextArea`/`TabView`, the `NativeControl`-implementing family — see that trait's own
//! doc comment), `TextBlock` (self-drawn primitive text),
//! `Shape` (`Rectangle`/`Ellipse`), `VerticalLayout`/`HorizontalLayout` (each embedding
//! shared `Layout` fields as their own `base`, but doing their own orientation-specific layout
//! math directly rather than delegating it to that base), and `Control` (a composable
//! multi-part component) are all peer implementations of the same `UIElement` trait.
//! `Margin`/`HorizontalAlignment`/`VerticalAlignment` (`UIElement`) are common to every one of
//! them, applied generically by this module's `measure`/`arrange` (WinUI3's
//! `UIElement.Measure`/`Arrange` wrapping each type's own `MeasureOverride`/`ArrangeOverride`) —
//! see docs/elwindui_spec.md 付録H.2.
//!
//! `H` (whatever a backend uses as its native widget handle, e.g. `elwindui-backend-appkit`'s
//! `AnyView`) appears only while RenderTree builds or reconciles a native command,
//! `collect_render_items<H>`, downcasting a leaf's `try_as_native_control()` result straight to `H`)
//! — the `UIElement` trait and every other concrete type
//! (`VerticalLayout`/`HorizontalLayout`/`Shape`/`TextBlock`/`Control`) are
//! handle-agnostic, since they never hold one.
//!
//! `Window` is deliberately *not* a `UIElement` — like WinUI3's `Window`, it's a separate
//! top-level host that owns a `Rc<dyn UIElement>` (its content), drives `layout_root`, and
//! its own client area (see `elwindui-backend-appkit`'s `TreeHostView`).
//!
//! **Ownership: `Rc`, not `Box`.** Every node holds a real parent back-reference
//! (`UIElement::parent`, WinUI3's `_parent`) so `dispatch_routed` can bubble a routed event
//! from any element up to the root by simply following `parent()` — no tree search needed, and
//! critically, no dependence on the tree having been built by a single static `.elwind` traversal.
//! A back-reference requires shared (`Rc`) ownership: `Box<dyn UIElement>`'s old parent-owns-child-
//! outright model had no room for a child to point back. Concrete `new()` constructors establish
//! their collection owner before any child is added.

use crate::base::{Point, Rect, Size};
use crate::input::RoutedEventArgs;
use crate::layout::{
    GridCell, GridLength, HorizontalAlignment, Orientation, VerticalAlignment, Visibility,
    align_within, apply_size_constraints, grid_arrange, grid_natural_size, grow_by_margin,
    shrink_by_margin, shrink_rect_by_margin, stack_arrange, stack_natural_size,
};
#[cfg(test)]
use crate::painter::RenderCommand;
pub use crate::painter::TextAlignment;
use crate::painter::{RenderContext, RenderGroup, RenderTree};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_RENDER_GROUP_ID: AtomicU64 = AtomicU64::new(1);

/// The backend-agnostic handle to whatever native host (`elwindui-backend-appkit`'s `TreeHostView`,
/// `elwindui-backend-winui3`'s `TreeHostPanel`) currently owns a given tree — the thing
/// `UIElement::invalidate`/`invalidate_arrange`/`invalidate_measure` (see that trait) ultimately
/// call to ask for a fresh `layout_root`/RenderTree reconciliation pass. Declared here (not a raw
/// `Rc<dyn Fn()>`) so backends
/// provide an `impl RelayoutHost for XHost` the same way they already provide `impl
/// elwindui_core::ui::Button for ButtonImpl`/etc. — this crate's own established "shared trait in
/// core, impl per backend" convention (see this module's own doc comment on `TextArea`/`Button`/...
/// just below `NativeControl`). Each backend's own `impl` should wrap a *weak* handle back to its
/// host (see e.g. `elwindui-backend-appkit`'s `AppKitRelayoutHost`) — a strong one would create a
/// reference cycle, since the host itself holds the tree that (via `UIElement::invalidate_host`
/// on that tree's root) holds this `Rc<dyn RelayoutHost>` right back.
pub trait RelayoutHost {
    fn request_relayout(&self, dirty_group_id: u64);
}

/// The fields every `UIElement` carries (WinUI3's `FrameworkElement` base class, via composition
/// since Rust has no class inheritance — each concrete type embeds one of these and delegates
/// `UIElement::base`).
///
/// Every field here is interior-mutable (`Cell`/`RefCell`, matching `routed_handlers`/`parent`,
/// which already were) — every `create_xxx(...)` factory in this crate (and every hand-written
/// backend's `create_button`/etc.) builds its own `UIElement::default()` internally, taking no
/// `base` parameter at all; `elwindui-codegen`'s generated code instead calls `set_margin`/
/// `set_horizontal_alignment`/`set_vertical_alignment`/`set_grid_cell` (and
/// `register_routed_handler`, already `Rc<RefCell<..>>`-based) through `&self` right after
/// construction, for whichever of these this specific use site actually specified. This is what
/// lets a native leaf (`Button`/`TextArea`/`TabView`, whose own `Type::new(..)` signature is fixed
/// by `elwindui-codegen`'s `Type::new(args)` calling convention) still have its use-site margin/
/// alignment applied, without threading them through every factory's constructor
/// argument list.
///
/// The common interface every element in the Visual tree implements — a backend's own
/// `NativeControlImpl`, `TextBlock`, `Shape`, `VerticalLayout`/`HorizontalLayout`, and
/// `Control` are all peers here, not variants of some enum.
/// New kinds (a future `Grid`, say) are added by implementing this trait; nothing here or in
/// `layout_root` needs to change.
///
/// `UIElement` is the root of the class hierarchy (docs/elwindui_spec.md 付録H.2.1a) —
/// `#[elwindui_macros::class]`'s "root class mode" (no `inherits`): every method on the paired
/// `impl UIElement { .. }` below becomes a *default* method here, embedded body and all, so every
/// other `#[class(inherits = ..)]`-managed subclass inherits all of them for free via Rust's own
/// default-method dispatch — only `base` (synthesized by the macro; its concrete location differs
/// per implementor) is a genuinely required method.
#[elwindui_macros::class]
pub struct UIElement {
    /// Stable identity of this Visual's retained RenderGroup. Never reused within a process.
    pub render_group_id: u64,
    pub margin: Cell<f32>,
    pub horizontal_alignment: Cell<HorizontalAlignment>,
    pub vertical_alignment: Cell<VerticalAlignment>,
    /// WinUI3's `UIElement.Visibility` — `Visible` (default) or `Collapsed`. See `Visibility`'s own
    /// doc comment for how `Collapsed` is handled by the layout/render/hit-test traversals.
    pub visibility: Cell<Visibility>,
    /// WPF-compatible inherited `ClipToBounds` local value. `None` inherits from the Visual parent;
    /// the root's effective value is false.
    pub clip_to_bounds: Cell<Option<bool>>,
    /// WinUI3's `FrameworkElement.Width`/`Height`/`MinWidth`/`MinHeight`/`MaxWidth`/`MaxHeight` —
    /// `None` is WinUI3's `NaN` sentinel ("unset", i.e. auto-sized). Applied generically by
    /// `UIElement::measure`/`arrange` (`crate::layout::apply_size_constraints`), the same way
    /// margin/alignment already are.
    pub width: Cell<Option<f32>>,
    pub height: Cell<Option<f32>>,
    pub min_width: Cell<Option<f32>>,
    pub min_height: Cell<Option<f32>>,
    pub max_width: Cell<Option<f32>>,
    pub max_height: Cell<Option<f32>>,
    /// WinUI3's `UIElement.DesiredSize` — the result of the most recent `UIElement::measure` pass,
    /// `None` before the first one (or right after `invalidate_measure` — see that method's own doc
    /// comment) rather than some zero-value placeholder, so a reader can distinguish "not measured
    /// yet" from "measured to be zero-sized". Written only by `measure` itself — externally
    /// read-only (the `measured_size()` getter has no paired public setter).
    pub measured_size: Cell<Option<Size>>,
    /// WinUI3's `UIElement.ActualWidth`/`ActualHeight`/`ActualOffset` — the *result* of this
    /// element's own most recent `arrange` pass, not an input to it. All three are set by the
    /// element itself, from within its own `arrange` call (`arranged_offset` is *not* set by the
    /// parent — see `UIElement::arrange`'s own doc comment), and are `None` before the first
    /// `arrange` pass (or right after `invalidate_arrange`/`invalidate_measure`) rather than some
    /// zero-value placeholder.
    pub arranged_width: Cell<Option<f32>>,
    pub arranged_height: Cell<Option<f32>>,
    pub arranged_offset: Cell<Option<Point>>,
    /// `#[routed]`-tagged callback fields (`on_click`, and any future one — see
    /// `docs/elwindui_spec.md` 4章), keyed by field name. Each value is a
    /// `Box<dyn Fn(&T, &RoutedEventArgs)>` erased to `Box<dyn Any>` (`T` is that field's own
    /// payload type — `()` for `on_click`, `usize` for a hypothetical routed `on_select`, ...);
    /// generated call sites know `T` statically from the `.elwind` declaration, so the downcast in
    /// `dispatch_routed` always succeeds (matching the type-erasure pattern used by
    /// `elwindui-builtins::appkit::tab_view`'s `items_source`).
    pub routed_handlers: RoutedHandlers,
    /// Generic, type-erased attached-property bag (docs/elwindui_spec.md §3の添付プロパティ), keyed
    /// by `(owner, field)` — e.g. `("Grid", "row")` — and populated right after construction from
    /// whatever `Owner::field: value` setters the `.elwind` source wrote on this specific element
    /// (`elwindui-codegen`'s `plan_element`/`emit_construction`/`emit_attached_setters`). Absent for
    /// any element that didn't set a given `(owner, field)` — the owner's own reader (e.g.
    /// `Grid`'s `grid_cell_of`) supplies the default in that case, since only the owner knows
    /// its own attached fields' declared defaults. Harmless, unconsulted data on any element that
    /// isn't actually a child of the matching owner, exactly like WPF's own attached properties. A
    /// future attached-property owner needs no changes here at all — it just calls
    /// `set_attached`/`get_attached` with its own `(owner, field)` keys.
    pub attached: RefCell<HashMap<(&'static str, &'static str), Box<dyn Any>>>,
    /// The Logical-tree parent. `Weak` (not `Rc`) since its owner already owns its children;
    /// a strong back-reference would create a cycle nothing could ever drop. `None` for a root
    /// of whatever logical tree this element is currently part of (there's no
    /// `Weak<dyn UIElement>::new()` — an unsizing coercion needs a concrete `Sized` source — so
    /// this is `Option`-wrapped rather than a permanently-empty `Weak`).
    pub parent: RefCell<Option<Weak<dyn UIElementExt>>>,
    /// The parent in the rendered Visual tree.  This is deliberately independent from
    /// `parent`, which is the Logical-tree relationship maintained by
    /// `UIElementCollection`.
    pub visual_parent: RefCell<Option<Weak<dyn UIElementExt>>>,
    /// The Visual tree's actual child storage. Every
    /// `UIElement`'s `visual_children()` reads this generically (`UIElement`'s own default trait
    /// method), so no concrete type implements that method itself anymore. Empty (and never
    /// populated) for a leaf like `NativeControlImpl`/`Shape`/`TextBlock`. A container
    /// (`Layout`/`Control`/`Grid`) shares this same storage with its own
    /// `UIElementCollection` mutations update this collection, but direct Visual mutations do
    /// not alter the Logical tree.
    pub visual_collection: UIElementVisualCollection,
    /// Set only on whichever element a backend host currently owns as the root of a hosted tree
    /// (`elwindui-backend-appkit`'s `TreeHostView::set_tree`/`elwindui-backend-winui3`'s
    /// `TreeHostPanel::set_tree`) — `None` on every other element, including every one of that
    /// root's own descendants. `UIElement::invalidate`/`invalidate_arrange`/`invalidate_measure`
    /// (see that trait) reach this by walking `parent()` up to the root, not by reading this field
    /// on `self` directly. See `RelayoutHost`'s own doc comment for why this is a trait object
    /// rather than a raw closure.
    pub invalidate_host: RefCell<Option<Rc<dyn RelayoutHost>>>,
}

impl std::fmt::Debug for UIElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UIElement")
            .field("render_group_id", &self.render_group_id)
            .field("margin", &self.margin.get())
            .field("horizontal_alignment", &self.horizontal_alignment.get())
            .field("vertical_alignment", &self.vertical_alignment.get())
            .field("visibility", &self.visibility.get())
            .field("clip_to_bounds", &self.clip_to_bounds.get())
            .field("width", &self.width.get())
            .field("height", &self.height.get())
            .field("min_width", &self.min_width.get())
            .field("min_height", &self.min_height.get())
            .field("max_width", &self.max_width.get())
            .field("max_height", &self.max_height.get())
            .field("measured_size", &self.measured_size.get())
            .field("arranged_width", &self.arranged_width.get())
            .field("arranged_height", &self.arranged_height.get())
            .field("arranged_offset", &self.arranged_offset.get())
            .field(
                "routed_handlers",
                &self.routed_handlers.borrow().keys().collect::<Vec<_>>(),
            )
            .field(
                "attached_keys",
                &self.attached.borrow().keys().cloned().collect::<Vec<_>>(),
            )
            .field(
                "has_parent",
                &self
                    .parent
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.upgrade().is_some()),
            )
            .field(
                "has_visual_parent",
                &self
                    .visual_parent
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.upgrade().is_some()),
            )
            .field("visual_children_len", &self.visual_collection.len())
            .field("invalidate_host", &self.invalidate_host.borrow().is_some())
            .finish()
    }
}

impl Default for UIElement {
    fn default() -> Self {
        let owner = Rc::new(RefCell::new(None));
        UIElement {
            render_group_id: NEXT_RENDER_GROUP_ID.fetch_add(1, Ordering::Relaxed),
            margin: Cell::new(0.0),
            horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            vertical_alignment: Cell::new(VerticalAlignment::Stretch),
            visibility: Cell::new(Visibility::Visible),
            clip_to_bounds: Cell::new(None),
            width: Cell::new(None),
            height: Cell::new(None),
            min_width: Cell::new(None),
            min_height: Cell::new(None),
            max_width: Cell::new(None),
            max_height: Cell::new(None),
            measured_size: Cell::new(None),
            arranged_width: Cell::new(None),
            arranged_height: Cell::new(None),
            arranged_offset: Cell::new(None),
            routed_handlers: Rc::new(RefCell::new(HashMap::new())),
            attached: RefCell::new(HashMap::new()),
            parent: RefCell::new(None),
            visual_parent: RefCell::new(None),
            visual_collection: UIElementVisualCollection::new(owner),
            invalidate_host: RefCell::new(None),
        }
    }
}

/// The type every widget wrapper wanting `#[routed]` support (not just `UIElement`, which
/// every `UIElement` already carries one of — a hand-written builtin like
/// `elwindui-builtins::appkit::Button` needs its *own* copy too, registered into at its own
/// construction time and later shared into the `NativeControlImpl` wrapping it, since that wrapper
/// doesn't exist yet when the widget itself is constructed and wired — see
/// `elwindui-codegen`'s `into_node_if_needed`) stores its handlers as.
pub type RoutedHandlers = Rc<RefCell<HashMap<&'static str, Vec<Box<dyn Any>>>>>;

/// Shared registration logic for anything holding a [`RoutedHandlers`] — `UIElement`'s own
/// `register_routed_handler` method delegates here, and any widget wrapper exposing its own
/// `register_routed_handler` (see this module's own doc comment) should too, rather than
/// reimplementing the erasure.
pub fn register_routed_handler<T: 'static>(
    handlers: &RoutedHandlers,
    name: &'static str,
    handler: Box<dyn Fn(&T, &RoutedEventArgs)>,
) {
    handlers
        .borrow_mut()
        .entry(name)
        .or_default()
        .push(Box::new(handler));
}

#[elwindui_macros::class]
impl UIElement {
    fn construct() -> Self {
        Self::default()
    }

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
    fn render_group_id(&self) -> u64 {
        self.as_ui_element().render_group_id
    }
    /// WPF's inherited `ClipToBounds`; the root defaults to false.
    fn clip_to_bounds(&self) -> bool {
        if let Some(value) = self.as_ui_element().clip_to_bounds.get() {
            value
        } else {
            self.visual_parent()
                .is_some_and(|parent| parent.clip_to_bounds())
        }
    }
    /// WinUI3's `FrameworkElement.Width`/`Height`/`MinWidth`/`MinHeight`/`MaxWidth`/`MaxHeight` —
    /// see `UIElement`'s own doc comment for these six fields.
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
    /// WinUI3's `UIElement.DesiredSize` — the result of the most recent `measure` pass, or `None`
    /// if it hasn't run since construction or the last `invalidate_measure`. See
    /// `UIElement::measured_size`'s own doc comment.
    fn measured_size(&self) -> Option<Size> {
        self.as_ui_element().measured_size.get()
    }
    /// WinUI3's `UIElement.ActualWidth`/`ActualHeight`/`ActualOffset` — the result of the most
    /// recent `arrange` pass, or `None` if it hasn't run since construction or the last
    /// `invalidate_arrange`/`invalidate_measure`. See `UIElement`'s own doc comment.
    fn arranged_width(&self) -> Option<f32> {
        self.as_ui_element().arranged_width.get()
    }
    fn arranged_height(&self) -> Option<f32> {
        self.as_ui_element().arranged_height.get()
    }
    fn arranged_offset(&self) -> Option<Point> {
        self.as_ui_element().arranged_offset.get()
    }
    /// Post-construction setters (docs/elwindui_spec.md 付録H.2.1a) for every field this trait
    /// already exposes a getter for — declared here (not just as `UIElement`'s own inherent
    /// methods) so they're reachable generically through `dyn UIElement`/any bound on this trait,
    /// not only through the concrete backing struct.
    fn set_margin(&self, margin: f32) {
        self.as_ui_element().margin.set(margin);
        self.invalidate_measure();
    }
    fn set_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.as_ui_element().horizontal_alignment.set(alignment);
        self.invalidate_arrange();
    }
    fn set_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.as_ui_element().vertical_alignment.set(alignment);
        self.invalidate_arrange();
    }
    fn set_visibility(&self, visibility: Visibility) {
        self.as_ui_element().visibility.set(visibility);
        self.invalidate_measure();
    }
    fn set_clip_to_bounds(&self, value: Option<bool>) {
        self.as_ui_element().clip_to_bounds.set(value);
        self.invalidate_arrange();
    }
    fn set_width(&self, width: Option<f32>) {
        self.as_ui_element().width.set(width);
        self.invalidate_measure();
    }
    fn set_height(&self, height: Option<f32>) {
        self.as_ui_element().height.set(height);
        self.invalidate_measure();
    }
    fn set_min_width(&self, min_width: Option<f32>) {
        self.as_ui_element().min_width.set(min_width);
        self.invalidate_measure();
    }
    fn set_min_height(&self, min_height: Option<f32>) {
        self.as_ui_element().min_height.set(min_height);
        self.invalidate_measure();
    }
    fn set_max_width(&self, max_width: Option<f32>) {
        self.as_ui_element().max_width.set(max_width);
        self.invalidate_measure();
    }
    fn set_max_height(&self, max_height: Option<f32>) {
        self.as_ui_element().max_height.set(max_height);
        self.invalidate_measure();
    }
    /// The parent in the Logical tree. `UIElementCollection` owns this relationship.
    fn parent(&self) -> Option<Rc<dyn UIElementExt>> {
        self.as_ui_element()
            .parent
            .borrow()
            .as_ref()
            .and_then(|p| p.upgrade())
    }
    /// WinUI3's `VisualTreeHelper.GetParent` — the parent in the rendered Visual tree.
    fn visual_parent(&self) -> Option<Rc<dyn UIElementExt>> {
        self.as_ui_element()
            .visual_parent
            .borrow()
            .as_ref()
            .and_then(|p| p.upgrade())
    }
    /// This element's own children in the **Visual tree** (WinUI3's own Visual-tree children,
    /// docs/elwindui_spec.md 付録H.2.2) — the only tree any code ever actually walks (there is no
    /// separate, generically-traversable Logical tree data structure; some components merely *have*
    /// Logical-tree-shaped children of their own — see `UIElementCollection`). A default method,
    /// not overridden by any concrete type: it reads `self.as_ui_element().visual_children` directly, which
    /// is empty for a leaf like `NativeControlImpl`/`TextBlock`/`Shape` and populated for a
    /// container (`Layout`/`Control`/`Grid`) via that same `UIElement`'s
    /// `UIElementCollection` updates. Returns an owned `Vec` (each
    /// `Rc<dyn UIElement>` cheaply cloned, a refcount bump), not `&[..]`: the underlying storage is
    /// `RefCell`-backed (mutable at any time via `UIElementCollection`'s `add`/`remove`/etc.), and a
    /// `std::cell::Ref` guard can't be smuggled out through a bare reference tied to `&self`.
    #[overridable]
    fn visual_children(&self) -> Vec<Rc<dyn UIElementExt>> {
        self.as_ui_element().visual_collection.to_vec()
    }
    /// WinUI3's `GetType().Name` (via `.NET` reflection), commonly paired with `VisualTreeHelper`
    /// when dumping/debugging a tree — see `crate::visual_tree`. A default method, not overridden by
    /// any concrete type: `std::any::type_name::<Self>()` is monomorphized per implementor, so this
    /// resolves to the real concrete type (`ButtonImpl`/`TextBlock`/...) even when called through
    /// `dyn UIElement`.
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    /// This element's own desired size, given `available` (margin already excluded by the caller,
    /// WinUI3's `MeasureOverride`) — measures/positions any children itself (calling
    /// `child.measure(..)`/reading `child.measured_size()`), rather than being handed a
    /// pre-computed array. Defaults to taking no space at all — every concrete leaf/container
    /// overrides this with real logic; nothing currently relies on this default actually being
    /// invoked.
    #[overridable]
    fn measure_override(&self, _available: Size) -> Size {
        Size {
            width: 0.0,
            height: 0.0,
        }
    }
    /// Arranges this element's own children (in this element's own local coordinate space), given
    /// the final size this element itself was assigned (WinUI3's `ArrangeOverride`) — calls
    /// `child.arrange(..)` itself for each child it has, rather than returning a rect list for a
    /// caller to apply. Returns the size actually used (WinUI3 allows this to differ slightly from
    /// `final_size`; the default and every override here just echo it back unchanged). Defaults to
    /// doing nothing (no children) — see `measure_override`'s own doc comment.
    #[overridable]
    fn arrange_override(&self, final_size: Size) -> Size {
        final_size
    }
    /// Records this element's own local drawing commands. Pure layout containers use the default
    /// no-op implementation; children are rendered by the visual-tree walker.
    #[overridable]
    fn render(&self, _context: &mut RenderContext<'_>) {}
    /// `Some(&self.handle)` (the raw native handle itself, erased to `&dyn Any`) for a backend's own
    /// `NativeControlImpl { handle: AnyView, .. }` and for any type that composes one as its own
    /// `base` field (docs/elwindui_spec.md 付録H.2.1a — e.g. a backend's `ButtonImpl { base:
    /// NativeControlImpl, .. }` overrides this to return `Some(&self.base.handle)`); `None` for every
    /// other `UIElement` (the default). `collect_render_items<H>` downcasts this directly to `H`
    /// (`downcast_ref::<H>()`), not to any `elwindui-core`-defined wrapper struct — measuring/placing
    /// a native handle is entirely backend-specific, so `elwindui_core::ui::NativeControl` (the
    /// marker trait every real native leaf implements) doesn't define one; see that trait's own doc
    /// comment.
    #[overridable]
    fn try_as_native_control(&self) -> Option<&dyn Any> {
        None
    }
    /// WPF's `UIElement.InvalidateVisual`: invalidates arrange state and asks the host for an
    /// asynchronous layout/render pass. The pass records this Visual's RenderGroup again.
    fn invalidate(&self) {
        self.invalidate_arrange();
    }
    /// WinUI3's `UIElement.InvalidateArrange` — marks this element's `arranged_width`/
    /// `arranged_height`/`arranged_offset` `None` (to be recomputed by the next `arrange` pass) and
    /// asks for a redraw. `measured_size` stays valid — only where this element ends up, not how
    /// big it wants to be, is in question (e.g. `UIElement::set_horizontal_alignment`).
    fn invalidate_arrange(&self) {
        self.as_ui_element().arranged_width.set(None);
        self.as_ui_element().arranged_height.set(None);
        self.as_ui_element().arranged_offset.set(None);
        request_relayout(self.as_ui_element());
    }
    /// WinUI3's `UIElement.InvalidateMeasure` — marks this element's `measured_size` *and*
    /// `arranged_width`/`arranged_height`/`arranged_offset` all `None` (a changed desired size
    /// can't leave a stale arrangement behind) and asks for a redraw. The strongest of the three —
    /// use whenever a change could affect `measure_override`'s result (e.g. `UIElement::set_margin`,
    /// `set_width`).
    fn invalidate_measure(&self) {
        self.as_ui_element().measured_size.set(None);
        self.as_ui_element().arranged_width.set(None);
        self.as_ui_element().arranged_height.set(None);
        self.as_ui_element().arranged_offset.set(None);
        request_relayout(self.as_ui_element());
    }
    /// Registers a handler for a `#[routed]`-tagged field named `name` on this element — see this
    /// struct's own `routed_handlers` doc comment for the erasure convention.
    fn register_routed_handler<T: 'static>(
        &self,
        name: &'static str,
        handler: Box<dyn Fn(&T, &RoutedEventArgs)>,
    ) where
        Self: Sized,
    {
        register_routed_handler(&self.as_ui_element().routed_handlers, name, handler);
    }
    /// Stores an attached-property value under `(owner, field)` — e.g. `("Grid", "row")` — type-
    /// erased into the shared `attached` bag (see that field's own doc comment). `owner`/`field` are
    /// always compile-time-known string literals from `elwindui-codegen`'s `emit_attached_setters`,
    /// which also picks `T` via an explicit turbofish matching the `#[attached]` field's declared
    /// type — never inferred from `value` alone, since a mismatched inferred type here would make
    /// `get_attached`'s `downcast_ref` silently miss and fall back to its caller's default.
    fn set_attached<T: 'static>(&self, owner: &'static str, field: &'static str, value: T)
    where
        Self: Sized,
    {
        self.as_ui_element()
            .attached
            .borrow_mut()
            .insert((owner, field), Box::new(value));
        self.invalidate_measure();
    }
    /// Reads an attached-property value previously stored under `(owner, field)`, or `default` if
    /// absent (never set on this element, or set with a different `T` — the same `downcast_ref`
    /// miss as an absent key). Callers are the *owner* component's own layout code (e.g. `Grid`'s
    /// `grid_cell_of`), which knows its own attached field's concrete type — see `set_attached`'s
    /// own doc comment for why the type must agree between writer and reader.
    fn get_attached<T: Clone + 'static>(
        &self,
        owner: &'static str,
        field: &'static str,
        default: T,
    ) -> T
    where
        Self: Sized,
    {
        self.as_ui_element()
            .attached
            .borrow()
            .get(&(owner, field))
            .and_then(|v| v.downcast_ref::<T>())
            .cloned()
            .unwrap_or(default)
    }
    /// Called by whatever backend host (`TreeHostView::set_tree`/`TreeHostPanel::set_tree`) is
    /// about to own this element as the root of a hosted tree — see `invalidate_host`'s own doc
    /// comment. `None` un-registers (e.g. a host discarding a tree it no longer owns).
    fn set_invalidate_host(&self, host: Option<Rc<dyn RelayoutHost>>) {
        *self.as_ui_element().invalidate_host.borrow_mut() = host;
    }
    /// WinUI3's `UIElement.Measure(Size availableSize)` — computes this element's own desired size
    /// (margin-inclusive) against `available`, recursing into children as `measure_override` (still
    /// freely overridable, unlike this method) needs them, and caches the result in
    /// `measured_size()`. `void` like WinUI3's own `Measure` — callers read the result back via
    /// `measured_size()` rather than this method's return value (there isn't one). Always
    /// recomputes when called, regardless of whether `measured_size()` was already `Some` — see
    /// `UIElement::measured_size`'s own doc comment for why this isn't a memoizing cache.
    fn measure(&self, available: Size) {
        let result = if self.visibility() == Visibility::Collapsed {
            Size {
                width: 0.0,
                height: 0.0,
            }
        } else {
            let inner_available = constrain(self, shrink_by_margin(available, self.margin()));
            let desired = constrain(self, self.measure_override(inner_available));
            grow_by_margin(desired, self.margin())
        };
        self.as_ui_element().measured_size.set(Some(result));
    }
    /// WinUI3's `UIElement.Arrange(Rect finalRect)` — `finalRect` is relative to this element's own
    /// parent (not absolute screen/window coordinates — see `elwindui_core::ui::layout_root`'s
    /// `collect_render_items` for where absolute positions actually get computed, by walking down
    /// accumulating each element's own `arranged_offset`). Applies this element's own margin and
    /// alignment against `finalRect` to compute its final position+size, caches those into
    /// `arranged_width`/`arranged_height`/`arranged_offset` (this element sets its *own*
    /// `arranged_offset` here — it is not set by the parent), then delegates arranging any children
    /// entirely to `arrange_override` (still freely overridable), which calls `child.arrange(..)`
    /// itself for each one it has.
    fn arrange(&self, final_rect: Rect) {
        if self.visibility() == Visibility::Collapsed {
            self.as_ui_element().arranged_width.set(Some(0.0));
            self.as_ui_element().arranged_height.set(Some(0.0));
            return;
        }
        // WinUI3: `Arrange` implicitly re-`Measure`s if `Measure` hasn't run since the last
        // invalidation — `measured_size()` being `None` here means exactly that.
        if self.measured_size().is_none() {
            self.measure(Size {
                width: final_rect.width,
                height: final_rect.height,
            });
        }
        let desired_with_margin = self.measured_size().unwrap_or_default();
        let slot = shrink_rect_by_margin(final_rect, self.margin());
        let desired_without_margin = shrink_by_margin(desired_with_margin, self.margin());
        let own_rect = align_within(
            slot,
            desired_without_margin,
            self.horizontal_alignment(),
            self.vertical_alignment(),
        );
        let own_size = Size {
            width: own_rect.width,
            height: own_rect.height,
        };
        self.as_ui_element()
            .arranged_width
            .set(Some(own_size.width));
        self.as_ui_element()
            .arranged_height
            .set(Some(own_size.height));
        self.as_ui_element().arranged_offset.set(Some(Point {
            x: own_rect.x,
            y: own_rect.y,
        }));
        self.arrange_override(own_size);
    }
}

/// Shared implementation for `UIElement::invalidate`/`invalidate_arrange`/`invalidate_measure` —
/// walks from `base`'s own element up to the root of whatever tree it's currently part of
/// (`UIElement::parent`, repeated until `None`) and, if that root has a `RelayoutHost` registered
/// (see `UIElement::invalidate_host`), asks it for a fresh layout pass. Takes `&UIElement`
/// (not `&dyn UIElement`) so the caller — a default trait method, where `Self` isn't known to be
/// `Sized`. A no-op if the Visual root has no registered host (e.g. a standalone test tree).
fn request_relayout(base: &UIElement) {
    let mut current = base
        .visual_parent
        .borrow()
        .as_ref()
        .and_then(|w| w.upgrade());
    let mut host = base.invalidate_host.borrow().clone();
    while let Some(element) = current {
        host = element
            .as_ui_element()
            .invalidate_host
            .borrow()
            .clone()
            .or(host);
        current = element.visual_parent();
    }
    if let Some(host) = host {
        host.request_relayout(base.render_group_id);
    }
}

/// Binds an already-constructed node to the owner slots used by its collections.  This is called
/// by each concrete `new()` immediately after it creates its `Rc<Self>`; children are then added
/// through the collection APIs, which perform all parent wiring.
pub fn bind_element_owner<T: UIElementExt + 'static>(this: &Rc<T>) {
    let erased: Rc<dyn UIElementExt> = this.clone();
    erased.as_ui_element().visual_collection.bind_owner(&erased);
}

/// The Visual tree's actual child storage (the low-level
/// counterpart to `Panel.Children`'s `UIElementCollection` below) — a plain, runtime-mutable
/// `add`/`insert`/`remove`/`remove_at`/`clear` collection. `UIElement::visual_children` holds
/// one of these directly; `UIElement::visual_children()` (the default trait method) just reads it.
/// Every mutation owns Visual-parent wiring and invalidates its owner.
#[derive(Clone)]
pub struct UIElementVisualCollection {
    storage: Rc<RefCell<Vec<Rc<dyn UIElementExt>>>>,
    owner: Rc<RefCell<Option<Weak<dyn UIElementExt>>>>,
}

impl UIElementVisualCollection {
    pub fn new(owner: Rc<RefCell<Option<Weak<dyn UIElementExt>>>>) -> Self {
        Self {
            storage: Rc::new(RefCell::new(Vec::new())),
            owner,
        }
    }
    fn bind_owner(&self, owner: &Rc<dyn UIElementExt>) {
        *self.owner.borrow_mut() = Some(Rc::downgrade(owner));
    }

    fn bind_weak_owner(&self, owner: Weak<dyn UIElementExt>) {
        *self.owner.borrow_mut() = Some(owner);
    }
    fn owner_rc(&self) -> Option<Rc<dyn UIElementExt>> {
        self.owner
            .borrow()
            .as_ref()
            .and_then(|owner| owner.upgrade())
    }
    pub fn owner_handle(&self) -> Rc<RefCell<Option<Weak<dyn UIElementExt>>>> {
        self.owner.clone()
    }
    pub fn add(&self, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().visual_parent.borrow_mut() = Some(Rc::downgrade(&owner));
            owner.invalidate_measure();
        }
        self.storage.borrow_mut().push(child);
    }
    pub fn insert(&self, index: usize, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().visual_parent.borrow_mut() = Some(Rc::downgrade(&owner));
            owner.invalidate_measure();
        }
        self.storage.borrow_mut().insert(index, child);
    }
    /// Removes the first entry pointer-equal to `child`, if any — returns whether one was found.
    pub fn remove(&self, child: &Rc<dyn UIElementExt>) -> bool {
        let mut storage = self.storage.borrow_mut();
        match storage.iter().position(|c| Rc::ptr_eq(c, child)) {
            Some(index) => {
                let removed = storage.remove(index);
                *removed.as_ui_element().visual_parent.borrow_mut() = None;
                if let Some(owner) = self.owner_rc() {
                    owner.invalidate_measure();
                }
                true
            }
            None => false,
        }
    }
    pub fn remove_at(&self, index: usize) -> Rc<dyn UIElementExt> {
        let child = self.storage.borrow_mut().remove(index);
        *child.as_ui_element().visual_parent.borrow_mut() = None;
        if let Some(owner) = self.owner_rc() {
            owner.invalidate_measure();
        }
        child
    }
    pub fn clear(&self) {
        let children = std::mem::take(&mut *self.storage.borrow_mut());
        for child in children {
            *child.as_ui_element().visual_parent.borrow_mut() = None;
        }
        if let Some(owner) = self.owner_rc() {
            owner.invalidate_measure();
        }
    }
    pub fn len(&self) -> usize {
        self.storage.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.storage.borrow().is_empty()
    }
    pub fn to_vec(&self) -> Vec<Rc<dyn UIElementExt>> {
        self.storage.borrow().clone()
    }
}

/// The Logical-tree-shaped child list a container (`Layout`/`Control` family) declares in
/// `.elwind` — WinUI3's own `UIElementCollection` (docs/elwindui_spec.md 付録H.2.2), e.g.
/// `Panel.Children`. There is no separate, generically-traversable Logical tree: this is simply the
/// convenience API a *particular* component exposes for its own children, which automatically stays
/// in sync with the real Visual tree — `add`/`insert`/`remove`/`remove_at`/`clear` all mutate the
/// its own storage and additionally keeps each affected child's Logical `parent` pointer correct.
/// Deliberately has no way to replace its storage wholesale (no `set_children`) — every mutation
/// goes through one of these add/remove operations, so the Visual tree can never silently drift out
/// of sync with whatever a container thinks its own children are.
#[derive(Clone)]
pub struct UIElementCollection {
    storage: Rc<RefCell<Vec<Rc<dyn UIElementExt>>>>,
    owner: Rc<RefCell<Option<Weak<dyn UIElementExt>>>>,
}

impl UIElementCollection {
    pub fn new(owner: Rc<RefCell<Option<Weak<dyn UIElementExt>>>>) -> Self {
        Self {
            storage: Rc::new(RefCell::new(Vec::new())),
            owner,
        }
    }
    fn owner_rc(&self) -> Option<Rc<dyn UIElementExt>> {
        self.owner.borrow().as_ref().and_then(|w| w.upgrade())
    }
    pub fn add(&self, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        if let Some(owner) = self.owner_rc() {
            owner.as_ui_element().visual_collection.add(child.clone());
        }
        self.storage.borrow_mut().push(child);
    }
    pub fn insert(&self, index: usize, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        if let Some(owner) = self.owner_rc() {
            owner
                .as_ui_element()
                .visual_collection
                .insert(index, child.clone());
        }
        self.storage.borrow_mut().insert(index, child);
    }
    pub fn remove(&self, child: &Rc<dyn UIElementExt>) -> bool {
        let mut storage = self.storage.borrow_mut();
        let removed = storage
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, child))
            .map(|index| storage.remove(index));
        if let Some(removed) = removed {
            *child.as_ui_element().parent.borrow_mut() = None;
            if let Some(owner) = self.owner_rc() {
                owner.as_ui_element().visual_collection.remove(&removed);
            }
            true
        } else {
            false
        }
    }
    pub fn remove_at(&self, index: usize) -> Rc<dyn UIElementExt> {
        let child = self.storage.borrow_mut().remove(index);
        *child.as_ui_element().parent.borrow_mut() = None;
        if let Some(owner) = self.owner_rc() {
            owner.as_ui_element().visual_collection.remove(&child);
        }
        child
    }
    pub fn clear(&self) {
        for child in self.to_vec() {
            *child.as_ui_element().parent.borrow_mut() = None;
            if let Some(owner) = self.owner_rc() {
                owner.as_ui_element().visual_collection.remove(&child);
            }
        }
        self.storage.borrow_mut().clear();
    }
    pub fn len(&self) -> usize {
        self.storage.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.storage.borrow().is_empty()
    }
    pub fn to_vec(&self) -> Vec<Rc<dyn UIElementExt>> {
        self.storage.borrow().clone()
    }
}

/// `Button`/`TextArea`/`TabView` — the only `UIElement`s with a real backend handle. Always a leaf as
/// far as this tree is concerned: whatever lives beneath it in its own backend-managed hierarchy
/// (e.g. `TabView`'s tab-switching) is opaque here. A pure marker trait (`trait_only` — no
/// `NativeControlImpl`/`<H>` here at all): measuring/placing a native handle is entirely
/// backend-specific (e.g. AppKit's `NSView.fittingSize()`/`setFrame:`), so instead of `elwindui-core`
/// owning a shared generic `NativeControlImpl<H>` that every backend's `H` would need to plug into,
/// each backend defines its own concrete, non-generic implementor (e.g.
/// `elwindui-backend-appkit::NativeControlImpl { handle: AnyView, .. }`, and its winui3 equivalent)
/// that `TextArea`/`Button`/`TabView` (that backend's own leaf widgets) inherit from — the same way
/// `VerticalLayout`/`Control`/`Grid` above each write their own `measure_override`, not through any
/// shared "MeasureNode" abstraction. `collect_render_items<H>` downcasts a leaf's
/// `try_as_native_control()` result directly to `H` (see that trait method's own doc comment) — no
/// wrapper struct type needs to be nameable from `elwindui-core` for this to work.
#[elwindui_macros::class(trait_only, inherits = crate::ui::UIElement)]
pub trait NativeControl {}

/// The property-setter traits below (`TextArea`/`Button`/`MenuItem`/`Menu`/`MenuBar`/`MenuBarItem`/
/// `Window`) are declared once here rather than duplicated per backend crate — every backend's own
/// hand-written `XImpl` (`elwindui-backend-appkit`/`elwindui-backend-winui3`) had been independently
/// declaring an identically-shaped trait (same method signatures, same doc-comment rationale)
/// purely because Rust has no cross-crate trait sharing without a common home for it; this is that
/// home. Each backend crate now just provides `impl Xxx for BackendXImpl { .. }` — the property
/// *shape* (what setters exist, what they take) is common to every backend, only the method
/// *bodies* (the actual platform API calls) differ, exactly the same split
/// `NativeControl`/`Layout`/`Shape`/`Control`/etc. above already model for the virtual builtins.
///
/// `Menu`/`MenuBar`/`MenuBarItem`/`Window` are *not* generic over the backend's own concrete
/// menu-entry/menu-bar-entry/menu/menu-bar type the way a backend's own `NativeControlImpl`'s
/// `handle` is — instead each such argument is `&dyn` (or `Rc<dyn>`) the matching leaf trait itself
/// (`MenuItem`/`Menu`/
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
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl)]
pub trait TextArea {
    fn set_text(&self, text: &str);
    fn set_on_change(&self, callback: Box<dyn Fn(String)>);
}

#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl)]
pub trait Button {
    fn set_enabled(&self, enabled: bool);
    fn set_on_click(&self, callback: Box<dyn Fn()>);
    fn set_text(&self, text: &str);
}

#[elwindui_macros::class(trait_only)]
pub trait MenuItem {
    fn set_text(&self, text: &str);
    fn set_enabled(&self, enabled: bool);
    fn set_shortcut(&self, key_equivalent: &str);
    fn set_on_select(&self, callback: Box<dyn Fn()>);
}

/// A generic, `Vec`-like collection abstraction — `add`/`insert`/`remove`/`remove_at`/`clear`/
/// `len`/`is_empty`/`to_vec` mirror `UIElementCollection`'s own method set (see that struct's own
/// doc comment), minus the `UIElement`-tree-specific `parent`-pointer wiring `add`/`insert`/
/// `remove`/`remove_at` do there — `ListExt<T>` items aren't necessarily `UIElement`s at all (e.g.
/// `Menu::items`/`MenuBar::items` hold `Rc<dyn MenuItemExt>`/`Rc<dyn MenuBarItemExt>`, neither of
/// which is part of the `UIElement` visual tree). A plain hand-written trait, not `#[class]`-managed
/// (the macro's `trait_only`/`struct_only` shapes are for the concrete elwindui class hierarchy;
/// `ListExt<T>` is a generic utility type, one level below that, the same way `UIElementCollection`
/// itself is a plain hand-written struct rather than a `#[class]`-managed one). Each backend
/// provides its own concrete implementor per `Menu`/`MenuBar` (see `Menu::items`/`MenuBar::items`'s
/// own doc comment) — `elwindui-core` only declares the shape.
pub trait ListExt<T: ?Sized> {
    fn add(&self, item: Rc<T>);
    fn insert(&self, index: usize, item: Rc<T>);
    fn remove(&self, item: &Rc<T>) -> bool;
    fn remove_at(&self, index: usize) -> Rc<T>;
    fn clear(&self);
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn to_vec(&self) -> Vec<Rc<T>>;
}

#[elwindui_macros::class(trait_only)]
pub trait Menu {
    fn add_item(&self, item: &dyn MenuItemExt);
    fn remove_item(&self, item: &dyn MenuItemExt);
    /// A live handle onto the same backing collection `add_item`/`remove_item` mutate — added
    /// alongside them (not a replacement) so `.elwind`'s `#[content(items)]` mechanism
    /// (`builtins.elwind`'s `Menu`, `docs/elwindui_builtins_spec.md` 付録M) can populate `Menu`'s
    /// nested `MenuItem { .. }` children through the same generic `ListExt`-typed
    /// content-field path every other multi-child builtin (`VerticalLayout`/`Grid`/`TabView`/...)
    /// already uses, instead of `elwindui-codegen` needing a `Menu`-specific construction branch.
    /// A borrow (mirroring `Layout::children`/`Control::children`), not an owned `Rc` — no backend
    /// needs to hand out an independently-owned handle here.
    fn items(&self) -> &dyn ListExt<dyn MenuItemExt>;
}

#[elwindui_macros::class(trait_only)]
pub trait MenuBarItem {
    fn set_text(&self, text: &str);
    fn set_submenu(&self, submenu: Rc<dyn MenuExt>);
}

#[elwindui_macros::class(trait_only)]
pub trait MenuBar {
    fn add_item(&self, item: &dyn MenuBarItemExt);
    fn remove_item(&self, item: &dyn MenuBarItemExt);
    /// See `Menu::items`'s own doc comment — same rationale, one level up (`MenuBar`'s children are
    /// `MenuBarItem`s rather than `MenuItem`s).
    fn items(&self) -> &dyn ListExt<dyn MenuBarItemExt>;
}

/// `TabView`'s own class trait (docs/elwindui_spec.md 付録H.2.1a). Deliberately empty: every one of
/// `TabView`'s real `.elwind`-facing setters (`set_children`/`set_dynamic_source`/
/// `set_items_source`/`set_closable`/`set_selected_index`/`set_on_select`/`set_on_close`/
/// `set_on_new_tab`) is either generic (`set_dynamic_source<T>`/`set_items_source<T>`, not
/// dyn-object-safe) or takes a backend-concrete `Rc<TabViewItem>`-shaped argument that has no
/// common cross-backend signature worth sharing — see each backend's own `TabView`/`TabViewItem`
/// doc comment. Existing purely so `elwindui_core::ui::TabViewExt` is a real, resolvable path —
/// `elwindui-codegen`'s `builtin_trait_use` needs every native/virtual builtin (with no exceptions)
/// to have one, so it can emit `use elwindui::core::ui::{Name}Ext as _;` uniformly instead of
/// special-casing `TabView`/`TabViewItem` out of an 11-name list (`docs/elwindui_macro_class_spec.md`).
/// Each backend implements this (empty) trait via `struct_only = elwindui_core::ui::TabViewExt` and
/// keeps every real setter `#[inherent]`, exactly as it already did before this trait existed (this
/// only swaps which trait path the backend's own `TabViewExt` resolves to — from a backend-local
/// auto-generated one to this shared, deliberately-empty one).
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl)]
pub trait TabView {}

/// `TabViewItem`'s own class trait — see `TabView`'s own doc comment for the "why empty" rationale;
/// same reasoning applies here (`set_header`/`set_content`/`set_closable`/`set_on_close`/
/// backend-specific setters all stay `#[inherent]`). No `inherits`: like `Window`,
/// `TabViewItem` is never itself embedded as a real `Rc<dyn UIElement>` node (see its own
/// `builtins.elwind` doc comment), so it has no meaningful `NativeControl`/`UIElement` ancestor.
#[elwindui_macros::class(trait_only)]
pub trait TabViewItem {}

/// `Window`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — also the `component X inherits
/// Window` (host-composition) bare name every backend's own `WindowImpl` implements.
/// `set_menu_bar`'s `Rc<dyn MenuBar>` follows the same trait-object-argument convention as
/// `Menu`/`MenuBar`/`MenuBarItem` just above (see this module's own doc comment on that group) —
/// `impl Window for WindowImpl` downcasts it back to its own concrete `MenuBarImpl` internally.
#[elwindui_macros::class(trait_only)]
pub trait Window {
    fn set_title(&self, title: &str);
    fn set_menu_bar(&self, menu_bar: Rc<dyn MenuBarExt>);
    fn set_content(&self, content: Rc<dyn UIElementExt>);
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
/// implemented by every layout-container virtual builtin (`VerticalLayout`/
/// `HorizontalLayout`/`Grid`), the same way `NativeControl` groups every native leaf.
///
/// Holds only `children` — the one field every layout-container virtual builtin needs
/// (docs/elwindui_spec.md 1426行目). `spacing` is *not* here: it only means anything to
/// `VerticalLayout`/`HorizontalLayout` (`Grid` has no use for it), so each of those two declares
/// its own `spacing` field instead of it living on this shared base. `VerticalLayout`/
/// `HorizontalLayout` do their own layout math directly against `elwindui_core::layout`'s
/// `stack_arrange`/`stack_natural_size` free functions with their own fixed `Orientation` literal —
/// neither delegates its `measure_override`/`arrange_override` to this struct's own (trivial, "take
/// no space" — see `UIElement::measure_override`'s own doc comment) default, since the orientation
/// (and so the entire layout algorithm) is a property of *which concrete type this is*, not of
/// shared state a common base could hold.
///
/// `abstract_class`: `Layout` itself is never instantiated (no `new`, and `#[class]`'s
/// `abstract_class` never auto-generates one even though `Layout` defines `construct` below) — only
/// its concrete subclasses (`VerticalLayout`/`HorizontalLayout`) are, each calling `Layout::
/// construct()` for their own `base` field (see e.g. `Shape::construct`/`Control::construct` for the
/// same shape one level up the hierarchy, where the base *is* directly instantiable).
#[elwindui_macros::class(inherits = crate::ui::UIElement, abstract_class)]
pub struct Layout {
    /// Logical children for this layout. Its mutations update the owner's Visual collection.
    pub children: UIElementCollection,
}

#[elwindui_macros::class]
impl Layout {
    /// Not `#[inherent]` — a plain method here becomes a default `LayoutExt` trait method
    /// (dispatched through `__dyn_layout`, docs/elwindui_macro_class_spec.md), so
    /// `VerticalLayout`/`HorizontalLayout`/`Grid` all get `self.children()` for free without
    /// redeclaring it themselves, the same way every `UIElement` (root class) method is inherited
    /// by every concrete leaf/container for free.
    fn children(&self) -> &UIElementCollection {
        &self.children
    }

    fn construct() -> Self {
        let base = UIElement::default();
        let children = UIElementCollection::new(base.visual_collection.owner_handle());
        Self { base, children }
    }
}

/// `VerticalLayout`'s own class trait (docs/elwindui_spec.md 付録H.2.1a). `spacing` lives here
/// (not on `Layout`) since it's meaningless to `Grid`, `Layout`'s other concrete subclass — see
/// `Layout`'s own doc comment.
#[elwindui_macros::class(inherits = crate::ui::Layout)]
pub struct VerticalLayout {
    spacing: Cell<f32>,
}

#[elwindui_macros::class]
impl VerticalLayout {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .map(|c| {
                c.measure(available);
                c.measured_size().unwrap_or_default()
            })
            .collect();
        stack_natural_size(Orientation::Vertical, self.spacing.get(), &child_sizes)
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .map(|c| c.measured_size().unwrap_or_default())
            .collect();
        let child_rects = stack_arrange(
            final_size,
            Orientation::Vertical,
            self.spacing.get(),
            &child_sizes,
        );
        for (child, rect) in self.visual_children().iter().zip(child_rects) {
            child.arrange(rect);
        }
        final_size
    }
    fn set_spacing(&self, spacing: f32) {
        self.spacing.set(spacing);
        self.invalidate_measure();
    }
    fn construct() -> Self {
        Self {
            base: Layout::construct(),
            spacing: Cell::new(0.0),
        }
    }
    fn new() -> Rc<Self> {
        let this = Rc::new(Self::construct());
        bind_element_owner(&this);
        this
    }
}

/// `HorizontalLayout`'s own class trait (docs/elwindui_spec.md 付録H.2.1a). `spacing` lives here
/// (not on `Layout`) — see `VerticalLayout`'s own doc comment.
#[elwindui_macros::class(inherits = crate::ui::Layout)]
pub struct HorizontalLayout {
    spacing: Cell<f32>,
}

#[elwindui_macros::class]
impl HorizontalLayout {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .map(|c| {
                c.measure(available);
                c.measured_size().unwrap_or_default()
            })
            .collect();
        stack_natural_size(Orientation::Horizontal, self.spacing.get(), &child_sizes)
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .map(|c| c.measured_size().unwrap_or_default())
            .collect();
        let child_rects = stack_arrange(
            final_size,
            Orientation::Horizontal,
            self.spacing.get(),
            &child_sizes,
        );
        for (child, rect) in self.visual_children().iter().zip(child_rects) {
            child.arrange(rect);
        }
        final_size
    }
    fn set_spacing(&self, spacing: f32) {
        self.spacing.set(spacing);
        self.invalidate_measure();
    }
    fn construct() -> Self {
        Self {
            base: Layout::construct(),
            spacing: Cell::new(0.0),
        }
    }
    fn new() -> Rc<Self> {
        let this = Rc::new(Self::construct());
        bind_element_owner(&this);
        this
    }
}

/// `Rectangle`/`Ellipse`. A pure leaf, like `TextBlock` — no children of its own (matching real
/// WinUI3's `Shape`, which likewise has no `Children`/content property; see docs/elwindui_spec.md
/// 付録H.2.2), so its natural size is just its own drawn bounds.
/// `Shape`'s own class trait (docs/elwindui_spec.md 付録H.2.1a); `Shape` has no further
/// DSL-level subclass today.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
pub struct Shape {
    pub fill: RefCell<Option<String>>,
    pub stroke: RefCell<Option<String>>,
    pub stroke_width: Cell<f32>,
}

#[elwindui_macros::class]
impl Shape {
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        Size {
            width: 0.0,
            height: 0.0,
        }
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        final_size
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
    fn construct() -> Self {
        Self {
            base: UIElement::default(),
            fill: RefCell::new(None),
            stroke: RefCell::new(None),
            stroke_width: Cell::new(0.0),
        }
    }
}

/// `builtin::Rectangle`(docs/elwindui_builtins_spec.md 付録G/N)。かつては `elwindui-codegen`(`builtins.elwind`の`view Rectangle`)が
/// 消費クレートごとに再生成していたが、バックエンド非依存な合成 builtin はここに一度だけ手書きする
/// 方が二重管理にならない。`#[ancestor]`(`elwindui_macros::class`の doc comment 参照)で`Shape`
/// 自身の共通描画メソッドを`base`委譲として登録している。
#[elwindui_macros::class(inherits = crate::ui::Shape)]
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
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        context.rectangle(
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.arranged_width().unwrap_or(0.0),
                height: self.arranged_height().unwrap_or(0.0),
            },
            self.corner_radius.unwrap_or(0.0),
            self.base.fill.borrow().clone(),
            self.base.stroke.borrow().clone(),
            self.base.stroke_width.get(),
        );
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    // The bare (not `Rc`-wrapped) value `#[class]`'s auto-generated `new` wraps — also what a future
    // `component X inherits Rectangle` would embed unwrapped as its own `base` field, mirroring
    // `Control`/`Shape`'s own `construct` (`Rectangle` is `#[sealed]` today, so nothing actually
    // reaches this via that path yet, but the shape stays consistent with every other builtin).
    fn construct(
        fill: Option<String>,
        stroke: Option<String>,
        stroke_width: Option<f32>,
        corner_radius: Option<f32>,
    ) -> Self {
        let shape = Shape::construct();
        shape.set_fill(fill);
        shape.set_stroke(stroke);
        shape.set_stroke_width(stroke_width.unwrap_or(0.0));
        Self {
            base: shape,
            stroke_width,
            corner_radius,
        }
    }
}

/// `builtin::Ellipse`(docs/elwindui_builtins_spec.md 付録G/N)。`Rectangle`の doc comment 参照。
#[elwindui_macros::class(inherits = crate::ui::Shape)]
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
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        context.ellipse(
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.arranged_width().unwrap_or(0.0),
                height: self.arranged_height().unwrap_or(0.0),
            },
            self.base.fill.borrow().clone(),
            self.base.stroke.borrow().clone(),
            self.base.stroke_width.get(),
        );
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    // The bare (not `Rc`-wrapped) value `#[class]`'s auto-generated `new` wraps — see `Rectangle`'s
    // own `construct` doc comment for why this split exists.
    fn construct(fill: Option<String>, stroke: Option<String>, stroke_width: Option<f32>) -> Self {
        let shape = Shape::construct();
        shape.set_fill(fill);
        shape.set_stroke(stroke);
        shape.set_stroke_width(stroke_width.unwrap_or(0.0));
        Self {
            base: shape,
            stroke_width,
        }
    }
}

/// Self-drawn primitive text (WinUI3's `TextBlock`) — no native widget, unlike the native `Text`
/// this replaces. A leaf, like `NativeControlImpl`. Field named `text` (not `content`) to match `builtin::TextBlock`'s own `#[param]
/// text` name — `elwindui-codegen`'s setter-based construction calls `.set_{param name}(..)`
/// generically, so the Rust field/setter name must agree with the DSL's own field name.
/// `TextBlock`'s own class trait (docs/elwindui_spec.md 付録H.2.1a); `TextBlock` has no
/// further DSL-level subclass today.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
pub struct TextBlock {
    pub text: RefCell<String>,
    pub color: RefCell<Option<String>>,
    pub alignment: Cell<TextAlignment>,
}

#[elwindui_macros::class]
impl TextBlock {
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        // `elwindui-core` has no font metrics of its own (measurement, like painting, is a
        // backend concern for self-drawn content — see `Shape`'s same split) — a rough per-
        // character estimate is enough to avoid collapsing to zero size; a backend may still
        // render a string that overflows this estimate.
        Size {
            width: self.text.borrow().chars().count() as f32 * 8.0,
            height: 16.0,
        }
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        final_size
    }
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        context.text(
            self.text.borrow().clone(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.arranged_width().unwrap_or(0.0),
                height: self.arranged_height().unwrap_or(0.0),
            },
            self.color.borrow().clone(),
            self.alignment.get(),
        );
    }
    fn set_text(&self, text: String) {
        *self.text.borrow_mut() = text;
        self.invalidate_measure();
    }
    fn set_color(&self, color: Option<String>) {
        *self.color.borrow_mut() = color;
        self.invalidate();
    }
    fn set_text_alignment(&self, alignment: TextAlignment) {
        self.alignment.set(alignment);
        self.invalidate();
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            text: RefCell::new(String::new()),
            color: RefCell::new(None),
            alignment: Cell::new(TextAlignment::Left),
        }
    }
}

/// A composable, multi-part component (WinUI3's `Control`) — Visually built from any number of
/// other `UIElement`s (`VerticalLayout`/`HorizontalLayout`/`Shape`/`TextBlock`/
/// `NativeControlImpl`/other `Control`s), stored as its own `UIElementCollection` (the Logical
/// tree this component declares, docs/elwindui_spec.md 付録H.2.2) — unlike `Shape`, which has
/// no children at all. `padding` shrinks the area its children are overlaid into, the
/// `Control`-level analog of `margin` on an individual element.
///
/// Scope note: this is intentionally minimal for now — `content_horizontal_alignment`/
/// `content_vertical_alignment` are stored but not yet consulted by `arrange_override` (each
/// child's *own* `horizontal_alignment`/`vertical_alignment`, applied generically by `arrange`
/// below, already governs its placement within the padded content area); template
/// replacement is future work.
/// `Control`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — exposes the fields a
/// DSL-level subclass composed via `base: Control` (e.g. `builtin::ContentControl`,
/// `crates/elwindui-builtins/src/builtins.elwind`) delegates to.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
pub struct Control {
    pub padding: Cell<f32>,
    pub content_horizontal_alignment: Cell<HorizontalAlignment>,
    pub content_vertical_alignment: Cell<VerticalAlignment>,
}

#[elwindui_macros::class]
impl Control {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let inner = self
            .visual_children()
            .iter()
            .fold(Size::default(), |acc, c| {
                c.measure(available);
                let s = c.measured_size().unwrap_or_default();
                Size {
                    width: acc.width.max(s.width),
                    height: acc.height.max(s.height),
                }
            });
        grow_by_margin(inner, self.padding.get())
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: final_size.width,
            height: final_size.height,
        };
        let content_area = shrink_rect_by_margin(full, self.padding.get());
        for child in self.visual_children().iter() {
            child.arrange(content_area);
        }
        final_size
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
        self.invalidate_measure();
    }
    fn set_content_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.content_horizontal_alignment.set(alignment);
        self.invalidate_arrange();
    }
    fn set_content_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.content_vertical_alignment.set(alignment);
        self.invalidate_arrange();
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            padding: Cell::new(0.0),
            content_horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            content_vertical_alignment: Cell::new(VerticalAlignment::Stretch),
        }
    }
}

/// `builtin::ContentControl`(docs/elwindui_spec.md 付録H.2.1a)— 単一の子(`content`)を持つ
/// `Control`の薄いラッパー。`Rectangle`の doc comment 参照(同じ理由でここに直接手書きする)。
/// Content is a single Visual child managed directly by this type.
#[elwindui_macros::class(inherits = crate::ui::Control)]
pub struct ContentControl {
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
}

#[elwindui_macros::class]
impl ContentControl {
    fn content(&self) -> Rc<dyn UIElementExt> {
        self.content
            .borrow()
            .clone()
            .expect("ContentControl has no content")
    }
    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        let old = self.content.borrow_mut().replace(content.clone());
        if let Some(old) = old {
            self.as_ui_element().visual_collection.remove(&old);
        }
        self.as_ui_element().visual_collection.add(content);
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    // The bare value is embedded as the base of generated subclasses. Content is attached only
    // after that outer `Rc` exists, through `set_content`, so collection mutation owns the Visual
    // parent wiring.
    fn construct() -> Self {
        Self {
            base: Control::construct(),
            content: RefCell::new(None),
        }
    }
    fn new() -> Rc<Self> {
        Rc::<Self>::new_cyclic(|owner: &Weak<Self>| {
            let this = Self::construct();
            let owner: Weak<dyn UIElementExt> = owner.clone();
            this.as_ui_element()
                .visual_collection
                .bind_weak_owner(owner);
            this
        })
    }
}

/// WPF/WinUI3-style row/column layout (`builtin::Grid`, docs/elwindui_spec.md §3). Each child's
/// cell placement comes from its own `UIElement::attached` bag (the `Grid::row`/`Grid::column`
/// attached properties it was constructed with, read back via `grid_cell_of` since only `Grid`
/// itself knows those two fields are `i32`), not a field on `Grid` itself — see `attached`'s
/// own doc comment. A child whose cell falls outside `row_definitions`/`column_definitions`'
/// bounds is clamped to the last row/column, mirroring `grid_arrange`'s own clamping. Row/column
/// spanning is out of scope for this pass (one child per cell) — a future `#[attached]
/// row_span`/`column_span` pair on `builtin::Grid` would extend this the same way `row`/`column`
/// were added, with no changes needed here beyond consulting the extra fields.
/// `rows`/`columns` (not `row_definitions`/`column_definitions`) to match `builtin::Grid`'s own
/// `#[param] rows`/`#[param] columns` names — `elwindui-codegen`'s setter-based construction calls
/// `.set_{param name}(..)` generically, so the Rust field/setter name must agree with the DSL's.
/// `Grid`'s own class trait (docs/elwindui_spec.md 付録H.2.1a) — inherits `Layout` (like
/// `VerticalLayout`/`HorizontalLayout`), so `children` comes from that shared base rather than
/// being declared on `Grid` itself (docs/elwindui_builtins_spec.md 付録F.11).
/// Reads a child's `Grid::row`/`Grid::column` attached-property values back out of its
/// `UIElement::attached` bag — `Grid` is the only thing that knows those two fields are `i32`
/// and default to `0`, so it (not `UIElement`) owns this downcast, mirroring how
/// `elwindui-codegen`'s `emit_attached_setters` also resolves the field's declared type from the
/// owner (`Grid`) itself, never `UIElement`.
fn grid_cell_of(child: &Rc<dyn UIElementExt>) -> GridCell {
    GridCell {
        row: child.as_ui_element().get_attached("Grid", "row", 0i32),
        column: child.as_ui_element().get_attached("Grid", "column", 0i32),
    }
}

#[elwindui_macros::class(inherits = crate::ui::Layout)]
pub struct Grid {
    pub rows: RefCell<Vec<GridLength>>,
    pub columns: RefCell<Vec<GridLength>>,
}

#[elwindui_macros::class]
impl Grid {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let children = self.children().to_vec();
        let cells: Vec<GridCell> = children.iter().map(grid_cell_of).collect();
        let child_sizes: Vec<Size> = children
            .iter()
            .map(|c| {
                c.measure(available);
                c.measured_size().unwrap_or_default()
            })
            .collect();
        grid_natural_size(
            &self.rows.borrow(),
            &self.columns.borrow(),
            &cells,
            &child_sizes,
        )
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let children = self.children().to_vec();
        let cells: Vec<GridCell> = children.iter().map(grid_cell_of).collect();
        let child_sizes: Vec<Size> = children
            .iter()
            .map(|c| c.measured_size().unwrap_or_default())
            .collect();
        let child_rects = grid_arrange(
            final_size,
            &self.rows.borrow(),
            &self.columns.borrow(),
            &cells,
            &child_sizes,
        );
        for (child, rect) in children.iter().zip(child_rects) {
            child.arrange(rect);
        }
        final_size
    }
    fn set_rows(&self, rows: Vec<GridLength>) {
        *self.rows.borrow_mut() = rows;
        self.invalidate_measure();
    }
    fn set_columns(&self, columns: Vec<GridLength>) {
        *self.columns.borrow_mut() = columns;
        self.invalidate_measure();
    }
    fn construct() -> Self {
        Self {
            base: Layout::construct(),
            rows: RefCell::new(Vec::new()),
            columns: RefCell::new(Vec::new()),
        }
    }
    fn new() -> Rc<Self> {
        let this = Rc::new(Self::construct());
        bind_element_owner(&this);
        this
    }
}

/// WinUI3's `FrameworkElement.MeasureCore`-style constraint step, used by `UIElement::measure`: an
/// explicit `width`/`height` overrides that axis outright, then both axes are clamped to
/// `min_width..max_width`/`min_height..max_height` (`crate::layout::apply_size_constraints`).
/// Applied twice per element per the same WinUI3 algorithm — once to the space handed down to
/// `measure_override` (a fixed `Width` shouldn't let a container measure against the parent's
/// *actual* available space), once to `measure_override`'s own returned size (a container's
/// natural content size shouldn't override an explicit `Width`/`Height`/`Max*`). Generic over
/// `?Sized` so it can be called with `self: &Self` from inside the `measure` trait default method
/// (where `Self` isn't known to be `Sized`, since `measure` must stay callable through
/// `dyn UIElement`) without an unsized coercion.
fn constrain<T: UIElementExt + ?Sized>(elem: &T, size: Size) -> Size {
    let overridden = Size {
        width: elem.width().unwrap_or(size.width),
        height: elem.height().unwrap_or(size.height),
    };
    apply_size_constraints(
        overridden,
        elem.min_width(),
        elem.max_width(),
        elem.min_height(),
        elem.max_height(),
    )
}

/// This element's natural (unconstrained) size — e.g. for a container that must report an
/// `intrinsicContentSize` to an Auto-Layout-managed ancestor (see `elwindui-backend-appkit`'s
/// `TreeHostView`) before it has ever actually been given a frame to lay out into.
pub fn natural_size(elem: &dyn UIElementExt) -> Size {
    elem.measure(Size {
        width: 0.0,
        height: 0.0,
    });
    elem.measured_size().unwrap_or_default()
}

/// Records one Visual's local retained commands. Geometry and hierarchy are reconciled separately
/// so a dirty Visual does not require replacing its RenderGroup allocation.
fn record_group_commands<H: Clone + 'static>(elem: &Rc<dyn UIElementExt>, group: &mut RenderGroup) {
    group.commands.clear();
    let size = Size {
        width: elem.arranged_width().unwrap_or(0.0),
        height: elem.arranged_height().unwrap_or(0.0),
    };
    let mut context = RenderContext::begin_group(&mut group.commands, group.offset, group.clip);
    if let Some(native) = elem
        .as_ref()
        .try_as_native_control()
        .and_then(|value| value.downcast_ref::<H>())
    {
        context.native_control(
            group.id,
            Rc::new(native.clone()),
            Rect {
                x: 0.0,
                y: 0.0,
                width: size.width,
                height: size.height,
            },
        );
    }
    elem.render(&mut context);
    context.end_group();
}

/// Builds one retained RenderGroup for every arranged, visible Visual.
fn build_render_group<H: Clone + 'static>(
    elem: &Rc<dyn UIElementExt>,
    offset: Point,
) -> Option<RenderGroup> {
    if elem.visibility() == Visibility::Collapsed {
        return None;
    }
    let size = Size {
        width: elem.arranged_width().unwrap_or(0.0),
        height: elem.arranged_height().unwrap_or(0.0),
    };
    let clip = elem.clip_to_bounds().then_some(Rect {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    });
    let id = elem.render_group_id();
    let mut group = RenderGroup::new(id, offset, clip);
    group.size = size;
    record_group_commands::<H>(elem, &mut group);
    for child in elem.visual_children() {
        let child_offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        if let Some(child_group) = build_render_group::<H>(&child, child_offset) {
            group.children.push(child_group);
        }
    }
    group.is_dirty = false;
    Some(group)
}

/// Measures and arranges a host's content root. Rendering is intentionally separate: a host keeps
/// its RenderTree and calls `RenderTree::new` once, then `RenderTree::reconcile` after each layout.
pub fn layout_root(root: &Rc<dyn UIElementExt>, available: Size) {
    root.measure(available);
    let allotted = Rect {
        x: 0.0,
        y: 0.0,
        width: available.width,
        height: available.height,
    };
    root.arrange(allotted);
}

fn index_render_groups(
    elem: &Rc<dyn UIElementExt>,
    group: &RenderGroup,
    path: Vec<usize>,
    group_paths: &mut HashMap<u64, Vec<usize>>,
    visual_index: &mut HashMap<u64, Weak<dyn UIElementExt>>,
) {
    group_paths.insert(group.id, path.clone());
    visual_index.insert(group.id, Rc::downgrade(elem));
    let mut group_children = group.children.iter().enumerate();
    for child in elem.visual_children() {
        if child.visibility() == Visibility::Collapsed {
            continue;
        }
        let Some((child_index, child_group)) = group_children.next() else {
            break;
        };
        let mut child_path = path.clone();
        child_path.push(child_index);
        index_render_groups(&child, child_group, child_path, group_paths, visual_index);
    }
}

fn reconcile_render_group<H: Clone + 'static>(
    elem: &Rc<dyn UIElementExt>,
    group: &mut RenderGroup,
    offset: Point,
) {
    let size = Size {
        width: elem.arranged_width().unwrap_or(0.0),
        height: elem.arranged_height().unwrap_or(0.0),
    };
    let clip = elem.clip_to_bounds().then_some(Rect {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    });
    if group.offset != offset || group.size != size || group.clip != clip {
        group.offset = offset;
        group.size = size;
        group.clip = clip;
        group.is_dirty = true;
    }

    let old_children = std::mem::take(&mut group.children);
    let mut old_by_id: HashMap<u64, RenderGroup> = old_children
        .into_iter()
        .map(|child| (child.id, child))
        .collect();
    let mut children = Vec::new();
    for child in elem.visual_children() {
        if child.visibility() == Visibility::Collapsed {
            continue;
        }
        let child_offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let id = child.render_group_id();
        let child_group = if let Some(mut existing) = old_by_id.remove(&id) {
            reconcile_render_group::<H>(&child, &mut existing, child_offset);
            existing
        } else {
            group.is_dirty = true;
            build_render_group::<H>(&child, child_offset)
                .expect("visible Visual must have a RenderGroup")
        };
        children.push(child_group);
    }
    if !old_by_id.is_empty() {
        group.is_dirty = true;
    }
    group.children = children;
    if group.is_dirty {
        record_group_commands::<H>(elem, group);
        group.is_dirty = false;
    }
}

impl RenderTree {
    /// Creates the initial retained tree from a layout-complete content root.
    pub fn new<H: Clone + 'static>(root: &Rc<dyn UIElementExt>) -> Self {
        let offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let root_group = build_render_group::<H>(root, offset)
            .unwrap_or_else(|| RenderGroup::new(root.render_group_id(), offset, None));
        let mut tree = Self::with_root(root_group);
        index_render_groups(
            root,
            &tree.root,
            Vec::new(),
            &mut tree.group_paths,
            &mut tree.visual_index,
        );
        tree
    }

    /// Reconciles an already retained tree after `layout_root`. Group identities and clean command
    /// buffers survive; only changed or explicitly invalidated groups record commands again.
    pub fn reconcile<H: Clone + 'static>(&mut self, root: &Rc<dyn UIElementExt>) -> bool {
        if self.root.id != root.render_group_id() {
            return false;
        }
        let offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        reconcile_render_group::<H>(root, &mut self.root, offset);
        self.group_paths.clear();
        self.visual_index.clear();
        index_render_groups(
            root,
            &self.root,
            Vec::new(),
            &mut self.group_paths,
            &mut self.visual_index,
        );
        true
    }

    pub fn root_id(&self) -> u64 {
        self.root.id
    }
}

fn rect_contains(rect: Rect, at: Point) -> bool {
    at.x >= rect.x && at.x <= rect.x + rect.width && at.y >= rect.y && at.y <= rect.y + rect.height
}

/// Re-runs the same read-only traversal `collect_render_items` (above) does, without needing to
/// know any backend's native handle type — hit-testing only needs each element's own already-
/// `arrange`d rect, never its handle. Returns the deepest (topmost) element whose rect contains
/// `at`, or `None` if `at` falls outside `elem`'s own bounds entirely. See
/// `elwindui_core::input::InputRouter`'s doc comment (modeled on WinUI3's routed events) —
/// bubbling from the returned element is then just `dispatch_routed` following `parent()`, no
/// path/ancestor computation needed here.
fn hit_test_at(
    elem: &Rc<dyn UIElementExt>,
    absolute_origin: Point,
    at: Point,
) -> Option<Rc<dyn UIElementExt>> {
    // A `Collapsed` element (and its whole subtree) is excluded from hit-testing, matching
    // `collect_render_items`'s own treatment — see `Visibility`'s own doc comment.
    if elem.visibility() == Visibility::Collapsed {
        return None;
    }
    let width = elem.arranged_width().unwrap_or(0.0);
    let height = elem.arranged_height().unwrap_or(0.0);
    let absolute_rect = Rect {
        x: absolute_origin.x,
        y: absolute_origin.y,
        width,
        height,
    };
    if !rect_contains(absolute_rect, at) {
        return None;
    }

    // Children are searched last-to-first: traversal order paints later children on top of
    // earlier ones (see 付録N's z-order note), so the *last* child whose own rect contains `at`
    // is the topmost, correctly-hit one.
    for child in elem.visual_children().iter().rev() {
        let offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let child_origin = Point {
            x: absolute_origin.x + offset.x,
            y: absolute_origin.y + offset.y,
        };
        if let Some(hit) = hit_test_at(child, child_origin, at) {
            return Some(hit);
        }
    }

    Some(Rc::clone(elem))
}

/// Hit-tests `root` at `at` (absolute coordinates, e.g. the hosting `TreeHostView`'s own local
/// point). Returns the deepest (topmost) hit element, or `None` if `at` falls outside `root`'s own
/// bounds entirely. Requires `root` to have already been laid out (e.g. via `layout_root`) — reads
/// cached `arranged_width`/`arranged_height`/`arranged_offset`, doesn't recompute them.
pub fn hit_test(root: &Rc<dyn UIElementExt>, at: Point) -> Option<Rc<dyn UIElementExt>> {
    // See `layout_root`'s own matching comment — `root`'s own `arranged_offset` (from its margin/
    // alignment against the original allotted rect) must be folded in here too, so hit-testing
    // agrees with `collect_render_items`'s rendered coordinates.
    let root_offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
    hit_test_at(root, root_offset, at)
}

/// Bubbles a routed event starting at `target` (e.g. `hit_test`'s return value, or a native leaf's
/// own tree node — see `elwindui-backend-appkit`'s `TreeHostView`): calls `target`'s own handlers
/// registered under `name` (via `UIElement::register_routed_handler::<T>`), then its parent's,
/// and so on up to the root (`UIElement::parent`), stopping as soon as one sets `args.handled`.
/// Works identically whether `target`'s tree was built by a single static `.elwind` traversal or
/// assembled at runtime (e.g. `TabView`'s `items_source`/`item_template`). `T` must match the type every handler for `name`
/// was registered with — see `UIElement::routed_handlers`'s doc comment for why the downcast
/// this performs always succeeds in practice (both sides come from the same `.elwind` field
/// declaration).
pub fn dispatch_routed<T: 'static>(
    target: &Rc<dyn UIElementExt>,
    name: &str,
    payload: &T,
    args: &RoutedEventArgs,
) {
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

    fn layout_tree<H: Clone + 'static>(root: &Rc<dyn UIElementExt>, available: Size) -> RenderTree {
        layout_root(root, available);
        RenderTree::new::<H>(root)
    }

    #[derive(Clone, PartialEq, Debug)]
    struct FakeHandle(&'static str, Size);

    impl FakeHandle {
        fn measure(&self, _available: Size) -> Size {
            self.1
        }
    }

    /// A minimal stand-in for a real backend's own `NativeControl`-implementing widget base (e.g.
    /// `elwindui-backend-appkit::NativeControlImpl { handle: AnyView, .. }`, shared by that backend's
    /// `TextArea`/`Button`/`TabView`) — exercises the same "concrete implementor writes its own
    /// `measure_override`/`try_as_native_control`" pattern those use, instead of relying on any
    /// generic measuring behavior from `elwindui-core::ui::NativeControl` itself (a pure marker trait
    /// — see that trait's own doc comment). Named `FakeNativeControl`, not the bare `NativeControl`
    /// that trait already uses, because `#[class]`-generated `__elwindui_inherit_*!` macros share a
    /// single flat, crate-wide namespace (unlike ordinary Rust items, which can share a bare name
    /// across different modules) — a same-crate bare-name collision is a real `E0428`, not just a
    /// registry ambiguity the way it used to be.
    #[elwindui_macros::class(struct_only = crate::ui::NativeControlExt, inherits = crate::ui::UIElement)]
    struct FakeNativeControl {
        handle: FakeHandle,
    }

    #[elwindui_macros::class]
    impl FakeNativeControl {
        #[overrides]
        fn measure_override(&self, available: Size) -> Size {
            self.handle.measure(available)
        }
        #[overrides]
        fn try_as_native_control(&self) -> Option<&dyn Any> {
            Some(&self.handle)
        }
        fn new(handle: FakeHandle) -> Rc<Self> {
            let this = Rc::new(Self {
                base: UIElement::default(),
                handle,
            });
            bind_element_owner(&this);
            this
        }
    }

    /// `#[overridable]`/`#[overrides]` usage example, exercised across a genuine 3-hop chain
    /// (`OverridableBase` -> `OverridableMid` -> `OverridableLeaf`) with two overridable methods —
    /// `OverridableMid` overrides only `label`, leaving `compute` untouched, and `OverridableLeaf`
    /// (which itself overrides neither) relies on defaults for both. This is exactly the scenario a
    /// single shared `#dyn_ident` accessor per trait used to get wrong (always reaching
    /// `OverridableBase`'s original `compute`/`label`, skipping `OverridableMid`'s own `label`
    /// override, because the accessor could only be reflexive-for-the-whole-trait or not) — see
    /// `per_method_accessor_ident`'s own doc comment for the fix (one dedicated accessor per
    /// `#[overridable]` method, resolved independently).
    #[elwindui_macros::class(inherits = crate::ui::UIElement)]
    struct OverridableBase {
        value: Cell<i32>,
    }

    #[elwindui_macros::class]
    impl OverridableBase {
        #[overridable]
        fn compute(&self, x: i32) -> i32 {
            x + self.value.get()
        }
        #[overridable]
        fn label(&self) -> &'static str {
            "base"
        }
        fn new() -> Self {
            Self {
                base: UIElement::default(),
                value: Cell::new(1),
            }
        }
    }

    /// hop-1: overrides only `label`, leaves `compute` untouched at `OverridableBase`'s own
    /// default — the partial-override case.
    #[elwindui_macros::class(inherits = crate::ui::tests::OverridableBase)]
    struct OverridableMid {}

    #[elwindui_macros::class]
    impl OverridableMid {
        #[overrides]
        fn label(&self) -> &'static str {
            "mid"
        }
        fn new() -> Self {
            Self {
                base: OverridableBase::new(),
            }
        }
    }

    /// hop-2: overrides neither method itself — both must resolve via defaults, dispatching
    /// through `OverridableMid`'s per-method accessors: `label` should stop at `OverridableMid`'s
    /// own override, `compute` should pass through it to reach `OverridableBase`'s original.
    #[elwindui_macros::class(inherits = crate::ui::tests::OverridableMid)]
    struct OverridableLeaf {}

    #[elwindui_macros::class]
    impl OverridableLeaf {
        fn new() -> Self {
            Self {
                base: OverridableMid::new(),
            }
        }
    }

    #[test]
    fn overridable_override_dispatches_through_inherit_macro() {
        let base = OverridableBase::new();
        assert_eq!(OverridableBaseExt::compute(&base, 5), 6);
        assert_eq!(OverridableBaseExt::label(&base), "base");

        let mid = OverridableMid::new();
        // `compute` isn't overridden at this hop — falls back to `OverridableBase`'s own default.
        assert_eq!(OverridableBaseExt::compute(&mid, 5), 6);
        assert_eq!(OverridableBaseExt::label(&mid), "mid");

        let leaf = OverridableLeaf::new();
        // Neither is overridden at `OverridableLeaf` itself: `compute` passes all the way through
        // `OverridableMid` (which never touched it) to `OverridableBase`'s original, while `label`
        // stops at `OverridableMid`'s own override — the exact case a single shared accessor got
        // wrong before the per-method accessor fix.
        assert_eq!(OverridableBaseExt::compute(&leaf, 5), 6);
        assert_eq!(OverridableBaseExt::label(&leaf), "mid");
    }

    fn size(width: f32, height: f32) -> Size {
        Size { width, height }
    }

    fn native(name: &'static str, size: Size) -> Rc<dyn UIElementExt> {
        FakeNativeControl::new(FakeHandle(name, size))
    }

    fn stack(
        orientation: Orientation,
        spacing: f32,
        children: Vec<Rc<dyn UIElementExt>>,
    ) -> Rc<dyn UIElementExt> {
        match orientation {
            Orientation::Vertical => {
                let node = VerticalLayout::new();
                node.set_spacing(spacing);
                for child in children {
                    node.children().add(child);
                }
                node
            }
            Orientation::Horizontal => {
                let node = HorizontalLayout::new();
                node.set_spacing(spacing);
                for child in children {
                    node.children().add(child);
                }
                node
            }
        }
    }

    fn split(tree: RenderTree) -> (Vec<(FakeHandle, Rect)>, Vec<(RenderCommand, Rect)>) {
        let mut natives = Vec::new();
        let mut paints = Vec::new();
        fn visit(
            group: &RenderGroup,
            origin: Point,
            natives: &mut Vec<(FakeHandle, Rect)>,
            paints: &mut Vec<(RenderCommand, Rect)>,
        ) {
            let origin = Point {
                x: origin.x + group.offset.x,
                y: origin.y + group.offset.y,
            };
            for command in &group.commands {
                match command {
                    RenderCommand::NativeControl { handle, rect, .. } => {
                        if let Some(handle) = handle.downcast_ref::<FakeHandle>() {
                            natives.push((
                                handle.clone(),
                                Rect {
                                    x: origin.x + rect.x,
                                    y: origin.y + rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                            ));
                        }
                    }
                    RenderCommand::Rectangle { rect, .. }
                    | RenderCommand::Ellipse { rect, .. }
                    | RenderCommand::Image { rect, .. } => paints.push((
                        command.clone(),
                        Rect {
                            x: origin.x + rect.x,
                            y: origin.y + rect.y,
                            width: rect.width,
                            height: rect.height,
                        },
                    )),
                    RenderCommand::Text { rect, .. } => paints.push((
                        command.clone(),
                        Rect {
                            x: origin.x + rect.x,
                            y: origin.y + rect.y,
                            width: rect.width,
                            height: rect.height,
                        },
                    )),
                    RenderCommand::Line { .. } | RenderCommand::Path { .. } => paints.push((
                        command.clone(),
                        Rect {
                            x: origin.x,
                            y: origin.y,
                            width: 0.0,
                            height: 0.0,
                        },
                    )),
                }
            }
            for child in &group.children {
                visit(child, origin, natives, paints);
            }
        }
        visit(
            &tree.root,
            Point { x: 0.0, y: 0.0 },
            &mut natives,
            &mut paints,
        );
        (natives, paints)
    }

    #[test]
    fn single_native_leaf_as_root_fills_available_space() {
        // The root's default alignment is `Stretch`, so it fills `available` regardless of its
        // own measured size — this matters for e.g. `TabView` (a native leaf) as `Window`'s
        // content: it must fill the window, not shrink to its own `fittingSize()`.
        let tree = native("a", size(10.0, 20.0));
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 100.0)));
        assert_eq!(
            natives,
            vec![(
                FakeHandle("a", size(10.0, 20.0)),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 100.0
                }
            )]
        );
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
        fn leaf(name: &'static str, s: Size) -> Rc<dyn UIElementExt> {
            let node = FakeNativeControl::new(FakeHandle(name, s));
            node.as_ui_element()
                .set_horizontal_alignment(HorizontalAlignment::Left);
            node.as_ui_element()
                .set_vertical_alignment(VerticalAlignment::Top);
            node
        }
        fn start_stack(
            orientation: Orientation,
            spacing: f32,
            children: Vec<Rc<dyn UIElementExt>>,
        ) -> Rc<dyn UIElementExt> {
            let node: Rc<dyn UIElementExt> = match orientation {
                Orientation::Vertical => {
                    let stack = VerticalLayout::new();
                    stack.set_spacing(spacing);
                    for child in children {
                        stack.children().add(child);
                    }
                    stack
                }
                Orientation::Horizontal => {
                    let stack = HorizontalLayout::new();
                    stack.set_spacing(spacing);
                    for child in children {
                        stack.children().add(child);
                    }
                    stack
                }
            };
            node.as_ui_element()
                .set_horizontal_alignment(HorizontalAlignment::Left);
            node.as_ui_element()
                .set_vertical_alignment(VerticalAlignment::Top);
            node
        }

        let tree = start_stack(
            Orientation::Vertical,
            5.0,
            vec![
                leaf("top", size(50.0, 10.0)),
                start_stack(
                    Orientation::Horizontal,
                    2.0,
                    vec![
                        leaf("left", size(20.0, 20.0)),
                        leaf("right", size(30.0, 20.0)),
                    ],
                ),
            ],
        );

        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert!(paints.is_empty());
        assert_eq!(natives.len(), 3);
        assert_eq!(
            natives[0],
            (
                FakeHandle("top", size(50.0, 10.0)),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 50.0,
                    height: 10.0
                }
            )
        );
        // inner stack starts at y = 10 (top's height) + 5 (spacing) = 15
        assert_eq!(
            natives[1],
            (
                FakeHandle("left", size(20.0, 20.0)),
                Rect {
                    x: 0.0,
                    y: 15.0,
                    width: 20.0,
                    height: 20.0
                }
            )
        );
        assert_eq!(
            natives[2],
            (
                FakeHandle("right", size(30.0, 20.0)),
                Rect {
                    x: 22.0,
                    y: 15.0,
                    width: 30.0,
                    height: 20.0
                }
            )
        );
    }

    #[test]
    fn stretch_default_fills_the_cross_axis_slot() {
        // Unlike the previous test, this one leaves alignment at its `Stretch` default — each
        // leaf should fill the *entire* stack width (the cross axis, for a vertical stack), not
        // just its own measured width.
        let tree = stack(
            Orientation::Vertical,
            0.0,
            vec![native("a", size(10.0, 20.0))],
        );
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 20.0
            }
        );
    }

    #[test]
    fn abstract_shape_has_no_commands_and_no_children() {
        // `Shape` (matching real WinUI3's `Shape`) is a pure leaf: no `Children`/content property
        // of its own — see `Shape`'s own doc comment.
        let shape = Shape::new();
        shape.set_fill(Some("#3498db".to_string()));
        let tree: Rc<dyn UIElementExt> = shape;

        assert!(tree.visual_children().is_empty());
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 50.0)));
        assert!(natives.is_empty());
        assert!(
            paints.is_empty(),
            "Shape is abstract; Rectangle/Ellipse render concrete commands"
        );
        assert!(natives.is_empty());
    }

    #[test]
    fn control_padding_shrinks_the_slot_its_children_are_arranged_into() {
        let control = ContentControl::new();
        control.set_padding(10.0);
        control.set_content(native("a", size(10.0, 20.0)));
        let tree: Rc<dyn UIElementExt> = control;
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 80.0
            }
        );
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
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element().set_margin(10.0);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 80.0
            }
        );
    }

    #[test]
    fn explicit_width_and_height_override_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element().set_width(Some(50.0));
        tree.as_ui_element().set_height(Some(5.0));
        // `Stretch` (the default) still governs slot placement; the explicit width/height above
        // constrains what `measure_override`'s own `available`/`desired` see, not the final
        // stretch-to-slot size — a non-`Stretch` alignment (below) is what actually surfaces the
        // explicit size in the arranged rect.
        tree.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        tree.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 5.0
            }
        );
    }

    #[test]
    fn min_and_max_clamp_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element().set_min_width(Some(30.0));
        tree.as_ui_element().set_max_height(Some(8.0));
        tree.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        tree.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 8.0
            }
        );
    }

    #[test]
    fn arranged_width_height_and_offset_are_populated_after_layout() {
        let leaf = native("a", size(10.0, 20.0));
        leaf.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        leaf.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let root = stack(
            Orientation::Vertical,
            5.0,
            vec![native("top", size(50.0, 10.0)), Rc::clone(&leaf)],
        );
        layout_tree::<FakeHandle>(&root, size(200.0, 200.0));

        assert_eq!(root.arranged_width(), Some(200.0));
        assert_eq!(root.arranged_height(), Some(200.0));
        assert_eq!(
            root.arranged_offset(),
            Some(Point { x: 0.0, y: 0.0 }),
            "root has no parent to set its own offset"
        );
        // second stack child ("top" is 10 tall, spacing is 5) starts at y = 15, relative to the stack
        assert_eq!(leaf.arranged_offset(), Some(Point { x: 0.0, y: 15.0 }));
        assert_eq!(leaf.arranged_width(), Some(10.0));
        assert_eq!(leaf.arranged_height(), Some(20.0));
    }

    #[test]
    fn measured_size_and_arranged_state_are_none_before_layout_and_after_invalidate() {
        let leaf = native("a", size(10.0, 20.0));
        assert_eq!(leaf.measured_size(), None);
        assert_eq!(leaf.arranged_width(), None);
        assert_eq!(leaf.arranged_height(), None);
        assert_eq!(leaf.arranged_offset(), None);

        leaf.measure(size(200.0, 200.0));
        assert_eq!(leaf.measured_size(), Some(size(10.0, 20.0)));
        leaf.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        });
        assert!(leaf.arranged_width().is_some());
        assert!(leaf.arranged_height().is_some());
        assert!(leaf.arranged_offset().is_some());

        leaf.invalidate_arrange();
        assert!(
            leaf.measured_size().is_some(),
            "invalidate_arrange must not touch measured_size"
        );
        assert_eq!(leaf.arranged_width(), None);
        assert_eq!(leaf.arranged_height(), None);
        assert_eq!(leaf.arranged_offset(), None);

        leaf.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        });
        leaf.invalidate_measure();
        assert_eq!(leaf.measured_size(), None);
        assert_eq!(leaf.arranged_width(), None);
        assert_eq!(leaf.arranged_height(), None);
        assert_eq!(leaf.arranged_offset(), None);
    }

    #[test]
    fn non_stretch_alignment_keeps_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Center);
        tree.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Center);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 45.0,
                y: 40.0,
                width: 10.0,
                height: 20.0
            }
        );
    }

    /// A minimal test-only fixture that both paints itself *and* has children — no real builtin
    /// combines the two today (`Shape` is a childless leaf; `Layout`/`Control`/`Grid`
    /// never paint), so `render_item_ordering_preserves_traversal_order_across_native_and_paint`
    /// (below) needs its own local type to exercise the paint-then-child traversal order.
    struct PaintingContainer {
        base: UIElement,
    }

    impl UIElementExt for PaintingContainer {
        fn as_ui_element(&self) -> &UIElement {
            &self.base
        }
        // Forwards to `self.base` (not reflexive `{ self }`) -- unlike `UIElement` itself (the
        // true declaring class, which explicitly overrides every one of its own `#[overridable]`
        // methods and so can never recurse through its own default), `PaintingContainer` does NOT
        // override `visual_children`/`try_as_native_control`, so a reflexive accessor here would
        // make their trait defaults dispatch straight back to `PaintingContainer` itself forever
        // (stack overflow) instead of reaching `UIElement`'s own real bodies.
        fn __dyn_ui_element(&self) -> &dyn UIElementExt {
            self.base.__dyn_ui_element()
        }
        // `visual_children`/`try_as_native_control` aren't overridden here, so their accessors
        // forward to `self.base` (same reasoning as `__dyn_ui_element` above).
        fn __dyn_x_for_visual_children(&self) -> &dyn UIElementExt {
            self.base.__dyn_x_for_visual_children()
        }
        fn __dyn_x_for_measure_override(&self) -> &dyn UIElementExt {
            self
        }
        fn __dyn_x_for_arrange_override(&self) -> &dyn UIElementExt {
            self
        }
        fn __dyn_x_for_render(&self) -> &dyn UIElementExt {
            self
        }
        fn __dyn_x_for_try_as_native_control(&self) -> &dyn UIElementExt {
            self.base.__dyn_x_for_try_as_native_control()
        }
        fn measure_override(&self, available: Size) -> Size {
            self.base
                .visual_children()
                .iter()
                .fold(Size::default(), |acc, c| {
                    c.measure(available);
                    let s = c.measured_size().unwrap_or_default();
                    Size {
                        width: acc.width.max(s.width),
                        height: acc.height.max(s.height),
                    }
                })
        }
        fn arrange_override(&self, final_size: Size) -> Size {
            let full = Rect {
                x: 0.0,
                y: 0.0,
                width: final_size.width,
                height: final_size.height,
            };
            for child in self.base.visual_children().iter() {
                child.arrange(full);
            }
            final_size
        }
        fn render(&self, context: &mut RenderContext<'_>) {
            context.rectangle(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: self.arranged_width().unwrap_or(0.0),
                    height: self.arranged_height().unwrap_or(0.0),
                },
                4.0,
                Some("#000000".to_string()),
                None,
                0.0,
            );
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
        let tree = Rc::new(PaintingContainer {
            base: UIElement::default(),
        });
        bind_element_owner(&tree);
        tree.as_ui_element()
            .visual_collection
            .add(native("child", size(10.0, 10.0)));
        let tree: Rc<dyn UIElementExt> = tree;
        let render_tree = layout_tree::<FakeHandle>(&tree, size(50.0, 50.0));
        assert!(matches!(
            render_tree.root.commands[0],
            RenderCommand::Rectangle { .. }
        ));
        assert!(matches!(
            render_tree.root.children[0].commands[0],
            RenderCommand::NativeControl { .. }
        ));
    }

    #[test]
    fn text_block_defaults_to_left_alignment_and_set_text_alignment_updates_paint() {
        let text_block = TextBlock::construct();
        assert_eq!(text_block.alignment.get(), TextAlignment::Left);
        let mut commands = Vec::new();
        text_block.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                alignment: TextAlignment::Left,
                ..
            }
        ));

        text_block.set_text_alignment(TextAlignment::Center);
        commands.clear();
        text_block.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                alignment: TextAlignment::Center,
                ..
            }
        ));
    }

    #[test]
    fn render_tree_indexes_stable_visual_ids_and_marks_only_target_group_dirty() {
        let child = native("child", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&child)]);
        let mut render_tree = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        let child_id = child.render_group_id();
        assert!(render_tree.group_paths.contains_key(&child_id));
        assert!(render_tree.visual_index[&child_id].upgrade().is_some());
        assert!(!render_tree.root.is_dirty);
        assert!(render_tree.mark_dirty(child_id));
        assert!(!render_tree.root.is_dirty);
        assert!(render_tree.root.children[0].is_dirty);
    }

    #[test]
    fn reconcile_reuses_matching_root_and_discards_removed_visual_indexes() {
        let first = native("first", size(10.0, 10.0));
        let second = native("second", size(10.0, 10.0));
        let root = stack(
            Orientation::Vertical,
            0.0,
            vec![Rc::clone(&first), Rc::clone(&second)],
        );
        layout_root(&root, size(40.0, 40.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&root);
        let root_address = (&render_tree.root as *const RenderGroup) as usize;
        let first_id = first.render_group_id();
        let second_id = second.render_group_id();

        assert!(render_tree.mark_dirty(first_id));
        layout_root(&root, size(40.0, 40.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));
        assert_eq!(
            root_address,
            (&render_tree.root as *const RenderGroup) as usize
        );
        assert!(render_tree.group_paths.contains_key(&first_id));
        assert!(render_tree.group_paths.contains_key(&second_id));

        assert!(root.as_ui_element().visual_collection.remove(&second));
        layout_root(&root, size(40.0, 40.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));
        assert!(!render_tree.group_paths.contains_key(&second_id));
        assert!(!render_tree.mark_dirty(second_id));
    }

    #[test]
    fn reconcile_rejects_a_different_content_root() {
        let first = native("first", size(10.0, 10.0));
        let second = native("second", size(10.0, 10.0));
        layout_root(&first, size(20.0, 20.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&first);
        layout_root(&second, size(20.0, 20.0));
        assert!(!render_tree.reconcile::<FakeHandle>(&second));
        assert_eq!(render_tree.root_id(), first.render_group_id());
    }

    #[test]
    fn reconcile_rerecords_native_commands_when_only_arranged_size_changes() {
        let root = native("root", size(10.0, 10.0));
        layout_root(&root, size(40.0, 30.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&root);
        let native_rect = |tree: &RenderTree| match &tree.root.commands[0] {
            RenderCommand::NativeControl { rect, .. } => *rect,
            _ => panic!("expected native command"),
        };
        assert_eq!(native_rect(&render_tree).width, 40.0);

        layout_root(&root, size(100.0, 80.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));
        assert_eq!(native_rect(&render_tree).width, 100.0);
        assert_eq!(native_rect(&render_tree).height, 80.0);
    }

    #[test]
    fn clip_to_bounds_defaults_false_and_inherits_from_visual_parent() {
        let child = native("child", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&child)]);
        assert!(!child.clip_to_bounds());
        root.set_clip_to_bounds(Some(true));
        assert!(child.clip_to_bounds());
        let render_tree = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(render_tree.root.clip.is_some());
        assert!(render_tree.root.children[0].clip.is_some());
        child.set_clip_to_bounds(Some(false));
        let render_tree = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(render_tree.root.children[0].clip.is_none());
    }

    #[test]
    fn logical_and_visual_parents_are_set_by_collections() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        assert!(Rc::ptr_eq(
            &leaf.parent().expect("leaf should have a logical parent"),
            &root
        ));
        assert!(Rc::ptr_eq(
            &leaf
                .visual_parent()
                .expect("leaf should have a visual parent"),
            &root
        ));
        assert!(root.parent().is_none());
    }

    #[test]
    fn runtime_add_and_remove_after_construction_wire_parent_and_visual_children() {
        // `UIElementCollection::add`/`remove` must work *after* the owner is already `Rc`-wrapped
        // after the owner is already constructed.
        let root = VerticalLayout::new();
        let root_erased: Rc<dyn UIElementExt> = root.clone();
        let children = root.children().clone();
        assert!(root.visual_children().is_empty());

        let child = native("a", size(10.0, 20.0));
        children.add(Rc::clone(&child));

        assert_eq!(root.visual_children().len(), 1);
        assert!(Rc::ptr_eq(
            &child
                .parent()
                .expect("add should wire the child's logical parent"),
            &root_erased
        ));
        assert!(Rc::ptr_eq(
            &child
                .visual_parent()
                .expect("add should wire the child's visual parent"),
            &root_erased
        ));

        assert!(children.remove(&child));
        assert!(root.visual_children().is_empty());
        assert!(
            child.parent().is_none(),
            "remove should clear the child's parent"
        );
        assert!(
            child.visual_parent().is_none(),
            "remove should clear the child's visual parent"
        );
    }

    #[test]
    fn logical_and_visual_collections_keep_their_parent_relationships_separate() {
        let root = VerticalLayout::new();
        let root_erased: Rc<dyn UIElementExt> = root.clone();

        let visual_only = TextBlock::new();
        root.as_ui_element()
            .visual_collection
            .add(visual_only.clone());
        assert!(visual_only.parent().is_none());
        assert!(Rc::ptr_eq(
            &visual_only.visual_parent().expect("visual parent"),
            &root_erased
        ));

        let logical_child = TextBlock::new();
        root.children().add(logical_child.clone());
        assert!(Rc::ptr_eq(
            &logical_child.parent().expect("logical parent"),
            &root_erased
        ));
        assert!(Rc::ptr_eq(
            &logical_child.visual_parent().expect("visual parent"),
            &root_erased
        ));
    }

    #[test]
    fn content_control_replaces_its_visual_child() {
        let first = TextBlock::new();
        let content_control = ContentControl::new();
        content_control.set_content(first.clone());
        let control: Rc<dyn UIElementExt> = content_control.clone();
        assert!(Rc::ptr_eq(
            &first.visual_parent().expect("initial visual parent"),
            &control
        ));

        let second = TextBlock::new();
        content_control.set_content(second.clone());
        assert!(first.visual_parent().is_none());
        assert!(Rc::ptr_eq(
            &second.visual_parent().expect("replacement visual parent"),
            &control
        ));
        assert_eq!(content_control.visual_children().len(), 1);
    }

    #[test]
    fn invalidate_family_reaches_a_relayout_host_registered_on_the_root() {
        struct CountingHost {
            calls: Rc<RefCell<usize>>,
        }
        impl RelayoutHost for CountingHost {
            fn request_relayout(&self, _dirty_group_id: u64) {
                *self.calls.borrow_mut() += 1;
            }
        }

        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);

        let calls = Rc::new(RefCell::new(0));
        root.as_ui_element()
            .set_invalidate_host(Some(Rc::new(CountingHost {
                calls: Rc::clone(&calls),
            })));

        // Called from the *leaf*, not the root — must walk `parent()` up to find the registered host.
        leaf.invalidate();
        leaf.invalidate_arrange();
        leaf.invalidate_measure();
        assert_eq!(*calls.borrow(), 3);

        root.as_ui_element().set_invalidate_host(None);
        leaf.invalidate();
        assert_eq!(
            *calls.borrow(),
            3,
            "un-registering the host should make invalidate a no-op again"
        );
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
            leaf.as_ui_element().register_routed_handler::<()>(
                "on_click",
                Box::new(move |_, _| *leaf_calls.borrow_mut() += 1),
            );
        }
        {
            let root_calls = Rc::clone(&root_calls);
            root.as_ui_element().register_routed_handler::<()>(
                "on_click",
                Box::new(move |_, args| {
                    *root_calls.borrow_mut() += 1;
                    args.handled.set(true);
                }),
            );
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
        assert_eq!(tree.arranged_width(), Some(0.0));
        assert_eq!(tree.arranged_height(), Some(0.0));
    }

    #[test]
    fn collapsed_child_is_excluded_from_stack_layout() {
        let collapsed = native("collapsed", size(50.0, 50.0));
        collapsed
            .as_ui_element()
            .set_visibility(Visibility::Collapsed);
        let visible = native("visible", size(30.0, 10.0));
        visible
            .as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        visible
            .as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let tree = stack(
            Orientation::Vertical,
            5.0,
            vec![Rc::clone(&collapsed), Rc::clone(&visible)],
        );

        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        // Known limitation (see `Visibility`'s own doc comment / the layout engine's own comment
        // above `measure`): `stack_arrange` still reserves the 5.0 `spacing` gap around the
        // zero-sized collapsed child, so `visible` starts at y = 5.0, not y = 0.0.
        assert_eq!(
            natives,
            vec![(
                FakeHandle("visible", size(30.0, 10.0)),
                Rect {
                    x: 0.0,
                    y: 5.0,
                    width: 30.0,
                    height: 10.0
                }
            )]
        );
    }

    #[test]
    fn collapsed_containers_subtree_is_entirely_excluded() {
        let leaf = native("child", size(10.0, 10.0));
        let container = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        container
            .as_ui_element()
            .set_visibility(Visibility::Collapsed);

        let (natives, paints) = split(layout_tree::<FakeHandle>(&container, size(100.0, 100.0)));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
        assert_eq!(
            leaf.visibility(),
            Visibility::Visible,
            "the child itself was never made Collapsed"
        );
    }

    #[test]
    fn collapsed_element_is_excluded_from_hit_test() {
        let tree = native("a", size(10.0, 20.0));
        tree.as_ui_element().set_visibility(Visibility::Collapsed);
        layout_tree::<FakeHandle>(&tree, size(100.0, 100.0));
        assert!(hit_test(&tree, Point { x: 5.0, y: 5.0 }).is_none());
    }
}
