//! Minimal end-to-end proof that `#[elwindui::component(inherits <path>)]` (a Rust attribute macro
//! `inherits` argument, see `elwindui_codegen::component_frontend`) can name a *user-defined*
//! component as its base, not just a builtin (`ContentControl`/`Window`/...) — the fix for Refs
//! #25. `LoudPanel` (`ui/loud_panel.rs`) inherits `LabeledPanel` (`ui/labeled_panel.rs`), written
//! as the full crate-root-qualified path `docs/specs/dsl_spec.md` §3 requires for a non-builtin
//! base. This is also the only place §3's `#[overridable]`/`#[overrides]`/`base::name(..)` method
//! inheritance (Refs #23) is exercised end to end through a user-defined `inherits` chain, rather
//! than just asserted on as generated-token text.

// Required in the crate root of anything using `#[elwindui_macros::class]` (which every
// `inherits`-carrying component becomes) — see docs/specs/macro_class_spec.md §10.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

mod ui;
use ui::{InheritanceDemoWindow, LabeledPanel, LoudPanel};

#[elwindui::main]
fn main() {
    let base = LabeledPanel::new();
    assert_eq!(base.label(), "panel");

    let derived = LoudPanel::new();
    assert_eq!(derived.label(), "panel!!!");
    println!(
        "ok: LabeledPanel::label()={:?} LoudPanel::label()={:?}",
        base.label(),
        derived.label()
    );

    let window = InheritanceDemoWindow::new();
    window.show();
}
