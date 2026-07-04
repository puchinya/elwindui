# ElwindUIL 言語仕様書

Rust向けGUIフレームワーク(Elwind)のための宣言的レイアウト記述言語。
Rustの構文・慣習に寄せることで学習コストを下げつつ、機械可読性・事前検証性を重視した設計。

---

## 1. 設計目標

- マシンが読みやすい(静的解析・型検査・網羅性検査がしやすい)
- 人が読みやすい(Rust経験者にとって既視感のある構文)
- 冗長度が低い
- GUIの親子関係を自然に表現できる
- 部品(コンポーネント)を定義し、パラメータ付きで再利用できる
- パラメータから他の属性値を計算できる
- スタイルシート的な制御文による要素生成ができる
- 多言語対応を言語仕様に統合する
- 他コンポーネントのインポートができる
- パラメータ(実体化時固定)とプロパティ(実行時可変)を区別できる
- 値制約を数式的に、かつ静的検査可能な形で定義できる
- 値候補がある場合は列挙体(enum)として定義できる

---

## 2. 基本構造

要素はRustの構造体リテラルに似た記法で記述し、ネストがそのまま親子関係になる。

```rust
Row {
    Text { text: "Hello" }
    Button { text: "OK" }
}
```

- 属性は `key: value` 形式
- カンマ・改行はどちらも区切りとして等価
- 単純な識別子・リテラルの参照は `${}` 不要。演算や結合を含む式のみ `format!` 等を使う

```rust
Text { text: label }                  // 単純参照
Text { text: format!("{label}!") }    // 式はformat!マクロで明示
```

---

## 3. component と view

コンポーネントは **`component`(データ定義)** と **`view`(描画ロジック)** の2ブロックに分離する。Rustの `struct` と `impl` の関係に対応する。

| | component | view |
|---|---|---|
| 役割 | 状態(フィールド)の定義 | 状態→見た目の写像 |
| 対応するRust概念 | `struct Foo { ... }` | `impl Foo { fn view(&self) -> Element }` |
| 書く内容 | 型・制約・初期値のみ | `if`/`for`/`match`による要素ツリーの組み立て |
| 変更頻度 | 低い(型は安定) | 高い(レイアウト調整で頻繁に変わる) |

```rust
component VolumeControl {
    #[param]
    orientation: Orientation = Orientation::Horizontal,

    #[range(0..=100)]
    #[step(5)]
    volume: i32 = bind!(settings.volume, TwoWay),

    #[computed]
    label: String = format!("{volume}%"),
}

view VolumeControl {
    if orientation == Orientation::Horizontal {
        Row { Slider { value: volume }, Text { text: label } }
    } else {
        Column { Slider { value: volume }, Text { text: label } }
    }
}
```

**呼び出し側:**

```rust
let sales_card = Card { title: "売上", value: 12000 };
```

- インスタンス命名は専用記号を使わず、Rustの `let` 束縛をそのまま使う
- 属性名と変数名が一致する場合はショートハンド可

```rust
Card { title, value }   // title: title, value: value の省略形
```

---

## 4. param と prop(実体化時固定 vs 実行時可変)

フィールドは既定で **`prop`**(実行時に読み書き可能)。`#[param]` アトリビュートを付けたときのみ、実体化時に一度だけ確定し以後不変になる。

| | `#[param]` を付けたフィールド | 既定(prop)のフィールド |
|---|---|---|
| 変更可能性 | 実体化時のみ、以後イミュータブル | 実行時いつでも変更可 |
| 使える式 | 静的評価式のみ(リテラル・他paramの参照・純粋関数・`env::*`) | 静的評価式に加え `bind!`・他propの参照・`#[computed]` |
| 主な用途 | 構造分岐(`if`/`for`の条件)、レイアウト決定 | 表示内容・状態の動的更新 |
| 実行時アクセス | 不可(代入するとコンパイルエラー) | `instance.field` で読み書き可 |

`#[computed]` を付けたフィールドは依存する他フィールドの変化に応じて自動再評価される読み取り専用の算出値。外部からの代入は静的エラーとなる。

```rust
component Cart {
    items: Vec<Item>,

    #[computed]
    total: f64 = items.iter().map(|i| i.price * i.qty).sum(),
}
```

**静的評価式に許可される要素(`#[param]`用):**

- リテラル(数値・文字列・真偽値・配列)
- 四則演算・比較・三項演算子相当の `if` 式
- 組み込み純粋関数(`min`, `max`, `round` など)
- 同一コンポーネント内の他の `#[param]` フィールドへの参照
- `env::*`(動的定数、後述)

**禁止される要素:**

- `bind!(...)` の使用
- prop(`#[param]`が付いていないフィールド)の参照
- 非純粋関数(`now()`, `random()` など)の直接呼び出し

---

## 5. 制御構文

Rust標準の制御構文をそのまま採用し、専用ディレクティブは設けない。

```rust
// 繰り返し
for item in items {
    Card { title: item.name, value: item.value }
}

// 条件分岐
if is_admin {
    Button { text: "管理画面" }
} else {
    Text { text: "権限がありません" }
}

// 分岐(網羅性検査つき)
match status {
    Status::Loading => Spinner {},
    Status::Error   => Text { text: "エラー", color: "#c0392b" },
    Status::Ok      => Text { text: "OK" },
}
```

`match` は列挙体の全メンバーを網羅していれば `_ =>` を省略できる。網羅されていない場合はコンパイルエラーとなる(Rustの`match`と同じ挙動)。

---

## 6. スタイル(横断的属性適用)

```rust
style {
    select(Text) { font_family: "Noto Sans" }
    select(Button, variant == "danger") { color: "#e74c3c" }
}
```

- `select(要素型, 条件式)` で対象を絞り込み、属性をマージ適用する
- インライン属性がスタイル定義より優先(後勝ち・詳細優先)

---

## 7. 値制約(アトリビュートによる数式的表現)

制約はRustのアトリビュート構文(`#[derive(...)]` と同じ見た目)で表現し、数式的な区間・パターンで記述する。

| 記法 | 意味 |
|---|---|
| `#[range(0..=1)]` | 閉区間 |
| `#[range(0..100)]` | 半開区間 |
| `#[range(0..=100)] #[step(5)]` | 区間+刻み幅(multipleOf相当) |
| `#[length(3..=16)]` | 文字列長の範囲 |
| `#[pattern(r"^[a-z]+$")]` | 正規表現 |
| `#[format(email)]` | 組込み検証型(email, url, color_hex 等) |
| `#[check(expr, message = "...")]` | 相関検証(数式化できない場合) |

```rust
component LoginForm {
    #[length(3..=16)]
    #[pattern(r"^[a-zA-Z0-9_]+$")]
    username: String,

    #[format(email)]
    email: String,

    password: String,

    #[check(password == confirm_password, message = "パスワードが一致しません")]
    confirm_password: String,
}
```

**検証タイミング:**

- リテラル値による制約違反 → ビルド時静的エラー
- `bind!` 等の動的値による制約違反 → 実行時エラー

---

## 8. 列挙体(enum)

値候補があるフィールドは共用体を書き捨てず、名前付き `enum` として定義する。Rustのenum構文をそのまま採用する。

```rust
enum Orientation {
    Horizontal,
    Vertical,
}

enum ThemeMode {
    #[label(t!("enum.theme.light"))]
    Light,
    #[label(t!("enum.theme.dark"))]
    Dark,
    Auto,
}

enum LogLevel {
    Debug = 0,
    Info = 10,
    Warning = 20,
    Error = 30,
}
```

- 値の参照は `EnumName::Member` の完全修飾のみ(裸文字列直書きは型不一致として静的エラー)
- `EnumName::values()` で全メンバーを列挙可能(`for`との組み合わせで選択UIを自動生成できる)
- `#[label(...)]` アトリビュートで多言語表示名を付与でき、`member.label()` で現在ロケールの文字列を取得する
- `match` と組み合わせることで、全メンバーを処理しているかどうかの網羅性検査が働く

```rust
view ThemeSelector {
    for m in ThemeMode::values() {
        Radio {
            text: m.label(),
            checked: selected == m,
            on_select: selected = m,
        }
    }
}
```

匿名の共用体型(`"a" | "b"` のようなインライン列挙)は採用しない。Rustに無名enumがないことと整合させ、値集合を扱う手段は常に名前付き `enum` に一本化する。

---

## 9. 動的定数(env / once)

「実体化時に一度だけ確定し、以後は変化しない」値を扱うための仕組み。`#[param]` の静的評価式の例外として参照を許可する。

```rust
component TitleBar {
    #[param]
    style: String = if env::os() == "macos" { "traffic-light" } else { "caption" },
}
```

**組み込み `env` 関数(例):**

- `env::os()` — `"windows" | "macos" | "linux" | "ios" | "android"`
- `env::platform()` — `"desktop" | "mobile" | "web"`
- `env::locale()` — 実行環境の既定ロケール
- `env::direction()` — `"ltr" | "rtl"`

**ユーザー拡張(一度だけ確定するグローバル値):**

```rust
once BUILD_CHANNEL: String = external::build_channel();

component DebugBanner {
    #[param]
    visible: bool = BUILD_CHANNEL != "stable",
}
```

- `external::*` の呼び出しはトップレベルの `once` 宣言でのみ許可し、動的性の入口を一箇所に集約する

---

## 10. データバインディング

```rust
volume: i32 = bind!(settings.volume, TwoWay),
```

- `bind!(path, mode)` — マクロ呼び出し形式(Rustの `vec!` 等の慣習に合わせる)
- `mode`:
    - `OneWay`(既定):外部→propの一方向反映
    - `TwoWay`:UI操作で外部側にも書き戻す
    - `OneTime`:実体化時に一度だけ取り込み、以後固定

---

## 11. 多言語対応(i18n)

翻訳文言は独自フォーマットを持たず、業界標準の **Fluent(.ftl)** をそのまま採用する。DSL側は `t!` マクロでFluentのメッセージIDを参照するだけで、複数形・性別分岐・日付/数値フォーマットはFluent自身の構文(`select`式、`NUMBER()`/`DATETIME()`関数)に委譲する。

```rust
Text { text: t!("dashboard-title") }
Text { text: t!("cart-item-count", count: n) }
Text { text: t!("order-saved-at", time: order.created_at) }
Text { text: t!("item-price", price: price) }
```

**言語ファイル(`.ftl`、言語ごとに分離):**

```ftl
# strings/ja.ftl
dashboard-title = ダッシュボード

cart-item-count = { $count ->
    [0] カートは空です
   *[other] {$count} 点の商品
}

order-saved-at = 保存日時: { DATETIME($time, dateStyle: "medium") }
item-price = { NUMBER($price, style: "currency", currency: "JPY") }
```

```ftl
# strings/en.ftl
dashboard-title = Dashboard

cart-item-count = { $count ->
    [0] Your cart is empty
    [one] {$count} item
   *[other] {$count} items
}

order-saved-at = Saved at: { DATETIME($time, dateStyle: "medium") }
item-price = { NUMBER($price, style: "currency", currency: "JPY") }
```

- 複数形・性別などの分岐はFluentの `select` 式(`[one]`/`[other]`等、CLDRカテゴリ準拠)にそのまま委譲する。DSL側に `plural!` のような専用マクロは不要で、`t!` 一本化できる
- 日付・数値のロケール依存フォーマットもFluent組み込みの `DATETIME()` / `NUMBER()` 関数に委譲する
- RTL言語対応のため、`padding_start`/`padding_end` 等の論理方向プロパティを使う
- フォールバック規則(FluentBundleの標準的な扱いに合わせる):

```
i18n {
    default: "en"
    fallback: ["en"]
    available: ["ja", "en", "ar"]
    resources: "strings/{locale}.ftl"
}
```

- ビルド時に `.ftl` を静的パースし、DSL内で参照している `t!("key", ...)` のメッセージIDが**全`available`言語で定義されているか**を機械的に検証する(未翻訳キーの検出)
- `t!` に渡す引数名は、対応する `.ftl` メッセージ内の `{ $引数名 }` と一致していなければ静的エラーとする

---

## 12. モジュール(import)

```rust
use components::card::Card;
use components::widgets::{StatCard, Badge};
use components::common_kit as UI;
use components::card::Card as ProductCard;
```

- Rustの `use` 構文と完全に一致させる
- 静的にimportを解決し、循環参照・未解決参照を機械的に検出できる

---

## 13. 要素ツリーの探索(Element trait / children)

### 役割分担の方針

「子要素を持つ」という性質は既存の `{}` ネスト構文がそのまま表現しているため、**children専用の新しいDSL構文は追加しない**。その代わり、コード生成器が全要素型に共通のトレイトを自動実装するという規約を仕様として定める。再帰探索アルゴリズム自体はDSLの文法ではなく、共通ランタイムライブラリ側の責務とする。

| 責務 | 担当 |
|---|---|
| 親子構造の宣言 | DSL構文(`{}` ネスト。追加構文は不要) |
| 動的生成された子要素(`if`/`for`/`match`の結果)をchildrenとして集約する規約 | コード生成器 |
| 全要素が`children()`/`id()`を返すという契約(トレイト定義) | コード生成器が自動実装 |
| 再帰探索アルゴリズム(`find_by_id`, `find_all` 等) | 共通ランタイムライブラリ(DSLとは独立に拡張・最適化可能) |
| 特定要素への後からのアクセス | `#[id(...)]` アトリビュート |

### 共通トレイト(コード生成器が自動実装)

```rust
trait Element {
    fn children(&self) -> Vec<&dyn Element>;
    fn id(&self) -> Option<&str> { None }
}
```

- `view` 内で `{}` ネストにより宣言された子要素は、そのままコード生成器によって `children()` の返り値に詰められる
- `if` / `for` / `match` によって実行時に確定する子要素も、生成時にフラット化された `Vec<Box<dyn Element>>` として `children()` に集約される、という規約に統一する

```rust
view Toolbar {
    Row {
        if show_save { ToolbarButton { text: "Save" } }
        for item in extra_buttons { ToolbarButton { text: item.label } }
    }
}
```

上記のように条件・繰り返しで生成された要素も、`Row` インスタンスの `children()` から一律に辿れる。

### 特定要素への名前付きアクセス:`#[id(...)]`

`let` 束縛は同一 `view` 関数内でのみ有効なため、外部(Rustロジック側)から後で要素を参照したい場合は `#[id(...)]` アトリビュートを付与する。

```rust
view NotepadWindow {
    #[id("editor")]
    let editor = TextArea { text: content };

    Column { editor, StatusBar { ... } }
}
```

- `#[id(...)]` が付いた要素は `id()` が対応する文字列を返すようコード生成される
- 付与していない要素は `id()` が `None` を返す(既定実装のまま)

### 再帰探索API(共通ランタイムライブラリ、DSL非依存)

```rust
// idによる再帰探索(深さ優先)
fn find_by_id<'a>(root: &'a dyn Element, id: &str) -> Option<&'a dyn Element> {
    if root.id() == Some(id) { return Some(root); }
    root.children().iter().find_map(|c| find_by_id(*c, id))
}

// 型による再帰探索(該当する型の要素をすべて収集)
fn find_all<'a, T: 'static>(root: &'a dyn Element) -> Vec<&'a T> {
    // children() を再帰的に辿り、Tにダウンキャスト可能なものを収集する
    ...
}
```

- 探索方式(深さ優先/幅優先)やキャッシュ戦略の変更は、**DSLの構文を一切変えずに**ライブラリ側の実装更新だけで完結する
- DSL側が保証するのは「`Element` トレイトを介してツリー全体に到達可能である」という契約のみ

---

## 14. 静的検証ルール一覧

コンパイラ/リンタが実行前に検出すべき項目:

