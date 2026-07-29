//! Verifies the DSL **shape** layer (`#[dsl_prop]` -> `__elwindui_shape_{Name}!`) actually reaches a
//! *different* crate. This is the whole point of routing the shape through a `macro_rules!` instead
//! of a compiler-side table: `#[elwindui::component]` expands in the consumer's crate, and a
//! proc-macro can never read another crate's expansion results. An integration test is a separate
//! crate from `elwindui-core`, so it exercises exactly that boundary.

use std::cell::RefCell;

/// Stands in for a backend's real `Button`: the shape macro emits plain method syntax
/// (`$recv.set_text(..)`), so anything with the right setters satisfies it.
#[derive(Default)]
struct FakeButton {
    text: RefCell<String>,
    enabled: RefCell<Option<bool>>,
    /// Declared by `NativeControl`, one hop up the chain — not by `Button` itself.
    background: RefCell<Option<u32>>,
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
    fn set_background(&self, background: u32) {
        *self.background.borrow_mut() = Some(background);
    }
    fn set_margin(&self, margin: f32) {
        *self.margin.borrow_mut() = Some(margin);
    }
}

#[test]
fn shape_macro_sets_a_string_prop_through_a_reference() {
    let button = FakeButton::default();
    // `text: String` — the setter takes `&str` by convention, so the macro wraps the value.
    elwindui_core::__elwindui_shape_Button!(@set button, text, String::from("save"));
    assert_eq!(*button.text.borrow(), "save");
}

#[test]
fn shape_macro_sets_a_non_string_prop_by_value() {
    let button = FakeButton::default();
    elwindui_core::__elwindui_shape_Button!(@set button, enabled, false);
    assert_eq!(*button.enabled.borrow(), Some(false));
}

/// A property `Button` doesn't declare is forwarded, unexamined, to its own ancestor's shape macro
/// — the same "what isn't mine goes up the chain" muncher shape `#[overrides]` routing already uses.
#[test]
fn shape_macro_forwards_an_undeclared_prop_one_hop_up() {
    let button = FakeButton::default();
    // `background` belongs to `NativeControl`, not `Button`.
    elwindui_core::__elwindui_shape_Button!(@set button, background, 0x336699u32);
    assert_eq!(*button.background.borrow(), Some(0x336699));
}

/// Forwarding is not limited to one hop: `margin` is declared all the way up on `UIElement`, so this
/// traverses `Button` -> `NativeControl` -> `UIElement`.
#[test]
fn shape_macro_forwards_through_the_whole_ancestor_chain() {
    let button = FakeButton::default();
    elwindui_core::__elwindui_shape_Button!(@set button, margin, 4.0f32);
    assert_eq!(*button.margin.borrow(), Some(4.0));
}
