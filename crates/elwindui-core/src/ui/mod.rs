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
//! see docs/design/gui_framework_design.md §5.3.
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
//! (`UIElement::visual_parent`, WinUI3's `_parent`) so `dispatch_routed` can bubble a routed event
//! from any element up to the root by simply following `visual_parent()` — no tree search needed,
//! and critically, no dependence on the tree having been built by a single static DSL
//! traversal. Matches real WinUI3/UWP, where measure/arrange/render/hit-test *and* routed-event
//! bubbling all walk the Visual tree — the separate Logical `parent` back-reference exists purely
//! as a receptacle for a future template/accessibility tree (see `UIElementCollection`'s own doc
//! comment) and plays no part in event routing. A back-reference requires shared (`Rc`) ownership,
//! allowing a child to point back to its parent. Every collection's own owner is already fully
//! established by the time `construct()` returns (via `#[class]`'s `__self_weak`, see
//! `UIElement::construct`) — well before any child is ever added.

use crate::base::{CornerRadius, Point, Rect, Size};
use crate::input::{FocusState, RoutedEventArgs};
use crate::theme::{
    SystemTheme, ThemeChangeImpact, ThemeContext, ThemeHandle, ThemeValue,
};
use crate::layout::{
    GridCell, GridLength, HorizontalAlignment, Orientation, VerticalAlignment, Visibility,
    align_within, apply_size_constraints, grid_arrange, grid_measure_pass1_available,
    grid_pass2_available, grid_resolve_track_sizes, grow_by_margin, shrink_by_margin,
    shrink_rect_by_margin, stack_arrange, stack_natural_size,
};
#[cfg(test)]
use crate::graphics::Color;
#[cfg(test)]
use crate::graphics::RenderCommand;
pub use crate::graphics::TextAlignment;
use crate::graphics::{
    Brush, ImageDrawOptions, ImageFit, ImageSource, RenderContext, RenderGroup, RenderTree,
    Stretch, StrokeStyle, VectorImageDrawOptions, VectorRasterizeMode,
};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

// Submodule declaration order is load-bearing. `#[elwindui_macros::class]` registers each class in
// a cross-invocation "same crate classes" table as it expands, and a derived class that expands
// *before* its base is registered falls back to emitting
// `crate::ui::__elwindui_macros_of_Base::__elwindui_props_Base` — a path that does not exist, since
// that wrapper module only re-exports the inherit trio and `trait_only` classes never emit it at
// all. So bases must be declared before anything that `inherits` them. (`pub use` order below is
// irrelevant — only `mod` order drives expansion order.)
//
// The submodules deliberately open with `use super::*;` rather than repeating this file's import
// block, which is what let the original single-file `ui.rs` be split as a pure code move.
mod element;

mod controls;

mod collections;
mod engine;
mod text_style;

