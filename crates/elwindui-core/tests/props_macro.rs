//! Verifies the property layer (`#[prop(..)]` -> `__elwindui_props_{Name}!`) actually reaches a
//! *different* crate. This is the whole point of routing the shape through a `macro_rules!` instead
//! of a compiler-side table: `#[elwindui::component]` expands in the consumer's crate, and a
//! proc-macro can never read another crate's expansion results. An integration test is a separate
//! crate from `elwindui-core`, so it exercises exactly that boundary.

use elwindui_core::graphics::{Brush, Color};
use std::cell::RefCell;

/// Stands in for a backend's real `Button`: the props macro emits plain method syntax
/// (`$recv.set_text(..)`), so anything with the right setters satisfies it.
#[derive(Default)]
struct FakeButton {
    text: RefCell<String>,
    enabled: RefCell<Option<bool>>,
    /// Declared by `NativeControl`, one hop up the chain — not by `Button` itself. Real type
    /// (`Option<Brush>`, matching `NativeControl::set_background`'s own signature), not a stand-in
    /// — this doubles as the cross-crate check that `wrap_prop_value`'s `.into()`/`Some(..)`
    /// wrapping for Brush/Color-typed props is generated correctly.
    background: RefCell<Option<Brush>>,
    /// Declared by `UIElement`, at the very top of the chain.
    margin: RefCell<Option<f32>>,
}

impl FakeButton {
    fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
    }
    fn set_enabled(&self, enabled: bool) {
        *self.enabled.borrow_mut() = Some(enabled);
    }
    fn set_background(&self, background: Option<Brush>) {
        *self.background.borrow_mut() = background;
    }
    fn set_margin(&self, margin: f32) {
        *self.margin.borrow_mut() = Some(margin);
    }
}

#[test]
fn props_macro_sets_a_string_prop_through_a_reference() {
    let button = FakeButton::default();
    // `text: String` — the setter takes `&str` by convention, so the macro wraps the value.
    elwindui_core::__elwindui_props_Button!(@set button, text, String::from("save"));
    assert_eq!(*button.text.borrow(), "save");
}

#[test]
fn props_macro_sets_a_non_string_prop_by_value() {
    let button = FakeButton::default();
    elwindui_core::__elwindui_props_Button!(@set button, enabled, false);
    assert_eq!(*button.enabled.borrow(), Some(false));
}

/// A property `Button` doesn't declare is forwarded, unexamined, to its own ancestor's props macro
/// — the same "what isn't mine goes up the chain" muncher shape `#[overrides]` routing already uses.
/// Also exercises `wrap_prop_value`'s `Option<Brush>` handling: a hex-string DSL literal reaches the
/// setter via `Brush`'s own `From<&str>`, `Some`-wrapped since the DSL convention writes the inner
/// value, never `Option` itself.
#[test]
fn props_macro_forwards_an_undeclared_prop_one_hop_up() {
    let button = FakeButton::default();
    // `background` belongs to `NativeControl`, not `Button`.
    elwindui_core::__elwindui_props_Button!(@set button, background, "#336699");
    assert_eq!(
        *button.background.borrow(),
        Some(Brush::Solid(Color::parse_hex("#336699").unwrap()))
    );
}

/// `.into()` is a no-op (std's reflexive `From<T> for T`) when the DSL value is already the target
/// type, not just when it's a hex-string literal.
#[test]
fn props_macro_passes_an_already_typed_brush_through_into() {
    let button = FakeButton::default();
    let brush = Brush::Solid(Color::parse_hex("#abcdef").unwrap());
    elwindui_core::__elwindui_props_Button!(@set button, background, brush.clone());
    assert_eq!(*button.background.borrow(), Some(brush));
}

/// Forwarding is not limited to one hop: `margin` is declared all the way up on `UIElement`, so this
/// traverses `Button` -> `NativeControl` -> `UIElement`.
#[test]
fn props_macro_forwards_through_the_whole_ancestor_chain() {
    let button = FakeButton::default();
    elwindui_core::__elwindui_props_Button!(@set button, margin, 4.0f32);
    assert_eq!(*button.margin.borrow(), Some(4.0));
}

