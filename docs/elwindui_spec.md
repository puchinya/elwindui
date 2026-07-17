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
    TextBlock { text: "Hello" }
    Button { text: "OK" }
}
```

- 属性は `key: value` 形式
- カンマ・改行はどちらも区切りとして等価
- 単純な識別子・リテラルの参照は `${}` 不要。演算や結合を含む式のみ `format!` 等を使う

```rust
TextBlock { text: label }                  // 単純参照
TextBlock { text: format!("{label}!") }    // 式はformat!マクロで明示
```

---

## 3. component と view

コンポーネントは **`component`(データ定義)** と **`view`(描画ロジック)** の2ブロックに分離する。Rustの `struct` と `impl` の関係に対応する。

| | component | view |
|---|---|---|
| 役割 | 状態(フィールド)の定義 | 状態→見た目の写像 |
| 対応するRust概念 | `struct Foo { ... }` | `impl Foo { fn view(&self) -> Rc<dyn UIElement> }` |
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
        Row { Slider { value: volume }, TextBlock { text: label } }
    } else {
        Column { Slider { value: volume }, TextBlock { text: label } }
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

### `inherits`:WinUI3方式のクラス継承

`component Name inherits Base { ... }` は `Base` を4通りに解決する(単なる構造的契約ではなく、WinUI3/C#の`Control → ContentControl → Button`と同じ実継承):

1. **`Base`が`NativeControl`マーカー** — 純粋なカテゴリタグ(フィールド継承なし)。ネイティブ実装を持つ末端要素(`Button`等)であることを示すのみ。意味のある継承元を持たない末端要素(例:`Window` — 実際のWinUI3では`Control`ファミリーを経由せず`Object`を直接継承する)は、`inherits NativeControl`で存在しない共通の祖先を示唆する代わりに`inherits`自体を省略し、`#[native]`属性(付録E)を直接付与する。
2. **`Base`が`view`を持たないプリミティブ形状ファミリー**(例:`builtin::Control`/`builtin::Rectangle`)、または**`Base`自身が既にシェイプ合成されているDSLコンポーネント**、または**`Base`が`view`を持たないネイティブ実装のホスト**(例:`Window`) — `Base`の`#[param]`/propフィールドを**再宣言なしに自動継承**し、さらに`Name`自身の`view`の中身は**常に暗黙に`Base`自身の属性・子要素**になる(ラッパー要素は書かない――`Base { ... }`という入れ子は書かず、`Base`の属性・子要素を`view`の`{}`直下にそのまま書く)。シェイプ合成/ホスト合成(後述の付録F.10参照)。
3. **`Base`が自前の`view`を持つ、それ自体は合成されていない論理コンポーネント**(builtinでもユーザー定義でも) — フィールドに加えて`view`(テンプレート)も継承する。`Name`が独自の`view`を書かなければ`Base`のテンプレートをそのまま(WinUI3の既定`ControlTemplate`のように)引き継ぎ、書けば**完全なテンプレート上書き**になる(ルート要素の型に制約はない)。
4. **`Base`がネイティブ実装のみの末端要素**(例:`Button`) — 継承不可。生成されるRustコードを持たないため、委譲先が存在しない。

```rust
component ContentControl inherits Control {
    content: std::rc::Rc<dyn UIElement>,
    // padding は Control から自動的に継承される — 再宣言不要、self.padding() がそのまま使える
}

view ContentControl {
    // `Control { .. }` というラッパーは書かない — `view`の中身がそのまま暗黙に Control の
    // 属性・子要素になる(2番目のケース)
    padding: padding
    content
}
```

`view`の中身が暗黙に`Base`自身になるかどうかは、`Base`が実際に合成可能(2番目のケースに当てはまるか)によって決まり、`Name`自身がラッパーを書くかどうかでは選べない――合成可能な`Base`を持つ`component`の`view`は常にこの形で書く。3番目のケース(合成されていない論理コンポーネントの継承)だけが、今まで通り「独自のルート要素を持つ完全なテンプレート上書き」になる。

継承したフィールドは、派生component自身の`view`が**同名のまま裸で参照**している場合のみ、派生側の実効フィールド(＝コンストラクタ引数)になる。リテラル値で上書きしている場合(例:`Rectangle { fill: "#3a3a3c" }`)や、そもそも参照していない場合は、その基底フィールドは派生側の公開APIには現れない。