1. `#[param]` フィールドの初期化式に `bind!` / propの参照 / `#[computed]` が出現 → エラー
2. `#[param]` フィールドの初期化式に非純粋関数(`now()`, `random()` 等)が出現 → エラー(`env::*` / `once` 値は例外)
3. `#[computed]` フィールドへの外部代入 → エラー
4. enum値の裸文字列直書き(完全修飾でない参照) → エラー
5. `match` におけるenumメンバーの網羅漏れ(`_ =>` なし) → エラー
6. 制約(`#[range]`, `#[length]`, `#[pattern]` 等)付きフィールドへのリテラル値代入が制約違反 → ビルド時エラー、動的値の場合は実行時エラー
7. `external::*` 呼び出しがトップレベルの `once` 宣言以外の場所に出現 → エラー
8. importの循環・未解決パス → エラー
9. `#[overrides(builtin::X)]` が付いていない通常の`component`の`view`内に `native!` ブロック、または `target::backend()` の参照が出現 → エラー(付録G.3参照。独自部品はバックエンド共通実装に限定する)
10. `view`内に`Canvas`が含まれているが `#[accessible(...)]` が付与されていない → 警告(付録H.4参照)
11. `on_mount`/`on_unmount`ブロックの外で`#[param]`フィールドの再代入相当の操作が行われている → エラー(付録I.3参照。paramの不変性は生涯を通じて保証される)
12. `bind!`の参照先が`store`宣言(付録J)の型・フィールドとして存在しない → エラー
13. `store`フィールドへの`#[param]`側からの直接参照(`bind!`を介さない読み取り)→ エラー(付録J.4参照。storeへのアクセスは常に`bind!`を経由する)
14. `NavigationHost`内の`match route { ... }` がRoute enumの全メンバーを網羅していない(`_ =>`なし) → エラー(8章の網羅性検査と同じ仕組み、付録L.2参照)
15. `Dialog`/`Menu`等のオーバーレイ系ビルトインの外側(通常のcomponent)で`native!`/`target::backend()`が出現 → エラー(ルール9と同じ原則、付録M参照)
16. `Transition`/`KeyframeAnimation`(付録N.6)で存在しないイージング関数名、または範囲外のキーフレーム位置(`0.0..=1.0`外)が指定されている → エラー
17. `Effect`(付録N.3)のパラメータが対応バックエンドでサポートされない組み合わせ(例:GTK4未対応のエフェクト種別)である場合 → 警告(該当バックエンドではフォールバック描画に切り替わる旨を明示)
18. `#[command]`が付与されたフィールドの型が`Command`でない → エラー
19. `viewmodel`定義内に`view`ブロック、またはビルトイン要素(`Row`/`Text`等)への直接参照が存在する → エラー(付録O.2参照。ViewModelは表示ロジックを持たず、MVVMのV/VM分離を静的に強制する)
20. `#[async_computed]` または `#[command(async, ...)]` が `viewmodel`/`store` 以外(通常の`component`のprop等)に付与されている → エラー(付録P参照。非同期状態はVM/Model層に閉じ込める)
21. `#[undoable]` が `viewmodel` の `#[observable]` フィールド以外(`store`や`component`のprop等)に付与されている → エラー(付録U参照)
22. `theme`の`variant`ブロックが`tokens{}`で宣言されていないトークン名を定義している、または`tokens{}`で宣言された一部のトークンを欠いている → エラー(付録R参照。全variant間でトークン集合の一致を保証する)
23. `VirtualList`に`key`が指定されていない状態で`items`の順序が変わる更新が行われる → 警告(付録Q参照。挿入位置ベースの再利用にフォールバックし、リコンサイル効率が低下する可能性がある)
24. `on_foreground`/`on_background`/`on_terminate`(付録W.5)が、アプリのエントリポイント(ルート)コンポーネント以外で宣言されている → 警告(OSレベルのライフサイクルは単一箇所への集約を推奨)

---

## 15. 全体サンプル

```rust
use components::slider::Slider;

enum Orientation {
    Horizontal,
    Vertical,
}

component VolumeControl {
    #[param]
    orientation: Orientation = Orientation::Horizontal,

    #[range(0..=100)]
    #[step(5)]
    volume: i32 = bind!(settings.volume, TwoWay),

    #[computed]
    label: String = format!("{volume}%"),
}

view VolumeControl {
    let slider = Slider { value: volume };

    if orientation == Orientation::Horizontal {
        Row { slider, Text { text: label } }
    } else {
        Column { slider, Text { text: label } }
    }
}
```

---

# 付録A. バックエンド抽象化(GUIフレームワークとの関係)

ElwindUILは特定のGUIフレームワークに依存しない**中間表現**として設計する。`.elwind`は論理的な要素ツリーを記述するのみで、egui・iced・druid/xilemなど具体的なフレームワークへの変換は「バックエンド」が担う。

```
.elwind ファイル(ElwindUIL構文)
        │  コンパイル
        ▼
共通AST(フレームワーク非依存の要素ツリー)
        │  バックエンド別コード生成
        ▼
┌─────────┬─────────┬─────────────┐
│  egui    │  iced    │ druid/xilem │ ...
│ backend  │ backend  │  backend     │
└─────────┴─────────┴─────────────┘
```

## A.1 バックエンド指定

```rust
#![backend(egui)]

use components::slider::Slider;
```

- ファイル単位、または`component`単位でアトリビュートにより指定する
- 制約検証・enum網羅性検査・i18n解決など、これまでの言語機能はすべてバックエンド非依存のフロントエンド解析段階で完結し、バックエンドの選択に影響されない

## A.2 論理要素 ⇔ 具体要素のマッピング例(eguiバックエンド)

| ElwindUIL論理要素 | eguiでの実体 |
|---|---|
| `Window { ... }` | `egui::Window::new(title).show(ctx, \|ui\| { ... })` |
| `Row { ... }` | `ui.horizontal(\|ui\| { ... })` |
| `Column { ... }` | `ui.vertical(\|ui\| { ... })` |
| `Text { text }` | `ui.label(text)` |
| `Button { text, on_click }` | `if ui.button(text).clicked() { on_click() }` |
| `TextArea { text }` | `ui.text_edit_multiline(&mut text)` |
| `Dropdown { ... }` | `egui::ComboBox::from_id_source(...)` |

## A.3 エスケープハッチ:`native!`

フレームワーク固有API(例:eguiのプロットウィジェット)を直接使いたい場合、専用ブロックで生Rustコードを埋め込む。

```rust
view Dashboard {
    Column {
        Text { text: "売上グラフ" }

        native! {
            egui::plot::Plot::new("sales").show(ui, |plot_ui| {
                plot_ui.line(egui::plot::Line::new(self.sales_points()));
            });
        }
    }
}
```

- `native! { ... }` 内は該当バックエンド専用コードであることが明示され、リンタは移植性のない箇所として検出できる

## A.4 即時モード/保持モードとparam・propの関係

- **egui(即時モード)**:`prop`はただの構造体フィールドとして扱われ、毎フレーム`view`相当の関数が呼ばれるため自然に反映される
- **iced/xilem系(保持モード)**:`prop`の変更を検知して`Message`を発行し、差分更新を行う

`Element`トレイト(`children()`/`id()`)や`param`/`prop`の意味自体はバックエンドを問わず共通であり、再描画のトリガー方式のみがバックエンドごとに異なる。

---

# 付録B. ツールチェーン仕様(コード生成・エディタ支援)

## B.1 ビルド時自動生成

`.elwind`をクレートの`build.rs`で自動的にRustソースへ変換する。

```rust
// build.rs
fn main() {
    println!("cargo:rerun-if-changed=src/ui");
    elwindui_codegen::compile_dir("src/ui", std::env::var("OUT_DIR").unwrap());
}
```

```rust
// main.rs
include!(concat!(env!("OUT_DIR"), "/notepad_window.rs"));
```

- `cargo:rerun-if-changed`により、`.elwind`保存後の次回ビルドで自動再生成される(手動コマンド不要)

**代替方式(proc-macro):**

```rust
elwindui::component! {
    include_str!("ui/notepad_window.elwind")
}
```

- 中間ファイルを生成せずコンパイル時に直接展開する。IDE補完精度を重視する場合は`build.rs`方式、シンプルさを重視する場合はproc-macro方式を選択する

## B.2 エディタ内リアルタイム診断(LSP)

専用言語サーバー(`elwindui-languageserver`)が以下を提供する。

- 入力中からの即時診断(制約違反、enum網羅漏れ、`#[param]`への`bind!`混入など)
- 生成されるRustコードのプレビュー表示
- enumメンバー等にホバーした際の、Fluentメッセージ(`t!`)の解決結果表示

## B.3 リアルタイムプレビュー

プレビューは目的に応じて3段階で提供する。

| レベル | 内容 | 状態保持 |
|---|---|---|
| ① 静的プレビュー | 保存のたびに`view`をダミー値/デフォルト値でインスタンス化し、画像としてエディタのWebViewに表示 | なし |
| ② インタラクティブプレビュー | プレビュー内で操作可能。`bind!`参照先を自動的にモックへ差し替え、スライダー等で仮想的に値を操作できる | あり(プレビュー専用の軽量ランタイム) |
| ③ 実行中アプリへの反映 | 実際に動作しているアプリ自体を保存と同時に更新する(ホットリロード) | あり(実行中プロセスの状態を維持) |

**①の処理フロー:**

```
.elwind保存 → LSPが増分パース → component既定値でインスタンス化
    → バックエンドのオフスクリーンレンダリング → WebViewへ画像送信
```

**②のモック化:**

`bind!(path, mode)`が使われている`prop`を自動検出し、プレビュー専用のコントロールUI(スライダー・テキスト欄等)に置き換えることで、実際の外部ストアなしに動作確認できるようにする。

## B.4 実行中アプリへのホットリロード

`hot-lib-reloader`等を用い、`view`関数を動的ライブラリとして差し替える。

```rust
#[hot_lib_reloader::hot_module(dylib = "notepad_ui")]
mod hot_notepad_ui {
    hot_functions_from_file!("src/ui/notepad_window.rs");
}
```

更新粒度の判断は、既存の`param`/`prop`の区分をそのまま利用する。

- `#[param]`フィールドに関わる変更 → 再マウント(状態リセット)
- 既定(prop)フィールドのみの変更 → 差分更新(状態を保持したまま反映)

## B.5 全体アーキテクチャ

```
┌──────────────────────────────────────────────┐
│ エディタ(VSCode等)                             │
│  ┌──────────────┐  ┌─────────────────────────┐ │
│  │ .elwindエディタ   │  │ プレビューパネル(WebView) │ │
│  │ (診断・補完)   │  │  ①静的 / ②操作可能        │ │
│  └──────────────┘  └─────────────────────────┘ │
└──────────────────────────────────────────────┘
        │ 保存イベント
        ▼
┌──────────────────────────────────────────────┐
│ elwindui-languageserver (LSP)                        │
│  - 増分パース・型検査・制約検証                  │
│  - プレビュー用インスタンス生成(既定値/モック)   │
└──────────────────────────────────────────────┘
        │
        ├─→ WebViewへ描画結果を送信(①②)
        │
        ▼(任意・実機確認したい場合)
┌──────────────────────────────────────────────┐
│ 実行中アプリ(dylibホットリロード)               │
│  - #[param]変更 → 再マウント                    │
│  - prop変更のみ → 差分更新、状態保持              │
└──────────────────────────────────────────────┘
```

これら付録A・Bはいずれも言語仕様(`component`/`view`/`param`/`prop`/`Element`トレイト等)を変更せずに構築できるツールチェーン層として位置づける。

---

# 付録C. OSネイティブツールキットへのバックエンド抽象化

`.elwind`の記述は常に1つに保ち、Windows向けビルドでは**WinUI 3**(Windows App SDK。旧WinUI 2/UWP版とは別系統)、macOS向けビルドでは**AppKit**、Linux向けビルドでは**GTK4**というOS標準ツールキットへ、コンパイル時に振り分けてコード生成する。

```
.elwind ファイル(共通定義、1つだけ)
        │
        ▼
共通AST(フレームワーク非依存)
        │
        ├─ Windows向けビルド → WinUI 3 backend(windows-rs経由)
        ├─ macOS向けビルド   → AppKit backend(objc2経由)
        ├─ Linux向けビルド   → GTK4 backend
        └─ 汎用ビルド        → egui/iced backend(付録A参照)
```

OS判定は実行時の`env::os()`(動的定数、実体化時に一度だけ確定)とは別物で、**ビルドターゲット(target triple)によりコンパイル時に確定する**分岐であることに注意する。

## C.1 バックエンド指定

```rust
#![backend(native)]   // ビルドターゲットに応じてOS標準ツールキットへ自動的に振り分ける
```

明示的に固定したい場合はRustの`cfg`属性の慣習に沿って個別指定する。

```rust
#[cfg(target_os = "windows")]
#![backend(winui3)]

#[cfg(target_os = "macos")]
#![backend(appkit)]

#[cfg(target_os = "linux")]
#![backend(gtk4)]
```

## C.2 論理要素 ⇔ 各ネイティブAPIのマッピング

| ElwindUIL論理要素 | WinUI 3 backend | AppKit backend | GTK4 backend |
|---|---|---|---|
| `Window { title, ... }` | `Microsoft::UI::Xaml::Window` | `NSWindow` | `gtk::ApplicationWindow` |
| `Button { text, on_click }` | `Microsoft::UI::Xaml::Controls::Button` | `NSButton` | `gtk::Button` |
| `TextArea { text }` | `Microsoft::UI::Xaml::Controls::TextBox`(`AcceptsReturn: true`) | `NSTextView` | `gtk::TextView` |
| `Column { ... }` | `Microsoft::UI::Xaml::Controls::StackPanel`(`Orientation: Vertical`) | `NSStackView(orientation: .vertical)` | `gtk::Box(orientation: Vertical)` |
| `Dropdown { ... }` | `Microsoft::UI::Xaml::Controls::ComboBox` | `NSPopUpButton` | `gtk::DropDown` |

**生成コードのイメージ(WinUI 3 backend、`windows-rs`経由):**

```rust
use microsoft::ui::xaml::controls::Button;

let button = Button::new()?;
button.SetContent(&t!("notepad-menu-save"))?;
button.Click(&EventHandler::new(move |_, _| {
    self.save_document();
    Ok(())
}))?;
```

**生成コードのイメージ(AppKit backend、`objc2`経由):**

```rust
let button = NSButton::buttonWithTitle_target_action(
    &t!("notepad-menu-save"), &target, sel!(save_document)
);
```

DSL記述者はこれらの違いを一切意識せず、`Button { text: t!("notepad-menu-save"), on_click: save_document() }` と書くだけでよい。

## C.3 OSごとの見た目差はスタイル層に閉じ込める

```rust
style {
    select(Button) {
        // 既定はOS標準の見た目に委ね、何も書かない
    }

    select(Button, backend == Backend::Winui3) { corner_radius: 4 }
    select(Button, backend == Backend::Appkit) { corner_radius: 6 }
}
```

`backend == Backend::Winui3` のような条件はビルドターゲットで確定するコンパイル時定数として扱われ、該当しない分岐はコード生成対象から静的に除外される(デッドコード除去と同様)。

## C.4 プラットフォーム固有機能へのエスケープハッチ

`native!`ブロック(付録A参照)を`#[cfg(backend = "...")]`と組み合わせて使う。

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

`#[cfg(backend = "...")]`が付いたブロックは、対象外のビルドではコード生成・型チェックの対象から除外される。

## C.5 `Element`トレイト・param/propとの整合

ネイティブバックエンドは保持モード(要素が生成後も明示的な更新まで存在し続ける)であるため、以下のように変換される。

- `prop`変更 → 対応するネイティブAPIのプロパティ更新呼び出し(例:WinUI 3なら`button.SetContent(new_text)`、AppKitなら`button.setTitle(new_text)`)
- `#[computed]`の再評価 → 依存する`prop`の変化に応じて該当ウィジェットのプロパティ更新コードが生成される
- `children()`の構成変化(`for`ループの要素数増減等) → コンテナへの`addChild`/`removeChild`相当のAPI呼び出しに変換される(差分検出はコード生成器の責務)

