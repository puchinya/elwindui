# ElwindUIL コード生成ツール(`elwindui-codegen`)設計書

本書は、`.elwind`ファイルをRustソースコードへ変換するコンパイラ本体(`elwindui-codegen`)の設計を定める。DSL構文そのものは`docs/elwindui_dsl_spec.md`、バックエンド抽象化・ランタイム等のフレームワーク設計は`docs/elwindui_gui_framework_design.md`を正とし、本書では「コンパイラというツール」の入出力・内部パイプライン・起動方式・実装トレードオフに焦点を当てる。

ElwindUILツールチェーン全体(本書・LSP・プレビュー・ホットリロード)のアーキテクチャ概観は§7を参照。

---

## 1. スコープ

### 1.1 責務

- `.elwind`ソースの構文解析・共通AST構築
- `docs/elwindui_dsl_spec.md`1〜15章・付録Aに定義された静的検証(14章の検証ルール一覧)の実行
- `#![backend(...)]` 指定・ビルドターゲットに基づく `target::backend()` の定数畳み込みと、非該当バックエンド分岐の静的除去(設計のみ、**未実装** — 下記実装状況の注を参照)
- バックエンド(WinUI 3/AppKit/GTK4)向けRustソースの生成
- 2つの起動方式(build.rs方式 / proc-macro方式)の提供

**実装状況の注**: 現時点の`elwindui-codegen`(`crates/elwindui-codegen/src/`)には`enum Backend`・`target::backend()`・`#![backend(...)]`属性のいずれも実装されていない。生成されるRustソースはバックエンド非依存(同一のソースがどのバックエンドクレートにもリンクできる)であり、実際にどのバックエンドを使うかは`elwindui`ファサードクレートのCargoフィーチャ(`backend-appkit`/`backend-winui3`/`backend-gtk4`)がどの`elwindui-backend-*`クレートをリンクするかだけで決まる(各バックエンドクレートが同名のビルトイン型を実装するため、コード生成器側でバックエンドごとに分岐する必要がない)。`docs/elwindui_gui_framework_design.md`§3.3が定義する定数畳み込み・分岐除去の仕組みは将来のフォワードルッキング設計であり、以下の記述はその設計を示すものである。

### 1.2 非責務(他ツールが担当)

- エディタ内の増分診断・ホバー表示・補完 → `docs/elwindui_tool_languageserver_design.md`(`elwindui-languageserver` LSP)
- 静的/インタラクティブプレビューのレンダリング → `docs/elwindui_tool_preview_design.md`
- 実行中プロセスへの差分反映(dylib差し替え) → `docs/elwindui_tool_hotreload_design.md`
- 言語仕様の意味論そのもの(param/propの区別、Element契約、bind!の意味等) → `docs/elwindui_gui_framework_design.md`

`elwindui-languageserver` はエディタ内診断のために `elwindui-codegen` と共通のパーサ・検証ロジックを再利用する想定だが、LSPプロセスとしての振る舞い(増分パース、プレビュー用インスタンス生成)は本書の対象外とする。

---

## 2. 入出力

| | 内容 |
|---|---|
| 入力 | `.elwind`ファイル(1つまたはディレクトリ配下一式) |
| 出力 | バックエンドごとのRustソースファイル(`.rs`) |
| 副作用 | build.rs方式では `OUT_DIR` 配下にファイルを書き出す。proc-macro方式ではコンパイル時にトークン列を直接展開する(中間ファイルなし) |

出力先ファイルは呼び出し側(`include!`マクロ、またはproc-macro展開位置)がRustモジュールとして取り込む前提とする。

---

## 3. パイプライン

```
.elwindソース
   │ ① 構文解析
   ▼
共通AST(フレームワーク非依存の要素ツリー)
   │ ② 静的検証(言語仕様14章のルール一覧)
   ▼
検証済みAST
   │ ③ (設計上)target::backend() の定数畳み込み・非該当分岐の除去 ※未実装、下記参照
   ▼
バックエンド非依存AST
   │ ④ Rustコード生成(バックエンド非依存)
   ▼
Rustソース(WinUI3/AppKit/GTK4のいずれのバックエンドクレートにもリンク可能)
```