**メソッド継承とオーバーライド**(C#の`virtual`/`override`/`base.Method()`相当):

```rust
component Control {
    #[virtual]
    fn label(&self) -> String {
        "control".to_string()
    }
}

component ContentControl inherits Control {
    #[override]
    fn label(&self) -> String {
        format!("{}!", base::label())
    }
}
```

- `#[virtual] fn name(&self, ...) -> T { ... }` — 派生componentがオーバーライド可能なメソッドを宣言する
- `#[override] fn name(...) { ... }` — 基底の同名`#[virtual]`メソッドと同じシグネチャで上書きする(シグネチャ不一致は静的エラー)
- `base::name(...)` — オーバーライドした本体から基底実装を呼び出す(C#の`base.Method()`相当)。同じ書き方で`on_mount`/`on_unmount`(付録I.1)内から基底のライフサイクルフックを呼ぶこともできる
- 継承・オーバーライドは1階層(直接の`inherits`先)のみ保証される。2階層以上に渡る`base::`連鎖は現時点では未対応

`#[computed]`フィールドも同様に、基底の同名フィールドを`#[override]`なしで再宣言するとエラーになり、`#[override]`を付けると上書きとして扱われる(型は基底と一致していなければならない)。

`#[overrides(builtin::X)]`(付録E)は`inherits`とは別の、無関係な仕組みである — 前者は同名builtinの明示的なシャドーイング、後者はクラス階層の構築であり、混同しないこと。

### 添付プロパティ(`#[attached]`):WPF/WinUI3方式

あるcomponentが宣言し、**任意の別要素が自分自身に設定できる**プロパティ(WPFの`Grid.Row`/`Grid.Column`相当)。
宣言したcomponent自身のインスタンスデータにはならない——スキーマ宣言のみで、宣言したcomponent自身の
コンストラクタ/生成structには一切現れない。

```rust
component Grid {
    #[param]
    rows: Vec<GridLength>,
    #[param]
    columns: Vec<GridLength>,
    #[param]
    children: Vec<AnyView>,

    #[attached]
    row: i32 = 0,
    #[attached]
    column: i32 = 0,
}
```

```rust
Grid {
    rows: [GridLength::Auto, GridLength::Star(1.0)]
    columns: [GridLength::Fixed(120.0), GridLength::Star(1.0)]
    TextBlock { text: "Header", Grid::row: 0, Grid::column: 0 }
    Button { text: "Click", Grid::row: 1, Grid::column: 1 }
}
```

- `#[attached]`フィールドは初期値(デフォルト)必須——設定しなかった要素に適用される既定値を表す
- 設定側の構文は`Owner::field: value`(Rustのパス区切り`::`)——`{}`内で通常の属性と自由に混在できる
- `Owner`は静的には「`field`という名前の`#[attached]`フィールドを持つ既知のcomponentか」だけを検証する。
  設定先の要素が実際に`Owner`(例:`Grid`)の子孫であるかどうかは**検証しない**——WPF同様、対応する
  コンテナの外で設定しても静的エラーにはならず、単に無視される
- 実装は`(owner, field) -> Box<dyn Any>`の型消去された汎用バッグ(`UIElementImpl::attached`)——
  `builtin::Grid`の`row`/`column`もこれ経由で格納される。オーナー自身が自分の宣言した型を知っている
  ので、書き込み側(`elwindui-codegen`の`emit_attached_setters`、`SymbolTable`の
  `TypeInfo::attached_field_types`から宣言型を引いて`set_attached::<T>(..)`のターボフィッシュに渡す)
  と読み出し側(例:`elwindui_core::ui::grid_cell_of`、`get_attached::<i32>(..)`)の双方が
  そのオーナーの持つスキーマ通りにdowncastする——WPFの添付プロパティも同じ設計。将来別のcomponentが
  独自の添付プロパティを持つ場合、`UIElementImpl`側・`elwindui-codegen`側とも変更は一切不要で、
  そのcomponent自身の`#[attached]`宣言と読み出しロジックを追加するだけでよい
- 添付プロパティが実際にレイアウトへ反映されるのは、子要素が仮想ビルトインそのもの(`TextBlock`/
  `Rectangle`/`Ellipse`/`VerticalLayout`/`HorizontalLayout`/`Control`/入れ子の`Grid`)の場合、
  `inherits NativeControl`で`base: elwindui_core::ui::NativeControlImpl<H>`を合成するネイティブ
  リーフ(`Button`/`TextArea`/`TabView`)の場合、およびユーザー定義の`component`+`view`ペアで
  その`view`ルートがネイティブでない場合(`into_node()`経由で`Rc<dyn UIElement>`として取り出せる場合)
  ——いずれも構築直後に`elwindui-codegen`の`emit_common_ui_element_setters`/`emit_construction`が
  `(erased).base().set_attached::<T>(..)`を呼ぶことで反映される(付録H.2.1a)。`view`ルート自身が
  ネイティブに解決するユーザー定義component(`inherits NativeControl`を宣言せず`Button`等を
  ラップするようなケース)への設定は、`.base()`へ到達する手段自体がまだ無く、引き続き未対応
  ——将来の拡張課題

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

### コールバック型フィールド: `fn(...)` 糖衣構文

フィールドがコールバック(関数)型を持つ場合、`Rc<dyn Fn(...)>` や `Box<dyn Fn(...)>` のような
型消去表現を`.elwind`ソース上に直接書くことは禁止される(14章ルール25)。かわりに以下の糖衣構文を使う:

```
fn(引数型, ...)                // 戻り値なし、必須
fn(引数型, ...) -> 戻り値型      // 戻り値あり、必須
fn(引数型, ...)?                 // 省略可能。既定値は `= None` で明示する
```

この糖衣構文はコード生成時、フィールドを持つ`component`のインスタンス化ごとに単相化
(monomorphize)された具体的なクロージャ引数として展開され、`Box<dyn Fn>`/`Rc<dyn Fn>`のような
実行時型消去は発生しない(付録O.5の「型消去を避け専用コードを生成する」方針と同じ)。

`fn(...)`型のフィールドの意味は`#[param]`の有無でそのまま決まり、コールバック専用の追加
アトリビュートは存在しない:

- **`#[param]`付き** = 実体化時に固定される値計算コールバック。静的評価式(その場で束縛された
  クロージャ)のみ許可される。例: `key: fn(&Item) -> usize`, `render_item: fn(&Item) -> View`。
- **`#[param]`無し(既定の`prop`)** = 実行時に差し替え可能な通知コールバック、いわゆる
  イベントハンドラ。例: `on_select: fn(usize)`, `on_close: fn(usize)`。

```rust
component VirtualList {
    #[param]
    key: fn(&Item) -> usize,      // 値計算コールバック(paramなので実体化時固定)

    on_select: fn(usize),         // 通知コールバック(propなので実行時に発火・差し替え可)
}
```

(`TabView`はかつてこの例に使われていたが、付録Yの刷新で`key`クロージャ自体が無くなった
(`for` の各要素は`Rc<T>`のポインタ同一性でそのままreconcileされる)ため、この例は
`VirtualList`(付録Q — `key`を今も同じ形で使う)に差し替えている。)

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
    TextBlock { text: "権限がありません" }
}

// 分岐(網羅性検査つき)
match status {
    Status::Loading => Spinner {},
    Status::Error   => TextBlock { text: "エラー", color: "#c0392b" },
    Status::Ok      => TextBlock { text: "OK" },
}
```

`match` は列挙体の全メンバーを網羅していれば `_ =>` を省略できる。網羅されていない場合はコンパイルエラーとなる(Rustの`match`と同じ挙動)。

`if`/`match`の各分岐(`else if`チェーンを含む)には、さらに`if`/`match`/`for`を入れ子で書ける——`else if`は`else`ブロックの中にネストした`if`が1つある形として扱われる。ただし`for`自身のbody(繰り返される側のテンプレート)はリテラル要素のみで、その中に`if`/`match`/`for`をさらに入れ子にすることはできない(各`for`項目は使い捨てのローカル構造体であり、入れ子の動的領域を持つ永続状態を持たないため)。

`#[content(field_name)]`(付録E)で指定した子要素の格納先フィールドがリスト型(`Vec<..>`/`ListExt<..>`)の場合、`if`/`match`/`for`のいずれも使える(前段落の入れ子ルールも同様)。フィールドが単一値型(例:`ContentControl`/`Window`の`content: Rc<dyn UIElement>`)の場合は`if`/`match`のみ使え、`for`は使えない(可変長のリストは単一の格納先に収まらないため)。単一値フィールド配下の`if`/`match`は、入れ子も含めたあらゆる分岐が最終的にちょうど1個の要素に還元できなければならない(1分岐に複数の裸の子要素を書くこともできない)。

---

## 6. スタイル(横断的属性適用)

> **実装状況**: `style { select(...) { ... } }`構文は`elwindui-codegen`のAST(`ast::Item`)に対応する項目がなく未実装。本章は設計のみ。

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

> **実装状況**: `elwindui-codegen`の`Attr`列挙体には`#[length(start..=end)]`のみ実装されている(`Attr::Length`)。`#[range]`/`#[step]`/`#[pattern]`/`#[format]`/`#[check]`は未実装で、本章の該当部分は設計のみ。

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

### PropertyChanged と部分更新

`#[observable]` のsetterは代入後に型付き `PropertyChanged` を発火する。`view` は式から
静的に取得した依存プロパティだけを購読し、その属性または動的領域だけを更新する。従って
`TextArea { text: doc.content }` の入力はその `TextArea` と `doc.content` に依存する表示だけを
更新し、親の `TabView` の children コレクションを再同期しない。二方向バインディングのwidget→model側は
setterを呼ぶだけで、別途コンポーネント全体の再同期を呼んではならない。

購読は `Subscription` で表され、表示領域が破棄されるとDropにより解除される。`for`/`if`/`match`
の構造変更は親view全体ではなく対応する動的領域だけを差し替える。依存プロパティを静的に
特定できない任意Rust式はビルド時エラーとし、必要な計算は `#[computed]` または解析可能な
prop参照へ分解する。

---

## 11. 多言語対応(i18n)

翻訳文言は独自フォーマットを持たず、業界標準の **Fluent(.ftl)** をそのまま採用する。DSL側は `t!` マクロでFluentのメッセージIDを参照するだけで、複数形・性別分岐・日付/数値フォーマットはFluent自身の構文(`select`式、`NUMBER()`/`DATETIME()`関数)に委譲する。

```rust
TextBlock { text: t!("dashboard-title") }
TextBlock { text: t!("cart-item-count", count: n) }
TextBlock { text: t!("order-saved-at", time: order.created_at) }
TextBlock { text: t!("item-price", price: price) }
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
- `use` は対象アイテムの**実際のRustパス**へ解決される。ある型名が `.elwind` ファイル内で参照可能なのは、
  (a) 同じファイル(=同じ実パスを持つモジュール)内でローカルに定義されている場合、または
  (b) その型の実パスを指す `use` がそのファイルにある場合、のいずれかに限る。ディレクトリ内の他の
  `.elwind` ファイルに同名の型が存在するというだけでは可視にならない(ただし付録B.1のとおり、複数の
  `.elwind` ファイルが結局同じRustスコープに`include!`される場合は、その同じスコープ内では通常の
  Rustのファイル分割同様`use`は不要)。ローカル定義でも`use`解決でもない型参照は、Rustの「見つからない
  型」エラーと同様、静的検証エラーとなる
- ViewModelの参照(付録O.2/O.4)も同じ規則に従う。`viewmodel`を`.elwind`内でDSLネイティブに書いた場合も、
  `#[elwindui::viewmodel] mod foo { .. }`として通常のRustファイルに書いた場合も、参照側は必ずその実パス
  (前者なら`.elwind`ファイルが実際にコンパイルされ`include!`される先のパス、後者なら`mod foo`が実際に
  宣言されているRustパス、例: `crate::foo::Foo`)を`use`する。`elwindui::viewmodel::X`のような、どの
  モジュールにも実在しない架空の名前空間を`use`することはできない

---

## 13. 要素ツリーの探索(UIElement / visual_tree)

### 役割分担の方針

「子要素を持つ」という性質は既存の `{}` ネスト構文がそのまま表現しているため、**children専用の新しいDSL構文は追加しない**。ツリー走査専用の別トレイトは設けず、付録H.2で定義済みの `UIElement`(全要素が実装する唯一の共通トレイト)が `visual_children()`/`parent()` を通じてこの契約をそのまま担う。再帰探索アルゴリズム自体はDSLの文法ではなく、共通ランタイムライブラリ(`elwindui_core::visual_tree`)側の責務とする。

| 責務 | 担当 |
|---|---|
| 親子構造の宣言 | DSL構文(`{}` ネスト。追加構文は不要) |
| 動的生成された子要素(`if`/`for`/`match`の結果)をchildrenとして集約する規約 | コード生成器 |
| 全要素が親子を辿れるという契約(`visual_children()`/`parent()`) | `UIElement`(付録H.2.1a、コード生成器が全要素型に自動実装) |
| 再帰探索アルゴリズム(`visual_tree::find_all` 等) | 共通ランタイムライブラリ(DSLとは独立に拡張・最適化可能) |
| 特定要素への後からのアクセス | `#[id(...)]` アトリビュート |

### 共通トレイト(コード生成器が自動実装)

`children()`/`id()`だけのための別トレイトは無い。全要素型が既に実装している`UIElement`(付録H.2.1a)がその役割を兼ねる:

```rust
trait UIElement: AsAny {
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>>;
    fn parent(&self) -> Option<Rc<dyn UIElement>>;
    // ... 他多数(margin/alignment/measure/arrangeなど、付録H.2参照)
}
```

- `view` 内で `{}` ネストにより宣言された子要素は、そのままコード生成器によって `visual_children()` の返り値に詰められる
- `if` / `for` / `match` によって実行時に確定する子要素も、生成時にフラット化されて同じ `visual_children()` に集約される、という規約に統一する

```rust
view Toolbar {
    Row {
        if show_save { ToolbarButton { text: "Save" } }
        for item in extra_buttons { ToolbarButton { text: item.label } }
    }
}
```

上記のように条件・繰り返しで生成された要素も、`Row` インスタンスの `visual_children()` から一律に辿れる。

### 共通属性:`#[routed]`(ルーティングイベント、WinUI3スタイル)

コールバック型フィールド(`fn()`等)には`#[routed]`アトリビュートを付けられる。付けたイベントは
発生元の要素から祖先へバブルする(WinUI3の`RoutedEvent`相当)。対象は`builtin::Button`の
`on_click`のような入力系イベントに限られ、`TabView`の`on_select(usize)`のような
ウィジェット固有の型付きペイロードを持つコールバックはルーティング対象外(従来通りの直接配線)。

```rust
component Button inherits NativeControl {
    #[routed]
    on_click: fn(),
}
```

ハンドラは要素自身の型消去レジストリ(`UIElementBase.routed_handlers`)にイベント名で登録され、
配送(`elwindui_core::ui::dispatch_routed`)は発生元要素から`UIElementBase.parent`(本物の親
ポインタ、要素が木に組み込まれる際に必ず設定される)を辿って祖先へバブルする。`RoutedEventArgs`の
`handled`フラグが立てられると、そこで伝播が止まる。親ポインタ方式のため、`for` のように
実行時に動的組み立てられた木でも、静的な`.elwind`構造と
同様にバブルが機能する(付録H参照)。現時点の実装範囲はAppKitバックエンドの`Button`のみ。

### 特定要素への名前付きアクセス:`#[id(...)]`

`let` 束縛は同一 `view` 関数内でのみ有効なため、外部(Rustロジック側)から後で要素を参照したい場合は `#[id(...)]` アトリビュートを付与する。

```rust
view NotepadWindow {
    #[id("editor")]
    let editor = TextArea { text: content };

    Column { editor, StatusBar { ... } }
}
```

- `#[id(...)]` を付けた `let` 束縛は、`{}` ネスト内で裸の識別子として(`Column { editor, .. }` のように)参照できる子要素になる
- 実装(`elwindui-codegen`)は`#[id(...)]`ごとに、その束縛の**具象Rust型をそのまま返す名前付きアクセサメソッド**(`pub fn <id>(&self) -> Rc<ConcreteType>`)を、その`view`を持つコンポーネント自身に生成する。`#[id(...)]`が付いた束縛は暗黙的に「実体化後も保持される」扱いになり(通常の子要素同様、動的な属性を持つ場合と同じ`stored`規約)、対応するフィールドから`.clone()`して返すだけの薄いメソッドになる
- **`#[id(...)]`は全てコンパイル時に確定している**ため、実行時に文字列で検索する仕組みは経由しない — 具象型を直接返す静的アクセサの方が付録O.5の「型消去を避け専用コードを生成する」方針に沿っており、ダウンキャストも不要になる
- **ランタイム文字列idによる検索は意図的に提供しない**。`UIElement`自体はidを保持するフィールドを持たず、名前付きアクセスは`#[id(...)]`一本に統一する。これはWinUI3が`VisualTreeHelper`(構造的な木の走査のみ、後述)と`FrameworkElement.FindName`(名前引き)を明確に分離しているのと同じ役割分担であり、`FindName`相当は`#[id(...)]`が静的に担う

### 再帰探索API:`visual_tree`(共通ランタイムライブラリ、DSL非依存、`#[id(...)]`とは別の汎用機構)

`elwindui_core::visual_tree`は、WinUI3の`VisualTreeHelper`に相当する自由関数群を提供する。`UIElement`自体が既に`visual_children()`/`parent()`を持つため、木の走査そのものはこのモジュールを経由しなくても行えるが、`visual_tree`は(a) WinUI3に近い呼び出し形(`visual_tree::get_child(elem, i)`)と、(b) `UIElement`単体には無い型ベースの再帰収集(`find_all`)をまとめて提供する。上記の`#[id(...)]`アクセサ生成とは独立した機構であり、`elwindui-codegen`はこれを使ったツリー構築を生成しない。通常の`#[id(...)]`アクセスには使われない。

```rust
pub fn get_children_count(element: &dyn UIElement) -> usize;
pub fn get_child(element: &dyn UIElement, index: usize) -> Option<Rc<dyn UIElement>>;
pub fn get_parent(element: &dyn UIElement) -> Option<Rc<dyn UIElement>>; // UIElement::parentのラップ

// 型による再帰探索(該当する型の要素をすべて収集。WinUI3のVisualTreeHelperには無い拡張)
pub fn find_all<T: 'static>(root: &dyn UIElement) -> Vec<Rc<dyn UIElement>> {
    // visual_children() を再帰的に辿り、as_any().downcast_ref::<T>()が成功するものを収集する
    ...
}
```

- idによる文字列検索(`find_by_id`相当)は無い。理由は前節参照 — ランタイムidを保持する要素が存在しない
- 探索方式(深さ優先/幅優先)やキャッシュ戦略の変更は、**DSLの構文を一切変えずに**ライブラリ側の実装更新だけで完結する
- DSL側が保証するのは「`UIElement` トレイトを介してツリー全体に到達可能である」という契約のみ

---

## 14. 静的検証ルール一覧

コンパイラ/リンタが実行前に検出すべき項目:

> **実装状況**: `crates/elwindui-codegen/src/validate.rs`(1616行)は、既に実装済みの言語機能・ビルトインに対応するルール(概ね1〜8, 10〜13, 18, 19, 25 — `#[param]`の静的性、`bind!`経由のstoreアクセス、`viewmodel`のV/VM分離、`#[command]`の型検査など)を実際に検査する。一方、対応するビルトイン/機能自体が未実装なルール(9: `target::backend()`自体が存在しないため検査不能、14: `NavigationHost`未実装、15: `Dialog`未実装、16・17: `Transition`/`Effect`未実装、20: `#[async_computed]`未実装、21: `#[undoable]`未実装、22: `theme`未実装、23: `VirtualList`未実装、24: `on_foreground`等のモバイルライフサイクル未実装)は`validate.rs`にも対応する検査が存在しない。

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
23. `VirtualList`に`key`が指定されていない状態で`items`の順序が変わる更新が行われる → 警告(付録Q参照。挿入位置ベースの再利用にフォールバックし、リコンサイル効率が低下する可能性がある)。一般の `for` は `Vec<Rc<T>>` のとき各要素の `Rc<T>` ポインタ同一性で子を再利用し、その他の collection は当該範囲を再構築する(付録Y参照)。`TabView` は `TabViewItem` を子として指定する。
24. `on_foreground`/`on_background`/`on_terminate`(付録W.5)が、アプリのエントリポイント(ルート)コンポーネント以外で宣言されている → 警告(OSレベルのライフサイクルは単一箇所への集約を推奨)
25. コールバック型のフィールドで `Rc<dyn Fn(...)>` / `Box<dyn Fn(...)>` のような型消去表現を直接使用している(`fn(...)` 糖衣構文を使っていない) → エラー(4章「コールバック型フィールド」参照)

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
        Row { slider, TextBlock { text: label } }
    } else {
        Column { slider, TextBlock { text: label } }
    }
}
```

---

# 付録A. バックエンド抽象化(GUIフレームワークとの関係)

ElwindUILは特定のGUIフレームワークに依存しない**中間表現**として設計する。`.elwind`は論理的な要素ツリーを記述するのみで、具体的なフレームワーク(WinUI 3 / AppKit / GTK4)への変換は「バックエンド」が担う。

```
.elwind ファイル(ElwindUIL構文)
        │  コンパイル
        ▼
