pub mod ast;
pub mod attr_frontend;
pub mod codegen;
pub mod component_frontend;
pub mod parser;
pub mod theme_frontend;
mod text_style;
pub mod validate;

/// Test-only stand-in for the old, workspace-wide builtin shape source and `builtin_modules()` (removed —
/// see 05d4861/29ced3d/c916322/d255f31/36292fb, `docs/status/implementation_status.md`, Refs #14):
/// every real builtin's shape now lives as `#[elwindui_macros::class]` DSL attributes on its actual
/// `elwindui-core`/`elwindui-backend-*` declaration, propagated to a consumer crate via the
/// `__elwindui_shape_*!` macro chain (`elwindui-macros::class::build_props_macro`) rather than a
/// parsed text `Module` — so production code (`generate_component_from_item_struct`,
/// `compile_dir_impl`) no longer chains a builtin `Module` in at all, relying entirely on
/// `codegen::emit_external_construction`'s "no local `TypeInfo`, construct via `elwindui::ui::{Name}`
/// and the shape macro's `@set`/`@clear`/`@children` protocol" path (validated end-to-end by
/// temporarily emptying this exact file and confirming every real example still builds and runs).
///
/// What that production path structurally *can't* do is compiler-side validation that needs an
/// actual field list (`is_abstract`/`#[sealed]`/required-attribute completeness/`#[routed]`-ness) —
/// a real Rust `type` has no equivalent a proc-macro can read across a crate boundary, so those
/// checks silently no-op for an external reference (`check_element_value`/`check_shortcut_attrs`/
/// `validate_inherits`'s own doc comments). That's an acceptable trade for *production* code (a
/// wrong reference still fails to compile, just later, via `elwindui::ui::{Name}::new()`/its shape
/// macro directly) — but several of `codegen.rs`/`validate.rs`'s own unit tests exist specifically to
/// exercise those richer checks, and unlike production they call `validate::validate`/
/// `codegen::generate_module` directly (no later rustc pass to fall back on). This file — a private
/// copy of the old real shape source, now allowed to drift out of sync with the real builtins without
/// consequence, since nothing production-facing reads it — gives those tests real `TypeInfo` to
/// check against again, exactly as `builtin_modules()` used to for everyone. It carries a `.txt`
/// extension rather than the DSL text form's old one, so that no source file in the repo claims to
/// be a compilable DSL module. Its content is still the same hand-written DSL text
/// `parser::parse_module` (test-only) parses; only the file extension changed.
#[cfg(test)]
const TEST_BUILTIN_SHAPE_SOURCE: &str = include_str!("testdata/builtins_dsl_text.txt");

/// Test-only counterpart to the removed `builtin_modules()` — see `TEST_BUILTIN_SHAPE_SOURCE`'s own
/// doc comment. `pub(crate)` (not `pub`): only `codegen.rs`/`validate.rs`/`component_frontend.rs`'s
/// own `#[cfg(test)] mod tests` blocks call this, never production code.
#[cfg(test)]
pub(crate) fn test_builtin_modules() -> Vec<ast::Module> {
    // `parse_module` always defaults a freshly-parsed module's `path` to `[]` already.
    let mut module = parser::parse_module(TEST_BUILTIN_SHAPE_SOURCE).unwrap_or_else(|e| {
        panic!("failed to parse test builtin shapes: {e}\n---\n{TEST_BUILTIN_SHAPE_SOURCE}")
    });
    // Marks every component parsed from here as eligible for `#[embedded]` — see
    // `ast::Module::is_builtin`'s doc comment and `validate::validate`'s check.
    module.is_builtin = true;
    vec![module]
}

