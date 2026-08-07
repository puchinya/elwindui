# NativeControl派生コントロール拡充 実装状況

AppKit/WinUI3/GTK4のネイティブコントロールを利用した標準UIコントロール群を追加していく取り組みの状況。`docs/status/implementation_status.md`(ワークスペース全体)とは別に、コントロール×バックエンド×要件のチェックリストとして管理する。

**凡例**: ✅ 実装・検証済み / 🟡 実装済みだが未検証 / ⬜ 未実装

---

## 0. スコープに関する方針

- **GTK4は対象外**(見落としではない)。GTK4バックエンド(`crates/elwindui-backend-gtk4`)は19行のスタブのみで、gtk4-rs依存すら無く、AppKit/WinUI3が持つ`native_ui`/`inner`相当の基盤(`AnyView`/`TreeHostView`/`NativeControl`構造)が一切存在しない。この基盤構築は個別コントロールの作業とは独立した大作業であり、§4として別建てで扱う
- **WinUI3側の実装はすべて「AppKitと構造的に一致するようミラーしたが、Windows環境が無いためビルド・実行検証を行っていない」**。既存のTextArea/Button/TabViewのWinUI3実装と同じ扱い
- AppKit側は`cargo build`/`cargo test`/アプリ起動/`tools/macos-ui-driver`による対話操作で検証する

---

## 1. 共通基盤(フォーカス配線ほか)

| 項目 | AppKit | WinUI3 | 備考 |
|---|---|---|---|
| ネイティブフォーカスイン→`FocusTracker`橋渡しの共通関数(`elwindui_core::focus::native_focus_gained`/`native_focus_lost`) | ✅ | ✅ 同じ関数をAppKit/WinUI3双方が呼ぶ(バックエンド非依存のためミラー不要) | `crates/elwindui-core/src/focus.rs`。owner_id解決は`RenderTree::visual_index`を再利用し、専用レジストリを持たない |
| `RenderCommand::NativeControl.owner_id`の保持・生存確認pruning | ✅ `TreeHostIvars::native_owner_ids` | ✅ `NativeChildKey.0`を`owner_id`として利用、追加フィールド不要 | WinUI3はownerとgroup idが1:1対応するため新規フィールドを持たない |
| OSネイティブフォーカスイベントの検知 | ✅ `ElwinduiWindow: NSWindow`サブクラスの`makeFirstResponder:`オーバーライド、`resolve_focus_owner`によるresponderチェーン走査 | 🟡 `FrameworkElement.GotFocus`/`LostFocus`を`reconcile_native_children`の新規アタッチ分岐で1回だけ配線(サブクラス化不要、WinUI3のルーテッドイベントをそのまま利用) | |
| 自前描画要素(`tab_stop: true`)のクリックフォーカス | ✅ `PointerDispatcher::handle`が`&FocusTracker`を受け取り、`Pressed`時に`set_focus` | ⬜ **対象コード自体が存在しない** — WinUI3バックエンドには`PointerDispatcher`の利用箇所が無い(自前描画要素のポインタディスパッチ機構が未実装)。既知のギャップ | `crates/elwindui-core/src/input.rs` |
| Tab/Shift+Tabでネイティブコントロールから抜ける動作 | ⬜ 未対応 | ⬜ 同左 | ネイティブウィジェットの既定キー処理が優先され、elwindui側の`FocusTracker::move_focus`に到達しない。AppKitのkey-view-loopチェーン等、より侵襲的な変更が必要 |

---

## 2. 実装済みコントロール