共通AST(フレームワーク非依存の要素ツリー)
        │  バックエンド別コード生成
        ▼
┌─────────┬─────────┬─────────┐
│  WinUI 3 │  AppKit  │  GTK4   │
│ backend  │ backend  │ backend │
└─────────┴─────────┴─────────┘
```

## A.1 バックエンド指定

```rust
#![backend(appkit)]

use components::slider::Slider;
```

- ファイル単位、または`component`単位でアトリビュートにより指定する
- 制約検証・enum網羅性検査・i18n解決など、これまでの言語機能はすべてバックエンド非依存のフロントエンド解析段階で完結し、バックエンドの選択に影響されない
- 論理要素 ⇔ 各ネイティブAPIの具体的なマッピング例は付録C.2を参照

## A.2 エスケープハッチ:`native!`

フレームワーク固有API(例:AppKitの`NSVisualEffectView`)を直接使いたい場合、専用ブロックで生Rustコードを埋め込む。

```rust
view Dashboard {
    Column {
        TextBlock { text: "売上グラフ" }

        native! {
            let vibrancy_view = NSVisualEffectView::new();
            self.window.contentView().addSubview(&vibrancy_view);
        }
    }
}
```

- `native! { ... }` 内は該当バックエンド専用コードであることが明示され、リンタは移植性のない箇所として検出できる

`UIElement`トレイト(`visual_children()`/`parent()`)や`param`/`prop`の意味自体はバックエンドを問わず共通。全バックエンドが保持モード(要素が生成後も明示的な更新まで存在し続ける)であるため、`prop`の変更はコード生成器がネイティブAPIのプロパティ更新呼び出しへと変換する(詳細は付録C.5)。

---

# 付録B. ツールチェーン仕様(コード生成・エディタ支援)

> **実装状況**: B.1(`build.rs`方式の`elwindui_codegen::compile_dir`、および`elwindui::component!`proc-macro方式)は両方とも実装済みで、`examples/notepad`(build.rs方式)・`examples/notepad-inline`(proc-macro方式)で実際に使われている。B.2(`elwindui-languageserver`)は実装あり。B.3(3段階リアルタイムプレビュー)はワークスペースに対応するクレートが存在せず未着手。B.4(実行中アプリへのホットリロード)は`elwindui-hotreload`クレートが存在するが、現状は「`#[param]`変更なら再マウント、prop変更のみなら差分更新」という粒度判定ロジックのみの実装で、`hot-lib-reloader`によるdylib差し替え自体は未実装。

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
- 生成された各`.rs`ファイルは、上記のように`main.rs`(クレートルート)へ`include!`される既定方式では、
  ディレクトリ構造をmodネストへ写像せず、フラットにクレートルートへ展開される。これは`include!`が
  ソーステキストをその場に貼り付けるのと同じなので、`src/ui`以下の複数の`.elwind`ファイルを同じ場所へ
  `include!`した場合、それらが生成する型は実際に同じRustスコープ(クレートルート)に存在することになり、
  Rust自身の規則どおり、その間で`use`は不要になる(同じファイルに書かれた複数の`struct`同士が
  `use`なしに参照し合えるのと同じ)。一方、`#[elwindui::viewmodel] mod foo { .. }`のように通常の`.rs`
  ファイル側に`mod`として宣言されたアイテムは、その`mod`が実際に宣言されている実パス(例:
  `crate::foo::Foo`)を持つため、`.elwind`側からは`use crate::foo::Foo;`のように実パスを`use`する
  必要がある(§12)。ディレクトリ構造をmodネストへ対応づける方式へ変更する場合は、この節を合わせて
  更新すること

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

