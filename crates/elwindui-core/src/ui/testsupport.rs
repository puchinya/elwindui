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

/// A minimal stand-in for a real backend's own `NativeControl`-implementing widget base (e.g.
/// `elwindui-backend-appkit::NativeControlImpl { handle: AnyView, .. }`, shared by that backend's
/// `TextArea`/`Button`/`TabView`) — exercises the same "concrete implementor writes its own
/// `measure_override`/`try_as_native_control`" pattern those use, instead of relying on any
/// generic measuring behavior from `elwindui-core::ui::NativeControl` itself (a pure marker trait
/// — see that trait's own doc comment). Named `FakeNativeControl`, not the bare `NativeControl`
/// that trait already uses, because `#[class]`-generated `__elwindui_inherit_*!` macros share a
/// single flat, crate-wide namespace (unlike ordinary Rust items, which can share a bare name
/// across different modules) — a same-crate bare-name collision is a real `E0428`.
#[elwindui_macros::class(struct_only = crate::ui::NativeControlExt, inherits = crate::ui::UIElement)]
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
            base: UIElement::construct(),
            handle,
        }
    }
}

/// Backend-independent stand-in for a real backend's `TextBox` leaf (e.g.
/// `elwindui-backend-appkit::native_ui::TextBox`) — exercises `elwindui_core::ui::TextBoxExt`'s
/// generated dispatch (measure/try_as_native_control via the inherited `FakeNativeControl` base,
/// plus the `TextBox`-specific setters) without needing a real AppKit/WinUI3 widget.
pub(crate) struct FakeTextBoxState {
    text: RefCell<String>,
    on_change: RefCell<Option<Box<dyn Fn(String)>>>,
}

#[elwindui_macros::class(struct_only = crate::ui::TextBoxExt, inherits = crate::ui::testsupport::FakeNativeControl)]
pub(crate) struct FakeTextBoxWidget {
    state: FakeTextBoxState,
}

#[elwindui_macros::class]
impl FakeTextBoxWidget {
    fn construct(handle: FakeHandle) -> Self {
        Self {
            base: FakeNativeControl::construct(handle),
            state: FakeTextBoxState {
                text: RefCell::new(String::new()),
                on_change: RefCell::new(None),
            },
        }
    }

    fn set_text(&self, text: &str) {
        *self.state.text.borrow_mut() = text.to_string();
        if let Some(callback) = self.state.on_change.borrow().as_ref() {
            callback(text.to_string());
        }
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.state.on_change.borrow_mut() = Some(callback);
    }
    fn set_placeholder(&self, _text: &str) {}
    fn set_read_only(&self, _read_only: bool) {}
    fn set_max_length(&self, _max_length: Option<u32>) {}
    fn set_text_alignment(&self, _alignment: TextAlignment) {}
}

/// Backend-independent stand-in for `PasswordBox` — see `FakeTextBoxWidget`'s own doc comment
/// for the pattern. `PasswordBoxExt`'s dispatch is exercised the same way; the test below
/// additionally checks the no-leak policy (`docs/status/control_status.md`)
/// that every `PasswordBox` implementation — fake or real — must uphold: nothing about this
/// fake ever prints or `Debug`s the password value.
pub(crate) struct FakePasswordBoxState {
    password: RefCell<String>,
    on_change: RefCell<Option<Box<dyn Fn(String)>>>,
}

#[elwindui_macros::class(struct_only = crate::ui::PasswordBoxExt, inherits = crate::ui::testsupport::FakeNativeControl)]
pub(crate) struct FakePasswordBoxWidget {
    state: FakePasswordBoxState,
}

#[elwindui_macros::class]
impl FakePasswordBoxWidget {
    fn construct(handle: FakeHandle) -> Self {
        Self {
            base: FakeNativeControl::construct(handle),
            state: FakePasswordBoxState {
                password: RefCell::new(String::new()),
                on_change: RefCell::new(None),
            },
        }
    }

    fn set_password(&self, password: &str) {
        *self.state.password.borrow_mut() = password.to_string();
        if let Some(callback) = self.state.on_change.borrow().as_ref() {
            callback(password.to_string());
        }
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.state.on_change.borrow_mut() = Some(callback);
    }
    fn set_placeholder(&self, _text: &str) {}
    fn set_max_length(&self, _max_length: Option<u32>) {}
    fn set_reveal_enabled(&self, _enabled: bool) {}
}