/// The attribute-macro counterpart to `generate_component_from_item_struct`: takes a
/// `#[elwindui::viewmodel] mod foo { struct Foo { ... } impl Foo { ... } }` (already parsed as a `syn::ItemMod` by the
/// `elwindui-macros` proc-macro), builds the same `ViewModelDef` AST `parser.rs` would from
/// equivalent DSL text (see `attr_frontend`), and feeds it through `generate_module` (not
/// `generate_viewmodel` directly — `generate_module` is also what conditionally emits the
/// `__elwindui_block_on_ready` helper an async `#[command]` needs, and there's no reason to
/// duplicate that check here).
pub fn generate_viewmodel_from_item_mod(
    item_mod: &syn::ItemMod,
) -> Result<proc_macro2::TokenStream, String> {
    let def = attr_frontend::viewmodel_def_from_item_mod(item_mod)?;
    let name = def.name.clone();
    // A single macro invocation has no directory of sibling modules to cross-reference (`use`
    // resolution is moot with only one module), so the exact real path doesn't matter here — `[]`
    // (crate root) is as good as any.
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: vec![ast::Item::ViewModel(def)],
        ..Default::default()
    };
    validate::validate(std::slice::from_ref(&module)).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(std::slice::from_ref(&module));
    let generated = codegen::generate_module(&module, &table);
    component_frontend::register_same_crate_viewmodel(&name, item_mod);
    Ok(generated)
}

/// `#[elwindui::dsl_enum] enum Name { A, B, C }` — the opt-in that makes a plain Rust `enum`
/// visible to `validate::validate`'s `match`-exhaustiveness checking the same way the DSL's own
/// `enum Name { .. }` syntax always was (§14's user-enum rule; see
/// `component_frontend::same_crate_enums`'s own doc comment for why an opt-in is needed at all —
/// unlike a `#[elwindui::component]` struct or `#[elwindui::viewmodel]` mod, nothing about a bare
/// `enum` item marks it as DSL-relevant to any proc-macro). Transparent passthrough: the enum body
/// is emitted completely unchanged (it's real Rust, matched with real Rust `match`/`if let`) — the
/// only effect is registering it via `component_frontend::register_same_crate_enum` so a later
/// same-crate `#[elwindui::component]`'s `view!` can be checked against it. Same
/// declared-before-use ordering constraint as the component/viewmodel registries.
pub fn generate_dsl_enum_from_item_enum(
    item_enum: &syn::ItemEnum,
) -> Result<proc_macro2::TokenStream, String> {
    let name = item_enum.ident.to_string();
    // Built purely to validate the enum shape up front (bare unit variants only) — the real
    // `EnumDef` a `view!` elsewhere gets checked against is rebuilt fresh, from the registered
    // source text, by `sibling_enum_modules` itself.
    component_frontend::enum_def_from_item_enum(item_enum)?;
    component_frontend::register_same_crate_enum(&name, item_enum);
    Ok(quote::quote! { #item_enum })
}

/// The attribute-macro counterpart for `component`/`view` (the struct+`view!` frontend, successor
/// to the removed `elwindui::component!` bang macro): takes an already-parsed
/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` (`base`
/// from the attribute's own `inherits Base` argument, `item_struct` parsed by the
/// `elwindui-macros` proc-macro) and builds the matching `ComponentDef`/`ViewDef` pair (see
/// `component_frontend`). Unlike `generate_viewmodel_from_item_mod`, this also chains in
/// `component_frontend::sibling_component_modules()`/`sibling_viewmodel_modules()`/
/// `sibling_enum_modules()`, so a `view!` can reference an *earlier* same-crate
/// `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` as a plain element
/// type, `bind!`/field target, or `match`/`if let` subject (each attribute-macro invocation
/// otherwise only ever sees its own single annotated item — see
/// `component_frontend::same_crate_components`'s own doc comment for the full mechanism and its
/// declaration-order requirement). A `view!` body routinely references
/// `Window`/`VerticalLayout`/etc. too, but those resolve with no `Module` chained in for them at all —
/// see `TEST_BUILTIN_SHAPE_SOURCE`'s own doc comment on why, and
/// `codegen::emit_external_construction`.
pub fn generate_component_from_item_struct(
    base: Option<String>,
    item_struct: &syn::ItemStruct,
) -> Result<proc_macro2::TokenStream, String> {
    // Shape errors (a malformed `view!`, a bad field attribute, ...) are reported here, against the
    // struct that actually contains them, rather than being deferred to the `impl` half.
    let (component_def, view_def) =
        component_frontend::component_and_view_from_item_struct(base.clone(), item_struct)?;
    let name = component_def.name.clone();
    // Validate here as well as in the `impl` half. Everything except `#[overrides]`-vs-base method
    // checking is already decidable from the struct alone (a non-exhaustive `match`, a typo'd
    // `vm.field`, a `#[bindable]` field whose type isn't a viewmodel, ...), and a diagnostic is far
    // more useful pointing at the struct that contains the mistake than at the `impl` below it.
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: component_frontend::component_module_items(component_def, view_def),
        allows_external_builtins: true,
        ..Default::default()
    };
    let all_modules: Vec<_> = std::iter::once(module)
        .chain(component_frontend::sibling_component_modules(&name))
        .chain(component_frontend::sibling_viewmodel_modules())
        .chain(component_frontend::sibling_enum_modules())
        .collect();
    validate::validate(&all_modules).map_err(|errors| errors.join("\n"))?;
    component_frontend::register_same_crate_component(&name, base.as_deref(), item_struct);
    // Emits nothing on purpose: the paired `#[elwindui::component] impl Name { .. }` generates the
    // whole type. This mirrors `#[elwindui_macros::class]` exactly — there too the `struct` half
    // only stashes what the `impl` half needs (`store_class_args`/`load_class_args`), and the
    // `impl` half is what emits the trait, the trait impl and `new()`. Components need the same
    // split because a `#[overridable]`/`#[overrides]` method body has nowhere to live on a bare
    // `struct`, and the generated type can only be emitted once — so it has to be emitted by
    // whichever half comes last, which is the `impl`.
    Ok(proc_macro2::TokenStream::new())
}

