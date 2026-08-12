//! Issue #84: end-to-end proc-macro coverage for `#[elwindui::environment_key]` +
//! `#[environment(name)]` — unlike the codegen-level tests (`elwindui-codegen`'s
//! `environment_key_tests`, which only check the generated Rust *source text*), this file is a
//! real integration test: it must actually compile and run through `rustc`, exercising the whole
//! macro pipeline for real.
//!
//! `VerticalLayout`/`TextBlock` are virtual builtins (no native backend handle), so — unlike
//! `for_item_two_way.rs`'s AppKit-backed host — construction here needs no native main-thread
//! call and can run directly under the test harness.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui::environment_key(
    name = environment_field_test_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct EnvironmentFieldTestLocale;

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentFieldTestView {
    #[environment(environment_field_test_locale)]
    locale: String,

    body: view! {
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentFieldTestView {}

#[test]
fn resolves_the_registered_default_when_nothing_overrides_it() {
    let _root = elwindui::core::environment::EnvironmentContext::root();
    let view = EnvironmentFieldTestView::new();
    assert_eq!(view.locale(), "en-US");
}

#[test]
fn resolves_an_ambient_override_present_at_construction() {
    let scoped = elwindui::core::environment::EnvironmentContext::root();
    scoped.set::<EnvironmentFieldTestLocale>("ja-JP".to_string());
    let _guard = scoped.enter();

    let view = EnvironmentFieldTestView::new();
    assert_eq!(view.locale(), "ja-JP");
}

#[test]
fn a_later_change_on_the_captured_context_live_updates_the_field() {
    let scoped = elwindui::core::environment::EnvironmentContext::root();
    scoped.set::<EnvironmentFieldTestLocale>("fr-FR".to_string());
    let view = {
        let _guard = scoped.enter();
        EnvironmentFieldTestView::new()
    };
    assert_eq!(view.locale(), "fr-FR");

    // The component captured `scoped` at construction (`__environment`, codegen.rs's
    // `environment_context_field_init`) and subscribed to its cell — a later change on that same
    // context, made after the ambient guard already dropped, must still reach it.
    scoped.set::<EnvironmentFieldTestLocale>("de-DE".to_string());
    assert_eq!(view.locale(), "de-DE");
}

#[test]
fn a_change_on_an_unrelated_context_does_not_affect_this_view() {
    let scoped = elwindui::core::environment::EnvironmentContext::root();
    scoped.set::<EnvironmentFieldTestLocale>("it-IT".to_string());
    let view = {
        let _guard = scoped.enter();
        EnvironmentFieldTestView::new()
    };
    assert_eq!(view.locale(), "it-IT");

    let unrelated = elwindui::core::environment::EnvironmentContext::root();
    unrelated.set::<EnvironmentFieldTestLocale>("es-ES".to_string());
    assert_eq!(view.locale(), "it-IT");
}