### 2.1 TextBox

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `elwindui-core::ui::TextBox`トレイト | ✅ | ✅(バックエンド非依存) |
| `#[elwindui_macros::class]`による`TextBox`宣言 | ✅ | ✅(バックエンド非依存、codegenは汎用) |
| 共通`NativeTextFieldCommon`/`NativeTextFieldDelegate`(NSTextField系ウィジェットの値比較ガード付き`set_string_value`・max_length切り詰め・単一デリゲートで`on_change`/`on_submit`両対応) | ✅ | N/A(WinUI3はTextBox/PasswordBoxで別クラス・別イベント名のため共通化の対象が少ない) |
| `InnerTextBox`(`NSTextField`ラップ) | ✅ | 🟡 `XamlTextBox`、`TextArea`と同一クラスを設定違いで共用 |
| `native_ui::TextBox` | ✅ | 🟡 |
| submit-on-Enter(`on_key_down`経由、専用イベントなし) | ✅ `control:textView:doCommandBySelector:`でTextBox専用に対応 | 🟡 `TextBox.KeyDown`がネイティブに発火するため特別な配線不要 |
| コアレベルテスト(`FakeTextBoxWidget`、`FakeNativeControl`継承) | ✅ measure/`try_as_native_control`/`on_change` dispatchを検証 | - |
| `docs/specs/builtins_spec.md` F.12 | ✅ | ✅(同一ドキュメント) |
| `selection_start`/`selection_length` | ⬜ 意図的に見送り | ⬜ 同左 |
| `max_length` | 🟡 デリゲート側で事後的に切り詰め(ネイティブAPI無し) | 🟡 `TextBox.MaxLength`ネイティブ対応 |

### 2.2 PasswordBox

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `elwindui-core::ui::PasswordBox`トレイト | ✅ | ✅(バックエンド非依存) |
| `#[elwindui_macros::class]`による`PasswordBox`宣言(`#[two_way] password`) | ✅ | ✅(バックエンド非依存) |
| `InnerPasswordBox`(`NSSecureTextField`をアップキャストして`NativeTextFieldCommon`を再利用し、TextBoxと同じデリゲート・値比較ガード・max_length切り詰めロジックを重複させない) | ✅ | 🟡 `XamlPasswordBox`、`PasswordBox`は`TextBox`とは別の実XAMLクラス |
| `native_ui::PasswordBox` | ✅ | 🟡 |
| `objc2-app-kit`の`NSSecureTextField`機能 | ✅ | N/A |
| `build.rs`の`PasswordBox`/`PasswordRevealMode` allow-list | N/A | 🟡 型名は実際のWindows環境でのビルドで最終確認が必要 |
| `reveal_enabled` | 🟡 setterは配線するが`true`は意図的にno-op(`NSSecureTextField`にネイティブ相当機能無し) | 🟡 `PasswordRevealMode::Peek`/`Hidden`にネイティブ対応 |
| コアレベルテスト(`FakePasswordBoxWidget`) | ✅ **漏洩防止方針を明示**——テストのアサーションは固定メッセージのみ使用し、パスワード文字列や実際の値を`assert_eq!`のデフォルトpanicメッセージ等で出力しない | - |
| パスワード内容の非露出(`Debug`/`Display`実装なし、ログ出力経路なし) | ✅ | ✅(構造ミラー) |
| `docs/specs/builtins_spec.md` F.13 | ✅ | ✅(同一ドキュメント) |

### 2.3 ScrollView