## C.6 まとめ

| 項目 | 担当 |
|---|---|
| `.elwind`の記述 | 常に1つ、プラットフォーム分岐は原則書かない |
| どのOSでどのツールキットを使うか | `#![backend(native)]` またはビルドターゲット別の明示指定(`winui3`/`appkit`/`gtk4`) |
| 論理要素→具体API変換 | 各バックエンドクレート(`elwindui-winui3`, `elwindui-appkit`, `elwindui-gtk4`) |
| OSごとの見た目差 | `style { select(..., backend == ...) }` |
| OS固有機能の直接利用 | `#[cfg(backend = "...")]` + `native!` |
| プロパティ変更の反映方式 | バックエンドが保持モードAPIへの更新呼び出しとして生成、DSL側の`param`/`prop`定義は不変 |

---

# 付録D. バックエンド種別の静的定数(`target::backend()`)

フレームワーク種別(WinUI 3 / AppKit / GTK4 / egui / iced 等)を、`.elwind`ファイル内の式から直接参照できる**コンパイル時静的定数**として扱う。これにより、抽象化されたコンポーネント定義を**1つの`.elwind`ファイル内で完結**させられる。

## D.1 `Backend` enumと`target::backend()`

```rust
enum Backend {
    Winui3,
    Appkit,
    Gtk4,
    Egui,
    Iced,
    Uikit,      // iOS
    Jetpack,    // Android
}
```

`Uikit`/`Jetpack`(付録W:モバイル対応)のように新しいバリアントを追加すると、`Backend`の全メンバーを明示的に列挙している既存の`match target::backend() { ... }`(付録F・付録N等のビルトインリファレンス実装)は8章の網羅性検査により**非網羅としてコンパイルエラーになる**。これは仕様の欠陥ではなく意図した挙動であり、「新しいバックエンドを追加した際に、どのビルトイン実装が未対応かを機械的に洗い出せる」という安全弁として機能する。付録Fのリファレンス実装はデスクトップ系backendの説明を目的として`Backend::Uikit | Backend::Jetpack`腕を省略しているため、実際のプロジェクトでモバイル対応する場合は付録Wの指針に沿って各ビルトインに対応腕を追加する。

- `target::backend()` はビルドターゲット(Cargoのfeature/target triple)からビルド時に一意に確定する定数関数
- `env::os()`(実体化時に一度だけ確定・以後不変な動的定数、9章)とは確定タイミングが異なる。`target::backend()`は**コンパイル前から確定している**ため、より強い静的性を持ち、`#[param]`の静的評価式に無条件で使用できる

| 定数 | 確定タイミング | `#[param]`初期化式での使用 |
|---|---|---|
| `env::os()` 等 | 実体化時に一度だけ | 許可(4章・9章の例外規定) |
| `target::backend()` | コンパイル時(ビルド構成から確定) | 常に許可 |

## D.2 1ファイルで完結する抽象コンポーネント定義

```rust
// components/notepad_window.elwind
component NotepadWindow {
    #[param]
    chrome_style: ChromeStyle = match target::backend() {
        Backend::Winui3 => ChromeStyle::Mica,
        Backend::Appkit => ChromeStyle::Vibrancy,
        _               => ChromeStyle::Flat,
    },

    #[length(0..=100000)]
    content: String = bind!(document.text, TwoWay),
}

view NotepadWindow {
    Window {
        Column {
            TextArea { text: content }

            match target::backend() {
                Backend::Winui3 => native! {
                    self.window.SystemBackdrop(MicaBackdrop::new());
                }
                Backend::Appkit => native! {
                    self.window.contentView().addSubview(&vibrancy_view);
                }
                _ => {}
            }
        }
    }
}
```

- ファイル外のバックエンド属性宣言(`#![backend(...)]`)に頼らず、ファイル内の`match`/`if`式として自然にプラットフォーム分岐を書ける
- `match target::backend() { ... }` は `Backend` の全メンバーを網羅しているかコンパイラが検査する(8章の網羅性検査と同じ仕組み)

## D.3 styleセレクタでの利用

```rust
style {
    select(Button, target::backend() == Backend::Winui3) { corner_radius: 4 }
    select(Button, target::backend() == Backend::Appkit) { corner_radius: 6 }
}
```

## D.4 コード生成時の畳み込み

`target::backend()`はコード生成器がビルド設定から得た値へ定数畳み込みし、該当しない分岐(他backend向けの`native!`ブロック等)は生成対象から静的に除去する。実行バイナリには不要な分岐コードが一切残らない。

```rust
// elwindui_codegen 内部(擬似)
const fn resolve_backend() -> Backend {
    #[cfg(feature = "backend-winui3")] { Backend::Winui3 }
    #[cfg(feature = "backend-appkit")] { Backend::Appkit }
    #[cfg(feature = "backend-gtk4")]   { Backend::Gtk4 }
}
```

## D.5 `#![backend(...)]`属性との役割の違い

| 概念 | 役割 | 確定タイミング |
|---|---|---|
| `#![backend(native)]` / `#![backend(winui3)]`(付録A・C) | どのコード生成器(crate)を使うかというビルド設定 | ビルド構成時 |
| `target::backend()`(本付録) | その結果を`.elwind`の式中から参照するための静的定数 | コンパイル時(式に畳み込み) |

両者は役割が異なるため併存する。前者はプロジェクト全体・ファイル単位のビルド設定、後者はコンポーネント定義内部の条件分岐に使う窓口である。

## D.6 まとめ

| 要件 | 対応 |
|---|---|
| 抽象化コンポーネント定義を1ファイルで完結させる | `target::backend()`という式内定数による分岐(ファイル外属性への依存を排除) |
| フレームワーク指定を静的定数として扱う | `Backend` enum + `target::backend()`(ビルド時確定、`#[param]`に無条件使用可) |
| 構造分岐・スタイル分岐の両方に対応 | `match`/`if`/`style select`いずれの条件にも使用可能 |
| 該当しないbackendのコードを含めない | コンパイル時の定数畳み込みにより非該当分岐を静的除去 |

---

# 付録E. 名前空間とビルトインのオーバーライド規則

ユーザーが`Button`のようなビルトインプリミティブと同名のコンポーネントを定義し、バックエンドごとの実装を`native!`で明示的に書き下したい場合(付録C・Dの応用)の名前解決規則を定める。**大原則として、暗黙のシャドーイングは一切許可しない。**

## E.1 ビルトインは予約名前空間に属する

```rust
builtin::Button
builtin::Text
builtin::Row
builtin::Column
builtin::TextArea
```

- これまで`Row { ... }`等と書いてきた記法は、`builtin::Row`への暗黙の`use`が常に効いている、という扱いにする
- ユーザーが同名の`component`を定義しても`builtin::X`自体は消えず、両者は別の完全修飾名を持つ

## E.2 衝突時の既定挙動:曖昧参照エラー

同一スコープに`builtin::X`とユーザー定義`X`が両方見える状態になった場合、暗黙の優先順位を付けず**静的エラー**とする。

```rust
component Button { ... }   // ユーザー定義

view Foo {
    Button { text: "OK" }   // エラー: builtin::Buttonとユーザー定義Buttonのどちらか曖昧
}
```

## E.3 意図の明示方法(1):別名での共存(推奨)

衝突を避ける最も単純な方法は、ビルトインと異なる名前を付けることである。

```rust
component CustomButton { ... }

view Foo {
    CustomButton { text: "OK" }   // 曖昧さなし
    Button { text: "Cancel" }     // builtin::Buttonがそのまま使われる
}
```

## E.4 意図の明示方法(2):`#[overrides(builtin::X)]`

ビルトインの挙動そのものを意図的に置き換えたい場合(例:全`Button`をネイティブ実装に統一するデザインシステム導入時)に使う。

```rust
#[overrides(builtin::Button)]
component Button {
    text: String,
    #[param]
    enabled: bool = true,
    on_click: fn(),
}

view Button {
    match target::backend() {
        Backend::Winui3 => native! { /* windows-rs実装 */ }
        Backend::Appkit => native! { /* objc2実装 */ }
        Backend::Gtk4   => native! { /* gtk-rs実装 */ }
        _ => Rect { enabled: enabled, on_click: on_click(), Text { text: text } }
    }
}
```

- `#[overrides(builtin::Button)]`が付いたコンポーネントは、そのスコープ内で`Button { ... }`と書いた際にビルトインより優先される
- コンパイラは**ビルトイン側の必須フィールド(シグネチャ)を満たしているか**を検査する。満たしていなければ「置き換え先と互換性がありません」という静的エラーになる

## E.5 `#[overrides]`のスコープ規則

効力は、そのコンポーネントを`use`で取り込んだファイル内でのみ有効とし、プロジェクト全体を暗黙に汚染しない。

```rust
use components::button::Button;   // #[overrides]付きButtonをインポート

view NotepadWindow {
    Button { text: "Save" }   // オーバーライド版が使われる
}
```

```rust
// このファイルではインポートしていないため、通常通りbuiltin::Buttonが使われる
view OtherScreen {
    Button { text: "OK" }   // builtin::Button
}
```

プロジェクト全体で一律に置き換えたい場合は、エントリポイントのファイルで`use`し、通常のモジュールシステムと同じ考え方で再エクスポート・伝播させる。

## E.6 ビルトインを明示的に指定する逃げ道

オーバーライドが有効なスコープ内でも、あえて元のビルトイン実装を使いたい場合に用いる。

```rust
view Foo {
    builtin::Button { text: "常に組み込み実装を使う" }
}
```

## E.7 静的検証ルールの追加

1. 同一スコープに`builtin::X`とユーザー定義`X`が両方見え、`#[overrides]`が付与されていない → 曖昧参照エラー
2. `#[overrides(builtin::X)]`が付いているが、ビルトイン`X`の必須フィールドを満たしていない → シグネチャ不一致エラー
3. `#[overrides]`の対象が存在しないビルトイン名を指している → 未解決参照エラー
4. 複数のコンポーネントが同じビルトインに対して`#[overrides]`を宣言し、同一スコープで両方が`use`されている → 多重オーバーライドエラー

## E.8 まとめ

| ケース | 挙動 |
|---|---|
| ユーザー定義コンポーネントが別名 | ビルトインと共存、曖昧さなし |
| 同名だが`#[overrides]`なし | 静的エラー(曖昧参照として拒否) |
| 同名で`#[overrides(builtin::X)]`あり | そのスコープ内でユーザー定義が優先、ビルトインは`builtin::X`で明示的にのみ参照可能 |
| シグネチャ不一致 | 静的エラー |

---

# 付録F. 標準ビルトイン部品のリファレンス実装

`Window`, `Column`/`Row`, `Text`, `TextArea`, `Dropdown`/`Option`, `Rect` など、これまで暗黙に使ってきたビルトインプリミティブは、実際には `builtin` 名前空間(付録E参照)に属し、コード生成器が標準で提供する。その内部実装は他のコンポーネントと同じ`component`/`view`構文で表現でき、`match target::backend()`による網羅性検査(付録D)や`native!`エスケープハッチ(付録A・C)がそのまま適用される。

## F.1 `builtin::Window`

```rust
enum Direction { Ltr, Rtl }

component Window {
    title: String,
    #[param]
    width: number = 800,
    #[param]
    height: number = 600,
    #[param]
    direction: Direction = env::direction(),
    children: Vec<Element>,
}

view Window {
    match target::backend() {
        Backend::Winui3 => native! {
            let win = Microsoft::UI::Xaml::Window::new()?;
            win.SetTitle(&title)?;
            win.SetFlowDirection(direction)?;
            win.SetContent(&build_children(children))?;
            win
        }
        Backend::Appkit => native! {
            let win = NSWindow::init_with_size(width, height);
            win.setTitle(&text::from(title));
            win.setContentView(&build_children(children));
            win
        }
        Backend::Gtk4 => native! {
            let win = gtk::ApplicationWindow::new(&app);
            win.set_title(Some(&title));
            win.set_default_size(width as i32, height as i32);
            win.set_child(Some(&build_children(children)));
            win
        }
        Backend::Egui | Backend::Iced => native! {
            egui::Window::new(&title)
                .default_size([width, height])
                .show(ctx, |ui| { render_children(ui, &children); })
        }
    }
}
```

## F.2 `builtin::Column` / `builtin::Row`

内部で共通の`Stack`部品に処理を委譲し、`Column`/`Row`はその薄いラッパーとして定義する。

```rust
enum Orientation { Vertical, Horizontal }

component Stack {
    #[param]
    orientation: Orientation,
    #[param]
    spacing: number = 0,
    children: Vec<Element>,
}

view Stack {
    match target::backend() {
        Backend::Winui3 => native! {
            let panel = microsoft::ui::xaml::controls::StackPanel::new()?;
            panel.SetOrientation(orientation)?;
            panel.SetSpacing(spacing)?;
            panel.SetChildren(&build_children(children))?;
            panel
        }
        Backend::Appkit => native! {
            let stack = NSStackView::new();
            stack.setOrientation(orientation);
            stack.setSpacing(spacing);
            stack.addSubviews(&build_children(children));
            stack
        }
        Backend::Gtk4 => native! {
            let b = gtk::Box::new(orientation.into(), spacing as i32);
            for child in build_children(children) { b.append(&child); }
            b
        }
        Backend::Egui | Backend::Iced => native! {
            match orientation {
                Orientation::Vertical   => ui.vertical(|ui| render_children(ui, &children)),
                Orientation::Horizontal => ui.horizontal(|ui| render_children(ui, &children)),
            }
        }
    }
}

component Column { children: Vec<Element> }
view Column { Stack { orientation: Orientation::Vertical, children } }

component Row { children: Vec<Element> }
view Row { Stack { orientation: Orientation::Horizontal, children } }
```

## F.3 `builtin::Text`

```rust
component Text {
    text: String,
    #[param]
    color: ColorHex? = None,
}

view Text {
    match target::backend() {
        Backend::Winui3 => native! {
            let tb = microsoft::ui::xaml::controls::TextBlock::new()?;
            tb.SetText(&text)?;
            if let Some(c) = color { tb.SetForeground(&brush_from(c))?; }
            tb
        }
        Backend::Appkit => native! {
            let label = NSTextField::labelWithString(&text);
            if let Some(c) = color { label.setTextColor(&nscolor_from(c)); }
            label
        }
        Backend::Gtk4 => native! {
            let lbl = gtk::Label::new(Some(&text));
            if let Some(c) = color { apply_css_color(&lbl, c); }
            lbl
        }
        Backend::Egui | Backend::Iced => native! {
            match color {
                Some(c) => ui.colored_label(egui_color(c), &text),
                None    => ui.label(&text),
            }
        }
    }
}
```

## F.4 `builtin::TextArea`

```rust
component TextArea {
    text: String = bind!(self.text, TwoWay),
    #[param]
    padding_start: number = 0,
    #[param]
    padding_end: number = 0,
}

view TextArea {
    match target::backend() {
        Backend::Winui3 => native! {
            let box_ = microsoft::ui::xaml::controls::TextBox::new()?;
            box_.SetAcceptsReturn(true)?;
            box_.SetText(&text)?;
            box_.TextChanged(&TextChangedHandler::new(move |new_text| { text = new_text; }))?;
            box_
        }
        Backend::Appkit => native! {
            let view = NSTextView::new();
            view.setString(&text);
            view.set_delegate_on_change(move |new_text| { text = new_text; });
            view
        }
        Backend::Gtk4 => native! {
            let buf = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
            buf.set_text(&text);
            let tv = gtk::TextView::with_buffer(&buf);
            buf.connect_changed(move |b| { text = b.text(&b.start_iter(), &b.end_iter(), false).to_string(); });
            tv
        }
        Backend::Egui | Backend::Iced => native! {
            ui.add(egui::TextEdit::multiline(&mut text))
        }
    }
}
```

## F.5 `builtin::Dropdown` / `builtin::Option`

