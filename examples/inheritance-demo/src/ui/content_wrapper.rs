// Reproduces `docs/specs/dsl_spec.md` §3's own `ContentControl inherits Control` example against a
// real, external builtin (`elwindui-core`'s compiled `Control`, no local `TypeInfo`/`ComponentDef`
// visible to this crate's own `#[elwindui::component]` expansion at all) — the exact end-to-end
// shape Issue #90 found broken via `cargo build -p notepad`: `padding`'s bare same-name forward
// (`padding: padding`) panicked in `elwindui-codegen` because `resolve_effective_fields` had no
// base field list to recognize it against (see that function's `synthesize_external_base_fields`
// fallback). Named `ContentWrapper` rather than the spec's own `ContentControl` simply to avoid
// shadowing the real builtin of that name in this demo (dsl_spec.md §3 confirms a local
// `ContentControl` would be allowed to shadow it, but that would be confusing here).
// dsl_spec.md §3 spells this field's type `Rc<dyn UIElement>` — the pre-rename trait name from
// before the repo-wide `struct = bare ClassName` / `trait = ClassNameExt` swap; the real, currently
// compiling trait is `UIElementExt`. Unrelated to Issue #90 (out of this fix's scope), so this uses
// today's real name rather than the spec's stale one — `generate_view`'s `field.ty.contains("dyn
// UIElement")` check (`codegen.rs`) is a substring match, so `dyn UIElementExt` still satisfies it.
use elwindui::ui::UIElementExt;

#[elwindui::component(inherits Control)]
struct ContentWrapper {
    content: std::rc::Rc<dyn UIElementExt>,
    // `padding` is *not* redeclared here — it's auto-inherited from `Control` and forwarded below
    // by the bare same-name reference `padding: padding`, exactly as dsl_spec.md §3 documents.
    template: template_view! {
    // No `Control { .. }` wrapper is written — `view!`'s own body supplies Control's attributes
    // and its single authored visual/template root (dsl_spec.md §3's shape-composition case).
        padding: padding
        content
    },
}

#[elwindui::component]
impl ContentWrapper {}
