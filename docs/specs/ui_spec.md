# ElwindUI UI Specification

本仕様書は、ElwindUI が提供する標準UI型およびそれらの公開インターフェース、プロパティ、イベント、バインディング意味論を定義する規範仕様（Normative Specification）である。

---

## 1. Scope

本書は `elwindui::ui` モジュールが公開するUI型（コントロール、レイアウト、シェイプ、ウィンドウ、メニュー、タブ等）の抽象モデルと各種プロパティ・イベントの契約を定義する。

特定platformのbackend実装や内部データ構造・codegenの詳細は本書の対象外であり、[`../design/README.md`](../design/README.md) から対象designを選ぶ。共通text propertyは [`text_style_spec.md`](text_style_spec.md) を正本とする。

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

`UIElement` を継承する全てのビジュアル要素は、共通して以下のレイアウト・外観・フォーカスプロパティを持つ。

| Name | Type | Binding | Description |
|---|---|---|---|
| `margin` | `Option<f32>` | OneWay | 外側余白 |
| `horizontal_alignment` | `Option<HorizontalAlignment>` | OneWay | 水平方向の配置（`Left`, `Center`, `Right`, `Stretch`） |
| `vertical_alignment` | `Option<VerticalAlignment>` | OneWay | 垂直方向の配置（`Top`, `Center`, `Bottom`, `Stretch`） |
| `visibility` | `Option<Visibility>` | OneWay | 表示状態（`Visible`, `Collapsed`） |
| `width` | `Option<f32>` | OneWay | 明示的幅 |
| `height` | `Option<f32>` | OneWay | 明示的高さ |
| `min_width` | `Option<f32>` | OneWay | 最小幅 |
| `max_width` | `Option<f32>` | OneWay | 最大幅 |
| `min_height` | `Option<f32>` | OneWay | 最小高さ |
| `max_height` | `Option<f32>` | OneWay | 最大高さ |
| `hit_test_visible` | `Option<bool>` | OneWay | ヒットテスト判定の有効/無効 |
| `tab_stop` | `Option<bool>` | OneWay | Tab キーフォーカス移動の対象に含まれるか |
| `focus_order` | `Option<i32>` | OneWay | Tab キー移動時の明示的な優先度順序 |

`NativeControl` を継承する全ての具象コントロールは、さらに以下の共通プロパティを持つ。

| Name | Type | Binding | Description |
|---|---|---|---|
| `background` | `Option<Brush>` | OneWay | 背景ブラシ |
| `tooltip` | `Option<String>` | OneWay | ツールチップテキスト |
| `font_family` | `Option<FontFamily>` | OneWay | フォントファミリー（`#[text_style]`） |
| `font_size` | `Option<f32>` | OneWay | フォントサイズ（`#[text_style]`） |
| `font_weight` | `Option<FontWeight>` | OneWay | フォントウェイト（`#[text_style]`） |
| `font_style` | `Option<FontStyle>` | OneWay | フォントスタイル（`#[text_style]`） |
| `font_stretch` | `Option<FontStretch>` | OneWay | フォントストレッチ（`#[text_style]`） |
| `character_spacing` | `Option<f32>` | OneWay | 文字間隔（`#[text_style]`） |
| `foreground` | `Option<Brush>` | OneWay | 文字色ブラシ（`#[text_style]`） |

### 2.4 Content model

各型は子要素の受け取り方式に応じて以下のいずれかのコンテンツモデルを持つ。

- **Leaf**: 子要素を持たない末端要素（例: `TextBlock`, `Button`）。
- **Single Content**: 単一のコンテンツスロット（`#[content(content)]`）を持つ要素（例: `ContentControl`, `ScrollView`, `Window`）。
- **Collection Content**: 複数の子要素コレクション（`#[content(children)]` または `#[content(items)]`）を持つ要素（例: `VerticalLayout`, `Grid`, `Menu`, `TabView`）。

### 2.5 Layout and rendering semantics

ElwindUI のすべてのビジュアル要素（`UIElement`）は、**Measure（計測）**、**Arrange（配置）**、**Render（描画）** の3段階のライフサイクルに従ってレイアウト計算および描画更新を行う。

#### 1. Measure パス（サイズ計測）