```rust
component Option {
    text: String,
    #[param]
    selected: bool = false,
}

component Dropdown {
    options: Vec<Option>,
}

view Dropdown {
    match target::backend() {
        Backend::Winui3 => native! {
            let combo = microsoft::ui::xaml::controls::ComboBox::new()?;
            for opt in &options { combo.Items().Append(&opt.text)?; }
            combo.SetSelectedIndex(find_selected_index(&options))?;
            combo
        }
        Backend::Appkit => native! {
            let popup = NSPopUpButton::new();
            for opt in &options { popup.addItemWithTitle(&opt.text); }
            popup.selectItemAtIndex(find_selected_index(&options));
            popup
        }
        Backend::Gtk4 => native! {
            let model = gtk::StringList::new(&options.iter().map(|o| o.text.as_str()).collect::<Vec<_>>());
            let dd = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
            dd.set_selected(find_selected_index(&options) as u32);
            dd
        }
        Backend::Egui | Backend::Iced => native! {
            egui::ComboBox::from_id_source("dropdown")
                .selected_text(current_selected_text(&options))
                .show_ui(ui, |ui| {
                    for opt in &options { ui.selectable_label(opt.selected, &opt.text); }
                })
        }
    }
}
```

## F.6 `builtin::Rect`(ネイティブAPIを持たないbackend向けの基礎要素)

`Button`(付録Eの`#[overrides(builtin::Button)]`例)がegui/iced backendで代替表現として利用する、クリック可能な最小コンテナ要素。

```rust
component Rect {
    #[param]
    enabled: bool = true,
    on_click: fn()? = None,
    children: Vec<Element>,
}

view Rect {
    match target::backend() {
        Backend::Egui => native! {
            let response = ui.add(egui::Button::new(""));
            if response.clicked() { if let Some(f) = &on_click { f(); } }
            render_children(ui, &children);
        }
        Backend::Iced => native! {
            iced::widget::container(render_children(&children))
                .into()
        }
        // ネイティブ系backendはButton自身が直接実装を持つため、Rectのこの分岐には到達しない
        _ => unreachable!()
    }
}
```

## F.7 部品の全体依存関係(メモ帳の例)

```
NotepadWindow
 ├─ Window
 │   └─ Column(Stack)
 │       ├─ Row(Stack)
 │       │   ├─ ToolbarButton → Button(#[overrides]) → Rect(egui/iced時)
 │       │   └─ Dropdown → Option
 │       ├─ TextArea
 │       └─ StatusBar
 │           └─ Row(Stack) → Text
```

## F.8 各部品で使われている仕様の対応

| 部品 | 使用している仕様 |
|---|---|
| `Window` | `#[param] direction = env::direction()`、`match target::backend()`の網羅性検査 |
| `Stack`(Column/Row) | 他コンポーネントを呼ぶだけの薄いラッパー(合成による名称分離) |
| `Text` | `ColorHex?`(nullable制約)、backendごとのカラー変換 |
| `TextArea` | `bind!(self.text, TwoWay)`による双方向バインディング |
| `Dropdown` / `Option` | `Vec<Option>`という複合型プロパティ、backendごとの選択状態同期 |
| `Rect` | `Backend::Egui`/`Backend::Iced`でのみ到達、他backendでは`unreachable!()` |

これらの標準ビルトイン実装は、通常はコード生成器(`elwindui-codegen`)が内部に持ち利用者が直接編集する必要はないが、`#[overrides(builtin::X)]`(付録E)を使うことで、プロジェクト固有の要件に応じて安全に差し替えられる。

---

# 付録G. 独自描画部品(Canvas / Painter)

グラフ・ゲージ・独自アニメーションのような「ピクセル単位で自分で描く」部品は、既存部品の組み合わせでは表現できない。これは`view`の宣言的な要素ツリー構文の対象外とし、**`Canvas`という専用ビルトインとRustの命令的な描画コードの組み合わせ**として扱う。

## G.1 基本方針

- レイアウト(どこに何を置くか)は引き続き宣言的な`.elwind`で書く
- 描画内容(何をどう塗るか)は宣言的に書かず、`Painter`という抽象描画APIを受け取るRust関数として書く
- `Painter`はバックエンドごとの実描画API(Direct2D/Win2D, Core Graphics, Cairo, egui::Painter等)を裏で呼ぶ薄い抽象化層であり、`elwindui-core`(付録H参照)に属する

```rust
use painters::volume_meter::draw_meter;

component VolumeMeter {
    #[range(0..=1)]
    level: f64,
}

view VolumeMeter {
    Canvas {
        width: 200
        height: 40
        on_paint: draw_meter(painter, level)
    }
}
```

## G.2 `Painter` 抽象APIとバックエンド対応

```rust
trait Painter {
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);
    fn stroke_circle(&mut self, center: Point, radius: f32, color: Color, width: f32);
    fn draw_line(&mut self, from: Point, to: Point, color: Color, width: f32);
    fn draw_path(&mut self, path: &Path, style: PaintStyle);
    fn draw_text(&mut self, text: &str, pos: Point, font: Font, color: Color);
    fn draw_image(&mut self, image: &Image, rect: Rect);
}
```

上記は基本図形のみの最小セットである。グラデーション・シャドウ・変形・アニメーション等、WinUI3の`Composition`/`Win2D`相当の機能一式は付録Nで`Painter`を拡張する形で定義する。

| Painterメソッド | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| `fill_rect` | Win2D `CanvasDrawingSession::FillRectangle` | Core Graphics `CGContextFillRect` | Cairo `cairo_rectangle`+`fill` | `egui::Painter::rect_filled` |
| `draw_line` | Win2D `DrawLine` | `CGContextStrokeLineSegments` | `cairo_move_to`/`line_to` | `Painter::line_segment` |
| `draw_text` | `CanvasTextLayout` | `NSAttributedString::draw` | Pango経由 | `Painter::text` |

`builtin::Canvas`自身は付録Fの他部品と同様、`match target::backend()`で各バックエンドのネイティブ描画コンテキストを`Painter`実装でラップして`on_paint`に渡す(バックエンド分岐が許されるのは`builtin`定義のみという原則はここでも維持される)。

```rust
component Canvas {
    #[param]
    width: number,
    #[param]
    height: number,
    on_paint: fn(&mut Painter),
}

view Canvas {
    match target::backend() {
        Backend::Winui3 => native! {
            let ctrl = microsoft::ui::xaml::controls::CanvasControl::new()?;
            ctrl.Draw(&DrawHandler::new(move |session| {
                let mut p = Win2DPainter::wrap(session);
                on_paint(&mut p);
            }))?;
            ctrl
        }
        Backend::Egui => native! {
            let (response, mut painter) = ui.allocate_painter(egui::vec2(width, height), egui::Sense::hover());
            let mut p = EguiPainter::wrap(&mut painter);
            on_paint(&mut p);
        }
        _ => native! { /* Appkit / Gtk4 も同様にラップ */ }
    }
}
```

## G.3 独自部品はバックエンド共通実装に限定する(重要ルール)

**バックエンド分岐(`native!`/`match target::backend()`)を書けるのは`builtin`定義と`#[overrides(builtin::X)]`が付いたコンポーネントだけ**とする(14章ルール9)。通常の独自部品は常にビルトイン要素の組み合わせ、または`Canvas`+`Painter`のみで実装する。

| コンポーネント種別 | バックエンド分岐の可否 |
|---|---|
| `builtin::*`(付録F) | 可(各OSネイティブAPIを直接呼ぶ) |
| `#[overrides(builtin::X)]`(付録E) | 可(ビルトインの置き換えという性質上必要) |
| 通常の独自部品 | 不可。常にバックエンド共通実装のみ |

**どうしてもネイティブAPIが必要だと感じた場合の判断フロー:**

```
独自部品を書いていて native! が必要だと感じたら:

  Q. これは既存ビルトインの代替実装か?
     YES → #[overrides(builtin::X)] として定義し直す(付録E)
     NO  → Canvas + Painter で表現できないか再検討する
           それでも無理な場合のみ、新しいビルトインをフレームワーク側に追加提案する
```

これにより、バックエンド分岐が必要な箇所は`builtin`一箇所に集約され、ユーザーが書く独自部品のコードベースにはバックエンド分岐が一切現れない状態を維持できる。

## G.4 描画コードのRustファイル分離

`on_paint`のようなコールバックは`on_click`と同じく、`.elwind`側は関数参照のみを持ち、実装は通常のRustファイルに分離する。

```rust
// src/painters/volume_meter.rs (通常のRustファイル、.elwindの外)
use elwindui::painter::{Painter, Rect, Color};

pub fn draw_meter(p: &mut Painter, level: f64) {
    p.fill_rect(Rect::new(0.0, 0.0, 200.0, 40.0), Color::hex("#eeeeee"));
    p.fill_rect(Rect::new(0.0, 0.0, 200.0 * level, 40.0), Color::hex("#2ecc71"));
    p.stroke_rect(Rect::new(0.0, 0.0, 200.0, 40.0), Color::hex("#999999"), 1.0);
}
```

**推奨ディレクトリ構成:**

```
src/
├── ui/                       # .elwind本体(レイアウト定義)
│   ├── notepad_window.elwind
│   └── volume_meter.elwind
├── painters/                 # 描画ロジック(通常のRust、バックエンド共通実装)
│   ├── volume_meter.rs
│   ├── knob.rs
│   └── mod.rs
└── logic/                    # on_click等の業務ロジック
    └── document.rs
```

`Painter`が既にバックエンド差異を吸収しているため、`painters/*.rs`は原則1ファイル1実装で全バックエンドに対応できる。`use painters::volume_meter::draw_meter;` は12章の`use`構文をそのまま使い、参照先が`.elwind`か`.rs`かはパスからコンパイラが自動判別する。

`Painter`で表現しきれないネイティブ専用描画がどうしても必要な場合のみ、`painters/<name>/`をディレクトリ化しRust標準の`#[cfg(feature = "backend-...")]`で分岐する。これは`.elwind`の文法ではなく通常のRustコード側の関心事であるため、`target::backend()`ではなくRust標準のcfg機構を使う。

## G.5 再描画のトリガーとアニメーション

`Canvas`の`prop`(例:`level`)が変わったら、通常の`prop`更新の仕組み(4章)と同じルールで再描画がトリガーされる。

毎フレーム再描画したいアニメーションの場合は`#[animated]`を付け、propの変化を待たず常に再描画対象にすることを明示する。

```rust
Canvas {
    #[animated]
    on_paint: draw_spinner(painter, elapsed_time())
}
```

`#[animated]`が付いた`on_paint`内でのみ、`elapsed_time()`のような非純粋関数の呼び出しが許可される(14章ルール2の例外)。

## G.6 インタラクション(クリック・ドラッグ)

```rust
component DraggableKnob {
    #[range(0..=1)]
    value: f64,
}

view DraggableKnob {
    Canvas {
        width: 60
        height: 60
        on_paint: draw_knob(painter, value)
        on_pointer_down: |pos| start_drag(pos)
        on_pointer_move: |pos| value = knob_value_from_pos(pos)
    }
}
```

座標系はDPIスケーリングを吸収した論理ピクセル座標に統一し、`Painter`実装側がバックエンドごとの実ピクセル変換を担う。

## G.7 混載部品(既存部品 + Canvas)

`Row`/`Column`/`Text`/`Button`のようなビルトイン部品と`Canvas`は、同じ`view`ブロック内で自由に混在できる。G.3のルールがそのまま効くため、混載していても全体がバックエンド共通のまま保たれる。

```rust
use painters::knob::draw_knob;

component VolumeSlider {
    #[range(0..=1)]
    value: f64,

    #[computed]
    percent_label: String = format!("{}%", (value * 100.0) as i32),
}

view VolumeSlider {
    Row {
        spacing: 12

        Text { text: t!("volume-label") }

        Canvas {
            width: 60
            height: 60
            on_paint: draw_knob(painter, value)
            on_pointer_move: |pos| value = knob_value_from_pos(pos)
            #[accessible(role: Slider, label: t!("a11y-volume"), value: percent_label)]
        }

        Text { text: percent_label }

        Button {
            text: t!("volume-mute")
            on_click: value = 0.0
        }
    }
}
```

混載が問題なく成立する理由:

| 仕組み | 混載を支えている理由 |
|---|---|
| `Element`トレイト(13章) | `Canvas`も`Row`も同じ`Element`として扱われ、ツリー上の位置づけに差がない |
| `LayoutNode`(付録H) | `Canvas`は「指定サイズを占有するノード」として他の部品と同じレイアウト計算に参加する |
| `Painter`抽象(本付録) | `Canvas`内部の描画がバックエンド非依存なので、混載してもバックエンド分岐が漏れ出さない |
| G.3のバックエンド分岐禁止ルール | 混載した`view`全体を見てもnative!が現れないため、静的検証にそのまま合格する |
| `#[accessible(...)]`推奨(付録H) | `Canvas`部分だけ明示的なアクセシビリティ情報が必要という区別が保たれ、混載時も漏れなく検証できる |

## G.8 まとめ

| 要件 | 対応 |
|---|---|
| グラフ・ゲージ等の独自描画 | `Canvas` + `on_paint: fn(&mut Painter)` |
| バックエンド間の描画API差異の吸収 | `Painter`トレイトと各backendのラッパー実装(`builtin::Canvas`内部のみ) |
| 独自部品はバックエンド共通実装に限定 | `native!`/`target::backend()`の使用を通常のcomponentでは静的エラーとする(14章ルール9) |
| propに連動した再描画 | 既存の`prop`更新ルール(4章)をそのまま流用 |
| 常時アニメーションさせたい | `#[animated]`で毎フレーム再描画対象と明示、非純粋関数呼び出しを許可 |
| クリック・ドラッグ等の入力 | `on_pointer_down`/`on_pointer_move`等のコールバックをCanvasに追加 |
| 描画コードの分離 | `painters/*.rs`にバックエンド共通実装を配置、`use`で参照 |
| 既存部品との混載 | 同じ`Element`ツリー・`LayoutNode`として自然に共存可能 |

---

# 付録H. コアランタイム(レイアウト・フォーカス・アクセシビリティ)

Button/Textのような個別ウィジェットの抽象化(付録F・G)とは別レイヤーとして、WinUI 3の`Composition`/`UIAutomation`/`Measure-Arrange`に相当する共通基盤を`elwindui-core`として定義し、各バックエンドがこれを実装する。

## H.1 全体構造

```
.elwind (component/view)
        │
        ▼
Element ツリー(13章で定義済み)
        │
        ▼
┌─────────────────────────────────────────┐
│ ElwindUIL Core Runtime(elwindui-core)      │
│  ├─ LayoutEngine      (制約ベースのMeasure/Arrange) │
│  ├─ FocusManager      (フォーカス移動・トラップ)     │
│  ├─ AccessibilityTree (UIAツリー相当)              │
│  ├─ InputRouter       (ヒットテスト・イベント配送)   │
│  └─ Painter           (付録G参照)                  │
└─────────────────────────────────────────┘
        │
        ▼
各バックエンド実装(WinUI3/AppKit/GTK4/egui/iced)
```

ネイティブ系バックエンドはOS標準機構に極力委譲し、egui/icedのような非ネイティブ系バックエンドはCore Runtimeの共通実装(または`accesskit`のような橋渡しクレート)に依存する。

## H.2 レイアウトエンジン

WinUI3の`Measure`/`Arrange`2パス方式を採用する。

```rust
trait LayoutNode {
    fn measure(&self, available: Size) -> Size;
    fn arrange(&mut self, final_rect: Rect);
}
```

- `Stack`(付録F.2)や`Canvas`(付録G)を含む全ビルトインがこのトレイトを実装する
- `.elwind`側では既存の`width`/`height`/`spacing`等の属性がそのままMeasure/Arrangeの入力になり、新しい構文は不要
- レイアウト計算自体は`elwindui-core`内の共通実装(1つのRustクレート)で行い、バックエンドは計算結果(確定した矩形座標)を受け取ってネイティブAPIに反映するだけ、という役割分担にする

