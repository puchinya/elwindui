//! T12: `new!` through a Cargo-renamed facade reaches the constructor ABI the facade re-exports
//! for a builtin written as a class-backed `#[component]`.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use ui_facade::core::ui::UIElementExt as _;

#[test]
fn new_through_a_renamed_facade_builds_a_core_builtin() {
    let layout = ui_facade::new!(ui_facade::ui::VerticalLayout(spacing: 8.0, margin: 2.0));
    assert_eq!(layout.margin(), 2.0);
}

/// A generated Component of another crate, built through the renamed facade's `new!`: its
/// constructor ABI reaches the application environment through the type itself.
#[test]
fn new_through_a_renamed_facade_builds_an_external_generated_component() {
    let component = ui_facade::new!(elwindui_external_component_fixture::ExternalRequiredBase(
        title: "x",
        count: 2
    ));
    use elwindui_external_component_fixture::ExternalRequiredBaseExt as _;
    assert_eq!(component.title(), "x");
}