これら付録A・Bはいずれも言語仕様(`component`/`view`/`param`/`prop`/`UIElement`トレイト等)を変更せずに構築できるツールチェーン層として位置づける。

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
        └─ Linux向けビルド   → GTK4 backend
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

## C.5 `UIElement`トレイト・param/propとの整合

ネイティブバックエンドは保持モード(要素が生成後も明示的な更新まで存在し続ける)であるため、以下のように変換される。

- `prop`変更 → 対応するネイティブAPIのプロパティ更新呼び出し(例:WinUI 3なら`button.SetContent(new_text)`、AppKitなら`button.setTitle(new_text)`)
- `#[computed]`の再評価 → 依存する`prop`の変化に応じて該当ウィジェットのプロパティ更新コードが生成される
- `children()`の構成変化(`for`ループの要素数増減等) → コンテナへの`addChild`/`removeChild`相当のAPI呼び出しに変換される(差分検出はコード生成器の責務)

## C.6 まとめ

| 項目 | 担当 |
|---|---|
| `.elwind`の記述 | 常に1つ、プラットフォーム分岐は原則書かない |
| どのOSでどのツールキットを使うか | `#![backend(native)]` またはビルドターゲット別の明示指定(`winui3`/`appkit`/`gtk4`) |
| 論理要素→具体API変換 | 各バックエンドクレート(`elwindui-backend-winui3`, `elwindui-backend-appkit`, `elwindui-backend-gtk4`) |
| OSごとの見た目差 | `style { select(..., backend == ...) }` |
| OS固有機能の直接利用 | `#[cfg(backend = "...")]` + `native!` |
| プロパティ変更の反映方式 | バックエンドが保持モードAPIへの更新呼び出しとして生成、DSL側の`param`/`prop`定義は不変 |

---

# 付録D. バックエンド種別の静的定数(`target::backend()`)

> **実装状況**: 本付録の`Backend` enumおよび`target::backend()`は**未実装のフォワードルッキング設計**である。現状の`elwindui-codegen`はバックエンド選択をCargoフィーチャフラグ(`crates/elwindui/Cargo.toml`の`backend-appkit`/`backend-winui3`/`backend-gtk4`)による`#[cfg(feature = "...")]`条件付きコンパイルのみで行っており、`.elwind`式中から参照できる`Backend`enumや`target::backend()`定数関数はコード中に存在しない。以下は将来この機構を導入する際の設計である。

フレームワーク種別(WinUI 3 / AppKit / GTK4 等)を、`.elwind`ファイル内の式から直接参照できる**コンパイル時静的定数**として扱う。これにより、抽象化されたコンポーネント定義を**1つの`.elwind`ファイル内で完結**させられる。

## D.1 `Backend` enumと`target::backend()`

```rust
enum Backend {
    Winui3,
    Appkit,
    Gtk4,
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
builtin::TextBlock
builtin::VerticalLayout
builtin::HorizontalLayout
builtin::TextArea
```

- これまで`Button { ... }`等と書いてきた記法は、`builtin::Button`への暗黙の`use`が常に効いている、という扱いにする
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
        _ => Rect { enabled: enabled, on_click: on_click(), TextBlock { text: text } }
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

## E.9 `component`宣言レベルの属性:`#[embedded]`/`#[sealed]`/`#[native]`/`#[abstract]`/`#[content(field_name)]`

`#[overrides(builtin::X)]`(E.4)がユーザー定義コンポーネント側に付ける属性なのに対し、`#[embedded]`/`#[sealed]`/`#[native]`は`elwindui-codegen`自身の`.elwind`ソース(`BUILTIN_SHAPE_SOURCE`、`crates/elwindui-codegen/src/builtins.elwind`)が自分自身に付ける属性。`#[content(field_name)]`だけはビルトイン限定ではなく、ユーザー定義コンポーネントでも使える。`#[abstract]`もビルトイン限定ではない一般属性。いずれも`component`宣言の直前に、`inherits`の有無に関わらず0個以上任意の順序で書ける(`enum`/`viewmodel`/`view`には付けられない)。

- **`#[embedded]`** — このコンポーネントが`BUILTIN_SHAPE_SOURCE`自身の組み込み部品であることを明示する。`elwindui-codegen`は`BUILTIN_SHAPE_SOURCE`由来のモジュールを内部的に`is_builtin`フラグ付きで扱っており、`#[embedded]`が付いたコンポーネントがそれ以外の場所(利用者自身の`.elwind`ファイル)から来ていれば静的エラーになる。
- **`#[sealed]`** — このコンポーネントを`component X inherits Y`の`Y`(継承元)として指定できないようにする。具象的な末端形状(`Rectangle`/`Ellipse` — 継承したければ合成可能な`Shape`を使う)や、そもそも継承先を持たないネイティブ末端要素(`Button`/`TextArea`/`TabView`/`TabViewItem`)に付与する。
- **`#[native]`** — `inherits`元を持たず(base-less)、かつ`view`も持たないコンポーネントに、「実Rust実装は各バックエンドクレートが手書きする」ことを明示する。`inherits NativeControl`(E.1の1.)と`is_native == true`として扱われる点は同じだが、`NativeControl`という共有タグを経由しない——2つの使い分けは「実際にビジュアルツリーに`Rc<dyn UIElement>`として埋め込まれ、`elwindui_core::ui::NativeControlImpl<H>`をバックエンド構造体の`base`として合成するか」で決まる(付録H.2.1a)。`Window`(実際のWinUI3の`Window`が`Control`ファミリーを経由せず`Object`を直接継承するのに対応)に加え、ビジュアルツリーに参加しない`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabViewItem`もこちらを使う。`#[native]`は`base`を持つコンポーネントや自前の`view`を持つコンポーネントには付けられず、`#[embedded]`と同様`BUILTIN_SHAPE_SOURCE`自身の宣言以外では使えない。
- **`#[abstract]`** — このコンポーネントを`view`内で直接インスタンス化できないようにする(`Type { .. }`という形で、属性値・クロージャ本体・裸のネスト子要素・単体の`view`ルートのどこに書いても静的エラー)。`component X inherits Y`の`Y`として指定するのは引き続き可能——むしろそれが本来の使い道で、`#[sealed]`のちょうど逆に位置する。唯一の例外は、`X`自身が`inherits`で名指ししている`#[abstract]`な`base`を、`X`自身の`view`の**ルート要素として**構築する場合(シェイプ合成、E.9の`Shape`の例。`validate::validate_inherits`が「ルート要素は`base`と一致しなければならない」を既に強制しているので、この一箇所だけ安全に許可される)。`builtins.elwind`の`UIElement`/`NativeControl`/`Layout`/`Shape`(いずれも「フィールドを持たない純粋なカテゴリタグ」、もしくは`Rectangle`/`Ellipse`が合成する土台)に付いており、直接使うことを意図した具象virtual builtin(`VerticalLayout`/`HorizontalLayout`/`Control`/`Grid`/`TextBlock`)には付かない。`codegen::generate_module`も`#[abstract]`なコンポーネントには`create_<snake case>(..)`/`new(..)`を一切生成しない。
- **`#[content(field_name)]`** — WinUI3の`ContentPropertyAttribute`相当。ある要素の`view`本体に「属性名を書かない裸のネスト子要素」(`Type { .. }`を`name: value`形式でなく直接`{}`内に書く)を渡した際、それがどのフィールドに束縛されるかを明示する。例:`MenuBarItem`は`#[content(submenu)]`を宣言しており、`MenuBarItem { text: "File", Menu { .. } }`の`Menu { .. }`は`submenu`フィールドに束縛される(`Window`/`ContentControl`/`TabViewItem`の`content`フィールドも同様に`#[content(content)]`を宣言している)。`field_name`は実在するフィールド名でなければならず(静的検証)、componentにつき最大1個。裸のネスト子要素があるのに`#[content(..)]`(または`children: Vec<..>`のようなリストフィールド)が無いcomponentにそれを渡すのはコード生成時エラーになる。

