# macOS GUI自動テストドライバ(`tools/macos-ui-driver`)実装状況

ユーザー提供の実装指示書(「elwindui macOS GUI自動テスト実装指示書」、Phase 1〜4)に基づき、AIエージェントがelwindui製macOSアプリを自動で起動・操作・検証できるCLIツールを`tools/macos-ui-driver`に実装していく取り組みの進捗を記録する。`docs/elwindui_nativecontrol_expansion_status.md`と同じ運用方針(マイルストーンごとに更新、完了の誇張をしない)を踏襲する。

---

## 0. スコープに関する方針

- **Phase 1は実装済み**(前セッション)。**Phase 2(Accessibilityツリー取得・要素検索・操作、driver側のみ)も実装・実機検証済み**(本セッション、ユーザー確認済みの指示——`crates/elwindui-backend-appkit`側のAccessibilityIdentifier配線はスコープ外とした)。Phase 3〜4(elwindui内部状態デバッグAPI・画像差分回帰テスト基盤)は未着手——§3に軽量バックログとして記録する。
- **今後のelwindui標準検証手段として位置づける**(ユーザー確認済み)。`docs/elwindui_nativecontrol_expansion_status.md`の§1・§1.8で「対話的な目視確認は権限問題のため未実施」と記録していた箇所は、Phase 1機能(`launch`/`list-windows`/`capture-window`/`terminate`)に加え、Phase 2の`click`/`set-focus`/`type-text`/`press-key`によるクリック・フォーカス・キー入力操作でも代替できることを実機確認した(§2.1)。ただし呼び出し側の注意点として、フォーカス確立とキー送信を跨ぐ操作列はユーザー承認等を挟まず1回のシェルコマンド内で完結させる必要がある(承認操作自体がフォーカスを奪いうるため)——同じく§2.1参照。
- 実装言語はSwift(Swift Package Manager、`swift build`)を選んだ——Rustではなく。理由: (1) Phase 2以降で必須になるAccessibility API(`AXUIElement`)はSwift/Objective-Cとの親和性が非常に高く、Rust側で素のFFIを書くより大幅に安全・簡潔になる、(2) このプロジェクト自身の`CLAUDE.md`/`AGENTS.md`の既存スクリーンショット手順も同じ理由でSwiftスニペットを使っている(先例との一貫性)、(3) 追加の外部依存(swift-argument-parser等)を使わず標準ライブラリのみで実装したため、ネットワークアクセス無しで`swift build`が通る。

---

## 1. Phase 1(実装済み・実機検証済み)

| コマンド | 状態 | 検証内容 |
|---|---|---|
| `doctor` | ✅ | 実機で`{"accessibility":true,"screen_recording":true,"macos_version":"...","success":true}`を確認。`CGPreflightScreenCaptureAccess`/`AXIsProcessTrusted`はどちらもプロンプトを出さない"preflight"版APIを使用(無人実行でも安全) |
| `launch --path <exe> [--arg ..]* [--cwd ..] [--wait-window-timeout <sec>]` | ✅ | `examples/controls-demo`の実バイナリで検証。`--wait-window-timeout`はポーリング(固定sleepではなく`pollUntil`ヘルパーで「対象pidが所有するlayer 0のウィンドウが現れる」を待つ)で動作確認済み |
| `list-windows [--pid <pid>] [--name <substring>]` | ✅ | `--pid`フィルタで対象ウィンドウのみが返ることを確認 |
| `capture-window --window-id <id> --out <path>` | ✅ | `CGWindowListCreateImage`(`.boundsIgnoreFraming`+`.bestResolution`)でウィンドウ単体を正確な境界・Retina解像度でPNG保存できることを実際の画像出力で確認(全画面キャプチャは行わない、`CLAUDE.md`の既存方針を踏襲) |
| `terminate --pid <pid> [--timeout <sec>]` | ✅ | `SIGTERM`→(タイムアウト時のみ)`SIGKILL`のエスカレーション、`kill(pid, 0)`によるポーリングでプロセス終了を確認。実際に対象プロセスが消えることを確認済み。存在しないpidに対しては`already_exited: true`で成功扱い(冪等) |
| エラー系(存在しないwindow-id/pid) | ✅ | `capture-window`は`success: false`+終了コード1、`terminate`は既に無いpidを成功として扱う(冪等)ことを確認 |

