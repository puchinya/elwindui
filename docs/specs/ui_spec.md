# ElwindUI UI Specification

本仕様書は、ElwindUI が提供する標準UI型およびそれらの公開インターフェース、プロパティ、イベント、バインディング意味論を定義する規範仕様（Normative Specification）である。

---

## 1. Scope

本書は `elwindui::ui` モジュールが公開するUI型（コントロール、レイアウト、シェイプ、ウィンドウ、メニュー、タブ等）の抽象モデルと各種プロパティ・イベントの契約を定義する。

特定プラットフォームのバックエンド実装（AppKit / WinUI 3 / GTK4 等）や内部データ構造・コード生成機構の詳細は本書の対象外であり、[GUI Framework Design](../design/gui_framework_design.md) を参照すること。

---

## 2. Common model

### 2.1 Rust namespace

ElwindUI の公開UI型は原則として以下の正準パスで公開される。

```rust
elwindui::ui::<Type>
```

本仕様書内では、可読性のために `elwindui::ui::` を省略して `Button` や `Window` のように表記する。DSL上では `use elwindui::ui::*;` により裸名で参照される。

### 2.2 UI element categories

標準UI型は以下のカテゴリ階層に従う。

- **`UIElement`**: 全てのビジュアル要素が共有する抽象基底モデル。
- **`NativeControl`**: ネイティブウィジェットとしてレンダリングされるコントロール群の抽象基底。
- **`Layout`**: 複数の子要素を配置・整列するレイアウトコンテナ群の抽象基底。
- **`Shape`**: 自己描画を行う図形プリミティブの抽象基底。
- **`Control`**: テンプレート合成やパディングを持つ汎用コントロール基底。
- **`ContentControl`**: 単一のコンテンツスロットを持つコントロール基底。

### 2.3 Common properties

`UIElement` を継承する全てのビジュアル要素は、共通して以下のレイアウト・外観プロパティを持つ。

| Name | Type | Binding | Description |
|---|---|---|---|
| `margin` | `Thickness` | OneWay | 外側余白 |
| `horizontal_alignment` | `HorizontalAlignment` | OneWay | 水平方向の配置（`Left`, `Center`, `Right`, `Stretch`） |
| `vertical_alignment` | `VerticalAlignment` | OneWay | 垂直方向の配置（`Top`, `Center`, `Bottom`, `Stretch`） |
| `visibility` | `Visibility` | OneWay | 表示状態（`Visible`, `Collapsed`, `Hidden`） |
| `grid_cell` | `GridCell` | OneWay | `Grid` 配置時の行・列位置（添付プロパティ `Grid::row` / `Grid::column`） |
| `width` | `Option<f32>` | OneWay | 明示的幅 |
| `height` | `Option<f32>` | OneWay | 明示的高さ |
| `min_width` | `Option<f32>` | OneWay | 最小幅 |
| `max_width` | `Option<f32>` | OneWay | 最大幅 |
| `min_height` | `Option<f32>` | OneWay | 最小高さ |
| `max_height` | `Option<f32>` | OneWay | 最大高さ |

`NativeControl` を継承する全ての具象コントロールは、さらに以下のプロパティを持つ。

| Name | Type | Binding | Description |
|---|---|---|---|
| `enabled` | `Option<bool>` | OneWay | コントロールの有効/無効状態 |
| `tooltip` | `Option<String>` | OneWay | ツールチップテキスト |

### 2.4 Content model

各型は子要素の受け取り方式に応じて以下のいずれかのコンテンツモデルを持つ。

- **Leaf**: 子要素を持たない末端要素（例: `TextBlock`, `Button`）。
- **Single Content**: 単一のコンテンツスロット（`#[content(content)]`）を持つ要素（例: `ContentControl`, `ScrollView`, `Window`）。
- **Collection Content**: 複数の子要素コレクション（`#[content(children)]` または `#[content(items)]`）を持つ要素（例: `VerticalLayout`, `Grid`, `Menu`, `TabView`）。

---

## 3. Base types

### `UIElement`

全てのビジュアル要素の抽象基底カテゴリ。

### `NativeControl`

ネイティブプラットフォームのウィジェットと接続される要素の基底カテゴリ。