// --- `@children`: attaching bare nested child elements ------------------------------------------

/// Stands in for a single-slot content host (`Window`/`ContentControl`/`TabViewItem`, whose content
/// property is an `Rc<dyn ..>`).
#[derive(Default)]
struct FakeWindow {
    content: RefCell<Option<&'static str>>,
}

impl FakeWindow {
    fn set_content(&self, content: &'static str) {
        *self.content.borrow_mut() = Some(content);
    }
}

/// Stands in for a live-collection host. `VerticalLayout` designates `children` via
/// `#[content(children)]` but does *not* declare the property — `Layout` does — so this exercises
/// the two-hop path: the designation resolves locally, the attach shape one class up.
#[derive(Default)]
struct FakeLayout {
    added: RefCell<Vec<&'static str>>,
}

struct FakeCollection<'a>(&'a FakeLayout);

impl FakeLayout {
    fn children(&self) -> FakeCollection<'_> {
        FakeCollection(self)
    }
}

impl FakeCollection<'_> {
    fn add(&self, child: &'static str) {
        self.0.added.borrow_mut().push(child);
    }
}

#[test]
fn children_go_into_a_single_slot_through_its_setter() {
    let window = FakeWindow::default();
    elwindui_core::__elwindui_props_Window!(@children window, ["body"]);
    assert_eq!(*window.content.borrow(), Some("body"));
}

#[test]
fn children_are_appended_to_a_collection_declared_by_an_ancestor() {
    let layout = FakeLayout::default();
    // `VerticalLayout` designates `children`; `Layout` declares it as a `UIElementCollection`, so
    // the attach shape is the `.children().add(..)` accessor loop, not a bulk setter.
    elwindui_core::__elwindui_props_VerticalLayout!(@children layout, ["a", "b"]);
    assert_eq!(*layout.added.borrow(), vec!["a", "b"]);
}

/// The third attach shape: a `Vec<T>` content property (`TabView`'s `children`) is replaced
/// wholesale through its setter, rather than appended to through an accessor.
#[derive(Default)]
struct FakeTabView {
    children: RefCell<Vec<&'static str>>,
}

impl FakeTabView {
    fn set_children(&self, children: Vec<&'static str>) {
        *self.children.borrow_mut() = children;
    }
}

#[test]
fn children_of_a_vec_property_are_set_in_bulk() {
    let tabs = FakeTabView::default();
    elwindui_core::__elwindui_props_TabView!(@children tabs, ["one", "two", "three"]);
    assert_eq!(*tabs.children.borrow(), vec!["one", "two", "three"]);
}

// --- `@routed`: registering a #[routed] callback instead of assigning it ------------------------
//
// `register_routed_handler` is a default `UIElementExt` method (`self.as_ui_element()...`), so it
// needs a *real* `UIElementExt` implementor to call against — a stand-in struct would have to
// reimplement the whole trait. `VerticalLayout` is a genuine, already-existing one; which concrete
// type it is doesn't matter here, only that it implements `UIElementExt`.

#[test]
fn routed_registers_a_zero_parameter_callback_declared_on_the_target_itself() {
    use elwindui_core::ui::UIElementExt;
    let widget = elwindui_core::ui::VerticalLayout::new();
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let fired2 = fired.clone();
    // `Button.on_click: fn()` -- declared directly on Button, zero parameters.
    elwindui_core::__elwindui_props_Button!(
        @routed widget, on_click,
        Box::new(move |_payload, _args: &elwindui_core::input::RoutedEventArgs| {
            fired2.set(true);
        })
    );
    let node: std::rc::Rc<dyn UIElementExt> = widget.clone();
    elwindui_core::ui::dispatch_routed(
        &node,
        "on_click",
        &(),
        &elwindui_core::input::RoutedEventArgs::default(),
    );
    assert!(fired.get());
}