/// Backend-independent stand-in for `ScrollView` — see `FakeTextBoxWidget`'s own doc comment
/// for the pattern. Unlike every other `Fake*Widget` here, `ScrollView`'s own content is a full
/// child subtree, not a plain value — this fake models that by overriding the already-
/// `#[overridable]` `visual_children()` (see that trait method's own doc comment) to expose
/// `content`, the same way `elwindui-test::tree`'s tree-dump helper would discover it on a real
/// backend's `InnerScrollView`.
pub(crate) struct FakeScrollViewState {
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
}

#[elwindui_macros::class(struct_only = crate::ui::ScrollViewExt, inherits = crate::ui::testsupport::FakeNativeControl)]
pub(crate) struct FakeScrollViewWidget {
    state: FakeScrollViewState,
}

#[elwindui_macros::class]
impl FakeScrollViewWidget {
    #[overrides]
    fn visual_children(&self) -> Vec<Rc<dyn UIElementExt>> {
        self.state.content.borrow().iter().cloned().collect()
    }

    fn construct(handle: FakeHandle) -> Self {
        Self {
            base: FakeNativeControl::construct(handle),
            state: FakeScrollViewState {
                content: RefCell::new(None),
            },
        }
    }

    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        *self.state.content.borrow_mut() = Some(content);
    }
    fn set_horizontal_scroll_enabled(&self, _enabled: bool) {}
    fn set_vertical_scroll_enabled(&self, _enabled: bool) {}
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
#[elwindui_macros::class(struct_only = crate::ui::NativeControlExt, inherits = crate::ui::UIElement)]
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
            base: UIElement::construct(),
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

#[elwindui_macros::class(struct_only = crate::ui::MenuItemExt)]
pub(crate) struct FakeMenuItem {
    text: RefCell<String>,
    enabled: Cell<bool>,
    shortcut: RefCell<Option<String>>,
    on_select: RefCell<Option<Box<dyn Fn()>>>,
}

#[elwindui_macros::class]
impl FakeMenuItem {
    fn construct() -> Self {
        Self {
            text: RefCell::new(String::new()),
            enabled: Cell::new(true),
            shortcut: RefCell::new(None),
            on_select: RefCell::new(None),
        }
    }
    fn text(&self) -> String {
        self.text.borrow().clone()
    }
    fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
    }
    fn enabled(&self) -> bool {
        self.enabled.get()
    }
    fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }
    fn shortcut(&self) -> Option<String> {
        self.shortcut.borrow().clone()
    }
    fn set_shortcut(&self, key_equivalent: &str) {
        *self.shortcut.borrow_mut() = if key_equivalent.is_empty() {
            None
        } else {
            Some(key_equivalent.to_string())
        };
    }
    fn set_on_select(&self, callback: Box<dyn Fn()>) {
        *self.on_select.borrow_mut() = Some(callback);
    }
    fn select(&self) {
        if let Some(cb) = self.on_select.borrow().as_ref() {
            cb();
        }
    }
}

#[elwindui_macros::class(struct_only = crate::ui::MenuExt)]
pub(crate) struct FakeMenu {
    items: crate::ui::ChildList<dyn crate::ui::MenuItemExt>,
}

#[elwindui_macros::class]
impl FakeMenu {
    fn construct() -> Self {
        Self {
            items: crate::ui::ChildList::new(),
        }
    }
    fn add_item(&self, item: &dyn crate::ui::MenuItemExt) {
        let _ = item;
    }
    fn remove_item(&self, item: &dyn crate::ui::MenuItemExt) {
        let _ = item;
    }
    fn items(&self) -> &dyn crate::ui::ListExt<dyn crate::ui::MenuItemExt> {
        self
    }
}

