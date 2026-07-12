# ElwindUIL 実装状況ドキュメント

`docs/elwindui_spec.md`・`docs/elwindui_builtins_spec.md`・`docs/elwindui_gui_framework_design.md`・`docs/elwindui_tool_*_design.md`は仕様/設計書であり、将来実装予定のフォワードルッキングな内容を含む。本ドキュメントは「実際に`crates/`配下に何が実装済みで、何が未着手か」を横断的に一覧化したもの。各仕様書ファイル自身にも該当箇所へ実装状況の注記を追加済みなので、詳細な文脈はそちらを参照すること。

このドキュメントは2026年時点のワークスペース(`cargo build --workspace`が通る状態)を実地調査して作成した。実装は日々変化するため、内容が古くなったと思われる場合は`crates/`を直接確認すること。

---

## 1. クレート別実装状況

| クレート | 行数(目安) | 状況 |
|---|---|---|
| `elwindui-core` | 約3100行 | 実装済み。`UIElement`クラス階層(`#[elwindui_macros::class]`)、`LayoutNode`/`Painter`トレイト、ルーティングイベント(`dispatch_routed`/`hit_test`)が実働。`FocusManager`/`AccessibilityNode`トレイトは**型定義のみ**で実装(`impl`)がテスト用ダミー1つ以外に存在しない |
| `elwindui-codegen` | 約8900行 | 実装済み(コンパイラ本体)。`build.rs`経由の`compile_dir`系と`elwindui::component!`/`#[elwindui::viewmodel]`プロシージャルマクロ系、両方の起動経路が実働し、それぞれ`examples/notepad`・`examples/notepad-inline`・`examples/viewmodel-attr-demo`で使用されている |
| `elwindui-macros` | - | 実装済み。`#[class(inherits/implements/supertrait/abstract_class/sealed)]` + `#[inherent]`/`#[ancestor]`によるクラス階層生成マクロ。`docs/elwindui_spec.md`付録H.2.1aの記述と一致 |
| `elwindui-i18n` | 58行+マクロ | 実装済み。Fluentベースのランタイム(`t!`, `declare!`マクロ)。ただしビルド時の`.ftl`静的検証(未翻訳キー検出・引数名整合性チェック)は`elwindui-codegen`側に存在せず未実装 |
| `elwindui-languageserver` | 約990行 | 部分実装。診断(`elwindui-codegen`の`parser`/`validate`を再利用)、シンタックスハイライト(semantic tokens)、メンバー補完(`vm.field`/`vm.command.*`)が実働。hover、生成コードプレビュー、オフスクリーンレンダリングと連携したインスタンス生成パイプラインは未実装 |
| `elwindui-hotreload` | 32行 | スタブのみ。`param`/`prop`差分からremount/patchを判定する純粋関数(`decide_reload_action`)だけが存在し、`hot-lib-reloader`統合・実際のdylib差し替えは未実装 |
| `elwindui-test` | 79行 | 部分実装。`render_tree`(`Element`ツリーのインデントダンプ)のみ実装。`render_canvas_snapshot`/`assert_image_snapshot!`は未実装(`canvas.rs`はdocコメントのみのスタブ) |
| `elwindui-backend-appkit` | 約1800行 | 実装済み・実機検証済み。本機で`cargo build`/実行/スクリーンショット確認済みの唯一のバックエンド |
| `elwindui-backend-winui3` | 約1600行 | 実装コードあり・未検証。appkitと同等の`builtins`モジュール構成を持つが、Windows環境が無いためビルド・動作とも未確認 |
| `elwindui-backend-gtk4` | 2行 | 未着手。`src/lib.rs`が2行のみのスタブで、`builtins`/`platform`/`application`モジュールが一切存在しない |
| `elwindui`(ファサード) | 61行 | 実装済み。`backend-appkit`/`backend-winui3`/`backend-gtk4`のCargoフィーチャで`core`/`i18n`/`backend`/`ui`を再エクスポートする |
| プレビューツール(`elwindui-preview`相当) | - | **ワークスペースに存在しない**。`docs/elwindui_tool_preview_design.md`は100%未着手のフォワードルッキング設計 |

---

## 2. バックエンド対応状況

| バックエンド | 状況 |
|---|---|
| AppKit(macOS) | 実装済み・実機検証済み |
| WinUI3(Windows) | 実装コードあり・未検証(Windows環境なし) |
| GTK4(Linux) | 未着手(2行のスタブのみ) |
| Uikit(iOS)/Jetpack(Android)(付録W) | 設計のみ、コード無し |