`ScrollView -> NativeScrollHost -> ElwinduiContentRoot -> content`という3層構造(`docs/design/gui_framework_design.md` §5.1b、`docs/specs/builtins_spec.md` 付録F.14)。TextBox/PasswordBoxと異なり、新規アーキテクチャ機構(`unconstrained_axes`)を必要とする唯一のコントロール。

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `elwindui-core::ui::ScrollView`トレイト(`set_content`/`set_horizontal_scroll_enabled`/`set_vertical_scroll_enabled`) | ✅ | ✅(バックエンド非依存) |
| `#[elwindui_macros::class]`による`ScrollView`宣言(`#[content(content)] content: Rc<dyn UIElement>`) | ✅ | ✅(バックエンド非依存) |
| `TreeHostIvars`/`TreeHostPanel`の`unconstrained_axes`(スクロールする軸のMeasureを無制約にし、`layout_root`後に自然サイズへホスト自身を成長させる) | ✅ `relayout()`の`(false, false)`経路が無変更であることも確認済み | 🟡 `relayout_static`のシグネチャに`unconstrained_axes: (bool, bool)`を追加し、全4呼び出し箇所(`WinUI3RelayoutHost::request_relayout`・`SizeChanged`クロージャ・`force_relayout`・`set_tree`)を更新 |
| `InnerScrollView` | ✅ `NSScrollView`+ネストした`TreeHostView` | 🟡 `ScrollViewer`+ネストした`TreeHostPanel` |
| スクロールしない軸のクロス軸追従 | ✅ `NSAutoresizingMaskOptions`(`ViewWidthSizable`/`ViewHeightSizable`)による自動追従、`NSNotificationCenter`購読は不要 | 🟡 `Canvas`は自動追従しないため`ScrollViewer.SizeChanged`ハンドラで明示的に`Width`/`Height`を同期(`InnerTabView::insert_tab`と同じ対処) |
| `native_ui::ScrollView` | ✅ | 🟡 |
| ネストしたコンテンツ内のネイティブコントロールへのフォーカス | ✅ 追加コード不要(`makeFirstResponder:`のresponderチェーン走査が任意の`TreeHostView`祖先を発見する設計のため) | 🟡 理論上は同様に追加コード不要のはずだが、`GotFocus`/`LostFocus`配線同様に実機確認できていない |
| `build.rs`の`ScrollViewer`/`ScrollMode` allow-list | N/A | 🟡 |
| コアレベルテスト(`FakeScrollViewWidget`、`visual_children()`オーバーライドでcontentの到達可能性を検証) | ✅ | - |
| スクロール位置取得・設定、`scroll_changed`イベント | ⬜ 意図的に見送り | ⬜ 同左 |
| `docs/specs/builtins_spec.md` F.14、`docs/design/gui_framework_design.md` §5.1b | ✅ | ✅(同一ドキュメント) |

### 2.4 Button(`role` / `is_default` / `tooltip`)

**`NativeButton`を新規コントロールとして分離するか要検討**としていた§3のバックログ項目は、**既存`Button`の拡張**として決着した。`role`/`is_default`が必要とするものはすべて同じ`NSButton`/`Button`ウィジェットのプロパティであり、別型を立てる理由が無いため。

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `ButtonRole`(`Normal`/`Primary`/`Destructive`)の定義 | ✅ | ✅(バックエンド非依存) |
| `role` | ✅ どちらの強調roleも`bezelColor`(塗りボタン)で、塗る色だけが違う——`Primary`=`controlAccentColor`、`Destructive`=`systemRedColor`。`hasDestructiveAction`はmacOS 11+のセマンティックsignalとして併設。**`examples/controls-demo`のButtonタブで3つのroleが視覚的に区別できることをスクリーンショットで確認済み** | 🟡 `Primary`=`AccentButtonStyle`。`Destructive`は**ネイティブ相当が無く**`SystemFillColorCriticalBrush`を前景色に設定するのみ |
| `role`の実機検証で却下した2案 | ✅ `contentTintColor`は標準のbordered push buttonのタイトルに効かず`Normal`と区別できなかった。`setAttributedTitle`の赤文字は`apply_text_style`が毎レイアウトパスで`setTitle`を呼んで破棄するため成立しない | N/A |
| `is_default` | ✅ `keyEquivalent = "\r"`。**AXの`AXDefaultButton`属性が該当ボタンを返すことで検証済み** | 🟡 `Button.IsDefault`が存在しない(`ContentDialog`のボタン専用)ため`KeyboardAccelerator`(`VirtualKey::Enter`)で代替 |
| `role`と`is_default`の直交性 | ✅ `Primary`が`keyEquivalent`を触らないのはこのため | 🟡 同左 |
| `hasDestructiveAction`(macOS 11+)のバージョンガード | ✅ `respondsToSelector:`で存在確認。**本クレート初のバージョン分岐**であり以後の前例とする | N/A |
| role別テーマトークン | ⬜ 意図的に追加しない — `button_background`等が既に全role共通の上書き口で、role専用トークンはシステムアクセントカラーと競合する | ⬜ 同左 |
| `icon`/`image` | ⬜ 未対応(`NSButton.image`とWinUI3の`Content`合成で作業の質が異なるため別スコープ) | ⬜ 同左 |
| `docs/specs/builtins_spec.md` F.15 | ✅ 新設(`:41`が付録F.6を指していた誤参照も修正) | ✅(同一ドキュメント) |

