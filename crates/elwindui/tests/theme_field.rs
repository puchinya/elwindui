//! Issue #96: end-to-end proc-macro coverage for `#[elwindui::theme]` — unlike the codegen-level
//! tests (`elwindui-codegen`'s `theme_frontend` tests, which only check the generated Rust *source
//! text*), this file is a real integration test: it must actually compile and run through `rustc`,
//! exercising the whole macro pipeline plus Environment's own reactive machinery for real.
//!
//! Mirrors `environment_field.rs`'s shape/rationale — see that file's own doc comment.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::environment::EnvironmentContext;
use elwindui::core::theme::Theme;

#[elwindui::environment_key(
    name = theme_field_test_tint,
    value = i32,
    default = 0
)]
pub struct ThemeFieldTestTint;

#[elwindui::theme]
struct ThemeFieldTestOceanTheme {
    #[theme(value = 1)]
    theme_field_test_tint: i32,
}

#[elwindui::theme]
struct ThemeFieldTestSolarizedTheme {
    #[theme(value = 2)]
    theme_field_test_tint: i32,
}

#[elwindui::component(inherits VerticalLayout)]
struct ThemeFieldTestView {
    #[environment(theme_field_test_tint)]
    tint: i32,

    body: view! {
        TextBlock { text: format!("{tint}") }
    },
}

#[elwindui::component]
impl ThemeFieldTestView {}

#[test]
fn applying_a_theme_overrides_the_environment_value_it_targets() {
    let scoped = EnvironmentContext::root();
    ThemeFieldTestOceanTheme.apply(&scoped);
    assert_eq!(scoped.get::<ThemeFieldTestTint>(), 1);
}

#[test]
fn a_component_constructed_under_an_applied_theme_observes_its_value() {
    let scoped = EnvironmentContext::root();
    ThemeFieldTestOceanTheme.apply(&scoped);
    let _guard = scoped.enter();

    let view = ThemeFieldTestView::new();
    assert_eq!(view.tint(), 1);
}

#[test]
fn switching_to_a_different_theme_live_updates_an_already_constructed_component() {
    let scoped = EnvironmentContext::root();
    ThemeFieldTestOceanTheme.apply(&scoped);
    let view = {
        let _guard = scoped.enter();
        ThemeFieldTestView::new()
    };
    assert_eq!(view.tint(), 1);

    // Switching "theme" is applying a different Preset instance to the same context, after the
    // ambient guard already dropped — Environment's own per-key subscription (not anything
    // Theme-specific) is what reaches the already-constructed component.
    ThemeFieldTestSolarizedTheme.apply(&scoped);
    assert_eq!(view.tint(), 2);
}

#[test]
fn reapplying_the_same_theme_instance_is_a_no_op_observably() {
    let scoped = EnvironmentContext::root();
    ThemeFieldTestOceanTheme.apply(&scoped);
    let view = {
        let _guard = scoped.enter();
        ThemeFieldTestView::new()
    };
    assert_eq!(view.tint(), 1);

    ThemeFieldTestOceanTheme.apply(&scoped);
    assert_eq!(view.tint(), 1);
}
