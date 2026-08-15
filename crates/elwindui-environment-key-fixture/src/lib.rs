//! Cross-crate test fixture for Issue #129 (`#[environment(some_crate::name)]`/
//! `EnvironmentScope { some_crate::name: value }`).
//!
//! Declares one `pub` Environment Key so `crates/elwindui`'s integration tests
//! (`tests/environment_field_cross_crate.rs`, `tests/environment_scope_cross_crate.rs`) can
//! reference it from a *different* crate — proving the `__elwindui_environment_key_{name}!`
//! macro-export path actually crosses a real crate boundary, not just a module boundary within
//! one crate (unlike every other `environment_field.rs`/`environment_scope.rs` test, which
//! deliberately declares its Key in the same file it's consumed from). Not published; workspace-
//! internal only.

#[elwindui::environment_key(
    name = fixture_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct FixtureLocaleKey;