#### `tooltip`(`NativeControl`に宣言)

`docs/specs/builtins_spec.md` 付録M.3 が「任意のビルトイン要素が持てる共通属性」と規定しているため、`Button`固有ではなく`NativeControl`に1回だけ宣言した。

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `NativeControl`への`#[prop(tooltip: Option<String>)]`宣言 | ✅ | ✅(バックエンド非依存) |
| 実装箇所 | ✅ `native_ui/control.rs`に1箇所。`AnyView::set_tooltip`→`NSView.toolTip` | 🟡 同構造。`ToolTipService.SetToolTip` |
| 派生する全ネイティブ葉での利用 | ✅ `Button`/`TextArea`/`TextBox`/`PasswordBox`/`ScrollView`/`TabView`。**`Button`と`TextBox`の両方でAXの`AXHelp`属性に設定値が現れることを確認済み**(Button固有ではなく`NativeControl`から継承していることの実証) | 🟡 同左 |
| `background`/`text_style`と違い`measure_override`でのpull-syncをしない | ✅ テーマトークンも遅延解決も無く、レイアウトにも影響しないためsetterでの直接pushが正しい | 🟡 同左 |
| `clear_tooltip` | ⬜ 不要 — codegenが`clear_<name>()`を生成するのは`theme!(..)`値を取り`PlatformDefault`に解決されうるプロパティのみ(`placeholder`に`clear_placeholder`が無いのと同じ) | ⬜ 同左 |
| 自前描画要素(`TextBlock`/`Shape`/レイアウト)の`tooltip` | ⬜ 未実装。ネイティブビューを持たないためホバー判定・表示遅延・ポップアップをelwindui側で実装する必要があり、ネイティブ葉への転送とは作業の質が異なる | ⬜ 同左 |

### 2.5 選択系(`CheckBox` / `RadioButton` / `ToggleSwitch`)

3つとも`docs/specs/builtins_spec.md` F.16/F.17/F.18に対応。AppKitでは`CheckBox`/`RadioButton`が`Button`と同じ`NSButton`をボタン種別違いで使うため、`inner/button.rs`の`ButtonTarget`クリックトランポリンを`pub(crate)`化して直接共有している(`ButtonTarget::attach`ヘルパ)。

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `CheckState`(`Unchecked`/`Checked`/`Indeterminate`)の定義 | ✅ | ✅(バックエンド非依存) |
| `CheckBox`: `Indeterminate`はプログラムから表示可能、ユーザークリックからは到達不可 | ✅ **実機で挙動修正済み**——当初`setAllowsMixedState(false)`で三状態サイクル自体を無効化する設計だったが、それだと`setState(.mixed)`というプログラムからの設定まで`.on`表示に潰れてしまい(`tools/macos-ui-driver`のスクリーンショット比較で発覚)、受け入れ条件「プログラムからは表示される」を満たせなかった。`allowsMixedState(true)`のままにし、`set_on_change`のクリックコールバック側でネイティブが`Mixed`に着地した場合だけ即座に`Checked`へ引き戻す方式へ修正(`inner/check_box.rs`) | 🟡 同じ推論に基づき`SetIsThreeState(true)`+`Indeterminate`イベントでの引き戻しへ構造ミラー(Windows環境が無く未検証) |
| `CheckBox`/`RadioButton`の`ButtonTarget`共有 | ✅ `inner/check_box.rs`・`inner/radio_button.rs`とも`crate::inner::button::ButtonTarget`を直接使用 | 🟡 同型対応 |
| `RadioButton`のグループ管理(elwindui側で論理管理、ネイティブグループ機能に非依存) | ✅ `native_ui/radio_button.rs`のスレッドローカル`GROUPS`(`Weak<dyn UIElementExt>`のレジストリ)。同一グループの他メンバーを明示的に`unchecked`にする | 🟡 同型対応 |
| AppKit自身の「同一superview+同一action」による暗黙のradio自動排他との衝突有無 | ✅ **`examples/controls-demo`のSelectionタブで実機確認済み** — 異なる`group`のRadioButtonが同一コンテナに同居しても互いに干渉しないことを確認 | N/A(未検証) |
| `ToggleSwitch`: `NSSwitch`(`objc2-app-kit` feature `"NSSwitch"`追加) | ✅ | N/A |
| `ToggleSwitch`に`text`プロパティが無いこと | ✅ 仕様どおり(F.18) | 🟡 同左 |
| role別/コントロール別のテーマトークン追加 | ⬜ 意図的に追加しない——`background_token`のdefault armが`native_control_background`にフォールバックし、かつ`NSButton`ファミリー全体で`apply_background`がno-opのため実害が無い(`ffi.rs`の`impl AppKitHandle for Retained<NSButton>`のdocコメント参照) | ⬜ 同左 |
| `objc2-app-kit` feature追加(`NSButtonCell`/`NSCell`/`NSSwitch`) | ✅ `NSButtonType`(`NSButtonCell`)と`NSControlStateValueOn/Off/Mixed`(`NSCell`)は`"NSButton"` featureだけでは届かず、個別追加が必要だった | N/A |
| `crates/elwindui-core/tests/props_macro.rs` | ✅ 3コントロール分のクロスクレート形状テスト追加 | — |
| `docs/specs/builtins_spec.md` F.16/F.17/F.18 | ✅ 新設 | ✅(同一ドキュメント) |

