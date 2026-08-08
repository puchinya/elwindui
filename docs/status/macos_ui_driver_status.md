# macOS GUI自動テストドライバ(`tools/macos-ui-driver`)実装状況

AIエージェントがelwindui製macOSアプリを自動で起動・操作・検証できるCLIツール。elwinduiの標準的なGUI検証手段として位置づける。

実装言語はSwift(Swift Package Manager、`swift build`)。理由: (1) Accessibility API(`AXUIElement`)はSwift/Objective-Cとの親和性が高く、Rust側で素のFFIを書くより安全・簡潔、(2) `AGENTS.md`/`CLAUDE.md`の既存スクリーンショット手順も同じ理由でSwiftスニペットを使う、(3) 外部依存(swift-argument-parser等)を使わず標準ライブラリのみで実装しているため、ネットワークアクセス無しで`swift build`が通る。

**すべてのコマンドの出力は単一行JSON、成功時終了コード0・失敗時1。**

---

## 1. プロセス・ウィンドウ操作(実装済み・実機検証済み)

| コマンド | 内容 |
|---|---|
| `doctor` | Accessibility / Screen Recording権限の状態を返す。`CGPreflightScreenCaptureAccess`/`AXIsProcessTrusted`というプロンプトを出さない"preflight"版APIを使うため無人実行でも安全 |
| `launch --path <exe> [--arg ..]* [--cwd ..] [--wait-window-timeout <sec>]` | `--wait-window-timeout`は固定sleepではなく`pollUntil`ヘルパーで「対象pidが所有するlayer 0のウィンドウが現れる」を待つ |
| `list-windows [--pid <pid>] [--name <substring>]` | pid/名前でフィルタしたウィンドウ一覧 |
| `capture-window --window-id <id> --out <path>` | `CGWindowListCreateImage`(`.boundsIgnoreFraming`+`.bestResolution`)でウィンドウ単体を正確な境界・Retina解像度でPNG保存する。全画面キャプチャは行わない |
| `terminate --pid <pid> [--timeout <sec>]` | `SIGTERM`→(タイムアウト時のみ)`SIGKILL`のエスカレーション、`kill(pid, 0)`によるポーリングで終了を確認。存在しないpidには`already_exited: true`で成功扱い(冪等) |

`capture-window`は存在しないwindow-idに対して`success: false`+終了コード1を返す。

`launch`はサブプロセスの標準出力・標準エラーを`FileHandle.nullDevice`へ捨てる。これをしないと、`$(...)`のようなコマンド置換で出力を読もうとしたとき、起動したGUIアプリがパイプの書き込み端を握ったまま動き続けるため、シェル側の読み取りがアプリ終了までブロックされる。呼び出し側は出力が必要な場合、コマンド置換ではなく`> /tmp/out.json`のようなファイルへのリダイレクトを使うのが確実。

---

## 2. `focus-window`(実装済み・実機検証済み)

`AXRaise`だけでアプリをフォアグラウンドにできると仮定してはならない。本コマンドは2段階前面化+4項目検証プロトコルを実装する:

1. `NSRunningApplication.activate(options:)`でアプリのアクティベーションを要求
2. 対象ウィンドウ(`--pid`必須、`--title`で複数ウィンドウ中から部分一致選択可)に`AXUIElementPerformAction(kAXRaiseAction)`を実行
3. `activate()`/`AXRaise`の戻り値は**信用しない**。`pollUntil`(既定`--timeout 3.0`秒)で以下4条件が**すべて同時に**真になるまで実際の状態を確認する: `NSRunningApplication.isActive`、`NSWorkspace.shared.frontmostApplication`が対象pidと一致、対象ウィンドウの`AXMain == true`、対象アプリの`AXFocusedWindow`が`CFEqual`で対象ウィンドウと一致
4. 成功・失敗いずれの場合も診断情報(`activate_requested_ok`/`ax_raise_status_ok`/`ax_main`/`ax_focused_window_matches_target`/`is_active`/`frontmost_application_name`/`frontmost_application_pid`/`activation_policy`/`macos_version`/`ax_title`)をJSONに含める

**環境制約**: サンドボックス化されたエージェント環境では、`activate_requested_ok`/`ax_raise_status_ok`/`ax_main`/`ax_focused_window_matches_target`がすべて`true`を返すにもかかわらず`is_active`が`false`のまま、実際には前面化していないことがある。AXレベルの個別シグナルはすべて成功を報告するが実際にはフォアグラウンドに来ていない、という失敗モードである。本コマンドは4条件の同時成立を要求するため、これを`success: false`+全診断情報付きで正しく報告する(誤検知は起きない)。これはドライバのバグではなく実行環境の制約であり、安定したE2E自動化が必要な場合はXCUITestを優先する。

---

## 3. Accessibilityツリー操作(実装済み・実機検証済み)

