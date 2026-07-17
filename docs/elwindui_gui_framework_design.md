# elwindui GUIフレームワーク設計書

本ドキュメントは**GUIフレームワーク本体**(バックエンド抽象化・`elwindui-core`ランタイム・ライフサイクル・Store/ViewModel/MVVM・UI機能拡張)の設計における正(authoritative source)である。「誰が何を実装するのか」「各機能はどの静的検証ルールで守られているか」「各層はどう連携するか」という設計上の関心に沿って構成している。ElwindUIL DSLの構文・文法そのもの(`component`/`view`/`param`/`prop`定義、制御構文、値制約、enum、i18n等)は`docs/elwindui_dsl_spec.md`が正であり、本ドキュメントの§2はその要点のみを設計文脈の中で引用する。

## 本ドキュメントのスコープ

**対象(本ドキュメント)**: バックエンド抽象化、`elwindui-core`ランタイム(`UIElement`クラス階層・レイアウト・フォーカス・アクセシビリティ・描画)、標準ビルトイン部品、ライフサイクル、Store/ViewModel/MVVM/非同期/Undo-Redoなどの状態管理層、キーボード・ナビゲーション・テーマ・エラーハンドリング・モバイル対応等のUI機能拡張。

**対象外(他の設計書を参照)**: ElwindUIL DSLの構文・静的検証ルール自体は`docs/elwindui_dsl_spec.md`、`builtin::`要素の個別リファレンス実装は`docs/elwindui_builtins_spec.md`、`.elwind`→Rustのコード生成コンパイラ(`elwindui-codegen`)・LSP(`elwindui-languageserver`)・エディタ内プレビュー・ホットリロード機構は`docs/elwindui_tool_*_design.md`を参照すること。

---

## 目次