// Glob re-exports, never named lists: `#[class]` emits a companion `__elwindui_macros_of_*` module
// next to each class, which downstream `#[component(inherits ..)]` resolves as
// `elwindui::ui::__elwindui_macros_of_Window`. Naming only the types here would strand those
// aliases in the submodule and break every inheriting user component — the same constraint
// `elwindui-backend-appkit`'s `native_ui/mod.rs` documents for its own split.
pub use collections::*;
pub use controls::*;
pub use element::*;
pub use engine::*;
pub use text_style::*;



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
    /// across different modules) — a same-crate bare-name collision is a real `E0428`.
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
    struct FakeTextBoxState {
        text: RefCell<String>,
        on_change: RefCell<Option<Box<dyn Fn(String)>>>,
    }

    #[elwindui_macros::class(struct_only = crate::ui::TextBoxExt, inherits = crate::ui::tests::FakeNativeControl)]
    struct FakeTextBoxWidget {
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
        assert_eq!(*seen.borrow(), vec!["hello".to_string(), "hello world".to_string()]);
    }

    /// Backend-independent stand-in for `PasswordBox` — see `FakeTextBoxWidget`'s own doc comment
    /// for the pattern. `PasswordBoxExt`'s dispatch is exercised the same way; the test below
    /// additionally checks the no-leak policy (`docs/status/nativecontrol_status.md`)
    /// that every `PasswordBox` implementation — fake or real — must uphold: nothing about this
    /// fake ever prints or `Debug`s the password value.
    struct FakePasswordBoxState {
        password: RefCell<String>,
        on_change: RefCell<Option<Box<dyn Fn(String)>>>,
    }

    #[elwindui_macros::class(struct_only = crate::ui::PasswordBoxExt, inherits = crate::ui::tests::FakeNativeControl)]
    struct FakePasswordBoxWidget {
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
    /// `docs/status/nativecontrol_status.md` requires of the real AppKit/WinUI3
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

    /// Backend-independent stand-in for `ScrollView` — see `FakeTextBoxWidget`'s own doc comment
    /// for the pattern. Unlike every other `Fake*Widget` here, `ScrollView`'s own content is a full
    /// child subtree, not a plain value — this fake models that by overriding the already-
    /// `#[overridable]` `visual_children()` (see that trait method's own doc comment) to expose
    /// `content`, the same way `elwindui-test::tree`'s tree-dump helper would discover it on a real
    /// backend's `InnerScrollView`.
    struct FakeScrollViewState {
        content: RefCell<Option<Rc<dyn UIElementExt>>>,
    }

    #[elwindui_macros::class(struct_only = crate::ui::ScrollViewExt, inherits = crate::ui::tests::FakeNativeControl)]
    struct FakeScrollViewWidget {
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

    #[test]
    fn fake_scroll_view_measures_through_inherited_native_control_base() {
        let widget = FakeScrollViewWidget::new(FakeHandle("scrollview", size(300.0, 200.0)));
        let widget: Rc<dyn UIElementExt> = widget;
        assert_eq!(natural_size(&*widget), size(300.0, 200.0));
        assert!(widget.try_as_native_control().is_some());
    }

    /// Verifies `content` stays reachable via `visual_children()` once set — the property a real
    /// backend's nested `TreeHostView`/`TreeHostPanel` content host relies on for hit-testing/
    /// tree-dump purposes (`docs/status/nativecontrol_status.md`).
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

    /// `#[overridable]`/`#[overrides]` usage example, exercised across a genuine 3-hop chain
    /// (`OverridableBase` -> `OverridableMid` -> `OverridableLeaf`) with two overridable methods —
    /// `OverridableMid` overrides only `label`, leaving `compute` untouched, and `OverridableLeaf`
    /// (which itself overrides neither) relies on defaults for both. This exercises resolution of
    /// overridable methods across the chain: one dedicated accessor per `#[overridable]` method is
    /// resolved independently, ensuring that overrides at intermediate hops are dispatched correctly
    /// while untouched methods pass through (see `per_method_accessor_ident`'s own doc comment for details).
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
        fn construct() -> Self {
            Self {
                base: UIElement::construct(),
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
        fn construct() -> Self {
            Self {
                base: OverridableBase::construct(),
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
        fn construct() -> Self {
            Self {
                base: OverridableMid::construct(),
            }
        }
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
        // size instead of filling its stack-allocated cross-axis slot.
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

    // `abstract_shape_has_no_commands_and_no_children` used to live here. It constructed a bare
    // `Shape`, set a fill on it, and asserted the tree still produced no paint commands — i.e. that
    // the base class has no render of its own. `Shape` is now `abstract_class`, so `#[class]` no
    // longer synthesizes `Shape::new()` and that state is unreachable by construction rather than by
    // assertion. The reachable half of what it covered — a shape with neither fill nor stroke paints
    // nothing — is exercised through a concrete `Rectangle` by
    // `shape_is_hit_testable_only_when_fill_or_stroke_is_set` below.

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

    /// Records every `available` it's actually measured with (in call order), so tests below can
    /// assert on it directly instead of inferring it indirectly through a returned size — including
    /// `Grid`'s two-pass measurement, where the same child is measured twice and both calls matter.
    /// Deliberately not built on `FakeHandle`/`FakeNativeControl` — `FakeHandle` derives `PartialEq`
    /// and is compared structurally by several other tests, so adding a recording field there would
    /// need every one of those to account for it too.
    #[elwindui_macros::class(struct_only = crate::ui::NativeControlExt, inherits = crate::ui::UIElement)]
    struct MeasureProbe {
        calls: RefCell<Vec<Size>>,
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
        fn last_available(&self) -> Size {
            *self.calls.borrow().last().expect("measure_override was never called")
        }
    }

    #[test]
    fn vertical_layout_measures_children_with_unconstrained_main_axis() {
        // A content-sized `VerticalLayout` must size itself from each child's own natural height,
        // not from whatever finite `available` its own parent happened to hand it — passing
        // `available.height` straight through to children would let a large parent silently
        // inflate every child's measured height.
        let probe = MeasureProbe::new(size(10.0, 20.0));
        let child: Rc<dyn UIElementExt> = probe.clone();
        let root = VerticalLayout::new();
        root.children().add(child);
        root.measure(size(200.0, 50.0));
        let last = probe.last_available();
        assert_eq!(
            last.width, 200.0,
            "cross axis (width) must stay constrained to the container's own available width"
        );
        assert!(
            last.height.is_infinite() && last.height > 0.0,
            "main axis (height) must be unconstrained, got {:?}",
            last.height
        );
    }

    #[test]
    fn horizontal_layout_measures_children_with_unconstrained_main_axis() {
        let probe = MeasureProbe::new(size(20.0, 10.0));
        let child: Rc<dyn UIElementExt> = probe.clone();
        let root = HorizontalLayout::new();
        root.children().add(child);
        root.measure(size(50.0, 200.0));
        let last = probe.last_available();
        assert_eq!(
            last.height, 200.0,
            "cross axis (height) must stay constrained to the container's own available height"
        );
        assert!(
            last.width.is_infinite() && last.width > 0.0,
            "main axis (width) must be unconstrained, got {:?}",
            last.width
        );
    }

    #[test]
    fn grid_measures_children_in_two_passes_per_track_kind() {
        // Single Auto row; Fixed(50)/Auto/Star(1.0) columns, one child per column.
        let root = Grid::new();
        root.set_rows(vec![GridLength::Auto]);
        root.set_columns(vec![
            GridLength::Fixed(50.0),
            GridLength::Auto,
            GridLength::Star(1.0),
        ]);

        let fixed_child = MeasureProbe::new(size(10.0, 10.0));
        fixed_child.set_attached("Grid", "column", 0i32);
        let auto_child = MeasureProbe::new(size(30.0, 10.0));
        auto_child.set_attached("Grid", "column", 1i32);
        let star_child = MeasureProbe::new(size(20.0, 10.0));
        star_child.set_attached("Grid", "column", 2i32);

        root.children().add(fixed_child.clone());
        root.children().add(auto_child.clone());
        root.children().add(star_child.clone());

        root.measure(size(300.0, 100.0));

        // Pass 1: `Fixed` measures at its own literal size; `Auto`/`Star` measure unconstrained
        // (on both axes -- the row is `Auto` too) so each child's own natural size is exactly what
        // comes back to resolve its track.
        assert_eq!(
            fixed_child.calls.borrow()[0],
            Size {
                width: 50.0,
                height: f32::INFINITY
            }
        );
        assert!(auto_child.calls.borrow()[0].width.is_infinite());
        assert!(star_child.calls.borrow()[0].width.is_infinite());

        // Pass 2: every child is re-measured at its now-fully-resolved cell size — `Fixed` column
        // stays 50, `Auto` column becomes its own natural width (30), `Star` column gets whatever's
        // left (300 - 50 - 30 = 220). The `Auto` row resolves to 10 (every child's own natural
        // height) and every child is re-measured at that height too.
        assert_eq!(
            fixed_child.last_available(),
            Size {
                width: 50.0,
                height: 10.0
            }
        );
        assert_eq!(
            auto_child.last_available(),
            Size {
                width: 30.0,
                height: 10.0
            }
        );
        assert_eq!(
            star_child.last_available(),
            Size {
                width: 220.0,
                height: 10.0
            }
        );
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
        tree.as_ui_element().set_width(50.0);
        tree.as_ui_element().set_height(5.0);
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
        tree.as_ui_element().set_min_width(30.0);
        tree.as_ui_element().set_max_height(8.0);
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

    #[test]
    fn render_item_ordering_preserves_traversal_order_across_native_and_paint() {
        // A painting container containing a native leaf child: traversal visits the container
        // itself (pushing its `Paint`) before recursing into its child (pushing the child's
        // `Native`), so the combined list must come back `[Paint, Native]` — a backend replaying
        // this list in order therefore places the native leaf *in front of* the container's own
        // paint, matching the source tree's parent-then-child nesting instead of an accidental
        // "all natives first" or "all paints first" batching.
        // `PaintingContainer` isn't `#[class]`-managed (it hand-implements `UIElementExt` directly,
        // above), so it has no auto-generated `new()` to reach `UIElement::construct`'s real,
        // hidden-weak-parameter form through — it calls the internal `__class_construct` directly,
        // via its own hand-rolled `Rc::new_cyclic`, exactly the shape any non-`#[class]` code needing
        // a real self-weak has to use.
        let tree = Rc::<PaintingContainer>::new_cyclic(|weak: &Weak<PaintingContainer>| {
            let weak: Weak<dyn UIElementExt> = weak.clone();
            PaintingContainer {
                base: UIElement::__class_construct(weak),
            }
        });
        tree.as_ui_element()
            .visual_collection
            .add(native("child", size(10.0, 10.0)));
        let tree: Rc<dyn UIElementExt> = tree;
        let render_tree = layout_tree::<FakeHandle>(&tree, size(50.0, 50.0));
        assert!(matches!(
            render_tree.root.commands[0],
            RenderCommand::FillRoundedRect { .. }
        ));
        assert!(matches!(
            render_tree.root.children[0].commands[0],
            RenderCommand::NativeControl { .. }
        ));
    }

    #[test]
    fn layout_background_is_transparent_by_default_and_paints_before_children() {
        let layout = VerticalLayout::new();
        let child = Rectangle::new();
        child.set_fill(Some(Brush::Solid(Color::rgb(10, 20, 30))));
        child.set_width(20.0);
        child.set_height(20.0);
        layout.children().add(child);

        let root: Rc<dyn UIElementExt> = layout.clone();
        let transparent = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(transparent.root.commands.is_empty());
        assert!(matches!(
            transparent.root.children[0].commands[0],
            RenderCommand::FillRoundedRect { .. }
        ));

        layout.set_background(Some(Brush::Solid(Color::rgb(1, 2, 3))));
        let painted = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(matches!(
            painted.root.commands[0],
            RenderCommand::FillRect { .. }
        ));
        assert!(matches!(
            painted.root.children[0].commands[0],
            RenderCommand::FillRoundedRect { .. }
        ));
    }

    #[test]
    fn text_block_defaults_to_left_alignment_and_set_text_alignment_updates_paint() {
        let text_block = TextBlock::new();
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
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                foreground: None,
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

        text_block.set_foreground(Some(Brush::Solid(Color::rgb(1, 2, 3))));
        commands.clear();
        text_block.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                foreground: Some(Brush::Solid(Color { r: 1, g: 2, b: 3, .. })),
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

    // --- Font/text-style tests (指示書 §14-30, §32) ---------------------------------------------

    #[test]
    fn as_text_style_owner_is_none_for_non_owning_elements() {
        // Grid/Layout/Shape must stay transparent to inheritance (指示書 §11) — verified directly
        // via the downcast hook rather than only indirectly through an inheritance chain.
        let grid = Grid::new();
        assert!(grid.as_text_style_owner().is_none());
        let stack = VerticalLayout::new();
        assert!(stack.as_text_style_owner().is_none());
        let rect = Rectangle::new();
        assert!(rect.as_text_style_owner().is_none());
    }

    #[test]
    fn as_text_style_owner_is_some_for_control_and_text_block() {
        let control = Control::new();
        assert!(control.as_text_style_owner().is_some());
        let text_block = TextBlock::new();
        assert!(text_block.as_text_style_owner().is_some());
    }

    #[test]
    fn orphan_text_block_resolves_to_backend_default() {
        let text_block = TextBlock::new();
        let style = text_block.resolved_text_style();
        assert_eq!(style, crate::graphics::text_backend().default_text_style());
    }

    #[test]
    fn control_font_size_inherits_through_grid_to_nested_text_block() {
        // Control -(Visual)-> Grid -(Visual)-> TextBlock: Grid is not a TextStyleOwner, so it must
        // not block inheritance (指示書 §11's own worked example).
        let control = Control::new();
        control.set_font_size(24.0);
        let grid = Grid::new();
        let text_block = TextBlock::new();
        grid.children().add(text_block.clone());
        control.as_ui_element().visual_collection.add(grid.clone());

        let style = text_block.resolved_text_style();
        assert_eq!(style.font_size, 24.0);
    }

    #[test]
    fn child_local_value_wins_over_inherited() {
        let control = Control::new();
        control.set_font_size(24.0);
        let text_block = TextBlock::new();
        text_block.set_font_size(12.0);
        control.as_ui_element().visual_collection.add(text_block.clone());

        assert_eq!(text_block.resolved_text_style().font_size, 12.0);
    }

    #[test]
    fn child_partial_override_leaves_other_properties_inherited() {
        // Setting only `font_size` locally must not disturb `font_family`/`font_weight`/etc. —
        // each of the seven properties resolves independently (指示書 §7/§19, never a wholesale
        // "inherit the whole struct" replacement).
        let control = Control::new();
        control.set_font_size(24.0);
        control.set_font_family(crate::graphics::FontFamily::new("Helvetica"));
        control.set_font_weight(crate::graphics::FontWeight::BOLD);
        let text_block = TextBlock::new();
        text_block.set_font_size(12.0);
        control.as_ui_element().visual_collection.add(text_block.clone());

        let style = text_block.resolved_text_style();
        assert_eq!(style.font_size, 12.0); // local wins
        assert_eq!(style.font_family, crate::graphics::FontFamily::new("Helvetica")); // inherited
        assert_eq!(style.font_weight, crate::graphics::FontWeight::BOLD); // inherited
    }

    #[test]
    fn clear_font_size_reverts_to_inherited_value() {
        let control = Control::new();
        control.set_font_size(24.0);
        let text_block = TextBlock::new();
        text_block.set_font_size(12.0);
        control.as_ui_element().visual_collection.add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 12.0);

        text_block.clear_font_size();
        assert_eq!(text_block.resolved_text_style().font_size, 24.0);
    }

    #[test]
    fn setting_font_size_invalidates_measure_but_foreground_only_invalidates_arrange() {
        let text_block = TextBlock::new();
        let host = Rc::new(RecordingRelayoutHost::default());
        text_block.set_invalidate_host(Some(host.clone() as Rc<dyn RelayoutHost>));
        layout_root(&(text_block.clone() as Rc<dyn UIElementExt>), size(100.0, 100.0));
        assert!(text_block.measured_size().is_some());
        assert!(text_block.arranged_width().is_some());

        text_block.set_font_size(20.0);
        assert!(
            text_block.measured_size().is_none(),
            "a font-size change must invalidate measure"
        );

        layout_root(&(text_block.clone() as Rc<dyn UIElementExt>), size(100.0, 100.0));
        assert!(text_block.measured_size().is_some());
        text_block.set_foreground(Some(crate::graphics::Brush::Solid(crate::graphics::Color::white())));
        assert!(
            text_block.measured_size().is_some(),
            "a foreground-only change must not invalidate measure"
        );
        assert!(text_block.arranged_width().is_none());
    }

    #[test]
    fn reparenting_text_block_re_resolves_from_the_new_parent() {
        let old_parent = Control::new();
        old_parent.set_font_size(10.0);
        let new_parent = Control::new();
        new_parent.set_font_size(30.0);
        let text_block = TextBlock::new();
        old_parent.as_ui_element().visual_collection.add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 10.0);

        old_parent.as_ui_element().visual_collection.remove(&(text_block.clone() as Rc<dyn UIElementExt>));
        new_parent.as_ui_element().visual_collection.add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 30.0);
    }

    #[test]
    fn removed_from_parent_falls_back_to_backend_default() {
        let parent = Control::new();
        parent.set_font_size(30.0);
        let text_block = TextBlock::new();
        parent.as_ui_element().visual_collection.add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 30.0);

        parent.as_ui_element().visual_collection.remove(&(text_block.clone() as Rc<dyn UIElementExt>));
        assert_eq!(
            text_block.resolved_text_style().font_size,
            crate::graphics::text_backend().default_text_style().font_size
        );
    }

    #[test]
    fn inheritance_parent_logical_falls_back_to_visual_when_no_logical_parent() {
        let root = VerticalLayout::new();
        let child = native("a", size(10.0, 10.0));
        root.as_ui_element().visual_collection.add(child.clone());
        // `child` has a Visual parent (`root`, via the raw visual collection) but no Logical
        // parent (never added through `UIElementCollection`) — `Logical` must still find `root`
        // by falling back to Visual (指示書 §14).
        assert!(child.parent().is_none());
        let via_logical = child
            .inheritance_parent(InheritanceParentKind::Logical)
            .expect("Logical must fall back to Visual when there is no logical parent");
        assert!(Rc::ptr_eq(&via_logical, &(root.clone() as Rc<dyn UIElementExt>)));
        let via_visual = child
            .inheritance_parent(InheritanceParentKind::Visual)
            .expect("Visual parent must be reachable directly");
        assert!(Rc::ptr_eq(&via_visual, &(root as Rc<dyn UIElementExt>)));
    }

    #[test]
    fn content_control_inherits_text_style_from_its_base_control() {
        // Regression guard for the `Attr::TextStyle` exemption in `resolve_effective_fields`/
        // `resolve_field_declaring_types` (`elwindui-codegen`'s `codegen.rs`) — without it, a
        // `has_view` component like `ContentControl` would silently lose all seven text-style
        // setters (they'd never even compile-error; the DSL setter would just not exist). This
        // exercises the *runtime* half: `ContentControl::as_text_style_owner()` must resolve
        // through the `#[class]` ancestor-forwarding chain to its embedded `base: Control`, which
        // really implements `TextStyleOwner` — not `ContentControl` itself (see
        // `emit_field_setter_call`'s own doc comment on why `elwindui-codegen` always goes through
        // `as_text_style_owner()` rather than assuming `TextStyleOwner` is implemented directly).
        let content_control = ContentControl::new();
        let owner = content_control
            .as_text_style_owner()
            .expect("ContentControl must resolve a TextStyleOwner through its Control base");
        owner.set_font_size(18.0);
        assert_eq!(
            content_control
                .as_text_style_owner()
                .unwrap()
                .resolved_text_style()
                .font_size,
            18.0
        );

        let inner = TextBlock::new();
        content_control.set_content(inner.clone());
        assert_eq!(inner.resolved_text_style().font_size, 18.0);
    }

    #[derive(Default)]
    struct RecordingRelayoutHost {
        requests: RefCell<Vec<u64>>,
    }
    impl RelayoutHost for RecordingRelayoutHost {
        fn request_relayout(&self, dirty_group_id: u64) {
            self.requests.borrow_mut().push(dirty_group_id);
        }
    }

    #[test]
    fn dynamic_child_slot_reuses_rc_item_children_and_applies_source_order() {
        struct TestList(RefCell<Vec<Rc<String>>>);

        impl ListExt<String> for TestList {
            fn add(&self, item: Rc<String>) {
                self.0.borrow_mut().push(item);
            }
            fn insert(&self, index: usize, item: Rc<String>) {
                self.0.borrow_mut().insert(index, item);
            }
            fn remove(&self, item: &Rc<String>) -> bool {
                let mut items = self.0.borrow_mut();
                let Some(index) = items.iter().position(|current| Rc::ptr_eq(current, item)) else {
                    return false;
                };
                items.remove(index);
                true
            }
            fn remove_at(&self, index: usize) -> Rc<String> {
                self.0.borrow_mut().remove(index)
            }
            fn clear(&self) {
                self.0.borrow_mut().clear();
            }
            fn len(&self) -> usize {
                self.0.borrow().len()
            }
            fn is_empty(&self) -> bool {
                self.0.borrow().is_empty()
            }
            fn to_vec(&self) -> Vec<Rc<String>> {
                self.0.borrow().clone()
            }
        }

        let slot = DynamicChildSlot::<String>::default();
        let host = TestList(RefCell::new(Vec::new()));
        let leading = Rc::new("leading".to_owned());
        let trailing = Rc::new("trailing".to_owned());
        let first = Rc::new("first".to_owned());
        let second = Rc::new("second".to_owned());
        let renders = Cell::new(0);
        let first_subscription_dropped = Rc::new(Cell::new(false));
        let second_subscription_dropped = Rc::new(Cell::new(false));
        host.add(Rc::clone(&leading));
        host.add(Rc::clone(&trailing));

        slot.replace_rc_items(&host, 1, &[Rc::clone(&first), Rc::clone(&second)], |item| {
            renders.set(renders.get() + 1);
            let dropped = if Rc::ptr_eq(item, &first) {
                Rc::clone(&first_subscription_dropped)
            } else {
                Rc::clone(&second_subscription_dropped)
            };
            DynamicChild::with_subscriptions(
                Rc::new(format!("child:{item}")),
                vec![crate::reactive::Subscription::new(move || {
                    dropped.set(true)
                })],
            )
        });
        let original = host.to_vec();
        assert_eq!(renders.get(), 2);
        assert!(Rc::ptr_eq(&original[0], &leading));
        assert!(Rc::ptr_eq(&original[3], &trailing));

        slot.replace_rc_items(&host, 1, &[Rc::clone(&second), Rc::clone(&first)], |_| {
            panic!("an unchanged Rc item must reuse its child")
        });
        let reordered = host.to_vec();
        assert_eq!(renders.get(), 2);
        assert!(Rc::ptr_eq(&reordered[0], &leading));
        assert!(Rc::ptr_eq(&reordered[1], &original[2]));
        assert!(Rc::ptr_eq(&reordered[2], &original[1]));
        assert!(Rc::ptr_eq(&reordered[3], &trailing));

        slot.replace_rc_items(&host, 1, &[Rc::clone(&second)], |_| {
            panic!("a retained Rc item must not be rendered again")
        });
        assert!(first_subscription_dropped.get());
        assert!(!second_subscription_dropped.get());

        slot.replace_children(
            &host,
            1,
            vec![
                Rc::new("first-child".to_owned()),
                Rc::new("second-child".to_owned()),
            ],
        );
        assert_eq!(slot.len(), 2);
        assert_eq!(
            host.to_vec().len(),
            4,
            "the range occupies both grouped children"
        );
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
    fn dispatch_routed_bubbles_via_visual_parent_even_without_a_logical_parent() {
        // `leaf` is added straight to `root`'s `visual_collection`, bypassing
        // `UIElementCollection` — matching `logical_and_visual_collections_keep_their_parent_relationships_separate`'s
        // `visual_only` pattern, `leaf` ends up with a `visual_parent` but no logical `parent()` at
        // all. `dispatch_routed` must still reach `root`'s handler, since it bubbles via
        // `visual_parent` (real WinUI3 semantics), not the Logical `parent` chain.
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![]);
        root.as_ui_element().visual_collection.add(leaf.clone());
        assert!(leaf.parent().is_none());

        let root_calls = Rc::new(RefCell::new(0));
        {
            let root_calls = Rc::clone(&root_calls);
            root.as_ui_element().register_routed_handler::<()>(
                "on_click",
                Box::new(move |_, _| *root_calls.borrow_mut() += 1),
            );
        }

        let args = RoutedEventArgs::default();
        dispatch_routed(&leaf, "on_click", &(), &args);
        assert_eq!(*root_calls.borrow(), 1);
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

    #[test]
    fn layout_containers_are_transparent_to_hit_testing() {
        let leaf = native("leaf", size(10.0, 10.0));
        leaf.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        leaf.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        layout_tree::<FakeHandle>(&root, size(100.0, 100.0));

        assert!(Rc::ptr_eq(
            &hit_test(&root, Point { x: 5.0, y: 5.0 }).expect("leaf should be hit"),
            &leaf
        ));
        assert!(
            hit_test(&root, Point { x: 50.0, y: 50.0 }).is_none(),
            "VerticalLayout has no Background/Fill concept, so its own empty space must not be \
             hit-testable — a click there falls through instead of hitting the container itself"
        );
    }

    fn rectangle(fill: Option<&str>, stroke: Option<&str>) -> Rc<dyn UIElementExt> {
        let to_brush = |hex: &str| Brush::Solid(Color::parse_hex(hex).unwrap());
        let rect = Rectangle::new();
        rect.set_fill(fill.map(to_brush));
        rect.set_stroke(stroke.map(to_brush));
        rect
    }

    #[test]
    fn shape_is_hit_testable_only_when_fill_or_stroke_is_set() {
        let transparent = rectangle(None, None);
        transparent.as_ui_element().set_width(20.0);
        transparent.as_ui_element().set_height(20.0);
        layout_tree::<FakeHandle>(&transparent, size(100.0, 100.0));
        assert!(
            hit_test(&transparent, Point { x: 5.0, y: 5.0 }).is_none(),
            "a Shape with neither fill nor stroke set paints nothing, so it must not be hit"
        );

        let filled = rectangle(Some("#ffffff"), None);
        filled.as_ui_element().set_width(20.0);
        filled.as_ui_element().set_height(20.0);
        layout_tree::<FakeHandle>(&filled, size(100.0, 100.0));
        assert!(hit_test(&filled, Point { x: 5.0, y: 5.0 }).is_some());
    }

    #[test]
    fn hit_test_visible_false_excludes_the_element_and_its_whole_subtree() {
        let leaf = native("leaf", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        layout_tree::<FakeHandle>(&root, size(100.0, 100.0));
        assert!(hit_test(&root, Point { x: 5.0, y: 5.0 }).is_some());

        root.as_ui_element().set_hit_test_visible(false);
        assert!(
            hit_test(&root, Point { x: 5.0, y: 5.0 }).is_none(),
            "IsHitTestVisible=false must exclude descendants too, not just the element itself"
        );
    }

    #[test]
    fn hit_test_respects_clip_to_bounds_only_when_actually_set() {
        // Manually wired (not `stack`/`layout_tree`) so the child's own arranged rect can be made
        // to genuinely overflow its parent's — exactly the case `clip_to_bounds` distinguishes.
        let child = native("child", size(50.0, 50.0));
        let parent = native("parent", size(20.0, 20.0));
        parent.as_ui_element().visual_collection.add(child.clone());
        parent.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        });
        child.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        });

        // Outside `parent`'s own 20x20 rect but inside the child's own (overflowing) 50x50 one.
        let outside_parent = Point { x: 30.0, y: 30.0 };
        assert!(
            Rc::ptr_eq(
                &hit_test(&parent, outside_parent).expect("the overflowing child should be hit"),
                &child
            ),
            "clip_to_bounds defaults to false, so a child rendering outside its parent's own \
             bounds must remain hit-testable there"
        );

        parent.as_ui_element().set_clip_to_bounds(Some(true));
        assert!(
            hit_test(&parent, outside_parent).is_none(),
            "once the parent opts into clip_to_bounds, the overflowing child must be excluded too"
        );
    }

    fn count_calls<T: 'static>(
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

    fn move_event(x: f32, y: f32) -> crate::input::RawPointerEvent {
        crate::input::RawPointerEvent {
            kind: crate::input::RawPointerEventKind::Moved,
            position: Point { x, y },
            modifiers: crate::input::KeyModifiers::default(),
            timestamp_ms: 0.0,
        }
    }

    fn press_event(
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

    fn release_event(
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

    #[test]
    fn pointer_entered_exited_do_not_refire_on_a_still_hovered_shared_ancestor() {
        let leaf_a = native("a", size(10.0, 10.0));
        let leaf_b = native("b", size(10.0, 10.0));
        let root = stack(
            Orientation::Vertical,
            0.0,
            vec![Rc::clone(&leaf_a), Rc::clone(&leaf_b)],
        );
        layout_tree::<FakeHandle>(&root, size(100.0, 100.0));

        let root_entered =
            count_calls::<crate::input::PointerEventArgs>(&root, "on_pointer_entered");
        let root_exited = count_calls::<crate::input::PointerEventArgs>(&root, "on_pointer_exited");
        let a_entered =
            count_calls::<crate::input::PointerEventArgs>(&leaf_a, "on_pointer_entered");
        let a_exited = count_calls::<crate::input::PointerEventArgs>(&leaf_a, "on_pointer_exited");
        let b_entered =
            count_calls::<crate::input::PointerEventArgs>(&leaf_b, "on_pointer_entered");
        let b_exited = count_calls::<crate::input::PointerEventArgs>(&leaf_b, "on_pointer_exited");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(&root, &focus, move_event(5.0, 5.0));
        assert_eq!((*root_entered.borrow(), *a_entered.borrow()), (1, 1));

        // Moving from `a` to `b` (both under the same `root`) must not re-fire `root`'s own
        // Entered/Exited — it was, and remains, hovered throughout.
        dispatcher.handle(&root, &focus, move_event(5.0, 15.0));
        assert_eq!(*a_exited.borrow(), 1);
        assert_eq!(*b_entered.borrow(), 1);
        assert_eq!((*root_entered.borrow(), *root_exited.borrow()), (1, 0));

        // Moving off the tree entirely (into the layout's own transparent empty space) exits
        // everything, `root` included.
        dispatcher.handle(&root, &focus, move_event(5.0, 90.0));
        assert_eq!(*b_exited.borrow(), 1);
        assert_eq!(*root_exited.borrow(), 1);
    }

    #[test]
    fn tap_fires_even_after_dragging_out_and_back_within_threshold() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        // Wanders far outside `leaf`'s own bounds mid-drag — implicit capture must keep routing
        // `Moved`/`Released` to `leaf` regardless.
        dispatcher.handle(&leaf, &focus, move_event(500.0, 500.0));
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(6.0, 6.0, crate::input::MouseButton::Left, 10.0),
        );

        assert_eq!(*tapped.borrow(), 1);
    }

    #[test]
    fn tap_does_not_fire_when_release_moves_past_the_threshold() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");
        let pressed = count_calls::<crate::input::PointerEventArgs>(&leaf, "on_pointer_pressed");
        let released = count_calls::<crate::input::PointerEventArgs>(&leaf, "on_pointer_released");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(20.0, 20.0, crate::input::MouseButton::Left, 10.0),
        );

        assert_eq!(*pressed.borrow(), 1);
        assert_eq!(*released.borrow(), 1);
        assert_eq!(*tapped.borrow(), 0);
    }

    #[test]
    fn double_tap_fires_on_a_second_nearby_tap_within_the_time_window() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");
        let double_tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_double_tapped");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Left, 10.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(6.0, 6.0, crate::input::MouseButton::Left, 100.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(6.0, 6.0, crate::input::MouseButton::Left, 110.0),
        );

        assert_eq!(*tapped.borrow(), 2);
        assert_eq!(*double_tapped.borrow(), 1);

        // A third tap right after pairs with nothing (the second tap's own record was consumed).
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(6.0, 6.0, crate::input::MouseButton::Left, 150.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(6.0, 6.0, crate::input::MouseButton::Left, 155.0),
        );
        assert_eq!(*tapped.borrow(), 3);
        assert_eq!(*double_tapped.borrow(), 1);
    }

    #[test]
    fn right_button_fires_right_tapped_not_tapped() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");
        let right_tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_right_tapped");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Right, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Right, 10.0),
        );

        assert_eq!(*tapped.borrow(), 0);
        assert_eq!(*right_tapped.borrow(), 1);
    }
}
