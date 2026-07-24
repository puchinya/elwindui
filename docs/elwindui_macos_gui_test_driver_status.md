# macOS GUI自動テストドライバ(`tools/macos-ui-driver`)実装状況

ユーザー提供の実装指示書(「elwindui macOS GUI自動テスト実装指示書」、Phase 1〜4)に基づき、AIエージェントがelwindui製macOSアプリを自動で起動・操作・検証できるCLIツールを`tools/macos-ui-driver`に実装していく取り組みの進捗を記録する。`docs/elwindui_nativecontrol_expansion_status.md`と同じ運用方針(マイルストーンごとに更新、完了の誇張をしない)を踏襲する。

---

## 0. スコープに関する方針

- **今回のセッションではPhase 1のみ実装した**(ユーザーの明示的な指示)。Phase 2(Accessibilityツリー取得・要素検索・操作)〜Phase 4(画像差分・回帰テスト基盤)は未着手——§2に軽量バックログとして記録する。
- **今後のelwindui標準検証手段として位置づける**(ユーザー確認済み)。`docs/elwindui_nativecontrol_expansion_status.md`の§1・§1.8で「対話的な目視確認は権限問題のため未実施」と記録していた箇所は、このツールのPhase 1機能(`launch`/`list-windows`/`capture-window`/`terminate`)で今後代替できる——ただし実際のクリック・入力操作の自動化はPhase 2(Accessibility要素の`press`/`click`/`type-text`)が必要で、まだ実装されていない。
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

**呼び出し側の既知の落とし穴**: `launch`はサブプロセスの標準出力をリダイレクトしない(`Process.standardOutput`未設定=親のstdoutをそのまま継承)。そのため`"$BIN" launch ... | ...`や``$(...)``のようなパイプ経由でコマンド出力を読もうとすると、起動したGUIアプリ自身がそのパイプの書き込み端を握ったまま動き続けるため、シェル側の読み取りがアプリ終了までブロックされる(macos-ui-driver自体は即座に終了しているにもかかわらず)。呼び出し側は`> /tmp/out.json`のような**ファイルへのリダイレクト**を使うこと(ファイル書き込みは読み手のブロックを起こさない)。ツール側の恒久修正(`process.standardOutput = FileHandle.nullDevice`または専用パイプ+`readabilityHandler`)はPhase 2着手時に検討する軽量バックログ項目とする。

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

## 2. Phase 2〜4 軽量バックログ(詳細設計は未着手)

- **Phase 2**: AppKitバックエンド側でelwindui各コントロールをAccessibilityツリーへ公開する変更(`crates/elwindui-backend-appkit`——`identifier`/`role`/`label`をAXの`accessibilityIdentifier`/`accessibilityLabel`等へマッピング。指示書§5「アクセシビリティ」の既存要件と合流させる)。ドライバ側は`dump-tree`/`find`/`press`/`click`/`type-text`/`press-key`/`wait-for`コマンドを追加。
- **Phase 3**: elwindui内部状態(Visual Tree、layout/render generation、draw count等)をJSONで取得するデバッグ専用API(`#[cfg(debug_assertions)]`限定、本番ビルドで無効化)。`dump_ui_tree`/`inspect`/`wait_for_idle`コマンド。
- **Phase 4**: スクリーンショットの画像差分(許容差・除外領域対応)、回帰テストスイート化、CI/専用Mac実行。

---

## 3. 既知の制約

- Windows/GTK4向けの同等ツールは対象外(macOS/AppKit限定)。
- `swift build`で生成される`.build/`はコミット対象外(`tools/macos-ui-driver/.gitignore`)——実行には毎回`swift build`が必要。
