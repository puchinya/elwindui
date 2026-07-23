# NativeControl派生コントロール拡充 実装状況ドキュメント

このドキュメントは、elwinduiにAppKit/WinUI3/GTK4のネイティブコントロールを利用した標準UIコントロール群(TextBox・PasswordBox・ScrollView・NativeButton・ComboBox・CheckBox・RadioButton・Slider・ToggleSwitch・ProgressBar・NumberBox・ContextMenu・Popup・ToolTip・MenuBar・SearchBox・DatePicker・TimePicker・ColorPicker・ListView・TreeView・WebView・DataGrid)を追加していく、複数セッションにまたがる大規模な取り組みの進捗を記録する。`docs/elwindui_implementation_status.md`(ワークスペース全体の実装状況)とは別に、このNativeControl拡充効果に特化して、コントロール×バックエンド×要件のチェックリストとして管理する。マイルストーンごとに更新すること(最後にまとめて更新しない)。

計画の全文は元の実装指示書(このドキュメントには転記しない)およびPhase 1の詳細設計セッションのやり取りを参照。ここでは「今何が実際に終わっているか」だけを追う。

---

## 0. スコープに関する重要な方針

- **GTK4はPhase 1のスコープから意図的に除外している**(見落としではない)。GTK4バックエンド(`crates/elwindui-backend-gtk4`)は現状19行のスタブのみで、gtk4-rs依存すら未追加、AppKit/WinUI3が持つ`native_ui.rs`/`inner.rs`相当の基盤(`AnyView`/`TreeHostView`/`NativeControl`構造)が一切存在しない。この基盤をゼロから構築するのは独立した大作業であり、TextBox/PasswordBox/ScrollView固有の作業に含めず、§4「GTK4基盤構築フォローアップ」として別建てで扱う。
- **この結果、Phase 1は指示書自身が定める完了条件(「AppKit・WinUI3・GTK4の3バックエンドが完了するまで、そのPhaseを完了扱いにしない」)を満たさない。** これは意図的な縮小スコープであり、完了の主張ではない。
- **WinUI3側の変更はすべて「AppKitと構造的に一致するようミラー実装したが、Windows環境がないためビルド・実行検証は一切行っていない」**。既存のTextArea/Button/TabViewのWinUI3実装と同じ扱い。
- AppKit側は実際に`cargo build`/`cargo test`/アプリ起動で検証する。

---

## 1. 共通基盤(§1 フォーカス配線ほか)

| 項目 | AppKit | WinUI3 | 備考 |
|---|---|---|---|
| ネイティブフォーカスイン→`FocusTracker`橋渡しの共通関数(`elwindui_core::focus::native_focus_gained`/`native_focus_lost`) | ✅ 実装・`cargo test -p elwindui-core`通過 | ✅ 同じ関数をAppKit/WinUI3双方が呼ぶ(バックエンド非依存のためミラー不要) | `crates/elwindui-core/src/focus.rs`。owner_id解決は`RenderTree::visual_index`を再利用、新規レジストリなし |
| `RenderCommand::NativeControl.owner_id`の保持・生存確認pruning | ✅ `TreeHostIvars::native_owner_ids`実装・`cargo test -p elwindui-backend-appkit`通過 | ✅ `NativeChildKey.0`(既存の仕組み、そのまま`owner_id`として利用可能なことを確認済み)を利用、追加フィールド不要 | AppKit: `crates/elwindui-backend-appkit/src/inner.rs`。WinUI3は元々ownerとgroup idが1:1対応していたため新規フィールド不要だった |
| OSネイティブフォーカスイベントの検知 | ✅ `ElwinduiWindow: NSWindow`サブクラスの`makeFirstResponder:`オーバーライド、`resolve_focus_owner`によるresponderチェーン走査 | 🟡 未検証・`FrameworkElement.GotFocus`/`LostFocus`を`reconcile_native_children`の新規アタッチ分岐で1回だけ配線(サブクラス化不要、WinUI3のルーテッドイベントをそのまま利用) | AppKit: `InnerWindow::new()`を`ElwinduiWindow`ベースに切り替え済み |
| 自前描画要素(`tab_stop: true`)のクリックフォーカス | ✅ `PointerDispatcher::handle`に`&FocusTracker`引数追加、`Pressed`時に`set_focus` | ⬜ **対象コード自体が存在しない** — WinUI3バックエンドには`PointerDispatcher`の利用箇所が現状ゼロ(自前描画要素のポインタディスパッチ機構が未実装)。ミラー先がないため対応不要、既知のギャップとして記録のみ | `crates/elwindui-core/src/input.rs`。呼び出し元シグネチャ変更に伴い`ui.rs`内のテスト16箇所も更新済み |
| 既存TextArea/TabView/Buttonの回帰確認(AppKit) | ✅ `cargo build`/`cargo test -p elwindui-core -p elwindui-backend-appkit`(174件)通過。`rust-analyzer diagnostics .`で新規warning/error無し。`notepad`を2回起動し数秒間クラッシュなしを確認、CoreGraphics window list上に正常なウィンドウ生成を確認 | - | 🟡 **クリック操作・TextArea入力・TabView切り替えなどの対話的動作の目視確認は未実施** — このマシンの実行環境に画面収録・アクセシビリティ権限が付与されておらず、`screencapture`/`osascript`によるスクリーンショット・自動クリックがいずれも失敗した。ユーザーによる手動確認待ち |
| Tab/Shift+Tabでネイティブコントロールから抜ける動作 | ⬜ 未対応(Phase 1スコープ外、既知の制限として記録) | ⬜ 同左 | ネイティブウィジェットの既定キー処理が優先されるため、elwindui側の`FocusTracker::move_focus`に到達しない。AppKitのkey-view-loopチェーン等、より侵襲的な変更が必要 |