impl crate::ui::ListExt<dyn crate::ui::MenuItemExt> for FakeMenu {
    fn add(&self, item: Rc<dyn crate::ui::MenuItemExt>) {
        self.items.add(item);
    }
    fn insert(&self, index: usize, item: Rc<dyn crate::ui::MenuItemExt>) {
        self.items.insert(index, item);
    }
    fn remove(&self, item: &Rc<dyn crate::ui::MenuItemExt>) -> bool {
        self.items.remove(item)
    }
    fn remove_at(&self, index: usize) -> Rc<dyn crate::ui::MenuItemExt> {
        self.items.remove_at(index)
    }
    fn clear(&self) {
        self.items.clear();
    }
    fn len(&self) -> usize {
        self.items.len()
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn crate::ui::MenuItemExt>> {
        self.items.to_vec()
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
        modifiers: crate::input::KeyModifiers::default(),
        timestamp_ms: at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_text_box_measures_through_inherited_native_control_base() {
        let widget = FakeTextBoxWidget::new(FakeHandle("textbox", size(80.0, 20.0)));
        let widget: Rc<dyn UIElementExt> = widget;
        // `natural_size` measures with an unconstrained `{0, 0}` available size and reads back
        // `measured_size()` directly — unlike `arranged_width`/`arranged_height` below, it isn't
        // affected by the default `Stretch` alignment filling `layout_tree`'s own `available`, so
        // it verifies `measure_override`/`FakeHandle::measure` ran with the right handle in
        // isolation.
        assert_eq!(natural_size(&*widget), size(80.0, 20.0));
        layout_tree::<FakeHandle>(&widget, size(200.0, 200.0));
        assert!(widget.try_as_native_control().is_some());
    }

    #[test]
    fn fake_text_box_set_text_dispatches_on_change() {
        let widget = FakeTextBoxWidget::new(FakeHandle("textbox", size(80.0, 20.0)));
        let seen = Rc::new(RefCell::new(Vec::new()));
        {
            let seen = Rc::clone(&seen);
            widget.set_on_change(Box::new(move |text| seen.borrow_mut().push(text)));
        }
        widget.set_text("hello");
        widget.set_text("hello world");
        assert_eq!(
            *seen.borrow(),
            vec!["hello".to_string(), "hello world".to_string()]
        );
    }

    #[test]
    fn fake_password_box_measures_through_inherited_native_control_base() {
        let widget = FakePasswordBoxWidget::new(FakeHandle("passwordbox", size(80.0, 20.0)));
        let widget: Rc<dyn UIElementExt> = widget;
        assert_eq!(natural_size(&*widget), size(80.0, 20.0));
        assert!(widget.try_as_native_control().is_some());
    }

    /// No-leak policy check: only the *length* of what `on_change` observed is asserted, and every
    /// assertion in this test uses a fixed, content-free message (never `assert_eq!`'s default
    /// panic message, which would print the actual value on failure) — the same discipline
    /// `docs/status/control_status.md` requires of the real AppKit/WinUI3
    /// implementations too.
    #[test]
    fn fake_password_box_set_password_dispatches_on_change_without_exposing_it_on_failure() {
        let widget = FakePasswordBoxWidget::new(FakeHandle("passwordbox", size(80.0, 20.0)));
        let seen_lengths = Rc::new(RefCell::new(Vec::new()));
        {
            let seen_lengths = Rc::clone(&seen_lengths);
            widget.set_on_change(Box::new(move |password| {
                seen_lengths.borrow_mut().push(password.chars().count())
            }));
        }
        widget.set_password("hunter2");
        widget.set_password("");
        assert!(
            *seen_lengths.borrow() == vec![7, 0],
            "password change callback fired the wrong number of times or with the wrong lengths"
        );
    }

    #[test]
    fn fake_scroll_view_measures_through_inherited_native_control_base() {
        let widget = FakeScrollViewWidget::new(FakeHandle("scrollview", size(300.0, 200.0)));
        let widget: Rc<dyn UIElementExt> = widget;
        assert_eq!(natural_size(&*widget), size(300.0, 200.0));
        assert!(widget.try_as_native_control().is_some());
    }

    /// Verifies `content` stays reachable via `visual_children()` once set — the property a real
    /// backend's nested `TreeHostView`/`TreeHostPanel` content host relies on for hit-testing/
    /// tree-dump purposes (`docs/status/control_status.md`).
    #[test]
    fn fake_scroll_view_content_reachable_via_visual_children() {
        let widget = FakeScrollViewWidget::new(FakeHandle("scrollview", size(300.0, 200.0)));
        let content = native("inner", size(50.0, 50.0));
        widget.set_content(Rc::clone(&content));
        let widget: Rc<dyn UIElementExt> = widget;
        let children = widget.visual_children();
        assert_eq!(children.len(), 1);
        assert!(Rc::ptr_eq(&children[0], &content));
    }

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
}
