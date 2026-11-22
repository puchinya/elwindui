# `#[class]` macro implementation design

Related specification: [`../../specs/macro_class_spec.md`](../../specs/macro_class_spec.md). Implementation invariants for agents are in [`../../agents/class-model.md`](../../agents/class-model.md).

## Expansion model

The macro parses the class declaration and paired implementation, validates the public class contract, then emits the concrete storage type, extension trait implementation, ancestor forwarding machinery, constructors, and generated accessors required by that contract.

Ordinary and root classes generate their own extension trait surface. These are the only class forms: the `trait_only` (interface without storage), `struct_only` (storage implementing an existing trait), `abi_props` and `no_ancestor_forward` modes were removed by Issue #313, together with the class-interface bridge macro and the `__elwindui_inherit_<Name>_skip!` entry point that existed only for them. The argument parsers of `#[class]` and `#[component]` reject each removed mode by name (`REMOVED_CLASS_MODES`), and a `trait` item is rejected as a class shape.

View components that are lowered into a generated `#[class]` implementation pass their companion
`#[overridable]`/`#[overrides]` method metadata through this same expansion boundary. The class macro
remains the single owner of ancestor forwarding and runtime virtual dispatch; component codegen does
not create a parallel vtable, forwarding table, or method-name-specific route.

## Ancestor forwarding

Generated `__elwindui_inherit_*!` macros carry the ancestor chain across module and crate boundaries. `#[overridable]` emits dynamic accessors used by forwarding; `#[overrides]` routes through the subclass implementation. Sealed members omit the override route.

