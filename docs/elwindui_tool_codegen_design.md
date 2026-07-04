# ElwindUIL コード生成ツール(`elwindui-codegen`)設計書

本書は、`.elwind`ファイルをRustソースコードへ変換するコンパイラ本体(`elwindui-codegen`)の設計を定める。言語仕様そのもの(`component`/`view`/`param`/`prop`/`Element`トレイト/バックエンド抽象化等)は `docs/elwindui_gui_framework_design.md` を正とし、本書では「コンパイラというツール」の入出力・内部パイプライン・起動方式・実装トレードオフに焦点を当てる。

参照元: `docs/elwindui_spec.md` 付録B.1(ビルド時自動生成)、付録B.5(全体アーキテクチャ)。

---

## 1. スコープ

### 1.1 責務

- `.elwind`ソースの構文解析・共通AST構築
- 言語仕様(1〜15章・付録A/C/D/E/F等)に定義された静的検証(14章の検証ルール一覧)の実行
- `#![backend(...)]` 指定・ビルドターゲットに基づく `target::backend()` の定数畳み込みと、非該当バックエンド分岐の静的除去
- バックエンド別(egui/iced、WinUI 3/AppKit/GTK4)Rustソースの生成
- 2つの起動方式(build.rs方式 / proc-macro方式)の提供

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
   │ ③ target::backend() の定数畳み込み・非該当分岐の除去
   ▼
バックエンド確定AST
   │ ④ バックエンド別コード生成
   ▼
Rustソース(egui / iced / WinUI3 / AppKit / GTK4 いずれか)
```

- **①構文解析**: Rust構文に似た `.elwind` の字句・構文解析を行い、共通ASTを構築する。`use`宣言(12章)による他コンポーネントのインポートもこの段階で解決し、循環参照・未解決参照を検出する。
- **②静的検証**: 言語仕様14章に列挙された検証ルール(`#[param]`初期化式への`bind!`混入禁止、enum網羅性検査、制約違反の検出、`native!`の出現位置制限など)をASTに対して実行する。違反はビルド時エラーとしてコンパイルを停止させる。
- **③定数畳み込み**: `target::backend()`(付録D)はビルド設定(Cargoのfeature/target triple)から一意に確定するため、コンパイル時にコード生成器が値を確定し、該当しない `match target::backend() { ... }` の腕や `#[cfg(backend = "...")]` 付き `native!` ブロックを生成対象から静的に除去する(付録D.4、付録C.4)。実行バイナリに不要な分岐コードが残らないことを保証する。
- **④バックエンド別コード生成**: 確定したASTから、選択されたバックエンド(付録A・C)向けのRustコードを生成する。ビルトイン要素(`builtin::Window`/`Row`/`Text`等、付録F)は他コンポーネントと同じ`component`/`view`構文で書かれたリファレンス実装として同一パイプラインで処理される。

---

## 4. 起動方式

仕様書(B.1)は2つの起動方式を挙げており、コード生成器はいずれからも呼び出せる形で実装する。

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

```rust
elwindui::component! {
    include_str!("ui/notepad_window.elwind")
}
```

- 中間ファイルを生成せず、コンパイル時にトークン列として直接展開する。
- ビルド構成がシンプルになる(`build.rs`の追加が不要)一方、生成コードが実ファイルとして残らないため、IDE補完の精度はbuild.rs方式に劣る。

### 4.3 選択指針

| 重視する観点 | 推奨方式 |
|---|---|
| IDE補完・生成コードの参照性 | build.rs方式 |
| ビルド構成のシンプルさ | proc-macro方式 |

いずれの方式でも②〜④のパイプライン(静的検証・定数畳み込み・コード生成)は共通の内部実装(`elwindui-codegen`本体)を呼び出すのみとし、起動方式の違いによってコンパイラの検証結果や生成コードの意味が変わることはない。

---

## 5. 他ツールとの連携点

- **`elwindui-languageserver`(LSP)**: エディタ内診断のため、本コンパイラのパーサ・検証ロジック(①②)を共有ライブラリとして呼び出す想定。ただし増分パースやプレビュー用インスタンス生成はLSP側の責務であり、本書では扱わない。
- **プレビューツール**: 静的プレビュー(付録B.3 ①)は「componentを既定値でインスタンス化しオフスクリーンレンダリングする」処理であり、コンパイラが生成したコード(またはLSPが保持する検証済みAST)を利用する。生成コード自体の変更は不要。
- **ホットリロードツール**: `#[param]`変更時は再マウント、prop変更のみの場合は差分更新という区別(付録B.4)は、コンパイラが出力するコードが`param`/`prop`の区別を保ったまま生成されていることが前提となる。コンパイラ側で両者を混同しないコード生成を保証する。

---

## 6. まとめ

| 要件 | 対応 |
|---|---|
| `.elwind` → Rust変換 | 構文解析→共通AST→静的検証→定数畳み込み→バックエンド別コード生成の4段パイプライン |
| 起動方式 | build.rs方式(IDE補完重視)/ proc-macro方式(シンプルさ重視)の2方式を提供 |
| 静的検証 | 言語仕様14章のルール一覧をASTに対して実行し、違反はビルド時エラー |
| バックエンド分岐の除去 | `target::backend()`の定数畳み込みにより非該当分岐を静的除去 |
| 他ツールとの関係 | LanguageServer/preview/hotreloadは本コンパイラの解析結果・生成コードを利用する側であり、検証ロジックの二重実装を避ける |