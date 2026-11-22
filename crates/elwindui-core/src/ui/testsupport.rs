//! Shared `#[cfg(test)]` fixtures for `crate::ui`'s own unit tests.
//!
//! Lives in one place rather than beside any single class because these are used across the
//! engine, collection, text-style, and per-control test modules alike: the `Fake*` widgets stand in
//! for a backend's real native leaves (there is no backend to link against in a core unit test),
//! the `Overridable*` chain exercises `#[class]`'s own inherit/override dispatch, and the rest are
//! tree-building and event-construction conveniences.
//!
//! Declared last in `mod.rs`: several of these are themselves `#[class]` declarations that inherit
//! real classes, so they must expand after every one of those is registered.

use super::*;

pub(crate) fn layout_tree<H: Clone + 'static>(
    root: &Rc<dyn UIElementExt>,
    available: Size,
) -> RenderTree {
    layout_root(root, available);
    RenderTree::new::<H>(root)
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) struct FakeHandle(pub(crate) &'static str, pub(crate) Size);

impl FakeHandle {
    pub(crate) fn measure(&self, _available: Size) -> Size {
        self.1
    }
}

/// A minimal native leaf for Core unit tests, which have no backend peer factory: it inherits
/// `NativeControl` but overrides `measure_override`/`try_as_native_control` itself, so its fixed
/// handle and size stand in for a realized peer and no peer is ever created. Named
/// `FakeNativeControl`, not the bare `NativeControl` that the base already uses, because `#[class]`-generated `__elwindui_inherit_*!` macros share a
/// single flat, crate-wide namespace (unlike ordinary Rust items, which can share a bare name
/// across different modules) — a same-crate bare-name collision is a real `E0428`.
#[elwindui_macros::class(inherits = crate::ui::NativeControl)]
pub(crate) struct FakeNativeControl {
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
    fn construct(handle: FakeHandle) -> Self {
        Self {
            base: NativeControl::construct(None),
            handle,
        }
    }
}

/// `#[overridable]`/`#[overrides]` usage example, exercised across a genuine 3-hop chain
/// (`OverridableBase` -> `OverridableMid` -> `OverridableLeaf`) with two overridable methods —
/// `OverridableMid` overrides only `label`, leaving `compute` untouched, and `OverridableLeaf`
/// (which itself overrides neither) relies on defaults for both. This exercises resolution of
/// overridable methods across the chain: one dedicated accessor per `#[overridable]` method is
/// resolved independently, ensuring that overrides at intermediate hops are dispatched correctly
/// while untouched methods pass through (see `per_method_accessor_ident`'s own doc comment for details).
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
pub(crate) struct OverridableBase {
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
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            value: Cell::new(1),
        }
    }
}

/// hop-1: overrides only `label`, leaves `compute` untouched at `OverridableBase`'s own
/// default — the partial-override case.
#[elwindui_macros::class(inherits = crate::ui::testsupport::OverridableBase)]
pub(crate) struct OverridableMid {}

#[elwindui_macros::class]
impl OverridableMid {
    #[overrides]
    fn label(&self) -> &'static str {
        "mid"
    }
    fn construct() -> Self {
        Self {
            base: OverridableBase::construct(),
        }
    }
}

/// hop-2: overrides neither method itself — both must resolve via defaults, dispatching
/// through `OverridableMid`'s per-method accessors: `label` should stop at `OverridableMid`'s
/// own override, `compute` should pass through it to reach `OverridableBase`'s original.
#[elwindui_macros::class(inherits = crate::ui::testsupport::OverridableMid)]
pub(crate) struct OverridableLeaf {}

#[elwindui_macros::class]
impl OverridableLeaf {
    fn construct() -> Self {
        Self {
            base: OverridableMid::construct(),
        }
    }
}

/// Issue #128 regression fixtures: `#[overridable]`/`#[overrides]` propagation across a deeper
/// ordinary chain than `OverridableBase`/`Mid`/`Leaf` above. `ChainBase` declares the
/// overridable `value`; `ChainNoOverride`/`ChainDerived`/`ChainDepth*` are its descendants.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
pub(crate) struct ChainBase {
    value: Cell<i32>,
}

