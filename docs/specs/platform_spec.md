# ElwindUI Platform Specification

本仕様書は、ElwindUI のnative runtime起動と、UIElement 階層に属さない OS サービス（ファイルダイアログ等）の公開仕様を定義する規範仕様（Normative Specification）である。

---

## 1. Scope

本書は facade crate が公開するnative runtime初期化・application loopと、`elwindui::platform` 名前空間配下の OS プラットフォーム固有サービスの契約を規定する。

なお、UI要素自身のドラッグ＆ドロップイベント等（UIツリー上のイベントハンドリング）は [UI Specification](ui_spec.md) の対象であり、本書の対象外である。

---

## 2. Namespace and Canonical Path

Runtime起動 API の正準パスは `elwindui::{init, InitError}`、`elwindui::application::run`、`#[elwindui::main]` である。OSサービス API の正準パスは以下の通りである。

```rust
elwindui::platform::<module>::<function>()
```

利用時は通常の Rust モジュールインポートに従う。

```rust
use elwindui::platform::file_dialog;

let path = file_dialog::open().await;
```

---

## 3. Runtime initialization and application loop

- `elwindui::init() -> Result<(), elwindui::InitError>` は、選択されたnative backendが必要とするprocess-wide初期化を行う。同一processでWindowを作成する前にUI threadから呼び出す。
- `elwindui::application::run(startup)` はnative application loopを開始し、そのUI thread上で `startup: FnOnce()` を呼び出す。`init()` と `run()` は同じOS threadから呼び出す。
- `#[elwindui::main]` は引数を取らない通常のRust `main` 関数に適用し、関数本体を `init()` 成功後に `application::run` へ渡す。初期化失敗時はapplication loopを開始しない。
- 対象OSに対応するbackend featureが有効でない構成はcompile errorとする。

---

## 4. File dialogs

`elwindui::platform::file_dialog` モジュールは、OS標準のモーダルファイル選択・保存ダイアログ機能を提供する。

### 4.1 `open`

OS標準のファイルオープンパネルを表示し、ユーザーが選択したファイルのパスを返す。

#### Signature

```rust
pub async fn open() -> Option<std::path::PathBuf>
```

#### Behavior & Semantics
- アプリケーションのUIスレッドから呼び出される。
- キャンセルされた場合、またはファイルが選択されなかった場合は `None` を返す。
- ユーザーがファイルを選択して確定した場合は `Some(PathBuf)` を返す。

---

### 4.2 `save`

OS標準のファイル保存パネルを表示し、ユーザーが指定した保存先ファイルのパスを返す。

#### Signature

```rust
pub async fn save() -> Option<std::path::PathBuf>
```

#### Behavior & Semantics
- アプリケーションのUIスレッドから呼び出される。
- キャンセルされた場合は `None` を返す。
- 保存先パスが確定した場合は `Some(PathBuf)` を返す。

---

## 5. Related specifications

- [UI Specification](ui_spec.md) - UI要素とコントロールの規範仕様
- [DSL Specification](dsl_spec.md) - DSLにおける非同期コマンド・ハンドラ呼び出しの仕様
