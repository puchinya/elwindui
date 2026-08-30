//! Issue #129: `EnvironmentScope { some_crate::name: value }` — the cross-crate counterpart to
//! `environment_scope.rs`. `elwindui_environment_key_fixture::FixtureLocaleKey`
//! (`crates/elwindui-environment-key-fixture`) is declared in a genuinely different crate than
//! this test, exercising the same `Owner::field: value`-grammar reuse
//! (`docs/design/tools/environment_key_macro_design.md`) real `rustc` compiles and runs, not just
//! the codegen-level source-text checks.
//!
//! The `#![allow(..)]` below is unrelated to Issue #129's own cross-crate mechanism — see
//! `environment_field_cross_crate.rs`'s own module doc comment for why it's still needed here
//! (`#[class]`'s pre-existing cross-crate builtin resolution, not this file's own qualified keys).

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::RefCell;

thread_local! {
    static LOCALE: RefCell<String> = RefCell::new(String::new());
}

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeCrossCrateChild {
    #[environment(elwindui_environment_key_fixture::fixture_locale)]
    locale: String,

    template: template_view!(|templated_parent: Self| {
        on_mount {
            LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    }),
}

#[elwindui::component]
impl EnvironmentScopeCrossCrateChild {}

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentScopeCrossCrateParent {
    body: view! {
        EnvironmentScope {
            elwindui_environment_key_fixture::fixture_locale: "ko-KR",
            EnvironmentScopeCrossCrateChild {}
        }
    },
}

#[elwindui::component]
impl EnvironmentScopeCrossCrateParent {}

#[test]
fn override_reaches_a_child_through_a_qualified_cross_crate_key() {
    elwindui::core::environment::application_environment()
        .set::<elwindui_environment_key_fixture::FixtureLocaleKey>("en-US".to_string());
    LOCALE.with(|c| *c.borrow_mut() = String::new());

    let _parent = EnvironmentScopeCrossCrateParent::new();

    assert_eq!(LOCALE.with(|c| c.borrow().clone()), "ko-KR");
}
