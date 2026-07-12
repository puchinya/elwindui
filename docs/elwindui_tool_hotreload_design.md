# ElwindUIL ホットリロード機構 設計書

出典: `docs/elwindui_spec.md` 付録B.4「実行中アプリへのホットリロード」、付録B.5「全体アーキテクチャ」

## 1. スコープ

本書は、実行中のElwindUILアプリケーションに対して `.elwind` の変更を即座に反映する**ホットリロード機構**そのものを対象とする。扱う範囲は以下に限定する。

- `view`関数を動的ライブラリ(dylib)として差し替える仕組み
- `#[param]`/`prop`の区分に基づく更新粒度(再マウント vs 差分更新)の判定ロジック
- `hot-lib-reloader`との統合方式

以下は対象外とし、それぞれの担当ドキュメントに譲る。

- 言語仕様上の`param`/`prop`の意味論そのもの(定義済み。本書はその契約を前提として利用するのみ) — `docs/elwindui_gui_framework_design.md`
- dylib化されるRustコードの生成パイプライン — `docs/elwindui_tool_codegen_design.md`
- `.elwind`保存イベントの検知・増分パースの起点(LSP側) — `docs/elwindui_tool_languageserver_design.md`
- プレビューレベル③(実行中アプリへの反映)におけるユーザー体験・WebView連携 — `docs/elwindui_tool_preview_design.md`(本書のホットリロード機構を裏側で利用する)

**実装状況の注**: 現時点の`elwindui-hotreload`クレート(`crates/elwindui-hotreload/src/lib.rs`)は32行のみで、§3「更新粒度の判定ロジック」に相当する`ReloadAction` enum(`Remount`/`Patch`)と`decide_reload_action(any_param_field_changed: bool) -> ReloadAction`という純粋関数、およびその単体テストのみを実装している。dylib差し替えの仕組み(§2の処理フロー全体、§4の`hot-lib-reloader`統合)は未実装であり、本書の残りの内容はすべてフォワードルッキングな設計である。

## 2. 全体の処理フロー

付録B.5の全体アーキテクチャにおいて、本機構は最終段に位置する。

```
.elwind保存
    │
    ▼
elwindui-languageserver (LSP) が増分パース・型検査・制約検証
    │
    ▼
elwindui-codegen が変更対象のRustコードを再生成
    │
    ▼
dylib (例: notepad_ui) を再ビルド
    │
    ▼
実行中アプリ側の hot-lib-reloader がdylibの更新を検知し差し替え
    │
    ├─ #[param]に関わる変更 → 再マウント(状態リセット)
    └─ propのみの変更     → 差分更新(状態を保持したまま反映)
```

実行中アプリへの反映は付録B.3のプレビュー3段階のうち「③実行中アプリへの反映」に相当し、任意(オプトイン)の経路である。LSPによる①静的プレビュー・②インタラクティブプレビューは本機構を経由せず、プレビュー専用の軽量ランタイム上で完結する(`docs/elwindui_tool_preview_design.md`参照)。

## 3. 更新粒度の判定ロジック

`param`/`prop`の区分(言語仕様側で定義済み)をそのまま更新粒度の判定基準として再利用する。新しい状態管理の概念は導入しない。

| 変更対象 | 判定 | 理由 |
|---|---|---|
| `#[param]`フィールドに関わる変更 | 再マウント(状態リセット) | `#[param]`は実体化時に一度だけ確定し以後不変という契約(言語仕様4章)を持つ。実行中インスタンスの`#[param]`値を書き換えることはこの不変性契約に反するため、既存インスタンスを破棄し新しい既定値で再インスタンス化するしかない |
| 既定(`prop`)フィールドのみの変更 | 差分更新(状態を保持したまま反映) | `prop`はもともと実行時に読み書き可能な値として設計されており、`view`関数の実装(見た目への写像ロジック)が変わってもフィールド自体の現在値は保持できる。dylib差し替え後も既存の状態を保持したまま新しい`view`関数を適用するだけでよい |

判定に必要な「どのフィールドが変更されたか」の情報は、elwindui-codegen側の差分検出結果(`docs/elwindui_tool_codegen_design.md`が担当する生成パイプラインの出力)を入力として受け取る想定とする。本書はその判定結果をどう適用するか(再マウント/差分更新のどちらの経路を通すか)を扱う。

## 4. hot-lib-reloaderとの統合方式

仕様書に示された統合パターンは以下の通り。

```rust
#[hot_lib_reloader::hot_module(dylib = "notepad_ui")]
mod hot_notepad_ui {
    hot_functions_from_file!("src/ui/notepad_window.rs");
}
```

- `view`関数群は`hot_module`でラップされたモジュールとして生成され、対応する`.elwind`から生成されたRustソース(`src/ui/notepad_window.rs`)を`hot_functions_from_file!`で束縛する
- dylib(例: `notepad_ui`)は独立してビルドされ、`hot-lib-reloader`が実行中プロセスにロードされたライブラリの更新を検知して透過的に差し替える
- アプリ本体からは`hot_notepad_ui`モジュール越しに`view`関数を呼び出す形になり、差し替え前後でシグネチャが変わらない限り呼び出し側のコードは変更不要

## 5. 他ツールとの連携インターフェース

| 連携先 | 本機構が受け取るもの | 本機構が提供するもの |
|---|---|---|
| elwindui-languageserver(LSP) | `.elwind`保存イベント・増分パース結果を起点とした再生成トリガー | なし(下流) |
| elwindui-codegen | 再生成されたRustソース、および変更されたフィールドが`#[param]`か`prop`かの差分情報 | なし(下流) |
| プレビューツール(レベル③) | — | dylib差し替え・状態保持/リセットの実行結果(反映完了通知など、プレビューUI側が実行中アプリの状態を表示するために必要な情報) |

## 6. まとめ

| 要件 | 対応 |
|---|---|
| 実行中アプリへの反映手段 | `hot-lib-reloader`によるdylib差し替え |
| 更新粒度の判断基準 | 既存の`param`/`prop`の区分をそのまま流用(新しい概念を追加しない) |
| `#[param]`変更時の扱い | 再マウント(状態リセット)。paramの不変性契約と整合 |
| `prop`変更時の扱い | 差分更新(状態保持)。propの可変性という既存の性質と整合 |
| 統合方式 | `#[hot_lib_reloader::hot_module(dylib = "...")]` + `hot_functions_from_file!` |