---

> 標準ビルトイン部品(`builtin::`名前空間のUI要素・`platform::`名前空間のOS機能アクセス)の仕様は本体から分離し、
> `docs/elwindui_builtins_spec.md` にまとめてある(付録F・G・L・M・N・Q・T、および新規追加の付録X・Y)。
> 本体中に残る「付録G参照」等の記述はそちらのファイルの節を指す。


# 付録H. コアランタイム(レイアウト・フォーカス・アクセシビリティ)

Button/Textのような個別ウィジェットの抽象化(付録F・G)とは別レイヤーとして、WinUI 3の`Composition`/`UIAutomation`/`Measure-Arrange`に相当する共通基盤を`elwindui-core`として定義し、各バックエンドがこれを実装する。

## H.1 全体構造

```
.elwind (component/view)
        │
        ▼
UIElement ツリー(13章で定義済み)
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
各バックエンド実装(WinUI3/AppKit/GTK4)
```

各バックエンドはOS標準機構(WinUI3の`UIAutomation`、AppKitの`NSAccessibility`、GTK4の`Atk`/AT-SPI等)に極力委譲し、Core Runtimeはレイアウト・フォーカス・ヒットテストなどバックエンド非依存の共通計算を担う。

## H.2 レイアウトエンジン

WinUI3の`Measure`/`Arrange`2パス方式を採用する。

```rust
trait LayoutNode {
    fn measure(&self, available: Size) -> Size;
    fn arrange(&mut self, final_rect: Rect);
}
```

- 各バックエンドのネイティブ葉ウィジェットのハンドル(`elwindui-backend-appkit::AnyView`等)がこのトレイトを実装する(下記`NativeControl<H>`経由で`UIElement`に接続される)
- `.elwind`側では既存の`width`/`height`/`spacing`等の属性がそのままMeasure/Arrangeの入力になり、新しい構文は不要
- レイアウト計算自体は`elwindui-core`内の共通実装(1つのRustクレート)で行い、バックエンドは計算結果(確定した矩形座標)を受け取ってネイティブAPIに反映するだけ、という役割分担にする

| バックエンド | レイアウト計算の主体 |
|---|---|
| WinUI3 | Core Runtimeで計算 → 結果を絶対配置コンテナに反映 |
| AppKit / GTK4 | 同様にCore Runtimeの計算結果を`NSView.frame`/`gtk_widget_size_allocate`に反映 |

この一元化により、全バックエンドで同一のレイアウト結果が保証される。

## H.2.1 `UIElement`階層(WinUI3方式)

要素ツリー(Visualツリー、H.2.2参照)は、WinUI3が実際に`UIElement`派生クラスの木として要素ツリーを
表現しているのに倣い、`Rc<dyn UIElement>`というトレイトオブジェクトの木そのものとして表現される
(別途「ツリー型」というラッパーは存在しない)。子から親への逆参照(`parent()`)を持つため`Box`ではなく
`Rc`で所有する(付録H.2.1a参照)。

### H.2.1a Rustコードでのクラス階層表現規約

elwindui本体(コード生成・手書きランタイム双方)でRustに“クラス”階層を実装する際は、以下の規約に従う
(Rustには実装継承がないため、trait(振る舞いの契約)と構造体合成(データの委譲)を組み合わせて疑似的に
表現する):

- コンポーネント(クラス)名を`Class`とすると:
  - **トレイト名**: `Class`。親クラスが`SuperClass`なら`trait Class: SuperClass`と宣言し、Rustのtrait境界で
    継承関係を表現する。
  - **構造体名**: `ClassImpl`。先頭のフィールドとして`base`という名前で親クラスの構造体を保持する:
    ```rust
    struct ClassImpl {
        base: SuperClassImpl,
        // Class自身が宣言したフィールドはこの後に続く
    }
    ```
  - 親を持たない既定(ルート)クラス(例: `UIElement`)は`base`フィールドを持たない。
  - `ClassImpl`は`Class`自身のトレイトに加えて、既定クラスまでの祖先トレイトを**すべて**実装する
    (`UIElement => Control => ContentControl`の継承チェーンなら、`ContentControlImpl`は`UIElement`・
    `Control`・`ContentControl`の3つのトレイトすべてを実装する)。祖先トレイトの各メソッドは
    `self.base.method(...)`へ委譲するだけの薄い実装になる。
  - 構造体の生成は構造体リテラルを直接書かず、ファクトリー関数`create_class(...)`を経由する
    (例: `Button`なら`create_button()`)。`margin`/`horizontal_alignment`/`vertical_alignment`/
    `grid_cell`(`UIElementImpl`が持つ共通フィールド)に加えて、**このクラス自身が
    宣言する`#[param]`フィールドも含めて全プロパティ**が`create_class(...)`の引数にはならない——
    ネイティブ手書きビルトイン(`Window`/`Button`/`TextArea`/`MenuBar`/`Menu`/`MenuItem`/
    `MenuBarItem`/`TabView`/`TabViewItem`)と`elwindui-core::ui`の仮想ビルトイン
    (`VerticalLayout`/`HorizontalLayout`/`Shape`/`TextBlock`/`Control`/`Grid`)は、`Copy`な
    フィールドを`Cell`、それ以外を`RefCell`で持ち(付録O.5)、`create_class()`は常に引数なしで
    `UIElementImpl::default()`相当の既定値を組み立てるだけ。使用箇所ごとの値は、構築**後**に
    `binding.set_<field>(..)`(margin等の共通属性なら`binding.base().set_margin(..)`)という
    呼び出しで反映する(`elwindui-codegen`の`emit_common_ui_element_setters`/
    `build_component_setters`)——`resync()`による値の再反映(二重バインディング等)も同じ
    `set_<field>(..)`を呼ぶだけで済む、単一の統一された仕組みになる。
    (`view`を持つコンポーネント——`ContentControl`/`Rectangle`/`Ellipse`のような組み込みでも、
    ユーザー定義componentでも——の生成`new(args)`は今のところ対象外で、引数どおりに構築する
    従来の方式のまま。)
- コード生成器(`elwindui-codegen`)が`component X inherits Y`から生成するコードも同じ形を取る——
  かつてのように親の実効フィールド/メソッドを1つのstructへ畳み込む(フラット化する)のではなく、
  `XImpl { base: YImpl, /* Xの宣言分のみ */ }`という実体合成にする。これにより`base::method(...)`
  (§3)は名前を変えたシャドーメソッドを介さず、文字通り`self.base.method(...)`に書き換えるだけで済む。
  合成済み(is_shape_composition/is_template_composition)の`.elwind` `component`は、実体のある
  構造体を`XImpl`(例: `ContentControlImpl`)へリネームし、空いた`X`という裸名を本物の
  `pub trait X: UIElement + <祖先の実トレイト>`として使う——手書きランタイム層とまったく同じ
  命名規約が両立する。`.elwind`のDSL構文や、他コンポーネントが`X { ... }`と書く箇所は一切変更不要
  ——`X`という名前を実際にRustの型として解決してコードを生成するのは`elwindui-codegen`の
  `emit_construction`(`concrete_type_ident`)だけであり、そこが常に`X::new(args)`を
  `XImpl::new(args)`へ書き換えて生成するため、外部から見た`X::new(args)`という既存の呼び出し規約は
  変わらず成立する。

```rust
pub trait UIElement: AsAny {
    fn base(&self) -> &UIElementImpl;
    fn margin(&self) -> f32 { self.base().margin.get() }
    fn horizontal_alignment(&self) -> HorizontalAlignment { self.base().horizontal_alignment.get() }
    fn vertical_alignment(&self) -> VerticalAlignment { self.base().vertical_alignment.get() }
    fn visual_children(&self) -> &[Rc<dyn UIElement>];
    fn measure_override(&self, available: Size, child_sizes: &[Size]) -> Size;
    fn arrange_override(&self, final_size: Size, child_sizes: &[Size]) -> Vec<Rect>;
    fn render(&self, context: &mut RenderContext) {}
    // `NativeControlImpl<H>`自身は`Some(self)`を返し、それを`base`として合成する型
    // (`ButtonImpl`等)は`Some(&self.base)`を返す——既定は`None`。付録H.2.1a参照。
    fn as_native_control(&self) -> Option<&dyn Any> { None }
}

// margin/alignment/visibility/grid_cellは全て内部可変(`Cell`/`RefCell`) —
// `create_xxx(...)`は常に`UIElementImpl::default()`を組み立てるだけで、使用箇所ごとの値は構築後に
// `set_margin(..)`等のセッターで反映する(前掲の規約説明参照)。
pub struct UIElementImpl {
    pub margin: Cell<f32>, // 一律のMargin。Thickness(上下左右個別)は未対応
    pub horizontal_alignment: Cell<HorizontalAlignment>, // Left | Center | Right | Stretch(既定)
    pub vertical_alignment: Cell<VerticalAlignment>,     // Top | Center | Bottom | Stretch(既定)
    pub visibility: Cell<Visibility>,                    // Visible(既定) | Collapsed
}
```

