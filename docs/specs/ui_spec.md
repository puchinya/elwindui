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
  - host-root-relative論理座標を基に視覚ツリーを検索（`hit_test`）し、クリッピング、表示状態（`visibility`）、ヒットテスト有効性（`hit_test_visible`）を考慮して最前面のターゲット要素を決定した後、Bubbling/Direct ルーティングを開始する。
  - press中は暗黙captureを維持し、pointerが元要素またはhost boundsの外へ移動しても`moved`/`released`をpress targetへ配送する。hoverの`entered`/`exited`判定は実cursor位置を使い続ける。
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

`PointerEventArgs.position`は常にhost root左上を原点とするY-down論理座標であり、bubble先要素相対ではない。`screen_position: Option<Point>`は同じnative eventの位置をprimary desktop左上基準のY-down論理座標へ正規化した値である。backendがnative変換を提供できない場合は`None`とし、Windowの外形位置やtitlebar寸法から推定してはならない。

`CoordinateHost`はhosted treeに同じ座標変換を提供する。

```rust
pub trait CoordinateHost {
    fn root_to_screen(&self, point: Point) -> Option<Point>;
    fn screen_to_root(&self, point: Point) -> Option<Point>;
}
```

両方向とも論理座標を受け取り、native変換に失敗した場合は`None`を返す。
#### Common UIElement Properties

| Name | Type | Binding | Description |
|---|---|---|---|
| `margin` | `Option<f32>` | OneWay | 外側余白 |
| `horizontal_alignment` | `Option<HorizontalAlignment>` | OneWay | 水平配置（Stretch, Left, Center, Right） |
| `vertical_alignment` | `Option<VerticalAlignment>` | OneWay | 垂直配置（Stretch, Top, Center, Bottom） |
| `visibility` | `Option<Visibility>` | OneWay | 表示状態（Visible, Collapsed） |
| `hit_test_visible` | `Option<bool>` | OneWay | ヒットテスト対象か否か（既定値: `true`） |
| `tab_stop` | `Option<bool>` | OneWay | キーボードフォーカス遷移（Tabキー）の対象か否か |
| `focus_order` | `Option<i32>` | OneWay | キーボードフォーカス移動の優先順位 |
| `width` | `Option<f32>` | OneWay | 明示的な幅 |
| `height` | `Option<f32>` | OneWay | 明示的な高さ |
| `min_width` | `Option<f32>` | OneWay | 最小幅 |
| `min_height` | `Option<f32>` | OneWay | 最小高さ |
| `max_width` | `Option<f32>` | OneWay | 最大幅 |
| `max_height` | `Option<f32>` | OneWay | 最大高さ |
| `context_menu` | `Option<Menu>` | OneWay | 要素またはその子孫に対する標準コンテキストメニュー（既定値: `None`） |
| `context_menu_presentation` | `ContextMenuPresentation` | OneWay | コンテキストメニューの表示方式（`Native` / `Custom`, 既定値: `Native`） |
| `context_popup` | `Option<ViewTemplate>` | OneWay | 任意のUIElementツリーを内容とするCustom Context Popup（既定値: `None`）。`ViewTemplate` は popup 専用ではない汎用の deferred View factory型 — 詳細は `docs/design/runtime/view_template_design.md` |

#### Context Request & Lookup Semantics

- **Context Request**:
  - コンテキストメニュー/ポップアップの表示要求は、プラットフォーム標準のコンテキスト操作（pointer-based platform context request、platform-standard keyboard context request、アクセシビリティ操作等）によって発生する。
  - Core は具体的な物理入力（右ボタン、Shift+F10、Ctrl+Click等）に依存せず、プラットフォーム中立な `ContextRequest` セマンティクスを扱う。
  - 既存の `on_right_tapped` は低レベルなポインタジェスチャイベントとして独立して維持され、コンテキストメニューの有無によって抑止・置換されない。
- **Ancestor Lookup**:
  - イベント対象の要素自身に `context_menu` / `context_popup` が指定されていない場合、視覚ツリー（Visual Tree）の親要素（`visual_parent`）をルート方向へ探索し、**最も近い祖先（nearest ancestor）** に設定された定義を採用する。