### `Layout`

子要素のサイズ計測（Measure）および配置（Arrange）を統括するレイアウトコンテナの基底カテゴリ。

### `Shape`

ベクター図形を描画する要素の基底カテゴリ。[Graphics Specification](graphics_spec.md) で定義される描画型を参照する。

#### Common Shape Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `fill` | `Option<Brush>` | OneWay | 塗りつぶしブラシ |
| `stroke` | `Option<Brush>` | OneWay | 輪郭線ブラシ |
| `stroke_style` | `Option<StrokeStyle>` | OneWay | 輪郭線のスタイル |

### `Control`

スタイリング、テーマ設定、テンプレート合成に対応した汎用コントロール基底。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `padding` | `Thickness` | OneWay | 内側余白 |
| `background` | `Option<Brush>` | OneWay | 背景ブラシ |

### `ContentControl`

`Control` を継承し、単一のコンテンツプロパティを提供する基底。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `content` | `UIElement` | OneWay | 保持する単一の子要素 |

---

## 4. Window

### `elwindui::ui::Window`

アプリケーションの最上位領域を表すトップレベルウィンドウ。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `title` | `String` | OneWay | ウィンドウタイトル |
| `menu_bar` | `Option<MenuBar>` | OneWay | アプリケーションメニューバー |
| `content` | `UIElement` | OneWay | ウィンドウのメインコンテンツ |
| `theme` | `Option<ThemeHandle>` | OneWay | ウィンドウ固有のテーマ設定 |
| `left` | `Option<f32>` | OneTime | 初期X座標 |
| `top` | `Option<f32>` | OneTime | 初期Y座標 |
| `width` | `Option<f32>` | OneTime | 初期幅 |
| `height` | `Option<f32>` | OneTime | 初期高さ |

#### Example

```rust
Window {
    title: "ElwindUI Application"
    width: 800.0
    height: 600.0
    content: VerticalLayout {
        TextBlock { text: "Hello, ElwindUI!" }
    }
}
```

---

## 5. Layout

### `elwindui::ui::VerticalLayout`

子要素を上から下へ垂直方向に一列に配置するレイアウトコンテナ。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `children` | `Vec<UIElement>` | OneWay | 子要素のコレクション |

### `elwindui::ui::HorizontalLayout`

子要素を左から右へ水平方向に一列に配置するレイアウトコンテナ。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `children` | `Vec<UIElement>` | OneWay | 子要素のコレクション |

### `elwindui::ui::Grid`

行（Row）と列（Column）の格子状に子要素を配置するレイアウトコンテナ。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `rows` | `Vec<GridLength>` | OneTime | 行サイズ定義（`Auto`, `Px(f32)`, `Star(f32)`） |
| `columns` | `Vec<GridLength>` | OneTime | 列サイズ定義（`Auto`, `Px(f32)`, `Star(f32)`） |
| `children` | `Vec<UIElement>` | OneWay | 子要素のコレクション |

#### Attached Properties

Gridの直下にある子要素は以下の添付プロパティを指定できる。

- `Grid::row`: 配置対象の行インデックス（0開始）
- `Grid::column`: 配置対象の列インデックス（0開始）

#### Example

```rust
Grid {
    rows: [GridLength::Auto, GridLength::Star(1.0)]
    columns: [GridLength::Px(200.0), GridLength::Star(1.0)]
    children: [
        TextBlock {
            Grid::row: 0
            Grid::column: 0
            text: "Sidebar"
        },
        TextBlock {
            Grid::row: 0
            Grid::column: 1
            text: "Main Content"
        }
    ]
}
```

### `elwindui::ui::ScrollView`

スクロール可能な領域の中に単一のコンテンツ要素を配置するコンテナ。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `content` | `UIElement` | OneWay | スクロール表示対象のコンテンツ |
| `horizontal_scroll_enabled` | `Option<bool>` | OneWay | 水平スクロールの有効化 |
| `vertical_scroll_enabled` | `Option<bool>` | OneWay | 垂直スクロールの有効化 |

---

## 6. Display

### `elwindui::ui::TextBlock`

