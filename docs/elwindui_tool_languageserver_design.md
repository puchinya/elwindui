# ElwindUIL LanguageServer(`elwindui-languageserver`)設計書

本書は、ElwindUILの言語サーバー `elwindui-languageserver` の設計を定める。ツールチェーン全体のアーキテクチャ概観は `docs/elwindui_tool_codegen_design.md` §6を参照。

## 1. スコープ

`elwindui-languageserver` は `.elwind` ファイルを対象とする専用言語サーバー(LSP)であり、エディタ(VSCode等)に対して以下を提供する裏方プロセスである。

- 入力中からの即時診断(制約違反、enum網羅漏れ、`#[param]`への`bind!`混入など)
- 生成されるRustコードのプレビュー表示
- enumメンバー等にホバーした際の、Fluentメッセージ(`t!`)の解決結果表示

プレビューパネル(WebView)自体の表示方式(静的/インタラクティブ/ホットリロード反映の3段階のUI)の詳細設計は `docs/elwindui_tool_preview_design.md` に譲る。本書はLanguageServerが「診断・型検査・制約検証・プレビュー用インスタンスの生成」までを担う範囲に限定する。コード生成(`build.rs`/proc-macro方式のコンパイラ本体)は `docs/elwindui_tool_codegen_design.md`、ホットリロード機構は `docs/elwindui_tool_hotreload_design.md`、DSL構文自体は `docs/elwindui_dsl_spec.md`、フレームワーク設計(`Element`トレイト等)は `docs/elwindui_gui_framework_design.md` を参照。

## 2. 全体アーキテクチャにおける位置づけ

ツールチェーン全体のアーキテクチャ図は `docs/elwindui_tool_codegen_design.md` §6を参照。`elwindui-languageserver` は、エディタからの保存イベントを起点に「増分パース・型検査・制約検証」(実装済み)と「プレビュー用インスタンス生成(既定値/モック)」(未実装、§4参照)を行う中核プロセスであり、その結果を①②のプレビューとしてWebViewへ送信する。実機での動作確認(③)が必要な場合のみ、下流の実行中アプリ(dylibホットリロード)へと連携が伸びる形になる。すなわちLanguageServerは、エディタ・プレビューパネル・ホットリロード対象アプリという3者をつなぐハブとして位置づけられる。

ツールチェーン層はいずれもDSLの言語仕様(`component`/`view`/`param`/`prop`/`Element`トレイト等)を変更せずに構築できるものと位置づけられており、LanguageServerも言語仕様に新たな構文を追加することはない。

## 3. 提供する診断・補完機能

以下の3つの機能を提供する。

1. **入力中からの即時診断**
   - 制約違反(`#[range]`/`#[length]`/`#[pattern]`/`#[format]`/`#[check]`等、7章)
   - enumの網羅漏れ(`match`が全メンバーを網羅していない、8章)
   - `#[param]`フィールドへの`bind!`混入など、param/propの静的評価式ルール違反(4章)
   - その他、14章「静的検証ルール一覧」に列挙された全項目(ルール1〜24)。これらはコンパイラ/リンタが実行前に検出すべき項目として定義されており、`elwindui-languageserver`はその実行環境をエディタ内でリアルタイムに提供する役割を担う。ルール個々の詳細(何が違反でどの付録が根拠か)は `docs/elwindui_gui_framework_design.md` を参照。
2. **生成されるRustコードのプレビュー表示**
   - コード生成器(`elwindui-codegen`)が出力するRustソースに相当する内容をエディタ上でプレビュー表示する。
3. **ホバー情報**
   - enumメンバー等にホバーした際、Fluentメッセージ(`t!`)の解決結果を表示する(11章のi18n仕組みと連動)。

**実装状況(Phase 1)**: 上記1(即時診断)は`elwindui_codegen::{parser, validate}`をそのまま再利用する形で実装済み(`src/diagnostics.rs`)。2(コードプレビュー)・3(ホバー)は未実装で、後続フェーズの範囲。

なお上記3機能には含まれていないが、Phase 1実装の一環として以下の2機能も追加した(実装が先行した追加機能):
- `textDocument/semanticTokens/full`によるシンタックスハイライト(キーワード/型名/文字列/数値/コメント/`#[...]`属性・`bind!`/`t!`マクロの色分け、`src/semantic_tokens.rs`)。これは`parser.rs`のAST(位置情報を持たない)を経由せず、ソーステキストを直接走査する専用の軽量トークナイザで実現している——`ast.rs`全体にspan情報を持たせる大掛かりな変更を避けつつ、色分け表示という実用上のニーズに応えるための独立した実装である。
- `textDocument/completion`による`vm.field`のメンバー補完(`src/completion.rs`、`elwindui_codegen::codegen::SymbolTable::resolve`を利用)。アクションも他のフィールドと同じ1階層の補完で扱われる(`Command`型が撤廃され、`.execute()`/`.can_execute`のような2階層の補完対象が無くなったため)。