#[elwindui_macros::class]
impl ChainBase {
    #[overridable]
    fn value(&self) -> i32 {
        self.value.get()
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            value: Cell::new(1),
        }
    }
}

/// 7.1: no override anywhere below `ChainBase` — dispatch must reach its own implementation.
#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainBase)]
pub(crate) struct ChainNoOverride {}

#[elwindui_macros::class]
impl ChainNoOverride {
    fn construct() -> Self {
        Self {
            base: ChainBase::construct(),
        }
    }
}

/// 7.2/9.1: a single descendant using ordinary `#[overrides]` + `self.base.value()` forwarding.
#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainBase)]
pub(crate) struct ChainDerived {}

#[elwindui_macros::class]
impl ChainDerived {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 100
    }
    fn construct() -> Self {
        Self {
            base: ChainBase::construct(),
        }
    }
}

/// 7.3/7.4/9.2: arbitrary descendant depth —
/// `ChainBase -> ChainDepthC (overrides) -> ChainDepthD (no override) -> ChainDepthE
/// (overrides)`. No depth-specific code exists anywhere in the fix; this fixture is what proves it.
#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainBase)]
pub(crate) struct ChainDepthC {}

#[elwindui_macros::class]
impl ChainDepthC {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 10
    }
    fn construct() -> Self {
        Self {
            base: ChainBase::construct(),
        }
    }
}

#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainDepthC)]
pub(crate) struct ChainDepthD {}

#[elwindui_macros::class]
impl ChainDepthD {
    fn construct() -> Self {
        Self {
            base: ChainDepthC::construct(),
        }
    }
}

#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainDepthD)]
pub(crate) struct ChainDepthE {}

#[elwindui_macros::class]
impl ChainDepthE {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 1000
    }
    fn construct() -> Self {
        Self {
            base: ChainDepthD::construct(),
        }
    }
}

/// 7.5/9.3/9.4: two independent `#[overridable]` methods on the same class, overridden at
/// different descendant depths — `ChainMultiBase -> ChainMultiC (overrides first) ->
/// ChainMultiD (overrides second) -> ChainMultiE (overrides first)`. Each override logs its own
/// name before forwarding via `self.base.*(log)`, letting a test assert both the exact per-method
/// dispatch target and the forwarding order.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
pub(crate) struct ChainMultiBase {}

#[elwindui_macros::class]
impl ChainMultiBase {
    #[overridable]
    fn first(&self, log: &RefCell<Vec<&'static str>>) -> i32 {
        log.borrow_mut().push("B");
        1
    }
    #[overridable]
    fn second(&self, log: &RefCell<Vec<&'static str>>) -> i32 {
        log.borrow_mut().push("B");
        2
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
        }
    }
}

#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainMultiBase)]
pub(crate) struct ChainMultiC {}

#[elwindui_macros::class]
impl ChainMultiC {
    #[overrides]
    fn first(&self, log: &RefCell<Vec<&'static str>>) -> i32 {
        log.borrow_mut().push("C");
        self.base.first(log) + 10
    }
    fn construct() -> Self {
        Self {
            base: ChainMultiBase::construct(),
        }
    }
}

#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainMultiC)]
pub(crate) struct ChainMultiD {}

#[elwindui_macros::class]
impl ChainMultiD {
    #[overrides]
    fn second(&self, log: &RefCell<Vec<&'static str>>) -> i32 {
        log.borrow_mut().push("D");
        self.base.second(log) + 20
    }
    fn construct() -> Self {
        Self {
            base: ChainMultiC::construct(),
        }
    }
}

#[elwindui_macros::class(inherits = crate::ui::testsupport::ChainMultiD)]
pub(crate) struct ChainMultiE {}