| バックエンド | レイアウト計算の主体 |
|---|---|
| egui / iced | Core Runtimeの共通計算をそのまま使う |
| WinUI3 | Core Runtimeで計算 → 結果を絶対配置コンテナに反映 |
| AppKit / GTK4 | 同様にCore Runtimeの計算結果を`NSView.frame`/`gtk_widget_size_allocate`に反映 |

この一元化により、全バックエンドで同一のレイアウト結果が保証される。

## H.3 フォーカス管理

```rust
trait FocusManager {
    fn move_focus(&mut self, direction: FocusDirection) -> Option<ElementId>;
    fn set_focus(&mut self, id: ElementId);
    fn focused(&self) -> Option<ElementId>;
    fn trap_focus(&mut self, scope: ElementId);
}

enum FocusDirection { Next, Previous, Up, Down, Left, Right }
```

**`.elwind`側の属性:**

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

Tab移動順序(`order`)や方向キー移動はCore Runtimeが共通ロジックとして提供する。ネイティブ系バックエンドはOS標準のフォーカス機構(WinUI3の`FocusManager`、AppKitの`NSResponder`チェーン、GTK4の`gtk_widget_grab_focus`)に結果を同期する(Core Runtime側を正、OS側はミラーとする)。

## H.4 アクセシビリティ

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

- ビルトイン部品(`Button`,`TextArea`等)は既定のroleを自動付与するため、通常は追加記述不要
- `Canvas`ベースの独自部品は意味情報を持たない矩形描画の集合でしかないため、`#[accessible(...)]`での明示を推奨する。付けない場合は14章ルール10により静的警告となる

**バックエンド実装義務:**

| バックエンド | 実装方法 |
|---|---|
| WinUI3 | `AutomationPeer`を生成し、Windows UI Automationに登録 |
| AppKit | `NSAccessibilityElement`プロトコルを実装 |
| GTK4 | `Atk`/AT-SPIブリッジに登録 |
| egui / iced | `accesskit`クレート経由でOSに直接ツリーを流し込む |

## H.5 Core Runtimeの位置づけ(クレート構成)

```
elwindui-core           # Element, LayoutEngine, FocusManager, AccessibilityTree, InputRouter, Painter(共通・バックエンド非依存)
elwindui-backend-winui3 # elwindui-coreを実装 + windows-rsでネイティブAPIに橋渡し
elwindui-backend-appkit # 同上、objc2経由
elwindui-backend-gtk4   # 同上、gtk-rs経由
elwindui-backend-egui   # 同上 + accesskitでa11y補完
elwindui-backend-iced   # 同上 + accesskitでa11y補完
```

`.elwind`コンパイラが生成するコードは常に`elwindui-core`のトレイト境界に対して書かれ、実行時にどのバックエンドクレートがリンクされるかで実体が決まる(付録D`target::backend()`と対応)。

## H.6 まとめ

| 要件 | 対応 |
|---|---|
| フォーカス管理の共通化 | `FocusManager`トレイト + `#[focus(order/trap)]`属性、ネイティブ系はOS機構とミラー同期 |
| アクセシビリティの共通化 | `AccessibilityNode`トレイト + `#[accessible(role/label/state)]`属性、egui/iced系は`accesskit`で補完 |
| レイアウト計算の共通化 | `LayoutNode`(Measure/Arrange)を`elwindui-core`で一元計算し、バックエンド間の見た目のズレを防止 |
| WinUI3ライクな基盤全体 | `elwindui-core`という共通クレートに集約し、各バックエンドがこれを実装する構成 |
| 独自部品(付録G)との整合 | `Canvas`ベースの部品は`#[accessible(...)]`の明示を推奨(付けない場合は静的警告) |

---

# 付録I. ライフサイクルフック

コンポーネントの生成時・破棄時・更新後に副作用のあるコードを挟むための仕組み。`view`ブロック内の先頭に宣言する。

## I.1 `on_mount` / `on_unmount`

```rust
view NotepadWindow {
    on_mount: {
        load_last_document();
    }

    on_unmount: {
        save_draft();
    }

    Window { ... }
}
```

- `on_mount` はコンポーネントが要素ツリーに初めて組み込まれた直後に**一度だけ**実行される
- `on_unmount` はツリーから除去される直前に**一度だけ**実行される
- どちらも通常のRustコードブロックであり、副作用のある処理(ファイルI/O、非同期タスクの起動等)を書いてよい。`#[param]`の静的評価式(4章)とは異なる実行コンテキストのため、非純粋関数の呼び出し制限は適用されない

## I.2 `on_update`:特定propの変化を監視

```rust
view NotepadWindow {
    on_update(content): {
        state = SaveState::Unsaved;
    }

    on_update(encoding): {
        reload_with_encoding(encoding);
    }

    Window { ... }
}
```

- `on_update(field_name): { ... }` は、指定した`prop`(または`#[computed]`)が変化するたびに実行される
- 複数フィールドを監視する場合は`on_update(a, b): { ... }`のようにカンマ区切りで列挙する(いずれかが変化した時点で発火)
- 引数を指定しない`on_update: { ... }`は、そのコンポーネントの**任意の`prop`変化**で発火する(頻度が高くなるため濫用しない、というガイドラインを併記する)

## I.3 制約:`#[param]`の不変性はライフサイクル全体で保証される

`on_mount`/`on_update`内であっても、`#[param]`フィールドへの代入は禁止する(14章ルール11)。`#[param]`は「実体化時のみ確定・以後不変」という4章の原則を、ライフサイクルフックの内側でも一貫して守る。

```rust
on_mount: {
    orientation = Orientation::Vertical   // エラー: #[param]フィールドは生涯不変
}
```

## I.4 `Element`トレイトとの関係

`on_mount`/`on_unmount`は13章で定義した`Element`トレイトの生成・破棄タイミングに対応する。コード生成器は各バックエンドのライフサイクル(WinUI3の`Loaded`/`Unloaded`イベント、AppKitの`viewDidAppear`/`viewWillDisappear`、GTK4の`realize`/`unrealize`、egui/icedでは初回フレーム検出/明示的破棄)にこれらのフックをマッピングする。この変換自体はビルトイン側の責務であり、通常の`component`では意識する必要はない。

## I.5 まとめ

| 要件 | 対応 |
|---|---|
| 生成時に一度だけ処理を実行 | `on_mount: { ... }` |
| 破棄時に一度だけ処理を実行 | `on_unmount: { ... }` |
| 特定propの変化を監視 | `on_update(field): { ... }` |
| 副作用・非純粋関数の許可 | ライフサイクルフック内は`#[param]`式とは別の実行コンテキストとして許可 |
| `#[param]`不変性の一貫性 | フック内での`#[param]`代入は静的エラー(14章ルール11) |

---

# 付録J. グローバル状態(Store)定義

これまで`bind!(settings.volume, TwoWay)`のように暗黙の存在として扱ってきた`settings`を、`store`という専用構文で明示的に定義する。

## J.1 `store`の定義

```rust
store AppSettings {
    #[range(0..=100)]
    volume: i32 = 50,

    theme: ThemeMode = ThemeMode::Auto,

    #[persist]
    recent_files: Vec<String> = [],
}
```

- `store`は`component`と似た構文だが`view`を持たない。**状態のみを保持する共有可能なデータ定義**である
- フィールドの型・制約(`#[range]`, `#[pattern]`等、7章)は`component`のprop定義と同じ書き方を使う
- `#[persist]`が付いたフィールドはアプリ終了後もディスクに永続化される(実際の永続化方式—ファイル/レジストリ/UserDefaults等—はバックエンドの責務)

## J.2 Storeの参照

```rust
use stores::app_settings::AppSettings;

component VolumeControl {
    prop volume: i32[0..=100] = bind!(AppSettings.volume, TwoWay),
}
```

- `bind!`の参照先は`store`型の完全修飾フィールドパス(`AppSettings.volume`)とする
- 12章の`use`構文でstore定義をインポートする
- `AppSettings`は既定でアプリ全体において**単一のシングルトンインスタンス**として扱われる(複数インスタンスが必要な場合はJ.5参照)

## J.3 Storeの変更はアプリ全体に伝播する

```rust
fn reset_volume() {
    AppSettings.volume = 0;   // 通常のRustロジック側からの変更
}
```

- `store`のフィールドはプレーンなRust構造体のフィールドとして扱われ、通常のロジック関数から直接代入できる
- 変更は`bind!`で購読している全ての`prop`に伝播し、それぞれの差分更新ルール(4章)に従って反映される

## J.4 制約:storeへのアクセスは常に`bind!`を経由する

```rust
component Bad {
    #[param]
    initial_volume: i32 = AppSettings.volume,   // エラー: #[param]はstoreを直接参照できない
}
```

- `#[param]`は静的評価式のみを許可する原則(4章)と一致させ、storeのような実行時に変化しうる値は必ず`prop`側で`bind!`を介して取り込む(14章ルール13)
- これにより「`#[param]`は本当に実体化時に確定し以後変化しない」という保証が、store導入後も揺らがない

## J.5 スコープ付きStore(シングルトンでない場合)

ドキュメント単位・ウィンドウ単位など、シングルトンではなく複数インスタンスを持たせたいstoreは`#[scoped]`を付けて宣言し、コンポーネントの`#[param]`経由で注入する。

```rust
#[scoped]
store DocumentState {
    #[length(0..=100000)]
    text: String = "",
    dirty: bool = false,
}
```

```rust
component NotepadWindow {
    #[param]
    #[inject]
    doc: DocumentState,

    content: String = bind!(doc.text, TwoWay),
}
```

- `#[inject]`が付いた`#[param]`フィールドは、呼び出し側がコンポーネント生成時に具体的な`DocumentState`インスタンスを渡す(複数のメモ帳ウィンドウをそれぞれ別ファイルにバインドする、といった用途に対応する)

## J.6 まとめ

| 要件 | 対応 |
|---|---|
| グローバル状態の明示的定義 | `store Name { ... }`(型・制約構文はcomponentと共通) |
| 永続化 | `#[persist]`アトリビュート |
| 状態の参照 | `bind!(StoreName.field, mode)` |
| paramとの整合性維持 | storeへの直接参照は`#[param]`で禁止、常に`prop`+`bind!`経由(14章ルール13) |
| 単一インスタンスでない場合 | `#[scoped]` store + `#[inject]` paramによる注入 |

---

# 付録K. キーボード入力・ショートカット

ポインタ系イベント(`on_pointer_down`等、付録G.6)に加え、キーボード入力・IME・アプリ全体のショートカットを扱うための構文。

## K.1 要素単位のキーイベント

```rust
TextArea {
    text: content
    on_key_down: |key| handle_key(key)
    on_text_input: |text| handle_ime_commit(text)
}
```

- `on_key_down` / `on_key_up` — 物理キーの押下・離上(修飾キー状態を含む`Key`型を受け取る)
- `on_text_input` — IME確定後の実文字列、または直接入力の文字を受け取る(IME変換中の未確定文字列はバックエンドが内部で処理し、DSL側には確定結果のみが渡る)

これらのイベントを受け取るには、当該要素がフォーカスを持っている必要がある(付録H.3の`FocusManager`と連動する)。

## K.2 グローバルショートカット

```rust
Button {
    text: t!("notepad-menu-save")
    #[shortcut("Ctrl+S")]
    on_click: save_document()
}
```

- `#[shortcut("...")]`はプラットフォーム非依存の修飾キー表記(`Ctrl`/`Shift`/`Alt`/`Meta`)を使う
- コード生成時に、macOS向けビルドでは`Ctrl`が自動的に`Cmd`に読み替えられる(WinUI3/GTK4/egui/iced等の他backendではそのまま`Ctrl`として扱う)、というプラットフォーム変換規則を標準で持つ
- 明示的にOSごとの割り当てを変えたい場合は複数指定できる

```rust
#[shortcut(winui3: "Ctrl+S", appkit: "Cmd+S")]
on_click: save_document()
```

## K.3 フォーカス外からのグローバルショートカット

`#[shortcut(...)]`が付いた要素は、既定では**その要素がフォーカスされていなくてもアプリウィンドウ内であれば発火する**(メニューショートカットと同じ扱い)。要素にフォーカスがある場合のみ発火させたい場合は`scope: local`を指定する。

```rust
#[shortcut("Ctrl+F", scope: local)]
on_key_down: |_| find_in_selection()
```

## K.4 まとめ

| 要件 | 対応 |
|---|---|
| キー押下・離上の検知 | `on_key_down` / `on_key_up` |
| IME確定後の文字入力 | `on_text_input` |
| アプリ全体のショートカット | `#[shortcut("Ctrl+S")]`、OS別修飾キー読み替えは標準で自動変換 |
| ショートカットの発火範囲 | 既定はウィンドウ全体、`scope: local`でフォーカス時のみに限定 |

---

# 付録L. 画面遷移(ナビゲーション)

複数画面を持つアプリのための、ルート(画面種別)ベースのナビゲーション機構。`NavigationHost`はビルトインとして提供される。

## L.1 ルート定義

```rust
enum Route {
    Main,
    Settings,
    Search,
}
```

## L.2 `NavigationHost`

```rust
component App {
    #[param]
    current_route: Route = Route::Main,
}

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

- `match current_route { ... }` は`Route` enumの全メンバーを網羅していなければ静的エラーになる(8章の網羅性検査、14章ルール14)
- `NavigationHost`はビルトインのため、内部で`match target::backend()`によるバックエンド別実装を持つ(付録G.3の原則通り、通常のcomponentではこの分岐は書けない)

| バックエンド | 実装 |
|---|---|
| WinUI3 | `Microsoft::UI::Xaml::Controls::Frame`によるページ遷移 |
| AppKit | `NSWindow`の`contentViewController`差し替え、またはシート/ウィンドウ切り替え |
| GTK4 | `gtk::Stack` + `gtk::StackTransitionType` |
| egui / iced | 内部状態による表示切り替え(単一ウィンドウ内でツリーの入れ替え) |

## L.3 遷移操作

```rust
fn open_settings() {
    navigate!(Route::Settings);
}

fn go_back() {
    navigate_back!();
}
```

- `navigate!(route)` — 指定ルートへ遷移し、遷移履歴に積む
- `navigate_back!()` — 履歴を1つ戻す(履歴が空の場合は何もしない)
- これらはマクロ呼び出し形式(10章の`bind!`と同じ慣習)であり、`NavigationHost`の内部履歴スタックを操作する

## L.4 まとめ

| 要件 | 対応 |
|---|---|
| 複数画面の定義 | `enum Route { ... }` |
| ルートに応じた画面切り替え | `NavigationHost { route, match route { ... } }` |
| 遷移漏れの検出 | `match`の網羅性検査(14章ルール14) |
| 遷移操作 | `navigate!(route)` / `navigate_back!()` |
| バックエンドごとの実装差 | `NavigationHost`内部にのみバックエンド分岐を許可(付録G.3の原則を維持) |

---

# 付録M. ダイアログ・ポップアップ・メニュー

メインウィンドウの外に浮く一時的なUI(モーダルダイアログ、コンテキストメニュー、ツールチップ)のためのビルトイン部品。

## M.1 `Dialog`(モーダル)

```rust
component App {
    #[param]
    show_settings: bool = false,
}

view App {
    NotepadWindow { }

    if show_settings {
        Dialog {
            title: t!("settings-title")
            on_dismiss: show_settings = false

            SettingsPanel { }
        }
    }
}
```

- `Dialog`はビルトインで、`#[focus(trap: true)]`(付録H.3)が自動的に適用される。ダイアログ表示中はTabキーによるフォーカス移動がダイアログ内に閉じ込められる
- `on_dismiss`はEscキー押下・ダイアログ外クリック(モードレス的操作)・明示的な閉じるボタンいずれからも発火する共通コールバック

