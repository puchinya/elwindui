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
