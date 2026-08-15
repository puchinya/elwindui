// Declaration order matters: `component_frontend::same_crate_components` is a registry each
// `#[elwindui::component]` expansion appends itself to, and a *later* module's `view!` can only
// resolve an *earlier* module's component as a sibling type — see
// `elwindui_codegen::component_frontend::sibling_component_modules`'s own doc comment. `RoundedPanel`
// has no sibling dependencies; `document_view` uses `RoundedPanel`; `notepad_window` uses
// `DocumentView` (and declares/uses its own sibling, `CustomCheckBox`).
//
// Each `mod` below is kept on its own blank-line-separated line rather than a contiguous block:
// `cargo fmt`'s `reorder_modules` (on by default) alphabetizes any *contiguous* run of `mod` items,
// which previously scrambled this load-bearing order silently (Issue #139) — the blank lines match
// `elwindui-core/src/ui/mod.rs`'s own defense against the same hazard.
mod rounded_panel;

mod document_view;

mod notepad_window;

pub use notepad_window::NotepadWindow;
