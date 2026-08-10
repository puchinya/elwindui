# elwindui ドキュメントインデックス ＆ ルーティングガイド

このディレクトリは ElwindUIL/elwindui のドキュメントを役割別に分けて配置している。

## ドキュメント分類と役割

| ディレクトリ | 役割 | 問いに答える |
|---|---|---|
| [`specs/`](specs/) | **仕様書**(規範) | 採用済みの規範仕様。「何であるべきか」 |
| [`design/`](design/) | **設計書** | `specs`で定義された仕様を実現する内部設計。「どう作るか」 |
| [`status/`](status/) | **実装状況** | 現在の実装状態。「今どこまで出来ているか」 |
| [`agents/`](agents/) | **AI技術ルール** | 「実装時に何を守るべきか」 |
| [`agent-workflow/`](agent-workflow/) | **作業フロー** | 「Issueをどう進めるか」 |

**`specs/`は未実装の仕様を含む場合があるが規範であり、`design/`はその実現方法を記述する内部設計書である。** 現在の「実際に動くもの」やバックエンド別の対応状況を知りたい場合は、必ず `status/` を参照すること。

---

## AIエージェント・人間共通ルーティングテーブル

タスクの目的に応じて、以下のドキュメントから優先的に読み込むこと。

| タスク | 一次参照ドキュメント | 補足 |
|---|---|---|
| **DSL構文・仕様確認** | [`specs/dsl_spec.md`](specs/dsl_spec.md) | 該当 section のみ部分読みする |
| **標準UI型・仕様確認** | [`specs/ui_spec.md`](specs/ui_spec.md) | `elwindui::ui` の公開UI型 |
| **グラフィックス仕様確認** | [`specs/graphics_spec.md`](specs/graphics_spec.md) | Color, Brush, Path, Image 等 |
| **OSサービス仕様確認** | [`specs/platform_spec.md`](specs/platform_spec.md) | `elwindui::platform` (file_dialog 等) |
| **DSL/codegen実装・修正** | [`agents/codegen.md`](agents/codegen.md) | 必要に応じ `specs/dsl_spec.md` §2/§13 を参照 |
| **`#[class]`マクロ・クラス階層** | [`agents/class-model.md`](agents/class-model.md) | 正本は `specs/macro_class_spec.md` |
| **ランタイム / UIツリー / レイアウト** | [`design/gui_framework_design.md`](design/gui_framework_design.md) | §5 コアランタイム |
| **バックエンド共通実装** | [`agents/backend-common.md`](agents/backend-common.md) | レイヤー構造（`native_ui -> inner -> host -> render -> ffi`） |
| **AppKit (macOS) バックエンド** | [`agents/appkit.md`](agents/appkit.md) | GUI検証・`macos-ui-driver` 手順 |
| **WinUI 3 (Windows) バックエンド** | [`agents/winui3.md`](agents/winui3.md) | C++/WinRT shim, PowerShell環境手順 |
| **ビルド・テスト・検証** | [`agents/testing.md`](agents/testing.md) | `rust-analyzer diagnostics` 手順含む |
| **現在のアクティブ実装状況確認** | 該当 [`status/*.md`](status/) | サマリは `status/implementation_status.md` |
| **Issue駆動開発プロセス** | 該当 [`agent-workflow/*.md`](agent-workflow/) | `phase:*` に応じた1文書のみロード |

---

## ファイル一覧

### `specs/` — 仕様書
| ファイル | 内容 |
|---|---|
| [`specs/dsl_spec.md`](specs/dsl_spec.md) | ElwindUIL **DSL構文のみ**。`component`/`view`分離、`param`/`prop`/`state`、静的検証ルール一覧(§13)等 |
| [`specs/ui_spec.md`](specs/ui_spec.md) | `elwindui::ui` の標準UI型と公開意味論 |
| [`specs/graphics_spec.md`](specs/graphics_spec.md) | Graphics型・描画意味論 (`Color`, `Brush`, `Path`, `Image` 等) |
| [`specs/platform_spec.md`](specs/platform_spec.md) | `elwindui::platform` のOSサービス (`file_dialog` 等) |
| [`specs/macro_class_spec.md`](specs/macro_class_spec.md) | `#[elwindui_macros::class]` マクロ仕様の正本 |

### `design/` — 設計書
| ファイル | 内容 |
|---|---|
| [`design/gui_framework_design.md`](design/gui_framework_design.md) | GUIフレームワーク本体・コアランタイム・ライフサイクル・MVVM等 |
| [`design/tools/codegen_design.md`](design/tools/codegen_design.md) | `elwindui-codegen` パイプライン設計 |
| [`design/tools/languageserver_design.md`](design/tools/languageserver_design.md) | LSP設計 |

### `status/` — 実装状況
| ファイル | 内容 |
|---|---|
| [`status/implementation_status.md`](status/implementation_status.md) | 横断サマリ |
| [`status/nativecontrol_status.md`](status/nativecontrol_status.md) | NativeControl バックエンド対応マトリクス |
| [`status/font_status.md`](status/font_status.md) | フォントモデル・バックエンド別状況 |
| [`status/theme_status.md`](status/theme_status.md) | テーマ機能 |
| [`status/winui3_backend_status.md`](status/winui3_backend_status.md) | WinUI 3バックエンド状況 |
| [`status/macos_ui_driver_status.md`](status/macos_ui_driver_status.md) | macOS GUI自動テストCLI状況 |

### `agents/` — AI技術ルール
| ファイル | 内容 |
|---|---|
| [`agents/common.md`](agents/common.md) | 全領域共通ルール (rustdoc, スコープ維持, アーキテクチャ非変量) |
| [`agents/codegen.md`](agents/codegen.md) | codegen / DSL実装ガイド |
| [`agents/class-model.md`](agents/class-model.md) | クラス階層・`#[class]` ガイド |
| [`agents/backend-common.md`](agents/backend-common.md) | バックエンド共通レイヤー・依存方向 |
| [`agents/appkit.md`](agents/appkit.md) | AppKit バックエンド・GUI検証手順 |
| [`agents/winui3.md`](agents/winui3.md) | WinUI 3 / Windows 開発ガイド |
| [`agents/testing.md`](agents/testing.md) | テスト・`rust-analyzer` 検証手順 |

### `agent-workflow/` — 作業フロー
| ファイル | 内容 |
|---|---|
| [`agent-workflow/`](agent-workflow/) | Issue駆動開発のフェーズ別ワークフロー (requirements / design / implementation / review / checkpoint / evidence) |

`docs_only_human/` は人間向けの解説であり、通常のエージェント作業では読み込まない。
