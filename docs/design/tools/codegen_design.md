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

### 3.2 Registry resolution

同一crate内のcomponent、viewmodel、DSL enumのmetadataはmacro展開中のregistryへ登録する。後続の展開はregistryから宣言済みmetadataを参照するため、DSLのcross-item検証は宣言順に依存する。

Rust pathそのものの最終的な名前解決は生成後のrustcへ委譲する。別crateの型情報をmacro process内registryへ複製しない。

### 3.3 Static validation

validatorは [`dsl_spec.md`](../../specs/dsl_spec.md) のcompile-time ruleをASTに適用する。代表的な対象は次である。

- property、event、binding targetの存在と型
- Once、OneWay、TwoWayの書込み可能性
- `#[param]`、`#[prop]`、`#[state]` の利用制約
- `for`、`if`、`match` など動的領域の構造
- DSL enumについてmacro展開時に判定できる網羅性
- content fieldの形状、native/virtual root互換性、shortcut targetと配置などの構造制約

macro processで完全に解決できないRust型やpathは、生成するRust構文によってrustcのtype checkとpattern exhaustiveness checkへ引き継ぐ。正しさを隠す合成的なwildcard armは生成しない。

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
