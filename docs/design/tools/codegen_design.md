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

- 見つかった各`view! { .. }`ブロックを、独立した隠しComponent/View pair(`ContentControl`基底、`__ElwinduiViewTemplateInstanceFor<Owner>_<ordinal>`という決定的な名前)として抽出する。この隠しComponentは唯一の合成field `#[param] __view_owner: Weak<SourceComponent>`を持ち、`ViewDef::implicit_owner = Some(ImplicitOwnerDef { field_name: "__view_owner", readable_fields, writable_fields })`としてmarkされる(PR #165 final rereview remediation, A2)。`SourceComponent`は常に*元の*lexical source Componentであり、nesting深度に関わらず不変(`DeferredViewExpr::lexical_owner`、PR #165 A3)——`context_popup`の中にさらに`context_popup: view! { .. }`が入れ子になっている場合でも、両方の隠しComponentが同じ`SourceComponent`を`__view_owner`の型として持つ。`readable_fields`/`writable_fields`もnesting深度に関わらず同一のschemaを共有する(`codegen::implicit_owner_schema`がlowering前に一度だけ計算し、以降すべてのnesting levelへそのまま伝播する — 各levelの隠しComponent自身の(ほぼ空の)field listから再計算することは決してない)。生成された隠しComponent自身の名前だけがnesting levelごとに変わる。`readable_fields`/`writable_fields`は`SourceComponent`の*effective*field list(継承分を含む、`resolve_effective_fields`)から導出し、`Prop`/`State`はreadable+writable、`Param`/`Computed`/`Environment`はreadable-only、`Attached`(実体を持たないschema宣言)は対象外とする。
- 元の`context_popup`属性値は、抽出した隠しComponent名を参照する`ViewExpr::DeferredView`markerへ置き換わる。
- 隠しComponentの本体のうち、通常の`view!`属性値・要素構築(DSL grammarの一部)は変換なしにそのまま既存pipelineへ流れる — 3.3のvalidation、3.4のdependency analysis、3.5のcode generationのいずれも、この隠しComponentを他の通常Componentと区別する特別な分岐を必要としない。唯一の例外は`__view_owner`(`implicit_owner.is_some()`)を`ControlTemplate`の`templated_parent`と同様にweak-owner/Environment伝播対象として扱う既存分岐(`is_replaceable_template_body`)であり、これも`templated_parent`向けに既に存在する仕組みの一般化であって新設ではない。DSL属性値内の裸名解決(`emit_expr`の`ViewExpr::Path`分岐)自体も、`implicit_owner.readable_fields`に実際に含まれる名前だけをowner fallback対象とする——下記のraw Rustパスと同じmembership判定を共有する。
- 一方、`on_mount`/`on_unmount`/`on_update`ブロックとevent handler closureの本体は、DSL grammarではなく**任意のRust文**であり、`view!`の属性値resolutionとは別の`syn::visit_mut::VisitMut`パス(`ViewClosureRewriter`)で書き換えられる。裸の1segment名の解決順序は次の通り:①現在のlexical scope stack(`let`/`if let`/`while let`/`match`/`for`/nested closureのbindingを実際のRust scopingと同じ深さで追跡する`ViewClosureRewriter::scopes`)上の実local/closure parameter、②隠しComponent自身のfield、③`implicit_owner.readable_fields`に含まれる既知のsource-owner field(`resolved_implicit_owner_field`、`<owner>.field()`へ変換)、④それ以外は通常のRust名としてそのまま残す。raw Rustは`view!`のDSL grammarと異なり任意のネストしたscopeを持ちうるため、単一のblock全体に対するflatなshadow setではなく、実際のRust lexical scopingに従うscope stackで追跡する——block-wide flatなmodelは、同一block内でouter fieldの読み取りと同名localのshadowingが混在するケースで意味論を変えてしまう既知のバグを持っていた(PR #165 review remediation round 1)。さらに、`implicit_owner`が設定されていて①②に該当しないというだけで無条件にowner fallbackするmodelは、そのComponentのfieldでも何でもない自由なRust名(module定数、`None`、他所のfree function呼び出し等)まで誤って`__view_owner`経由のgetter呼び出しに書き換えてしまう欠陥を持っていた(PR #165 final rereview remediation, A2)——③のmembership判定はこれを防ぐ。書込み(`x = rhs`という代入の左辺が裸の1segment名の場合)も同じ優先順位で解決する: 隠しComponent自身のmutable own fieldなら`self.set_x(rhs)`、それ以外で`implicit_owner.writable_fields`に含まれる場合(`Prop`/`State`のみ)は`resolved_implicit_owner_setter`経由で`<owner>.set_x(rhs)`、どちらでもなければ通常のRust代入としてそのまま残す。

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
