//! Issue #129: end-to-end proc-macro coverage for `#[environment(some_crate::name)]` — the
//! cross-crate counterpart to `environment_field.rs`. `elwindui_environment_key_fixture::
//! FixtureLocaleKey` (`crates/elwindui-environment-key-fixture`) is declared in a genuinely
//! different crate than this test, so this exercises the real
//! `elwindui_environment_key_fixture::__elwindui_environment_key_fixture_locale!` cross-crate
//! macro path (`docs/design/tools/environment_key_macro_design.md`), not just a same-crate
//! `#[environment(name)]` reference under a different file.
//!
//! The `#![allow(..)]` below is unrelated to Issue #129's own cross-crate mechanism (which
//! deliberately avoids ever needing it — see `codegen::environment_key_type`'s own doc comment):
//! it's the same pre-existing allow `environment_field.rs`/`environment_scope.rs` already carry,
//! required by `#[class]`'s own `__elwindui_props_*!`/`__elwindui_inherit_*!` cross-crate builtin
//! resolution (`TextBlock`/`VerticalLayout` are declared in a different crate, `elwindui-core`).

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentFieldCrossCrateView {
    #[environment(elwindui_environment_key_fixture::fixture_locale)]
    locale: String,

    body: view! {
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentFieldCrossCrateView {}

#[test]
fn resolves_a_cross_crate_environment_key_via_a_qualified_path() {
    elwindui::core::environment::application_environment()
        .set::<elwindui_environment_key_fixture::FixtureLocaleKey>("ja-JP".to_string());

    let view = EnvironmentFieldCrossCrateView::new();
    assert_eq!(view.locale(), "ja-JP");
}