標準AppKitコントロールは何もしなくても`role`/`title`/`value`をAXツリーへ自動的に公開するため、`crates/elwindui-backend-appkit`側でAccessibilityIdentifier/Label配線を行わなくてもdriver側コマンドのみで機能する。

`press`は独立コマンドではなく`click --via ax-press`へ統合されている。

| コマンド | 内容 |
|---|---|
| `dump-tree --pid <pid> [--window-id ..] [--window-title ..] [--max-depth 40]` | AXツリーをJSONで取得する。`position`/`size`は`list-windows`が返すウィンドウ境界と同一のtop-left原点グローバル座標系(AXの`kAXPositionAttribute`と`CGWindowBounds`は同じ座標系) |
| `find --pid <pid> [...] --role/--title/--title-contains/--identifier` | セレクタに一致する要素を返す。0件でも`success:true`(存在確認として正常) |
| `set-focus --pid <pid> [...] <selector>` | `AXUIElementSetAttributeValue(kAXFocusedAttribute)`による直接フォーカス設定 |
| `click --pid <pid> [...] <selector> [--via mouse\|ax-press\|ax-increment\|ax-decrement] [--fraction 0.0..1.0]` | `--via mouse`は実座標への本物の`CGEventPost`(既定で要素中央。`--fraction`(既定`0.5`)で要素の左端からの相対位置を指定でき、`AXSlider`のような「クリック位置がそのまま値になる」コントロールをドラッグ無しで任意値へ動かせる)。`--via ax-press`は`AXPress`アクション。`--via ax-increment`/`ax-decrement`は`kAXIncrementAction`/`kAXDecrementAction`——`AXSlider`は`AXPress`に対応しない(`ax_press_status_ok: false`)ため、キーボード相当の刻み幅操作にはこちらを使う |
| `type-text --pid <pid> [...] <selector> --text <s> [--focus-via ..]` | 文字列入力。`after_value`/`value_matches_expected`を返す |
| `press-key --pid <pid> [...] --key <k> [--modifiers ..]` | 単一キー送信。修飾キーは`cmd`/`ctrl`/`alt`/`shift` |
| `wait-for --pid <pid> [...] --condition <c> [--value ..] [--timeout ..]` | 条件は`exists`/`not-exists`/`enabled`/`focused`/`value-equals`/`ax-attribute` |

`click`はセレクタが0件のとき、また複数件のとき(`--index`を促す)にいずれも`fail()`(exit 1)する——推測での実行は行わない。

**`AXValue`の数値/真偽値誤判定バグを修正済み**: `axJSONValue`が`raw as? Bool`を`raw as? NSNumber`より先に試していたため、`NSNumber`のBoolへの寛容なブリッジング(`(0 as NSNumber) as? Bool`は`false`として成功する)により、値がちょうど`0`または`1`になった数値系`AXValue`(`AXSlider`の`value`、`AXCheckBox`の`value`等)が誤って真偽値として報告されていた(`Slider`(#37)の実機検証中に発覚)。`CFGetTypeID`で`CFBooleanGetTypeID()`/`CFNumberGetTypeID()`を明示的に判定するよう修正——`AXCheckBox.value`が(たまたま`false`/`true`と一致していたのではなく)実際には`0`/`1`/`2`(`NSControlStateValueOff/On/Mixed`)という数値であったことも、この修正で正しく可視化されるようになった。

### 3.1 呼び出し側の必須の注意点

**`click`/`set-focus`(フォーカス確立)と`type-text`/`press-key`(キー送信)は、1回のシェルコマンド内で完結させること。** 間にユーザー承認や別のツール呼び出しを挟むと、承認UIへの操作自体が対象アプリからキーボードフォーカスを奪い、合成キーストロークが対象アプリではなく承認UI側へ配信される。`focus_confirmed:true`が返っていてもキー入力が反映されない、という形で現れる。

この注意点を守った上での`controls-demo`実機検証では、`click --via mouse`→`type-text`→`after_value`一致、`press-key --key space`による文字挿入、`press-key --key tab`による`AXFocusedUIElement`の次要素への移動がいずれも正常に動作する。

---

## 4. 未実装

- **elwindui内部状態のデバッグAPI**: Visual Tree、layout/render generation、draw count等をJSONで取得する`#[cfg(debug_assertions)]`限定API(本番ビルドで無効化)と、それを読む`dump_ui_tree`/`inspect`/`wait_for_idle`コマンド。詳細設計は未着手
- **画像差分回帰テスト基盤**: スクリーンショットの画像差分(許容差・除外領域対応)、回帰テストスイート化、CI/専用Mac実行。詳細設計は未着手
- `collect-logs`コマンド
- `crates/elwindui-backend-appkit`側のAccessibilityIdentifier/Label配線

---

## 5. 既知の制約

- Windows/GTK4向けの同等ツールは対象外(macOS/AppKit限定)
- `swift build`で生成される`.build/`はコミット対象外(`tools/macos-ui-driver/.gitignore`)——実行には毎回`swift build`が必要