/// The generating half of the pair: `#[elwindui::component] impl Name { .. }`, whose `struct`
/// counterpart has already registered itself (see `generate_component_from_item_struct`). This is
/// what actually emits the component — the struct, its `#[elwindui_macros::class]` declaration and
/// paired class `impl`, `new()`, the accessors, the resync machinery, and §3's
/// `#[overridable]`/`#[overrides]` methods (see `component_frontend::methods_from_item_impl` for
/// the accepted method shape, and `docs/specs/dsl_spec.md` §3).
///
/// The `impl` block is required even when it declares no methods: it is the only place the type can
/// be emitted from, because a `#[overrides]` body can only be known once the `impl` is in hand and
/// the generated type must be emitted exactly once. Mirrors `#[elwindui_macros::class]`'s own
/// `struct`-stores/`impl`-emits split.
///
/// An `#[overrides]` method also gets its base's original body kept as a private `__base_<name>`
/// shadow (`codegen::resolve_effective_methods`), since `base::<name>(..)` in the override body is
/// rewritten to `self.__base_<name>(..)`.
pub fn generate_component_from_item_impl(
    item_impl: &syn::ItemImpl,
) -> Result<proc_macro2::TokenStream, String> {
    let (name, methods) = component_frontend::methods_from_item_impl(item_impl)?;
    let Some((mut component_def, view_def)) = component_frontend::registered_component_parts(&name)
    else {
        return Err(format!(
            "{name}: no `#[elwindui::component] struct {name} {{ .. }}` was expanded before this \
             `impl` block — declare the struct first"
        ));
    };
    component_def.methods = methods;
    let base = component_def.base.clone();
    let base_path = component_def.base_path.clone();
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: component_frontend::component_module_items(component_def, view_def),
        allows_external_builtins: true,
        ..Default::default()
    };
    let all_modules: Vec<_> = std::iter::once(module.clone())
        .chain(component_frontend::sibling_component_modules(&name))
        .chain(component_frontend::sibling_viewmodel_modules())
        .chain(component_frontend::sibling_enum_modules())
        .collect();
    validate::validate(&all_modules).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(&all_modules);
    // A bare, non-builtin `inherits <Base>` compiles here (unlike `validate::validate_inherits`,
    // shared with the DSL text frontend, which has no qualified-path escape hatch to require at
    // all — see `ComponentDef::base_path`'s own doc comment) but is *guaranteed* to fail later,
    // confusingly, once the `#[elwindui::class(inherits = ..)]` this emits is itself expanded
    // (`elwindui_macros::class::validate_fully_qualified_path`). Catching it here, with the base's
    // own DSL name in hand, gives a much more actionable diagnostic (Refs #25).
    if let Some(base) = &base {
        let base_is_builtin = table
            .resolve(&module, base)
            .is_none_or(|info| info.is_builtin);
        if !base_is_builtin && base_path.is_none() {
            return Err(format!(
                "{name}: inherits `{base}`, but `{base}` is a user-defined component — write a \
                 full crate-root-qualified path instead of a bare name (e.g. `inherits \
                 crate::ui::{base}`). Also make sure the module exposing `{base}` re-exports it \
                 with a glob (`pub use some_module::*;`), not a named list — #[class] generates a \
                 companion `__elwindui_macros_of_{base}` alongside `{base}` itself that a named \
                 re-export would strand (docs/specs/dsl_spec.md §3)."
            ));
        }
    }
    let generated = codegen::generate_module(&module, &table);
    component_frontend::register_same_crate_component_methods(&name, item_impl);
    Ok(generated)
}