**すべてのコマンドの出力は単一行JSON、成功時終了コード0・失敗時1** — 指示書の「すべての結果をJSONで返す」要件を満たす。

**未対応(Phase 1の範囲外)**:
- `wait-for`(汎用条件待機コマンド)は独立コマンドとしては未実装——`launch`の`--wait-window-timeout`という限定形のみ実装済み。汎用的な「要素が出現した/enabledになった/selectedが変わった」等の待機はPhase 2以降、Accessibilityツリー取得と一緒に実装する。
- `collect-logs`は未実装。

**呼び出し側の既知の落とし穴(Phase 2で修正済み)**: `launch`は当初サブプロセスの標準出力をリダイレクトしていなかった(`Process.standardOutput`未設定=親のstdoutをそのまま継承)。そのため`$(...)`のようなコマンド置換でコマンド出力を読もうとすると、起動したGUIアプリ自身がそのパイプの書き込み端を握ったまま動き続けるため、シェル側の読み取りがアプリ終了までブロックされる(macos-ui-driver自体は即座に終了しているにもかかわらず)。**Phase 2の実機検証中に実際にこれで2分間のハングを引き起こした**ため、`process.standardOutput = FileHandle.nullDevice`(`.standardError`も同様)を`cmdLaunch`に追加して恒久修正した。今後の呼び出し側も、コマンド置換ではなく`> /tmp/out.json`のような**ファイルへのリダイレクト**を使うことが引き続き無難(標準出力を捨てるようになったため、コマンド置換自体は安全になったが、他の出力が必要な場合はファイル経由が確実)。

---

## 1.5 `focus-window`(実装済み・実機検証済み、ユーザー指定プロトコル準拠)

ユーザーから「`AXRaise`だけでアプリをフォアグラウンドにできると仮定しないこと」という明示的な指示を受けて実装した、2段階前面化+4項目検証プロトコル:

1. `NSRunningApplication.activate(options:)`でアプリのアクティベーションを要求。
2. 対象ウィンドウ(`--pid`必須、`--title`で複数ウィンドウ中から部分一致選択可)に`AXUIElementPerformAction(kAXRaiseAction)`を実行。
3. `activate()`/`AXRaise`の戻り値は**信用しない**。`pollUntil`(既定`--timeout 3.0`秒)で以下4条件が**すべて同時に**真になるまで実際の状態を確認する: `NSRunningApplication.isActive`、`NSWorkspace.shared.frontmostApplication`が対象pidと一致、対象ウィンドウの`AXMain == true`、対象アプリの`AXFocusedWindow`が`CFEqual`で対象ウィンドウと一致。
4. 成功・失敗いずれの場合も診断情報(`activate_requested_ok`/`ax_raise_status_ok`/`ax_main`/`ax_focused_window_matches_target`/`is_active`/`frontmost_application_name`/`frontmost_application_pid`/`activation_policy`/`macos_version`/`ax_title`)をJSONに含める。

**実機検証結果**(`examples/controls-demo`実バイナリに対して実行、2026-07-24): `activate_requested_ok`/`ax_raise_status_ok`/`ax_main`/`ax_focused_window_matches_target`は**すべて`true`**を返したにもかかわらず、`is_active`は`false`、実際の`frontmost_application`はこのエージェント環境のシェルの親であるSafariのままだった——つまり**AXレベルの個別シグナルはすべて成功を報告するが、実際にはアプリはフォアグラウンドに来ていない**という、ユーザーが指示書で名指しした失敗モードそのものを実機で再現した。本コマンドは戻り値だけで成功と判断せず、4条件の同時成立を要求する設計になっているため、これを`success: false`+全診断情報付きで正しく報告した(誤ってtrueを返す誤検知は起きなかった)。これは「このサンドボックス化されたエージェント環境自体が外部CLIによる前面化奪取を許可しない」という、ユーザーの指示書が想定していた環境制約に該当する——ドライバのバグではなく、report対象の実行環境上の制約として記録する。安定したE2E自動化が必要な場合はXCUITestを優先すべき、という指示書の指針もこの結果と整合する。

