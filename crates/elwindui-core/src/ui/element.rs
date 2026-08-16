//! The root of the class hierarchy: [`UIElement`] itself, plus the two host-capability traits a
//! backend implements to service invalidation ([`RelayoutHost`]) and focus ([`FocusHost`]) requests
//! that bubble up from anywhere in the tree.
//!
//! Every other class in `crate::ui` ultimately `inherits = crate::ui::UIElement`, so this module is
//! declared first in `mod.rs` — see the ordering note there.

use super::*;

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
/// How much of the layout pipeline a change invalidates. Ordered weakest -> strongest
/// (`#[derive(PartialOrd, Ord)]` follows declaration order) so a host coalescing several
/// `request_relayout` calls within one runloop turn can just keep the `max` of what it's seen —
/// see e.g. `elwindui-backend-appkit`'s `AppKitRelayoutHost`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum InvalidationKind {
    /// Paint only: this element's `RenderGroup` commands must be re-recorded, but its measured
    /// and arranged geometry are still valid, so a host may skip re-running layout entirely
    /// (`layout_root`) and go straight to reconcile + replay. Nothing today produces this kind
    /// except `UIElement::invalidate_render` — see that method's own doc comment on why
    /// `invalidate()`'s many existing call sites are not simply retargeted to it.
    #[default]
    Render,
    /// Where this element sits changed, but not how big it wants to be.
    Arrange,
    /// This element's desired size may have changed.
    Measure,
}

pub trait RelayoutHost {
    fn request_relayout(&self, dirty_group_id: u64, kind: InvalidationKind);
}

/// The `FocusHost` counterpart to `RelayoutHost` — registered the same way (`UIElement::focus_host`
/// on a hosted tree's root, set by the host's own `set_tree`), discovered the same way
/// (`request_focus` walks `visual_parent` up to the root, mirroring `request_relayout`), and backed
/// by the same "wrap a weak handle back to the host" convention. `UIElementExt::focus()` is the
/// public entry point every element gets for free — see docs/design/runtime/input_focus_design.md.
pub trait FocusHost {
    /// Always requests `FocusState::Programmatic` — matching real WinUI3, where `Control.Focus()`'s
    /// public entry point only ever sets `Programmatic`; `Keyboard`/`Pointer` are set exclusively by
    /// the framework's own input handling (`KeyboardDispatcher`/a future click-to-focus wiring).
    /// Returns `false` if `target` isn't a tab stop (`UIElementExt::is_tab_stop`).
    fn request_focus(&self, target: &Rc<dyn UIElementExt>) -> bool;
}