#[elwindui_macros::class]
impl ChainMultiE {
    #[overrides]
    fn first(&self, log: &RefCell<Vec<&'static str>>) -> i32 {
        log.borrow_mut().push("E");
        self.base.first(log) + 100
    }
    fn construct() -> Self {
        Self {
            base: ChainMultiD::construct(),
        }
    }
}

pub(crate) fn size(width: f32, height: f32) -> Size {
    Size { width, height }
}

pub(crate) fn native(name: &'static str, size: Size) -> Rc<dyn UIElementExt> {
    FakeNativeControl::new(FakeHandle(name, size))
}

pub(crate) fn stack(
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

pub(crate) fn split(tree: RenderTree) -> (Vec<(FakeHandle, Rect)>, Vec<(RenderCommand, Rect)>) {
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
                RenderCommand::FillRect { rect, .. }
                | RenderCommand::StrokeRect { rect, .. }
                | RenderCommand::FillRoundedRect { rect, .. }
                | RenderCommand::StrokeRoundedRect { rect, .. }
                | RenderCommand::FillEllipse { rect, .. }
                | RenderCommand::StrokeEllipse { rect, .. }
                | RenderCommand::Text { rect, .. } => paints.push((
                    command.clone(),
                    Rect {
                        x: origin.x + rect.x,
                        y: origin.y + rect.y,
                        width: rect.width,
                        height: rect.height,
                    },
                )),
                RenderCommand::DrawImage { dest, .. }
                | RenderCommand::DrawVectorImage { dest, .. } => paints.push((
                    command.clone(),
                    Rect {
                        x: origin.x + dest.x,
                        y: origin.y + dest.y,
                        width: dest.width,
                        height: dest.height,
                    },
                )),
                RenderCommand::DrawLine { .. }
                | RenderCommand::FillPath { .. }
                | RenderCommand::StrokePath { .. } => paints.push((
                    command.clone(),
                    Rect {
                        x: origin.x,
                        y: origin.y,
                        width: 0.0,
                        height: 0.0,
                    },
                )),
                RenderCommand::PushClip { .. }
                | RenderCommand::PopClip
                | RenderCommand::PushTransform { .. }
                | RenderCommand::PopTransform
                | RenderCommand::PushOpacity { .. }
                | RenderCommand::PopOpacity => {}
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

/// Records every `available` it's actually measured with (in call order), so tests below can
/// assert on it directly instead of inferring it indirectly through a returned size — including
/// `Grid`'s two-pass measurement, where the same child is measured twice and both calls matter.
/// Deliberately not built on `FakeHandle`/`FakeNativeControl` — `FakeHandle` derives `PartialEq`
/// and is compared structurally by several other tests, so adding a recording field there would
/// need every one of those to account for it too.
#[elwindui_macros::class(inherits = crate::ui::NativeControl)]
pub(crate) struct MeasureProbe {
    pub(crate) calls: RefCell<Vec<Size>>,
    reported: Size,
}

#[elwindui_macros::class]
impl MeasureProbe {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        self.calls.borrow_mut().push(available);
        self.reported
    }
    fn construct(reported: Size) -> Self {
        Self {
            base: NativeControl::construct(None),
            calls: RefCell::new(Vec::new()),
            reported,
        }
    }
}

impl MeasureProbe {
    pub(crate) fn last_available(&self) -> Size {
        *self
            .calls
            .borrow()
            .last()
            .expect("measure_override was never called")
    }
}

/// A wrapping-text stand-in: its desired height grows with the number of `line_height` lines a
/// `natural_width` run needs at the available width, so tests can observe whether a Grid measured
/// it at its resolved column width or unconstrained.
#[elwindui_macros::class(inherits = crate::ui::NativeControl)]
pub(crate) struct WrapProbe {
    natural_width: f32,
    line_height: f32,
}

#[elwindui_macros::class]
impl WrapProbe {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        if !available.width.is_finite() || available.width >= self.natural_width {
            return Size {
                width: self.natural_width,
                height: self.line_height,
            };
        }
        let lines = (self.natural_width / available.width.max(1.0)).ceil();
        Size {
            width: available.width,
            height: lines * self.line_height,
        }
    }
    fn construct(natural_width: f32, line_height: f32) -> Self {
        Self {
            base: NativeControl::construct(None),
            natural_width,
            line_height,
        }
    }
}

