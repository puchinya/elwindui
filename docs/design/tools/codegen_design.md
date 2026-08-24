# ElwindUIL コード生成設計

本書は、ElwindUIL の Rust 属性マクロ入力を解析し、検証済みの Rust トークン列へ変換する `elwindui-codegen` の内部architectureを定める。公開構文と診断contractは [`dsl_spec.md`](../../specs/dsl_spec.md)、`#[class]` の展開方式は [`class_macro_design.md`](class_macro_design.md)を正本とする。

## 1. 責務

`elwindui-codegen` は次を担当する。

- `#[elwindui::component]`、`#[elwindui::viewmodel]`、`#[elwindui::dsl_enum]` の入力を共通ASTへ変換する
- `view! { ... }` の要素treeと式を解析する
- 名前、型、binding方向、動的領域などを静的に検証する
- dependencyと購読ownershipを解析する
- backend非依存のRustトークン列を生成する
- spanを保持し、公開contractに沿ったcompile-time diagnosticを返す

LSP、preview、hot reloadのプロセス制御は、それぞれの設計文書の責務である。

- [`languageserver_design.md`](languageserver_design.md)
- [`preview_design.md`](preview_design.md)
- [`hotreload_design.md`](hotreload_design.md)

## 2. 入出力境界

入力はRust compilerが属性macroへ渡すtoken streamである。属性frontendは `syn` のRust ASTと、`view!` field内のDSL tokenを共通ASTへ正規化する。

出力はmacro展開位置へ直接埋め込むRust token streamである。中間source fileは生成しない。生成物は共通runtime traitを参照し、使用backendはfacade crateのCargo featureとlink対象backend crateで決まる。code generator自身はbackendごとのsource branchを持たない。

## 3. Pipeline

```text
Rust attribute input / view! tokens
              ↓
frontend normalization
              ↓
common AST and registry resolution
              ↓
static validation and dependency analysis
              ↓
backend-independent Rust token generation
```

### 3.1 Frontend normalization

`component_frontend` と `attr_frontend` は `struct`、`mod`、`enum` を共通ASTへ変換する。`view!` fieldの内側だけは `parser` がDSL grammarとして解析する。この境界より後のvalidationとgenerationは、どの属性macroから入力されたかに依存しない。

`#[control_template(target = T)]`も同じView ASTへlowerし、reserved `templated_parent: Weak<T>`を持つ
private template-instance Componentと`ControlTemplate<T>` factoryを生成する。public declarationは
`Name::template()`を提供するzero-sized namespaceとして残す。

### 3.2 Registry resolution

同一crate内のcomponent、viewmodel、DSL enumのmetadataはmacro展開中のregistryへ登録する。後続の展開はregistryから宣言済みmetadataを参照するため、DSLのcross-item検証は宣言順に依存する。

Rust pathそのものの最終的な名前解決は生成後のrustcへ委譲する。別crateの型情報をmacro process内registryへ複製しない。

### 3.2a rust-analyzerとの二重展開境界(#146)

3.2のsame-crate registryは、「`cargo build`はクレートごとに新規プロセスでコンパイルし、宣言順に一度だけ展開されれば正しく埋まる」という通常rustcの前提の上でのみ機能する。rust-analyzerはワークスペース全体で1つの永続`proc-macro-srv`を使い、マクロ展開をインクリメンタル・オンデマンドに行うため、ソース上は正しい宣言順であってもregistry参照側が先に評価され、幽霊(ghost) diagnosticsを出すことがある(Component struct/impl pair、Theme→same-crate Environment Key参照が代表例)。