### 2.6 `Dropdown` / `DropdownItem`

`docs/specs/builtins_spec.md` F.5に対応(Phase 3、Issue #35)。バックログの「ComboBox」と実質同一スコープだった仕様書F.5の未実装項目を統合し、`Option`という子コンポーネント名を`DropdownItem`へ改名、選択状態を`Dropdown.selected_index`の`#[two_way]`へ一本化した(詳細はF.5本文参照)。

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `NSPopUpButton`は`NSButton`のサブクラスであるため`ButtonTarget`を再利用 | ✅ `inner/dropdown.rs`が`crate::inner::button::ButtonTarget`を直接使用 | N/A(WinUI3側は`SelectionChangedEventHandler`を使用) |
| `items`変更時のネイティブ側同期 | ✅ 全再構築方式(`removeAllItems`→再`addItemWithTitle`→`selected_index`再適用)。`TabView`/`MenuBar`のような`Rc`同一性差分ではない——`DropdownItem`が自前の編集状態を持たない軽量な値であるため | 🟡 同型対応(`Items().Clear()`→再`Append`) |
| `items`の動的な追加・削除でネイティブ選択状態が保持されること | ✅ **`examples/controls-demo`のDropdownタブで実機確認済み**(`tools/macos-ui-driver`)——4番目の項目をトグルで追加/削除しても、既存の選択(例: "Medium")が再構築後も維持されることを確認 | N/A(未検証) |
| クリックによる`selected_index`変更 | ✅ **実機確認済み**——`NSPopUpButton`のポップアップメニューから項目をAX経由でクリックし、ネイティブ側の値とアプリ側ラベルの両方が追従することを確認 | N/A(未検証) |
| `DropdownItem`はネイティブ実体を持たない(`MenuItem`同様) | ✅ `text: RefCell<String>`のみ保持。`Dropdown`側が各アイテムを`as_any().downcast_ref`して`text()`を読み出し、ネイティブ項目リストを再構築する | 🟡 同型対応 |
| `objc2-app-kit` feature追加(`NSPopUpButton`) | ✅ | N/A |
| `crates/elwindui-core/tests/props_macro.rs` | ✅ `DropdownItem.text`・`Dropdown.selected_index`(two-way)・`Dropdown.enabled`のクロスクレート形状テスト追加 | — |
| `docs/specs/builtins_spec.md` F.5 | ✅ `Dropdown`/`DropdownItem`実装内容へ更新(旧`Dropdown`/`Option`案から改訂) | ✅(同一ドキュメント) |

**`elwindui-codegen`の一般バグを発見・修正**: `#[content(field_name)]`で宣言したリスト型content field(`Vec<..>`/`ListExt<..>`)の中身を`if`/`for`で動的に変える(`Dropdown`の`items`に`if vm.dropdown_extra_item { DropdownItem { .. } }`のような分岐を書く)と、コード生成側の動的子要素リフレッシュ処理(`codegen.rs`の`dynamic_region_refresh_method`)が実際の`#[content(..)]`名を見ず常に`.children()`という決め打ちのメソッド呼び出しを生成していた。`TabView`/`VerticalLayout`/`HorizontalLayout`/`Grid`はいずれも実際のcontent fieldが`children`という名前なので偶然動いていたが、フィールド名が`items`の`Dropdown`(および同じく`items`の`Menu`)で初めて顕在化した——`Menu`はこれまで静的な子要素しか使われておらず、この動的リフレッシュ経路自体が未通過だった。

修正は2箇所: (1) 同一クレート内のユーザーコンポーネント向けには、`codegen.rs`側がすでに持っている`TypeInfo.content_field`をこの箇所でも読むようにした(スカラー分岐は元々読んでいたのに、リスト分岐だけ読んでいなかった)。(2) クロスクレートのビルトイン(`Dropdown`/`Menu`など、ローカルな`TypeInfo`を持たない)向けには、`elwindui-macros`の`build_props_macro`(`class.rs`)に新しい shape-macro クエリ `@content_field_get $recv:expr` を追加した——`@content_item_dyn`と同じ1ホップ方式(このクラス自身の`#[content(..)]`宣言がそのまま答えになる。型解決が要らないぶん`@content_item_dyn`より単純)。`codegen.rs`はローカル情報が無い場合、この新クエリ経由で`elwindui::core::#props_macro!(@content_field_get self.#parent_binding)`を生成し、実際にコンパイルされる時点で正しいゲッター呼び出しへ展開されるようにした。ワークスペース全体のテスト(457件)に回帰なし。

### 2.7 `examples/controls-demo`

`examples/graphics-demo`と同じ構造(単一`main.rs`、`#[elwindui::viewmodel]`、`TabView`+タブごとの機能領域)。

| タブ | 内容 |
|---|---|
| TextBox | 値・placeholder・focus状態表示・event log・submit-on-Enter |
| PasswordBox | 値の長さのみ表示、実際の値は一切表示しない(漏洩防止方針をデモ自身が実演) |
| ScrollView | ビューポートより高いコンテンツ。ネストした`TextBox`でネスト内フォーカスを確認できる |
| Button | 3つのrole・is_default・tooltip |
| Selection | CheckBox(三状態のプログラム設定含む)・同一グループのRadioButton3つ・ToggleSwitch |
| Dropdown | 3項目の`Dropdown`・`selected_index`の双方向バインディング・ボタンによる4番目の項目の動的追加削除 |
| 回帰確認 | 既存`TextArea`/`Button` |

対話的な動作確認(クリック・入力・フォーカス切り替え・スクロール)は`tools/macos-ui-driver`で行う(`docs/status/macos_ui_driver_status.md`)。

**`#[computed]`初期化式の書き方に関する注意**: Selectionタブの`check_box_checked_label`等は当初`self.check_box_checked.get()`のように内部ストレージへ直接アクセスする書き方だったが、これは`elwindui-codegen`の依存関係抽出(`codegen.rs`の`referenced_fields`)が裸の1セグメントパス(`check_box_checked`のような素のフィールド名)しか検出しない設計のため、依存先の観測プロパティが変化しても`recompute_<name>`が呼ばれず、ラベルが初期値のまま固まって更新されないというバグを引き起こした(`tools/macos-ui-driver`での実クリック検証で発覚)。同じ理由で`format!(...)`のようなマクロ呼び出しの引数に埋め込んだフィールド参照も(`t!`マクロ以外は)不可視——`rewrite_field_refs`/`referenced_fields`はマクロの生トークン列の中までは踏み込まない。正しい書き方は`examples/notepad`と同じ「裸のフィールド名」糖衣構文(例: `check_box_checked`単体、または`.get()`無しでメソッドを呼ぶ`toggle_is_on.to_string()`)を使い、`format!`でラップしたい場合は`match`/`if`式で先に`String`へ変換してから渡す。**同型の既存バグが`password_box_length`(`#[computed(expr = format!("{}", self.password_box_value.borrow().chars().count()))]`)にも残っている**——本Issueのスコープ外(Selectionタブ新設より前から存在)のため未修正のまま残置。

**AppKit実機能ライフサイクルテストが未着手である理由**: `MainThreadMarker::new()`は`cargo test`のデフォルトテストハーネス(ワーカースレッド)で`None`を返す。`harness = false`のカスタムテストバイナリが必要だが、`inner`/`native_ui`モジュールの型が`pub(crate)`のため外部`tests/`統合テストからアクセスできず、設計に追加検討が必要。現状は`examples/controls-demo`+`macos-ui-driver`による確認で代替している。

---

## 3. 未実装コントロールのバックログ(詳細設計は未着手)

(**NativeButton**は§2.4で既存`Button`の拡張として決着済み。**CheckBox/RadioButton/ToggleSwitch**は§2.5で実装済み。**ComboBox**は§2.6で仕様書の`Dropdown`(付録F.5)と統合の上実装済み。いずれもバックログから除去した。)

- **Slider** — AppKit: `NSSlider` / WinUI3: `Slider`
- **ProgressBar** — AppKit: `NSProgressIndicator` / WinUI3: `ProgressBar`。indeterminate状態はネイティブアニメーションを使い、elwindui側でフレーム生成しない
- **NumberBox** — AppKit: `NSTextField`+`NSStepper`合成 / WinUI3: `NumberBox`(ネイティブ一体型)。入力中文字列と確定値を区別する設計が必要
- **その他** — ContextMenu / Popup / ToolTip / SearchBox / DatePicker / TimePicker / ColorPicker / ListView / TreeView / WebView / DataGrid

---

## 4. GTK4基盤構築(独立タスク)

**あらゆる**GTK4版NativeControlの前提条件:

- `gtk4-rs`のワークスペース依存追加
- `crates/elwindui-backend-gtk4/src/`に`native_ui`/`inner`をゼロから設計・構築。AppKit/WinUI3の`AnyView`/`TreeHost*`/`NativeControl`構造(生存確認によるアタッチ/デタッチ diff、Measure/Arrange委譲、フォーカス橋渡し)をミラーする
- 個別コントロールの実装ではなく、GTK4対応全体の土台として独立に見積もり・スケジュールする

---

## 5. 既知の制約

- **WinUI3**: Windows環境が無いため`cargo build`/`cargo test`/実行のいずれも不可能。すべての変更は構造レビューのみ
- **GTK4**: 未着手
- **`Option<String>`プロパティは`&str`しか受け付けない**: `wrap_prop_value`(`crates/elwindui-macros/src/class.rs`)が`&(..)`を挿入するのは**裸の`String`**プロパティ(`Button::text`)だけで、`Option<String>`は`is_string_type`の判定を外れて値が素通しになる。そのためsetterが`&str`を取る`Option<String>`プロパティ——`NativeControl::tooltip`、`TextBox::placeholder`、`PasswordBox::placeholder`——にはDSLの文字列リテラルしか渡せず、`bind!`した`String`のビューモデルフィールドは型エラーになる。`crates/elwindui-core/tests/props_macro.rs`の`props_macro_forwards_tooltip_up_to_native_control`がこの挙動を固定している
- **`view!`の`Option<T>`自動ラップ**: `Option<T>`型のプロパティ(`max_length: Option<u32>`等)に裸のリテラル値(`40`や`40u32`)を書いても`Some(..)`への自動ラップが効かず型エラーになる。`vm.some_field`のような変数参照(`bind!`経由)は`Option<bool>`等で自動ラップされる(`Button.enabled: vm.save_can_execute`、`examples/notepad`)ため、自動ラップの対象は変数参照のみでリテラル値には適用されない。`Some(40u32)`のような関数呼び出し形の式もDSLパーサーが受け付けない(識別子を期待するパースエラー)。`examples/controls-demo`では`TextBox`/`PasswordBox`の`max_length`指定を省略して回避している
