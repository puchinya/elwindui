//! CI-5 of #80 (docs/design/runtime/component_lifecycle_design.md §4d): `#[environment(name)]`
//! fields now resolve from `self.__mount_environment` (the `EnvironmentContext` `mount()` was
//! actually called with), not a second, independent ambient read in `construct()`. The legacy,
//! ambient-captured `__environment` field is gone; `recompute_<name>`/the live subscription both
//! read through `__mount_environment` instead.
//!
//! `environment_field.rs`/`theme_field.rs` already cover the observable resolve/live-update
//! behavior. This file adds the two things CI-5 specifically changed the *mechanism* of and that
//! aren't otherwise exercised: (a) a component's own environment field value is available to a
//! *nested user component* child's own constructor argument -- which only works if
//! `own_environment_resolve_stmts` genuinely runs before `child_construct_stmts` inside
//! `__build_view()` -- and (b) the live-update subscription still fires correctly now that it's
//! installed against `self.__mount_environment` instead of the deleted `__environment` field.
//!
//! CI-6 of #80: ambient `EnvironmentContext::current()`/`.enter()` were removed; `mount()` now
//! bridges with `elwindui::core::environment::application_environment()` (a single, persistent
//! singleton) directly -- each test below uses its own dedicated
//! `#[elwindui::environment_key]`/component set so the two tests in this file can't race on the
//! same key under `cargo test`'s pooled-thread harness.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui::environment_key(
    name = mount_time_resolution_nested_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct MountTimeResolutionNestedLocale;

#[elwindui::component(inherits ContentControl)]
struct MountTimeResolutionNestedChild {
    #[param]
    label: String,

    body: view! {
        TextBlock { text: label }
    },
}

#[elwindui::component]
impl MountTimeResolutionNestedChild {}

#[elwindui::component(inherits VerticalLayout)]
struct MountTimeResolutionNestedParent {
    #[environment(mount_time_resolution_nested_locale)]
    locale: String,

    body: view! {
        #[id("child")]
        let child = MountTimeResolutionNestedChild { label: locale };

        child
    },
}

#[elwindui::component]
impl MountTimeResolutionNestedParent {}

#[test]
fn own_environment_field_is_resolved_before_a_nested_child_component_is_constructed() {
    elwindui::core::environment::application_environment()
        .set::<MountTimeResolutionNestedLocale>("ja-JP".to_string());

    let parent = MountTimeResolutionNestedParent::new();

    // The parent's own `locale` field resolved correctly.
    assert_eq!(parent.locale(), "ja-JP");
    // And a *nested user component* child's own `#[param]` constructor argument -- built inside
    // `child_construct_stmts`, which only sees the correct value if `own_environment_resolve_stmts`
    // ran first -- observed the same, already-resolved value, not a stale/default one.
    assert_eq!(parent.child().label(), "ja-JP");
}

#[elwindui::environment_key(
    name = mount_time_resolution_live_update_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct MountTimeResolutionLiveUpdateLocale;

#[elwindui::component(inherits VerticalLayout)]
struct MountTimeResolutionLiveUpdateView {
    #[environment(mount_time_resolution_live_update_locale)]
    locale: String,

    body: view! {
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl MountTimeResolutionLiveUpdateView {}

#[test]
fn live_update_through_mount_environment_still_reaches_recompute_and_subscribers() {
    let application_environment = elwindui::core::environment::application_environment();
    application_environment.set::<MountTimeResolutionLiveUpdateLocale>("fr-FR".to_string());
    let view = MountTimeResolutionLiveUpdateView::new();
    assert_eq!(view.locale(), "fr-FR");

    // Changed after construction -- only reachable via the live subscription installed against
    // `self.__mount_environment` (CI-5), since nothing is ambient anymore here (CI-6).
    application_environment.set::<MountTimeResolutionLiveUpdateLocale>("de-DE".to_string());
    assert_eq!(view.locale(), "de-DE");
}