- **①構文解析**: Rust構文に似た `.elwind` の字句・構文解析を行い、共通ASTを構築する。`use`宣言(12章)による他コンポーネントのインポートもこの段階で解決し、循環参照・未解決参照を検出する。
- **②静的検証**: 言語仕様14章に列挙された検証ルール(`#[param]`初期化式への`bind!`混入禁止、enum網羅性検査、制約違反の検出、`native!`の出現位置制限など)をASTに対して実行する。違反はビルド時エラーとしてコンパイルを停止させる。
- **③定数畳み込み(未実装)**: `docs/elwindui_gui_framework_design.md`§3.3は、`target::backend()`をビルド設定(Cargoのfeature/target triple)から一意に確定し、該当しない `match target::backend() { ... }` の腕や `#[cfg(backend = "...")]` 付き `native!` ブロックを生成対象から静的に除去する設計を定めているが、現在の`elwindui-codegen`にはこの段階が存在しない(`enum Backend`/`target::backend()`はコード中どこにも実装されていない)。実際には生成コードはバックエンドを問わず同一であり、この段階は素通りする。
- **④コード生成**: 検証済みASTから、バックエンドを問わず同一のRustコードを生成する。ビルトイン要素(`builtin::Window`/`Row`/`Text`等、`docs/elwindui_builtins_spec.md`付録F)は他コンポーネントと同じ`component`/`view`構文で書かれたリファレンス実装として同一パイプラインで処理される。生成コードが実際にどのバックエンドで動くかは、リンクされる`elwindui-backend-*`クレート(各バックエンドクレートが同名のビルトイン型を実装している)によって決まる——`docs/elwindui_gui_framework_design.md`§1・§3が想定する「バックエンドごとに異なるコードを生成する」段階は現状ここには存在しない。

---

## 4. 起動方式

コード生成器は2つの起動方式のいずれからも呼び出せる形で実装する。

### 4.1 build.rs方式

```rust
// build.rs
fn main() {
    println!("cargo:rerun-if-changed=src/ui");
    elwindui_codegen::compile_dir("src/ui", std::env::var("OUT_DIR").unwrap());
}
```

```rust
// main.rs
include!(concat!(env!("OUT_DIR"), "/notepad_window.rs"));
```

- `cargo:rerun-if-changed` により `.elwind` 保存後の次回ビルドで自動再生成され、手動コマンドが不要になる。
- `OUT_DIR` に生成済みRustソースが実体として残るため、IDE(rust-analyzer等)がそれを直接解析でき、生成コードに対する補完・型情報の精度が高い。

### 4.2 proc-macro方式

`component`/`view`を`.elwind`テキストとして書く代わりに、通常のRust `struct`定義として書く。
フィールドは`#[param]`/`#[prop]`等の属性を伴う通常のフィールドとして、`view { .. }`要素ツリーは
`view!`マクロ呼び出しを型に持つ1フィールドとして表現する。

```rust
#[elwindui::component(inherits Window)]
struct NotepadWindow {
    #[bindable]
    vm: std::rc::Rc<NotepadViewModel>,

    body: view! {
        title: vm.window_title
        content: VerticalLayout {
            TextArea { text: vm.content }
        }
    }
}
```

- `view!`は実在するマクロではなく、一度も展開されない。`#[elwindui::component]`(属性マクロ)が
  `struct`全体を丸ごと別のコードへ置き換えるため、内側の`view!`呼び出しはRustが実際に展開する
  対象には現れない ── マクロ呼び出しがRustの*型*位置において構文的に妥当(`field: some_macro! {
  .. }`は`syn::Type::Macro`としてパースされる)であることを利用したトリック。`view!`のトークンは
  `.elwind`テキストと同じ生のDSLテキストとして読み出され、既存のパーサ(`crates/elwindui-codegen/
  src/parser.rs`)へそのままかけられる。
- 中間ファイルを生成せず、コンパイル時にトークン列として直接展開する。
- ビルド構成がシンプルになる(`build.rs`の追加が不要)一方、生成コードが実ファイルとして残らないため、IDE補完の精度はbuild.rs方式に劣る。加えて`view!`の中身自体は、rust-analyzer自身による補完・型チェックの対象にもならない(`elwindui-languageserver`側の別途拡張が必要、未着手)。

### 4.3 選択指針

| 重視する観点 | 推奨方式 |
|---|---|
| IDE補完・生成コードの参照性 | build.rs方式 |
| ビルド構成のシンプルさ | proc-macro方式 |

いずれの方式でも②〜④のパイプライン(静的検証・定数畳み込み・コード生成)は共通の内部実装(`elwindui-codegen`本体)を呼び出すのみとし、起動方式の違いによってコンパイラの検証結果や生成コードの意味が変わることはない。

