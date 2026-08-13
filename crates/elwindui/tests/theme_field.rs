//! Issue #96: end-to-end proc-macro coverage for `#[elwindui::theme]` — unlike the codegen-level
//! tests (`elwindui-codegen`'s `theme_frontend` tests, which only check the generated Rust *source
//! text*), this file is a real integration test: it must actually compile and run through `rustc`,
//! exercising the whole macro pipeline plus Environment's own reactive machinery for real.
//!
//! Mirrors `environment_field.rs`'s shape/rationale — see that file's own doc comment. CI-6 of #80:
//! any test that constructs a component now applies its theme to
//! `elwindui::core::environment::application_environment()` (a single, persistent singleton)
//! *before* constructing, instead of `.enter()`-ing an ambient `EnvironmentContext::root()` — each
//! such test uses its own dedicated `#[elwindui::environment_key]`/`#[elwindui::theme]`/component
//! set so no two test functions race on the same key. A test that only exercises `Theme::apply`
//! against a plain, freestanding `EnvironmentContext::root()` (never touching
//! `application_environment()`) needs no such isolation and is unchanged.

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

#[test]
fn applying_a_theme_overrides_the_environment_value_it_targets() {
    let scoped = EnvironmentContext::root();
    ThemeFieldTestOceanTheme.apply(&scoped);
    assert_eq!(scoped.get::<ThemeFieldTestTint>(), 1);
}

#[elwindui::environment_key(
    name = theme_field_construct_tint,
    value = i32,
    default = 0
)]
pub struct ThemeFieldConstructTint;

#[elwindui::theme]
struct ThemeFieldConstructOceanTheme {
    #[theme(value = 1)]
    theme_field_construct_tint: i32,
}

#[elwindui::component(inherits VerticalLayout)]
struct ThemeFieldConstructView {
    #[environment(theme_field_construct_tint)]
    tint: i32,

    body: view! {
        TextBlock { text: format!("{tint}") }
    },
}

#[elwindui::component]
impl ThemeFieldConstructView {}

#[test]
fn a_component_constructed_under_an_applied_theme_observes_its_value() {
    ThemeFieldConstructOceanTheme.apply(&elwindui::core::environment::application_environment());

    let view = ThemeFieldConstructView::new();
    assert_eq!(view.tint(), 1);
}

#[elwindui::environment_key(
    name = theme_field_switch_tint,
    value = i32,
    default = 0
)]
pub struct ThemeFieldSwitchTint;

#[elwindui::theme]
struct ThemeFieldSwitchOceanTheme {
    #[theme(value = 1)]
    theme_field_switch_tint: i32,
}

#[elwindui::theme]
struct ThemeFieldSwitchSolarizedTheme {
    #[theme(value = 2)]
    theme_field_switch_tint: i32,
}

#[elwindui::component(inherits VerticalLayout)]
struct ThemeFieldSwitchView {
    #[environment(theme_field_switch_tint)]
    tint: i32,

    body: view! {
        TextBlock { text: format!("{tint}") }
    },
}

#[elwindui::component]
impl ThemeFieldSwitchView {}

#[test]
fn switching_to_a_different_theme_live_updates_an_already_constructed_component() {
    let application_environment = elwindui::core::environment::application_environment();
    ThemeFieldSwitchOceanTheme.apply(&application_environment);
    let view = ThemeFieldSwitchView::new();
    assert_eq!(view.tint(), 1);

    // Switching "theme" is applying a different Preset instance to the same context — Environment's
    // own per-key subscription (not anything Theme-specific) is what reaches the already-constructed
    // component.
    ThemeFieldSwitchSolarizedTheme.apply(&application_environment);
    assert_eq!(view.tint(), 2);
}

#[elwindui::environment_key(
    name = theme_field_reapply_tint,
    value = i32,
    default = 0
)]
pub struct ThemeFieldReapplyTint;

#[elwindui::theme]
struct ThemeFieldReapplyOceanTheme {
    #[theme(value = 1)]
    theme_field_reapply_tint: i32,
}

#[elwindui::component(inherits VerticalLayout)]
struct ThemeFieldReapplyView {
    #[environment(theme_field_reapply_tint)]
    tint: i32,

    body: view! {
        TextBlock { text: format!("{tint}") }
    },
}

#[elwindui::component]
impl ThemeFieldReapplyView {}

#[test]
fn reapplying_the_same_theme_instance_is_a_no_op_observably() {
    let application_environment = elwindui::core::environment::application_environment();
    ThemeFieldReapplyOceanTheme.apply(&application_environment);
    let view = ThemeFieldReapplyView::new();
    assert_eq!(view.tint(), 1);

    ThemeFieldReapplyOceanTheme.apply(&application_environment);
    assert_eq!(view.tint(), 1);
}