`Visibility`はWinUI3の`UIElement.Visibility`と同じく`Visible`(既定)/`Collapsed`の2値のみ(WPFの
`Hidden`相当は無い)。`Collapsed`な要素はレイアウト上スペースを一切取らず(`measure`が常に
`(0, 0)`を返す——自身の`Width`/`Height`指定も無視する)、`arrange`/`hit_test`の対象からもその
子孫ごと除外される(描画されず、ヒットテストにも当たらない)。`margin`/`horizontal_alignment`と
同じ共通属性だが、`.elwind`側の`margin`のような即値配線(`emit_common_ui_element_setters`)はまだ
無く、`set_visibility(..)`をRustから直接呼ぶ形にとどまる。

`UIElement`はこの階層の既定(ルート)クラスなので`UIElementImpl`は`base`フィールドを持たない。
`UIElement`トレイト自体はハンドル型`H`について非ジェネリックである。実ネイティブハンドルを持つのは
`NativeControlImpl<H>`(下記)だけであり、木を歩く汎用関数(`measure`/`arrange`/`layout_root`)の
方がハンドル型`H`についてジェネリックになっている。

```
UIElement (トレイト、Margin/Alignment共通実装。UIElementImplがbaseなしの既定クラス。
 │        `builtins.elwind`上もDSLの`component UIElement {}`として存在する全ての根)
 ├─ NativeControl<H> => Button, TextArea, TabView, ... (実ハンドルHを保持する、ビジュアルツリーに
 │                       実際に埋め込まれる型のみ。MenuBar/MenuBarItem/Menu/MenuItem/TabViewItemは
 │                       ツリーに参加しない(measure/arrangeが呼ばれない)ため`#[native]`直接指定で
 │                       この枝に入らない——`Window`と同じ扱い、付録H.2.1a・builtins.elwindの
 │                       `NativeControl`マーカー自身のコメント参照)
 ├─ TextBlock            (プリミティブ描画・非native、付録F.3)
 ├─ Shape => Rectangle, Ellipse (プリミティブ図形、子を持たない。付録F.6)
 ├─ Control              (Padding + ContentAlignmentを持つ、複数の小部品からなる複合部品。
 │   │                    `children: UIElementCollection`をLogicalツリーとして持つ、付録H.2.2)
 │   └─ ContentControl   (Content1つだけを持つ複合部品、`inherits`によるDSL合成の実例。付録E)
 └─ Layout => VerticalLayout, HorizontalLayout, Grid (レイアウトコンテナを束ねる共通親。
                          `builtins.elwind`上もDSLの`component Layout inherits UIElement { children:
                          UIElementCollection }`として存在し、`children`はVerticalLayout/
                          HorizontalLayout/Gridへ自動的に継承される(付録H.2.2)。付録F.2・付録F.11)
```

`Layout`は`children: UIElementCollection`という1フィールドのみを持ち、`VerticalLayoutImpl`/
`HorizontalLayoutImpl`/`GridImpl`がこれを実装する。`VerticalLayoutImpl`/`HorizontalLayoutImpl`は
さらに、DSL上には現れない共通の内部実装`StackImpl`(`orientation`/`spacing`/`children`を持つ)を
`base`フィールドとして共有し、`UIElement`をそこへ委譲する——`NativeControlImpl<H>`を`Button`/
`TextArea`/`TabView`が共有するのと同じ trait+Impl+base の形(付録H.2.1a)。`GridImpl`は
`rows`/`columns`/`children`を自前で持ち、`StackImpl`は経由しない。

`VerticalLayout`/`HorizontalLayout`/`Grid`はいずれも自前の`view`を持たない仮想ビルトインなので、
`Layout`の`children`フィールドを無条件に継承する(`resolve_effective_fields`)——各コンポーネント
側で`children`を再宣言する必要はない。ただし`#[content(children)]`(WinUI3の
`ContentPropertyAttribute`相当、下記H.2.2参照)はフィールドと違い継承されないため、3つとも
個別に宣言している。

いずれも実装型は`XxxImpl`(`StackImpl`/`VerticalLayoutImpl`/`HorizontalLayoutImpl`/`ShapeImpl`/
`TextBlockImpl`/`ControlImpl`/`GridImpl`/`NativeControlImpl<H>`)で、対応する`create_xxx(...)`
クラス自身の`new()`で`Rc`化してから、各コレクションの`add()`で子を木に組み込む。

`NativeControlImpl<H>`の判定は`UIElement`の`as_native_control(&self) -> Option<&dyn Any>`という
デフォルト`None`のメソッド経由で行う(`NativeControlImpl<H>`自身が`Some(self)`を返す)。単純な
`AsAny`経由の`downcast_ref::<NativeControlImpl<H>>()`ではなく、この一段の間接参照を挟むのは、
`ButtonImpl`のように`base: NativeControlImpl<H>`を**自分自身のフィールドとして合成する型**
(付録H.2.1a)がある場合、木に置かれる実際の具象型は`ButtonImpl`であって`NativeControlImpl<H>`
そのものではなく、`Any::downcast_ref`は実際の具象型に対してしか成功しないため——`ButtonImpl`は
`as_native_control`を`Some(&self.base)`とオーバーライドして委譲する。「実ハンドルを持つ」という
概念を持たない大多数の実装(`VerticalLayoutImpl`/`HorizontalLayoutImpl`/`GridImpl`/`Shape`/
`TextBlock`/`Control`)は既定の`None`のままでよく、
不要なボイラープレートを背負わない。

`Window`は`UIElement`を派生しない。WinUI3の`Window`が`UIElement`ではなく独立したトップレベルの
ホストであるのと同様、`Window`は`content: Rc<dyn UIElement>`を保持し自身のクライアント領域に
対して`measure`/`arrange`を呼び出す**ホスト**である(AppKitの`TreeHostView`/WinUI3の
`TreeHostPanel`がこの役割を実装する)。

`VerticalLayout`/`HorizontalLayout`は交差軸方向の配置を一律設定として持たない
(かつての`CrossAlign`パラメータは廃止)——各子要素自身の`horizontal_alignment`/
`vertical_alignment`が交差軸配置を決める、WinUI3の`StackPanel`と同じ設計である。主軸方向は
常に「Auto」(子の自然サイズ)である。

`Grid`(実装済み、docs/elwindui_builtins_spec.md参照)は行/列ベースのレイアウトで、
`VerticalLayout`/`HorizontalLayout`にはない「残り領域を`*`比例配分で埋める」手段
(`GridLength::Star`)を提供する。各子の行/列位置は§3の添付
プロパティ(`Grid::row`/`Grid::column`)で指定し、`UIElementImpl.grid_cell`(既定`(0, 0)`)として
子要素自身が保持する——`Grid`自身が子ごとの別テーブルを持つわけではない。

## H.2.2 Logical/Visualツリーの分離

WinUI3に倣い、「`.elwind`で書かれた見た目上の参照関係」(Logicalツリー)と
「実際にlayoutされる`Rc<dyn UIElement>`の木」(Visualツリー)を区別する。既存の
`component`+`view`パターン(例:`DocumentView`)は、実質的に既に「1つの論理ノード →
展開された`UIElement`木」というLogical/Visual構造を持っている。

- **Logicalツリー**:`.elwind`上の参照関係(例:`NotepadWindow`から見て`DocumentView`は1個の
  ノード)。将来のテンプレート機能・アクセシビリティツリーはこちらを対象にする。今回は
  `LogicalNode { type_name, children }`という最小限の型のみ導入し、コード生成側からはまだ
  生成されない(将来のテンプレート/データバインド機能向けの受け皿として未使用のまま残す)。
  `Layout`(`VerticalLayout`/`HorizontalLayout`/`Grid`)/`Control`が`.elwind`上で宣言する
  `children: UIElementCollection`(下記)は、この`LogicalNode`とは別の、より具体的な仕組み——
  これらのコンテナはテンプレート機構を持たない(1つの`.elwind`宣言がそのまま1つのVisual
  ノードになる)ため、Logical上の子要素リストが同時にVisual上の子要素そのものでもある。
- **Visualツリー**: 実際にlayoutされる`Rc<dyn UIElement>`の木(`Layout`/`Shape`/`TextBlock`/
  `NativeControl`/`Control`から組み立てられる)。H.2.1・付録Fで説明している木はこちら。
  `UIElement::visual_children(&self) -> &[Rc<dyn UIElement>]`がこの木を歩く汎用関数
  (`measure`/`arrange`/`hit_test`)から参照される、Visualツリー専用のアクセサ。