- **排他制約**:
  - 同一の要素に対して `context_menu` と `context_popup` を同時に指定することは禁止され、DSL コンパイル時エラーとなる。
- **Presentation**:
  - `ContextMenuPresentation::Native`: プラットフォーム固有のネイティブメニュー（macOS: `NSMenu`, Windows: `MenuFlyout`）を用いて表示する。
  - `ContextMenuPresentation::Custom`: 標準 `Menu` セマンティックモデルを ElwindUI 自前描画の `ContextMenuPresenter` を介して `PopupSurface` 上に表示する。
  - `context_popup` は常に Custom Render（`PopupSurface`）として表示される。

#### PopupSurface & Lifecycle

- **PopupSurface**:
  - プラットフォームの独立ポップアップサーフェス（ウィンドウ/パネル）を利用し、owner Window の境界を越えて表示可能。
  - owner Window より前面かつ `NativeControl` よりも前面の Z-order を維持する（application-global な最前面にはしない）。
  - モニターの有効表示領域（work area）内に収まるよう、右/下の端部では自動的に左/上方向へ反転（flip）配置される。
- **Lifecycle & Environment**:
  - `context_popup` の内容は表示要求（open）時点で構築（build）される。owner Component の mount 時点で一度だけ構築されるのではなく、popup が開かれるたびに新たに `ViewTemplate::build` が呼ばれる。
  - `ViewTemplate` の factory は owner を `Weak` としてのみ捕捉する（strong retain cycle を作らない）。owner が既に解放されている場合、`ViewTemplate::build` は `None` を返し、popup は表示されない。
  - **契約の区別**: 上記は `ViewTemplate` 自体（低レベル API、`ViewTemplate::new(|ctx| ...)`）が機械的に保証する範囲である。「owner の現在値を自動的に読む」こと自体は、この低レベル API 単体では保証されない——`ctx.owner`/`ctx.environment` から現在値を読むかどうかはクロージャの書き方次第であり、誤って構築前の値をキャプチャすることも可能である。「owner の bare な識別子参照が自動的に現在値を読む」という保証は、宣言的 `context_popup: view! { .. }` DSL（下記、実装済み）が提供する、より強い契約である。詳細は [`../design/runtime/view_template_design.md`](../design/runtime/view_template_design.md) §4 を参照。
  - build 対象の `EnvironmentContext` は、ターゲット要素の有効な Environment から `derive()` された popup 専用の派生コンテキストであり、ターゲット要素自身の Environment を変更しない。この派生コンテキストには宣言的な dismiss 用の `PopupDismissAction`（`crate::ui::popup::PopupDismissActionKey`、`#[environment(popup_dismiss)]` フィールド構文で解決可能なフレームワーク組み込みキー）が設定される。
  - **Build abort（owner 消失）**: `ViewTemplate::build` は owner が既に解放されている場合 `None` を返す（この判定は `ViewTemplate` 自体が機械的に強制する——factory クロージャが `ctx.owner` を一度も参照しなくても、owner が死んでいれば factory 自体が呼ばれない）。`ContextMenuService::open_custom_popup` はこの場合 popup を一切表示せず `None` を返す。
  - **Pre-show dismiss（build/mount 中の dismiss）**: build 中（将来、宣言的 Component root の `on_mount` を含む）に `PopupDismissAction::dismiss()` が呼ばれた場合、popup は一切表示されない（表示してから即座に閉じる、という動作にはしない）。この時点で構築済みの content は `unmount_subtree` により正しく teardown される。ネイティブサーフェスがまだ存在しない段階の dismiss 要求を握りつぶさないための保証。
  - **Backend show failure（ネイティブ表示の失敗）**: `PopupHost::show_popup` は表示に失敗した場合 `None` を返す（WinUI3 の座標変換・`Popup` 生成失敗等）。バックエンドは「実体のない `PopupSurfaceHandle`」を返してはならない。表示に失敗した場合、`ContextMenuService::open_custom_popup`/`open_custom_menu` は既に構築済みの content を `unmount_subtree` により teardown してから `None` を返す。
  - **Close 時の解放（2段階の契約）**: ポップアップが閉じられた際は、`elwindui_core::ui::unmount_subtree` による child-first の再帰的 unmount（`on_unmount`・購読解除を含む）が必ず実行される。ただし「ネイティブ detach のどの部分より前か」は、closeが誰の主導で発生したかによって以下の2段階に分かれる（Issue #161 のレビューで判明した、WinUI3 の `Popup` ネイティブ light-dismiss の実際の挙動に基づく訂正）。
    - **全 dismissal 経路に共通する移植可能な保証**: `unmount_subtree` は必ず1回、ElwindUI 側の popup host が content tree を detach/解放する処理（`TreeHost::clear_tree()` 相当）より前、かつ `PopupSurfaceHandle` 自身が content root への強参照を解放する前に実行される。close 完了後は `PopupSurfaceHandle` 自身が popup content root への強参照を保持し続けてはならない——unmount 済みだが解放されない、という状態を作らない（`InnerPopupSurface` の `content` フィールドは `RefCell<Option<Rc<..>>>` とし、close 処理内の `take()` で解放する）。
    - **ElwindUI 主導の close（追加の保証）**: `PopupDismissAction`、項目選択、popup replacement、明示的な `PopupSurfaceHandle::close()`、`Drop` 等、ElwindUI 自身がネイティブ close 要求を発行する経路では、`unmount_subtree` はネイティブの可視性変更・ウィンドウ関係解除などの detach 操作よりも前に実行される（従来通りの teardown-before-detach）。
    - **バックエンドのネイティブ起源の dismiss 通知（例外）**: バックエンドのツールキットが「閉じた後」の通知しか提供しない経路（現時点では WinUI3 の `Popup.Closed` — ネイティブが `Popup.IsOpen` を `false` にした**後**にのみ発火する、Microsoft公式ドキュメントで確認済み）では、ElwindUI が通知を受け取った時点で既にネイティブの可視性変更が完了している場合がある。この経路でも通知を受けたコールバックは直ちに `unmount_subtree` を実行し、それを ElwindUI 側の host detach/解放より前に置くが、`on_unmount` がネイティブ popup をまだ可視状態として観測できることは保証されない。AppKit の light-dismiss は自前の `NSEvent` 監視で検出しclose自体をAppKit側が発行するため、この例外の対象外である（ElwindUI 主導の close と同じ、より強い順序を維持する）。
  - `context_popup: view! { .. }` という宣言的 DSL 構文（open 時に評価される deferred View、`view!` の既存文法をそのまま再利用し、owner Component の bare な識別子参照が自動的に現在値へ解決される）は実装済みである（Issue [#162](https://github.com/puchinya/elwindui/issues/162)、ランタイム/バックエンド基盤自体は [#161](https://github.com/puchinya/elwindui/issues/161)）。macro展開時に隠しComponent/Viewへlowerされ、owner の bare な識別子参照は enclosing Component 自身の scope に対して通常の `view!` 本体と同じ名前解決規則で解決される——詳細な機構は [`../design/runtime/popup_context_menu_design.md`](../design/runtime/popup_context_menu_design.md) の該当節、[`../design/tools/codegen_design.md`](../design/tools/codegen_design.md) §3.35 を参照。低レベル API `ViewTemplate::new(|ctx| ...)` も引き続き利用可能であり、宣言的 DSL はこの API へコンパイルされる。

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
| `transparent` | `bool` | OneWay | `true` ではclient領域のalphaを保持し、未描画・透明pixel越しに背後のwindowを表示する。既定値は`false`。window decorationは変更しない |
| `always_on_top` | `bool` | OneWay | `true` では通常windowより上のplatform topmost/floating levelを使い、`false` では通常levelへ戻す。既定値は`false` |
| `left` | `Option<f32>` | OneTime | 初期X座標 |
| `top` | `Option<f32>` | OneTime | 初期Y座標 |
| `width` | `Option<f32>` | OneTime | 初期幅 |
| `height` | `Option<f32>` | OneTime | 初期高さ |

#### Example

```rust
Window {
    title: "ElwindUI Application"
    transparent: false
    always_on_top: false
    width: 800.0
    height: 600.0
    content: VerticalLayout {
        TextBlock { text: "Hello, ElwindUI!" }
    }
}
```

`transparent` と `always_on_top` は初回 `show()` の前でも表示中でも setter により変更できる。透過はclick-through、frameless化、任意contentからのwindow dragを含まない。

#### Lifecycle (CI-8 of #80, `docs/design/runtime/component_lifecycle_design.md` §4g)

A `#[elwindui::component(inherits Window)]`-declared component ("host composition") is initially not mounted: `let window = MyWindow::new(..);` creates the logical instance and the native window shell but does not evaluate its `view!` body or build its content tree.

- `show()`: on the first call on an unmounted instance, establishes this Window's effective Environment (derived from the Application Environment — `elwindui::core::environment::application_environment()`), performs the initial `view!` build exactly once, then displays the native window. A property set between `new()` and the first `show()` (e.g. `set_title`) is observed by that initial build. Re-showing an already-mounted, hidden window does not remount or rebuild it.
- `hide()`: visibility only. The mounted content tree, Environment subscriptions, and component state all remain alive. A subsequent `show()` makes the window visible again without remounting/rebuilding.
- `close()`: ends the mount lifetime. It first runs the framework unmount path, including `unmount_override`, recursively unmounts the Window content subtree with `unmount_subtree`, then runs the Window component's local `on_unmount`/property-changed/Environment-subscription cleanup, and finally releases/closes the native Window exactly once. Repeated `close()` calls are no-ops.
- **Native close routing (Issue #162)**: a user-initiated native close affordance (the AppKit title-bar close button / `windowShouldClose:`, the WinUI 3 window chrome close button / `AppWindow.Closing`) now enters this *same* `close()` lifecycle rather than a separate native-only teardown path. When the generated owner and its installed close-request handler are both alive and the handler accepts the request, the native attempt is vetoed and the common framework `close()` lifecycle handles it instead, so a native close and a programmatic `close()` call are observably identical. If no handler is installed, or the generated owner is already gone, the native default close is allowed to proceed uncancelled rather than stranding the native window open with nothing left to ever close it. Before its own content unmounts, `unmount_override` additionally closes this Window's own active custom popup/context-menu surface (`docs/design/runtime/popup_context_menu_design.md`'s Owner-`Window`-close-interaction paragraph), so a `context_popup`/`context_menu` left open when the owner Window closes is torn down (its own `unmount_subtree`, including `on_unmount`) rather than orphaned. See `docs/design/runtime/component_lifecycle_design.md` §4i for the `mount_override`/`unmount_override` mechanism this is built on.

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

### `elwindui::ui::IconElement` / `IconSourceElement`

`IconElement` は backend-neutral な自己描画アイコン要素の抽象基底であり、直接構築できない。共通baseが所有するicon propertyは `foreground` だけであり、font-family/font-size/font-style/font-weight等のfont-specific propertyは含まない。これらは将来のderived `FontIcon` の責務である。共有可能な `IconSource` 値を Visual tree に配置する場合は、具象realization leafの `IconSourceElement` を使用する。

#### Properties

| Owner | Name | Type | Binding | Description |
|---|---|---|---|---|
| `IconElement` | `foreground` | `Option<Brush>` | OneWay | monochrome な `SystemIcon` の描画 Brush。未指定時は Visual tree の foreground を継承する |
| `IconSourceElement` | `icon_source` | `Option<IconSource>` | OneWay | 表示する共有可能なアイコン値。`None` はアイコンを描画しない |

- `IconSource::System` の自然サイズは canonical geometry の 16×16 logical unitsであり、arranged boundsには縦横比を維持して収める。有効な local/inherited `foreground` が存在しない場合は、platform色を推測せず描画commandを生成しない。
- `IconSource::Image` の自然サイズは既存 `ImageSource` の intrinsic sizeとし、raster/vectorの既存描画経路を再利用する。`foreground` によるrecolorは行わない。
- `IconSource` は複数箇所で共有できるvalue typeだが、`IconSourceElement` は通常の `UIElement` と同様に一つのVisual parentだけが所有する。

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

`MenuItem` のコレクションを保持するメニュー。`MenuBarItem` のサブメニュー（`submenu`）および `UIElement` のコンテキストメニュー（`context_menu`）の双方で共通のセマンティックモデルとして再利用される。

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
| `icon` | `Option<IconSource>` | OneWay | 任意の backend-neutral メニューアイコン。`MenuBar` submenu と `context_menu` の両方の表示に共通で使われる |
| `shortcut` | `Option<String>` | OneWay | キーボードショートカット記号（例: `"Cmd+S"`） |
| `enabled` | `Option<bool>` | OneWay | 有効状態 |

#### Events

| Name | Type | Description |
|---|---|---|
| `on_select` | `fn()` | メニュー選択時に発火 |

#### `IconSource` / `SystemIcon`

`icon` プロパティの型 `elwindui::core::graphics::IconSource` は、ユーザー定義アイコンとシステム定義アイコンの両方を表す backend-neutral な value type である。値型としての詳細（`ImageSource` との関係、`SystemIcon` variant 一覧）は [Graphics Specification §9 Icons](graphics_spec.md#9-icons) を正とする。本節は `MenuItem.icon` としての normative な公開契約のみを定義する。

```rust
pub enum IconSource {
    System(SystemIcon),
    Image(ImageSource),
}
```

- `IconSource::Image(ImageSource)`: 既存の `ImageSource::Raster(BitmapImage)` / `ImageSource::Vector(VectorImage)` をそのまま利用する。
- `IconSource::System(SystemIcon)`: `SystemIcon` は semantic な `#[non_exhaustive]` enum であり、**backend 固有の識別子（SF Symbol 名、WinUI `Symbol` 名、GTK icon 名等）を一切公開しない**。初期リリースで許可される variant は `Add`, `Remove`, `Delete`, `Edit`, `Copy`, `Cut`, `Paste`, `Undo`, `Redo`, `Search`, `Settings`, `Refresh` の12種のみであり、ElwindUI が対象とする全 backend(AppKit / WinUI 3 / GTK4)間で同一の意味として表現可能な subset に限定される。

#### Native / Custom presentation semantics

`context_menu_presentation`(`UIElement` 共通プロパティ、本書 §2 参照)の値に関わらず、同一の `MenuItem.icon` が表示される。

- `ContextMenuPresentation::Native`(および `MenuBarItem.submenu`): backend のネイティブメニューアイテムの native icon slot(AppKit `NSMenuItem.image` / WinUI 3 `MenuFlyoutItem.Icon`)に、`SystemIcon` は OS/toolkit のシステムアイコン(SF Symbols / `SymbolIcon`)として、ユーザー定義アイコンは native な 16×16 相当サイズの画像として反映される。
- `ContextMenuPresentation::Custom`: Core の `ContextMenuPresenter` が構築する `UIElement` ツリー内に、`IconSourceElement` を使うbackend-neutral な 16×16 DIP の leading icon slot として表示される。`SystemIcon` は ElwindUI 内部の canonical monochrome vector fallback で描画され(OS のネイティブシステムアイコンそのものではない)、ユーザー定義アイコンはそのまま `ImageSource` として描画される。メニュー内の項目が1つでも `icon` を持てば、全ての行に同じ leading slot が予約される(icon を持たない行は空スロットのまま、ラベルの位置は揃う)。メニュー全体に `icon` を持つ項目が1つも無ければ、leading slot 自体を作らず既存のレイアウトを維持する。

#### Failure semantics

アイコンは補助情報であり、失敗が `MenuItem` 自体の失敗に昇格することはない。

- ユーザー定義アイコンのデコード/ラスタライズに失敗した場合: アイコンを表示しないが、`text`/`enabled`/`shortcut`/`on_select` は通常通り機能する。パニックせず、`MenuItem` を削除しない。
- ネイティブ `SystemIcon` ルックアップ(例: SF Symbol)が実行時に失敗した場合: アイコンを省略し、`MenuItem` は正常に残る。ただし、ある `SystemIcon` variant に対して backend 側の mapping 自体が存在しないことは実行時 fallback で隠してはならず、compile-time/design defect として扱う。

`icon` の repeated mutation(`set_icon(Some(a))` → `set_icon(Some(b))` → `set_icon(None)`)は、都度アイコンを差し替え/クリアするのみで、`text`/`enabled`/`shortcut`/`on_select` の状態には影響しない。

`MenuBarItem` 自体には `icon` プロパティを追加しない。

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
- [Graphics Specification](graphics_spec.md) - `Color`, `Brush`, `Path`, `BitmapImage`, `IconSource` などの描画仕様
- [Platform Specification](platform_spec.md) - OSサービス（ファイルダイアログ等）の仕様