/// The fields every `UIElement` carries (WinUI3's `FrameworkElement` base class, via composition
/// since Rust has no class inheritance — each concrete type embeds one of these and delegates
/// `UIElement::base`).
///
/// Every field here is interior-mutable (`Cell`/`RefCell`, matching `routed_handlers`/`parent`,
/// which already were) — every `create_xxx(...)` factory in this crate (and every hand-written
/// backend's `create_button`/etc.) builds its own `UIElement::construct()` internally, taking no
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
/// `UIElement` is the root of the class hierarchy (docs/design/runtime/ui_tree_design.md) —
/// `#[elwindui_macros::class]`'s "root class mode" (no `inherits`): every method on the paired
/// `impl UIElement { .. }` below becomes a *default* method here, embedded body and all, so every
/// other `#[class(inherits = ..)]`-managed subclass inherits all of them for free via Rust's own
/// default-method dispatch — only `base` (synthesized by the macro; its concrete location differs
/// per implementor) is a genuinely required method.
///
/// The `#[dsl_prop(..)]` lines are this class's DSL-visible surface — the properties every element,
/// builtin or user-defined, picks up for free. They terminate the `__elwindui_shape_*!` forwarding
/// chain: a property no descendant declares ends up here, and anything this class doesn't declare
/// either becomes a `compile_error!` naming it (see `build_shape_macro`).
#[elwindui_macros::class(abstract_class)]
#[prop(margin: Option<f32>)]
#[prop(horizontal_alignment: Option<crate::layout::HorizontalAlignment>)]
#[prop(vertical_alignment: Option<crate::layout::VerticalAlignment>)]
#[prop(visibility: Option<crate::layout::Visibility>)]
#[prop(width: Option<f32>)]
#[prop(height: Option<f32>)]
#[prop(min_width: Option<f32>)]
#[prop(min_height: Option<f32>)]
#[prop(max_width: Option<f32>)]
#[prop(max_height: Option<f32>)]
#[prop(hit_test_visible: Option<bool>)]
#[prop(tab_stop: Option<bool>)]
#[prop(focus_order: Option<i32>)]
#[prop(routed, on_key_down: fn(crate::input::KeyEventArgs))]
#[prop(routed, on_key_up: fn(crate::input::KeyEventArgs))]
#[prop(routed, on_text_input: fn(crate::input::TextInputEventArgs))]
#[prop(routed, on_got_focus: fn())]
#[prop(routed, on_lost_focus: fn())]
#[prop(routed, on_pointer_pressed: fn(crate::input::PointerEventArgs))]
#[prop(routed, on_pointer_released: fn(crate::input::PointerEventArgs))]
#[prop(routed, on_pointer_moved: fn(crate::input::PointerEventArgs))]
#[prop(routed, on_pointer_entered: fn(crate::input::PointerEventArgs))]
#[prop(routed, on_pointer_exited: fn(crate::input::PointerEventArgs))]
#[prop(routed, on_pointer_wheel_changed: fn(crate::input::PointerWheelEventArgs))]
#[prop(routed, on_tapped: fn(crate::input::TappedEventArgs))]
#[prop(routed, on_double_tapped: fn(crate::input::TappedEventArgs))]
#[prop(routed, on_right_tapped: fn(crate::input::TappedEventArgs))]
#[prop(context_menu: Option<Rc<dyn crate::ui::MenuExt>>)]
#[prop(context_menu_presentation: crate::ui::ContextMenuPresentation)]
#[prop(context_popup: Option<crate::ui::PopupContentTemplate>)]
pub struct UIElement {
    /// Stable identity of this Visual's retained RenderGroup. Never reused within a process.
    pub render_group_id: u64,
    pub margin: Cell<f32>,
    pub horizontal_alignment: Cell<HorizontalAlignment>,
    pub vertical_alignment: Cell<VerticalAlignment>,
    /// WinUI3's `UIElement.Visibility` — `Visible` (default) or `Collapsed`. See `Visibility`'s own
    /// doc comment for how `Collapsed` is handled by the layout/render/hit-test traversals.
    pub visibility: Cell<Visibility>,
    /// WinUI3's `UIElement.IsHitTestVisible` — `true` (default) means normal hit-testing;
    /// `false` excludes this element *and its entire subtree* from `hit_test` while leaving
    /// rendering/layout untouched (unlike `Visibility::Collapsed`, which affects layout too). See
    /// `hit_test_at`'s own doc comment.
    pub hit_test_visible: Cell<bool>,
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
    pub context_menu: RefCell<Option<Rc<dyn MenuExt>>>,
    pub context_menu_presentation: Cell<ContextMenuPresentation>,
    pub context_popup: RefCell<Option<PopupContentTemplate>>,
    pub environment: RefCell<Option<crate::environment::EnvironmentContext>>,
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
    /// `docs/specs/dsl_spec.md` §12), keyed by field name. Each value is a
    /// `Box<dyn Fn(&T, &RoutedEventArgs)>` erased to `Box<dyn Any>` (`T` is that field's own
    /// payload type — `()` for `on_click`, `usize` for a hypothetical routed `on_select`, ...);
    /// generated call sites know `T` statically from the DSL declaration, so the downcast in
    /// `dispatch_routed` always succeeds (matching generated dynamic child ranges' type-erasure
    /// pattern).
    pub routed_handlers: RoutedHandlers,
    /// Generic, type-erased attached-property bag (docs/specs/dsl_spec.md §3の添付プロパティ), keyed
    /// by `(owner, field)` — e.g. `("Grid", "row")` — and populated right after construction from
    /// whatever `Owner::field: value` setters the DSL source wrote on this specific element
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
    /// WinUI3's `Control.IsTabStop` — whether this element participates in `FocusTracker`'s tab
    /// order at all. `false` by default; a `NativeControl<H>`-backed leaf (`Button`/`TextArea`/
    /// `TabView`) sets this `true` in its own `new()` (mirrors `Button::new()`'s `on_click` wiring),
    /// and `#[focus(order: ..)]` forces it `true` on whatever field it's declared on
    /// (`elwindui-codegen`'s `emit_wiring`). See `UIElementExt::is_tab_stop`.
    pub tab_stop: Cell<bool>,
    /// WinUI3's `Control.TabIndex` — `None` (default) falls back to tree/declaration order, the
    /// same way an unset `TabIndex` does in real WinUI3. See `UIElementExt::focus_order`.
    pub focus_order: Cell<Option<i32>>,
    /// WinUI3's `Control.FocusState` — written only by `FocusTracker::set_focus`/`clear_focus`, read
    /// via `UIElementExt::focus_state`.
    pub focus_state: Cell<FocusState>,
    /// The `FocusHost` counterpart to `invalidate_host` — see that field's own doc comment and
    /// `FocusHost`'s own.
    pub focus_host: RefCell<Option<Rc<dyn FocusHost>>>,
    /// `#[shortcut(...)]`-annotated fields declared on this element, registered here by
    /// `elwindui-codegen`'s generated `new()` — not yet reachable from any `ShortcutRegistry` (this
    /// element doesn't know which tree/window it'll end up hosted under yet). A host's own
    /// `set_tree` walks the whole freshly-set tree once and feeds every element's own
    /// `declared_shortcuts` into its `ShortcutRegistry` — see `crate::input::ShortcutDecl`'s own doc
    /// comment.
    pub declared_shortcuts: RefCell<Vec<crate::input::ShortcutDecl>>,
}