`elwindui_core::ui::UIElementCollection`はWinUI3自身の`UIElementCollection`に相当する型で、
`Layout`/`Control`が`.elwind`上で`#[content(children)]`(WinUI3の`ContentPropertyAttribute`
相当)付きで宣言するLogicalツリーの子要素リストを表す。`StackImpl`/`ControlImpl`/`GridImpl`は
これを実フィールドとして保持し、`visual_children()`はそこから`as_slice()`で直接導出される
(`UIElementCollection::new(Vec<Rc<dyn UIElement>>) -> Self`/`as_slice(&self) ->
&[Rc<dyn UIElement>]`のみを持つ薄いラッパー)。`Shape`(`Rectangle`/`Ellipse`)は実WinUI3の
`Shape`同様、子要素を一切持たない純粋なリーフである。

`Control`(H.2.1参照)は「Logical上は1ノード、Visual上は複数の小部品」という構造を体現する型として
導入された——`Padding: f32`(一律)と`ContentAlignment`(`HorizontalAlignment`/`VerticalAlignment`
の組)、および`children: UIElementCollection`を持ち、WinUI3の`Control`基底クラス(独自描画では
なく複数の小部品を持てるカスタム部品)に相当する。

## H.2.3 再描画要求(`invalidate`)と`RelayoutHost`のコアレシング契約

`UIElement`は`invalidate`/`invalidate_measure`/`invalidate_arrange`(WinUI3の
`InvalidateVisual`/`InvalidateMeasure`/`InvalidateArrange`相当)を持つ。見た目(サイズ・配置・
描画内容)に影響する値を変更するプロパティセッター(`TextBlock::set_text`、`Shape::set_fill`、
`UIElementImpl::set_margin`等)は、値を書き換えた後に必ずこのいずれかを呼び、自分がホストされている
ツリーへ再レイアウトを要求しなければならない。呼び忘れると、モデル側の値は正しく更新されているのに
画面には一切反映されない(コード生成が`resync()`から`set_text(...)`等を呼んでも無効化されない)、
という不具合になる。

```rust
trait UIElement {
    fn invalidate(&self);          // 既定実装: request_relayout(self.base())
    fn invalidate_measure(&self);  // 同上
    fn invalidate_arrange(&self);  // 同上
}
```

`elwindui-core`のレイアウトエンジンは要素ごとのMeasure/Arrangeキャッシュを持たない(H.2の
`layout_root` は measure/arrange を実行し、host 保持の `RenderTree::reconcile` が描画 tree を同期する)ため、上記3メソッドは現状すべて同一の
`request_relayout`——ホストされているツリーの根まで`parent()`を辿り、そこに登録された
`RelayoutHost`(`UIElementImpl::invalidate_host`)へ再レイアウトを依頼する——に帰着する。3つを
分離してあるのは将来Measure/Arrangeを別々にキャッシュ・再計算できるようにするための拡張余地であり、
現時点で意味の使い分けはない。

**`RelayoutHost::request_relayout()`の契約**: 「呼ばれた**今すぐ**」ではなく「しかるべき
タイミングで**1回だけ**」ツリー全体の再レイアウトを行うことを、実装する各バックエンドに義務付ける。
具体的には:

1. 同一の同期実行区間内(例:1回の`resync()`が複数の`set_*`を呼ぶケース)で
   `request_relayout()`が複数回呼ばれても、実際の再レイアウトは高々1回にまとめる(コアレシング/
   デバウンス)。
2. 実際の再レイアウトは、その場で同期的に行うのではなく、ホストのUIイベントループの次のタイミング
   (次の描画サイクル/次のディスパッチキュー実行)に委ねる。
3. 「今すぐ全部やり直す」実装(呼ばれるたびに同期的にツリー全体を再構築する実装)はこの契約に
   違反する——ツリーが大きい場合や1回の`resync()`が複数のセッターを呼ぶ場合に、無駄な再構築が
   その回数分発生してしまうため。

各バックエンドでの実装方針:

| バックエンド | `request_relayout`の実装 |
|---|---|
| AppKit | `NSView.setNeedsLayout(true)`を呼ぶだけでよい——AppKit自身が次の描画サイクルまでに1回へコアレシングする(追加のフラグ管理は不要)。 |
| WinUI3 | 実ネイティブの`Canvas.Children`を毎回同期的に総入れ替えする実装のため、`pending: Cell<bool>`で多重エンキューを防いだ上で、この`DispatcherQueue.TryEnqueue`で実際の再レイアウトをUIスレッド上で1回だけ実行する(`elwindui_backend_winui3::WinUI3RelayoutHost`参照)。 |
| GTK4(将来実装時) | `gtk_widget_queue_allocate`/`glib::idle_add_local`等、同種の「次のイベントループ反復まで間引く」機構を使うこと。 |

新しいバックエンドを追加する際・既存バックエンドの`RelayoutHost`実装をレビューする際は、この
コアレシング契約を満たしているかを確認すること。

## H.3 フォーカス管理

> **実装状況**: `FocusManager`/`AccessibilityNode`/`InputRouter`トレイトは`elwindui-core`(`focus.rs`/`accessibility.rs`/`input.rs`)に実装済み。ただし`.elwind`側の`#[focus(order/trap)]`/`#[accessible(...)]`宣言構文は`elwindui-codegen`のパーサーに未実装で、現状はRustコードから各トレイトを直接使う形にとどまる。

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

## H.5 Core Runtimeの位置づけ(クレート構成)

```
elwindui-core           # UIElement, LayoutEngine, FocusManager, AccessibilityTree, InputRouter, Painter(共通・バックエンド非依存)
elwindui-backend-winui3 # elwindui-coreを実装 + windows-rsでネイティブAPIに橋渡し(実装あり、Windows環境未検証)
elwindui-backend-appkit # 同上、objc2経由(実装済み・動作確認済み)
elwindui-backend-gtk4   # 同上、gtk-rs経由(現状スタブのみ)
```

`.elwind`コンパイラが生成するコードは常に`elwindui-core`のトレイト境界に対して書かれ、実行時にどのバックエンドクレートがリンクされるかで実体が決まる(付録D`target::backend()`と対応)。

## H.6 まとめ

| 要件 | 対応 |
|---|---|
| フォーカス管理の共通化 | `FocusManager`トレイト + `#[focus(order/trap)]`属性、ネイティブ系はOS機構とミラー同期 |
| アクセシビリティの共通化 | `AccessibilityNode`トレイト + `#[accessible(role/label/state)]`属性、各バックエンドはOS標準のa11y APIに登録 |
| レイアウト計算の共通化 | `LayoutNode`(Measure/Arrange)を`elwindui-core`で一元計算し、バックエンド間の見た目のズレを防止 |
| WinUI3方式の要素階層 | `UIElement`トレイト(非ジェネリック)+ `NativeControl<H>`(実ハンドルを保持する唯一の型)+ `AsAny`によるダウンキャスト(H.2.1) |
| Margin/Alignment | `UIElementBase`(一律`f32`のMargin、`HorizontalAlignment`/`VerticalAlignment`、既定`Stretch`)を全`UIElement`が共通して持つ(H.2.1) |
| Logical/Visualツリーの分離 | `.elwind`上の参照関係(Logical、`UIElementCollection`)と実際にlayoutされる`Rc<dyn UIElement>`の木(Visual、`visual_children()`)を区別、`Control`/`Layout`がその橋渡し(H.2.2) |
| WinUI3ライクな基盤全体 | `elwindui-core`という共通クレートに集約し、各バックエンドがこれを実装する構成 |
| 独自部品(付録G)との整合 | `Canvas`ベースの部品は`#[accessible(...)]`の明示を推奨(付けない場合は静的警告) |

---

# 付録I. ライフサイクルフック

コンポーネントの生成時・破棄時・更新後に副作用のあるコードを挟むための仕組み。`view`ブロック内の先頭に宣言する。

**実装状況**: `on_mount`は実装済み(生成される`new()`の中、`resync()`直後にそのまま展開される)。`on_unmount`はパース・検証・コード生成は実装済みだが、`elwindui_core::ui`に要素の破棄(detach/teardown)通知が現状存在しないため、実行時に呼び出されるトリガーはまだない(`__run_on_unmount`という到達可能なメソッドとしては生成される)。`on_update`(I.2)、およびI.3の`#[param]`不変性の静的検証は未実装。§3で述べた`inherits`の`base::on_mount()`/`base::on_unmount()`呼び出しは実装済み(1階層のみ)。

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

## I.4 `UIElement`トレイトとの関係

`on_mount`/`on_unmount`は13章で定義した`UIElement`トレイトの生成・破棄タイミングに対応する。コード生成器は各バックエンドのライフサイクル(WinUI3の`Loaded`/`Unloaded`イベント、AppKitの`viewDidAppear`/`viewWillDisappear`、GTK4の`realize`/`unrealize`)にこれらのフックをマッピングする。この変換自体はビルトイン側の責務であり、通常の`component`では意識する必要はない。

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

> **実装状況**: `store`宣言構文は`elwindui-codegen`のパーサーに未実装(現状の`bind!`/`#[observable]`/`#[computed]`は`viewmodel`向けにのみ実装されている)。本付録は設計のみ。

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

> **実装状況**: 本付録の`#[focus(order/trap)]`・ショートカット構文は`elwindui-codegen`に未実装。本付録は設計のみ。

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
- コード生成時に、macOS向けビルドでは`Ctrl`が自動的に`Cmd`に読み替えられる(WinUI3/GTK4等の他backendではそのまま`Ctrl`として扱う)、というプラットフォーム変換規則を標準で持つ
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