**実装状況の注**: 両方式とも実装済みで、実サンプルで検証されている。build.rs方式は`elwindui_codegen::compile_dir`/`compile_dir_with_extra_viewmodels`(`crates/elwindui-codegen/src/lib.rs`)として実装され、`examples/notepad`が利用する。proc-macro方式は`elwindui_macros::component`/`#[elwindui_macros::viewmodel]`(`crates/elwindui-macros/src/lib.rs`、`elwindui::component`/`#[elwindui::viewmodel]`として再エクスポート)として実装され、`examples/notepad-inline`(component+view+viewmodel全て)と`examples/viewmodel-attr-demo`(`#[elwindui::viewmodel]`のみ、view層無し)が利用する。`component`は`struct`に付与する属性マクロで、`view`要素ツリーは`view!`型フィールド(`crates/elwindui-codegen/src/component_frontend.rs`が処理)として書く。

---

## 5. 他ツールとの連携点

- **`elwindui-languageserver`(LSP)**: エディタ内診断のため、本コンパイラのパーサ・検証ロジック(①②)を共有ライブラリとして呼び出す想定。ただし増分パースやプレビュー用インスタンス生成はLSP側の責務であり、本書では扱わない。
- **プレビューツール**: 静的プレビュー(`docs/elwindui_tool_preview_design.md`のレベル①)は「componentを既定値でインスタンス化しオフスクリーンレンダリングする」処理であり、コンパイラが生成したコード(またはLSPが保持する検証済みAST)を利用する。生成コード自体の変更は不要。
- **ホットリロードツール**: `#[param]`変更時は再マウント、prop変更のみの場合は差分更新という区別(`docs/elwindui_tool_hotreload_design.md`)は、コンパイラが出力するコードが`param`/`prop`の区別を保ったまま生成されていることが前提となる。コンパイラ側で両者を混同しないコード生成を保証する。

---

## 6. ツールチェーン全体アーキテクチャ

`.elwind`ファイルの保存から、エディタ内診断・プレビュー・実行中アプリへの反映までを横断する、ElwindUILツールチェーン全体の構成。

```
┌──────────────────────────────────────────────┐
│ エディタ(VSCode等)                             │
│  ┌──────────────┐  ┌─────────────────────────┐ │
│  │ .elwindエディタ   │  │ プレビューパネル(WebView) │ │
│  │ (診断・補完)   │  │  ①静的 / ②操作可能        │ │
│  └──────────────┘  └─────────────────────────┘ │
└──────────────────────────────────────────────┘
        │ 保存イベント
        ▼
┌──────────────────────────────────────────────┐
│ elwindui-languageserver (LSP)                        │
│  - 増分パース・型検査・制約検証                  │
│  - プレビュー用インスタンス生成(既定値/モック)   │
└──────────────────────────────────────────────┘
        │
        ├─→ WebViewへ描画結果を送信(①②)
        │
        ▼(任意・実機確認したい場合)
┌──────────────────────────────────────────────┐
│ 実行中アプリ(dylibホットリロード)               │
│  - #[param]変更 → 再マウント                    │
│  - prop変更のみ → 差分更新、状態保持              │
└──────────────────────────────────────────────┘
```

本コンパイラ(`elwindui-codegen`)自体はこの図の中心には現れないが、LSPが再利用する構文解析・静的検証ロジック(§3の①②)、および実際の`cargo build`時に`.elwind`をRustソースへ変換する処理(build.rs方式・proc-macro方式のいずれも)の両方を提供し、上記の全経路の土台になっている。ここに挙げるツール群(本書・LSP・プレビュー・ホットリロード)はいずれも、DSLの言語仕様(`component`/`view`/`param`/`prop`/`Element`トレイト等、`docs/elwindui_dsl_spec.md`・`docs/elwindui_gui_framework_design.md`)自体を変更せずに構築できるツールチェーン層として位置づける。

---

## 7. まとめ

| 要件 | 対応 |
|---|---|
| `.elwind` → Rust変換 | 構文解析→共通AST→静的検証→定数畳み込み→バックエンド別コード生成の4段パイプライン |
| 起動方式 | build.rs方式(IDE補完重視)/ proc-macro方式(シンプルさ重視)の2方式を提供 |
| 静的検証 | 言語仕様14章のルール一覧をASTに対して実行し、違反はビルド時エラー |
| バックエンド分岐の除去 | `target::backend()`の定数畳み込みにより非該当分岐を静的除去(**未実装**。現状は生成コードがバックエンド非依存で、リンクする`elwindui-backend-*`クレートの選択のみでバックエンドが決まる) |
| 他ツールとの関係 | LanguageServer/preview/hotreloadは本コンパイラの解析結果・生成コードを利用する側であり、検証ロジックの二重実装を避ける |