`#[class]`マクロが既に採用している[rust-analyzer shadow expansion](class_macro_design.md#registry-and-analysis)と同じ二重展開modelを、Component/Themeにも適用する。

```text
normal rustc/cargo
    -> 既存のstrict registry-backed expansion(3.2/3.3のまま)
    -> #[cfg(not(rust_analyzer))] real generated items

rust-analyzer
    -> source-local/self-contained shadow expansion
    -> #[cfg(rust_analyzer)] shadow items
```

境界となる原則は次のとおり。

- **通常rustc側のstrict semanticsは変更しない。** source上で本当にmissingなComponent struct/same-crate Environment Keyは、通常の`cargo build`/`cargo check`では引き続きcompile errorになる。「rust-analyzerで誤診断になるからstrict validationを削除する」は採らない。
- **validationをitem-localとregistry依存の2種類に分ける。** attribute構文、対応不能なtarget item、`view!`構文異常などitem単体で判定できるerrorは、RA/rustc問わず常時diagnosticのままにする。Component structの登録有無やTheme参照先Environment Keyの存在などcross-item registry依存のerrorだけを、`#[cfg(not(rust_analyzer))]`側のreal expansionへ限定する。
- **rust-analyzer shadowはIDEのname/type resolutionに必要な最小限の形状のみを提供する。** 型名、constructor/property/methodのsurfaceだけを生成し、runtime実装やcross-item semantic validationをshadow内で再現しない——それらの一部のcross-item errorはIDEではなくcargo checkでのみ確定するという意図的なtrade-offである。
- **rust-analyzer検出はconsumer側生成Rust itemsの`cfg(rust_analyzer)`だけで行う。** proc-macroプロセス内でenvironment変数・process名等からRA/rustcを推測しない。同様にproc-macroからsource filesystemを走査してregistryの穴を埋めることもしない——両方ともincremental analysis・unsaved buffer・macro hygiene・workspace isolationを壊すため採用しない。
- **同じsource item -> 同じshadow shape。** rust-analyzerのproc-macro-srvが同じitemを複数回展開しても、shadowの正しさはprocess-local registryの過去の状態に依存してはならない。

Component struct halfとimpl halfは、runtime上「struct halfはmetadata登録のみ、impl halfが実型を生成する」という既存contractのまま変わらない。rust-analyzer shadowだけは例外で、struct half自身がown source fieldsから型・constructor・property shapeのshadowを出し(`build_component_struct_shadow`)、impl halfはregistry lookupより前のitem-local method parsingからmethod shadowを出す(`build_component_impl_shadow`)。両者が使うconstructor/getter/setter classificationは、real generatorとも共有する単一のsource-local helper(`component_public_shape`、`crates/elwindui-codegen/src/component_frontend.rs`)に集約し、別実装として複製しない。Themeはmarker型と`Theme` implの存在だけをIDEへ伝えれば十分なため、Environment Keyごとのreal `set::<K>()` bodyを再現しないno-op shadow(`build_theme_shadow`)を生成する。real側とshadow側のtop-level itemsは常に`cfg(not(rust_analyzer))`/`cfg(rust_analyzer)`で排他になるよう、`crates/elwindui-codegen/src/rust_analyzer_shadow.rs`の`gate_real_items_for_rustc`が一箇所でcfg付与を担う。

Environment Key/ViewModel/Store/DSL enumのdefining expansion(`#[elwindui::environment_key]`等)はそれ自身のdefinitionにprevious sibling registry lookupを必要としないため、shadow化の対象外とし、既存のself-contained expansionをそのまま維持する。

#### item-local/registry依存の分類実装(PR #169レビュー是正)

「item-localとregistry依存の2種類に分ける」という原則は、`validate::validate_classified`(`crates/elwindui-codegen/src/validate.rs`)が**構造的**に分類する(PR #169レビュー是正、round 2)。診断を発行するヘルパー関数はすべて`errors: &mut ValidationErrors`という単一のsinkを引数に取り、その場で`.item_local(message)`か`.registry_dependent(message)`のどちらかを明示的に呼ぶ——`table.resolve`・`enum_variants`(全sibling moduleから構築)・`codegen::resolve_effective_fields`・same-crate Environment Key lookupのいずれかを経由する分岐は`registry_dependent`、componentの`fields`/`base`/自身のAST構造のみを見る分岐は`item_local`という基準で、61箇所の発行site全てを個別にタグ付けした。診断messageの文字列比較・複数回実行しての差分・部分文字列一致によるsubstring判定は一切行わない(round 1の`validate_classified`は挙動ベースの差分実装だったが、round 2レビューでcontract違反として指摘され、この構造的実装に置き換えられた)。返り値は分類済みの`Vec<ValidationDiagnostic>`(各要素が`ValidationDependency::{ItemLocal, RegistryDependent}`と`message`を持つ)であり、単一の`Result<(), Vec<String>>`へ折り畳む`validate::validate`はこの上に薄く被せた後方互換ラッパーに過ぎない。

この「発行siteごとの判断」という原則は、**ヘルパー関数の識別だけで一律に決めてはならない**——`resolve_effective_fields`のように一般にはsame-crate registryを参照し得る汎用ヘルパーを呼んでいても、*その特定の呼び出しの分岐*が実際にregistryへ触れているかどうかで判定する(PR #169レビュー是正、round 3、A1/AD-R3-1)。典型例が`#[content(field_name)]`の未知fieldチェックで、`codegen::content_field_knowledge(from, c, name, modules)`(`ContentFieldKnowledge::{KnownPresent, KnownMissingItemLocal, NeedsSameCrateRegistry}`)が事前に判定する: `name`がcomponent自身の`c.fields`にあれば`KnownPresent`(registry未参照)。無ければ、baseが無いなら`KnownMissingItemLocal`(`resolve_effective_fields`のbase無し分岐はまさに`c.fields.clone()`そのものでregistry非依存)。baseがあっても`codegen::find_component_and_module`がそのbaseを`modules`(same-crate同名module検索)の中に見つけられなければ、`codegen::synthesize_external_base_fields`(`c`・base名・自身のview以外を一切参照しない、既存のexternal-base合成ロジック)の結果だけで判定できるので、これも`KnownMissingItemLocal`。baseが`modules`内に実在する`ComponentDef`として見つかった場合のみ、そのsibling自身の(再帰的な)effective fieldsに依存するため`NeedsSameCrateRegistry`となり、初めて`resolve_effective_fields`本来の同名field検索を呼んでその結果を`registry_dependent`へ回す。base無しの単純な`#[content]`typoをregistry依存として誤ってgateしていた旧実装(round 2以前)は、このsite単位の分岐判定に置き換えられた。

`lib.rs`の`ComponentGenerationFailure`は`ItemLocal(String)`/`RegistryDependent(String)`(struct/impl半分それぞれの個別チェック——`same_crate_control_target`、`lookup_same_crate_environment_key`、struct-before-impl registry miss、base qualified-path解決——は元々1診断1発行siteなのでそのまま)に加えて、`Classified(Vec<ValidationDiagnostic>)`を持つ(round 2、AD-R2-3)。`classify_validate_result`は`validate_classified`の結果を**折り畳まずそのまま**この`Classified`に包んで返し、呼び出し元(`generate_component_from_item_struct_with_template`/`generate_component_from_item_impl`)は`validation_diagnostic_tokens`で診断ごとに個別ゲートする——`ItemLocal`は無条件の生成`compile_error!`、`RegistryDependent`は`#[cfg(not(rust_analyzer))]`付きの`compile_error!`——を shadow と並べて`Ok(..)`で返す。1回の`validate_classified`呼び出しでitem-localとregistry依存の両方が同時に出た場合でも、どちらも欠落・混同されずそれぞれ正しくgateされ、shadowも破棄されない(旧実装は「1件でもitem-localがあれば全体を無条件`Err`」という worst-case への折り畳みだったため、shadowごと失われていた)。

`ComponentPublicShape`(`component_frontend.rs`)は、own field単位のconstructor/getter/setter surfaceに加えて`constructor_return: ComponentConstructorReturn`(`SelfValue`/`RcSelf`)を持つ(round 2、AD-R2-4/B1是正)——view-lessなComponentのreal `new(..)`(`codegen::generate_component`)はbare `Self`を返し、`has_view`なComponent(`codegen::generate_view`)は常に`std::rc::Rc<Self>`を返すという実際の非対称性を、`view.is_some()`から機械的に導出する。rust-analyzer struct shadow(`build_component_struct_shadow`)はこの`constructor_return`を直接読み、`new(..)`の戻り値型を独自に再判定しない。`component_public_shape`はさらに、`has_view`な(`view: Some(..)`な)Componentの own `on_*`-named no-initializer fieldを`#[routed]`callbackとしてconstructor/accessor surfaceから除外する(round 2、AD-R2-5)。同様に、view-lessなreal generator(`codegen::generate_component`)自身のown no-initializer field分類(deferred `Option<T>` vs required)も`component_public_shape`の`deferred_option_fields`を消費する(round 2、AD-R2-6)——real generatorとshadowが同じ分類結果を共有し、互いに独立して食い違うことがない。

**`component_public_shape`はsource-localのみを入力として受け取る(PR #169レビュー是正、round 3、A2/AD-R3-2)。** `component: &ComponentDef`引数は、そのComponentの`struct`宣言が文字通り書いた自分自身のfieldのみを持つ、un-flattenedな`ComponentDef`でなければならない——`codegen::resolve_effective_fields`/`info.effective_fields`が返す、ancestorのfieldまで再帰的に合成された「有効field列」を絶対に渡してはならない。Round 2時点では、`codegen::generate_view`の呼び出し元(`Item::Component`のdispatch arm)が構築する*synthetic*な`ComponentDef`(`fields: info.effective_fields.clone()`)がそのままshapeへ渡っており、ancestorのfieldをこのComponent自身のfieldであるかのように分類してしまう構造的な誤りがあった——本来`component_public_shape`はsource-localな分類器としてのみ設計されている(この節の冒頭、および`component_public_shape`自身のdoc commentが元々明言していた契約)。Round 3で是正: `codegen.rs`の`generate_component`/`generate_view`はどちらも2つの`ComponentDef`を明示的に別引数として受け取る——文字通りのsource(`source_component`、`Item::Component(c) => { .. }`のdispatch armが持つ元の`c`をそのまま渡す)と、ancestor合成済みのeffective(`c`/`component`、これまで通りsynthetic)。`component_public_shape(source_component, Some(view))`は必ず前者から呼ぶ。own-field API membership(constructor param/deferred/readable/writable/`on_*`除外/constructor_return)はshapeが単独の真実源であり続けるが、inherited fieldのforwarding・storage・parameter-slicing(`forward_param_names`/`base_param_count`/`is_inherited_view_composition`等)は一切shapeを経由せず、これまで通りeffectiveな`component`/`c`から直接組み立てる——`source_component.fields`に無い名前(=inherited)は、`generate_view`の`is_param_eligible`/`is_deferred_own_field`/`mutable_required_names`いずれも、round 2以前から存在する直接判定(`strip_option`/`FieldKind::Prop`チェック等)へフォールバックする、という明示的な境界を持つ。`generate_component`(view-less)も同じ境界を`declared_own_field_names`(`source_component.fields`由来)で持つ。

`component_public_shape`は`view: Option<&ViewDef>`を受け取り、own `Option<T>`fieldの deferred/required 判定を、real generation(`codegen::generate_view`)と同じ`codegen::view_references_name_anywhere`traversalで行う——PR #169レビュー指摘A2: 無属性(`#[param]`を書かない)fieldは`FieldKind::Prop`になるが、初期値なしの`Prop`もrequired constructor paramになる実際のルールを、旧実装は`FieldKind::Param`のみの対象と誤認しており、通常fieldをshadowのconstructorから丸ごと落としていた。さらに、required(非deferred)な`Prop`fieldはconstruction後もruntime-mutableのため、`has_view`な実装(`generate_view`の`mutable_required_names`、`FieldKind::Prop`のみが対象)ではsetterを持つ一方、view-lessな実装(`generate_component`)は同じrequired fieldにsetterを一切持たない——この実装差そのものをshadow側でも再現する(`view.is_some()`で分岐)。Round 3では、この`mutable_required_names`自体もown fieldについては`component.fields.iter().any(f.kind==Prop)`という独立した再判定ではなく、`own_shape.writable_fields`への所属チェックへ置き換えた(AD-R3-5)——`required_own_names`が既に「(own fieldなら)shapeのconstructor_paramsに含まれる=required」を保証しているため、`writable_fields`との交差がそのまま「required かつ writable」を表す。

**source-localなfieldについて、`ComponentPublicShape`は単なる並行記述ではなく、API membershipの唯一の権威である(PR #169レビュー是正、round 4、A/AD-R4-1)。** rust-analyzer shadowとreal generatorのどちらも、own fieldが「constructor引数を持つか」「getterを持つか」「setterを持つか」「そのvisibilityが`pub`か非`pub`か」を、`ComponentPublicShape`の4つの surface(`constructor_params`/`deferred_option_fields`/`readable_fields`/`writable_fields`、visibilityは`readable_fields`/`writable_fields`が持つ`ShadowVisibility`)から読む。`FieldDef`(`f.kind`/`f.initializer`/`strip_option`等)は、shapeが既に決定した「APIが存在するかどうか」を一切左右せず、「そのAPIをどう実装するか」(storageがCellかRefCellか、getter/setter本体、`recompute_<name>`のcompute式、property-change通知呼び出し)だけを選ぶ。Round 3時点では`codegen::generate_component`(view-less path)がこの原則の唯一の未達成点で、own no-initializer fieldのdeferred/required区分だけがshape駆動(round 2、AD-R2-6)で、それ以外(constructor param membership自体・getter/setter existence・visibility)は依然`f.initializer`/`f.kind`から独立に再判定されていた。Round 4で是正: `generate_component`は`own_field_shape`から`own_constructor_params`/`own_deferred_fields`/`own_readable_fields`/`own_writable_fields`という4つの`HashMap`索引を一度だけ構築し(`declared_own_field_names`で判定される)own fieldのすべての`match &f.initializer`分岐で、既存のstorage/body生成ロジックはそのまま保ちつつ、accessorのexistenceとvisibilityだけをこれらの索引から読むよう書き換えた——`ShadowVisibility::Public`は`pub fn`、`ShadowVisibility::Private`は`fn`という固定写像は`rust_analyzer_shadow::shadow_vis_tokens`(`pub(crate)`化してreal generator側からも再利用)が担う。inherited field(`declared_own_field_names`に無い名前)はこの索引を一切参照せず、round 3以前からの直接判定(`strip_option`/`FieldKind::Prop`チェック等)へ完全にフォールバックする(AD-R4-8)——この境界のおかげで、既存220件超のtestが1件も挙動変化なく通過した(APIの表面はRound 3時点と完全に同一のまま、"誰が決めているか"だけが単一の権威に統合された)。

**own fieldについて、`writable_fields`はsetterの存在・パラメータ型・visibilityすべての権威である(PR #169レビュー是正、round 5、AD-R5-1)。** Round 4の`own_writable_fields: HashMap<&str, (&str, ShadowVisibility)>`はtupleの型要素(setterのパラメータ型)を`if let Some((_, visibility)) = ...`という形で握りつぶしており、own setterのシグネチャは依然として`#inner_ty`(deferred own field、`strip_option`/`own_deferred_fields`由来)または`#ty`(defaulted Prop/State own field、`f.ty`由来)という第二の型ソースから組み立てられていた——実際の値は今日のところ`writable_fields`の型と一致するが、real generatorの中に「setterの型を決める」ポリシーが構造的に2つ存在する状態だった。Round 5で是正: `own_writable_fields.get(name)`から取れる`setter_ty_str`を`syn::Type`としてparseし、setterの`value: #setter_ty`パラメータそのものに使う——deferred own fieldの`#set_body`(`Some(value)`でラップする代入本体)・defaulted Prop/Stateの`#set_body`(Cell/RefCell代入+recompute cascade)は一切変更していない、これらは実装機構(AD-R5-2)であって、shapeが決めるAPI表面(パラメータの型そのもの)ではないため。既存の全testが1件も挙動変化なく通過し、生成される公開API自体はRound 4時点と完全に同一である。

`#[elwindui::control_template]`は独自のRA shadow(`rust_analyzer_shadow::build_control_template_shadow`)を持つ——PR #169レビュー指摘A3: 隠しComponent struct半分の戻り値(RA shadowを含む)を`generate_control_template_from_item_struct`が破棄していたため、隠しComponent impl半分のRA shadowが宣言されない型を参照する状態になっていた。現在は隠しstruct半分の戻り値を保持しつつ、`TemplateName::template()`の公開宣言自体も`gate_real_items_for_rustc`で`cfg(not(rust_analyzer))`へ隔離し、専用のsignature-onlyなRA shadow(`__new_unmounted`/`mount`/`into_node`等、汎用Component shadowが決して持たないruntime専用methodを一切呼ばない)を別途生成する。

真に外部(ローカル`TypeInfo`なし)なbaseを`inherits`し、自前の`view`内でbaseの属性を同名のまま裸参照している(`padding: padding`)component(Refs #90)については、baseの完全な field 一覧を持たないため、`resolve_effective_fields`は代わりにview自身の裸参照を唯一の証拠として当該fieldを合成する(`codegen.rs`の`synthesize_external_base_fields`)。合成されたfieldの型は具体的なRust型文字列ではなく、`{Base}!(@field_type {name})`という型位置macro呼び出し文字列——実際の型解決はconsumer crate側での`__elwindui_props_*!`展開(`class_macro_design.md`)まで遅延する、この節の冒頭の方針そのものの型情報版である。合成fieldの宣言元(`declaring_types`)は、辿れる祖先が存在しない以上、component自身とする。

### 3.3 Static validation

validatorは [`dsl_spec.md`](../../specs/dsl_spec.md) のcompile-time ruleをASTに適用する。代表的な対象は次である。

- property、event、binding targetの存在と型
- Once、OneWay、TwoWayの書込み可能性
- `#[param]`、`#[prop]`、`#[state]` の利用制約
- `for`、`if`、`match` など動的領域の構造
- DSL enumについてmacro展開時に判定できる網羅性
- content fieldの形状、native/virtual root互換性、shortcut targetと配置などの構造制約
- replaceable template内の`#[id]`、複数`ContentPresenter`、dynamic region内`ContentPresenter`

macro processで完全に解決できないRust型やpathは、生成するRust構文によってrustcのtype checkとpattern exhaustiveness checkへ引き継ぐ。正しさを隠す合成的なwildcard armは生成しない。

ControlTemplateのcross-crate target、Environment Key Value、`templated_parent` getter、
`ContentPresenter` targetはそれぞれ生成した`ControlExt`、型一致、method resolution、`ContentControlExt` boundで検査する。

### 3.35 Deferred view lowering (`context_popup: view! { .. }`, #162)

`context_popup`属性に`ViewTemplate`型の通常式ではなく裸の`view! { .. }`ブロックが与えられた場合、staticvalidation(3.3、`check_vm_references`をenclosing scopeの`vm_fields`/`component_name`/`table`で再利用して検証済み——validationは常に元の(lowering前の)`ViewExpr::DeferredView`ノードに対して行われ、enclosing lexical Componentのscopeに対する検証が可能)の直後、`codegen::build_symbol_table`より前に、`lower_deferred_views_in_module`が該当moduleを一度だけ走査し変換する。この変換は新しいruntime binding機構を導入せず、既存のComponent/View構築pipelineへ委譲する:

- 見つかった各`view! { .. }`ブロックを、独立した隠しComponent/View pair(`ContentControl`基底、`__ElwinduiViewTemplateInstanceFor<Owner>_<ordinal>`という決定的な名前)として抽出する。この隠しComponentは唯一の合成field `#[param] __view_owner: Weak<SourceComponent>`を持ち、`ViewDef::implicit_owner = Some(ImplicitOwnerDef { field_name: "__view_owner", readable_fields, writable_fields, reactive_fields, bindable_fields })`としてmarkされる(PR #165 final rereview remediation, A2、`reactive_fields`/`bindable_fields`はPR #165 post-final rereview remediation, A8/A9で追加)。`SourceComponent`は常に*元の*lexical source Componentであり、nesting深度に関わらず不変(`DeferredViewExpr::lexical_owner`、PR #165 A3)——`context_popup`の中にさらに`context_popup: view! { .. }`が入れ子になっている場合でも、両方の隠しComponentが同じ`SourceComponent`を`__view_owner`の型として持つ。schemaの4集合すべてがnesting深度に関わらず同一である(`codegen::implicit_owner_schema`がlowering前に一度だけ計算し、以降すべてのnesting levelへそのまま伝播する — 各levelの隠しComponent自身の(ほぼ空の)field listから再計算することは決してない)。生成された隠しComponent自身の名前だけがnesting levelごとに変わる。4集合は`SourceComponent`の*effective*field list(継承分を含む、`resolve_effective_fields`)から導出する: `readable_fields`は`Prop`/`State`/`Param`/`Computed`/`Environment`、`writable_fields`は`Prop`/`State`のみ、`reactive_fields`は`Prop`/`State`/`Computed`/`Environment`(`Param`は構築後に再代入されずPropertyChanged variantを持たないため除外——`generate_view`自身の`component_property_variants`構築が実際に検証済みの根拠)、`bindable_fields`は`Attr::Bindable`を持つfield(常に`FieldKind::Param`、`TypeInfo::bindable_fields`をそのまま再利用)。`Attached`(実体を持たないschema宣言)はいずれの集合にも含まれない。
- 元の`context_popup`属性値は、抽出した隠しComponent名を参照する`ViewExpr::DeferredView`markerへ置き換わる。
- 隠しComponentの本体のうち、通常の`view!`属性値・要素構築(DSL grammarの一部)は変換なしにそのまま既存pipelineへ流れる — 3.3のvalidation、3.4のdependency analysis、3.5のcode generationのいずれも、この隠しComponentを他の通常Componentと区別する特別な分岐を必要としない。唯一の例外は`__view_owner`(`implicit_owner.is_some()`)を`ControlTemplate`の`templated_parent`と同様にweak-owner/Environment伝播対象として扱う既存分岐(`is_replaceable_template_body`)であり、これも`templated_parent`向けに既に存在する仕組みの一般化であって新設ではない。DSL属性値内の裸名解決(`emit_expr`の`ViewExpr::Path`分岐)自体も、`implicit_owner.readable_fields`に実際に含まれる名前だけをowner fallback対象とする——下記のraw Rustパスと同じmembership判定を共有する。
- 一方、`on_mount`/`on_unmount`/`on_update`ブロックとevent handler closureの本体は、DSL grammarではなく**任意のRust文**であり、`view!`の属性値resolutionとは別の`syn::visit_mut::VisitMut`パス(`ViewClosureRewriter`)で書き換えられる。裸の1segment名の解決順序は次の通り:①現在のlexical scope stack(`let`/`if let`/`while let`/`match`/`for`/nested closureのbindingを実際のRust scopingと同じ深さで追跡する`ViewClosureRewriter::scopes`)上の実local/closure parameter、②隠しComponent自身のfield、③`implicit_owner.readable_fields`に含まれる既知のsource-owner field(`resolved_implicit_owner_field`、`<owner>.field()`へ変換)、④それ以外は通常のRust名としてそのまま残す。raw Rustは`view!`のDSL grammarと異なり任意のネストしたscopeを持ちうるため、単一のblock全体に対するflatなshadow setではなく、実際のRust lexical scopingに従うscope stackで追跡する——block-wide flatなmodelは、同一block内でouter fieldの読み取りと同名localのshadowingが混在するケースで意味論を変えてしまう既知のバグを持っていた(PR #165 review remediation round 1)。さらに、`implicit_owner`が設定されていて①②に該当しないというだけで無条件にowner fallbackするmodelは、そのComponentのfieldでも何でもない自由なRust名(module定数、`None`、他所のfree function呼び出し等)まで誤って`__view_owner`経由のgetter呼び出しに書き換えてしまう欠陥を持っていた(PR #165 final rereview remediation, A2)——③のmembership判定はこれを防ぐ。書込み(`x = rhs`という代入の左辺が裸の1segment名の場合)も同じ優先順位で解決する: 隠しComponent自身のmutable own fieldなら`self.set_x(rhs)`、それ以外で`implicit_owner.writable_fields`に含まれる場合(`Prop`/`State`のみ)は`resolved_implicit_owner_setter`経由で`<owner>.set_x(rhs)`、どちらでもなければ通常のRust代入としてそのまま残す。

**PR #165 post-final rereview remediation、A8**(source-qualified 2segment path): 上記①〜④は裸の1segment名の話であり、`vm.label`/`vm.save`のような2segment pathには別の欠陥があった——`emit_path_get`/`emit_setter`は元々owner segment(`vm`)を無条件に`self.vm`(または裸の`vm`、construction mode時)として emitしていたため、隠しComponent自身が物理的に`vm` fieldを持たない場合(常にそう)、構文的には正しいが`rustc`が「no field `vm`」で拒否する不正なコードを生成していた(`assert_valid_rust`の`syn::parse2`のみのcheckでは検出できない)。共有resolver `path_owner_value_tokens`が両関数の前段に入り: ①`owner` segmentが現在生成中Component自身の実fieldなら(`ctx.own_fields`、`__view_owner`自身や`ControlTemplate`の`templated_parent`を含む)従来通り、②実fieldでなくかつ`implicit_owner.bindable_fields`に含まれるなら`__view_owner`をupgradeしその`vm()` getterを経由してbridgeする(`<upgraded>.vm().label()`)、③どちらでもなければ元の(無条件)挙動へfall backする。raw Rust側(`ViewClosureRewriter`)の2segment `Expr::Path`分岐にも対称的な`resolved_implicit_bindable_owner`を追加している。

**PR #165 post-final rereview remediation、A9**(direct source-field/qualified-pathのreactive追跡): 依存追跡3関数(`collect_view_expr_owner_properties`/`view_expr_has_reactive_dependency`/`view_expr_depends_on`)は元々`ctx.mutable_own_fields`(隠しComponent自身のmutable own field、通常ほぼ空)しか見ておらず、直接の裸source field参照(`TextBlock { text: label }`)もsource-qualified path(`vm.label`)も、生成時点で正しく読めても`__resync___view_owner`/`__resync_vm`のmatch armに一切登録されないため、popupが開いている間ライブ更新されなかった。3関数それぞれへ`implicit_owner.reactive_fields`(裸の1segment名を`(__view_owner, field)`という正準dependency identityへ変換)と`implicit_owner.bindable_fields`(2segment pathの`owner`側判定)を考慮する分岐を追加した——`format!`マクロの inline capture分岐も同様に拡張済み。`vm`のような論理bindable ownerは物理fieldではないため、既存の`bind_owners`(物理field専用)には追加できない——別途`implicit_bind_owners`(`ctx.own_fields`に同名の実fieldが無い`bindable_fields`のみ)を導出し、`property_resync_methods_for`を*変更なしに*再利用して(この関数自体は`owner_name`を単純な文字列一致でしか見ておらず、物理fieldかどうかを区別しない)`__resync_vm`相当のmethodを生成し、`subscribe_stmts`側にのみ新しいbridging code(`__view_owner`をupgrade→`.vm()`→その値へ`ObservableExt::subscribe_property_changed`)を追加している——第二の購読engineではなく、既存機構の並行適用である。

`context_popup`の代入site自体では、`ViewExpr::DeferredView`から`ViewTemplate::new(move |ctx| { .. })`を生成する(`docs/design/runtime/view_template_design.md` §2の`ViewTemplate`をそのまま利用)。生成closureがenclosing lexical ownerのweak参照を復元する方法は、このfactory式が実際にemitされる場所によって2通りある(PR #165 A3):トップレベル(`ctx.implicit_owner`が`None`、つまりこのfactoryが真のlexical owner自身の生成コード内でemitされる場合)では`self.__self_weak`(`__build_view`の`__most_derived` localと同じ復元手順)からdowncastして復元するが、nested(`ctx.implicit_owner`が`Some`、つまり別の隠しComponentの内部でemitされる場合)では`self`自体が真のlexical ownerとは異なる型のインスタンスであるため`__self_weak`downcastは使えず、代わりに外側の隠しComponent自身が既に保持している`self.__view_owner.clone()`をそのまま再利用する。いずれの経路でも、popup open毎に隠しComponentの新しいinstanceを`__new_unmounted`→`mount`する。詳細な実行時sequenceは`docs/design/runtime/popup_context_menu_design.md`の該当節を参照する。

`context_popup`のように`elwindui-codegen`内にlocal `TypeInfo`を持つ対象(このcrate自身のtest fixture)向けの代入は、`is_option`に基づき`factory`または`Some(factory)`を直接emitする(3.5のlocal-TypeInfo経路)。一方、実際の`TextBlock`/`Window`など real builtin(local `TypeInfo`を持たない)向けの代入は、`__elwindui_props_{Type}!(@field_type {field})`という既存のcross-crate field-type transport(Refs #90、`synthesize_external_base_fields`と共有)を通じて実際の宣言型を読み取り、`elwindui::core::ui::__coerce_deferred_view_assignment_target::<@field_type ...>(factory)`へ変換する(PR #165 A4)。この関数は`ViewTemplate`/`Option<ViewTemplate>`のみを実装するsealed trait `DeferredViewAssignmentTarget`の`from_view_template`を呼び出し、宣言された型が受け付けない場合は`#[diagnostic::on_unimplemented]`付きのcompile-time errorとなる——`Some(factory)`を無条件でemitすることは決してない(型がまだ分かっていない段階で形を決め打ちしないため)。

### 3.4 Dependency analysis

binding式からdependencyを抽出し、initial assignment、sourceからtargetへのsubscription、必要ならtargetからsourceへのwrite-backを生成できる形へ正規化する。

動的 `for` のitem bindingでは、安定したitem identityと書込み可能fieldを検証する。生成したsubscriptionは各dynamic childが所有し、childの削除・置換と同時にdropされる。汎用runtime Binding objectへ意味論を移さず、静的に解決した型付き経路を生成する。

### 3.5 Code generation

generatorは検証済みASTから次を組み立てる。

- componentのconstruct、property初期化、child tree構築
- event wiringとtyped callback
- dependency subscriptionとlifetime ownership
- dynamic regionの生成、更新、reconciliation呼出し
- viewmodel actionとDSL enum連携に必要なRust構文

event名とpayload型は宣言metadataから導出する。特定event名をcode generatorへ追加して意味を決める方式は採らない。

#### Effective content lowering

裸の子要素の格納先は、要素の実効 `#[content(field)]` metadata と、その field の型だけから
決める。generator は型名 (`Control` など) を検査して lowering 経路を選択してはならない。

```text
effective #[content(field)] metadata
        + field type
        -> scalar or collection lowering
```

scalar field は静的にちょうど一つの子を検証し、`set_<field>(child)` へ lower する。
collection field は宣言された collection surface へ順序を保って挿入し、既存の dynamic
reconciliation を使う。`Control` は内部 scalar `visual_root` content destination を宣言し、
setter が private `template_root` replacement path へ委譲するが、この runtime 実装は generic
lowering からは見えない。`Layout` の `children`、`ContentControl` の `content`、将来の
specialized control の typed `children` はすべて同じ metadata-driven rule を通る。

#### Component override bridge

`#[elwindui::component]` の companion `impl` にある `#[overridable]` / `#[overrides]` は
`ComponentDef.methods` / `MethodDef` の metadata として保持し、view を持つ component の
生成時に、合成された `#[class]` impl の同じ method itemへ再付与する。これは component
専用のvtableやmethod-name/type-name dispatchを追加する処理ではない。生成された class implを
`#[class]` macroへ渡し、ancestor Ext trait / `UIElementExt` の既存override chainを唯一の
runtime authorityとして利用する。viewを持たないcomponentの通常のcomponent override lowering
も同じmetadata semanticsを保ち、component-to-componentの既存挙動を変更しない。生成された
component型に残る同名のinherent compatibility methodは、既存の具象component APIを保つための
直接呼出し面であり、hostのtrait-object dispatchや別のoverride chainではない。

## 4. Diagnostic設計

各AST nodeは可能な限り元tokenのspanを保持する。parser、resolver、validatorのerrorは、原因となるDSL tokenまたはRust attributeへ関連付ける。複数の独立したerrorを収集できる場合はまとめて返すが、invalid ASTから意味を推測してgenerationを継続しない。

公開されるerror条件と意味はspecが定め、本書はそのerrorを安定して生成する内部経路だけを定める。

## 5. 他ツールとの境界

Language Serverはparserとvalidationを再利用できるが、document lifecycle、incremental analysis、editor protocolはLSP側が所有する。previewとhot reloadは検証済みcomponentまたは生成物を利用するが、codegen pipelineへ実行中applicationの状態を持ち込まない。

`#[class]` macroは別pipelineであり、token rewriting、継承metadata、rust-analyzer向け展開上の配慮は [`class_macro_design.md`](class_macro_design.md) が定める。

## 6. Invariants

- 公開DSL semanticsは [`dsl_spec.md`](../../specs/dsl_spec.md) から導出する。
- backend固有APIやbackend選択を生成コードへ埋め込まない。
- event payloadやproperty typeを名前のhard-codeで決めない。
- validationをruntime panicへ遅延させない。
- subscriptionのownerとdrop境界を生成時に明示する。
- LSP、preview、hot reloadの状態をcompiler内部へ持ち込まない。