かつて存在した非ネイティブ系バックエンドクレートはワークスペースから完全に削除済みで、現在のバックエンド候補は上記のネイティブ3種(+将来のモバイル2種)のみ。

**重要な設計と実装の乖離**: `docs/elwindui_spec.md`付録Dが説明する`enum Backend` + `target::backend()`(コンパイル時定数、`match`網羅性検査による新バックエンド追加時の安全弁)は**コード中のどこにも実体が存在しない**。実際のバックエンド選択は`elwindui`ファサードクレートのCargoフィーチャフラグ(`backend-appkit`/`backend-winui3`/`backend-gtk4`)による`#[cfg(feature = ...)]`のみで行われている。これに伴い、`native!`/`match target::backend()`をビルトイン限定にする14章ルール9、`NavigationHost`の`Route`網羅性ルール14、オーバーレイ系ビルトインの分岐制限ルール15なども、前提となる仕組み自体が無いため検証しようがない。

---

## 3. ビルトインウィジェット実装状況

`crates/elwindui-codegen/src/builtins.elwind`を正とする。詳細な分類ツリーは`docs/elwindui_builtins_spec.md`冒頭を参照。

### 実装済み(`.elwind`宣言 + バックエンド実体あり)

`Window` / `VerticalLayout` / `HorizontalLayout`(`Row`/`Column`という名称ではない) / `Shape`(抽象) / `Rectangle` / `Ellipse` / `Control` / `ContentControl` / `Grid` / `TextArea` / `Button` / `TextBlock` / `MenuBar` / `MenuBarItem` / `Menu` / `MenuItem` / `TabView` / `TabViewItem`

- `Menu`/`MenuItem`は`MenuBarItem.submenu`経由での利用は実装済みだが、任意要素に`context_menu`属性で汎用的に付けるコンテキストメニュー機構は未実装。
- `tooltip`共通属性も未実装。

### 未実装(仕様のみ、`.elwind`宣言なし)

`Dropdown` / `Option`(付録F.5)、`Canvas`(付録G)、`NavigationHost` / `Route`(付録L)、`Dialog`(付録M.1)、`Tooltip`(付録M.3)、`VirtualList`(付録Q)

### `platform::`名前空間(付録T)

| 機能 | 状況 |
|---|---|
| `platform::file_dialog::open()`/`save()` | 実装済み(AppKit検証済み・WinUI3未検証・GTK4無し)。戻り値は`Option<PathBuf>`のみで、仕様書にあるファイルフィルタ指定引数は無い |
| `platform::clipboard::*` | 未実装(コード自体が存在しない) |
| ドラッグ&ドロップ(`draggable`/`on_drag_start`/`on_drop`) | 未実装 |

---

## 4. 言語コア機能の実装状況(`docs/elwindui_spec.md` §1〜14)

| 機能 | 状況 |
|---|---|
| `component`/`view`分離 | 実装済み |
| `param`/`prop`区別(`#[param]`、静的評価式制限) | 実装済み |
| 制御構文(`if`/`for`/`match`) | 実装済み。`match`の網羅性は、生成先が実際のRust `enum`+`match`であるため多くの場面でRustコンパイラ自身の網羅性検査に乗る形で機能する |
| `style{}`(横断的属性適用) | **未実装**。`elwindui-codegen`のASTに`Style`ノードが存在しない |
| 値制約(`#[range]`/`#[step]`/`#[length]`/`#[pattern]`/`#[format]`/`#[check]`) | `#[length]`のみ実装。他は未実装 |
| `enum`(`EnumName::values()`、`#[label(...)]`) | `EnumDef`はASTに存在(実装済み)。`values()`/`#[label]`によるi18nラベル付与の実装範囲は個別確認が必要 |
| `env::*` / `once` | **未実装**。`elwindui-codegen`にDSLキーワードとしての扱いが無い |
| `bind!` | 実装済み(`Initializer::Bind`) |
| `command!` | 実装済み(`Initializer::Command`) |
| i18n(Fluent、`t!`) | ランタイム(`elwindui-i18n`)は実装済み。ビルド時の`.ftl`静的検証(未翻訳キー検出・引数名整合性チェック)は未実装 |
| モジュール(`use`) | 生成先が実際のRustコードのため`use`解決自体はRustコンパイラに委譲される。循環参照・未解決パスの独自の機械的検出は未確認 |
| `Element`トレイト(`children()`/`id()`/`find_by_id`/`find_all`) | 実装済み |
| 14章 静的検証ルール(全25項目) | 部分実装。`crates/elwindui-codegen/src/validate.rs`(約1600行)がルール18(`#[command]`フィールド型)・19(`viewmodel`内`view`参照禁止)を含む多くの言語機能バリデーションを実装しているが、ルール番号がソース上に明示されているのはルール18・19のみで、前提機能自体が未実装のルール(9・14・15など、`target::backend()`依存)は検証不能 |

