// `LoudPanel inherits LabeledPanel` — a *user-defined* base, written as the full crate-root-
// qualified path `crate::ui::LabeledPanel` (the public path `ui/mod.rs`'s glob re-export exposes,
// not the private `crate::ui::labeled_panel::LabeledPanel` submodule path) exactly as
// `docs/specs/dsl_spec.md` §3 now requires for a non-builtin base (Refs #25). `#[overrides]` +
// `base::label(..)` (Refs #23) is exercised end to end here for the first time — previously only
// asserted on generated-token text (`elwindui_codegen`'s own `user_base_inherits_tests`), never
// actually compiled or run.
use crate::ui::LabeledPanel;

#[elwindui::component(inherits crate::ui::LabeledPanel)]
struct LoudPanel {
    template: template_view!(|templated_parent: Self| { LabeledPanel {} }),
}

#[elwindui::component]
impl LoudPanel {
    #[overrides]
    fn label(&self) -> String {
        format!("{}!!!", base::label())
    }
}