- **インターフェース**: `measure(available_size: Size) -> Size`
- **目的**: 親コンテナから提示された利用可能領域（`available_size`）の制約下で、要素が希望する自然サイズ（Desired Size）を計算する。
- **制約評価と並び順**:
  1. `margin` を `available_size` から減算する。
  2. 明示的なプロパティ（`width`, `height`, `min_width`, `max_width`, `min_height`, `max_height`）によるサイズクランプを適用する。
  3. コレクション・コンテンツを持つ要素は、各子要素の `measure` を呼び出し、レイアウト規則（縦並び、横並び、グリッド等）に従って自身の Desired Size を決定する。
- **`visibility` の影響**:
  - `Visible`: 通常通りサイズ計測を行い、レイアウト領域を占有する。
  - `Collapsed`: 計測結果を常に Desired Size `(0, 0)` とみなし、レイアウト計算から完全に除外される。
- **無制約計測（Unconstrained Measure）**:
  - `ScrollView` などのスクロール可能軸では、利用可能サイズとして `f32::INFINITY`（無制約）を子要素へ渡し、コンテンツ本来の自然サイズへ成長させる。

#### 2. Arrange パス（絶対配置）

- **インターフェース**: `arrange(final_rect: Rect)`
- **目的**: 親コンテナが決定した最終的な確定領域（`final_rect`）を基に、要素自身の配置位置（Offset）と最終サイズ（Render Size）を確定し、必要に応じて子要素へサブ領域を割り当てる。
- **整列（Alignment）の適用**:
  - `horizontal_alignment` (`Left`, `Center`, `Right`, `Stretch`) および `vertical_alignment` (`Top`, `Center`, `Bottom`, `Stretch`) を評価し、`final_rect` 内での配置座標を確定する。
  - `margin` によるオフセットを最終位置へ加算する。

#### 3. Render パス（描画出力）

- **目的**: Arrange パスで確定した領域情報に従い、画面へビジュアル要素を出力する。
- `Shape`や`TextBlock`等の自前描画要素と`NativeControl`は、同じarranged bounds、visibility、clip、opacity semanticsに従って出力される。

#### 4. レイアウトの無効化（Layout Invalidation）

見た目やサイズに影響を与えるproperty（`width`, `height`, `margin`, `text`, `visibility` 等）が更新された場合、必要なMeasure/Arrange/Renderが次のUI更新で再実行される。同じ更新中の複数変更はobservableな中間layoutを公開せずcoalesceしてよい。

### 2.6 Event routing model

ElwindUI のイベント伝播はルーティングイベントモデルを採用している。`#[routed]` が指定されたイベントは、単一要素内にとどまらず視覚ツリー（Visual Tree）に沿って伝播する。

#### 1. Routing Strategies（ルーティング戦略）

イベントの種類に応じて以下の2つのルーティング戦略が用いられる。

1. **Bubbling（バブリング / 下から上へ）**:
   - イベント発生源（Target）のノードから開始し、`visual_parent` チェーンを辿ってルート要素（Window/Root）に向けて親方向へ順次伝播する。
   - 主な対象: マウス/ポインタイベント（`on_pointer_pressed`, `on_pointer_released`, `on_pointer_moved`, `on_pointer_wheel_changed`, `on_tapped`, `on_double_tapped`, `on_right_tapped`）、キーボード入力（`on_key_down`, `on_key_up`, `on_text_input`）。
2. **Direct（ダイレクト / ルーティングなし）**:
   - ツリー伝播を行わず、イベントが発生した特定の要素のハンドラのみを直接呼び出す。
   - 主な対象: 領域進入/退出（`on_pointer_entered`, `on_pointer_exited`）、フォーカス変化（`on_got_focus`, `on_lost_focus`）、コントロール固有の状態変化通知（`TabView::on_select` 等）。

#### 2. Event Handling & Handled Flag

ルーティングイベントは引数として `RoutedEventArgs` を受け取る。
- **`handled` フラグ**: いずれかのハンドラが `args.set_handled(true)` を呼び出すと、その時点で上位ノードへのイベント伝播が打ち切られる。
- コントロールが固有の標準動作（例: `Button` が Enter キーでクリックを発火する処理）を完了した場合、通常 `handled = true` を設定して親要素への重複伝播を防止する。

#### 3. Input Dispatch & Hit Testing

- **ポインタイベント**:
  - 画面座標を基に視覚ツリーを検索（`hit_test`）し、クリッピング、表示状態（`visibility`）、ヒットテスト有効性（`hit_test_visible`）を考慮して最前面のターゲット要素を決定した後、Bubbling/Direct ルーティングを開始する。