#[test]
fn routed_forwards_a_one_parameter_callback_declared_two_hops_up() {
    use elwindui_core::ui::UIElementExt;
    let widget = elwindui_core::ui::VerticalLayout::new();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let seen2 = seen.clone();
    // `on_key_down: fn(KeyEventArgs)` is declared on `UIElement`, two hops above `Button`
    // (`Button` -> `NativeControl` -> `UIElement`) -- not on `Button` itself.
    elwindui_core::__elwindui_props_Button!(
        @routed widget, on_key_down,
        Box::new(move |payload, _args: &elwindui_core::input::RoutedEventArgs| {
            *seen2.borrow_mut() = Some(*payload);
        })
    );
    let args = elwindui_core::input::KeyEventArgs {
        key: elwindui_core::input::Key::Enter,
        modifiers: elwindui_core::input::KeyModifiers::default(),
        is_repeat: false,
    };
    let node: std::rc::Rc<dyn UIElementExt> = widget.clone();
    elwindui_core::ui::dispatch_routed(
        &node,
        "on_key_down",
        &args,
        &elwindui_core::input::RoutedEventArgs::default(),
    );
    assert_eq!(*seen.borrow(), Some(args));
}

// --- `@attached_set`: `Owner::field: value` attached properties ---------------------------------
//
// Unlike `@set`/`@routed`/`@children`, no ancestor chain to walk: the DSL syntax always names the
// owning class explicitly (`Grid::row`), so `elwindui-codegen` calls straight into `Grid`'s own
// props macro.

// --- `@set` on a `#[routed]` property: bare closure in, adapter built by the declaration --------
//
// `elwindui-codegen`'s whole point in having this: for an *external* element it has no `TypeInfo`
// for at all, it can call `@set` uniformly for every attribute the DSL wrote — whether it's a plain
// value or a `#[routed]` callback — without ever needing to know which. It builds a bare
// closure/callable matching the DSL's own written arity either way; the declaration (not the caller)
// decides whether that becomes a direct setter call or a routed registration.

#[test]
fn set_on_a_routed_property_accepts_a_bare_zero_parameter_closure() {
    use elwindui_core::ui::UIElementExt;
    let widget = elwindui_core::ui::VerticalLayout::new();
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let fired2 = fired.clone();
    // No `Box::new`, no `&RoutedEventArgs` parameter -- exactly the plain closure `elwindui-codegen`
    // already builds today for `Button.on_click: fn()`'s own DSL syntax (`on_click: || { .. }`).
    elwindui_core::__elwindui_props_Button!(@set widget, on_click, move || { fired2.set(true); });
    let node: std::rc::Rc<dyn UIElementExt> = widget.clone();
    elwindui_core::ui::dispatch_routed(
        &node,
        "on_click",
        &(),
        &elwindui_core::input::RoutedEventArgs::default(),
    );
    assert!(fired.get());
}

#[test]
fn set_on_a_routed_property_accepts_a_bare_one_parameter_closure() {
    use elwindui_core::ui::UIElementExt;
    let widget = elwindui_core::ui::VerticalLayout::new();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let seen2 = seen.clone();
    elwindui_core::__elwindui_props_Button!(@set widget, on_key_down, move |e| { *seen2.borrow_mut() = Some(e); });
    let args = elwindui_core::input::KeyEventArgs {
        key: elwindui_core::input::Key::Escape,
        modifiers: elwindui_core::input::KeyModifiers::default(),
        is_repeat: true,
    };
    let node: std::rc::Rc<dyn UIElementExt> = widget.clone();
    elwindui_core::ui::dispatch_routed(
        &node,
        "on_key_down",
        &args,
        &elwindui_core::input::RoutedEventArgs::default(),
    );
    assert_eq!(*seen.borrow(), Some(args));
}

#[test]
fn attached_set_stores_a_value_readable_back_through_get_attached() {
    use elwindui_core::ui::UIElementExt;
    let widget = elwindui_core::ui::VerticalLayout::new();
    // `Grid::row: i32` -- the turbofish on `set_attached::<i32>` comes from `#[prop(attached, row:
    // i32 = 0)]`'s own declared type, not anything elwindui-codegen still knows.
    elwindui_core::__elwindui_props_Grid!(@attached_set row, widget, 2);
    assert_eq!(
        widget.as_ui_element().get_attached::<i32>("Grid", "row", 0),
        2
    );
}