**Ancestor-trait resolution — `__ElwindUIOwnExt` (PR #164 review finding A2)**: every class's own `macro_reexport_mod_ident` wrapper module (`__elwindui_macros_of_<bare_name>`) exposes a stable hidden alias, `pub use <ClassName>Ext as __ElwindUIOwnExt;`. Descendants resolve an ancestor's trait purely through this alias — `<ancestor's own module prefix>::__ElwindUIOwnExt` — never through a same-crate registry lookup or a naming-convention guess. Generic reattachment (a descendant's own `inherits = Parent<Concrete>` trailing type arguments, which a plain `use` cannot itself carry) is handled by the mechanisms below.

Path resolution follows the same discipline throughout — computed fresh by each caller from information *it* reliably holds (its own `inherits = ..` argument text, reached through the target's existing wrapper module, `path_module_prefix`), never baked once into a macro body that later gets re-expanded from an arbitrary third crate.

**Registry-independent generic reattachment — `__ElwindUIOwnExtBound`/`__elwindui_apply_own_ext` (PR #164 final remediation round, finding A5)**: a descendant's own `inherits = Ancestor<Args>` trailing generic arguments are reattached onto the ancestor's own `{Name}Ext` trait (`ParentExt<T>`). An earlier revision decided this via a same-crate registry (`same_crate_classes`), which is empty in a separate `cargo build` process and so cannot work cross-crate — replaced by two proc-macro-time/macro-expansion-time mechanisms, split because Rust does not permit a macro invocation in supertrait-bound position (`trait X: some_macro!() {}` is a hard parse error), so a single macro-protocol (as an early draft of this fix assumed) cannot cover both call sites:
- **Bound position** (a descendant's own supertrait bound): every class-managed ancestor generates `__ElwindUIOwnExtBound_<bare_name>`, a `#[doc(hidden)]` wrapper trait declared with *exactly* the ancestor's own generic parameter list, supertrait-bound to the ancestor's real `{Name}Ext` trait, satisfied via a blanket impl over *any* type implementing that real trait (not a narrow impl for just the declaring struct — a multi-hop descendant like `Shape`/`Control` also implements `UIElementExt` without literally being `UIElement`). A descendant's own bound resolves through this wrapper (`ancestor_own_trait_bound`), which — being an ordinary trait, not a macro — participates in bound position exactly as ordinary Rust generics resolution expects.
- **Dispatch-routing position** (whether to pass a descendant's own concrete generic-argument tokens further down the recursion): every class-managed ancestor also generates `__elwindui_apply_own_ext_<bare_name>`, a macro-expansion-time policy macro that echoes the caller's concrete generic-argument tokens back. Each recursion step generates a fresh, uniquely-named continuation macro that already has everything else baked in, receiving only the policy macro's resolved tokens and splicing them into the final entry-macro invocation.

Neither mechanism ever inspects `same_crate_classes`/any other same-crate-only registry, and neither ever globally appends or drops generic arguments independent of the specific ancestor being recursed through.

**Recursive forwarding also uses the dispatch-routing policy — `AncestorRoute` (PR #164 final remediation round, finding A6)**: A5's fix above only covers the *immediate* `inherits = ..` edge (`expand_impl`'s own prelude, resolved once per class, at that class's own generation time). But `build_inherit_macros` generates classify macros that are *reused* for every future descendant depth — when class `D` (`inherits = Concrete<T>`) has its own classify muncher's base case fire for some later descendant `E` (`inherits = D`), that base case needs to forward *past* `D` into `Concrete`'s own real interface, and an earlier revision still baked that resolved trait as a literal token there too, via the same same-crate-registry heuristic A5 replaced for the immediate hop — correct only when `Concrete` happened to be in the same crate as `D`. Fixed by extending A5's own protocol to the recursive case: `build_ancestor_route` builds an `AncestorRoute` (deliberately unresolved — `own_ext_prefix`/`concrete_args`/`concrete`/`policy_path`, all `rewrite_crate_segment`-treated) in place of the old `(next_trait, next_concrete)` pair; `build_inherit_macros` uses it to generate one fixed, once-per-class continuation macro (`__elwindui_continue_recursive_inherits_of_<bare_name>`, generated *once* per class regardless of how many future descendants eventually reach it — a per-invocation name would collide the second time a different descendant reaches the same hop), which classify's own base-case body invokes through `Concrete`'s own `__elwindui_apply_own_ext_*!` policy macro (extended with a second arm accepting `$SubType`/trailing override tokens alongside the continuation path, since those are only known once a real descendant chain actually reaches this hop — not at `D`'s own proc-macro time). `ancestor_own_trait` (the old registry-dependent resolution) is deleted entirely. Regression coverage: `crates/elwindui/tests/class_chain_cross_crate.rs`'s `GenericOrdinaryDeepDerived` (a *second* hop below a generic ordinary ancestor, T20), plus a macro-expansion-level test (`class.rs`'s own `recursive_forwarding_metadata_uses_the_next_ancestors_own_policy_macro`, T21) proving the generated recursive metadata calls the policy macro rather than embedding a resolved trait.

**Root-mode descendants (PR #164 final remediation round, finding C2)**: a root class (no `inherits` at all, e.g. `UIElement`) exposes `as_ui_element(&self) -> &Self`, a required trait method whose return type is hard-pinned to the root's own concrete type. Every descendant's classify macro forwards it to `self.base.as_ui_element()` through a separately-threaded `$RootConcrete` macro parameter (the root's own concrete path, supplied fresh by each caller from its own fully-qualified `inherits=` text, because a `#[macro_export]`'d body cannot name it itself). Runtime coverage: `crates/elwindui/tests/class_chain_cross_crate.rs`'s `BridgeFixtureRoot` -> `BridgeFixtureConcrete` -> `BridgeFixtureDerived` (cross-crate).

Generated macros use fully qualified paths. `$crate` is retained where it must refer to the declaring crate; tokens that intentionally refer to the consuming crate are rewritten during expansion.

## Construction

`construct` is the single source for constructor arguments. Expansion generates `new`, initializes the weak self handle after allocation, and invokes `on_constructed` only after the object can be safely upgraded through that handle.

Expansion also generates a second, unconditional constructor, `__new_unmounted` (CI-7 of the elwindui #80 lifecycle-refactor tracking issue, `docs/design/runtime/component_lifecycle_design.md` §4f) — the same allocation step as `new`, without the trailing `on_constructed` invocation. It exists so a caller (today, only `EnvironmentScope`'s own generated code) can construct an object and then call `mount(environment)` on it explicitly, against a specific `EnvironmentContext`, instead of letting construction auto-mount it. Both constructors are always emitted together; there is no class-level flag selecting one or the other, because the choice belongs to the call site, not the type.

Hand-written `new` or `__new_unmounted` is rejected because either would compete with the generated ownership sequence.

## Registry and analysis

Cross-item class information is stored in a compilation-scoped registry keyed so separate consuming crates cannot contaminate one another. Source-order dependencies are minimized, but rust-analyzer may expand incomplete files in an order unavailable to normal rustc.

The rust-analyzer shadow expansion supplies enough generated structure for name resolution without changing rustc output. Spans from user declarations are preserved on generated public items and forwarding calls where possible so diagnostics and breakpoints point back to source.

## Shape macro protocol (`__elwindui_props_*!`)

Every class also emits a `__elwindui_props_{Name}!` declarative macro carrying its DSL-visible property surface across the same crate boundary `__elwindui_inherit_*!` carries the ancestor chain across (`elwindui-codegen` never has a local `TypeInfo` for a real builtin — only a same-crate DSL fixture in tests does — so it constructs against this macro instead of a registry entry). Each query is a two-hop `@x`/`@x_from` pair: the entry seeds `$origin` with this class's own name and hands off to the `_from` form, which either matches a literal per-property arm or forwards unmatched names to the parent's own macro, unchanged, so `$origin` still names the type the use site actually wrote by the time a terminal `compile_error!` or catch-all is reached.

- `@set`/`@set_from` — assigns a plain (non-attached, non-collection-content) property, via `wrap_prop_value`'s per-property conversion (a `String` gets `&`, `Brush`/`Color` gets `.into()` plus a declared-type-driven `Some(..)`, a plain type passes through unchanged).
- `@set_with_environment`/`@set_with_environment_from` — assigns a property while carrying the node's effective `EnvironmentContext`. A `#[prop(semantic_brush, ..)]` arm resolves `BrushStyle` and maps `PlatformDefault` to `@clear`; an ordinary property delegates to `@set` unchanged. `#[text_style]` contributes equivalent arms for its injected fields, with only `foreground` marked semantic.
- `@is_semantic_brush`/`@is_semantic_brush_from` — expands to a boolean capability query derived from `#[prop(semantic_brush, ..)]` or `#[text_style]`, used to install semantic Environment subscriptions without a code-generator-side property-name list.
- `@clear`/`@clear_from` — resets a property to its declared platform/property default. A semantic `@set_with_environment` arm maps `ResolvedValue::PlatformDefault` here rather than materializing a concrete brush in core.
- `@routed`/`@routed_from` — registers a `#[routed]` callback, building the bubbling adapter around a bare DSL-supplied closure.
- `@attached_set` — a `#[attached]` property's setter, one hop only (no ancestor forwarding — the DSL's `Owner::field` syntax always names the owning class explicitly).
- `@children`/`@children_into` — attaches bare nested child elements to whichever property `#[content(..)]` names, forwarding up the chain until the declaring class is found.
- `@children_erased`/`@children_erased_into` — the same metadata-selected destination for a child that is
  already an `Rc<dyn UIElementExt>` (or another erased content trait). This path calls the declaring
  extension trait with the value unchanged, so cross-crate scalar content does not attempt an
  `into_ui_element_node()` conversion on a trait object. It is a transport detail, not a second
  content model.
- `@content_shape` — selects one of caller-supplied scalar/collection lowering blocks from the
  effective content property's declared shape. Codegen uses this only when an external class has no
  local field table; the decision still comes from `#[content(..)]` metadata and field type.
- `@content_slot_type` — the type-position companion to `@content_shape`, selecting `()` for an
  external scalar content destination and `DynamicChildSlot<T>` for a collection destination. It
  keeps generated storage metadata-driven when the declaring class is outside the codegen crate.
- `@field_type`/`@field_type_from` — expands, in **type position**, to the real Rust type a declared property was given (`elwindui-codegen`'s `resolve_effective_fields`/`synthesize_external_base_fields`, Refs #90: a consumer component that bare-forwards an inherited attribute value from a genuinely external base — `padding: padding` on `#[elwindui::component(inherits Control)]`, dsl_spec.md §3's `ContentControl` pattern — has no local `TypeInfo` to read `padding`'s declared type from, so the synthesized field's own `FieldDef::ty` is instead the literal text `elwindui::core::__elwindui_props_Control!(@field_type padding)`; `generate_view`'s `syn::parse_str::<syn::Type>` already parses an arbitrary type-position macro invocation as an ordinary `syn::Type::Macro`, so this needs no special casing downstream). Same per-property arm/forwarding/terminal-`compile_error!` shape as `@set`, restricted to the same `takes_set_arm`-eligible property set (a routed/attached/collection-content property has no single settable "value type" a struct field could hold).
- `@content_item_dyn`/`@content_field_get` — cross-crate queries backing dynamic (`for`/`if`/`match`) region reconciliation against a content-collection field with no local `TypeInfo`.
- `@content_item_type`/`@content_dynamic_host` — generated-component collection queries. A concrete
  `Vec<Rc<T>>` preserves `T` as the dynamic slot item type and exposes the generated component's
  `DynamicChildHost<T>` adapter; it must not be erased into `dyn Vec<..>`. Framework-owned live
  collections continue to use `@content_item_dyn` and their getter. The distinction is derived from
  the declared content shape, not from a component or crate name. The generated host is currently
  emitted for a directly-declared content field; inherited-only generated content host forwarding
  is a separate capability tracked in follow-up Issue #194.
- `@assert_undeclared`/`@assert_declared` — compile-time collision/designation probes, emitted in item position next to the class declaration rather than consulted from a use site.

Only `@set`'s `wrap_prop_value` knows how to shape a value into a declared property's real setter; a bare-forwarded field whose own value already carries that exact shape (rather than the bare/literal shape every ordinary call site supplies) is `elwindui-codegen`'s own responsibility to normalize before calling `@set` — see `emit_resync`/`emit_external_attribute_sets`/`build_component_args`'s own `ty.contains('!')` branches in `codegen.rs`.

### Inherited property access arms (#166)

`@inherited_get $recv, name` / `@inherited_set $recv, name, value` /
`@inherited_op_assign $recv, name, op, value` は、派生生成componentの`view`内の裸名を宣言元の祖先へ
解決するためのarmである。宣言した側のarmは自classの`{Class}Ext`(`exported_own_ext_path`)を
展開先のblock scopeへ`use .. as _`し、呼び出し側のtrait importに依存しない。このclassが`@set`対象として宣言した(`routed`以外の)propertyは
`$recv.name()` / `$recv.set_name(value)` / 「getterで取得した一時値へ元の`op=`(単一token`$op:tt`)を
適用してsetterへ渡す」へ展開し、それ以外の名前は親classのshape macroへ転送する。親を持たない終端では、
元の裸の名前(`name` / `name = value` / `name op= value`)をそのまま返し、未宣言名をpropertyとして
推測しない。生成componentは公開writable Propだけをclass propertyとして
宣言するため、`#[state]`・computedはこの経路では解決されない。`#[param]`は`#[fixed(name)]`(読み取り専用の
コンストラクタ入力)として宣言され、`@inherited_get`だけが宣言元のgetterへ解決する(`@set`・`@field_type`等の
property protocolには参加しない)。派生componentへ自動転送された基底の必須入力を裸名で読むための経路である。

`@initial_with_environment $recv, name, value, environment`(Issue #166)は、派生componentの`view`のrootが、
埋め込んだ基底(`$recv`はその参照)にmount前の初期値を設定するための経路である。各classは自分が宣言した名前だけを、
自分の値に自分のtraitで設定し、それ以外の名前は自分の`base`(`&$recv.base`)と共に親classのshape macroへ転送する。
派生classは祖先のtraitをすべて実装しているわけではなく、隠しinitial setterも委譲されないため、受け手を宣言元の
値そのものにする。

- `#[prop]`・`#[text_style]`のpropertyは、そのclassの`@set_with_environment`で設定する。semantic brushの
  propertyはここでは設定しない(下の`@initial_brush_with_environment`で設定する)。
- `#[fixed(name: Type, default)]`(既定値付きのコンストラクタ入力)は、隠しinitial setter
  `__set_initial_param_<name>`で設定する。値はconstructor ABIと同じく宣言型で変換する
  (`String`/`Vec`は`.to_string()`/`.to_vec()`、`Option<T>`は`Some(..)`で包み、`None`はそのまま)。
- `#[fixed(name)]`(必須のコンストラクタ入力)は、構築時にコンストラクタへ渡し済みなので何もしない。
- 親を持たない終端まで宣言元がなければ`compile_error!`にする。

`@required_params $cb, { .. }`(Issue #166)は、`#[constructor_inputs(a, b, ..)]`で宣言されたコンストラクタ
引数名を`$cb! { { .. } [a, b, ..] }`として返す。宣言のないclass(builtin)は空のリストを返すので、別クレートの
派生componentはどの基底にも同じ形で問い合わせられる。

`@initial_brush_with_environment`は同じ連鎖を、派生componentがmountされた後(`__build_view`)にmount時の
環境で実行する。semantic brushのpropertyだけを設定し、それ以外の名前は何もしない(未宣言名の報告は構築時の
`@initial_with_environment`が行う)。semantic brushは派生componentが置かれた環境(`EnvironmentScope`など)で
解決する必要があり、構築時にはその環境がまだ決まっていないためである。

`@set_chain_with_environment $recv, name, value, environment`(PR #312 contract v3)は、
`@initial_with_environment`と同じく宣言元のclassを`&$recv.base`でたどり、そのclassの値に自分のtraitで設定する。
semantic brushもその場で`environment`で解決して設定する。手書きストレージのcomponentのconstructor ABIが、
自分では列挙しない名前(祖先やCore interfaceのproperty)をこの経路へ渡す。呼び出し側でtraitを
importしなくても設定できる。

### `#[component]`のclass-backed形(PR #312 contract v3)

`elwindui_macros::component`は、手書きストレージを持つcomponent(structの段に`#[class]`の形の属性を持つ
struct、`construct`だけで判定されるstruct)を、引数を`#[class]`の形
(`inherits = Path`など)に変えてこの層(`class::expand`)へそのまま渡す(`component_class.rs`)。`#[class]`自体の
展開は変えない。struct側の情報(抽象か、`#[constructor_inputs]`)はimpl側でconstructor ABIを
生成するために保存する(トークンはマクロ呼び出しをまたいで保持できないので文字列で保存する)。
`#[constructor_inputs(..)]`は`@required_params`の答えにもなる。このため`#[constructor_inputs(..)]`(空の
`#[constructor_inputs()]`を含む)は、それだけでproperty-shape macro(`__elwindui_props_<Name>!`)を出力する
宣言として扱う。`inherits`のないrootでも同じで、constructor ABIと派生componentはこのmacroに問い合わせる
(Issue #313 review A1、回帰テストは`crates/elwindui/tests/constructor_inputs_only_root.rs`)。

## Validation boundary

Publicly observable accepted/rejected forms remain in the specification. Parser representation, token generation, registry layout, shadow expansion, and debugging strategy belong only here.

## Generated component shape forwarding across crate boundaries

When `#[component]` generates a Control-derived class, the component code generator re-emits the
component's own public `#[prop]` and `#[content(...)]` declarations into the generated class shape.
The resulting `__elwindui_props_<Name>!` macro is therefore available to a downstream `view!`
expansion through the defining crate root, including property setters, content-shape queries, event
and two-way wiring, and resync. This is shape metadata forwarding; it is not runtime reflection or
a second property system.

The forwarded surface is deliberately limited to public construction/content metadata. Internal
`#[computed]`, `#[state]`, and `#[environment]` values do not become writable external properties.
The external DSL still names the generated component through a qualified Rust path. The generated
extension trait is imported from that authored type path (including any intermediate module), while
the `#[macro_export]` shape macro is always rooted at the defining crate; no generated shape assumes
that the defining crate provides a `pub mod ui` compatibility namespace. The stable framework
support paths used inside the exported shape remain `elwindui::core::ui::...`.

For source-local own fields, `ComponentPublicShape::writable_fields` is the sole authority for
whether the external class shape contains a writable property and for the setter parameter type.
The source `FieldDef` is joined only for declaration attributes such as `two_way`, `routed`, and
semantic-brush behavior. The internal `#[prop(owned, ..)]` marker preserves generated setter
conventions: only an actual `String` setter parameter receives `.to_string()`; every other actual
setter type is passed through unchanged. The separate generated constructor ABI is consumed by
`elwindui::new!`: required/defaulted Param shape is supplied by `ComponentPublicShape`/`TypeInfo`,
while ordinary Props remain optional pre-mount initial values and are not promoted to constructor
parameters.

`owned` is an internal class-shape flag used only when generated component metadata must preserve an
owned setter convention, for example `#[prop(owned, title: String)]`. It is not recommended
user-facing DSL syntax. The generated shape converts an authored string value to the owned `String`
expected by a generated setter; hand-written builtin declarations retain their existing borrowed
`String`/`&str` setter convention. Non-String owned values retain their declared type without a
String-specific conversion. Direct macro tests cover both cases.