| バックエンド | 実装 |
|---|---|
| WinUI3 | `Microsoft::UI::Xaml::Controls::ContentDialog` |
| AppKit | `NSAlert`またはシート(`beginSheet`) |
| GTK4 | `gtk::Dialog` |
| egui / iced | 半透明オーバーレイ上に浮かせた`Window`/`Modal`表現 |

## M.2 `Menu` / `MenuItem`(コンテキストメニュー)

```rust
Menu {
    for item in [
        MenuItem { text: t!("menu-cut"), on_select: cut() },
        MenuItem { text: t!("menu-copy"), on_select: copy() },
        MenuItem { text: t!("menu-paste"), on_select: paste() },
    ] {
        item
    }
}
```

```rust
TextArea {
    text: content
    context_menu: Menu { ... }   // 右クリックで表示するメニューを紐付ける
}
```

## M.3 `Tooltip`

```rust
Button {
    text: t!("notepad-menu-save")
    tooltip: t!("tooltip-save")
    on_click: save_document()
}
```

- `tooltip`は任意のビルトイン要素が持てる共通属性として提供し、ホバー時に各OS標準のツールチップ表示を行う

## M.4 制約の継承

`Dialog`/`Menu`/`Tooltip`はいずれもビルトインであり、内部で`match target::backend()`を持つ。通常の`component`側でこれらを利用する際は、他のビルトイン同様バックエンド分岐を意識する必要はなく、独自部品からこれらを組み合わせて使う場合もG.3の「バックエンド分岐禁止」原則がそのまま適用される(14章ルール15)。

## M.5 まとめ

| 要件 | 対応 |
|---|---|
| モーダルダイアログ | `Dialog { on_dismiss, ... }`、フォーカストラップを自動適用 |
| コンテキストメニュー | `Menu` / `MenuItem`、`context_menu`属性での紐付け |
| ツールチップ | 任意要素が持てる共通属性`tooltip` |
| バックエンドごとの実装差 | ビルトイン内部にのみ分岐を許可し、独自部品からの利用時は分岐禁止原則を維持(14章ルール15) |

---

# 付録N. 描画機能の拡張(Composition相当のビジュアル効果)

付録G.2で定義した`Painter`は塗り・線・テキストの最小セットのみだった。ここでは**WinUI3の`Win2D`(即時描画)と`Composition`(合成レイヤー)に相当する機能一式**を`Painter`の拡張として定義し、グラデーション・シャドウ・ぼかし・変形・アニメーション・レイヤー合成をカバーする。

## N.1 ブラシ(Brush)

塗り潰し・線を「単色」以外でも表現できるよう、`Color`単体ではなく`Brush`型を受け取れるようにする。

```rust
enum Brush {
    Solid(Color),
    LinearGradient { stops: Vec<GradientStop>, start: Point, end: Point },
    RadialGradient { stops: Vec<GradientStop>, center: Point, radius: f32 },
    Image { image: Image, tile: TileMode },
    Acrylic { tint: Color, tint_opacity: f32, blur_amount: f32 },   // WinUI3のAcrylic素材相当
}

struct GradientStop { offset: f32[0.0..=1.0], color: Color }
```

```rust
trait Painter {
    // ...(G.2の基本メソッドに加えて)
    fn fill_rect_brush(&mut self, rect: Rect, brush: &Brush);
    fn stroke_path_brush(&mut self, path: &Path, brush: &Brush, width: f32);
}
```

| Brush種別 | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| `LinearGradient` | `LinearGradientBrush` | `CGGradient` + `drawLinearGradient` | Cairo `LinearGradient` | `egui::Mesh`によるグラデーション三角形 |
| `Acrylic` | `AcrylicBrush`(ネイティブサポート) | `NSVisualEffectView`重畳で近似 | 非対応(単色フォールバック、17番ルールで警告) | 非対応(単色フォールバック) |

## N.2 図形・パス(Geometry)

ベジエ曲線・弧を含む複雑な形状と、線のスタイル(端点・接合・破線)を定義する。

```rust
struct Path {
    // move_to, line_to, cubic_bezier_to, quadratic_bezier_to, arc_to, close を組み立てて構築
}

struct StrokeStyle {
    cap: LineCap,       // Butt, Round, Square
    join: LineJoin,     // Miter, Round, Bevel
    dash: Vec<f32>,     // 破線パターン(空なら実線)
}
```

```rust
fn draw_meter(p: &mut Painter, level: f64) {
    let mut path = Path::new();
    path.move_to(Point::new(0.0, 20.0));
    path.cubic_bezier_to(Point::new(50.0, 0.0), Point::new(150.0, 40.0), Point::new(200.0, 20.0));
    p.stroke_path_brush(&path, &Brush::Solid(Color::hex("#2ecc71")), 2.0);
}
```

## N.3 エフェクト(Effect)

シャドウ・ぼかし・色調補正など、要素単位で適用する視覚効果。

```rust
enum Effect {
    DropShadow { offset: Point, blur_radius: f32, color: Color },
    Blur { radius: f32 },
    ColorMatrix { matrix: [f32; 20] },   // 彩度・色相調整等
    Opacity { value: f32[0.0..=1.0] },
}
```

```rust
Canvas {
    width: 200
    height: 40
    #[effect(DropShadow { offset: Point::new(0.0, 2.0), blur_radius: 4.0, color: Color::hex("#00000040") })]
    on_paint: draw_meter(painter, level)
}
```

| Effect種別 | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| `DropShadow` | `Compositor.CreateDropShadow` | `CALayer.shadowOffset/shadowRadius` | Cairo手動合成 | 手動でオフセット矩形を追加描画して近似 |
| `Blur` | `GaussianBlurEffect`(Win2D) | `CIGaussianBlur` | 非対応(17番ルールで警告、フォールバックはブラー無し) | 非対応(同上) |

`#[effect(...)]`が付与された`Canvas`は、内部で`Painter`が返す描画結果をオフスクリーンサーフェスに一度レンダリングしてからエフェクトを適用する(N.5のレイヤー合成の仕組みを利用する)。

## N.4 トランスフォーム(Transform)

```rust
enum Transform {
    Translate(f32, f32),
    Rotate(f32),          // ラジアン
    Scale(f32, f32),
    Skew(f32, f32),
    Matrix([f32; 6]),      // アフィン変換行列
}
```

```rust
trait Painter {
    // ...
    fn push_transform(&mut self, transform: Transform);
    fn pop_transform(&mut self);
}
```

```rust
fn draw_knob(p: &mut Painter, value: f64) {
    p.push_transform(Transform::Rotate((value * std::f64::consts::TAU) as f32));
    p.draw_line(Point::ORIGIN, Point::new(0.0, -20.0), Color::hex("#2ecc71"), 3.0);
    p.pop_transform();
}
```

- `push_transform`/`pop_transform`はスタック方式(SVG/Canvas 2D APIと同じ慣習)とし、ネストした変形を自然に表現できる

## N.5 レイヤー合成(オフスクリーンサーフェス・不透明度・ブレンドモード)

```rust
trait Painter {
    // ...
    fn begin_layer(&mut self, opacity: f32[0.0..=1.0], blend_mode: BlendMode);
    fn end_layer(&mut self);
    fn clip_rect(&mut self, rect: Rect);
    fn clip_path(&mut self, path: &Path);
}

enum BlendMode { Normal, Multiply, Screen, Overlay, Darken, Lighten }
```

- `begin_layer`/`end_layer`は一時的なオフスクリーンサーフェスへの描画を開始・確定する(WinUI3の`Compositor.CreateContainerVisual`相当)。N.3のエフェクトはこの仕組みの上に実装される
- `clip_rect`/`clip_path`は以降の描画を指定領域内にクリップする(スタック方式でネスト可能)

| 機能 | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| レイヤー合成 | `ContainerVisual` + `CompositionEffectBrush` | `CALayer`の階層合成 | Cairoの`push_group`/`pop_group` | オフスクリーン`egui::Painter`への描画 → テクスチャ合成 |
| クリップ | `Visual.Clip` | `CGContextClip` | `cairo_clip` | `egui::Painter::with_clip_rect` |

## N.6 アニメーション(Transition / KeyframeAnimation)

WinUI3の`Composition`が提供する「暗黙アニメーション(値が変わると自動的に補間される)」と「明示的なキーフレームアニメーション」の両方を用意する。

**暗黙アニメーション(prop変化に自動追従):**

```rust
component ProgressBar {
    #[range(0..=1)]
    #[transition(duration: 200ms, easing: EaseOutCubic)]
    value: f64,
}
```

- `#[transition(duration, easing)]`が付いた`prop`は、値が変化した際にUIが自動的に指定時間・イージング関数で補間描画される。`Canvas`の`on_paint`やビルトイン部品(`ProgressBar`等)の内部実装が、この補間後の中間値を使って再描画する
- `easing`には標準イージング関数(`Linear`, `EaseIn/EaseOut/EaseInOutCubic`, `EaseOutBack`, `Spring { stiffness, damping }`等)を指定する。存在しない名前は14章ルール16により静的エラー

**明示的なキーフレームアニメーション(Canvas内での手続き的制御):**

```rust
Canvas {
    #[animated]
    on_paint: |p| {
        let t = KeyframeAnimation::new()
            .add(0.0, 0.0)
            .add(0.5, 1.2)
            .add(1.0, 1.0)
            .easing(Easing::EaseOutBack)
            .sample(elapsed_time());
        draw_bouncing_icon(p, t);
    }
}
```

- `KeyframeAnimation`は0.0〜1.0の正規化時刻に対する値を複数指定し、`sample(t)`で任意時刻の補間値を取得する。キーフレーム位置が範囲外の場合は14章ルール16によりエラー

## N.7 リッチテキスト描画

```rust
struct TextRun {
    text: String,
    font: Font,
    color: Brush,
    weight: FontWeight,
}

trait Painter {
    // ...
    fn draw_rich_text(&mut self, runs: &[TextRun], layout_rect: Rect, align: TextAlign, wrap: WrapMode);
}
```

- 複数の書式(太字強調・色分け等)が混在するテキストを1回の描画呼び出しでレイアウトできる(WinUI3の`CanvasTextLayout`/AppKitの`NSAttributedString`相当)

## N.8 まとめ

| WinUI3相当の機能 | ElwindUILでの対応 |
|---|---|
| ブラシ(単色/グラデーション/画像/Acrylic) | `Brush` enum + `fill_rect_brush`/`stroke_path_brush` |
| ジオメトリ(ベジエ・弧・線スタイル) | `Path` + `StrokeStyle`(cap/join/dash) |
| エフェクト(シャドウ・ブラー・色調補正) | `Effect` enum + `#[effect(...)]`属性 |
| 変形(移動・回転・拡縮・スキュー) | `Transform` enum + `push_transform`/`pop_transform` |
| Composition(レイヤー合成・クリップ・ブレンド) | `begin_layer`/`end_layer`/`clip_rect`/`clip_path`/`BlendMode` |
| 暗黙アニメーション | `#[transition(duration, easing)]` |
| キーフレームアニメーション | `KeyframeAnimation`(`Canvas`内で手続き的に使用) |
| リッチテキスト | `TextRun` + `draw_rich_text` |

いずれもG.2で定義した`Painter`トレイトの拡張メソッド・付随データ型として`elwindui-core`(付録H)に属し、バックエンドごとの実装差はG.3の原則通り`builtin::Canvas`内部にのみ許可される。GTK4のように一部エフェクト(Acrylic/Blur)が未対応のバックエンドでは、静的警告(14章ルール17)とともに単色/効果無しへのフォールバック描画が行われる。

---

# 付録O. MVVM対応

WinUI3/WPF由来のMVVM(Model-View-ViewModel)パターンをElwindUILに導入する。Rustの所有権モデルはC#のようなイベントデリゲート主体のMVVM実装と相性が悪いため、**新しい実行時機構を作らず、既存の`#[computed]`(4章、静的依存関係抽出)と`store`(付録J)の仕組みを再利用する**ことで、動的ディスパッチや参照カウント地獄を避けた低オーバーヘッドな実装にする。

## O.1 設計方針:M/V/VMの対応関係

| MVVMの層 | ElwindUILでの対応 |
|---|---|
| Model | 通常のRust構造体、または`store`(付録J、アプリ全体の永続的データ) |
| ViewModel | 本付録で定義する`viewmodel`(Viewに紐づく表示用データ+操作) |
| View | 既存の`component`/`view`(3章)。ViewModelを`#[inject]`で受け取り、表示のみを担当する |

Viewは業務ロジックを一切持たない、という制約は既にG.3(独自部品はバックエンド共通実装限定)やI章(ライフサイクル)で部分的に担保されているが、本付録では**ViewModelに業務ロジック・Commandを集約し、Viewは常にViewModelを介してのみ状態にアクセスする**という設計を明文化する。

## O.2 `viewmodel`の定義

```rust
viewmodel NotepadViewModel {
    #[observable]
    #[length(0..=100000)]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",

    #[observable]
    state: SaveState = SaveState::Unsaved,

    #[computed]
    char_count: i32 = content.chars().count() as i32,

    #[computed]
    window_title: String = t!("notepad-window-title", file_name: file_name),

    #[command(can_execute: state != SaveState::Saving)]
    save: Command = command!(|| {
        state = SaveState::Saving;
        document::save(&content);
        state = SaveState::Saved;
    }),

    #[command]
    open: Command = command!(|| {
        content = document::open_dialog();
        state = SaveState::Unsaved;
    }),
}
```

- `viewmodel`は`store`(付録J)と同じフィールド構文(型・制約・`#[computed]`)を再利用する。**新しい式構文は導入しない**
- `#[observable]`は`prop`に相当する「実行時に変化しView側へ伝播する」フィールドを表す修飾子(既定でstoreのフィールドと同じ扱い)
- `viewmodel`は`view`ブロックを持てない。ビルトイン要素(`Row`/`Text`等)への参照が内部に出現すると14章ルール19により静的エラーとなり、M/V/VMの分離が構文レベルで強制される

## O.3 Command(操作の抽象化)

WPF/WinUI3の`ICommand`に相当する型を導入する。

```rust
struct Command {
    can_execute: bool,   // 内部的には#[computed]と同じ仕組みで再評価される
}

impl Command {
    fn execute(&self);
}
```

```rust
#[command(can_execute: state != SaveState::Saving)]
save: Command = command!(|| { /* 実行内容 */ }),
```

- `can_execute`式は`#[computed]`(4章)と同じ静的依存関係抽出の対象になる。依存する`#[observable]`フィールド(ここでは`state`)が変化するたびに自動再評価される
- `command!(|| { ... })`マクロは10章の`bind!`と同じ「マクロ呼び出しでロジックを包む」慣習に従う
- View側では`vm.save.execute()`を`on_click`等に渡すだけでよく、`vm.save.can_execute`を`enabled`属性にそのまま渡せば有効/無効の切り替えも自動化される

## O.4 ViewModelとViewの結合

ViewModelはView単位で注入される(付録J.5の`#[scoped]` store + `#[inject]`と同じ仕組みを流用する)。

```rust
component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    // 双方向編集が必要なフィールドは既存のbind!パターンでpropとして写し取る
    content: String = bind!(vm.content, TwoWay),
}

view NotepadWindow {
    Window {
        title: vm.window_title

        Column {
            Row {
                Button {
                    text: t!("notepad-menu-save")
                    on_click: vm.save.execute()
                    enabled: vm.save.can_execute
                }
                Button {
                    text: t!("notepad-menu-open")
                    on_click: vm.open.execute()
                }
            }

            TextArea { text: content }

            StatusBar {
                items: [
                    Text { text: vm.state.label() },
                    Text { text: t!("notepad-status-chars", count: vm.char_count) },
                ]
            }
        }
    }
}
```