**未完了(このドキュメント作成時点で未着手)**: §2 TextArea/TabViewの対話的回帰確認(権限待ち)、§5 ScrollView、§6 ドキュメント追加の残り(`elwindui_gui_framework_design.md`新§5.1b)、§7 `examples/controls-demo`。

---

## 1.5 TextBox(§3.0 共通化 + §3 実装 完了)

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `elwindui-core::ui::TextBox`トレイト | ✅ | ✅(バックエンド非依存) |
| `builtins.elwind`の`TextBox`宣言 | ✅ | ✅(バックエンド非依存、codegenは完全に汎用) |
| §3.0a 共通`NativeTextFieldCommon`/`NativeTextFieldDelegate`(NSTextField系ウィジェットの値比較ガード付きset_string_value・max_length切り詰め・単一デリゲートでon_change/on_submit両対応) | ✅ 実装・`cargo build`/`cargo test`通過 | N/A(WinUI3はTextBox/PasswordBoxで別クラス・別イベント名のため共通化の対象が少なく、新規共通化コードは追加していない) |
| `InnerTextBox`(`NSTextField`ラップ) | ✅ | 🟡 未検証(`XamlTextBox`、`TextArea`と同一クラスを設定違いで共用) |
| `native_ui::TextBox` | ✅ | 🟡 未検証 |
| submit-on-Enter(`on_key_down`経由、専用イベントなし) | ✅ `control:textView:doCommandBySelector:`でTextBox専用に対応 | 🟡 未検証(`TextBox.KeyDown`はネイティブに発火するため特別な配線不要) |
| コアレベルテスト(`FakeTextBoxWidget`、`FakeNativeControl`継承) | ✅ `cargo test -p elwindui-core`通過(measure/try_as_native_control/on_change dispatchを検証) | - |
| AppKit実機能ライフサイクルテスト(§3.0c/§3e) | ⬜ **未着手** — `MainThreadMarker::new()`が`cargo test`のデフォルトテストハーネス(ワーカースレッド)では`None`を返すことを実機で確認済み(空の`#[test]`で検証)。`harness = false`のカスタムテストバイナリが必要だが、`inner`/`native_ui`モジュールの型が`pub(crate)`のため外部`tests/`統合テストからはアクセスできず、設計に追加検討が必要。デモアプリ(`examples/controls-demo`、§7)による手動確認で代替する方針 | - |
| `docs/elwindui_builtins_spec.md` F.12 | ✅ | ✅(同一ドキュメント) |
| `selection_start`/`selection_length` | ⬜ 意図的に見送り(既知のギャップとして明記) | ⬜ 同左 |
| max_length非対称性 | 🟡 デリゲート側で事後的に切り詰め(ネイティブAPI無し) | ✅ `TextBox.MaxLength`ネイティブ対応(未検証) |

---

## 1.6 PasswordBox(§4 完了)

