//! Theme-as-Preset-over-Environment (`docs/specs/theme_environment_spec.md` §3–§6, Issue #96).
//!
//! A Theme is not a second resolution mechanism alongside Environment ([`crate::environment`]) — it
//! is a batch of [`crate::environment::EnvironmentContext::set`] calls. See
//! `docs/design/runtime/theme_environment_design.md` (`## Theme`) for the full rationale, including
//! why there is no separate `EnvironmentOverrides` type and why applying a Theme is scoped to
//! [`crate::environment::EnvironmentContext::application_environment`] only (no per-Window override
//! in this iteration).

use crate::environment::EnvironmentContext;

/// Implemented by code generated from `#[elwindui::theme]`. Applying a Theme overrides whichever
/// Environment Keys its `#[theme(value = ..)]` fields target — `docs/specs/theme_environment_spec.md`
/// §3/§4.
pub trait Theme {
    /// Overrides this Theme's Environment values on `env`.
    fn apply(&self, env: &EnvironmentContext);
}