impl std::fmt::Debug for UIElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UIElement")
            .field("render_group_id", &self.render_group_id)
            .field("margin", &self.margin.get())
            .field("horizontal_alignment", &self.horizontal_alignment.get())
            .field("vertical_alignment", &self.vertical_alignment.get())
            .field("visibility", &self.visibility.get())
            .field("hit_test_visible", &self.hit_test_visible.get())
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
            .field("tab_stop", &self.tab_stop.get())
            .field("focus_order", &self.focus_order.get())
            .field("focus_state", &self.focus_state.get())
            .field("focus_host", &self.focus_host.borrow().is_some())
            .finish()
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
        UIElement {
            render_group_id: NEXT_RENDER_GROUP_ID.fetch_add(1, Ordering::Relaxed),
            margin: Cell::new(0.0),
            horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            vertical_alignment: Cell::new(VerticalAlignment::Stretch),
            visibility: Cell::new(Visibility::Visible),
            hit_test_visible: Cell::new(true),
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
            visual_collection: UIElementVisualCollection::new(__self_weak.clone()),
            invalidate_host: RefCell::new(None),
            tab_stop: Cell::new(false),
            focus_order: Cell::new(None),
            focus_state: Cell::new(FocusState::Unfocused),
            focus_host: RefCell::new(None),
            declared_shortcuts: RefCell::new(Vec::new()),
            context_menu: RefCell::new(None),
            context_menu_presentation: Cell::new(ContextMenuPresentation::Native),
            context_popup: RefCell::new(None),
            environment: RefCell::new(None),
        }
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
    /// The single source of truth for whether this element takes part in measure/arrange/
    /// rendering/hit-testing at all. Every one of `measure`/`arrange`/`build_render_group`/
    /// `reconcile_render_group`/`hit_test_at` must call this rather than re-checking
    /// `visibility()` directly — a child skipped here is a child `build_render_group`/
    /// `reconcile_render_group` never push into `RenderGroup.children` at all, so a second,
    /// independently-written condition anywhere else in that walk would silently desync
    /// `RenderTree::group_paths`' child indices from `RenderGroup`'s own dense `children`.
    ///
    /// Currently equivalent to `visibility() == Visibility::Visible`, but kept as its own method
    /// (rather than inlining that comparison at each call site) so a future container-level
    /// participation signal — e.g. a hosted tree being temporarily deactivated by its own
    /// `TreeHostView`/`TreeHostPanel` (docs/design/runtime/ui_tree_design.md) — has exactly one
    /// place to fold in, without hunting down every call site again. As of this writing no such
    /// second signal exists in this crate: TabView/ScrollView content lives in its own separately
    /// hosted tree rather than as `visual_children()` of the tab strip, so a hosted tree simply
    /// isn't laid out at all while its host is inactive, and nothing here needs to model that.
    fn participates_in_layout(&self) -> bool {
        self.visibility() == Visibility::Visible
    }
    /// WinUI3's `UIElement.IsHitTestVisible` — see `UIElement::hit_test_visible`'s own doc comment.
    fn hit_test_visible(&self) -> bool {
        self.as_ui_element().hit_test_visible.get()
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
    /// Post-construction setters (docs/design/runtime/ui_tree_design.md) for every field this trait
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
    /// See `UIElement::hit_test_visible`'s own doc comment. Hit-testing only — no layout/render
    /// effect, so unlike most other setters here this doesn't invalidate anything.
    fn set_hit_test_visible(&self, hit_test_visible: bool) {
        self.as_ui_element().hit_test_visible.set(hit_test_visible);
    }
    fn set_clip_to_bounds(&self, value: Option<bool>) {
        self.as_ui_element().clip_to_bounds.set(value);
        self.invalidate_arrange();
    }
    // `Option<f32>`-typed at the `#[class]` declaration (an unset value means "let
    // natural sizing decide"), but taking the plain, unwrapped `f32` here — matching every other
    // deferred `Option<T>`-declared common property's own setter (`set_margin(&self, margin: f32)`
    // above, `set_enabled(&self, enabled: bool)` on `Button`/`MenuItem`, ...): "unset" is expressed
    // purely by never calling the setter at all (the constructed default, `None`, stays in place),
    // never by an explicit `None` argument — no DSL syntax spells that anyway. Keeping every
    // deferred common property on this one shared convention lets `elwindui-codegen`'s generic,
    // field-name-agnostic setter emission (`build_component_args`/`build_component_setters`/
    // `build_component_optional_setters`) apply to all of them uniformly, with no per-field
    // Option-wrapping decision needed anywhere in codegen.
    fn set_width(&self, width: f32) {
        self.as_ui_element().width.set(Some(width));
        self.invalidate_measure();
    }
    fn set_height(&self, height: f32) {
        self.as_ui_element().height.set(Some(height));
        self.invalidate_measure();
    }
    fn set_min_width(&self, min_width: f32) {
        self.as_ui_element().min_width.set(Some(min_width));
        self.invalidate_measure();
    }
    fn set_min_height(&self, min_height: f32) {
        self.as_ui_element().min_height.set(Some(min_height));
        self.invalidate_measure();
    }
    fn set_max_width(&self, max_width: f32) {
        self.as_ui_element().max_width.set(Some(max_width));
        self.invalidate_measure();
    }
    fn set_max_height(&self, max_height: f32) {
        self.as_ui_element().max_height.set(Some(max_height));
        self.invalidate_measure();
    }
    fn context_menu(&self) -> Option<Rc<dyn MenuExt>> {
        self.as_ui_element().context_menu.borrow().clone()
    }
    fn set_context_menu(&self, menu: Option<Rc<dyn MenuExt>>) {
        *self.as_ui_element().context_menu.borrow_mut() = menu;
    }
    fn context_menu_presentation(&self) -> ContextMenuPresentation {
        self.as_ui_element().context_menu_presentation.get()
    }
    fn set_context_menu_presentation(&self, presentation: ContextMenuPresentation) {
        self.as_ui_element()
            .context_menu_presentation
            .set(presentation);
    }
    fn context_popup(&self) -> Option<PopupContentTemplate> {
        self.as_ui_element().context_popup.borrow().clone()
    }
    fn set_context_popup(&self, popup: Option<PopupContentTemplate>) {
        *self.as_ui_element().context_popup.borrow_mut() = popup;
    }
    fn environment_context(&self) -> Option<crate::environment::EnvironmentContext> {
        self.as_ui_element().environment.borrow().clone()
    }
    fn set_environment_context(&self, env: crate::environment::EnvironmentContext) {
        *self.as_ui_element().environment.borrow_mut() = Some(env);
    }
    fn effective_environment(&self) -> crate::environment::EnvironmentContext {
        if let Some(env) = self.as_ui_element().environment.borrow().as_ref() {
            return env.clone();
        }
        if let Some(parent) = self.visual_parent() {
            return parent.effective_environment();
        }
        if let Some(logical_parent) = self.parent() {
            return logical_parent.effective_environment();
        }
        crate::environment::EnvironmentContext::root()
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
    /// docs/design/runtime/ui_tree_design.md) — the only tree any code ever actually walks (there is no
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
    /// WinUI3/WPF's "an element with no `Background`/`Fill` isn't itself hit-testable" rule
    /// (`hit_test_at`'s own doc comment) — whether *this element's own bounds* (not its children's)
    /// should be considered a hit-test candidate. Defaults to `true` (every leaf-like element —
    /// `NativeControl`/`TextBlock` — represents real content). `Layout`/`Control` (no background
    /// concept at all) override this to `false`; `Shape` overrides it to
    /// whether `fill`/`stroke` is actually set. This is independent of `hit_test_visible`, which
    /// excludes the whole subtree unconditionally.
    #[overridable]
    fn hit_test_content(&self) -> bool {
        true
    }
    /// `Some(&self.handle)` (the raw native handle itself, erased to `&dyn Any`) for a backend's own
    /// `NativeControlImpl { handle: AnyView, .. }` and for any type that composes one as its own
    /// `base` field (docs/design/runtime/ui_tree_design.md — e.g. a backend's `ButtonImpl { base:
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
    ///
    /// Deliberately still an alias for `invalidate_arrange` rather than `invalidate_render` —
    /// `invalidate()` has many existing call sites across this crate and both backends that have
    /// never been individually audited for whether the change they guard could affect
    /// `measure_override`/`arrange_override` (font/text/size/margin/visibility changes must stay
    /// `Arrange` or `Measure`; only a provably paint-only change is safe to migrate to
    /// `invalidate_render`). See `InvalidationKind::Render`'s own doc comment.
    fn invalidate(&self) {
        self.invalidate_arrange();
    }
    /// Paint-only invalidation: this element's `RenderGroup` commands must be re-recorded, but
    /// nothing about its measured or arranged geometry is in question, so a host may skip
    /// `layout_root` entirely for this pass. See `InvalidationKind::Render`'s own doc comment on
    /// why callers must self-audit before using this instead of `invalidate()`.
    fn invalidate_render(&self) {
        request_relayout(self.as_ui_element(), InvalidationKind::Render);
    }
    /// WinUI3's `UIElement.InvalidateArrange` — marks this element's `arranged_width`/
    /// `arranged_height`/`arranged_offset` `None` (to be recomputed by the next `arrange` pass) and
    /// asks for a redraw. `measured_size` stays valid — only where this element ends up, not how
    /// big it wants to be, is in question (e.g. `UIElement::set_horizontal_alignment`).
    fn invalidate_arrange(&self) {
        self.as_ui_element().arranged_width.set(None);
        self.as_ui_element().arranged_height.set(None);
        self.as_ui_element().arranged_offset.set(None);
        request_relayout(self.as_ui_element(), InvalidationKind::Arrange);
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
        request_relayout(self.as_ui_element(), InvalidationKind::Measure);
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
    /// WinUI3's `Control.IsTabStop` — see `UIElement::tab_stop`'s own doc comment.
    fn is_tab_stop(&self) -> bool {
        self.as_ui_element().tab_stop.get()
    }
    fn set_tab_stop(&self, value: bool) {
        self.as_ui_element().tab_stop.set(value);
    }
    /// WinUI3's `Control.TabIndex` — see `UIElement::focus_order`'s own doc comment.
    fn focus_order(&self) -> Option<i32> {
        self.as_ui_element().focus_order.get()
    }
    fn set_focus_order(&self, value: Option<i32>) {
        self.as_ui_element().focus_order.set(value);
    }
    /// WinUI3's `Control.FocusState` — see `UIElement::focus_state`'s own doc comment. Written only
    /// by `crate::focus::FocusTracker::set_focus`/`clear_focus`, never directly.
    fn focus_state(&self) -> FocusState {
        self.as_ui_element().focus_state.get()
    }
    /// `pub(crate)`-in-spirit (public for `crate::focus::FocusTracker`'s sake, which lives in a
    /// sibling module of this same crate — there is no narrower visibility that still reaches it):
    /// not meant to be called from outside `FocusTracker`. See `UIElement::focus_state`'s own doc
    /// comment.
    fn set_focus_state(&self, value: FocusState) {
        self.as_ui_element().focus_state.set(value);
    }
    /// Called by whatever backend host is about to own this element as the root of a hosted tree —
    /// the `FocusHost` counterpart to `set_invalidate_host`, set at the same time by the same
    /// caller. `None` un-registers.
    fn set_focus_host(&self, host: Option<Rc<dyn FocusHost>>) {
        *self.as_ui_element().focus_host.borrow_mut() = host;
    }
    /// Registers a `#[shortcut(...)]`-annotated field's binding on this element — see
    /// `UIElement::declared_shortcuts`'s own doc comment.
    fn declare_shortcut(&self, decl: crate::input::ShortcutDecl) {
        self.as_ui_element()
            .declared_shortcuts
            .borrow_mut()
            .push(decl);
    }
    /// Every `#[shortcut(...)]` this element has declared — see `UIElement::declared_shortcuts`'s
    /// own doc comment. A host's own `set_tree` calls this on every node while walking a freshly-set
    /// tree, feeding each result into its `ShortcutRegistry`.
    fn declared_shortcuts(&self) -> Vec<crate::input::ShortcutDecl> {
        self.as_ui_element().declared_shortcuts.borrow().clone()
    }
    /// WinUI3's `Control.Focus()` — forces this element to become its hosted tree's focused
    /// element, always with `FocusState::Programmatic` (see `FocusHost::request_focus`'s own doc
    /// comment). Walks up the Visual-parent chain (mirroring `request_relayout`) looking for the
    /// `FocusHost` a backend host registered via `set_focus_host`. Returns `false` if this element
    /// isn't a tab stop, isn't part of a hosted tree, or the containing tree has no host wired up
    /// (e.g. a standalone test tree).
    fn focus(&self) -> bool {
        let target = self.as_ui_element().visual_collection.owner_rc();
        match target {
            Some(target) => request_focus(&target),
            None => false,
        }
    }
    /// WinUI3's `UIElement.Measure(Size availableSize)` — computes this element's own desired size
    /// (margin-inclusive) against `available`, recursing into children as `measure_override` (still
    /// freely overridable, unlike this method) needs them, and caches the result in
    /// `measured_size()`. `void` like WinUI3's own `Measure` — callers read the result back via
    /// `measured_size()` rather than this method's return value (there isn't one). Always
    /// recomputes when called, regardless of whether `measured_size()` was already `Some` — see
    /// `UIElement::measured_size`'s own doc comment for why this isn't a memoizing cache.
    fn measure(&self, available: Size) {
        let result = if !self.participates_in_layout() {
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
        if !self.participates_in_layout() {
            self.as_ui_element().arranged_width.set(Some(0.0));
            self.as_ui_element().arranged_height.set(Some(0.0));
            // `arranged_offset` is set too (unlike width/height, which the non-participating
            // case above always resets, this used to leave a stale offset from the last time
            // this element *did* arrange) — a reader that only checks `arranged_offset().is_some()`
            // to decide "has this element ever been positioned" would otherwise see a leftover
            // real position for an element that currently contributes nothing to the tree.
            self.as_ui_element()
                .arranged_offset
                .set(Some(Point { x: 0.0, y: 0.0 }));
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
        let mut slot = shrink_rect_by_margin(final_rect, self.margin());
        let desired_without_margin = shrink_by_margin(desired_with_margin, self.margin());
        // WinUI3/WPF: an explicit `Width`/`Height` wins over `Stretch` — `Stretch` only fills the
        // slot when that axis was never set at all (`align_within`'s own "fills the slot" rule).
        // Shrinking the slot itself to the explicit size here (rather than teaching `align_within`
        // about "explicit-ness") keeps that function exactly what its own doc comment says it is:
        // pure size-in/rect-out math with no widget knowledge — the same way real WPF's own
        // `FrameworkElement.ArrangeCore` consults `this.Width`/`this.Height` directly, right where
        // `this` is available, rather than threading an "is explicit" flag into a separate helper.
        if self.width().is_some() {
            slot.width = slot.width.min(desired_without_margin.width);
        }
        if self.height().is_some() {
            slot.height = slot.height.min(desired_without_margin.height);
        }
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
    /// The parent property-inheritance (font, and any future inherited property) should walk from
    /// this element — which tree to follow is the *caller's* choice (指示書 §14), not a fixed
    /// per-element policy: font resolution always passes `Visual` (指示書 §13), while some future
    /// consumer wanting `DataContext`-style inheritance would pass `Logical`. `Logical` falls back to
    /// `Visual` when there's no logical parent, matching WinUI3's own
    /// `GetInheritanceParentInternal()` fallback behavior (指示書 §14).
    ///
    /// Overridable so a Popup/Portal/ControlTemplate-style element whose *visual* parent doesn't
    /// reach its real inheritance source (指示書 §28) can substitute its own answer — no such
    /// override exists yet (未対応, see `docs/design/runtime/text_design.md`), but the hook is here.
    #[overridable]
    fn inheritance_parent(&self, kind: InheritanceParentKind) -> Option<Rc<dyn UIElementExt>> {
        match kind {
            InheritanceParentKind::Visual => self.visual_parent(),
            InheritanceParentKind::Logical => self.parent().or_else(|| self.visual_parent()),
        }
    }
    /// Downcast hook for the font-inheritance walk (`inherited_text_style`) — the `TextStyleOwner`
    /// analogue of `try_as_native_control` just above, same shape and same rationale: `None` for
    /// every `UIElement` that doesn't hold a `TextStyleStorage` of its own (`Grid`/`Layout`/`Shape`/
    /// `Image`/... — 指示書 §11 requires these stay transparent to inheritance, not blocking it),
    /// `Some(self)` for `Control`/`TextBlock`/each backend's own `NativeControl`.
    #[overridable]
    fn as_text_style_owner(&self) -> Option<&dyn TextStyleOwner> {
        None
    }
}

/// Shared implementation for `UIElement::invalidate`/`invalidate_arrange`/`invalidate_measure` —
/// walks from `base`'s own element up to the root of whatever tree it's currently part of
/// (`UIElement::parent`, repeated until `None`) and, if that root has a `RelayoutHost` registered
/// (see `UIElement::invalidate_host`), asks it for a fresh layout pass. Takes `&UIElement`
/// (not `&dyn UIElement`) so the caller — a default trait method, where `Self` isn't known to be
/// `Sized`. A no-op if the Visual root has no registered host (e.g. a standalone test tree).
pub(crate) fn request_relayout(base: &UIElement, kind: InvalidationKind) {
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
        host.request_relayout(base.render_group_id, kind);
    }
}

/// Shared implementation for `UIElementExt::focus` — mirrors `request_relayout`'s "walk
/// `visual_parent` to the root, looking for the nearest registered host" shape, but (unlike
/// `request_relayout`, which only ever needs `base.render_group_id`, a plain `u64`) keeps `target`
/// itself as the fixed argument passed to whichever `FocusHost` is found, since `FocusHost::
/// request_focus` needs the real `Rc<dyn UIElementExt>` to hand to `FocusTracker::set_focus`.
pub(crate) fn request_focus(target: &Rc<dyn UIElementExt>) -> bool {
    let mut current = Some(Rc::clone(target));
    let mut host = target.as_ui_element().focus_host.borrow().clone();
    while let Some(element) = current {
        host = element.as_ui_element().focus_host.borrow().clone().or(host);
        current = element.visual_parent();
    }
    match host {
        Some(host) => host.request_focus(target),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::*;

    #[test]
    fn invalidate_family_reaches_a_relayout_host_registered_on_the_root() {
        struct CountingHost {
            calls: Rc<RefCell<usize>>,
        }
        impl RelayoutHost for CountingHost {
            fn request_relayout(&self, _dirty_group_id: u64, _kind: InvalidationKind) {
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
    fn invalidate_arrange_and_measure_send_their_own_kind() {
        struct KindRecordingHost {
            kinds: Rc<RefCell<Vec<InvalidationKind>>>,
        }
        impl RelayoutHost for KindRecordingHost {
            fn request_relayout(&self, _dirty_group_id: u64, kind: InvalidationKind) {
                self.kinds.borrow_mut().push(kind);
            }
        }

        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        let kinds = Rc::new(RefCell::new(Vec::new()));
        root.as_ui_element()
            .set_invalidate_host(Some(Rc::new(KindRecordingHost {
                kinds: Rc::clone(&kinds),
            })));

        leaf.invalidate_render();
        leaf.invalidate_arrange();
        leaf.invalidate_measure();
        leaf.invalidate(); // still an `invalidate_arrange` alias, not `Render` — see its own doc comment

        assert_eq!(
            *kinds.borrow(),
            vec![
                InvalidationKind::Render,
                InvalidationKind::Arrange,
                InvalidationKind::Measure,
                InvalidationKind::Arrange,
            ]
        );
    }

    #[test]
    fn invalidate_render_leaves_measured_and_arranged_state_untouched() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        // No host needed — `invalidate_render` clearing nothing is a property of the element
        // itself, independent of whether a host is registered to receive the request.
        let _ = root;

        leaf.as_ui_element().measured_size.set(Some(Size {
            width: 10.0,
            height: 20.0,
        }));
        leaf.as_ui_element().arranged_width.set(Some(10.0));
        leaf.as_ui_element().arranged_height.set(Some(20.0));
        leaf.as_ui_element()
            .arranged_offset
            .set(Some(Point { x: 0.0, y: 0.0 }));

        leaf.invalidate_render();

        assert!(leaf.as_ui_element().measured_size.get().is_some());
        assert!(leaf.as_ui_element().arranged_width.get().is_some());
        assert!(leaf.as_ui_element().arranged_height.get().is_some());
        assert!(leaf.as_ui_element().arranged_offset.get().is_some());
    }

    #[test]
    fn invalidation_kind_orders_render_weakest_and_measure_strongest() {
        assert!(InvalidationKind::Render < InvalidationKind::Arrange);
        assert!(InvalidationKind::Arrange < InvalidationKind::Measure);
        assert_eq!(
            InvalidationKind::Render.max(InvalidationKind::Measure),
            InvalidationKind::Measure
        );
    }
}