/// Phase 4 (`docs/status/implementation_status.md`): exercises `#[elwindui::dsl_enum]` end to
/// end through the exact same path `generate_component_from_item_struct` uses in production
/// (register the enum via `generate_dsl_enum_from_item_enum`, then chain it in via
/// `component_frontend::sibling_enum_modules()` while building a sibling component) — confirming a
/// `match` in a Rust-macro-path `view!` gets real exhaustiveness checking against a same-crate
/// `#[elwindui::dsl_enum]`, the gap `component_frontend::same_crate_enums`'s own doc comment
/// describes. Names are unique per test (`DslEnumTest*`) — `same_crate_enums`/
/// `same_crate_components` are process-global statics keyed by `compiling_crate_key()`, which is
/// constant across every test in this one crate's test binary, so two tests reusing the same enum/
/// component name would leak state into each other (same constraint `component_frontend.rs`'s own
/// tests already live with).
#[cfg(test)]
mod dsl_enum_tests {
    use super::*;

    fn register_enum(src: &str) {
        let item_enum: syn::ItemEnum = syn::parse_str(src).expect("enum should parse");
        generate_dsl_enum_from_item_enum(&item_enum).expect("dsl_enum generation should succeed");
    }

    #[test]
    fn exhaustive_match_against_sibling_dsl_enum_validates() {
        register_enum("enum DslEnumTestStatusA { Loading, Ready }");
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct DslEnumTestScreenA {
                #[prop]
                status: DslEnumTestStatusA,
                body: view! {
                    VerticalLayout {
                        match status {
                            DslEnumTestStatusA::Loading => TextBlock { text: "loading" },
                            DslEnumTestStatusA::Ready => TextBlock { text: "ready" },
                        }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let result = generate_component_from_item_struct(None, &item_struct);
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    #[test]
    fn non_exhaustive_match_against_sibling_dsl_enum_is_rejected() {
        register_enum("enum DslEnumTestStatusB { Loading, Ready }");
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct DslEnumTestScreenB {
                #[prop]
                status: DslEnumTestStatusB,
                body: view! {
                    VerticalLayout {
                        match status {
                            DslEnumTestStatusB::Loading => TextBlock { text: "loading" },
                        }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("non-exhaustive match should be rejected");
        assert!(
            err.contains("not exhaustive") && err.contains("Ready"),
            "error should name the missing variant: {err}"
        );
    }
}

/// Phase 4: confirms the same-crate viewmodel registry (`component_frontend::same_crate_viewmodels`,
/// populated by `generate_viewmodel_from_item_mod`) actually catches, on the Rust-macro path, the
/// two mistakes its own doc comment claims it fixes — a typo'd `vm.field` reference and a
/// `#[bindable]` field whose type isn't a `viewmodel` at all — mirroring `validate.rs`'s own
/// `rejects_reference_to_unknown_vm_field`/`rejects_bindable_field_whose_type_is_not_a_viewmodel`
/// tests for the DSL-text frontend. Names are unique per test for the same reason
/// `dsl_enum_tests` uses unique names (shared process-global registries).
#[cfg(test)]
mod viewmodel_registry_tests {
    use super::*;

    fn register_viewmodel(src: &str) {
        let item_mod: syn::ItemMod = syn::parse_str(src).expect("mod should parse");
        generate_viewmodel_from_item_mod(&item_mod).expect("viewmodel generation should succeed");
    }

    #[test]
    fn typo_vm_field_reference_is_rejected_on_macro_path() {
        register_viewmodel(
            r#"
            mod vm_typo_a_mod {
                struct VmTypoA {
                    #[observable(default = String::new())]
                    content: String,
                }
            }
            "#,
        );
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct ScreenTypoA {
                #[param]
                #[inject]
                vm: VmTypoA,
                body: view! {
                    VerticalLayout {
                        TextBlock { text: vm.no_such_field }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("a typo'd vm field reference should be rejected");
        assert!(err.contains("no_such_field"), "error: {err}");
    }

    #[test]
    fn bindable_field_on_non_viewmodel_type_is_rejected_on_macro_path() {
        // A plain sibling component (not a viewmodel), registered exactly like any other
        // #[elwindui::component] would be.
        let not_vm_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct NotAViewModelB {
                #[param]
                label: String,
                body: view! { TextBlock { text: label } },
            }
            "#,
        )
        .unwrap();
        generate_component_from_item_struct(None, &not_vm_struct)
            .expect("sibling component should build");

        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct WindowBindableB {
                #[bindable]
                thing: std::rc::Rc<NotAViewModelB>,
                body: view! {
                    Window { TextBlock { text: "x" } }
                },
            }
            "#,
        )
        .unwrap();
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("#[bindable] on a non-viewmodel type should be rejected");
        assert!(err.contains("isn't a `viewmodel`"), "error: {err}");
    }
}

/// `#[elwindui::component] impl Name { .. }` — §3's `#[overridable]`/`#[overrides]` method
/// inheritance on the Rust-macro path (`generate_component_methods_from_item_impl`). The
/// downstream half (`codegen::resolve_effective_methods`'s `__base_<name>` shadows,
/// `rewrite_base_calls`, `validate`'s signature check) is already covered by `parser.rs`/
/// `validate.rs`'s own tests against the DSL text form; these cover the macro front door and the
/// struct-before-impl pairing it depends on.
///
/// Component names are unique per test for the same reason `dsl_enum_tests` does it — the
/// same-crate registries are process-global statics shared by every test in this binary.
#[cfg(test)]
mod component_impl_tests {
    use super::*;

    fn declare(base: Option<&str>, src: &str) {
        let item_struct: syn::ItemStruct = syn::parse_str(src).expect("struct should parse");
        generate_component_from_item_struct(base.map(str::to_string), &item_struct)
            .expect("struct half should generate");
    }

    fn methods(src: &str) -> Result<String, String> {
        let item_impl: syn::ItemImpl = syn::parse_str(src).expect("impl should parse");
        generate_component_from_item_impl(&item_impl).map(|t| t.to_string())
    }

    /// The `struct` half emits nothing at all now — every token comes from the `impl` half.
    #[test]
    fn the_struct_half_emits_nothing() {
        let item_struct: syn::ItemStruct =
            syn::parse_str(r#"struct MiSilent { body: view! { VerticalLayout { } }, }"#)
                .expect("struct should parse");
        let out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should succeed");
        assert!(out.is_empty(), "struct half should emit nothing, got: {out}");
    }

    #[test]
    fn overridable_method_is_emitted_as_a_public_inherent_method() {
        declare(
            None,
            r#"struct MiBase { body: view! { VerticalLayout { } }, }"#,
        );
        let out = methods(
            r#"
            impl MiBase {
                #[overridable]
                fn label(&self) -> String { "base".to_string() }
            }
            "#,
        )
        .expect("impl half should generate");
        assert!(
            out.contains("struct MiBase"),
            "the impl half emits the whole type: {out}"
        );
        assert!(out.contains("pub fn label"), "should emit `label`: {out}");
    }

    #[test]
    fn overrides_gets_a_base_shadow_and_a_rewritten_base_call() {
        declare(None, r#"struct MiSuper { body: view! { VerticalLayout { } }, }"#);
        methods(
            r#"
            impl MiSuper {
                #[overridable]
                fn label(&self) -> String { "super".to_string() }
            }
            "#,
        )
        .expect("base impl should generate");
        declare(
            Some("crate::MiSuper"),
            r#"struct MiDerived { body: view! { MiSuper { } }, }"#,
        );
        let out = methods(
            r#"
            impl MiDerived {
                #[overrides]
                fn label(&self) -> String { format!("{}!", base::label()) }
            }
            "#,
        )
        .expect("derived impl should generate");
        assert!(
            out.contains("fn __base_label"),
            "base body should be kept as a private shadow: {out}"
        );
        assert!(
            out.contains("self . __base_label ()") || out.contains("self.__base_label()"),
            "`base::label()` should be rewritten onto the shadow: {out}"
        );
    }

    #[test]
    fn overrides_without_a_matching_overridable_is_rejected() {
        declare(None, r#"struct MiNoHook { body: view! { VerticalLayout { } }, }"#);
        declare(
            Some("crate::MiNoHook"),
            r#"struct MiNoHookChild { body: view! { MiNoHook { } }, }"#,
        );
        let err = methods(
            r#"
            impl MiNoHookChild {
                #[overrides]
                fn missing(&self) -> String { String::new() }
            }
            "#,
        )
        .expect_err("overriding a method the base never declared should be rejected");
        assert!(err.contains("no matching"), "error: {err}");
    }

    #[test]
    fn signature_mismatch_against_the_base_is_rejected() {
        declare(None, r#"struct MiSigBase { body: view! { VerticalLayout { } }, }"#);
        methods(
            r#"
            impl MiSigBase {
                #[overridable]
                fn label(&self) -> String { String::new() }
            }
            "#,
        )
        .expect("base impl should generate");
        declare(
            Some("crate::MiSigBase"),
            r#"struct MiSigChild { body: view! { MiSigBase { } }, }"#,
        );
        let err = methods(
            r#"
            impl MiSigChild {
                #[overrides]
                fn label(&self, extra: i32) -> String { let _ = extra; String::new() }
            }
            "#,
        )
        .expect_err("a different signature should be rejected");
        assert!(err.contains("different signature"), "error: {err}");
    }

    #[test]
    fn an_untagged_fn_is_rejected() {
        declare(None, r#"struct MiUntagged { body: view! { VerticalLayout { } }, }"#);
        let err = methods(r#"impl MiUntagged { fn helper(&self) -> String { String::new() } }"#)
            .expect_err("an untagged fn should be rejected");
        assert!(
            err.contains("#[overridable]") && err.contains("#[overrides]"),
            "error should name both tags: {err}"
        );
    }

    #[test]
    fn an_impl_before_its_struct_is_rejected() {
        let err = methods(
            r#"
            impl MiNeverDeclared {
                #[overridable]
                fn label(&self) -> String { String::new() }
            }
            "#,
        )
        .expect_err("an impl with no registered struct should be rejected");
        assert!(err.contains("declare the struct first"), "error: {err}");
    }

    #[test]
    fn a_trait_impl_is_rejected() {
        declare(None, r#"struct MiTraitImpl { body: view! { VerticalLayout { } }, }"#);
        let err = methods(r#"impl Clone for MiTraitImpl { fn clone(&self) -> Self { todo!() } }"#)
            .expect_err("a trait impl should be rejected");
        assert!(err.contains("trait impl"), "error: {err}");
    }
}

/// Reproduction scaffolding for `Derived inherits <user component>` (Refs #23's investigation,
/// Refs #25's fix). Unlike `component_impl_tests` above (which only asserts `Ok(())`/error
/// *strings*, discarding the generated token text), these tests inspect the actual generated
/// `#[elwindui::class(inherits = ..)]` argument — that blind spot is exactly what let #25's bug
/// (a bare, unqualified name for a user-defined base, silently rejected only once real code tried
/// to compile it) slip past this same test module for a full release cycle.
#[cfg(test)]
mod user_base_inherits_tests {
    use super::*;

    fn declare(base: Option<&str>, src: &str) -> Result<(), String> {
        let item_struct: syn::ItemStruct = syn::parse_str(src).expect("struct should parse");
        generate_component_from_item_struct(base.map(str::to_string), &item_struct).map(|_| ())
    }

    fn build(src: &str) -> Result<String, String> {
        let item_impl: syn::ItemImpl = syn::parse_str(src).expect("impl should parse");
        generate_component_from_item_impl(&item_impl).map(|t| t.to_string())
    }

    /// `inherits crate::UbBase` (a fully crate-root-qualified path, as `#25`'s fix now requires
    /// for a user-defined base): `UbDerived` must build, and — the actual regression check — its
    /// generated `#[elwindui::class(inherits = ..)]` argument must carry that same qualified path
    /// verbatim, not the bare `UbBase` `codegen::base_trait_path` used to emit before the fix
    /// (which `elwindui_macros::class::validate_fully_qualified_path` would reject the moment this
    /// token stream was ever fed through the real `#[class]` proc macro).
    #[test]
    fn derived_from_a_user_component_builds_with_a_qualified_path() {
        declare(
            Some("ContentControl"),
            r#"struct UbBase { body: view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("base struct");
        build(r#"impl UbBase { }"#).expect("base impl");
        declare(
            Some("crate::UbBase"),
            r#"struct UbDerived { body: view! { UbBase { } }, }"#,
        )
        .expect("derived struct");
        let out = build(r#"impl UbDerived { }"#).expect("derived impl should generate");
        assert!(
            out.contains("inherits = crate :: UbBase"),
            "expected the qualified path to survive into `#[elwindui::class(inherits = ..)]` \
             verbatim: {out}"
        );
    }

    /// The exact failure Issue #25 reported: a *bare* name for a user-defined base. Must now be
    /// rejected up front, with a diagnostic that names the actual problem (a missing qualified
    /// path) — not left to surface later as `elwindui_macros::class`'s own internal-sounding
    /// `__elwindui_inherit_*!` error once the generated (broken) code is actually compiled.
    #[test]
    fn derived_from_a_user_component_with_a_bare_base_name_is_rejected() {
        declare(
            Some("ContentControl"),
            r#"struct UbBareBase { body: view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("base struct");
        build(r#"impl UbBareBase { }"#).expect("base impl");
        declare(
            Some("UbBareBase"),
            r#"struct UbBareDerived { body: view! { UbBareBase { } }, }"#,
        )
        .expect("derived struct");
        let err = build(r#"impl UbBareDerived { }"#)
            .expect_err("a bare name naming a user-defined base should be rejected");
        assert!(
            err.contains("crate::ui::UbBareBase") || err.contains("crate::UbBareBase"),
            "error should suggest a qualified path: {err}"
        );
    }

    /// Regression guard: a *builtin* base must keep emitting its old, bare-but-fully-resolvable
    /// `elwindui::ui::X` form — #25's fix only changes user-defined bases, never builtin ones.
    #[test]
    fn derived_from_a_builtin_base_still_emits_the_builtin_path() {
        declare(
            Some("ContentControl"),
            r#"struct UbBuiltinDerived { body: view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("derived struct");
        let out = build(r#"impl UbBuiltinDerived { }"#).expect("derived impl should generate");
        assert!(
            out.contains("inherits = elwindui :: ui :: ContentControl"),
            "builtin base should stay fully-qualified via the existing `elwindui::ui::` rule: {out}"
        );
    }
}
