//! Issue #84: end-to-end proc-macro coverage for `#[elwindui::environment_key]` +
//! `#[environment(name)]` — unlike the codegen-level tests (`elwindui-codegen`'s
//! `environment_key_tests`, which only check the generated Rust *source text*), this file is a
//! real integration test: it must actually compile and run through `rustc`, exercising the whole
//! macro pipeline for real.
//!
//! `VerticalLayout`/`TextBlock` are virtual builtins (no native backend handle), so — unlike
//! `for_item_two_way.rs`'s AppKit-backed host — construction here needs no native main-thread
//! call and can run directly under the test harness.
//!
//! CI-6 of #80: ambient `EnvironmentContext::current()`/`.enter()` were removed; a generated
//! component's `mount()` now bridges with `elwindui::core::environment::application_environment()`
//! directly — a single, persistent, `thread_local!` singleton, not a fresh context per test.
//! `cargo test`'s default harness runs tests on a pooled set of threads, so each test below uses
//! its own dedicated `#[elwindui::environment_key]` type (never touched by any other test function
//! in this file, or elsewhere in the workspace) rather than sharing one key on
//! `application_environment()` — this makes isolation correct regardless of thread/scheduling
//! luck, with no serialization needed.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui::environment_key(
    name = environment_field_default_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct EnvironmentFieldDefaultLocale;

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentFieldDefaultView {
    #[environment(environment_field_default_locale)]
    locale: String,

    body: view! {
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentFieldDefaultView {}

#[test]
fn resolves_the_registered_default_when_nothing_overrides_it() {
    let view = EnvironmentFieldDefaultView::new();
    assert_eq!(view.locale(), "en-US");
}

#[elwindui::environment_key(
    name = environment_field_override_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct EnvironmentFieldOverrideLocale;

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentFieldOverrideView {
    #[environment(environment_field_override_locale)]
    locale: String,

    body: view! {
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentFieldOverrideView {}

#[test]
fn resolves_an_override_already_present_on_application_environment_at_construction() {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentFieldOverrideLocale>("ja-JP".to_string());

    let view = EnvironmentFieldOverrideView::new();
    assert_eq!(view.locale(), "ja-JP");
}

#[elwindui::environment_key(
    name = environment_field_live_update_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct EnvironmentFieldLiveUpdateLocale;

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentFieldLiveUpdateView {
    #[environment(environment_field_live_update_locale)]
    locale: String,

    body: view! {
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentFieldLiveUpdateView {}

#[test]
fn a_later_change_on_application_environment_live_updates_the_field() {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentFieldLiveUpdateLocale>("fr-FR".to_string());
    let view = EnvironmentFieldLiveUpdateView::new();
    assert_eq!(view.locale(), "fr-FR");

    // The component resolved and subscribed against `application_environment()` at mount
    // (`__mount_environment`, docs/design/runtime/component_lifecycle_design.md §4d) — a later
    // change on that same singleton must still reach it.
    elwindui::core::environment::application_environment()
        .set::<EnvironmentFieldLiveUpdateLocale>("de-DE".to_string());
    assert_eq!(view.locale(), "de-DE");
}

#[elwindui::environment_key(
    name = environment_field_unrelated_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct EnvironmentFieldUnrelatedLocale;

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentFieldUnrelatedView {
    #[environment(environment_field_unrelated_locale)]
    locale: String,

    body: view! {
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentFieldUnrelatedView {}

#[test]
fn a_change_on_a_disconnected_context_does_not_affect_a_view_mounted_against_application_environment()
 {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentFieldUnrelatedLocale>("it-IT".to_string());
    let view = EnvironmentFieldUnrelatedView::new();
    assert_eq!(view.locale(), "it-IT");

    // A freestanding `EnvironmentContext::root()` has no relationship to `application_environment()`
    // — mutating it must not reach a view that was mounted against the latter.
    let unrelated = elwindui::core::environment::EnvironmentContext::root();
    unrelated.set::<EnvironmentFieldUnrelatedLocale>("es-ES".to_string());
    assert_eq!(view.locale(), "it-IT");
}
