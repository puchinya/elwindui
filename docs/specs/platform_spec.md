# ElwindUI Platform Specification

本仕様書は、ElwindUI において UIElement 階層に属さない OS サービス（ファイルダイアログ等）の公開仕様を定義する規範仕様（Normative Specification）である。

---

## 1. Scope

本書は `elwindui::platform` 名前空間配下で公開される OS プラットフォーム固有のサービス機能の契約および非同期インターフェースを規定する。

なお、UI要素自身のドラッグ＆ドロップイベント等（UIツリー上のイベントハンドリング）は [UI Specification](ui_spec.md) の対象であり、本書の対象外である。

---

## 2. Namespace and Canonical Path

OSサービス API の正準パスは以下の通りである。

```rust
elwindui::platform::<module>::<function>()
```

利用時は通常の Rust モジュールインポートに従う。

```rust
use elwindui::platform::file_dialog;

let path = file_dialog::open().await;
```

---

## 3. File dialogs

`elwindui::platform::file_dialog` モジュールは、OS標準のモーダルファイル選択・保存ダイアログ機能を提供する。

### 3.1 `open`

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

### 3.2 `save`

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

## 4. Related specifications

- [UI Specification](ui_spec.md) - UI要素とコントロールの規範仕様
- [DSL Specification](dsl_spec.md) - DSLにおける非同期コマンド・ハンドラ呼び出しの仕様