`--title`で存在しないタイトルを指定した場合のエラー(`no AX window with title containing "..." (found: [...])`)、存在しないpidを指定した場合のエラー(`no running application with pid ...`)もそれぞれ実機で確認済み。

---

## 2. Phase 2(実装済み・実機検証済み、driver側のみ)

**スコープ**: ユーザー確認済みの方針として、`crates/elwindui-backend-appkit`側でのAccessibilityIdentifier/Label配線(Rust側変更)は今回のスコープ外とした。標準AppKitコントロールは何もしなくても`role`/`title`/`value`をAXツリーへ自動的に公開している(実機確認済み——例えば無地の`NSTextField`が`AXTextField`として、タブボタンが`title`のテキストで、それぞれ識別子配線なしに選択可能)ため、driver側コマンドのみでも機能する。

`press`は独立コマンドにせず`click --via ax-press`へ統合した。ドキュメントの元案にない`set-focus`(`AXUIElementSetAttributeValue(kAXFocusedAttribute)`による直接フォーカス設定)を新規追加した——理由は下記2.1参照。

| コマンド | 状態 | 検証内容 |
|---|---|---|
| `dump-tree --pid <pid> [--window-id ..] [--window-title ..] [--max-depth 40]` | ✅ | `controls-demo`実機で`node_count:23`のツリーを取得。`position`/`size`が`list-windows`の返すウィンドウ境界内に収まることを確認(設計段階の未検証だった座標系の仮定を実地検証——AXの`kAXPositionAttribute`と`CGWindowBounds`は同一のtop-left原点グローバル座標系であることを確認) |
| `find --pid <pid> [...] --role/--title/--title-contains/--identifier` | ✅ | `--title-contains PasswordBox`で1件、`--role AXTextField`で1件、存在しないタイトルで`match_count:0`+`success:true`(存在確認としては正常)をそれぞれ実機確認 |
| `set-focus --pid <pid> [...] <selector>` | ✅ | `AXTextField`に対して`focus_confirmed:true`を確認(下記2.1) |
| `click --pid <pid> [...] <selector> [--via mouse\|ax-press]` | ✅ | `--via ax-press`でタブボタン(`PasswordBox`)のクリックが実際にタブ切り替えを起こすことをスクリーンショットで確認(既存の手動`osascript`テストの回帰確認)。`--via mouse`(実座標への本物の`CGEventPost`)で`AXTextField`のクリック→フォーカス取得も確認(下記2.1) |
| `type-text --pid <pid> [...] <selector> --text <s> [--focus-via ..]` | ✅ | `AXTextField`への`--text "hello"`が`after_value:"hello"`+`value_matches_expected:true`+アプリ自身の「current value: hello」表示・キャレット表示で確認(下記2.1、当初の失敗は誤検知だったことが判明) |
| `press-key --pid <pid> [...] --key <k> [--modifiers ..]` | ✅ | `--key space`でフィールドの値が変化(スペース文字挿入)、`--key tab`で`AXFocusedUIElement`が`AXTextField`→`AXScrollArea`(次のキービューループ要素)へ実際に移動することを確認(下記2.1) |
| `wait-for --pid <pid> [...] --condition <c> [--value ..] [--timeout ..]` | ✅ | `--condition enabled`が0.02秒程度で即座に成功、`--condition value-equals --value "definitely-wrong"`が`timed_out:true`で正しくタイムアウトすることを確認 |
| 異常系(セレクタ0件/複数件) | ✅ | `find`は0件でも`success:true`(存在確認として正常)、`click`は0件で`fail()`(exit 1)、複数件(`×`ボタン4個)で`--index`を促す`fail()`をそれぞれ実機確認。推測での実行は一切発生しなかった |

**未対応(Phase 2の範囲外)**: `crates/elwindui-backend-appkit`側のAccessibilityIdentifier/Label配線(前述のスコープ判断により今回未実施)。

---

## 2.1 決定的テスト結果: TextBoxのクリック→フォーカス→キー入力

