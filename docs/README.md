# elwindui ドキュメント

このディレクトリはElwindUIL/elwinduiのドキュメントを3つの役割に分けて置いている。

| ディレクトリ | 役割 | 問いに答える |
|---|---|---|
| `specs/` | **仕様書**(規範) | 「これは何であるべきか」 |
| `design/` | **設計書** | 「どう作るか」 |
| `status/` | **実装状況** | 「今どこまで出来ているか」 |

**`specs/`と`design/`はフォワードルッキングな内容を含む。** 設計として定めてあるが未実装、という記述が
各所にある。「実際に動くもの」を知りたい場合は必ず`status/`を見ること。最終的な真実は`crates/`配下の
コードであり、`status/`もドリフトしうる。

## 実装状況バッジ

`specs/`と`design/`の機能単位の節見出しには、その機能の実装状況を示すバッジが付いている。

| バッジ | 意味 |
|---|---|
| ✅ | 実装済み(少なくとも1バックエンドで実機検証済み) |
| 🚧 | 部分実装 |
| 📋 | 仕様のみ(コード無し) |

バックエンド別の粒度は見出しには載せない。`status/implementation_status.md` §4の
「機能 × バックエンド マトリクス」を参照すること(AppKit / WinUI3 / GTK4)。

## ファイル一覧

### `specs/` — 仕様書

| ファイル | 内容 |
|---|---|
| [`specs/dsl_spec.md`](specs/dsl_spec.md) | ElwindUIL **DSL構文のみ**。`component`/`view`の分離、`param`/`prop`、制御構文、`style`、値制約、`enum`、`env::*`/`once`、`bind!`、i18n(Fluent)、`use`、`UIElement`ツリー探索、静的検証ルール一覧(§14)。付録Aは`builtin::`名前空間と`#[overrides(builtin::X)]`のシャドーイング規則、および`#[embedded]`/`#[sealed]`/`#[native]`/`#[abstract]`/`#[text_style]`/`#[content(field_name)]`のcomponent単位属性 |
| [`specs/builtins_spec.md`](specs/builtins_spec.md) | 個々の`builtin::`要素と`platform::`名前空間のリファレンス。付録Fが標準部品(`Window`/`VerticalLayout`/`HorizontalLayout`/`TextBlock`/`TextArea`/`Control`/`ContentControl`/`Grid`/`TextBox`/`PasswordBox`/`ScrollView`ほか)、付録G/Nが`Canvas`/`Painter`と描画拡張、付録Lがナビゲーション、付録Mがダイアログ/メニュー/ツールチップ、付録Qが`VirtualList`、付録Tがクリップボード/ファイルダイアログ/D&D、付録X/Yが`MenuBar`と`TabView` |
| [`specs/macro_class_spec.md`](specs/macro_class_spec.md) | `#[elwindui_macros::class]`属性マクロの仕様。クラス階層生成、`__elwindui_inherit_*!`マクロトリオ、`__dyn_x`アクセサ方式による祖先メソッド継承、コンストラクタ自動生成、rust-analyzer対応。`design/gui_framework_design.md` §5.1aの要約と食い違う場合は**こちらが正** |

### `design/` — 設計書

| ファイル | 内容 |
|---|---|
| [`design/gui_framework_design.md`](design/gui_framework_design.md) | GUIフレームワーク本体。バックエンド抽象化(§3)、標準ビルトイン部品(§4)、コアランタイム(§5 — `UIElement`/`UIElementExt`クラス階層、Logical/Visualツリー分離、レイアウトエンジン、フォーカス、アクセシビリティ、`Canvas`/`RenderContext`、ルーティングイベント)、ライフサイクル(§6)、`store`/`viewmodel`/非同期/Undo-Redo(§7)、UI機能拡張(§8)、スナップショットテスト(§9) |
| [`design/tools/codegen_design.md`](design/tools/codegen_design.md) | `elwindui-codegen`(コンパイラ本体)の入出力・内部パイプライン・起動方式。§7にツールチェーン全体のアーキテクチャ概観 |
| [`design/tools/languageserver_design.md`](design/tools/languageserver_design.md) | `elwindui-languageserver`(LSP)の設計 |
| [`design/tools/preview_design.md`](design/tools/preview_design.md) | エディタ内プレビュー機能の設計(クレート未着手) |
| [`design/tools/hotreload_design.md`](design/tools/hotreload_design.md) | 実行中アプリへのホットリロード機構の設計(スタブのみ) |

### `status/` — 実装状況

| ファイル | 内容 |
|---|---|
| [`status/implementation_status.md`](status/implementation_status.md) | **横断サマリ。まずここを読む。** クレート別状況、サンプルアプリ、バックエンド対応、機能×バックエンドのマトリクス、ビルトイン一覧、言語コア機能、UI機能拡張、ツールチェーン、バックエンドcrateのファイル構成、既知のギャップ |
| [`status/nativecontrol_status.md`](status/nativecontrol_status.md) | NativeControl派生コントロール(`TextBox`/`PasswordBox`/`ScrollView`ほか)のコントロール×バックエンド×要件チェックリスト |
| [`status/font_status.md`](status/font_status.md) | フォント/テキストスタイル機能の単一の真実の源。共通フォントモデル、プロパティ単位の継承、計測シーム(`TextBackend`)、バックエンド別対応 |
| [`status/theme_status.md`](status/theme_status.md) | テーマ/デザイントークン(`#[elwindui::theme_definition]`/`theme!`)の状況 |
| [`status/winui3_backend_status.md`](status/winui3_backend_status.md) | WinUI 3バックエンドの状況。ビルド環境、C++/WinRTシム、既知の落とし穴、退行させてはいけない項目 |
| [`status/macos_ui_driver_status.md`](status/macos_ui_driver_status.md) | `tools/macos-ui-driver`(macOS GUI自動テストCLI)のコマンド一覧と制約 |
| [`status/appkit_memory_baseline.md`](status/appkit_memory_baseline.md) | AppKit backend の A-D 基礎メモリ baseline、測定環境、再現スクリプト |

### エージェント向け

| パス | 内容 |
|---|---|
| [`agent-workflow/`](agent-workflow/) | Issue駆動開発のフェーズ別ワークフロー(requirements / design / implementation / review / checkpoint / evidence)。該当フェーズのものだけを読む |
| [`agents/windows.md`](agents/windows.md) | Windows上で作業する場合の追加手順 |

`docs_only_human/`は人間向けの解説であり、通常のエージェント作業では読み込まない。

## どこから読むか

- **DSLの書き方を知りたい** → `specs/dsl_spec.md`、次に`specs/builtins_spec.md`
- **フレームワークの内部構造を知りたい** → `design/gui_framework_design.md` §5
- **ある機能が動くのか知りたい** → `status/implementation_status.md` §4のマトリクス
- **バックエンドを実装/修正したい** → `status/implementation_status.md` §9(層構成)、次に該当バックエンドの`status/`
- **`#[class]`マクロを触りたい** → `specs/macro_class_spec.md`