- **キーボードイベント**:
  - フォーカス管理機構（Focus Tracker）が保持する現在のフォーカス要素をターゲットとして決定した後、Bubbling ルーティングを開始する。

### 2.7 Focus management and keyboard navigation

ElwindUI は、キーボード入力およびアクセシビリティ操作のためのフォーカス管理モデルを提供する。

#### 1. Focus Model & Scope

- **フォーカスモデル**: 各ウィンドウ/ルートコンテナは、ツリー内で現在アクティブな単一のキーボードフォーカス保持要素を追跡・管理する。
- **フォーカス移動**: マウスクリック、Tabキー操作、またはプログラムによる `focus()` 呼び出しによってフォーカスが切り替わる。

#### 2. Focus Events

フォーカス状態の変化に伴い、以下のルーティングイベントが発火する。

| Event Name | Signature | Strategy | Description |
|---|---|---|---|
| `on_got_focus` | `fn()` | Direct | 要素がフォーカスを獲得した際に発火 |
| `on_lost_focus` | `fn()` | Direct | 要素からフォーカスが離脱した際に発火 |

#### 3. Keyboard Navigation Semantics

- **Tab / Shift+Tab 遷移**:
  - `focus_order` の昇順に従ってフォーカスを移動する。`focus_order` が同値の場合は視覚ツリーの先順位（Pre-order traversal）に従う。`tab_stop: false` の要素はスキップされる。
- **デフォルトボタン（Default Accelerator）**:
  - 単一ラインテキストボックス（`TextBox`）等の入力中に Enter キーが押下された場合、現在のウィンドウ内で `is_default: true` が指定された `Button` のクリックイベントを自動発火する。

---

## 3. Base types

### `UIElement`

全てのビジュアル要素の抽象基底カテゴリ。

#### Common UIElement Events

| Event Name | Signature | Strategy | Description |
|---|---|---|---|
| `on_key_down` | `fn(KeyEventArgs)` | Bubbling | フォーカス保持中に物理キーが押下された際に発火 |
| `on_key_up` | `fn(KeyEventArgs)` | Bubbling | フォーカス保持中に物理キーが離された際に発火 |
| `on_text_input` | `fn(TextInputEventArgs)` | Bubbling | 文字列テキスト入力が行われた際に発火 |
| `on_got_focus` | `fn()` | Direct | 要素がフォーカスを獲得した際に発火 |
| `on_lost_focus` | `fn()` | Direct | 要素からフォーカスが脱落した際に発火 |
| `on_pointer_pressed` | `fn(PointerEventArgs)` | Bubbling | ポインタが要素上で押下された際に発火 |
| `on_pointer_released` | `fn(PointerEventArgs)` | Bubbling | ポインタが要素上で離された際に発火 |
| `on_pointer_moved` | `fn(PointerEventArgs)` | Bubbling | ポインタが要素上で移動した際に発火 |
| `on_pointer_entered` | `fn(PointerEventArgs)` | Direct | ポインタが要素領域内に進入した際に発火 |
| `on_pointer_exited` | `fn(PointerEventArgs)` | Direct | ポインタが要素領域外へ退出した際に発火 |
| `on_pointer_wheel_changed` | `fn(PointerWheelEventArgs)` | Bubbling | マウスホイール操作時に発火 |
| `on_tapped` | `fn(TappedEventArgs)` | Bubbling | タップ/クリック操作時に発火 |
| `on_double_tapped` | `fn(TappedEventArgs)` | Bubbling | ダブルタップ操作時に発火 |
| `on_right_tapped` | `fn(TappedEventArgs)` | Bubbling | 右クリック/副操作時に発火 |

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
| `stroke_width` | `Option<f32>` | OneWay | 輪郭線の太さ |

### `Control`

スタイリング、テーマ設定、テンプレート合成に対応した汎用コントロール基底。
template-enabled派生型ではmount時に選択した単一template rootをVisual childとして保持し、logical childにはしない。
typed factoryと選択規則は[ControlTemplate Specification](control_template_spec.md)で定義する。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `children` | `UIElementCollection` | OneWay | 子要素のコレクション |
| `padding` | `Option<f32>` | OneWay | 内側余白 |

### `ContentControl`

