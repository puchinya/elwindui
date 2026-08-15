// Declaration order matters: `component_frontend::same_crate_components` is a registry each
// `#[elwindui::component]` expansion appends itself to, and a *later* module's `view!` can only
// resolve an *earlier* module's component as a sibling type (`examples/notepad/src/ui/mod.rs`'s
// own comment explains the mechanism in full). `labeled_panel` has no sibling dependencies;
// `loud_panel` inherits it (`inherits crate::ui::LabeledPanel`) and also references it as a plain
// view element; `inheritance_demo_window` uses both.
//
// Each `mod` below is kept on its own blank-line-separated line rather than a contiguous block:
// `cargo fmt`'s `reorder_modules` (on by default) alphabetizes any *contiguous* run of `mod` items,
// which previously scrambled this load-bearing order silently (Issue #139) — the blank lines match
// `elwindui-core/src/ui/mod.rs`'s own defense against the same hazard.
mod content_wrapper;

mod labeled_panel;

mod loud_panel;

mod inheritance_demo_window;

// Glob re-exports, not a named list: `#[class]` generates a companion
// `__elwindui_macros_of_LabeledPanel` alongside `LabeledPanel` itself — `loud_panel.rs`'s
// `inherits crate::ui::LabeledPanel` needs that companion reachable at this exact
// `crate::ui::LabeledPanel` path (Refs #25). A named re-export here (`pub use
// labeled_panel::LabeledPanel;`) would strand it and break the inheriting component.
pub use content_wrapper::*;
pub use inheritance_demo_window::InheritanceDemoWindow;
pub use labeled_panel::*;
pub use loud_panel::*;
