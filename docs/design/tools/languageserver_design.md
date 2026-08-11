# ElwindUIL Language Server設計

本書は `elwindui-languageserver` のdocument lifecycle、incremental analysis、editor protocol境界を定める。DSL contractは [`dsl_spec.md`](../../specs/dsl_spec.md)、compiler pipelineは [`codegen_design.md`](codegen_design.md)、実装状況は [`../../status/tooling_status.md`](../../status/tooling_status.md)を参照する。

## 1. 責務

Language Serverは次を担当する。

- workspace内Rust documentのopen/change/save/close状態を管理する
- ElwindUIL attributeと `view!` 範囲を抽出する
- codegenのparserとvalidatorを再利用してdiagnosticをpublishする
- DSL contextに応じたcompletion、hover、semantic tokenを返す
- preview要求をpreview subsystemへroutingする

Rust全体のtype check、build、backend実行はrustc、Cargo、各toolへ委譲する。

## 2. Analysis pipeline

```text
LSP document event
       ↓
document snapshot and affected-range selection
       ↓
Rust syntax parse and ElwindUIL item extraction
       ↓
codegen frontend / validation reuse
       ↓
diagnostics and editor features
```

document snapshotにはversionを付け、古いanalysis結果を新しいversionへpublishしない。syntax errorでfull ASTを構築できない場合も、取得可能なrangeとtokenから限定的なeditor responseを返せる構造にする。

## 3. Compilerとの共有境界

parser、AST、validation ruleをLSP用に再実装しない。compilerと共有する層はfile I/O、LSP type、backend handleを含まず、token、AST、diagnostic spanで入出力する。

macro expansion時にしか取得できないcross-item情報は、workspace indexが同等のmetadataを供給する。確定できない型やpathを推測で有効扱いせず、rustc diagnosticと競合しない補助情報として扱う。

## 4. Editor feature

- Diagnostic: DSL syntax、name、binding direction、compile-time constraintを元rangeへ対応付ける
- Completion: component property、event、ViewModel member、enum memberをcontextで絞る
- Hover: public typeとcontractへの短い説明を返し、内部implementation detailを公開APIとして表示しない
- Semantic token: `view!` 内のelement、property、event、binding、control-flowを分類する
- Generated-code view: debug用途のvirtual documentとして生成物を提示し、source documentとして編集させない

featureは同じanalysis snapshotを共有し、相互に異なるname resolution結果を作らない。

## 5. Preview連携

preview要求は対象component、document version、検証済みmetadataを [`preview_design.md`](preview_design.md) の境界へ渡す。render processやWebView stateをLanguage Serverのanalysis stateへ混在させない。preview側のfailureはcompiler diagnosticと区別したtool diagnosticとして返す。

## 6. Invariants

- DSL semanticsの正本はspecとcodegen validatorであり、LSP独自ruleを追加しない。
- stale document versionの結果をpublishしない。
- editor request処理でnative backend objectを生成しない。
- workspace indexはsource metadataから再構築可能にする。
- featureの対応状態とgapはdesignではなくtooling statusに記録する。