- 双方向バインディングが必要なフィールド(`TextArea`の`content`等)は、これまで通り`component`側の`prop`として`bind!(vm.field, TwoWay)`で写し取る(J.2と同一パターン)
- 読み取り専用の表示(`vm.window_title`, `vm.char_count`, `vm.state.label()`)は、`view`式の中で直接参照してよい。これは14章ルール13の対象外である(ルール13は`#[param]`初期化式への直接参照のみを禁止しており、通常の`view`式は元々動的評価が前提のため制限しない)

## O.5 低オーバーヘッドな内部表現

C#のMVVM実装は`INotifyPropertyChanged`イベント+ボックス化されたデリゲートに依存し、実行時のイベント購読・発火コストと動的ディスパッチを伴う。ElwindUILでは以下の方針でこれを避ける。

**1. 依存関係はコンパイル時に静的抽出する(4章の`#[computed]`と同一の仕組み)**

`window_title`が`file_name`に依存する、`char_count`が`content`に依存する、といった関係は`.elwind`のAST解析時点で判明しているため、実行時に依存グラフを構築・走査する必要がない。コード生成器は「`content`が変化したら`char_count`と`command!`の`can_execute`を直接呼び出して再計算する」という**具体的な更新関数を静的に生成**する。これは動的な購読リスト(`Vec<Box<dyn Fn()>>`等)を持たない。

**2. `#[observable]`フィールドは`Cell<T>`/`Copy`前提の生成コードにする**

```rust
// コード生成器が生成する内部表現(擬似)
struct NotepadViewModel {
    content: RefCell<String>,       // 非Copy型はRefCell
    file_name: Cell<StrHandle>,     // 文字列はインターン化しCopyなハンドルで保持する等、実装は自由
    state: Cell<SaveState>,         // Copy型のenumはCellで十分
}
```

- `Copy`可能な型(enum、数値、bool)は`Cell<T>`で保持し、`RefCell`の借用チェックオーバーヘッドすら発生しない
- `Copy`でない型(`String`, `Vec<T>`等)のみ`RefCell`を使う。付録Jのstoreと共通の実装方針である
- ヒープ確保が発生するのはViewModelインスタンス生成時のみで、値の読み書き自体はO(1)のCell操作にとどまる

**3. 動的ディスパッチ(`dyn Trait`)を使わない**

`Command`の`execute`本体(`command!`マクロの中身)は、コード生成時に**具体的なクロージャ型として単相化**される。`Box<dyn Fn()>`のような型消去は行わず、各`viewmodel`ごとに専用の構造体・メソッドが生成される。これにより仮想関数呼び出しのコストが発生しない。

**4. 複雑な相互依存がある場合のフォールバック**

依存関係が動的(実行時にしか確定しない参照パス等)で静的解析が困難なケースに限り、`elwindui-core`が提供する小さな汎用リアクティブグラフ(スロットマップ+世代インデックスによる`SignalId`、Leptos/Xilem系のリアクティブランタイムと同様の設計)にフォールバックする。ただし本付録で示した通常のMVVM用途(observable + computed + command)では、このフォールバックは基本的に発生しない。

## O.6 テスト容易性

`viewmodel`は`view`を持たず、ビルトイン要素にも依存しないため、**バックエンド(WinUI3/AppKit/GTK4/egui/iced)を一切起動せずに単体テストできる**。

```rust
#[test]
fn save_disables_command_while_saving() {
    let vm = NotepadViewModel::new();
    vm.content = "hello".into();
    vm.save.execute();
    assert_eq!(vm.state, SaveState::Saving);
    assert!(!vm.save.can_execute);
}
```

これはMVVMパターン本来の利点(表示ロジックと業務ロジックの分離によるテスト容易性)を、ElwindUILでも通常の`#[test]`だけで実現できることを意味する。

## O.7 `store`(付録J)との関係

| | `store`(付録J) | `viewmodel`(本付録) |
|---|---|---|
| 目的 | アプリ全体で共有される永続的/半永続的データ | 特定のView(画面)のための表示用データと操作 |
| インスタンス | 既定でシングルトン(`#[scoped]`で複数化可) | 常にView単位、`#[inject]`でView生成時に注入 |
| Command(操作) | 持たない。素のRustロジック関数を直接呼ぶ | `Command`型で保持し、`can_execute`込みでViewに公開する |
| 典型的な関係 | ViewModelが内部で`bind!(SomeStore.field, ...)`を使い、Store由来のデータをView向けに`#[computed]`で加工することが多い | - |

## O.8 まとめ

| 要件 | 対応 |
|---|---|
| MVVMのViewModel層 | `viewmodel`構文(`store`と同じフィールド構文を再利用) |
| 操作(Command)の抽象化 | `Command`型 + `#[command(can_execute: ...)]` + `command!(...)` |
| View/ViewModelの結合 | `#[param] #[inject] vm: ViewModelType`(付録J.5の注入パターンを流用) |
| V/VM分離の静的強制 | `viewmodel`内でのビルトイン要素参照・`view`ブロックを静的エラー(14章ルール19) |
| 低オーバーヘッドな実装 | 依存関係の静的抽出(4章と同一)、`Cell`/`RefCell`ベースの内部表現、動的ディスパッチ排除、複雑ケースのみ汎用リアクティブグラフにフォールバック |
| テスト容易性 | `viewmodel`はバックエンド非依存で通常の`#[test]`により単体テスト可能 |

---

# 付録P. 非同期処理

ファイル読込・API呼び出し等の非同期処理と、`prop`/`Command`の連携を定義する。新しい実行モデルは導入せず、既存の`#[computed]`(4章)・`Command`(付録O.3)を非同期版に拡張する形にする。

## P.1 `AsyncState<T>`

```rust
enum AsyncState<T> {
    Idle,
    Loading,
    Success(T),
    Error(String),
}
```

- 通常のenumとして8章の網羅性検査がそのまま適用される
- 非同期処理の結果は必ずこの4状態のいずれかとして扱い、`match`で全状態を処理することを強制する

## P.2 `#[async_computed]`:非同期の算出プロパティ

```rust
viewmodel DocumentViewModel {
    #[observable]
    file_path: String,

    #[async_computed]
    content: AsyncState<String> = task!(async {
        fs::read_to_string(&file_path).await
    }),
}
```

- `#[async_computed]`は`#[computed]`の非同期版。依存する`#[observable]`フィールド(`file_path`)が変化すると自動的に再実行され、実行中は`AsyncState::Loading`になる
- `task!(async { ... })`マクロは`command!`(付録O.3)と同じ「マクロでロジックを包む」慣習に従う
- `#[async_computed]`/`#[command(async, ...)]`が`viewmodel`/`store`以外に付与されている場合は静的エラー(14章ルール20)。非同期状態はVM/Model層に閉じ込め、`component`の`#[param]`静的評価式(4章)を汚染しない

## P.3 View側での扱い

```rust
match vm.content {
    AsyncState::Idle    => Text { text: "" }
    AsyncState::Loading => Spinner {}
    AsyncState::Success(text) => TextArea { text }
    AsyncState::Error(msg)    => Text { text: msg, color: "#e74c3c" }
}
```

- `AsyncState`はenumなので、8章の網羅性検査により状態の処理漏れ(例:`Error`ケースの表示忘れ)が静的に検出される

## P.4 非同期Command

```rust
#[command(async, can_execute: state != SaveState::Saving)]
save: Command = command!(async || {
    state = SaveState::Saving;
    match document::save_async(&content).await {
        Ok(_)  => state = SaveState::Saved,
        Err(e) => { state = SaveState::Unsaved; last_error = Some(e.to_string()); }
    }
}),
```

- `#[command(async, ...)]`は実行中自動的に`can_execute`が`false`扱いになる(多重実行の防止、UI側は`enabled: vm.save.can_execute`をそのまま使えば良い)
- キャンセル可能にしたい場合は`#[command(async, cancellable)]`を付け、`vm.save.cancel()`を呼べるようにする

## P.5 実行基盤

`elwindui-core`(付録H)はホストアプリの非同期ランタイム(tokio/async-std、またはOS標準のディスパッチキュー)を直接指定せず、`spawn(fut)`という薄い抽象を提供する。各バックエンドクレートがこれを実際のランタイムに橋渡しする。

| バックエンド | 橋渡し先 |
|---|---|
| WinUI3 | `DispatcherQueue`経由でUIスレッドに結果を戻す |
| AppKit | `DispatchQueue.main` |
| GTK4 | `glib::MainContext` |
| egui / iced | ホストアプリが持つ`tokio`ランタイム(または`smol`等)に委譲 |

## P.6 まとめ

| 要件 | 対応 |
|---|---|
| 非同期データ取得の状態表現 | `AsyncState<T>`(enum、網羅性検査対象) |
| propの非同期算出 | `#[async_computed]` + `task!(async { ... })` |
| 非同期Command(多重実行防止込み) | `#[command(async, can_execute: ...)]` + `command!(async || { ... })` |
| キャンセル | `#[command(async, cancellable)]` + `vm.command.cancel()` |
| ランタイム統合 | `elwindui-core::spawn`を各バックエンドがホストの非同期基盤に橋渡し |

---

# 付録Q. リスト仮想化

大量データを`for`ループでそのまま描画すると全要素が`Element`として生成され性能が破綻する。表示範囲のみを描画する`VirtualList`ビルトインを提供する。

## Q.1 基本構文

```rust
VirtualList {
    items: documents
    key: |doc| doc.id
    item_height: 32
    render_item: |doc| Row { Text { text: doc.name } }
}
```

- `items` — 全データ(`Vec<T>`)
- `key` — 要素の同一性を判定する関数。リスト順序が変わっても同じ`key`を持つデータは`Element`インスタンスを使い回す(Reactのkey付きリコンサイルと同じ考え方)
- `item_height` — 固定高さの場合はこの値でMeasureパス(付録H.2)をスキップし、表示範囲を定数時間で計算する
- `render_item` — 1件分の`view`を返すクロージャ

## Q.2 可変高さの場合

```rust
VirtualList {
    items: documents
    key: |doc| doc.id
    estimated_item_height: 40
    render_item: |doc| DocumentCard { doc }
}
```

- `estimated_item_height`のみを指定した場合、実際の高さは初回描画時に`LayoutNode::measure`(付録H.2)で計測し、以後はキャッシュして再利用する

## Q.3 要素の再利用(リサイクル)

- スクロールに応じて画面外に出た`Element`インスタンスはすぐに破棄せず、プールに戻して次に表示範囲へ入るデータの描画に再利用する
- 再利用されるインスタンスでは`on_mount`(付録I)は初回プール生成時のみ発火し、以降は`prop`の更新のみが行われる(通常の差分更新、4章)。これによりライフサイクルフックの発火回数を抑えつつ、GUI側の状態(スクロール位置、フォーカス等)を不要に破棄しない

## Q.4 `key`未指定時の挙動

`key`を指定せずに`items`の順序が変わる更新を行うと、挿入位置ベースの再利用にフォールバックし、意図しない要素の使い回し(例:別データなのに同じ`Element`インスタンスが再利用されフォーカス状態が誤って引き継がれる)が起きうる。これを防ぐため、14章ルール23により静的警告を出す。

## Q.5 バックエンド対応

| バックエンド | 実装 |
|---|---|
| WinUI3 | `ItemsRepeater` + `VirtualizingLayout` |
| AppKit | `NSTableView` / `NSCollectionView`(セル再利用機構をそのまま利用) |
| GTK4 | `gtk::ListView` + `GListModel`(GTK4は元々仮想化前提の設計) |
| egui / iced | `elwindui-core`の`LayoutEngine`(付録H.2)がビューポート情報を持ち、表示範囲外の`render_item`呼び出し自体をスキップする |

## Q.6 まとめ

| 要件 | 対応 |
|---|---|
| 大量データの効率描画 | `VirtualList { items, item_height/estimated_item_height, render_item }` |
| 順序変更時の安全な再利用 | `key`関数による同一性判定 |
| リサイクルとライフサイクルの整合 | プール再利用時は`on_mount`を再発火させず、prop更新のみで反映 |
| `key`未指定時の注意喚起 | 14章ルール23による静的警告 |

---

# 付録R. テーマ/デザイントークン

`style{}`(6章)は個別属性の上書きにとどまるため、カラーパレット・スペーシング・タイポグラフィを一元管理する`theme`構文を追加する。

## R.1 `theme`の定義

```rust
theme AppTheme {
    tokens {
        color primary
        color background
        color text
        spacing unit
        font body
        font heading
    }

    variant Light {
        primary: "#2ecc71"
        background: "#ffffff"
        text: "#111111"
        unit: 8
        body: Font { family: "Noto Sans", size: 14 }
        heading: Font { family: "Noto Sans", size: 20, weight: Bold }
    }

    variant Dark {
        primary: "#27ae60"
        background: "#111111"
        text: "#eeeeee"
        unit: 8
        body: Font { family: "Noto Sans", size: 14 }
        heading: Font { family: "Noto Sans", size: 20, weight: Bold }
    }
}
```

- `tokens { ... }` でトークン名と種別(`color`/`spacing`/`font`)を宣言し、各`variant`ブロックで具体値を与える
- 全`variant`は`tokens{}`で宣言されたトークンを過不足なく持たなければならない(14章ルール22)。これにより「ダークモードだけ特定の色が定義されていない」という事故を静的に防ぐ

## R.2 トークンの参照

```rust
style {
    select(Button) { background: AppTheme.primary }
    select(Text) { font: AppTheme.body }
}
```

```rust
Canvas {
    on_paint: |p| p.fill_rect_brush(rect, &Brush::Solid(AppTheme.primary))
}
```

- `AppTheme.token名`という`.`アクセス(9章の`env.*`や付録Jの`store`フィールド参照と同じ慣習)で、現在選択中のvariantにおける値が解決される
- `style{}`だけでなく、付録N(`Painter`/`Brush`)からも直接参照できる

## R.3 実行時のvariant切り替え

```rust
#![theme(AppTheme, variant: bind!(AppSettings.theme_mode, OneWay))]
```

- ファイル単位のアトリビュートで、どの`store`フィールドが現在のvariantを決めるかを宣言する(`#![backend(...)]`と同じ慣習)
- `AppSettings.theme_mode`(付録Jのstore)が変化すると、`AppTheme.*`を参照している全ての`style`/`Painter`呼び出しが自動的に再評価される(既存の`prop`差分更新の仕組みに乗る)

## R.4 まとめ

| 要件 | 対応 |
|---|---|
| デザイントークンの一元管理 | `theme Name { tokens { ... } variant X { ... } }` |
| variant間の整合性保証 | 全variantが同じトークン集合を持つことを静的検証(14章ルール22) |
| styleからの参照 | `AppTheme.token名`(`.`アクセス) |
| 描画コードからの参照 | `Painter`/`Brush`からも同じ記法で参照可能(付録N) |
| 実行時切り替え | `#![theme(Name, variant: bind!(...))]` + storeとの連携 |

---

# 付録S. エラーハンドリング(エラーバウンダリ)

`view`内で予期しないエラーが発生した際に、アプリ全体をクラッシュさせずに該当部分だけフォールバック表示に切り替える仕組み。

## S.1 `ErrorBoundary`ビルトイン

```rust
view App {
    ErrorBoundary {
        fallback: |err| Text { text: t!("error-fallback", message: err.to_string()), color: "#e74c3c" }

        NotepadWindow { }
    }
}
```

- `ErrorBoundary`は子要素の`view`構築・`#[computed]`評価・`Canvas`の`on_paint`実行中に発生したエラーを捕捉し、`fallback`に置き換えて表示する
- ネストが可能で、内側の`ErrorBoundary`が捕捉範囲を限定する(画面全体ではなく特定のカードだけをフォールバック表示にする、といった使い方ができる)

## S.2 捕捉対象と実装方針

内部的には該当サブツリーの構築処理を`std::panic::catch_unwind`相当の仕組みで囲む(コード生成器が`UnwindSafe`境界を自動的に満たす形でラップする)。