`Control` を継承し、単一のコンテンツプロパティを提供する基底。
raw `ContentControl`は従来どおりcontentをdirect Visual childとして表示する。template-enabled派生型では
contentのlogical parentを`ContentControl`に保ったまま、template内の`ContentPresenter`だけがVisual配置する。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `content` | `UIElement` | OneWay | 保持する単一の子要素 |

### `ContentPresenter`

templated parentのlogical contentを自身の単一Visual childとして表示するbackend非依存Control。
logical parentは書き換えず、`set_content`による置換へ追従する。通常のapplication treeで独立した
content containerとして使うためのControlではない。

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

#### Lifecycle (CI-8 of #80, `docs/design/runtime/component_lifecycle_design.md` §4g)

A `#[elwindui::component(inherits Window)]`-declared component ("host composition") is initially not mounted: `let window = MyWindow::new(..);` creates the logical instance and the native window shell but does not evaluate its `view!` body or build its content tree.

- `show()`: on the first call on an unmounted instance, establishes this Window's effective Environment (derived from the Application Environment — `elwindui::core::environment::application_environment()`), performs the initial `view!` build exactly once, then displays the native window. A property set between `new()` and the first `show()` (e.g. `set_title`) is observed by that initial build. Re-showing an already-mounted, hidden window does not remount or rebuild it.
- `hide()`: visibility only. The mounted content tree, Environment subscriptions, and component state all remain alive. A subsequent `show()` makes the window visible again without remounting/rebuilding.
- `close()`: ends the mount lifetime — cancels this component's own property-changed/`on_update`/Environment subscriptions and releases the native window. Does not (yet) recursively cascade unmount into descendant Components' own subscriptions/state; see the design doc for the current implementation boundary.

---

---

## 5. Layout

### `elwindui::ui::VerticalLayout`

子要素を上から下へ垂直方向に一列に配置するレイアウトコンテナ。`Layout` を継承し `children` および `background` を共有する。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `spacing` | `Option<f32>` | OneWay | 子要素間の垂直スペーシング |

### `elwindui::ui::HorizontalLayout`

子要素を左から右へ水平方向に一列に配置するレイアウトコンテナ。`Layout` を継承し `children` および `background` を共有する。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `spacing` | `Option<f32>` | OneWay | 子要素間の水平スペーシング |

### `elwindui::ui::Grid`

行（Row）と列（Column）の格子状に子要素を配置するレイアウトコンテナ。`Layout` を継承し `children` および `background` を共有する。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `rows` | `Vec<GridLength>` | OneTime | 行サイズ定義（`Auto`, `Fixed(f32)`, `Star(f32)`） |
| `columns` | `Vec<GridLength>` | OneTime | 列サイズ定義（`Auto`, `Fixed(f32)`, `Star(f32)`） |

#### Attached Properties

Gridの直下にある子要素は以下の添付プロパティを指定できる。

- `Grid::row`: 配置対象の行インデックス（0開始、既定値 `0`）
- `Grid::column`: 配置対象の列インデックス（0開始、既定値 `0`）

#### Example

```rust
Grid {
    rows: [GridLength::Auto, GridLength::Star(1.0)]
    columns: [GridLength::Fixed(200.0), GridLength::Star(1.0)]
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

`TextBlock` は `#[text_style]` を持ち、共通フォント属性（`font_family`, `font_size`, `font_weight`, `font_style`, `font_stretch`, `character_spacing`, `foreground`）が使用可能である。

### `elwindui::ui::Image`

ラスタ画像またはベクター画像を表示する要素。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `source` | `Option<ImageSource>` | OneWay | 画像データソース（ファイルパス/ビットマップ/VectorImage） |
| `stretch` | `Option<Stretch>` | OneWay | 拡大縮小ストレッチモード（`None`, `Fill`, `Uniform`, `UniformToFill`） |
| `rasterize` | `Option<VectorRasterizeMode>` | OneWay | ベクターラスタライズモード |

---

## 7. Shapes

### `elwindui::ui::Rectangle`

長方形を描画する図形要素。`Shape` を継承し `fill`, `stroke`, `stroke_width` を共有する。

#### Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `corner_radius` | `Option<f32>` | OneWay | 長方形の四角の角丸半径 |

### `elwindui::ui::Ellipse`

楕円・円を描画する図形要素。`Shape` を継承し `fill`, `stroke`, `stroke_width` を共有する。

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