### `viewmodel`の2つの書き方と`use`

`viewmodel`は上記のようにDSLネイティブな`.elwind`構文として書く以外に、WPF/WinUI3のMVVMがViewModelを
ホスト言語側に置くのと同様、通常のRustファイルに`#[elwindui::viewmodel]`属性付きの`mod`として書くことも
できる:

```rust
// main.rs (通常の.rsファイル。.elwindではない)
#[elwindui::viewmodel]
mod notepad_view_model {
    struct NotepadViewModel {
        #[observable(default = Vec::new())]
        documents: Vec<Document>,

        #[command(can_execute = documents.len() > 0)]
        save: Command,
    }

    impl NotepadViewModel {
        async fn save(&self) { /* ... */ }
    }
}
```

どちらの書き方でも、Viewからは§12の規則どおり**実際のRustパスを`use`する**。前者(DSLネイティブ)なら
その`.elwind`ファイルが実際にコンパイル・配置される先のパス、後者(Rust属性マクロ)なら`mod`が実際に
宣言されているパス(上記の例なら`crate::notepad_view_model::NotepadViewModel`)である。`elwindui::
viewmodel::X`のような、どちらの実装にも対応しない架空の名前空間は使えない。

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
`NotepadViewModel`を実際に参照するには、O.2で述べた実パスを`use`しておく必要がある(§12)。例えば
`NotepadViewModel`がO.2のRust属性マクロ例のように`main.rs`の`mod notepad_view_model`として定義されて
いるなら、このファイルの先頭で`use crate::notepad_view_model::NotepadViewModel;`とする。

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
                    TextBlock { text: vm.state.label() },
                    TextBlock { text: t!("notepad-status-chars", count: vm.char_count) },
                ]
            }
        }
    }
}
```

- 双方向バインディングが必要なフィールド(`TextArea`の`content`等)は、これまで通り`component`側の`prop`として`bind!(vm.field, TwoWay)`で写し取る(J.2と同一パターン)
- 読み取り専用の表示(`vm.window_title`, `vm.char_count`, `vm.state.label()`)は、`view`式の中で直接参照してよい。これは14章ルール13の対象外である(ルール13は`#[param]`初期化式への直接参照のみを禁止しており、通常の`view`式は元々動的評価が前提のため制限しない)

### O.4.1 `command`属性(WinUI3の`Command`プロパティ相当の糖衣構文)

上記の`on_click`/`enabled`を`Command`ごとに2属性書く代わりに、`command`属性1つで両方をまとめて指定できる:

```
Button {
    text: t!("notepad-menu-save")
    command: vm.save
}
```

これは`on_click: vm.save.execute()` + `enabled: vm.save.can_execute`を書いた場合と**完全に同じ**コードを生成する糖衣構文であり、`command`という実体を持つ値やフィールドが新たに導入されるわけではない(O.5の方針どおり、`Command`は各`viewmodel`ごとに単相化された`execute`/`can_execute`メソッドへ静的に展開されるだけで、実行時に受け渡し可能な共通の`Command`型の値は存在しない)。

- 展開先の「トリガーとなるイベント」は、対象の`component`/ビルトインが自前で宣言している**唯一の`on_*`フィールド**から自動的に決まる(`Button`なら`on_click`、`MenuItem`なら`on_select`)。`on_*`フィールドが0個または複数ある場合は展開できず、`command`属性は単に無視される。
- `enabled`フィールドを持つ対象であれば、`can_execute`への結線も自動的に追加される。持たない対象では`execute`側の結線のみ行われる。
- 特定のウィジェット名に対するハードコードではなく、「`on_*`フィールドを1つだけ持つ」という構造だけを見て展開されるため、ビルトインに限らずユーザー定義の`component`(ネイティブ・仮想いずれも)が自前で唯一の`on_*`イベントを宣言していれば同様に使える。
- 同じ要素に`command`と明示的な`on_click`/`enabled`を両方書いた場合、明示的な指定が優先される(`command`側の展開はまだ設定されていない属性を補うだけで、上書きはしない)。

### O.4.2 `resync()`と購読(いつ再同期されるか)

コード生成器は初期構築時だけ全属性を設定する`resync()`を生成する。以後の更新はブランケットな
再同期ではなく、各`viewmodel`の型付き`PropertyChanged`を購読して行う。購読コールバックは
`PropertyId`ごとの静的に生成された更新関数を呼び、当該プロパティを参照する属性だけを再評価する。

- `#[observable]` の setter は代入後に必ず対応する `PropertyId` を通知する。`#[computed]` と
  `can_execute` は依存する setter の後で再計算され、値自身の `PropertyId` も通知する。
- 二方向バインディングの widget→model 側は setter の呼び出しだけを行う。setter の通知による
  model→widget 更新は同じ値なら native setter の no-op となるため、編集状態を破壊しない。
- `Subscription` は表示オブジェクトが保持し、表示オブジェクトまたは動的領域の破棄時に Drop で
  解除される。通知中の購読追加・解除も安全でなければならない。
- 子 viewmodel の変更を親 viewmodel のコレクション変更として転送しない。たとえば文書本文の変更は
その文書を表示する `TextArea` と文字数表示を更新するだけで、親 `TabView` の children を更新しない。

`on_*` コールバックの後に行う全体 `resync()` は互換目的の初期化以外には用いない。非同期コマンドを
含め、状態変更の反映は常に setter の `PropertyChanged` 通知により行われる。

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

`viewmodel`は`view`を持たず、ビルトイン要素にも依存しないため、**バックエンド(WinUI3/AppKit/GTK4)を一切起動せずに単体テストできる**。

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

> **実装状況**: `elwindui-core::task`の`spawn_local`(P.5相当のタスク起動プリミティブ)は実装済みで、`examples/notepad`の非同期ファイル保存/読込(`platform::file_dialog`)で実際に使われている。一方`AsyncState<T>`enum・`#[async_computed]`属性・`task!`マクロは本付録の設計のみでコード中に実体がなく未実装。`#[observable]`/`#[computed]`/`command!`(付録O)は実装済み。

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
    AsyncState::Idle    => TextBlock { text: "" }
    AsyncState::Loading => Spinner {}
    AsyncState::Success(text) => TextArea { text }
    AsyncState::Error(msg)    => TextBlock { text: msg, color: "#e74c3c" }
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

## P.6 まとめ

| 要件 | 対応 |
|---|---|
| 非同期データ取得の状態表現 | `AsyncState<T>`(enum、網羅性検査対象) |
| propの非同期算出 | `#[async_computed]` + `task!(async { ... })` |
| 非同期Command(多重実行防止込み) | `#[command(async, can_execute: ...)]` + `command!(async || { ... })` |
| キャンセル | `#[command(async, cancellable)]` + `vm.command.cancel()` |
| ランタイム統合 | `elwindui-core::spawn`を各バックエンドがホストの非同期基盤に橋渡し |

---

# 付録R. テーマ/デザイントークン

`style{}`(6章)は個別属性の上書きにとどまるため、カラーパレット・スペーシング・タイポグラフィを一元管理する`theme`構文を追加する。

> **実装状況**: `theme`構文は`elwindui-codegen`のAST(`ast::Item`)に対応する項目がなく未実装。本付録は設計のみ。

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

> **実装状況**: `ErrorBoundary`ビルトイン・`#[command(catches: ...)]`ともに`crates/elwindui-codegen/src/builtins.elwind`にはまだ定義がなく未実装。本付録は設計のみ。

## S.1 `ErrorBoundary`ビルトイン

```rust
view App {
    ErrorBoundary {
        fallback: |err| TextBlock { text: t!("error-fallback", message: err.to_string()), color: "#e74c3c" }

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

# 付録U. Undo/Redo共通パターン

編集操作の取り消し・やり直しを、`viewmodel`のフィールドに対する共通の仕組みとして提供する。

> **実装状況**: `#[undoable]`属性は`elwindui-codegen`の`Attr`列挙体に未実装。本付録は設計のみ。

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

> **実装状況**: `elwindui-test`クレートは`render_tree`(V.1)のみ実装済み。`render_canvas_snapshot`/`assert_image_snapshot!`(V.2)は未実装。

## V.1 要素ツリーのスナップショット

```rust
#[test]
fn notepad_initial_view_matches_snapshot() {
    let vm = NotepadViewModel::new();
    let tree = elwindui_test::render_tree(&NotepadWindow { vm });
    assert_snapshot!(tree);
}
```

- `render_tree`は`UIElement`ツリー(13章)を、各ノードの型名(`UIElement::type_name()`)によるテキスト表現(インデント付きの構造ダンプ)に変換する
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
- 新しいバックエンド種別(テスト専用のBackend variant等)は追加せず、既存の各ネイティブバックエンドのオフスクリーン描画機能を用いる想定(付録D同様、`Backend`enum/`target::backend()`自体が未実装のため、この網羅性検査は現状働かない)

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

> **実装状況**: iOS(UIKit)/Android(Jetpack)バックエンドクレートはワークスペースに存在せず未着手。付録D自体が未実装のフォワードルッキング設計であるため、`Backend` enumへの`Uikit`/`Jetpack`追加も含め本付録は設計のみ。

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