テキストを表示する読み取り専用の表示要素。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | OneWay | 表示テキスト |
| `text_alignment` | `Option<TextAlignment>` | OneWay | テキスト配置（`Left`, `Center`, `Right`, `Justified`） |
| `text_wrapping` | `Option<TextWrapping>` | OneWay | テキスト折り返し（`NoWrap`, `Wrap`） |
| `font_family` | `Option<FontFamily>` | OneWay | フォントファミリー |
| `font_size` | `Option<f32>` | OneWay | フォントサイズ（pt/px） |
| `font_weight` | `Option<FontWeight>` | OneWay | フォントウェイト |
| `font_style` | `Option<FontStyle>` | OneWay | フォントスタイル（`Normal`, `Italic`） |
| `foreground` | `Option<Brush>` | OneWay | 文字色ブラシ |

### `elwindui::ui::Image`

ラスタ画像またはベクター画像を表示する要素。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `source` | `Option<ImageSource>` | OneWay | 画像データソース（ファイルパス/ビットマップ/VectorImage） |
| `fit` | `Option<ImageFit>` | OneWay | 拡大縮小フィットモード（`None`, `Fill`, `Contain`, `Cover`） |
| `sampling` | `Option<ImageSampling>` | OneWay | サンプリングフィルタ（`Linear`, `Nearest`） |

---

## 7. Shapes

### `elwindui::ui::Rectangle`

長方形を描画する図形要素。角丸（`corner_radius`）をサポートする。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `corner_radius` | `Option<CornerRadius>` | OneWay | 長方形の四角の角丸半径 |
| `fill` | `Option<Brush>` | OneWay | 塗りつぶしブラシ |
| `stroke` | `Option<Brush>` | OneWay | 輪郭線ブラシ |
| `stroke_style` | `Option<StrokeStyle>` | OneWay | 輪郭線のスタイル |

### `elwindui::ui::Ellipse`

楕円・円を描画する図形要素。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `fill` | `Option<Brush>` | OneWay | 塗りつぶしブラシ |
| `stroke` | `Option<Brush>` | OneWay | 輪郭線ブラシ |
| `stroke_style` | `Option<StrokeStyle>` | OneWay | 輪郭線のスタイル |

---

## 8. Input

### `elwindui::ui::Button`

ユーザーによるクリック操作を受け付けるネイティブのプッシュボタン。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | OneWay | ボタンのラベルテキスト |
| `enabled` | `Option<bool>` | OneWay | 有効/無効状態 |
| `role` | `Option<ButtonRole>` | OneWay | 強調役割（`Normal`, `Primary`, `Destructive`） |
| `is_default` | `Option<bool>` | OneWay | ウィンドウの既定ボタン指定 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_click` | `fn()` | ボタンクリック時に発火 |

#### Semantics

- `role` はボタンの操作の意味的強調を表し、プラットフォーム固有のアクセント/破壊的表現にマップされる。
- `is_default` は Enter キーによる規定実行対象かを制御し、`role` とは独立している。

#### Example

```rust
Button {
    text: "保存"
    role: ButtonRole::Primary
    on_click: vm.save_document
}
```

### `elwindui::ui::TextBox`

単一ラインのテキスト入力コントロール。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | TwoWay | 入力テキスト |
| `placeholder` | `Option<String>` | OneWay | プレースホルダーテキスト |
| `read_only` | `Option<bool>` | OneWay | 読み取り専用フラグ |
| `max_length` | `Option<u32>` | OneWay | 最大入力文字数制限 |
| `text_alignment` | `Option<TextAlignment>` | OneWay | 入力テキストの水平配置 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(String)` | テキスト変更時に発火 |

### `elwindui::ui::TextArea`

複数ラインのテキスト入力・編集コントロール。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | TwoWay | 入力テキスト内容 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(String)` | 内容変更時に発火 |

### `elwindui::ui::PasswordBox`

マスク表示される安全なパスワード入力コントロール。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `password` | `String` | TwoWay | 入力パスワード文字列 |
| `placeholder` | `Option<String>` | OneWay | プレースホルダー |
| `max_length` | `Option<u32>` | OneWay | 最大入力文字数 |
| `reveal_enabled` | `Option<bool>` | OneWay | マスク解除表示ボタンの有効化（対応環境のみ） |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(String)` | 入力変更時に発火 |