まとめると、現在実装済みなのは「即時診断・シンタックスハイライト・メンバー補完」の3つであり、§4に述べる「プレビュー用インスタンス生成」パイプライン(オフスクリーンレンダリングを含む)およびホバー情報・生成コードプレビューは未実装。

## 4. 増分パース〜プレビュー用インスタンス生成のパイプライン

**実装状況の注**: 本章が述べる「component既定値でのインスタンス化」以降(オフスクリーンレンダリング・WebViewへの画像送信を含む)は未実装。現在実装済みなのは「増分パース(保存/変更イベントで再パース)→ 型検査・制約検証 → 診断のpublish」までであり、これは§3の1(即時診断)と同じ範囲にとどまる。以下は`docs/elwindui_tool_preview_design.md`のレベル①として定義されているフローであり、将来実装の対象。

`docs/elwindui_tool_preview_design.md`のレベル①(静的プレビュー)の処理フローとして定義されている通り、LanguageServer内部のパイプラインは以下の順で進行する。

```
.elwind保存 → LSPが増分パース → component既定値でインスタンス化
    → バックエンドのオフスクリーンレンダリング → WebViewへ画像送信
```

- **増分パース**: 保存イベントをトリガーに、変更箇所を中心に再パースする。
- **型検査・制約検証**: `docs/elwindui_dsl_spec.md`3章の`component`/`view`定義、7章の値制約、8章のenum網羅性検査などを実行し、診断結果(エラー/警告)をエディタに返す。
- **プレビュー用インスタンス生成**: `component`の既定値でインスタンス化する(①静的プレビュー向け)。②インタラクティブプレビュー向けには、`docs/elwindui_tool_preview_design.md`に定義される通り「`bind!(path, mode)`が使われている`prop`を自動検出し、プレビュー専用のコントロールUI(スライダー・テキスト欄等)に置き換える」モック化処理を行う。
- 生成されたインスタンスの実際の描画(バックエンドのオフスクリーンレンダリング)およびWebViewへの送信は、プレビューパネル側の責務との境界にあたる(詳細は`docs/elwindui_tool_preview_design.md`)。LanguageServerはレンダリング可能なインスタンス(既定値/モック値で構築された要素ツリー)を生成し引き渡すところまでを担う。

## 5. 他ツールとの連携インターフェース

- **コード生成器(`elwindui-codegen`)との関係**: LanguageServerは`.elwind`のパーサ・ASTをコード生成器と共有する(ビルド時自動生成と同一の解析基盤)。「生成されるRustコードのプレビュー表示」は、コード生成器が最終的に出力するのと同じASTから導出される。共有パーサ/ASTの具体的な実装分割は `docs/elwindui_tool_codegen_design.md` 側の設計に従う。
- **プレビューパネル(WebView)への送信内容**: ①静的プレビューでは既定値インスタンスのオフスクリーンレンダリング結果(画像)、②インタラクティブプレビューでは`bind!`参照先をモック化したインスタンスと、それを操作するためのコントロールUI情報を送信する。WebView側の受信・表示・操作UIの詳細は `docs/elwindui_tool_preview_design.md` を参照。
- **ホットリロードとの関係**: ③実行中アプリへの反映は、LanguageServerが直接担うものではなく任意経路として存在する(`docs/elwindui_tool_codegen_design.md`§6の図参照)。ホットリロードは`#[param]`変更時の再マウント、prop変更のみの場合の差分更新という、既存の`param`/`prop`区分をそのまま利用する仕組みであり、LanguageServerが行う型検査・制約検証の結果(検証済みであること)を前提として実行される。詳細設計は `docs/elwindui_tool_hotreload_design.md` を参照。

## 6. 非スコープ

- プレビューパネルのUI・3段階のプレビューレベルそのものの設計 → `docs/elwindui_tool_preview_design.md`
- `build.rs`/proc-macro方式によるコード生成の詳細 → `docs/elwindui_tool_codegen_design.md`
- `hot-lib-reloader`等を用いた動的ライブラリ差し替えの詳細 → `docs/elwindui_tool_hotreload_design.md`
- DSLの言語仕様そのもの(`component`/`view`/`param`/`prop`等) → `docs/elwindui_dsl_spec.md`、フレームワーク設計(`Element`トレイト等) → `docs/elwindui_gui_framework_design.md`