1. [全体アーキテクチャ概観](#1-全体アーキテクチャ概観)
2. [言語コアモデル](#2-言語コアモデル)
3. [バックエンド抽象化](#3-バックエンド抽象化)
4. [標準ビルトイン部品](#4-標準ビルトイン部品)
5. [コアランタイム(elwindui-core)](#5-コアランタイムelwindui-core)
6. [ライフサイクル](#6-ライフサイクル)
7. [状態管理とMVVM](#7-状態管理とmvvm)
8. [UI機能拡張ビルトイン](#8-ui機能拡張ビルトイン)
9. [テスト支援](#9-テスト支援)
10. [静的検証ルール一覧(14章)と機能対応表](#10-静的検証ルール一覧14章と機能対応表)
11. [責務分担まとめ:コンパイラ/コード生成器 vs ランタイムライブラリ](#11-責務分担まとめコンパイラコード生成器-vs-ランタイムライブラリ)

---

## 1. 全体アーキテクチャ概観

ElwindUILは特定のGUIフレームワークに依存しない中間表現として設計されている。

```
.elwind ファイル(ElwindUIL構文)
        │  コンパイル(ツール設計書側の責務)
        ▼
共通AST(フレームワーク非依存の要素ツリー)
        │  バックエンド別コード生成
        ▼
┌────────────────────────────────────────────┐
│ ElwindUIL Core Runtime(elwindui-core)         │  ← 本ドキュメントの主対象
│  Element / LayoutEngine / FocusManager /      │
│  AccessibilityTree / InputRouter / Painter    │
└────────────────────────────────────────────┘
        │
        ▼
┌──────────┬──────────┬──────────┐
│ WinUI3   │ AppKit   │ GTK4     │  …Uikit/Jetpack(§8.8)
│ backend  │ backend  │ backend  │
└──────────┴──────────┴──────────┘
```

クレート構成(§5.9):

```
elwindui-core           # Element, LayoutEngine, FocusManager, AccessibilityTree, InputRouter, Painter(共通・バックエンド非依存)
elwindui-backend-winui3 # elwindui-coreを実装 + windows-rsでネイティブAPIに橋渡し
elwindui-backend-appkit # 同上、objc2経由
elwindui-backend-gtk4   # 同上、gtk-rs経由
```

`.elwind`コンパイラが生成するコードは常に`elwindui-core`のトレイト境界に対して書かれ、実行時にどのバックエンドクレートがリンクされるかで実体が決まる。バックエンド指定は`#![backend(...)]`(ビルド設定)と`target::backend()`(式内定数、§3.3)の2つの窓口を持つ設計だが、いずれも現時点では未実装(実際のバックエンド選択は`elwindui`ファサードクレートのCargoフィーチャ`backend-appkit`/`backend-winui3`/`backend-gtk4`のみで行われる。詳細は§3.3、`docs/elwindui_implementation_status.md`)。

---

## 2. 言語コアモデル

### 2.1 `component` と `view` の分離

`component`(状態定義)と`view`(描画ロジック)を分離する。Rustの`struct`/`impl`に対応する。

| | `component` | `view` |
|---|---|---|
| 役割 | 状態(フィールド)の定義 | 状態→見た目の写像 |
| 書く内容 | 型・制約・初期値のみ | `if`/`for`/`match`による要素ツリー組み立て |
| 変更頻度 | 低い | 高い |

インスタンス化はRustの`let`束縛をそのまま使い、専用構文は導入しない(`Card { title, value }`のようなフィールド名ショートハンドのみ許可)。

### 2.2 `param` と `prop`(実体化時固定 vs 実行時可変)

フレームワーク全体の不変条件の中で最も重要なのがこの区別である。

| | `#[param]` | 既定(`prop`) |
|---|---|---|
| 変更可能性 | 実体化時のみ、以後イミュータブル | 実行時いつでも変更可 |
| 使える式 | リテラル・他paramの参照・純粋関数・`env::*`・`once`値のみ | 上記に加え`bind!`・propの参照・`#[computed]` |
| 主な用途 | 構造分岐(`if`/`for`の条件)、レイアウト決定 | 表示内容・状態の動的更新 |

`#[computed]`は依存する他フィールドの変化に応じ自動再評価される読み取り専用の算出値であり、外部からの代入は許されない。

**この区別はフレームワーク全体で一貫して守られる**: ライフサイクルフック内でも(§6.1)、Store/ViewModelからの参照でも(§7.1, §7.2)、モバイルのデバイス情報からも(§8.8)、`#[param]`は「実体化時固定」という性質を失わない。これは14章ルール1・2・11・13・21(§10参照)により静的に強制される。

### 2.3 制御構文

Rust標準の`if`/`for`/`match`をそのまま採用し、専用ディレクティブは設けない。`match`は対象がenumの場合、全メンバー網羅で`_ =>`を省略できる。**網羅されていない場合はコンパイルエラー**となる(これは`Backend`・`Route`(§8.2)・`AsyncState`(§7.3)など、フレームワーク全体の多くのenumで繰り返し利用される中核機構)。

### 2.4 `style`(横断的属性適用)

```rust
style {
    select(Text) { font_family: "Noto Sans" }
    select(Button, variant == "danger") { color: "#e74c3c" }
}
```

`select(要素型, 条件式)`で対象を絞り込み属性をマージ適用する。インライン属性がスタイル定義より優先(後勝ち・詳細優先)。`theme`のトークン参照(§8.5)や`target::backend()`による条件分岐(§3.3)もこのセレクタ条件式内で使える。

### 2.5 値制約(アトリビュートによる数式的表現)

| 記法 | 意味 |
|---|---|
| `#[range(0..=1)]` | 閉区間 |
| `#[range(0..100)]` | 半開区間 |
| `#[step(5)]` | 刻み幅(multipleOf相当、`#[range]`と併用) |
| `#[length(3..=16)]` | 文字列長の範囲 |
| `#[pattern(r"^[a-z]+$")]` | 正規表現 |
| `#[format(email)]` | 組込み検証型(email, url, color_hex等) |
| `#[check(expr, message = "...")]` | 相関検証(数式化できない場合) |

検証タイミング: リテラル値による制約違反はビルド時静的エラー、`bind!`等の動的値による違反は実行時エラー。

### 2.6 `enum`

値候補があるフィールドは常に名前付き`enum`として定義する(匿名共用体は採用しない)。

- 参照は`EnumName::Member`の完全修飾のみ(裸文字列は静的エラー)
- `EnumName::values()`で全メンバー列挙(`for`と組み合わせ選択UIを自動生成可能)
- `#[label(...)]`で多言語表示名を付与、`member.label()`で現在ロケールの文字列取得
- `match`との組み合わせで網羅性検査が働く(§2.3)

### 2.7 動的定数(`env` / `once`)

「実体化時に一度だけ確定し以後不変」な値を`#[param]`の静的評価式の例外として参照可能にする。

- `env::os()` / `env::platform()` / `env::locale()` / `env::direction()` — 組み込み
- `once NAME: T = external::foo()` — ユーザー拡張。`external::*`呼び出しはトップレベルの`once`宣言でのみ許可され、動的性の入口を一箇所に集約する

`target::backend()`(§3.3)はこれとは確定タイミングが異なる**コンパイル時定数**であり、`env::*`より強い静的性を持つ。

### 2.8 データバインディング

```rust
volume: i32 = bind!(settings.volume, TwoWay),
```

`bind!(path, mode)`のmodeは`OneWay`(既定、外部→propの一方向反映)/`TwoWay`(UI操作で外部にも書き戻す)/`OneTime`(実体化時に一度だけ取り込み以後固定)。参照先は`store`(§7.1)・`viewmodel`(§7.2)・ビルトインStore(§8.8)のフィールドパスであり、いずれも`#[param]`から直接参照することはできない(14章ルール12・13)。

### 2.9 多言語対応(i18n)

翻訳は業界標準の**Fluent(.ftl)**をそのまま採用し、DSL側は`t!`マクロでメッセージIDを参照するだけに留める。複数形・性別分岐・日付/数値フォーマットはFluent自身の構文(`select`式、`NUMBER()`/`DATETIME()`)に委譲する。

- ビルド時に`.ftl`を静的パースし、`t!("key", ...)`が全`available`言語で定義されているかを機械的に検証(未翻訳キー検出)
- `t!`の引数名は対応する`.ftl`メッセージ内の`{ $引数名 }`と一致しなければ静的エラー
- RTL対応のため`padding_start`/`padding_end`等の論理方向プロパティを使う

### 2.10 モジュール(import)

Rustの`use`構文と完全に一致させる(`use components::card::Card as ProductCard;`等)。静的にimportを解決し、循環参照・未解決参照を機械的に検出する。

### 2.11 要素ツリーの探索(`Element`トレイト)

「子要素を持つ」性質は既存の`{}`ネスト構文が表現するため、children専用の新DSL構文は追加しない。代わりにコード生成器が全要素型に共通のトレイトを自動実装する。

```rust
trait Element {
    fn children(&self) -> Vec<&dyn Element>;
    fn id(&self) -> Option<&str> { None }
}
```

| 責務 | 担当 |
|---|---|
| 親子構造の宣言(`{}`ネスト) | DSL構文(追加構文なし) |
| 動的生成された子要素(if/for/matchの結果)の集約規約 | コード生成器 |
| `children()`/`id()`実装 | コード生成器が自動実装 |
| 再帰探索アルゴリズム(`find_by_id`, `find_all`) | 共通ランタイムライブラリ(`elwindui-core`) |
| 特定要素への後からのアクセス | `#[id(...)]`アトリビュート |

探索方式(深さ優先/幅優先)やキャッシュ戦略の変更は、DSL構文を変えずライブラリ側の実装更新だけで完結する。DSL側が保証するのは「`Element`トレイトを介してツリー全体に到達可能」という契約のみ。

---

## 3. バックエンド抽象化

### 3.1 全体像

.elwindは論理的な要素ツリーを記述するのみで、各OSネイティブツールキットへの変換は「バックエンド」が担う(§1参照)。制約検証・enum網羅性検査・i18n解決などの言語機能はすべてバックエンド非依存のフロントエンド解析段階(ツール側の責務)で完結し、バックエンド選択に影響されない。

### 3.2 OSネイティブツールキットへの抽象化

Windows→**WinUI 3**(windows-rs経由)、macOS→**AppKit**(objc2経由)、Linux→**GTK4**という、OS標準ツールキットへコンパイル時に振り分ける。OS判定は実行時の`env::os()`(実体化時に一度だけ確定する動的定数)とは別物で、**ビルドターゲット(target triple)によりコンパイル時に確定する**分岐である点に注意する。

```rust
#![backend(native)]   // ビルドターゲットに応じてOS標準ツールキットへ自動的に振り分ける
```

明示固定したい場合はRustの`cfg`属性の慣習に沿う:

```rust
#[cfg(target_os = "windows")]
#![backend(winui3)]

#[cfg(target_os = "macos")]
#![backend(appkit)]

#[cfg(target_os = "linux")]
#![backend(gtk4)]
```

**論理要素 ⇔ 各ネイティブAPIのマッピング例:**

| ElwindUIL論理要素 | WinUI 3 backend | AppKit backend | GTK4 backend |
|---|---|---|---|
| `Window { title, ... }` | `Microsoft::UI::Xaml::Window` | `NSWindow` | `gtk::ApplicationWindow` |
| `Button { text, on_click }` | `Microsoft::UI::Xaml::Controls::Button` | `NSButton` | `gtk::Button` |
| `TextArea { text }` | `Microsoft::UI::Xaml::Controls::TextBox`(`AcceptsReturn: true`) | `NSTextView` | `gtk::TextView` |
| `Column { ... }` | `Microsoft::UI::Xaml::Controls::StackPanel`(`Orientation: Vertical`) | `NSStackView(orientation: .vertical)` | `gtk::Box(orientation: Vertical)` |
| `Dropdown { ... }` | `Microsoft::UI::Xaml::Controls::ComboBox` | `NSPopUpButton` | `gtk::DropDown` |

DSL記述者はこれらの違いを一切意識せず、`Button { text: t!("save"), on_click: save_document() }`と書くだけでよい(実際の各ビルトインのリファレンス実装は`docs/elwindui_builtins_spec.md`付録Fを参照)。

OSごとの見た目差はスタイル層に閉じ込める:

```rust
style {
    select(Button) {
        // 既定はOS標準の見た目に委ね、何も書かない
    }
    select(Button, backend == Backend::Winui3) { corner_radius: 4 }
    select(Button, backend == Backend::Appkit) { corner_radius: 6 }
}
```

`backend == Backend::Winui3`のような条件はビルドターゲットで確定するコンパイル時定数として扱われ、該当しない分岐はコード生成対象から静的に除外される(デッドコード除去と同様)。

プラットフォーム固有機能へのエスケープハッチは`native!`ブロック(§2.11外の特殊構文。フレームワーク固有API、例:AppKitの`NSVisualEffectView`を直接埋め込む)を`#[cfg(backend = "...")]`と組み合わせて使う:

```rust
view NotepadWindow {
    Column {
        TextArea { text: content }

        #[cfg(backend = "winui3")]
        native! {
            // WinUI 3固有: Mica素材背景を有効化
            self.window.SystemBackdrop(MicaBackdrop::new());
        }

        #[cfg(backend = "appkit")]
        native! {
            // AppKit固有: ウィンドウにVisual Effect Viewを追加
            self.window.contentView().addSubview(&vibrancy_view);
        }
    }
}
```

`#[cfg(backend = "...")]`が付いたブロックは対象外のビルドではコード生成・型チェックの対象から除外され、`native! { ... }`自体もリンタが移植性のない箇所として検出できるよう明示される。

`prop`変更の反映方式はいずれのバックエンドでも「対応ネイティブAPIのプロパティ更新呼び出し」(保持モード)になる。

- `prop`変更 → 対応するネイティブAPIのプロパティ更新呼び出し(例:WinUI 3なら`button.SetContent(new_text)`、AppKitなら`button.setTitle(new_text)`)
- `#[computed]`の再評価 → 依存する`prop`の変化に応じて該当ウィジェットのプロパティ更新コードが生成される
- `children()`の構成変化(`for`ループの要素数増減等) → コンテナへの`addChild`/`removeChild`相当のAPI呼び出しに変換される(差分検出はコード生成器の責務)

`Element`トレイト(`children()`/`id()`)や`param`/`prop`の意味自体はバックエンドを問わず共通である。全バックエンドが保持モード(要素が生成後も明示的な更新まで存在し続ける)であるため、`prop`の変更はコード生成器がネイティブAPIのプロパティ更新呼び出しへと変換する。

| 項目 | 担当 |
|---|---|
| `.elwind`の記述 | 常に1つ、プラットフォーム分岐は原則書かない |
| どのOSでどのツールキットを使うか | `#![backend(native)]` またはビルドターゲット別の明示指定(`winui3`/`appkit`/`gtk4`) |
| 論理要素→具体API変換 | 各バックエンドクレート(`elwindui-backend-winui3`, `elwindui-backend-appkit`, `elwindui-backend-gtk4`) |
| OSごとの見た目差 | `style { select(..., backend == ...) }` |
| OS固有機能の直接利用 | `#[cfg(backend = "...")]` + `native!` |
| プロパティ変更の反映方式 | バックエンドが保持モードAPIへの更新呼び出しとして生成、DSL側の`param`/`prop`定義は不変 |

### 3.3 `target::backend()`(コンパイル時静的定数)

```rust
enum Backend {
    Winui3, Appkit, Gtk4,
    Uikit,      // iOS(§8.8)
    Jetpack,    // Android(§8.8)
}
```

**実装状況**: `Backend` enumと`target::backend()`はいずれも現時点で未実装(`crates/`配下のRustソースに実体が存在しない)。実際のバックエンド選択は`elwindui`ファサードクレートのCargoフィーチャ(`backend-appkit`/`backend-winui3`/`backend-gtk4`)による`#[cfg(feature = ...)]`のみで行われており、本節が説明する「コンパイル時定数+`match`網羅性検査」の仕組みはフォワードルッキングな設計である。

`target::backend()`はビルドターゲットからビルド時に一意に確定する定数関数で、`#[param]`の静的評価式に無条件で使用できる(`env::os()`より確定タイミングが早い)。これにより、抽象化されたコンポーネント定義を1つの`.elwind`ファイル内で完結できる:

```rust
component NotepadWindow {
    #[param]
    chrome_style: ChromeStyle = match target::backend() {
        Backend::Winui3 => ChromeStyle::Mica,
        Backend::Appkit => ChromeStyle::Vibrancy,
        _               => ChromeStyle::Flat,
    },
}
```

`match target::backend() { ... }`は`Backend`の全メンバー網羅を要求される(§2.3の網羅性検査と同じ仕組み)。**新しいバックエンド(`Uikit`/`Jetpack`等)を追加すると、既存の全ビルトインリファレンス実装が非網羅エラーになる** — これは仕様の欠陥ではなく、「新バックエンド追加時にどのビルトインが未対応かを機械的に洗い出す」安全弁として意図された挙動である。ビルトインのリファレンス実装(§4)はデスクトップ系backendの説明を目的として`Backend::Uikit | Backend::Jetpack`腕を省略するため、モバイル対応時は§8.8の指針に沿って各ビルトインに対応腕を追加する。

`env::os()`(実体化時に一度だけ確定・以後不変、§2.7)と`target::backend()`は確定タイミングが異なる:

| 定数 | 確定タイミング | `#[param]`初期化式での使用 |
|---|---|---|
| `env::os()` 等 | 実体化時に一度だけ | 許可(§2.2・§2.7の例外規定) |
| `target::backend()` | コンパイル時(ビルド構成から確定) | 常に許可 |

`style`セレクタの条件式でも同様に使える:

```rust
style {
    select(Button, target::backend() == Backend::Winui3) { corner_radius: 4 }
    select(Button, target::backend() == Backend::Appkit) { corner_radius: 6 }
}
```

コード生成時、`target::backend()`はコード生成器がビルド設定から得た値へ定数畳み込みし、該当しない分岐(他backend向けの`native!`ブロック等)は生成対象から静的に除去される。実行バイナリには不要な分岐コードが一切残らない:

```rust
// elwindui_codegen 内部(擬似)
const fn resolve_backend() -> Backend {
    #[cfg(feature = "backend-winui3")] { Backend::Winui3 }
    #[cfg(feature = "backend-appkit")] { Backend::Appkit }
    #[cfg(feature = "backend-gtk4")]   { Backend::Gtk4 }
}
```

`#![backend(...)]`(§3.2)とは役割が異なるため併存する:

| 概念 | 役割 | 確定タイミング |
|---|---|---|
| `#![backend(native)]` / `#![backend(winui3)]`(§3.2) | どのコード生成器(crate)を使うかというビルド設定 | ビルド構成時 |
| `target::backend()`(本節) | その結果を`.elwind`の式中から参照するための静的定数 | コンパイル時(式に畳み込み) |

前者はプロジェクト全体・ファイル単位のビルド設定、後者はコンポーネント定義内部の条件分岐に使う窓口である。まとめ:

| 要件 | 対応 |
|---|---|
| 抽象化コンポーネント定義を1ファイルで完結させる | `target::backend()`という式内定数による分岐(ファイル外属性への依存を排除) |
| フレームワーク指定を静的定数として扱う | `Backend` enum + `target::backend()`(ビルド時確定、`#[param]`に無条件使用可) |
| 構造分岐・スタイル分岐の両方に対応 | `match`/`if`/`style select`いずれの条件にも使用可能 |
| 該当しないbackendのコードを含めない | コンパイル時の定数畳み込みにより非該当分岐を静的除去 |

### 3.4 名前空間とビルトインのオーバーライド規則

ビルトインは予約名前空間`builtin::*`に属し(`Row { ... }`は`builtin::Row`への暗黙の`use`が常に効いている扱い)、ユーザー定義コンポーネントが同名の場合は`#[overrides(builtin::X)]`を明示しない限り静的エラーになる(暗黙のシャドーイングは一切許可しない)。この名前解決規則自体はDSLの静的検証ルールの一部であり、`docs/elwindui_dsl_spec.md`付録Aが正とする。§4.1で述べる「バックエンド分岐を書けるのは`builtin`定義と`#[overrides(builtin::X)]`が付いたコンポーネントだけ」という制限は、この節で定義される名前解決の上に成り立つ。

---

## 4. 標準ビルトイン部品

`Window`/`Column`/`Row`/`Text`/`TextArea`/`Dropdown`等は`builtin`名前空間に属し、コード生成器が標準実装として提供する。内部実装は他コンポーネントと同じ`component`/`view`構文で表現でき、`match target::backend()`(§3.3)による網羅性検査と`native!`エスケープハッチがそのまま適用される設計だが、§3.3の通り`target::backend()`自体は未実装のため、実際の`crates/elwindui-codegen/src/builtins.elwind`はバックエンド分岐を持たず、バックエンドごとの実体はCargoフィーチャで選択される別クレート(`elwindui-backend-appkit`等)側に委ねられている。

**実装状況**: `builtins.elwind`に実装済みなのは`Window`/`VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`Control`/`ContentControl`/`Grid`/`TextArea`/`Button`/`TextBlock`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabView`/`TabViewItem`(`Row`/`Column`という名称ではなく`HorizontalLayout`/`VerticalLayout`という名称で実装されている点に注意)。`Dropdown`/`Option`、`Canvas`、`NavigationHost`/`Route`、`Dialog`、`Tooltip`、`VirtualList`は仕様のみで未実装。詳細は`docs/elwindui_builtins_spec.md`冒頭の分類ツリーと`docs/elwindui_implementation_status.md`を参照。

**代表的な実装パターン(`Stack` → `Column`/`Row`)**: 共通の`Stack`部品に`orientation`を渡して処理を委譲し、`Column`/`Row`はその薄いラッパーとして定義する。

```rust
component Stack { #[param] orientation: Orientation, #[param] spacing: number = 0, children: Vec<Element> }
component Column { children: Vec<Element> }
view Column { Stack { orientation: Orientation::Vertical, children } }
```

### 4.1 独自部品はバックエンド共通実装に限定する(最重要ルール)

**バックエンド分岐(`native!`/`match target::backend()`)を書けるのは`builtin`定義と`#[overrides(builtin::X)]`が付いたコンポーネントだけ**(14章ルール9)。通常の独自部品は常にビルトイン要素の組み合わせ、または`Canvas`+`Painter`(§5.4)のみで実装する。

| コンポーネント種別 | バックエンド分岐の可否 |
|---|---|
| `builtin::*` | 可 |
| `#[overrides(builtin::X)]` | 可 |
| 通常の独自部品 | 不可。常にバックエンド共通実装のみ |

判断フロー: 「`native!`が必要だと感じたら」→ 既存ビルトインの代替実装なら`#[overrides(builtin::X)]`として定義し直す → それも違うなら`Canvas`+`Painter`で表現できないか再検討する → それでも無理な場合のみ新規ビルトイン追加を提案する。

このルールはダイアログ・メニュー(§8.3)、ナビゲーション(§8.2)等、他の全ビルトイン層にも同じ原則(14章ルール9・14・15)として繰り返し適用される。

---

## 5. コアランタイム(elwindui-core)

Button/Textのような個別ウィジェット抽象化(§4)とは別レイヤーとして、WinUI 3の`Composition`/`UIAutomation`/`Measure-Arrange`に相当する共通基盤を`elwindui-core`として定義し、各バックエンドがこれを実装する。

```
.elwind (component/view)
        │
        ▼
UIElement ツリー(§2.11、§5.1)
        │
        ▼
┌─────────────────────────────────────────┐
│ ElwindUIL Core Runtime(elwindui-core)      │
│  ├─ LayoutEngine      (制約ベースのMeasure/Arrange) │
│  ├─ FocusManager      (フォーカス移動・トラップ)     │
│  ├─ AccessibilityTree (UIAツリー相当)              │
│  ├─ InputRouter       (ヒットテスト・イベント配送)   │
│  └─ Painter           (§5.7参照)                  │
└─────────────────────────────────────────┘
        │
        ▼
各バックエンド実装(WinUI3/AppKit/GTK4)
```

各バックエンドはOS標準機構(WinUI3の`UIAutomation`、AppKitの`NSAccessibility`、GTK4の`Atk`/AT-SPI等)に極力委譲し、Core Runtimeはレイアウト・フォーカス・ヒットテストなどバックエンド非依存の共通計算を担う。

### 5.1 `UIElement`階層(WinUI3方式)

要素ツリー(Visualツリー、§5.2)は、WinUI3が実際に`UIElement`派生クラスの木として要素ツリーを表現しているのに倣い、`Rc<dyn UIElement>`というトレイトオブジェクトの木そのものとして表現される(別途「ツリー型」というラッパーは存在しない)。子から親への逆参照(`parent()`)を持つため`Box`ではなく`Rc`で所有する。

#### 5.1a Rustコードでのクラス階層表現規約

elwindui本体(コード生成・手書きランタイム双方)でRustに"クラス"階層を実装する際は、以下の規約に従う(Rustには実装継承がないため、trait(振る舞いの契約)と構造体合成(データの委譲)を組み合わせて疑似的に表現する)。**実装を伴う正の仕様は`docs/elwindui_macro_class_spec.md`(`#[elwindui_macros::class]`マクロの完全な仕様)であり、以下は要点の要約。命名の詳細に食い違いがあれば同書と`crates/elwindui-macros/src/class.rs`/`crates/elwindui-core/src/ui.rs`を優先すること。**

- コンポーネント(クラス)名を`Class`とすると:
  - **構造体名**: 常にソースに書いたとおりの素の識別子`Class`(接尾辞なし)。先頭のフィールドとして`base`という名前で親クラスの構造体を保持する:
    ```rust
    struct Class {
        base: SuperClass,
        // Class自身が宣言したフィールドはこの後に続く
    }
    ```
  - **トレイト名**: `{Class}Ext`(Rustは同一モジュール内で構造体とトレイトが同じ裸名を共有できないため、接尾辞はトレイト側に付く)。親クラスが`SuperClass`なら`trait ClassExt: SuperClassExt`と宣言し、Rustのtrait境界で継承関係を表現する。
  - 親を持たない既定(ルート)クラス(例: `UIElement`)は`base`フィールドを持たない。
  - `Class`構造体は`ClassExt`自身のトレイトに加えて、既定クラスまでの祖先トレイトを**すべて**実装する(`UIElementExt => ControlExt => ContentControlExt`の継承チェーンなら、`ContentControl`構造体は`UIElementExt`・`ControlExt`・`ContentControlExt`の3つのトレイトすべてを実装する)。祖先トレイトの各メソッドは`self.base.method(...)`へ委譲するだけの薄い実装になる。
  - 構造体の生成は構造体リテラルを直接書かず、ファクトリー関数`create_class(...)`を経由する(例: `Button`なら`create_button()`)。`margin`/`horizontal_alignment`/`vertical_alignment`/`grid_cell`(`UIElement`が持つ共通フィールド)に加えて、**このクラス自身が宣言する`#[param]`フィールドも含めて全プロパティ**が`create_class(...)`の引数にはならない——ネイティブ手書きビルトイン(`Window`/`Button`/`TextArea`/`MenuBar`/`Menu`/`MenuItem`/`MenuBarItem`/`TabView`/`TabViewItem`)と`elwindui-core::ui`の仮想ビルトイン(`VerticalLayout`/`HorizontalLayout`/`Shape`/`TextBlock`/`Control`/`Grid`)は、`Copy`なフィールドを`Cell`、それ以外を`RefCell`で持ち(§7.2)、`create_class()`は常に引数なしで`UIElement::default()`相当の既定値を組み立てるだけ。使用箇所ごとの値は、構築**後**に`binding.set_<field>(..)`(margin等の共通属性なら`binding.base().set_margin(..)`)という呼び出しで反映する(`elwindui-codegen`の`emit_common_ui_element_setters`/`build_component_setters`)——`resync()`による値の再反映(二重バインディング等)も同じ`set_<field>(..)`を呼ぶだけで済む、単一の統一された仕組みになる。(`view`を持つコンポーネント——`ContentControl`/`Rectangle`/`Ellipse`のような組み込みでも、ユーザー定義componentでも——の生成`new(args)`は今のところ対象外で、引数どおりに構築する従来の方式のまま。)
  - `Button`/`TextArea`/`Window`/`Menu`/`MenuBar`/`MenuItem`/`MenuBarItem`/`TabView`/`TabViewItem`のように、このクレート(`elwindui-core`)自身は対応する構造体を持たず各バックエンドクレートが個別に実装する「純粋インターフェース宣言」(`trait_only`、`docs/elwindui_macro_class_spec.md`§2.2)は、`{Class}Ext`ではなく素の`Class`という裸名のトレイトになる(同名の競合する構造体がこのクレート内に存在しないため)。
- コード生成器(`elwindui-codegen`)が`component X inherits Y`から生成するコードも同じ形を取る——親の実効フィールド/メソッドを1つのstructへ畳み込む(フラット化する)のではなく、`X { base: Y, /* Xの宣言分のみ */ }`という実体合成にする。これにより`base::method(...)`(§2.1)は名前を変えたシャドーメソッドを介さず、文字通り`self.base.method(...)`に書き換えるだけで済む。`.elwind`のDSL構文や、他コンポーネントが`X { ... }`と書く箇所は一切変更不要——`X::new(args)`という既存の呼び出し規約は変わらず成立する。

```rust
pub trait UIElementExt: AsAny {
    fn base(&self) -> &UIElement;
    fn margin(&self) -> f32 { self.base().margin.get() }
    fn horizontal_alignment(&self) -> HorizontalAlignment { self.base().horizontal_alignment.get() }
    fn vertical_alignment(&self) -> VerticalAlignment { self.base().vertical_alignment.get() }
    fn visual_children(&self) -> Vec<Rc<dyn UIElementExt>>;
    fn measure_override(&self, available: Size, child_sizes: &[Size]) -> Size;
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect>;
    fn render(&self, context: &mut RenderContext) {}
    fn as_native_control(&self) -> Option<&dyn Any> { None }
}

// margin/alignment/visibility等は全て内部可変(`Cell`/`RefCell`) —
// `create_xxx(...)`は常に`UIElement::default()`相当を組み立てるだけで、使用箇所ごとの値は構築後に
// `set_margin(..)`等のセッターで反映する(前掲の規約説明参照)。以下は要点のみの簡略化した抜粋で、
// 実際の`UIElement`構造体は`width`/`height`/`measured_size`/`arranged_offset`/`routed_handlers`等
// 多数のフィールドを追加で持つ(`crates/elwindui-core/src/ui.rs`参照)。
pub struct UIElement {
    pub margin: Cell<f32>, // 一律のMargin。Thickness(上下左右個別)は未対応
    pub horizontal_alignment: Cell<HorizontalAlignment>, // Left | Center | Right | Stretch(既定)
    pub vertical_alignment: Cell<VerticalAlignment>,     // Top | Center | Bottom | Stretch(既定)
    pub visibility: Cell<Visibility>,                    // Visible(既定) | Collapsed
}
```

`Visibility`はWinUI3の`UIElement.Visibility`と同じく`Visible`(既定)/`Collapsed`の2値のみ(WPFの`Hidden`相当は無い)。`Collapsed`な要素はレイアウト上スペースを一切取らず(`measure`が常に`(0, 0)`を返す——自身の`Width`/`Height`指定も無視する)、`arrange`/`hit_test`の対象からもその子孫ごと除外される(描画されず、ヒットテストにも当たらない)。`margin`/`horizontal_alignment`と同じ共通属性だが、`.elwind`側の`margin`のような即値配線(`emit_common_ui_element_setters`)はまだ無く、`set_visibility(..)`をRustから直接呼ぶ形にとどまる。

`UIElement`はこの階層の既定(ルート)クラスなので`base`フィールドを持たない。`UIElementExt`トレイト自体はハンドル型`H`について非ジェネリックである。実ネイティブハンドルを持つのは各バックエンドの`NativeControl`実装(下記)だけであり、木を歩く汎用関数(`measure`/`arrange`/`layout_root`)の方がハンドル型`H`についてジェネリックになっている。

```
UIElement (構造体、Margin/Alignment共通実装。baseなしの既定クラス。トレイトは`UIElementExt`。
 │        `builtins.elwind`上もDSLの`component UIElement {}`として存在する全ての根)
 ├─ NativeControl<H> => Button, TextArea, TabView, ... (実ハンドルHを保持する、ビジュアルツリーに
 │                       実際に埋め込まれる型のみ。MenuBar/MenuBarItem/Menu/MenuItem/TabViewItemは
 │                       ツリーに参加しない(measure/arrangeが呼ばれない)ため`#[native]`直接指定で
 │                       この枝に入らない——`Window`と同じ扱い、§5.1a・builtins.elwindの
 │                       `NativeControl`マーカー自身のコメント参照)
 ├─ TextBlock            (プリミティブ描画・非native、付録F.3)
 ├─ Shape => Rectangle, Ellipse (プリミティブ図形、子を持たない。付録F.6)
 ├─ Control              (Padding + ContentAlignmentを持つ、複数の小部品からなる複合部品。
 │   │                    `children: UIElementCollection`をLogicalツリーとして持つ、§5.2)
 │   └─ ContentControl   (Content1つだけを持つ複合部品、`inherits`によるDSL合成の実例。§2.1)
 └─ Layout => VerticalLayout, HorizontalLayout, Grid (レイアウトコンテナを束ねる共通親。
                          `builtins.elwind`上もDSLの`component Layout inherits UIElement { children:
                          UIElementCollection }`として存在し、`children`はVerticalLayout/
                          HorizontalLayout/Gridへ自動的に継承される(§5.2)。付録F.2・付録F.11)
```

`Layout`は`children: UIElementCollection`という1フィールドのみを持ち、`VerticalLayout`/`HorizontalLayout`/`Grid`構造体がこれを実装する。`VerticalLayout`/`HorizontalLayout`はさらに、DSL上には現れない共通の内部実装`Stack`(`orientation`/`spacing`/`children`を持つ)を`base`フィールドとして共有し、`UIElementExt`をそこへ委譲する——各バックエンドの`NativeControl`実装を`Button`/`TextArea`/`TabView`が共有するのと同じ trait+struct+base の形(§5.1a)。`Grid`は`rows`/`columns`/`children`を自前で持ち、`Stack`は経由しない。

`VerticalLayout`/`HorizontalLayout`/`Grid`はいずれも自前の`view`を持たない仮想ビルトインなので、`Layout`の`children`フィールドを無条件に継承する(`resolve_effective_fields`)——各コンポーネント側で`children`を再宣言する必要はない。ただし`#[content(children)]`(WinUI3の`ContentPropertyAttribute`相当、§5.2参照)はフィールドと違い継承されないため、3つとも個別に宣言している。

これらの構造体は、対応する`create_xxx(...)`クラス自身の`new()`で`Rc`化してから、各コレクションの`add()`で子を木に組み込む。

実ハンドルを持つ型(各バックエンドの`NativeControl`実装)の判定は`UIElementExt`の`as_native_control(&self) -> Option<&dyn Any>`というデフォルト`None`のメソッド経由で行う(`NativeControl`自身が`Some(self)`を返す)。単純な`AsAny`経由の`downcast_ref`ではなく、この一段の間接参照を挟むのは、`Button`構造体のように`base: NativeControl`を**自分自身のフィールドとして合成する型**(§5.1a)がある場合、木に置かれる実際の具象型は`Button`であって`NativeControl`そのものではなく、`Any::downcast_ref`は実際の具象型に対してしか成功しないため——`Button`は`as_native_control`を`Some(&self.base)`とオーバーライドして委譲する。「実ハンドルを持つ」という概念を持たない大多数の実装(`VerticalLayout`/`HorizontalLayout`/`Grid`/`Shape`/`TextBlock`/`Control`)は既定の`None`のままでよく、不要なボイラープレートを背負わない。

`Window`は`UIElement`を派生しない。WinUI3の`Window`が`UIElement`ではなく独立したトップレベルのホストであるのと同様、`Window`は`content: Rc<dyn UIElement>`を保持し自身のクライアント領域に対して`measure`/`arrange`を呼び出す**ホスト**である(AppKitの`TreeHostView`/WinUI3の`TreeHostPanel`がこの役割を実装する)。

`VerticalLayout`/`HorizontalLayout`は交差軸方向の配置を一律設定として持たない——各子要素自身の`horizontal_alignment`/`vertical_alignment`が交差軸配置を決める、WinUI3の`StackPanel`と同じ設計である。主軸方向は常に「Auto」(子の自然サイズ)である。

`Grid`(実装済み、`docs/elwindui_builtins_spec.md`参照)は行/列ベースのレイアウトで、`VerticalLayout`/`HorizontalLayout`にはない「残り領域を`*`比例配分で埋める」手段(`GridLength::Star`)を提供する。各子の行/列位置は`elwindui_dsl_spec.md`§3の添付プロパティ(`Grid::row`/`Grid::column`)で指定し、`UIElement.grid_cell`(既定`(0, 0)`)として子要素自身が保持する——`Grid`自身が子ごとの別テーブルを持つわけではない。

### 5.2 Logical/Visualツリーの分離

WinUI3に倣い、「`.elwind`で書かれた見た目上の参照関係」(Logicalツリー)と「実際にlayoutされる`Rc<dyn UIElement>`の木」(Visualツリー)を区別する。既存の`component`+`view`パターン(例:`DocumentView`)は、実質的に既に「1つの論理ノード → 展開された`UIElement`木」というLogical/Visual構造を持っている。

- **Logicalツリー**:`.elwind`上の参照関係(例:`NotepadWindow`から見て`DocumentView`は1個のノード)。将来のテンプレート機能・アクセシビリティツリーはこちらを対象にする。今回は`LogicalNode { type_name, children }`という最小限の型のみ導入し、コード生成側からはまだ生成されない(将来のテンプレート/データバインド機能向けの受け皿として未使用のまま残す)。`Layout`(`VerticalLayout`/`HorizontalLayout`/`Grid`)/`Control`が`.elwind`上で宣言する`children: UIElementCollection`(下記)は、この`LogicalNode`とは別の、より具体的な仕組み——これらのコンテナはテンプレート機構を持たない(1つの`.elwind`宣言がそのまま1つのVisualノードになる)ため、Logical上の子要素リストが同時にVisual上の子要素そのものでもある。
- **Visualツリー**: 実際にlayoutされる`Rc<dyn UIElement>`の木(`Layout`/`Shape`/`TextBlock`/`NativeControl`/`Control`から組み立てられる)。§5.1・付録Fで説明している木はこちら。`UIElement::visual_children(&self) -> &[Rc<dyn UIElement>]`がこの木を歩く汎用関数(`measure`/`arrange`/`hit_test`)から参照される、Visualツリー専用のアクセサ。

`elwindui_core::ui::UIElementCollection`はWinUI3自身の`UIElementCollection`に相当する型で、`Layout`/`Control`が`.elwind`上で`#[content(children)]`(WinUI3の`ContentPropertyAttribute`相当)付きで宣言するLogicalツリーの子要素リストを表す。`Stack`/`Control`/`Grid`構造体はこれを実フィールドとして保持し、`visual_children()`はそこから`as_slice()`で直接導出される(`UIElementCollection::new(Vec<Rc<dyn UIElement>>) -> Self`/`as_slice(&self) -> &[Rc<dyn UIElement>]`のみを持つ薄いラッパー)。`Shape`(`Rectangle`/`Ellipse`)は実WinUI3の`Shape`同様、子要素を一切持たない純粋なリーフである。

`Control`(§5.1参照)は「Logical上は1ノード、Visual上は複数の小部品」という構造を体現する型として導入された——`Padding: f32`(一律)と`ContentAlignment`(`HorizontalAlignment`/`VerticalAlignment`の組)、および`children: UIElementCollection`を持ち、WinUI3の`Control`基底クラス(独自描画ではなく複数の小部品を持てるカスタム部品)に相当する。

### 5.3 レイアウトエンジン

WinUI3の`Measure`/`Arrange`2パス方式を採用する。

```rust
trait LayoutNode {
    fn measure(&self, available: Size) -> Size;
    fn arrange(&mut self, final_rect: Rect);
}
```

各バックエンドのネイティブ葉ウィジェットのハンドル(`elwindui-backend-appkit::AnyView`等)がこのトレイトを実装する(`NativeControl<H>`経由で`UIElement`に接続される)。`Stack`や`Canvas`を含む全ビルトインがこのトレイトを実装する。`.elwind`側の`width`/`height`/`spacing`等の属性がそのままMeasure/Arrangeの入力になり、新しい構文は不要。**レイアウト計算は`elwindui-core`内の共通実装(1つのRustクレート)で一元化され**、バックエンドは計算結果(確定した矩形座標)を受け取ってネイティブAPIに反映するだけ、という役割分担にする。

| バックエンド | レイアウト計算の主体 |
|---|---|
| WinUI3 | Core Runtimeで計算 → 結果を絶対配置コンテナに反映 |
| AppKit / GTK4 | 同様にCore Runtimeの計算結果を`NSView.frame`/`gtk_widget_size_allocate`に反映 |

この一元化により、全バックエンドで同一のレイアウト結果が保証される。

### 5.4 再描画要求(`invalidate`)と`RelayoutHost`のコアレシング契約

`UIElement`は`invalidate`/`invalidate_measure`/`invalidate_arrange`(WinUI3の`InvalidateVisual`/`InvalidateMeasure`/`InvalidateArrange`相当)を持つ。見た目(サイズ・配置・描画内容)に影響する値を変更するプロパティセッター(`TextBlock::set_text`、`Shape::set_fill`、`UIElement::set_margin`等)は、値を書き換えた後に必ずこのいずれかを呼び、自分がホストされているツリーへ再レイアウトを要求しなければならない。呼び忘れると、モデル側の値は正しく更新されているのに画面には一切反映されない(コード生成が`resync()`から`set_text(...)`等を呼んでも無効化されない)、という不具合になる。

```rust
trait UIElement {
    fn invalidate(&self);          // 既定実装: request_relayout(self.base())
    fn invalidate_measure(&self);  // 同上
    fn invalidate_arrange(&self);  // 同上
}
```

`elwindui-core`のレイアウトエンジンは要素ごとのMeasure/Arrangeキャッシュを持たない(§5.3の`layout_root`はmeasure/arrangeを実行し、host保持の`RenderTree::reconcile`が描画treeを同期する)ため、上記3メソッドは現状すべて同一の`request_relayout`——ホストされているツリーの根まで`parent()`を辿り、そこに登録された`RelayoutHost`(`UIElement::invalidate_host`)へ再レイアウトを依頼する——に帰着する。3つを分離してあるのは将来Measure/Arrangeを別々にキャッシュ・再計算できるようにするための拡張余地であり、現時点で意味の使い分けはない。

**`RelayoutHost::request_relayout()`の契約**: 「呼ばれた**今すぐ**」ではなく「しかるべきタイミングで**1回だけ**」ツリー全体の再レイアウトを行うことを、実装する各バックエンドに義務付ける。具体的には:

1. 同一の同期実行区間内(例:1回の`resync()`が複数の`set_*`を呼ぶケース)で`request_relayout()`が複数回呼ばれても、実際の再レイアウトは高々1回にまとめる(コアレシング/デバウンス)。
2. 実際の再レイアウトは、その場で同期的に行うのではなく、ホストのUIイベントループの次のタイミング(次の描画サイクル/次のディスパッチキュー実行)に委ねる。
3. 「今すぐ全部やり直す」実装(呼ばれるたびに同期的にツリー全体を再構築する実装)はこの契約に違反する——ツリーが大きい場合や1回の`resync()`が複数のセッターを呼ぶ場合に、無駄な再構築がその回数分発生してしまうため。

各バックエンドでの実装方針:

| バックエンド | `request_relayout`の実装 |
|---|---|
| AppKit | `NSView.setNeedsLayout(true)`を呼ぶだけでよい——AppKit自身が次の描画サイクルまでに1回へコアレシングする(追加のフラグ管理は不要)。 |
| WinUI3 | 実ネイティブの`Canvas.Children`を毎回同期的に総入れ替えする実装のため、`pending: Cell<bool>`で多重エンキューを防いだ上で、`DispatcherQueue.TryEnqueue`で実際の再レイアウトをUIスレッド上で1回だけ実行する(`elwindui_backend_winui3::WinUI3RelayoutHost`参照)。 |
| GTK4(将来実装時) | `gtk_widget_queue_allocate`/`glib::idle_add_local`等、同種の「次のイベントループ反復まで間引く」機構を使うこと。 |

新しいバックエンドを追加する際・既存バックエンドの`RelayoutHost`実装をレビューする際は、このコアレシング契約を満たしているかを確認すること。

### 5.5 フォーカス管理

```rust
trait FocusManager {
    fn move_focus(&mut self, direction: FocusDirection) -> Option<ElementId>;
    fn set_focus(&mut self, id: ElementId);
    fn focused(&self) -> Option<ElementId>;
    fn trap_focus(&mut self, scope: ElementId);
}

enum FocusDirection { Next, Previous, Up, Down, Left, Right }
```

`.elwind`側は`#[focus(order: 1)]`/`#[focus(trap: true)]`属性で参加する。

```rust
component LoginForm {
    #[focus(order: 1)]
    username: String,
    #[focus(order: 2)]
    password: String,

    #[focus(trap: true)]
    ...
}
```

Tab移動順序(`order`)や方向キー移動はCore Runtimeが共通ロジックとして提供する。ネイティブ系バックエンドはOS標準のフォーカス機構(WinUI3の`FocusManager`、AppKitの`NSResponder`チェーン、GTK4の`gtk_widget_grab_focus`)にCore Runtime側を正としてミラー同期する。`Dialog`(§8.3)は既定で`#[focus(trap: true)]`が自動適用される。

**実装状況**: `FocusManager`トレイト自体は`elwindui-core::focus`に定義済みだが、現時点では`#[cfg(test)]`内のダミー実装(`SingleFocus`)が1つあるのみで、`UIElement`ツリーやいずれのバックエンドとも結線されていない。`#[focus(...)]`属性・OS標準フォーカス機構へのミラー同期は未実装。

### 5.6 アクセシビリティ

```rust
trait AccessibilityNode {
    fn role(&self) -> AccessibilityRole;
    fn label(&self) -> String;
    fn state(&self) -> AccessibilityState;
    fn children(&self) -> Vec<&dyn AccessibilityNode>;
}

enum AccessibilityRole { Button, TextInput, CheckBox, Slider, StaticText, ... }
```

**`.elwind`側の属性:**

```rust
Button {
    text: t!("notepad-menu-save")
    #[accessible(role: Button, label: t!("a11y-save-button"))]
    on_click: save_document()
}
```

ビルトイン部品は既定roleを自動付与するため通常追記不要だが、`Canvas`ベースの独自部品(§5.7)は意味情報を持たないため`#[accessible(role:, label:, ...)]`の明示を推奨し、付けない場合14章ルール10により静的警告となる。

**バックエンド実装義務:**

| バックエンド | 実装方法 |
|---|---|
| WinUI3 | `AutomationPeer`を生成し、Windows UI Automationに登録 |
| AppKit | `NSAccessibilityElement`プロトコルを実装 |
| GTK4 | `Atk`/AT-SPIブリッジに登録 |

**実装状況**: `AccessibilityNode`トレイトと`AccessibilityRole`/`AccessibilityState`の型定義は`elwindui-core::accessibility`に存在するが、いずれの型に対しても実装(`impl AccessibilityNode for ...`)がなく、`UIElement`ツリーにもいずれのバックエンドのネイティブアクセシビリティAPIにも未結線(`#[accessible(...)]`属性・14章ルール10の警告を含め未実装)。

### 5.7 独自描画部品(Canvas / RenderContext)

グラフ・ゲージ等「ピクセル単位で自分で描く」部品は宣言的な`view`構文の対象外とし、`Canvas`ビルトイン+命令的な`RenderContext`描画コードの組み合わせとして扱う。レイアウトは引き続き`.elwind`で宣言的に書き、描画内容は`RenderContext`を受け取るRust関数として書く。

```rust
struct RenderContext {
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);
    fn stroke_circle(&mut self, center: Point, radius: f32, color: Color, width: f32);
    fn draw_line(&mut self, from: Point, to: Point, color: Color, width: f32);
    fn draw_path(&mut self, path: &Path, style: PaintStyle);
    fn draw_text(&mut self, text: &str, pos: Point, font: Font, color: Color);
    fn draw_image(&mut self, image: &Image, rect: Rect);
}
```

`builtin::Canvas`自身は他ビルトイン同様`RenderContext`へ命令を記録し、backend は保持された`RenderTree`を再生する(バックエンド分岐が許されるのは`builtin`定義のみという§4.1の原則がここでも維持される)。

描画コードは`.elwind`の外、通常のRustファイル(`src/painters/*.rs`)に分離する。推奨ディレクトリ構成:

```
src/
├── ui/       # .elwind本体(レイアウト定義)
├── painters/ # 描画ロジック(通常のRust、バックエンド共通実装)
└── logic/    # on_click等の業務ロジック
```

`Canvas`の`prop`が変わると通常の`prop`更新ルール(§2.2)で再描画がトリガーされる。毎フレーム再描画したい場合は`#[animated]`を付け、その内部でのみ非純粋関数呼び出し(`elapsed_time()`等)が許可される(14章ルール2の例外)。クリック・ドラッグは`on_pointer_down`/`on_pointer_move`で扱い、座標系は論理ピクセルに統一してバックエンド側が実ピクセル変換を担う。

`Canvas`と`Row`/`Column`等の既存部品は同じ`Element`ツリー・`LayoutNode`として自然に混在できる(§2.11・§5.3が支えている)。

### 5.8 描画機能の拡張(Composition相当のビジュアル効果)

`Painter`の基本セット(塗り・線・テキスト)を拡張し、WinUI3の`Win2D`/`Composition`相当の機能を提供する。いずれも`elwindui-core`に属し、バックエンド差異は`builtin::Canvas`内部にのみ許可される(§4.1原則の継続)。

| 機能 | 型/メソッド | 備考 |
|---|---|---|
| ブラシ(単色/グラデーション/画像/Acrylic) | `Brush` enum + `fill_rect_brush`/`stroke_path_brush` | GTK4はAcrylic/Blur非対応時、単色フォールバック+静的警告(14章ルール17) |
| ジオメトリ(ベジエ・弧) | `Path` + `StrokeStyle`(cap/join/dash) | |
| エフェクト(シャドウ・ブラー・色調補正) | `Effect` enum + `#[effect(...)]` | オフスクリーンサーフェスへレンダリング後に適用 |
| 変形(移動・回転・拡縮・スキュー) | `Transform` enum + `push_transform`/`pop_transform` | スタック方式、ネスト可 |
| レイヤー合成・クリップ・ブレンド | `begin_layer`/`end_layer`/`clip_rect`/`clip_path` + `BlendMode` | エフェクトの基盤機構 |
| 暗黙アニメーション | `#[transition(duration, easing)]` | propに付与、値変化時に自動補間描画 |
| キーフレームアニメーション | `KeyframeAnimation::new().add(t, v).easing(...).sample(t)` | `Canvas`内での手続き的制御、位置は`0.0..=1.0`(範囲外は14章ルール16でエラー) |
| リッチテキスト | `TextRun` + `draw_rich_text` | 複数書式混在テキストの1回描画 |

存在しないイージング関数名や範囲外キーフレーム位置は14章ルール16で静的エラーとなる。

### 5.9 Core Runtimeの位置づけ(クレート構成)

```
elwindui-core           # UIElement, LayoutEngine, FocusManager, AccessibilityTree, InputRouter, Painter(共通・バックエンド非依存)
elwindui-backend-winui3 # elwindui-coreを実装 + windows-rsでネイティブAPIに橋渡し(実装あり、Windows環境未検証)
elwindui-backend-appkit # 同上、objc2経由(実装済み・動作確認済み)
elwindui-backend-gtk4   # 同上、gtk-rs経由(現状スタブのみ)
```

`.elwind`コンパイラが生成するコードは常に`elwindui-core`のトレイト境界に対して書かれ、実行時にどのバックエンドクレートがリンクされるかで実体が決まる(§3.3の`target::backend()`と対応)。`elwindui-core`が`UIElement`/`LayoutEngine`/`FocusManager`/`AccessibilityTree`/`InputRouter`/`Painter`という共通・バックエンド非依存な基盤を持ち、各`elwindui-backend-*`クレートがこれを実装してネイティブAPIへ橋渡しする、という構成が全体を貫く設計原則である。

### 5.10 ルーティングイベント(WinUI3スタイル)

WinUI3の`RoutedEvent`に倣い、`#[routed]`属性(`#[two_way]`と同じ、`.elwind`のコールバック型フィールドに付与するアトリビュート)を付けたイベントは、発生元の要素から祖先へバブルする。対象は`on_click`のような入力系イベントに限られ、`TabView`の`on_select(usize)`のようなウィジェット固有の型付きペイロードを持つコールバックはルーティング対象外(既存の直接配線のまま)。

```rust
// crates/elwindui-codegen/src/builtins.elwind (Button)
component Button inherits NativeControl {
    #[routed]
    on_click: fn(),
}
```

**木構造は`Box`ではなく`Rc`で、本物の親ポインタを持つ**。各ノードは論理親`parent`とVisual親`visual_parent`への弱参照を持つ。要素を各コレクションへ追加した瞬間にそのコレクションが対応する親参照を設定する。これにより`dispatch_routed`は論理親を、レイアウトはVisual親を単純に辿れる。

```rust
// elwindui-core
pub fn dispatch_routed<T: 'static>(target: &Rc<dyn UIElement>, name: &str, payload: &T, args: &RoutedEventArgs);
pub fn hit_test(root: &Rc<dyn UIElement>, available: Size, at: Point) -> Option<Rc<dyn UIElement>>;
```

`hit_test`は座標から最深(最前面)の要素を1つ返すだけで、経路は返さない — バブルは戻り値から`dispatch_routed`するだけで済む(親ポインタが経路計算を代替する)。`RoutedEventArgs { handled: Cell<bool> }`にハンドラが`handled`を立てると、そこで伝播が止まる。

ハンドラ本体は`UIElementBase.routed_handlers`(イベント名で引く型消去レジストリ、`Rc<RefCell<HashMap<&'static str, Vec<Box<dyn Any>>>>>`)に登録される。`Button`のようなネイティブウィジェットは、自分自身の構築時点(まだ`NativeControl`ラッパーが存在しない、木構築はboundary-up)に自分自身の`routed_handlers`へ登録し、`elwindui-codegen`の`into_node_if_needed`がラップ時に同じ`Rc`を共有する。実際のネイティブクリック配線(`NSButton`のtarget-action等)は、木とネイティブハンドルの両方が同時にスコープ内にある唯一の場所である各バックエンドの`relayout`(`TreeHostView`/`TreeHostPanel`)が担う。

現時点の実装範囲は、AppKitバックエンドの`Button`のみ(検証済み)。トンネリング(`Preview*`)、`Canvas`上のポインタイベント、WinUI3バックエンドでの実配線は将来の課題として残る。

### 5.11 まとめ

| 要件 | 対応 |
|---|---|
| WinUI3方式の要素階層 | `UIElement`トレイト(非ジェネリック)+ `NativeControl<H>`(実ハンドルを保持する唯一の型)+ `AsAny`によるダウンキャスト(§5.1) |
| Margin/Alignment | `UIElement`(一律`f32`のMargin、`HorizontalAlignment`/`VerticalAlignment`、既定`Stretch`)を全`UIElement`が共通して持つ(§5.1) |
| Logical/Visualツリーの分離 | `.elwind`上の参照関係(Logical、`UIElementCollection`)と実際にlayoutされる`Rc<dyn UIElement>`の木(Visual、`visual_children()`)を区別、`Control`/`Layout`がその橋渡し(§5.2) |
| レイアウト計算の共通化 | `LayoutNode`(Measure/Arrange)を`elwindui-core`で一元計算し、バックエンド間の見た目のズレを防止(§5.3) |
| 再描画要求の一元化 | `RelayoutHost`のコアレシング契約により、1回の同期実行区間で複数回の変更更新があっても再レイアウトは高々1回(§5.4) |
| フォーカス管理の共通化 | `FocusManager`トレイト + `#[focus(order/trap)]`属性、ネイティブ系はOS機構とミラー同期(§5.5) |
| アクセシビリティの共通化 | `AccessibilityNode`トレイト + `#[accessible(role/label/state)]`属性、各バックエンドはOS標準のa11y APIに登録(§5.6) |
| 独自部品(付録G)との整合 | `Canvas`ベースの部品は`#[accessible(...)]`の明示を推奨(付けない場合は静的警告)(§5.7) |
| WinUI3ライクな基盤全体 | `elwindui-core`という共通クレートに集約し、各バックエンドがこれを実装する構成(§5.9) |

---

## 6. ライフサイクル

### 6.1 コンポーネント単位(`on_mount` / `on_unmount` / `on_update`)

```rust
view NotepadWindow {
    on_mount: { load_last_document(); }
    on_unmount: { save_draft(); }
    on_update(content): { state = SaveState::Unsaved; }
    Window { ... }
}
```

- `on_mount`はコンポーネントが要素ツリーに初めて組み込まれた直後に一度だけ、`on_unmount`はツリーから除去される直前に一度だけ実行される
- `on_update(field, ...)`は指定propまたは`#[computed]`の変化毎に発火(複数フィールドを監視する場合は`on_update(a, b): { ... }`のようにカンマ区切りで列挙し、いずれかが変化した時点で発火)。無引数の`on_update: { ... }`は任意prop変化で発火(頻度が高くなるため濫用注意)
- これらは通常のRustコードブロックであり、`#[param]`静的評価式(§2.2)とは別の実行コンテキストのため非純粋関数呼び出し制限は適用されない
- **ただし`#[param]`フィールドへの代入はライフサイクルフック内でも禁止**される(14章ルール11)。`#[param]`の「実体化時のみ確定・以後不変」という原則はフックの内側でも一貫する

コード生成器は各バックエンドのライフサイクル(WinUI3の`Loaded`/`Unloaded`、AppKitの`viewDidAppear`/`viewWillDisappear`、GTK4の`realize`/`unrealize`)にこれらのフックをマッピングする。この変換自体はビルトイン側の責務であり、通常の`component`では意識する必要はない。リスト仮想化(§8.4)でリサイクルされる要素は、プール再利用時に`on_mount`を再発火させず`prop`更新のみで反映する点に注意。

**実装状況**: `on_mount`は実装済み(生成される`new()`の中、`resync()`直後にそのまま展開される)。`on_unmount`はパース・検証・コード生成は実装済みだが、`elwindui_core::ui`に要素の破棄(detach/teardown)通知が現状存在しないため、実行時に呼び出されるトリガーはまだない(`__run_on_unmount`という到達可能なメソッドとしては生成される)。`on_update`、および`#[param]`不変性(14章ルール11)の静的検証は未実装。§2.1で述べた`inherits`の`base::on_mount()`/`base::on_unmount()`呼び出しは実装済み(1階層のみ)。

### 6.2 アプリ全体(OSレベル、モバイル)

§6.1がコンポーネント単位のライフサイクルであるのに対し、モバイルではアプリプロセス全体のバックグラウンド/フォアグラウンド遷移がある。これはルートコンポーネントに対するフックとして別軸で提供する。

```rust
component App {
    on_foreground: { resume_sync(); }
    on_background: { save_state(); }
    on_terminate: { flush_pending_writes(); }
}
```

エントリポイント(ルート)コンポーネント以外での宣言は14章ルール24により静的警告。iOSの`applicationDidEnterBackground`等、Androidの`onPause`等にマッピングされ、デスクトップ系では`on_background`=最小化、`on_terminate`=プロセス終了に対応する。

---

## 7. 状態管理とMVVM

### 7.1 グローバル状態(Store)

`bind!(settings.volume, TwoWay)`のように暗黙に扱ってきた`settings`を、`store`という専用構文で明示的に定義する。

**実装状況**: `store`宣言構文は`elwindui-codegen`のパーサーに未実装(現状の`bind!`/`#[observable]`/`#[computed]`は`viewmodel`向けにのみ実装されている)。本節は設計のみ。

```rust
store AppSettings {
    #[range(0..=100)]
    volume: i32 = 50,
    theme: ThemeMode = ThemeMode::Auto,
    #[persist]
    recent_files: Vec<String> = [],
}
```

- `store`は`component`と似た構文だが`view`を持たない。状態のみを保持する共有可能なデータ定義
- フィールドの型・制約構文は`component`のprop定義と共通
- `#[persist]`が付いたフィールドはアプリ終了後もディスクに永続化される(実際の方式はバックエンドの責務)
- 参照は`bind!(AppSettings.volume, TwoWay)`。`AppSettings`は既定でシングルトン
- storeの変更はプレーンなRust構造体フィールドとして通常ロジックから直接代入でき(`AppSettings.volume = 0;`)、`bind!`で購読する全propに伝播する
- **`#[param]`はstoreを直接参照できない**(14章ルール13)。storeのような実行時変化しうる値は必ず`prop`側で`bind!`を介して取り込む
- シングルトンでなくインスタンスを複数持たせたい場合は`#[scoped]`を付け、`#[param] #[inject]`で注入する(ドキュメント単位・ウィンドウ単位のstoreなど)

### 7.2 ViewModel / アクション(MVVM)

WinUI3/WPF由来のMVVMパターンを、**新しい実行時機構を作らず**`#[computed]`(§2.2)と`store`(§7.1)の仕組みを再利用して導入する。

| MVVMの層 | ElwindUILでの対応 |
|---|---|
| Model | 通常のRust構造体、または`store` |
| ViewModel | `viewmodel`(本節) |
| View | 既存の`component`/`view`。ViewModelを`#[inject]`で受け取り表示のみ担当 |

`viewmodel`はRustネイティブ構文(`#[elwindui::viewmodel] mod foo { struct Foo { ... } impl Foo { ... } }`、通常のRustファイルに書く。WPF/WinUI3のMVVMがViewModelをホスト言語側に置くのと同様)で書く:

```rust
#[elwindui::viewmodel]
mod notepad_view_model {
    struct NotepadViewModel {
        #[observable(default = String::new())]
        #[length(0..=100000)]
        content: String,

        #[observable(default = SaveState::Unsaved)]
        state: SaveState,

        #[computed(expr = content.chars().count() as i32)]
        char_count: i32,

        #[computed(expr = state != SaveState::Saving)]
        save_can_execute: bool,
    }

    impl NotepadViewModel {
        fn save(&self) {
            state = SaveState::Saving;
            document::save(&content);
            state = SaveState::Saved;
        }
    }
}
```

- `struct`は`#[observable]`/`#[computed]`フィールドのみを宣言する。`impl`ブロック内の`fn`/`async fn`は**すべて自動的にアクションとして公開される**(`viewmodel`の呼び出し可能な操作) — struct側に対応するフィールド宣言は一切不要で、`fn`の存在そのものがアクションの宣言を兼ねる。生成される公開メソッド名はそのfnの名前そのもの(`pub fn save(&self)`)で、`Command`型のような専用のラッパー型・`.execute()`呼び出しは存在しない
- `fn`本体内の代入(`state = ...`)・読み取り(`content`)は、同じ`struct`のフィールドへの参照として自動的に`self.set_state(...)`/`self.content()`へ書き換えられる(`#[computed]`の初期化式と同じ規約)
- 非同期化したい場合は単に`async fn`と書く — 専用の属性(旧`#[command(async)]`)は不要で、`fn`自体の`async`キーワードから構造的に判定される。生成コードは`elwindui-core::task::spawn_local`で包まれ、View側からは同期アクションと同じ`vm.save()`という書き方で呼べる(§7.3参照)
- WinUI3/WPFの`Command.CanExecute`に相当する「実行可否」は、専用の仕組みを持たず**普通の`#[computed]`フィールド**として自分で書く(上の`save_can_execute`)。命名規約もなく、View側から好きな名前で`enabled: vm.save_can_execute`のように参照する
- `viewmodel`は`view`ブロックを持てず、ビルトイン要素への参照が内部に出現すると14章ルール19により静的エラーとなる(V/VM分離が構文レベルで強制される)
- View側はViewModelを`#[param] #[inject]`(実体は`#[bindable]`、下記)で受け取り(§7.1の`#[scoped]`+`#[inject]`と同じ注入パターン)、双方向編集フィールドは`bind!(vm.field, TwoWay)`でpropに写し取り、読み取り専用表示(`vm.window_title`等)・アクション呼び出し(`vm.save`)は`view`式中で直接参照してよい(14章ルール13の対象外 — ルール13は`#[param]`初期化式への直接参照のみを禁止)。アクション参照に`()`は付けない — `vm.char_count`のような他の0引数ゲッターと同じ規約
- `.elwind`のDSLネイティブ`viewmodel Name { ... }`構文は`#[observable]`/`#[computed]`のみをサポートし、アクションを宣言する手段を持たない。アクションが必要な`viewmodel`は上記のRustネイティブ構文を使う

**`on_*`イベント属性へのクロージャ構文**: `TabView`の`on_select: fn(usize)`のように引数を取るイベントハンドラは、`|param, ...| 式`または`|param, ...| { 文; ... }`という明示的なクロージャで書く(パラメータは型注釈なし・宣言側の`fn(T0, T1, ...)`から位置対応で型が決まる):

```rust
TabView {
    on_select: |index| vm.select_tab(index)
    on_new_tab: vm.new_tab
}
```

引数を取らないハンドラ(`fn()`)は`on_new_tab: vm.new_tab`のように従来どおりベアパスの糖衣で書ける — このときクロージャを書く必要はない。

**再同期のタイミング**: コード生成器は初期構築時だけ全属性を設定する`resync()`を生成し、以後の更新はブランケットな再同期ではなく各`viewmodel`の型付き`PropertyChanged`購読で行う。`#[observable]`のsetterは代入後に対応する`PropertyId`を通知し、`#[computed]`(アクションの実行可否を表す`#[computed]`フィールドも含む)は依存するsetterの後で再計算されて自身の`PropertyId`も通知する。`Subscription`は表示オブジェクトが保持し、破棄時にDropで解除される。子viewmodelの変更を親viewmodelのコレクション変更として転送しない(文書本文の変更は`TextArea`と文字数表示だけを更新し、親`TabView`のchildrenは更新しない)。

**低オーバーヘッドな内部表現**: 依存関係はコンパイル時に静的抽出し(`#[computed]`と同一の仕組み)、動的な購読リスト(`Vec<Box<dyn Fn()>>`)は持たない。`Copy`可能な型は`Cell<T>`、非`Copy`型のみ`RefCell`で保持し、アクションの本体は具体的なクロージャ型として単相化する(`dyn Trait`を使わない)。複雑な相互依存で静的解析が困難な場合のみ、`elwindui-core`が提供する汎用リアクティブグラフ(スロットマップ+世代インデックスの`SignalId`、Leptos/Xilem系のリアクティブランタイムと同様の設計)にフォールバックする。

**`#[bindable]`**: `component`がviewmodelを注入するフィールドには`#[bindable]`を付ける(`#[param] #[inject]`を暗黙に含む)。`#[elwindui::component]` + `body: view! { .. }`(`docs/elwindui_tool_codegen_design.md`の代替方式(proc-macro)参照)のように`component`とそれが注入する`viewmodel`が別々のマクロ呼び出しとして展開される場合、`component`側のコード生成はその`viewmodel`の実体型を型解決できない(各proc-macro展開は自分自身のトークンしか見えない)。`#[bindable]`はこの判定を型解決ではなく構文マーカーに置き換え、`elwindui::core::reactive::ObservableExt`(プロパティ名文字列で`PropertyChanged`購読を公開する共通トレイト)経由で、型名を知らなくても既存の細粒度更新の仕組みを維持する(呼び出しは常に具体的な型に単相化され、`dyn ObservableExt`は使わない)。`#[bindable]`はviewmodel注入の標準形であり、プロジェクト全体でこの形に統一する。

`viewmodel`は`view`を持たず、ビルトイン要素にも依存しないため、バックエンドを一切起動せず通常の`#[test]`で単体テスト可能(§9参照):

```rust
#[test]
fn save_disables_while_saving() {
    let vm = NotepadViewModel::new();
    vm.set_content("hello".to_string());
    vm.save();
    assert_eq!(vm.state(), SaveState::Saving);
    assert!(!vm.save_can_execute());
}
```

`store`との関係:

| | `store` | `viewmodel` |
|---|---|---|
| 目的 | アプリ全体で共有される永続的/半永続的データ | 特定View向けの表示用データと操作 |
| インスタンス | 既定でシングルトン(`#[scoped]`で複数化可) | 常にView単位、`#[inject]`で注入 |
| アクション | 持たない(素のRustロジック関数を直接呼ぶ) | `impl`ブロックの`fn`がそのまま公開メソッドになる。実行可否は普通の`#[computed]`で自分で表現する |

### 7.3 非同期処理

ファイル読込・API呼び出し等の非同期処理と、`prop`/アクションの連携を定義する。新しい実行モデルは導入せず、既存の`#[computed]`・アクションを非同期版に拡張する。

**実装状況**: `elwindui-core::task`の`spawn_local`(非同期タスク起動プリミティブ)は実装済みで、`examples/notepad`の非同期ファイル保存/読込(`platform::file_dialog`)で実際に使われている。一方`AsyncState<T>`enum・`#[async_computed]`属性・`task!`マクロは設計のみでコード中に実体がなく未実装。

```rust
enum AsyncState<T> { Idle, Loading, Success(T), Error(String) }

viewmodel DocumentViewModel {
    #[observable]
    file_path: String,

    #[async_computed]
    content: AsyncState<String> = task!(async { fs::read_to_string(&file_path).await }),
}
```

View側は他のenumと同じく`match`で扱い、網羅性検査により状態の処理漏れ(例:`Error`ケースの表示忘れ)が静的に検出される:

```rust
match vm.content {
    AsyncState::Idle    => TextBlock { text: "" }
    AsyncState::Loading => Spinner {}
    AsyncState::Success(text) => TextArea { text }
    AsyncState::Error(msg)    => TextBlock { text: msg, color: "#e74c3c" }
}
```

- `AsyncState<T>`は通常のenumとして網羅性検査の対象(`match`でIdle/Loading/Success/Errorの処理漏れを静的検出)
- `#[async_computed]`は`#[computed]`の非同期版。`#[observable]`依存が変化すると自動再実行され、実行中は`Loading`
- `#[async_computed]`が`viewmodel`/`store`以外に付与された場合は静的エラー(14章ルール20) — 非同期状態はVM/Model層に閉じ込め、`component`の`#[param]`静的評価式を汚染しない。アクション側の非同期は`async fn`という構造そのものから判定されるため(§7.2)、対応する専用属性は存在しない — `viewmodel`の`impl`ブロック以外に`async fn`アクションを書く場所自体がないので、この種の静的検査を別途必要としない
- 実行中の多重実行防止・キャンセルは(旧`#[command(async, ...)]`が担っていた領域)専用の仕組みを持たない — 必要なら`#[computed]`の実行可否フィールド(§7.2)を自分で`false`にする、または独自の`Cancelled`状態を`#[observable]`で管理する
- `elwindui-core`はホストの非同期ランタイムを直接指定せず`spawn(fut)`という薄い抽象を提供し、各バックエンドがWinUI3の`DispatcherQueue`/AppKitの`DispatchQueue.main`/GTK4の`glib::MainContext`に橋渡しする

### 7.4 Undo/Redo

編集操作のUndo/Redoを`viewmodel`フィールドへの共通仕組みとして提供する。

**実装状況**: `#[undoable]`属性は`elwindui-codegen`の`Attr`列挙体に未実装。本節は設計のみ。

```rust
viewmodel NotepadViewModel {
    #[observable]
    #[undoable(coalesce: 500ms)]
    content: String = String::new(),
}
```

`#[undoable]`フィールドが1つ以上ある`viewmodel`には、`undo`/`redo`アクション(§7.2 — `impl`ブロックの`fn`と同じ形で自動生成される)と対応する`can_undo`/`can_redo`の`#[computed]`フィールドが自動的に追加され、§7.2の通常のアクションと同じ書き方で結線できる:

```rust
Button { text: t!("menu-undo"), on_click: vm.undo, enabled: vm.can_undo }
```

- `#[undoable]`は`viewmodel`の`#[observable]`フィールドにのみ付与できる(14章ルール21) — Undo単位は「1つのViewの編集セッション」に紐づくため、アプリ全体共有の`store`や`component`の`prop`には意味を持たない
- `#[undoable]`フィールドが1つ以上ある`viewmodel`には`undo`/`redo`アクションと`can_undo`/`can_redo`が自動追加される
- `coalesce: 500ms`で連続入力を1つのUndoエントリにまとめる(`#[transition(duration:...)]`と同じ「時間指定アトリビュート」の慣習)

---

## 8. UI機能拡張ビルトイン

### 8.1 キーボード入力・ショートカット

ポインタ系イベント(`on_pointer_down`等、§5.7)に加え、キーボード入力・IME・アプリ全体のショートカットを扱うための構文。

**実装状況**: `#[focus(order/trap)]`・ショートカット構文はいずれも`elwindui-codegen`に未実装。本節は設計のみ。

**要素単位のキーイベント:**

```rust
TextArea {
    text: content
    on_key_down: |key| handle_key(key)
    on_text_input: |text| handle_ime_commit(text)
}
```

- `on_key_down` / `on_key_up` — 物理キーの押下・離上(修飾キー状態を含む`Key`型を受け取る)
- `on_text_input` — IME確定後の実文字列、または直接入力の文字を受け取る(IME変換中の未確定文字列はバックエンドが内部で処理し、DSL側には確定結果のみが渡る)

これらのイベントを受け取るには、当該要素がフォーカスを持っている必要がある(§5.5の`FocusManager`と連動する)。

**グローバルショートカット:**

```rust
Button {
    text: t!("notepad-menu-save")
    #[shortcut("Ctrl+S")]
    on_click: save_document()
}
```

`#[shortcut("...")]`はプラットフォーム非依存の修飾キー表記(`Ctrl`/`Shift`/`Alt`/`Meta`)を使う。コード生成時に、macOS向けビルドでは`Ctrl`が自動的に`Cmd`に読み替えられる(WinUI3/GTK4等の他backendではそのまま`Ctrl`として扱う)、というプラットフォーム変換規則を標準で持つ。明示的にOSごとの割り当てを変えたい場合は複数指定できる:

```rust
#[shortcut(winui3: "Ctrl+S", appkit: "Cmd+S")]
on_click: save_document()
```

既定では、`#[shortcut(...)]`が付いた要素はその要素がフォーカスされていなくてもアプリウィンドウ内であれば発火する(メニューショートカットと同じ扱い)。要素にフォーカスがある場合のみ発火させたい場合は`scope: local`を指定する:

```rust
#[shortcut("Ctrl+F", scope: local)]
on_key_down: |_| find_in_selection()
```

### 8.2 画面遷移(ナビゲーション)

`NavigationHost`ビルトインによるルートベースの画面遷移機構。

```rust
enum Route { Main, Settings, Search }

view App {
    NavigationHost {
        route: current_route
        match current_route {
            Route::Main     => NotepadWindow { }
            Route::Settings => SettingsWindow { }
            Route::Search   => SearchDialog { }
        }
    }
}
```

`match current_route { ... }`は`Route`の全メンバー網羅を要求される(14章ルール14、§2.3と同じ仕組み)。遷移操作は`navigate!(route)`(遷移+履歴push)/`navigate_back!()`(履歴を1つ戻す)。`NavigationHost`はビルトインのため内部で`match target::backend()`を持つ(WinUI3=`Frame`、AppKit=`contentViewController`差し替え、GTK4=`gtk::Stack`)。§4.1の原則通り、通常のcomponentはこの分岐を書けない。

### 8.3 ダイアログ・ポップアップ・メニュー

- `Dialog`: モーダル。`#[focus(trap: true)](§5.5)`が自動適用され、`on_dismiss`はEsc・外側クリック・明示的な閉じるボタンいずれからも発火
- `Menu`/`MenuItem`: コンテキストメニュー。`context_menu`属性で任意要素に紐付け
- `tooltip`: 任意のビルトイン要素が持てる共通属性

いずれもビルトインで内部に`match target::backend()`を持ち、独自部品からの利用時は§4.1のバックエンド分岐禁止原則がそのまま適用される(14章ルール15)。

### 8.4 リスト仮想化

大量データを`for`でそのまま描画すると全要素が`Element`化され性能が破綻するため、表示範囲のみ描画する`VirtualList`を提供する。

```rust
VirtualList {
    items: documents
    key: |doc| doc.id
    item_height: 32
    render_item: |doc| Row { Text { text: doc.name } }
}
```

- `key`関数は要素の同一性判定に使い、順序が変わっても同じkeyのデータは`Element`インスタンスを使い回す(Reactのkey付きリコンサイル相当)
- `item_height`固定なら§5.3のMeasureパスをスキップし定数時間で表示範囲を計算、`estimated_item_height`のみなら初回`measure`で実測しキャッシュ
- 画面外に出た`Element`はプールに戻し再利用する。再利用インスタンスでは`on_mount`(§6.1)は初回プール生成時のみ発火し、以降は`prop`更新のみ行う
- `key`未指定で順序が変わる更新を行うと挿入位置ベースの再利用にフォールバックし、14章ルール23により静的警告

### 8.5 テーマ/デザイントークン

`style{}`(§2.4)は個別属性の上書きに留まるため、カラーパレット・スペーシング・タイポグラフィを一元管理する`theme`構文を用意する。

**実装状況**: `theme`構文は`elwindui-codegen`のAST(`ast::Item`)に対応する項目がなく未実装。本節は設計のみ。

```rust
theme AppTheme {
    tokens { color primary; color background; color text; spacing unit; font body; font heading }
    variant Light { primary: "#2ecc71"; background: "#ffffff"; ... }
    variant Dark  { primary: "#27ae60"; background: "#111111"; ... }
}
```

- 全`variant`は`tokens{}`宣言のトークンを過不足なく持たねばならない(14章ルール22) — 「ダークモードだけ特定の色が未定義」という事故を静的に防ぐ
- 参照は`AppTheme.token名`という`.`アクセス(`env::*`やstoreフィールド参照と同じ慣習)。`style{}`からも`Painter`/`Brush`(§5.8)からも同じ記法で参照可能
- 実行時切り替えはファイル単位アトリビュート`#![theme(AppTheme, variant: bind!(AppSettings.theme_mode, OneWay))]`で宣言し、storeの変化に応じて`AppTheme.*`参照箇所が自動再評価される(既存のprop差分更新の仕組みに乗る)

### 8.6 エラーハンドリング(エラーバウンダリ)

`view`内の予期しないエラーでアプリ全体をクラッシュさせず、該当部分だけフォールバック表示に切り替える。

**実装状況**: `ErrorBoundary`ビルトイン・`#[catches(...)]`ともに`crates/elwindui-codegen/src/builtins.elwind`にはまだ定義がなく未実装。本節は設計のみ。

```rust
ErrorBoundary {
    fallback: |err| Text { text: t!("error-fallback", message: err.to_string()), color: "#e74c3c" }
    NotepadWindow { }
}
```

- `view`構築・`#[computed]`評価・`Canvas`の`on_paint`実行中のエラーを捕捉し`fallback`に置き換える。ネスト可能で内側の`ErrorBoundary`が捕捉範囲を限定する
- 内部的には`catch_unwind`相当の仕組みで囲むが、ネイティブAPI呼び出し(COM/Objective-C/GObject)を跨ぐパニックは言語境界でUB化する恐れがあるため、ネイティブ呼び出し部分は`Result`化を必須としcatch_unwindは純粋Rustロジックの範囲に留める(ベストエフォート方針)
- 同期アクションのエラーは`#[catches(ErrorType)]`をアクションのfnに付与すると`viewmodel`の`last_error`相当フィールドに自動格納(§7.3の非同期版と対になる同期パターン)
- 未捕捉時は`elwindui-core`既定のフォールバック画面(デバッグ=詳細スタック、リリース=簡潔メッセージ)でクラッシュを防止する

### 8.7 クリップボード・ドラッグ&ドロップ・ファイルダイアログ

OS機能へのアクセスをGUI要素ではなく`platform::`名前空間の関数として提供する(`env::*`/`external::*`と同じ「明示的な入口」の思想)。

```rust
platform::clipboard::write_text(&content);
let text: Option<String> = platform::clipboard::read_text();
```

ファイルダイアログは本質的に非同期(ユーザー操作待ち)なので常に`Future`を返し、§7.3の非同期アクション(`async fn`)パターンと組み合わせる。ドラッグ&ドロップは`draggable: bool`/`on_drag_start`/`on_drop`を任意のビルトイン要素が持てる共通属性として提供する。

**実装状況**: `platform::file_dialog`(`open()`/`save() -> Option<PathBuf>`)のみ実装済み(AppKitで動作検証済み、WinUI3は未検証、GTK4は未実装。フィルタ指定引数は現状なし)。上記コード例の`platform::clipboard::write_text`/`read_text`、およびドラッグ&ドロップの`draggable`/`on_drag_start`/`on_drop`属性は未実装(コード自体が存在しない)。

| 機能 | WinUI3 | AppKit | GTK4 |
|---|---|---|---|---|
| クリップボード | `Clipboard`/`DataPackage` | `NSPasteboard` | `Gdk::Clipboard` |
| ファイルダイアログ | `FileOpenPicker`/`FileSavePicker` | `NSOpenPanel`/`NSSavePanel` | `gtk::FileChooserNative` |
| D&D | `DragDrop`イベント | `NSDraggingDestination` | `Gtk::DropTarget` |

### 8.8 モバイル対応(iOS / Android)

§3.3の`Backend` enumに`Uikit`(iOS)/`Jetpack`(Android)を追加し、既存バックエンド抽象化をそのまま拡張する。バリアント追加に伴う既存ビルトインの網羅性エラー(§3.3参照)は、各`builtin`定義に対応する`native!`腕を追加することで解消する。

**実装状況**: iOS(UIKit)/Android(Jetpack)バックエンドクレートはワークスペースに存在せず未着手。§3.3の`Backend` enum/`target::backend()`自体が未実装のフォワードルッキング設計であるため、`Uikit`/`Jetpack`追加も含め本節は設計のみ。

```rust
#[cfg(target_os = "ios")]
#![backend(uikit)]

#[cfg(target_os = "android")]
#![backend(jetpack)]
```

| ElwindUIL論理要素 | UIKit(iOS) | Android(jni経由) |
|---|---|---|
| `Window` | `UIWindow` + `UIViewController` | `Activity` + `ComposeView`/`Fragment` |
| `Button` | `UIButton` | `android.widget.Button` または Compose `Button` |
| `TextArea` | `UITextView` | `EditText` |
| `Stack`(Column/Row) | `UIStackView` | `LinearLayout` |

Rustバインディングは、iOSは`objc2`(AppKitと同系統のクレート)、Androidは`jni`クレート経由でJava/Kotlin APIを呼ぶ。

- **画面サイズ・向き・セーフエリア**: 実行中に変化しうる値であるため`env::*`を拡張せず、§7.1と同じ`store`の仕組みを使ったビルトインStoreとして提供する(`store platform::Device { orientation, safe_area, window_size }`)。参照は通常のstoreと同じく`bind!`経由必須(14章ルール13)
- **セーフエリアのレイアウト反映**: `Window`ビルトインは既定で`respects_safe_area: true`を持ち、§5.3のレイアウトエンジンがセーフエリアを差し引いて利用可能領域を計算する
- **タッチジェスチャー**: `on_swipe`/`on_pinch`/`on_long_press`を任意のビルトイン要素の共通属性として一般化(§5.7の`on_pointer_down`等の拡張)。デスクトップ系backendはマウス操作からの近似にフォールバック
- **OSレベルライフサイクル**: §6.2参照
- **DPI対応**: 論理ピクセル座標の方針(§5.7)を継承。`Image::asset("icon")`がDPI別バリアント(`icon@1x/@2x/@3x.png`)を実行環境のスケールファクタから自動解決
- **パーミッション**: `platform::permissions::request(Permission::Camera).await`が直接`PermissionStatus`を返す(§8.7の`platform::`名前空間+§7.3の非同期パターンの組み合わせ)

---

## 9. テスト支援

§7.2の`viewmodel`単体テスト(バックエンド非起動)に加え、`view`が組み立てる要素ツリー・描画結果を検証するスナップショットテストを提供する。

| テスト対象 | 手段 |
|---|---|
| ビジネスロジック・アクションの振る舞い | 通常の`#[test]` + `viewmodel`の直接操作(バックエンド起動不要) |
| 要素ツリーの構造(レイアウト・分岐結果) | `elwindui_test::render_tree` + `assert_snapshot!`(`insta`クレート等の慣習に合わせる) |
| Canvas等のピクセル単位の描画結果 | `elwindui_test::render_canvas_snapshot` + `assert_image_snapshot!` |

```rust
#[test]
fn notepad_initial_view_matches_snapshot() {
    let vm = NotepadViewModel::new();
    let tree = elwindui_test::render_tree(&NotepadWindow { vm });
    assert_snapshot!(tree);
}
```

`render_tree`は`UIElement`ツリー(§2.11)を、各ノードの型名(`UIElement::type_name()`)によるテキスト表現(インデント付きの構造ダンプ)に変換する。`assert_snapshot!`は既存のRustスナップショットテストの慣習(`insta`クレート等)に合わせ、差分があれば失敗し、承認コマンドで期待値を更新できるようにする。

```rust
#[test]
fn knob_renders_correctly_at_half_value() {
    let image = elwindui_test::render_canvas_snapshot(|p| draw_knob(p, 0.5), Size::new(60.0, 60.0));
    assert_image_snapshot!(image);
}
```

`render_canvas_snapshot`はツール設計書側のオフスクリーンレンダリング機能(プレビュー用)を再利用する。新しいBackend variant(テスト専用等)は追加せず既存backendのヘッドレスモードを用いることで、`match target::backend()`の網羅性検査(§2.3・§3.3)に影響を与えない設計になっている。

**実装状況**: `elwindui-test`クレートは`render_tree`(`Element`ツリーのインデント付きダンプ)のみ実装済み。`render_canvas_snapshot`/`assert_image_snapshot!`は未実装(依存先のオフスクリーンレンダリング機能自体がプレビューツール未着手のため存在しない、`docs/elwindui_tool_preview_design.md`参照)。`viewmodel`単体テストは通常のRustの`#[test]`をそのまま使えるため追加のテストヘルパーを要さず、この点はドキュメント通り機能する。

---

## 10. 静的検証ルール一覧(14章)と機能対応表

コンパイラ/リンタが実行前に検出すべき項目(ツール側の実装対象だが、各ルールがフレームワークのどの不変条件を守っているかは設計上重要なため一覧化する)。

**実装状況**: `crates/elwindui-codegen/src/validate.rs`(約1600行)が静的検証の実装本体だが、本表の24ルールすべてが1対1でルール番号付きで実装されているわけではない(現状ソース中に明示的なルール番号コメントがあるのはルール18・19のみ)。多くはこの表の各節が説明する言語機能(`#[param]`/`bind!`/enum網羅性検査/`viewmodel`のview参照禁止等)のバリデーションとして実質的に実装されているが、ルール9(`native!`/`target::backend()`制限)・14(`NavigationHost`網羅性)・15(オーバーレイ系の分岐制限)は§3.3・§8.2・§8.3で述べた通り前提となる`target::backend()`自体が未実装のため検証しようがない。ルールごとの詳細な実装状況は`docs/elwindui_implementation_status.md`を参照。

| # | ルール概要 | 関連する本ドキュメントの節 |
|---|---|---|
| 1 | `#[param]`初期化式に`bind!`/propの参照/`#[computed]`が出現 → エラー | §2.2 |
| 2 | `#[param]`初期化式に非純粋関数(`now()`,`random()`等)が出現 → エラー(`env::*`/`once`は例外) | §2.2, §2.7, §5.7(`#[animated]`が例外) |
| 3 | `#[computed]`フィールドへの外部代入 → エラー | §2.2 |
| 4 | enum値の裸文字列直書き(完全修飾でない参照) → エラー | §2.6 |
| 5 | `match`におけるenumメンバーの網羅漏れ → エラー | §2.3 |
| 6 | 制約付きフィールドへの制約違反代入 → ビルド時/実行時エラー | §2.5 |
| 7 | `external::*`呼び出しが`once`宣言以外に出現 → エラー | §2.7 |
| 8 | importの循環・未解決パス → エラー | §2.10 |
| 9 | `#[overrides(builtin::X)]`のない通常componentに`native!`/`target::backend()`出現 → エラー | §3.4, §4.1 |
| 10 | `Canvas`を含む`view`に`#[accessible(...)]`なし → 警告 | §5.6, §5.7 |
| 11 | ライフサイクルフック外での`#[param]`相当の再代入 → エラー | §6.1 |
| 12 | `bind!`参照先が`store`宣言に存在しない → エラー | §7.1 |
| 13 | `store`/ViewModel/ビルトインStoreフィールドへの`#[param]`側からの直接参照 → エラー | §7.1, §7.2, §8.8 |
| 14 | `NavigationHost`内`match route`の非網羅 → エラー | §8.2 |
| 15 | ダイアログ/メニュー等オーバーレイ系ビルトイン外での`native!`/`target::backend()` → エラー | §8.3 |
| 16 | `Transition`/`KeyframeAnimation`の不正イージング名・範囲外キーフレーム → エラー | §5.8 |
| 17 | `Effect`のバックエンド非対応組み合わせ → 警告(フォールバック明示) | §5.8 |
| 18 | (欠番 — `Command`機構撤廃により削除。アクションはRustの`impl` fnとして自動検出されるため、対応する型検査自体が不要になった) | §7.2 |
| 19 | `viewmodel`定義内に`view`ブロック/ビルトイン要素への直接参照 → エラー | §7.2 |
| 20 | `#[async_computed]`が`viewmodel`/`store`以外に付与 → エラー | §7.3 |
| 21 | `#[undoable]`が`viewmodel`の`#[observable]`以外に付与 → エラー | §7.4 |
| 22 | `theme`の`variant`間でトークン集合が不一致 → エラー | §8.5 |
| 23 | `VirtualList`に`key`なしで順序変更 → 警告 | §8.4 |
| 24 | `on_foreground`/`on_background`/`on_terminate`がルート以外で宣言 → 警告 | §6.2 |

---

## 11. 責務分担まとめ:コンパイラ/コード生成器 vs ランタイムライブラリ

| 責務 | 担当 |
|---|---|
| `.elwind`のパース・型検査・14章の静的検証ルール適用 | コンパイラ(ツール設計書側、`elwindui-codegen`) |
| `component`/`view`/`enum`/`store`/`viewmodel`からのRustコード生成 | コンパイラ |
| バックエンド判定の定数畳み込み(`target::backend()`)、非該当分岐の除去 | コンパイラ |
| `.ftl`の静的パースと`t!`キー・引数名の整合性検証 | コンパイラ |
| `Element`/`LayoutNode`/`FocusManager`/`AccessibilityNode`/`Painter`トレイトの定義と共通実装 | `elwindui-core`(本ドキュメント§5) |
| `find_by_id`/`find_all`などの再帰探索アルゴリズム | `elwindui-core`(DSL非依存、独立に拡張可能) |
| レイアウト計算(Measure/Arrange)の一元実装 | `elwindui-core`(§5.3) |
| ViewModelの依存関係静的抽出結果に基づく`Cell`/`RefCell`ベースの実行時更新 | コンパイラが生成するコード + `elwindui-core`の汎用リアクティブグラフ(フォールバック時のみ) |
| ネイティブAPIへの橋渡し(WinUI3/AppKit/GTK4/Uikit/Jetpack) | 各`elwindui-backend-*`クレート |
| コード生成・LSP診断・プレビュー・ホットリロード | ツール設計書(`docs/elwindui_tool_*_design.md`)側の対象 |

この分担の要点: **DSLの文法自体は「`Element`ツリーへの到達可能性」「param/propの静的/動的区別」「enum網羅性」といった契約のみを保証し、探索アルゴリズムやレイアウト計算・リアクティブ更新の実装詳細はすべて`elwindui-core`側の関心事として分離されている**。これにより、将来的にレイアウトエンジンの実装を差し替えたり新しいバックエンドを追加したりしても、DSLの構文自体は変更不要という設計になっている。