### `elwindui::ui::CheckBox`

ON / OFF / （選択的）Indeterminate 状態を持つネイティブのチェックボックス。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | OneWay | ラベルテキスト |
| `checked` | `CheckState` | TwoWay | チェック状態（`Unchecked`, `Checked`, `Indeterminate`） |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(CheckState)` | 状態変更時に発火 |

#### Semantics

ユーザーのクリック操作は `Unchecked` と `Checked` の2状態間のみを切り替える。`Indeterminate`（中間状態）はプログラムからの状態表示専用である。

### `elwindui::ui::RadioButton`

同グループ内で相互排他的に選択されるラジオボタン。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | OneWay | ラベルテキスト |
| `checked` | `bool` | TwoWay | 選択状態 |
| `group` | `Option<String>` | OneWay | 相互排他グループ識別名 |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(bool)` | 状態変更時に発火 |

### `elwindui::ui::ToggleSwitch`

ON / OFF を切り替えるスイッチコントロール。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `is_on` | `bool` | TwoWay | スイッチのON/OFF状態 |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(bool)` | 状態変更時に発火 |

### `elwindui::ui::Slider`

範囲内の連続値を指定するスライダー。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `value` | `f32` | TwoWay | 現在の値 |
| `min` | `f32` | OneWay | 最小値 |
| `max` | `f32` | OneWay | 最大値 |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(f32)` | 値変更時に発火 |

### `elwindui::ui::Dropdown`

単一選択を行うドロップダウンリスト（ドロップダウンメニュー）。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `items` | `Vec<DropdownItem>` | OneWay | ドロップダウンの選択肢一覧 |
| `selected_index` | `usize` | TwoWay | 現在選択されているアイテムのインデックス |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_change` | `fn(usize)` | 選択インデックス変更時に発火 |

### `elwindui::ui::DropdownItem`

`Dropdown` 内の個々の選択項目。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | OneWay | 表示テキスト |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

---

## 9. Menu

### `elwindui::ui::MenuBar`

アプリケーション最上部に配置されるメニューバー。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `items` | `Vec<MenuBarItem>` | OneWay | 最上位メニュー項目のコレクション |

### `elwindui::ui::MenuBarItem`

メニューバー上の個々のトップレベル項目。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | OneWay | メニュー表示名 |
| `submenu` | `Menu` | OneWay | クリック時に開く子メニュー |

### `elwindui::ui::Menu`

`MenuItem` のコレクションを保持するドロップダウンメニュー。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `items` | `Vec<MenuItem>` | OneWay | メニュー項目のリスト |

### `elwindui::ui::MenuItem`

メニュー内の個々の実行可能項目。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `text` | `String` | OneWay | メニュー表示名 |
| `shortcut` | `Option<String>` | OneWay | キーボードショートカット記号（例: `"Cmd+S"`） |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_select` | `fn()` | メニュー選択時に発火 |

---

## 10. Tabs

### `elwindui::ui::TabView`

複数のタブページを切り替えて表示するコンテナ。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `children` | `Vec<TabViewItem>` | OneWay | タブ項目のリスト |
| `selected_index` | `usize` | TwoWay | 現在アクティブなタブのインデックス |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_select` | `fn(usize)` | アクティブタブ切り替え時に発火 |
| `on_new_tab` | `fn()` | 新規タブ追加ボタンが押された際に発火 |

### `elwindui::ui::TabViewItem`

`TabView` 内の1つのタブヘッダーおよびコンテンツのペア。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `header` | `String` | OneWay | タブタイトル |
| `content` | `UIElement` | OneWay | タブ選択時に表示される子UI |
| `closable` | `Option<bool>` | OneWay | 閉じボタンを表示するか否か |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_close` | `fn()` | タブ閉じるボタンが押された際に発火 |

---

## 11. Related specifications

- [DSL Specification](dsl_spec.md) - ElwindUI DSL の構文・バインディングルール
- [Graphics Specification](graphics_spec.md) - `Color`, `Brush`, `Path`, `Image` などの描画仕様
- [Platform Specification](platform_spec.md) - OSサービス（ファイルダイアログ等）の仕様
