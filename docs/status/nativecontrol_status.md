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

### 2.4 `examples/controls-demo`

`examples/graphics-demo`と同じ構造(単一`main.rs`、`#[elwindui::viewmodel]`、`TabView`+タブごとの機能領域)。

| タブ | 内容 |
|---|---|
| TextBox | 値・placeholder・focus状態表示・event log・submit-on-Enter |
| PasswordBox | 値の長さのみ表示、実際の値は一切表示しない(漏洩防止方針をデモ自身が実演) |
| ScrollView | ビューポートより高いコンテンツ。ネストした`TextBox`でネスト内フォーカスを確認できる |
| 回帰確認 | 既存`TextArea`/`Button` |

対話的な動作確認(クリック・入力・フォーカス切り替え・スクロール)は`tools/macos-ui-driver`で行う(`docs/status/macos_ui_driver_status.md`)。

**AppKit実機能ライフサイクルテストが未着手である理由**: `MainThreadMarker::new()`は`cargo test`のデフォルトテストハーネス(ワーカースレッド)で`None`を返す。`harness = false`のカスタムテストバイナリが必要だが、`inner`/`native_ui`モジュールの型が`pub(crate)`のため外部`tests/`統合テストからアクセスできず、設計に追加検討が必要。現状は`examples/controls-demo`+`macos-ui-driver`による確認で代替している。

---

## 3. 未実装コントロールのバックログ(詳細設計は未着手)

- **NativeButton** — 既存`Button`(既にNativeControl派生として実装済み)の拡張として扱うか、role(`normal`/`primary`/`destructive`)等を持つ新規コントロールとして分離するか要検討。AppKit: `NSButton` / WinUI3: `Button`
- **ComboBox** — 編集不可の選択コントロール。AppKit: `NSPopUpButton` / WinUI3: `ComboBox`。仕様書の`Dropdown`(付録F.5、未実装)との名称・スコープ重複を実装時に整理する必要がある
- **CheckBox** — AppKit: `NSButton`(`NSButtonType.Switch`) / WinUI3: `CheckBox`。三状態(`CheckState::Indeterminate`)はユーザー操作からは遷移不可にする
- **RadioButton** — AppKit: `NSButton`(`NSButtonType.Radio`) / WinUI3: `RadioButton`。グループ管理はネイティブのグループ機能に依存せず、elwindui側で論理管理する
- **Slider** — AppKit: `NSSlider` / WinUI3: `Slider`
- **ToggleSwitch** — AppKit: `NSSwitch`(10.15+)またはカスタム合成 / WinUI3: `ToggleSwitch`
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
- **`view!`の`Option<T>`自動ラップ**: `Option<T>`型のプロパティ(`max_length: Option<u32>`等)に裸のリテラル値(`40`や`40u32`)を書いても`Some(..)`への自動ラップが効かず型エラーになる。`vm.some_field`のような変数参照(`bind!`経由)は`Option<bool>`等で自動ラップされる(`Button.enabled: vm.save_can_execute`、`examples/notepad`)ため、自動ラップの対象は変数参照のみでリテラル値には適用されない。`Some(40u32)`のような関数呼び出し形の式もDSLパーサーが受け付けない(識別子を期待するパースエラー)。`examples/controls-demo`では`TextBox`/`PasswordBox`の`max_length`指定を省略して回避している