直前のセッションで`osascript`/System Events経由の手動テストにより「TextBoxフィールドはクリックしてもフォーカスを取得できない」という事象が見つかっていた(静的表示確認セッション、2026-07-25)。この事象の切り分けが今回のPhase 2実装の主目的の一つだった。

`set-focus`/`click --via mouse`という専用コマンドを使って同じ`controls-demo`実バイナリに対して再テストした結果、まず次が判明した:

1. **`click --pid <pid> --role AXTextField --via mouse`(実座標への本物の`CGEventPost`マウスクリック)は成功してフォーカスを取得した**——`AXFocused`が`false`→`true`に変化(`changed.focused:true`)。アプリ自身のUI(「focus state: Focused (Pointer)」ラベル)でも独立に確認できた。つまり前回の手動テストで観測された「クリックでフォーカスできない」という事象は、**TextBoxの実装側の恒常的なバグではなかった**。
2. **`set-focus`(`AXUIElementSetAttributeValue`による直接フォーカス設定)も同様に成功**(`focus_confirmed:true`)。

一方、`click`(別のツール呼び出し)の直後に別の`type-text`呼び出しを行うと、`focus_confirmed:true`にもかかわらずキーストロークがフィールドに一切反映されない現象が発生した。**この原因はドライバでもアプリでもなく、呼び出し方法にあった**: 各ツール呼び出しの間にユーザーによる操作承認(パーミッションダイアログのクリック)が挟まると、そのクリック操作自体がIDE(VSCode)側へキーボードフォーカスを奪い返してしまい、直後に送信した合成キーストロークが`controls-demo`ではなくIDE側に配信されていた——ユーザー自身がこの原因を指摘し、再検証を依頼した。

**修正した検証方法**(承認待ちの隙間を作らないよう、前面化→クリック→型入力を1回のシェルコマンドにまとめて実行)で再テストした結果:

- `click --via mouse`→`type-text --text "hello" --focus-via none`(間に承認待ちなし)→**`after_value:"hello"`、`value_matches_expected:true`で成功**。スクリーンショットでもフィールド内に"hello"の表示・キャレット・「current value: hello」を確認。
- `press-key --key space`→フィールドの`value`が空文字列からスペース文字を含む値に変化(実際に文字入力として反映)。
- `press-key --key tab`→`AXFocusedUIElement`が`AXTextField`から次のキービューループ要素(`AXScrollArea`)へ実際に移動。

**結論**: TextBox NativeControlのクリック→フォーカス→キー入力は一貫して正常に機能している。当初「キーボードイベントが配信されない」ように見えた事象は、**ツール呼び出しの合間に発生したユーザーの承認操作(パーミッションダイアログのクリック)によるフォーカス横取りが原因**であり、ドライバのバグでも`focus-window`が記録したような環境制約(§1.5)でもなかった。**教訓**: `click`(フォーカス確立)と`type-text`/`press-key`(キー送信)のように、フォーカス状態を跨いで連続する操作は、ユーザー承認や他のツール呼び出しを挟まず1回のシェルコマンド内で完結させること——挟まると、承認UIへの操作自体が対象アプリからフォーカスを奪う可能性がある。

---

## 3. Phase 3〜4 軽量バックログ(詳細設計は未着手)

- **Phase 3**: elwindui内部状態(Visual Tree、layout/render generation、draw count等)をJSONで取得するデバッグ専用API(`#[cfg(debug_assertions)]`限定、本番ビルドで無効化)。`dump_ui_tree`/`inspect`/`wait_for_idle`コマンド。
- **Phase 4**: スクリーンショットの画像差分(許容差・除外領域対応)、回帰テストスイート化、CI/専用Mac実行。
- (スコープ外として保留)`crates/elwindui-backend-appkit`側のAccessibilityIdentifier/Label配線。

---

## 4. 既知の制約

- Windows/GTK4向けの同等ツールは対象外(macOS/AppKit限定)。
- `swift build`で生成される`.build/`はコミット対象外(`tools/macos-ui-driver/.gitignore`)——実行には毎回`swift build`が必要。
