// Declaration order matters: `component_frontend::same_crate_components` is a registry each
// `#[elwindui::component]` expansion appends itself to, and a *later* module's `view!` can only
// resolve an *earlier* module's component as a sibling type — see
// `elwindui_codegen::component_frontend::sibling_component_modules`'s own doc comment. `RoundedPanel`
// has no sibling dependencies; `document_view` uses `RoundedPanel`; `notepad_window` uses
// `DocumentView` (and declares/uses its own sibling, `CustomCheckBox`).
mod document_view;
mod notepad_window;
mod rounded_panel;

pub use notepad_window::NotepadWindow;