| バックエンド | 実装上の注意 |
|---|---|
| WinUI3 / AppKit / GTK4 | ネイティブAPI呼び出し(COM/Objective-C/GObject)を跨ぐパニックは言語境界でUB化する恐れがあるため、ネイティブ呼び出し部分は結果を`Result`で返すラッパーに限定し、`catch_unwind`は純粋なRustロジック(`#[computed]`評価、`Painter`呼び出し)の範囲に留める、というベストエフォートの方針を明記する |
| egui / iced | Rust側で完結するため`catch_unwind`がそのまま有効 |

## S.3 Commandのエラー捕捉

同期的な`Command`のエラーも同様に扱えるよう、`#[command(catches: ErrorType)]`を用意する。

```rust
#[command(catches: DocError)]
save: Command = command!(|| {
    document::save(&content)?;   // Err(DocError)を返すとlast_errorに自動格納される
}),
```

- `catches`を指定すると、`Command`実行中に返ったエラーが自動的に`viewmodel`の`last_error: Option<ErrorType>`相当のフィールドに格納される(付録Pの非同期Commandのエラー処理と同じパターンの同期版)

## S.4 デフォルトフォールバック

`ErrorBoundary`で囲まれていない箇所でエラーが発生した場合、`elwindui-core`が提供する既定のエラー画面(デバッグビルドでは詳細なスタック情報、リリースビルドでは簡潔なメッセージ)に切り替わり、アプリ全体のクラッシュを防ぐ。

## S.5 まとめ

| 要件 | 対応 |
|---|---|
| サブツリー単位のエラー捕捉 | `ErrorBoundary { fallback: \|err\| ..., children }` |
| Command実行時のエラー捕捉 | `#[command(catches: ErrorType)]`(同期)、付録P(非同期)と対になる仕組み |
| ネイティブ境界のパニック対策 | ネイティブAPI呼び出しは`Result`化を必須とし、`catch_unwind`は純粋Rustロジックの範囲に限定 |
| 未捕捉時の挙動 | `elwindui-core`既定のフォールバック画面でクラッシュを防止 |

---

# 付録T. クリップボード・ドラッグ&ドロップ・ファイルダイアログ

OS機能へのアクセスを、GUI要素ではなく`platform::`名前空間の関数として提供する(9章の`env::*`、5章で触れた`external::*`と同じ「明示的な入口」の考え方)。

## T.1 クリップボード

```rust
platform::clipboard::write_text(&content);
let text: Option<String> = platform::clipboard::read_text();
```

## T.2 ファイルダイアログ(非同期)

```rust
#[command(async)]
open: Command = command!(async || {
    if let Some(path) = platform::file_dialog::open(FileFilter::new(t!("filter-text"), &["txt"])).await {
        content = fs::read_to_string(&path).await.unwrap_or_default();
    }
}),
```

- ファイルダイアログは本質的に非同期(ユーザーの操作待ち)であるため、常に`Future`を返し、付録Pの`#[command(async)]`パターンと組み合わせて使う

## T.3 ドラッグ&ドロップ

```rust
TextArea {
    text: content
    draggable: false
    on_drop: |files: Vec<PathBuf>| open_files(files)
}
```

- `on_drag_start` / `on_drop` / `draggable: bool` は任意のビルトイン要素が持てる共通属性として提供する(付録Mの`tooltip`/`context_menu`と同じ位置づけ)

## T.4 バックエンド対応

| 機能 | WinUI3 | AppKit | GTK4 | egui / iced |
|---|---|---|---|---|
| クリップボード | `Clipboard`/`DataPackage` | `NSPasteboard` | `Gdk::Clipboard` | `arboard`クレート経由 |
| ファイルダイアログ | `FileOpenPicker`/`FileSavePicker` | `NSOpenPanel`/`NSSavePanel` | `gtk::FileChooserNative` | `rfd`クレート経由 |
| ドラッグ&ドロップ | `DragDrop`イベント | `NSDraggingDestination` | `Gtk::DropTarget` | Canvas内ヒットテストで独自実装 |

## T.5 まとめ

| 要件 | 対応 |
|---|---|
| クリップボード操作 | `platform::clipboard::read_text()` / `write_text()` |
| ファイルダイアログ | `platform::file_dialog::open()`/`save()`(非同期、付録Pと連携) |
| ドラッグ&ドロップ | `draggable` / `on_drag_start` / `on_drop` 共通属性 |
| OS機能アクセスの一貫性 | `platform::`名前空間に集約し、`env::`/`external::`と同じ「明示的な入口」の思想を踏襲 |

---

# 付録U. Undo/Redo共通パターン

編集操作の取り消し・やり直しを、`viewmodel`のフィールドに対する共通の仕組みとして提供する。

## U.1 `#[undoable]`

```rust
viewmodel NotepadViewModel {
    #[observable]
    #[undoable]
    content: String = String::new(),
    ...
}
```

- `#[undoable]`が付いた`#[observable]`フィールドは、値が変化するたびに変更前の値が内部のUndoスタックへ自動的に積まれる
- `#[undoable]`は`viewmodel`の`#[observable]`フィールドにのみ付与できる(14章ルール21)。Undoの単位は「1つのViewの編集セッション」に紐づくため、アプリ全体で共有される`store`や、実体化時固定の`component`の`prop`には意味を持たない

## U.2 自動生成される`undo`/`redo`

```rust
vm.undo.execute();
vm.redo.execute();
vm.can_undo   // bool
vm.can_redo   // bool
```

- `#[undoable]`フィールドが1つ以上ある`viewmodel`には、`undo: Command`/`redo: Command`と、対応する`can_undo`/`can_redo`(付録O.3の`Command`と同じ仕組み)が自動的に追加される
- ボタンへの結線はO.4の通常のCommandと同じ書き方でよい

```rust
Button { text: t!("menu-undo"), on_click: vm.undo.execute(), enabled: vm.can_undo }
```

## U.3 変更の一括化(coalescing)

キー入力のたびに1文字ごとのUndo単位が積まれると使い勝手が悪いため、時間窓でまとめる。

```rust
#[observable]
#[undoable(coalesce: 500ms)]
content: String = String::new(),
```

- 500ms以内に連続して発生した変更は1つのUndoエントリにまとめられる。付録N.6の`#[transition(duration: ...)]`と同じ「時間指定アトリビュート」の慣習を踏襲する

## U.4 まとめ

| 要件 | 対応 |
|---|---|
| 変更履歴の自動記録 | `#[undoable]`(`viewmodel`の`#[observable]`フィールド限定、14章ルール21) |
| Undo/Redo操作 | 自動生成される`vm.undo`/`vm.redo`(`Command`型、`can_undo`/`can_redo`込み) |
| 連続入力の一括化 | `#[undoable(coalesce: 500ms)]` |

---

# 付録V. テスト支援(スナップショットテスト)

付録O.6の`viewmodel`単体テストに加え、`view`が実際に組み立てる要素ツリー・描画結果を検証するためのスナップショットテスト機構を提供する。

## V.1 要素ツリーのスナップショット

```rust
#[test]
fn notepad_initial_view_matches_snapshot() {
    let vm = NotepadViewModel::new();
    let tree = elwindui_test::render_tree(&NotepadWindow { vm });
    assert_snapshot!(tree);
}
```

- `render_tree`は`Element`ツリー(13章)をテキスト表現(インデント付きの構造ダンプ)に変換する
- `assert_snapshot!`は既存のRustスナップショットテストの慣習(`insta`クレート等)に合わせ、差分があれば失敗し、承認コマンドで期待値を更新できるようにする

## V.2 Canvas描画のビジュアルリグレッション

```rust
#[test]
fn knob_renders_correctly_at_half_value() {
    let image = elwindui_test::render_canvas_snapshot(|p| draw_knob(p, 0.5), Size::new(60.0, 60.0));
    assert_image_snapshot!(image);
}
```

- 付録B.3(リアルタイムプレビュー)で定義済みの「オフスクリーンレンダリング」機能をそのまま再利用し、`Painter`ベースの描画関数を画像として出力・比較する
- 新しいバックエンド種別(テスト専用のBackend variant等)は追加せず、既存の`Backend`(egui等)のヘッドレスモードを用いる。これにより`match target::backend()`の網羅性検査(8章)に影響を与えない

## V.3 ViewModelテストとの役割分担

| テスト対象 | 手段 |
|---|---|
| ビジネスロジック・Commandの振る舞い | 付録O.6:通常の`#[test]` + `viewmodel`の直接操作(バックエンド起動不要) |
| 要素ツリーの構造(レイアウト・分岐結果) | V.1:`render_tree` + `assert_snapshot!` |
| Canvas等のピクセル単位の描画結果 | V.2:`render_canvas_snapshot` + `assert_image_snapshot!` |

## V.4 まとめ

| 要件 | 対応 |
|---|---|
| 要素ツリーの回帰テスト | `render_tree` + `assert_snapshot!` |
| 描画結果のビジュアルリグレッション | `render_canvas_snapshot` + `assert_image_snapshot!`(付録B.3のオフスクリーン描画を再利用) |
| 新規Backend種別の追加回避 | 既存backendのヘッドレスモードを使い、enum網羅性検査(8章)への影響を避ける |
| ViewModel/View/描画テストの役割分担 | 付録O.6・V.1・V.2で階層的にカバー |

---

# 付録W. モバイル対応(iOS / Android)

これまでのバックエンド抽象化(付録A・C・D)をそのまま拡張し、iOS/Androidをネイティブバックエンドの1つとして扱う。スマホ特有の要素(画面回転、セーフエリア、タッチジェスチャー、OSレベルのアプリライフサイクル、DPI、パーミッション)を補う。

## W.1 バックエンドの追加

`Backend` enum(付録D.1)に`Uikit`(iOS)/`Jetpack`(Android)を追加済み。

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

Rustバインディングは、iOSは`objc2`(AppKitと同系統のクレート)、Androidは`jni`クレート経由でJava/Kotlin APIを呼ぶ。バリアント追加に伴う既存ビルトインの網羅性エラー(付録D.1の注記参照)は、各`builtin`定義に`Backend::Uikit`/`Backend::Jetpack`向けの`native!`腕を追加することで解消する。

## W.2 画面サイズ・向き・セーフエリア(ビルトインStoreとして提供)

画面の向き・サイズ・セーフエリアは**実行中に変化しうる値**であり、`env::*`(9章、実体化時に一度だけ確定し以後不変)の性質とは合わないため、`env::*`を拡張せず、付録Jと同じ`store`の仕組みを使ったビルトインStoreとして提供する。

```rust
store platform::Device {
    orientation: Orientation,
    safe_area: EdgeInsets,
    window_size: Size,
}

enum Orientation { Portrait, LandscapeLeft, LandscapeRight, PortraitUpsideDown }
struct EdgeInsets { top: f32, bottom: f32, left: f32, right: f32 }
```

- 通常の`store`同様、参照は`bind!`を経由する(14章ルール13がここにも適用され、`#[param]`側からの直接参照は禁止される)

```rust
component NotepadWindow {
    orientation: Orientation = bind!(platform::Device.orientation, OneWay),
    safe_area: EdgeInsets = bind!(platform::Device.safe_area, OneWay),
}
```

## W.3 セーフエリアのレイアウトへの反映

`Window`ビルトイン(付録F.1)は既定で`respects_safe_area: true`を持ち、レイアウトエンジン(付録H.2)の`measure`が利用可能領域を計算する際に`platform::Device.safe_area`を差し引く。

```rust
Window {
    respects_safe_area: true   // 既定値。ノッチ・ホームインジケータ領域を自動的に避ける
}
```

## W.4 タッチジェスチャー

付録G.6の`on_pointer_down`/`on_pointer_move`(元は`Canvas`専用)を、任意のビルトイン要素が持てる共通属性として一般化し、高レベルジェスチャーを追加する。

```rust
Image {
    src: photo
    on_swipe: |direction| handle_swipe(direction)
    on_pinch: |scale| handle_zoom(scale)
    on_long_press: |pos| show_context_menu(pos)
}
```

- `on_swipe(direction: SwipeDirection)` / `on_pinch(scale: f32)` / `on_long_press(pos: Point)` は`InputRouter`(付録H.1)がジェスチャー認識を行い、確定したイベントのみをコールバックへ渡す
- デスクトップ系backend(WinUI3/AppKit/GTK4)ではマウス操作からの近似(ホイールでのピンチ相当等)にフォールバックし、対応しないジェスチャーは単に発火しない

## W.5 アプリ全体のライフサイクル(OSレベル)

付録Iの`on_mount`/`on_unmount`は**コンポーネント単位**(要素ツリーへの出入り)のライフサイクルだったが、モバイルではアプリプロセス全体がバックグラウンド/フォアグラウンドを行き来する。これをルートコンポーネントに対するフックとして提供する。

```rust
component App {
    on_foreground: {
        resume_sync();
    }
    on_background: {
        save_state();
    }
    on_terminate: {
        flush_pending_writes();
    }
}
```

- `on_foreground`/`on_background`/`on_terminate`はエントリポイント(ルート)コンポーネントに書くことを推奨する。ルート以外での宣言は14章ルール24により静的警告となる
- バックエンド対応:iOSの`applicationDidEnterBackground`/`applicationWillEnterForeground`、Androidの`onPause`/`onResume`/`onStop`にマッピングされる。デスクトップ系backendでは`on_background`はウィンドウの最小化、`on_terminate`はプロセス終了時に対応する

## W.6 画面密度(DPI)対応

付録G.6・付録N.4で既に採用している「論理ピクセル座標に統一し、バックエンドが物理ピクセル変換を担う」方針をそのまま踏襲する。追加で、画像アセットの解像度違いを扱うため`Image::asset`にDPI別バリアントの自動解決を導入する。

```rust
Image { src: Image::asset("icon") }
```

```
assets/icon/
├── icon@1x.png
├── icon@2x.png
└── icon@3x.png
```

- コード生成器は実行環境の`window_size`(W.2)やOS標準のスケールファクタから適切な倍率のファイルを選択する。命名規則はiOSの`@2x`/`@3x`慣習に合わせ、Android(`drawable-mdpi`等)向けにはビルド時変換を行う

## W.7 パーミッション

位置情報・カメラ等のOS権限リクエストは、常にユーザー操作を伴う非同期処理であるため、付録Pの非同期パターンと付録Tの`platform::`名前空間を組み合わせて提供する。

```rust
enum PermissionStatus { Granted, Denied, NotDetermined }

#[command(async)]
request_camera: Command = command!(async || {
    match platform::permissions::request(Permission::Camera).await {
        PermissionStatus::Granted => start_camera(),
        _ => show_permission_denied_message(),
    }
}),
```

- `platform::permissions::request(...)`は`AsyncState`(付録P.1)ではなく直接`PermissionStatus`を返す`Future`とする(ダイアログ表示中の「Loading」状態をUIに露出する必要性が薄いため、シンプルな`await`のみで足りるケースとして扱う)

## W.8 まとめ

| 要件 | 対応 |
|---|---|
| iOS/Androidのバックエンド追加 | `Backend::Uikit`/`Backend::Jetpack`(付録D.1)、既存ビルトインへの網羅性エラーが対応漏れを機械的に検出 |
| 画面サイズ・向き・セーフエリア | `store platform::Device`(実行時変化する値のため`env::*`ではなくStoreとして提供) |
| セーフエリアのレイアウト反映 | `Window { respects_safe_area: true }` + レイアウトエンジン(付録H.2)との連携 |
| タッチジェスチャー | `on_swipe`/`on_pinch`/`on_long_press`を任意のビルトイン要素の共通属性として一般化(付録G.6の拡張) |
| OSレベルのアプリライフサイクル | ルートコンポーネントの`on_foreground`/`on_background`/`on_terminate`(付録Iのコンポーネント単位ライフサイクルとは別軸、14章ルール24) |
| DPI対応 | 論理ピクセル座標の方針を継承、`Image::asset(...)`によるアセット解像度自動解決 |
| パーミッション | `platform::permissions::request(...)`(付録T・Pのパターンを組み合わせ) |