/// A minimal test-only fixture that both paints itself *and* has children — no real builtin
/// combines the two today (`Shape` is a childless leaf; `Layout`/`Control`/`Grid`
/// never paint), so `render_item_ordering_preserves_traversal_order_across_native_and_paint`
/// (below) needs its own local type to exercise the paint-then-child traversal order.
pub(crate) struct PaintingContainer {
    pub(crate) base: UIElement,
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
    fn __dyn_x_for___accessibility_template_children(&self) -> &dyn UIElementExt {
        self.base.__dyn_x_for___accessibility_template_children()
    }
    fn __dyn_x_for_apply_template(&self) -> &dyn UIElementExt {
        self.base.__dyn_x_for_apply_template()
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
    fn __dyn_x_for_accessibility_intrinsic_semantics(&self) -> &dyn UIElementExt {
        self.base.__dyn_x_for_accessibility_intrinsic_semantics()
    }
    fn __dyn_x_for_perform_accessibility_action(&self) -> &dyn UIElementExt {
        self.base.__dyn_x_for_perform_accessibility_action()
    }
    fn __dyn_x_for_hit_test_content(&self) -> &dyn UIElementExt {
        self.base.__dyn_x_for_hit_test_content()
    }
    // Neither overridden here, so both forward to `self.base` — same reasoning as
    // `__dyn_ui_element` above.
    fn __dyn_x_for_inheritance_parent(&self) -> &dyn UIElementExt {
        self.base.__dyn_x_for_inheritance_parent()
    }
    fn __dyn_x_for_as_text_style_owner(&self) -> &dyn UIElementExt {
        self.base.__dyn_x_for_as_text_style_owner()
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
        context.fill_rounded_rect(
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.arranged_width().unwrap_or(0.0),
                height: self.arranged_height().unwrap_or(0.0),
            },
            CornerRadius::uniform(4.0),
            &Brush::Solid(Color::black()),
        );
    }
}

#[derive(Default)]
pub(crate) struct RecordingRelayoutHost {
    requests: RefCell<Vec<u64>>,
}
impl RelayoutHost for RecordingRelayoutHost {
    fn request_relayout(&self, dirty_group_id: u64, _kind: InvalidationKind) {
        self.requests.borrow_mut().push(dirty_group_id);
    }
}

pub(crate) fn rectangle(fill: Option<&str>, stroke: Option<&str>) -> Rc<dyn UIElementExt> {
    let to_brush = |hex: &str| Brush::Solid(Color::parse_hex(hex).unwrap());
    let rect = Rectangle::new();
    rect.set_fill(fill.map(to_brush));
    rect.set_stroke(stroke.map(to_brush));
    rect
}

pub(crate) fn count_calls<T: 'static>(
    elem: &Rc<dyn UIElementExt>,
    name: &'static str,
) -> Rc<RefCell<i32>> {
    let count = Rc::new(RefCell::new(0));
    let counted = Rc::clone(&count);
    elem.as_ui_element().register_routed_handler::<T>(
        name,
        Box::new(move |_: &T, _: &RoutedEventArgs| {
            *counted.borrow_mut() += 1;
        }),
    );
    count
}

pub(crate) fn move_event(x: f32, y: f32) -> crate::input::RawPointerEvent {
    crate::input::RawPointerEvent {
        kind: crate::input::RawPointerEventKind::Moved,
        position: Point { x, y },
        screen_position: None,
        modifiers: crate::input::KeyModifiers::default(),
        timestamp_ms: 0.0,
    }
}