| 項目 | AppKit | WinUI3 |
|---|---|---|
| `elwindui-core::ui::PasswordBox`トレイト | ✅ | ✅(バックエンド非依存) |
| `builtins.elwind`の`PasswordBox`宣言(`#[two_way] password`) | ✅ | ✅(バックエンド非依存) |
| `InnerPasswordBox`(`NSSecureTextField`をアップキャストして`NativeTextFieldCommon`を再利用、TextBoxと同じデリゲート・値比較ガード・max_length切り詰めロジックを重複実装せず) | ✅ `cargo build`/`cargo test`通過 | 🟡 未検証(`XamlPasswordBox`、`PasswordBox`は`TextBox`とは別の実XAMLクラス) |
| `native_ui::PasswordBox` | ✅ | 🟡 未検証 |
| `objc2-app-kit`の`NSSecureTextField`機能追加 | ✅ | N/A |
| `build.rs`の`PasswordBox`/`PasswordRevealMode`allow-list追加 | N/A | 🟡 未検証(型名は実際のWindows環境でのビルドで最終確認が必要) |
| `reveal_enabled` | 🟡 setterは配線するが`true`は意図的にno-op(`NSSecureTextField`にネイティブ相当機能無し、コメント付き) | ✅ `PasswordRevealMode::Peek`/`Hidden`にネイティブ対応(未検証) |
| コアレベルテスト(`FakePasswordBoxWidget`) | ✅ `cargo test -p elwindui-core`通過。**漏洩防止方針を明示**——テストのアサーションは固定メッセージのみ使用し、パスワード文字列や実際の値を`assert_eq!`のデフォルトpanicメッセージ等で出力しない | - |
| パスワード内容の非露出(`Debug`/`Display`実装なし、ログ出力経路なし) | ✅ | ✅(構造ミラー) |
| AppKit実機能ライフサイクルテスト | ⬜ TextBoxと同じ理由で未着手(§1.5参照) | - |
| `docs/elwindui_builtins_spec.md` F.13 | ✅ | ✅(同一ドキュメント) |

---

## 2. Phase 2-4 軽量バックログ(詳細設計は未着手)

Phase 1完了後、個別に詳細計画する。現時点ではコントロール名とバックエンド対応部品の見立てのみ記録する。

- **NativeButton** — 既存`Button`(builtins.elwind上は`Button`という名前で既にNativeControl派生として実装済み)の拡張として扱うか、role(`normal`/`primary`/`destructive`)等を持つ新規コントロールとして分離するか要検討。AppKit: `NSButton` / WinUI3: `Button`。
- **ComboBox** — 編集不可の選択コントロールとして新規実装。AppKit: `NSPopUpButton` / WinUI3: `ComboBox`。既存spec上の`Dropdown`(付録F.5、未実装)との名称・スコープ重複を実装時に整理する必要あり。
- **CheckBox** — AppKit: `NSButton`(`NSButtonType.Switch`) / WinUI3: `CheckBox`。三状態(`CheckState::Indeterminate`)はユーザー操作からは遷移不可にする。
- **RadioButton** — AppKit: `NSButton`(`NSButtonType.Radio`) / WinUI3: `RadioButton`。グループ管理はネイティブのグループ機能に依存せず、elwindui側で論理管理する。
- **Slider** — AppKit: `NSSlider` / WinUI3: `Slider`。
- **ToggleSwitch** — AppKit: `NSSwitch`(10.15+)またはカスタム合成 / WinUI3: `ToggleSwitch`。
- **ProgressBar** — AppKit: `NSProgressIndicator` / WinUI3: `ProgressBar`。indeterminate状態はネイティブアニメーションを使用し、elwindui側でフレーム生成しない。
- **NumberBox** — AppKit: `NSTextField`+`NSStepper`合成 / WinUI3: `NumberBox`(ネイティブ一体型)。入力中文字列と確定値を区別する設計が必要。

Phase 3(ContextMenu/Popup/ToolTip/MenuBar/SearchBox/DatePicker/TimePicker/ColorPicker)・Phase 4(ListView/TreeView/WebView/DataGrid)は指示書記載の順序・対応部品表をそのまま参照し、このドキュメントでは繰り返さない。

---

## 3. GTK4基盤構築フォローアップ(独立タスク、Phase 1には含まない)

TextBox/PasswordBox/ScrollViewを含む**あらゆる**GTK4版NativeControlの前提条件:

- `gtk4-rs`のワークスペース依存追加。
- `crates/elwindui-backend-gtk4/src/native_ui.rs`+`inner.rs`をゼロから設計・構築。AppKit/WinUI3の`AnyView`/`TreeHost*`/`NativeControl`構造(生存確認によるアタッチ/デタッチ diff、Measure/Arrange委譲、フォーカス橋渡し)をミラーする。
- 個別コントロール(TextBox等)の実装ではなく、GTK4対応全体の土台として独立に見積もり・スケジュールする。

---

## 4. 検証環境の既知の制約

- WinUI3: Windows環境が無いため、`cargo build`/`cargo test`/実行のいずれも不可能。すべての変更は目視での構造レビューのみ。
- GTK4: 未着手。
- AppKit: `cargo build`/`cargo test`は実行可能。実機でのアプリ起動も可能だが、**この実行環境には画面収録・アクセシビリティ権限が付与されていない**ため、`screencapture`によるスクリーンショット取得・`osascript`/System Eventsによる自動クリック操作ができない。対話的な動作確認(クリック・入力・タブ移動等)はユーザーによる手動確認、またはこれらの権限が付与された環境でのみ可能。