---

## 5. UI機能拡張(付録I〜W)の実装状況

| 付録 | 機能 | 状況 |
|---|---|---|
| I | ライフサイクルフック(`on_mount`/`on_unmount`/`on_update`) | `on_mount`は実装・結線済み。`on_unmount`はパース・コード生成されるが、`elwindui-core::ui`に実際のツリー離脱(デタッチ)フックが無いため**呼び出されない** |
| J | `store`(グローバル状態) | **未実装**。ASTに`Store`ノードが無い |
| K | キーボードショートカット(`#[shortcut(...)]`、`#[focus(...)]`) | **未実装**(`FocusManager`は型のみ存在、§1参照) |
| L | ナビゲーション(`NavigationHost`/`Route`) | **未実装**(§3のビルトイン一覧参照) |
| M | ダイアログ/メニュー/ツールチップ | `Menu`/`MenuItem`本体は実装済み、`Dialog`/`Tooltip`および汎用`context_menu`/`tooltip`属性は未実装 |
| N | 描画拡張(Brush/Geometry/Effect/Transform/レイヤー合成/アニメーション) | 未実装。`Painter`基本セット(塗り・線・テキスト)のみ`elwindui-core`に存在、`Canvas`自体が未実装のため利用できない |
| O | MVVM(`viewmodel`/`Command`) | 実装済み。`#[observable]`/`#[computed]`/`#[command]`が動作し、`examples/notepad`のMVVM構成で実際に使われている |
| P | 非同期処理 | 部分実装。`spawn`相当(`spawn_local`)は実装済みで`examples/notepad`が使用。`AsyncState<T>`/`#[async_computed]`/`task!`マクロは未実装 |
| Q | リスト仮想化(`VirtualList`) | 未実装 |
| R | テーマ/デザイントークン(`theme`) | 未実装 |
| S | エラーバウンダリ(`ErrorBoundary`) | 未実装(`.elwind`宣言なし) |
| T | クリップボード/D&D/ファイルダイアログ | §3参照(file_dialogのみ実装) |
| U | Undo/Redo(`#[undoable]`) | 未実装 |
| V | スナップショットテスト | `render_tree`のみ実装。`render_canvas_snapshot`は未実装(§1参照) |
| W | モバイル対応(iOS/Android) | 未実装(設計のみ) |

---

## 6. ツールチェーン状況(`docs/elwindui_tool_*_design.md`)

| ツール | 状況 |
|---|---|
| `elwindui-codegen`(コード生成) | 実装済み。`build.rs`経由・プロシージャルマクロ経由の両方が実働。バックエンド選択の定数畳み込み(付録D)は前提機能が無いため未実装 |
| `elwindui-languageserver`(LSP) | 部分実装。診断・シンタックスハイライト・メンバー補完まで実働。hover・プレビュー用インスタンス生成パイプラインは未実装 |
| ホットリロード(`elwindui-hotreload`) | スタブのみ。remount/patch判定ロジックのみ存在、dylib差し替えは未実装 |
| リアルタイムプレビュー | **クレート自体が存在しない**。100%未着手 |

---

## 7. 既知の主なギャップまとめ

- **GTK4バックエンドは事実上何も実装されていない**(2行のスタブ)。本ドキュメントの他の章で「WinUI3/AppKit/GTK4」と横並びで書かれている箇所の多くは、GTK4に関しては未着手であることに注意。
- **フォーカス管理・アクセシビリティは型定義のみ**で、`UIElement`ツリーにもバックエンドのネイティブAPI(`AutomationPeer`/`NSAccessibilityElement`/AT-SPI)にも未結線。
- **ルーティングイベント(`#[routed]`)の実配線はAppKitバックエンドの`Button`のみ検証済み**で、トンネリングイベント・`Canvas`上のポインタイベント・WinUI3での実配線は未着手。
- **`store`/`viewmodel`のうち`viewmodel`(MVVM)は実装済みだが`store`(グローバル状態)は未実装**——`examples/notepad`のMVVMは`viewmodel`のみで構成されている。
- **`Backend` enum / `target::backend()`が存在しないため、これに依存する多くの静的検証ルール・ビルトイン(`NavigationHost`、ダイアログ/メニューのバックエンド分岐等)が「未実装」の根本原因になっている。** 将来この仕組みを実装する際は、影響範囲がドキュメント全体に及ぶことに留意する。