pub(crate) fn press_event(
    x: f32,
    y: f32,
    button: crate::input::MouseButton,
    at_ms: f64,
) -> crate::input::RawPointerEvent {
    crate::input::RawPointerEvent {
        kind: crate::input::RawPointerEventKind::Pressed(button),
        position: Point { x, y },
        screen_position: None,
        modifiers: crate::input::KeyModifiers::default(),
        timestamp_ms: at_ms,
    }
}

pub(crate) fn release_event(
    x: f32,
    y: f32,
    button: crate::input::MouseButton,
    at_ms: f64,
) -> crate::input::RawPointerEvent {
    crate::input::RawPointerEvent {
        kind: crate::input::RawPointerEventKind::Released(button),
        position: Point { x, y },
        screen_position: None,
        modifiers: crate::input::KeyModifiers::default(),
        timestamp_ms: at_ms,
    }
}

pub(crate) fn cancel_event(x: f32, y: f32) -> crate::input::RawPointerEvent {
    crate::input::RawPointerEvent {
        kind: crate::input::RawPointerEventKind::Canceled,
        position: Point { x, y },
        screen_position: None,
        modifiers: crate::input::KeyModifiers::default(),
        timestamp_ms: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overridable_override_dispatches_through_inherit_macro() {
        let base = OverridableBase::new();
        assert_eq!(OverridableBaseExt::compute(&*base, 5), 6);
        assert_eq!(OverridableBaseExt::label(&*base), "base");

        let mid = OverridableMid::new();
        // `compute` isn't overridden at this hop — falls back to `OverridableBase`'s own default.
        assert_eq!(OverridableBaseExt::compute(&*mid, 5), 6);
        assert_eq!(OverridableBaseExt::label(&*mid), "mid");

        let leaf = OverridableLeaf::new();
        // Neither is overridden at `OverridableLeaf` itself: `compute` passes all the way through
        // `OverridableMid` (which never touched it) to `OverridableBase`'s original, while `label`
        // stops at `OverridableMid`'s own override.
        assert_eq!(OverridableBaseExt::compute(&*leaf, 5), 6);
        assert_eq!(OverridableBaseExt::label(&*leaf), "mid");
    }

    /// Issue #128, 7.1: with no override anywhere below it, `ChainBase`'s own implementation runs.
    #[test]
    fn chain_two_hop_no_override_reaches_the_base() {
        let leaf = ChainNoOverride::new();
        assert_eq!(ChainBaseExt::value(&*leaf), 1);
    }

    /// Issue #128, 7.2/9.1: an `#[overrides]` descendant dispatches to itself, and
    /// `self.base.value()` reaches the base implementation.
    #[test]
    fn chain_two_hop_override_dispatches_to_the_descendant() {
        let derived = ChainDerived::new();
        assert_eq!(ChainBaseExt::value(&*derived), 101);
    }

    /// Issue #128, 7.3/7.4/9.2: arbitrary descendant depth,
    /// with an intermediate hop (`ChainDepthD`) that overrides nothing — dispatch must still land
    /// on the nearest real override (`ChainDepthC`), and the deepest override (`ChainDepthE`)
    /// must still be able to forward all the way back to the concrete base. No code anywhere in the
    /// fix is specific to this depth.
    #[test]
    fn chain_four_hop_override_chain_requires_no_new_code() {
        let d = ChainDepthD::new();
        assert_eq!(ChainBaseExt::value(&*d), 11);

        let e = ChainDepthE::new();
        assert_eq!(ChainBaseExt::value(&*e), 1011);
    }

    /// Issue #128, 7.5/9.4: two independent `#[overridable]` methods on the same class, overridden at different descendant depths, must resolve independently.
    #[test]
    fn chain_independent_override_slots_resolve_independently() {
        let e = ChainMultiE::new();
        let log = RefCell::new(Vec::new());
        assert_eq!(ChainMultiBaseExt::first(&*e, &log), 111);
        assert_eq!(ChainMultiBaseExt::second(&*e, &log), 22);
    }

    /// Issue #128, 9.3: forwarding order through `self.base.*()` for a method overridden at two
    /// different hops must visit the closest override first, then each ancestor's own override or
    /// concrete implementation in turn.
    #[test]
    fn chain_override_chain_order_log_matches_expected_sequence() {
        let e = ChainMultiE::new();
        let log = RefCell::new(Vec::new());
        ChainMultiBaseExt::first(&*e, &log);
        assert_eq!(*log.borrow(), vec!["E", "C", "B"]);
    }
}

/// Deterministic `AnimationFrameHost`: never syncs to the wall clock, so tests drive the runtime
/// with explicit `tick` timestamps, and counts frame wake requests.
pub(crate) struct FakeAnimationFrameHost {
    pub(crate) runtime: Rc<AnimationRuntime>,
    pub(crate) frame_requests: Cell<usize>,
}

impl AnimationFrameHost for FakeAnimationFrameHost {
    fn animation_runtime(&self) -> Rc<AnimationRuntime> {
        Rc::clone(&self.runtime)
    }
    fn request_animation_frame(&self) {
        self.frame_requests.set(self.frame_requests.get() + 1);
    }
}

pub(crate) fn install_animation_host(root: &Rc<dyn UIElementExt>) -> Rc<FakeAnimationFrameHost> {
    let host = Rc::new(FakeAnimationFrameHost {
        runtime: AnimationRuntime::new(),
        frame_requests: Cell::new(0),
    });
    root.set_animation_frame_host(Some(host.clone() as Rc<dyn AnimationFrameHost>));
    host
}

/// A hosted `VerticalLayout` of fixed 100x20 leaves with zero spacing, already laid out and
/// retained, for layout reflow tests. `relayout` mirrors a host pass: `layout_root` then
/// `RenderTree::reconcile`.
pub(crate) struct ReflowFixture {
    pub(crate) root: Rc<dyn UIElementExt>,
    pub(crate) children: UIElementCollection,
    pub(crate) items: Vec<Rc<dyn UIElementExt>>,
    pub(crate) host: Rc<FakeAnimationFrameHost>,
    pub(crate) tree: RenderTree,
}

pub(crate) fn reflow_leaf(name: &'static str) -> Rc<dyn UIElementExt> {
    native(name, size(100.0, 20.0))
}

impl ReflowFixture {
    pub(crate) fn new(names: &[&'static str]) -> Self {
        let layout = VerticalLayout::new();
        let children = layout.children().clone();
        let items: Vec<_> = names.iter().map(|name| reflow_leaf(name)).collect();
        for item in &items {
            children.add(Rc::clone(item));
        }
        let root: Rc<dyn UIElementExt> = layout;
        let host = install_animation_host(&root);
        let tree = layout_tree::<FakeHandle>(&root, size(100.0, 200.0));
        Self {
            root,
            children,
            items,
            host,
            tree,
        }
    }

    pub(crate) fn item(&self, index: usize) -> Rc<dyn UIElementExt> {
        Rc::clone(&self.items[index])
    }

    pub(crate) fn relayout(&mut self) {
        layout_root(&self.root, size(100.0, 200.0));
        assert!(self.tree.reconcile::<FakeHandle>(&self.root));
    }

    pub(crate) fn tick_ms(&self, ms: u64) -> bool {
        self.host.runtime.tick(std::time::Duration::from_millis(ms))
    }

    pub(crate) fn group(&self, elem: &Rc<dyn UIElementExt>) -> &RenderGroup {
        let path = self
            .tree
            .group_paths
            .get(&elem.render_group_id())
            .expect("element must have a retained group");
        let mut group = &self.tree.root;
        for index in path {
            group = &group.children[*index];
        }
        group
    }
}

pub(crate) fn reflow_of(elem: &Rc<dyn UIElementExt>) -> crate::base::Vector {
    elem.as_ui_element().layout_reflow_translation.get()
}

/// The vertical position this element is shown at inside its parent: target layout plus reflow.
pub(crate) fn visible_y(elem: &Rc<dyn UIElementExt>) -> f32 {
    elem.arranged_offset().expect("arranged").y + reflow_of(elem).y
}
