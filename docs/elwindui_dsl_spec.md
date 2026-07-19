# ElwindUIL DSL仕様書

Rust向けGUIフレームワーク(Elwind)のための宣言的レイアウト記述言語(ElwindUIL)の構文・意味論の仕様書。
Rustの構文・慣習に寄せることで学習コストを下げつつ、機械可読性・事前検証性を重視した設計。

本書はDSLの構文・静的検証ルールのみを対象とする。バックエンド抽象化・`elwindui-core`ランタイム・ライフサイクル・Store/ViewModel/MVVM等のGUIフレームワーク本体の設計は `docs/elwindui_gui_framework_design.md`、`builtin::`名前空間の個別UI要素・`platform::`名前空間のOS機能は `docs/elwindui_builtins_spec.md`、コード生成・LSP・プレビュー・ホットリロード等のツールチェーンは `docs/elwindui_tool_*_design.md` を参照。

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

1. **`Base`が`NativeControl`マーカー** — 純粋なカテゴリタグ(フィールド継承なし)。ネイティブ実装を持つ末端要素(`Button`等)であることを示すのみ。意味のある継承元を持たない末端要素(例:`Window` — 実際のWinUI3では`Control`ファミリーを経由せず`Object`を直接継承する)は、`inherits NativeControl`で存在しない共通の祖先を示唆する代わりに`inherits`自体を省略し、`#[native]`属性(付録A)を直接付与する。
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
- `base::name(...)` — オーバーライドした本体から基底実装を呼び出す(C#の`base.Method()`相当)。同じ書き方で`on_mount`/`on_unmount`(`docs/elwindui_gui_framework_design.md`§6.1)内から基底のライフサイクルフックを呼ぶこともできる
- 継承・オーバーライドは1階層(直接の`inherits`先)のみ保証される。2階層以上に渡る`base::`連鎖は現時点では未対応

`#[computed]`フィールドも同様に、基底の同名フィールドを`#[override]`なしで再宣言するとエラーになり、`#[override]`を付けると上書きとして扱われる(型は基底と一致していなければならない)。

`#[overrides(builtin::X)]`(付録A)は`inherits`とは別の、無関係な仕組みである — 前者は同名builtinの明示的なシャドーイング、後者はクラス階層の構築であり、混同しないこと。

### Rustファイル内での代替記法:`#[elwindui::component]`

`component`/`view`ブロックのペアは、`.elwind`テキストとして書く代わりに、通常のRustファイル中で
属性マクロ`#[elwindui::component(inherits Base)]`を使って1つの`struct`として書くこともできる
(`inherits`が無い場合は引数省略)。フィールドは`#[param]`/`#[prop]`等の属性を伴う通常のフィールドと
してそのまま書き、`view { .. }`の要素ツリーは`view!`マクロ呼び出しを型に持つ1フィールド(フィールド名は
任意)として表現する。冒頭のVolumeControlの例を、今度は`ContentControl`を継承する形で書き直した記法
(`ContentControl`は`Control`から見て既にシェイプ合成済みのDSLコンポーネント=上記2番目のケースなので、
`content`フィールドが再宣言なしに自動継承され、`view`の中身はラッパーを書かず暗黙に`content`属性になる):

```rust
#[elwindui::component(inherits ContentControl)]
struct VolumeControl {
    #[param]
    orientation: Orientation,

    #[prop(default = 50)]
    volume: i32,

    #[computed(expr = volume.to_string() + "%")]
    label: String,

    body: view! {
        // `ContentControl { .. }` というラッパーは書かない — 裸の子要素がそのまま暗黙に`content`に
        // バインドされる(付録Eの`#[content(field_name)]`規約)
        match orientation {
            Orientation::Horizontal => { HorizontalLayout { TextBlock { text: label } } }
            Orientation::Vertical => { VerticalLayout { TextBlock { text: label } } }
        }
    }
}
```

- 通常のRust `struct`フィールドには`field: Ty = expr`という初期化式の構文が無いため、デフォルト値・
  算出式は`#[prop(default = expr)]`/`#[computed(expr = expr)]`/`#[attached(default = expr)]`のように
  対応する属性自身の名前付き引数として渡す——`viewmodel`側の`#[observable(default = expr)]`/
  `#[computed(expr = expr)]`(`docs/elwindui_gui_framework_design.md`§7.2)と同じ仕組み
  (`crates/elwindui-codegen/src/attr_frontend.rs`が両方の`struct`フロントエンドで共有)
- `#[param]`フィールドは現状この属性マクロ経由ではデフォルト値を持てない(対応する`default = expr`
  引数が実装されていない)ため、`orientation`は常に必須のコンストラクタ引数になる——`.elwind`
  テキスト形式の`#[param] orientation: Orientation = Orientation::Horizontal`のようなデフォルト付き
  `#[param]`は、この記法では今のところ表現できない
- `volume`は`#[prop(default = 50)]`という通常の(実行時可変な)`prop`——ここでは自己完結した例にする
  ため固定値をデフォルトにしているが、外部の`store`/`viewmodel`へ結びつけたい場合は呼び出し側で
  `bind!`を使う(`#[prop(default = bind!(..))]`のようにフィールド宣言自体に埋め込むのではなく、
  §10のバインディング機構は`.elwind`テキスト形式で書くのが素直)
- `component`自身の`#[prop(default=...)]`/`#[computed(...)]`フィールドを、その同じcomponentの
  `view`から裸の識別子で参照できる(`text: label`)——コード生成側は、`volume`が変わるたびに
  `label`を再計算して該当するビューノードだけを再同期する専用の`{Component}Property`通知機構を
  生成する(`mutable_required_names`と同型、`docs/elwindui_gui_framework_design.md`§7.2の
  `viewmodel`の仕組みをcomponent自身のフィールドにも広げたもの)。ただし依存関係の検出は
  `#[computed(expr = ...)]`の式を実際に`syn`で走査して見つかったフィールド参照に限られるため、
  `t!(...)`の中に書かれた参照以外は、マクロの不透明なトークン列に隠れた参照(例:
  `format!("{volume}%")`のようなインライン捕捉)を検出できない——`volume.to_string() + "%"`の
  ように、参照したいフィールドを実際の`syn::Expr`として現れる形で書く必要がある
- `match`の条件式(`orientation`)は裸のフィールド参照のみで、`if orientation ==
  Orientation::Horizontal`のような比較式はDSLの`if`条件文法では扱えない(現状`if`/`match`の
  条件式パーサは裸のパス参照程度しか受け付けない)——enumによる分岐は`if`の等値比較ではなく
  `match`を使う
- `#[elwindui::component]`の引数は`inherits Base`のみ(`=`は付けない——`.elwind`側の
  `component Name inherits Base`と同じ綴り。`#[elwindui_macros::class]`の`inherits = ..`とは
  流儀が異なるので混同しないこと)
- `view!`は実在するマクロとして展開されるわけではない——`#[elwindui::component]`が`struct`全体を
  丸ごと別のコードへ置き換えるため、`view!`呼び出しのトークンはRustの型位置(`syn::Type::Macro`として
  構文的に妥当)を借りた記法にすぎず、生のDSLテキストとして読み出されて既存のパーサへそのままかけられる
- 呼び出し側(インスタンス化)は`.elwind`形式で書いた場合と完全に同一——通常のRustの`let`束縛を使う
- `viewmodel`にも同様の代替記法(`#[elwindui::viewmodel] mod foo { .. }`)があり、`#[bindable]`
  (`docs/elwindui_gui_framework_design.md`§7.2)経由で`component`側と結線する
- `#[virtual]`/`#[override]`メソッドはこの記法では未対応——バインドできる`impl`本体の置き場所が
  自然には定まらないため(`.elwind`テキスト形式でのみサポート)
- `.elwind`テキスト形式(build.rs方式)との比較・使い分けの指針は`docs/elwindui_tool_codegen_design.md`
  §4を参照。実装は`elwindui_macros::component`(`elwindui::component`として再エクスポート)で、
  `examples/notepad-inline`が実例

### `#[elwindui::template]`:再利用可能な名前付きテンプレート(Rustファイル内の代替記法)

> **実装状況**: 設計のみ・未実装。`#[elwindui::component]`/`#[elwindui::viewmodel]`と同系統だが、専用の`fn`向け属性マクロとしてはまだ存在しない。

`template: |control| Grid { .. }`のようなインライン値クロージャ(前節「`ControlTemplate<Self>`」参照)はその場限り(1箇所)の書き方しかできない。複数のコンポーネントで同じテンプレートを使い回したい(WinUI3で`ControlTemplate`を`Style`リソースとして共有するのと同じ用途)場合のために、`#[elwindui::component]`(`struct`に付与)・`#[elwindui::viewmodel]`(`mod`に付与)と同系統の、**単一の`fn`に付与する新しい属性マクロ**を用意する:

```rust
#[elwindui::template]
fn button_template(inst: &Button) -> Rc<dyn UIElement> {
    Row {
        Rectangle { .. }
        inst.content
    }
}
```

- パラメータは必ず1個。型注釈は普通のRustとして必須(`.elwind`の値クロージャと違い、これは生Rustの`fn`宣言なので型省略はできない)。戻り値の型は`Rc<dyn UIElement>`固定
- `#[elwindui::component]`(`elwindui_macros::component`、`crates/elwindui-macros/src/lib.rs`)と同じトリック——`fn`の本体ブロックをRustとして解釈させず、生のDSLテキストとして`elwindui-codegen`の既存パーサに渡し、パラメータ名(`inst`)を「テンプレート対象インスタンス」として束縛した状態でコード生成する想定(`elwindui-codegen`側に姉妹フロントエンドを追加する実装になる見込み)
- 値としての参照は裸パス(`template: button_template`)。これは`ControlTemplate<Self>`型フィールドへの裸パス代入の規則(前節参照——関数アイテムそのものを値として使う、既存の0引数呼び出し糖衣とは別の意味)に従う。パラメータ型が厳密にフィールドの`Self`と一致しない関数を指している場合はエラー(14章ルール29)
- `docs/elwindui_tool_codegen_design.md`§4.2/§4.3も参照(`component`/`viewmodel`と並ぶ3つ目のRust代替記法として言及)

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
- 実装は`(owner, field) -> Box<dyn Any>`の型消去された汎用バッグ(`UIElement::attached`)——
  `builtin::Grid`の`row`/`column`もこれ経由で格納される。オーナー自身が自分の宣言した型を知っている
  ので、書き込み側(`elwindui-codegen`の`emit_attached_setters`、`SymbolTable`の
  `TypeInfo::attached_field_types`から宣言型を引いて`set_attached::<T>(..)`のターボフィッシュに渡す)
  と読み出し側(例:`elwindui_core::ui::grid_cell_of`、`get_attached::<i32>(..)`)の双方が
  そのオーナーの持つスキーマ通りにdowncastする——WPFの添付プロパティも同じ設計。将来別のcomponentが
  独自の添付プロパティを持つ場合、`UIElement`側・`elwindui-codegen`側とも変更は一切不要で、
  そのcomponent自身の`#[attached]`宣言と読み出しロジックを追加するだけでよい
- 添付プロパティが実際にレイアウトへ反映されるのは、子要素が仮想ビルトインそのもの(`TextBlock`/
  `Rectangle`/`Ellipse`/`VerticalLayout`/`HorizontalLayout`/`Control`/入れ子の`Grid`)の場合、
  `inherits NativeControl`で各バックエンドの`NativeControl`実装を`base`として合成するネイティブ
  リーフ(`Button`/`TextArea`/`TabView`)の場合、およびユーザー定義の`component`+`view`ペアで
  その`view`ルートがネイティブでない場合(`into_node()`経由で`Rc<dyn UIElement>`として取り出せる場合)
  ——いずれも構築直後に`elwindui-codegen`の`emit_common_ui_element_setters`/`emit_construction`が
  `(erased).base().set_attached::<T>(..)`を呼ぶことで反映される(`docs/elwindui_gui_framework_design.md`§5.1a)。`view`ルート自身が
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
実行時型消去は発生しない(`docs/elwindui_gui_framework_design.md`§7.2の「型消去を避け専用コードを生成する」方針と同じ)。

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

### コールバック型フィールドへのクロージャ値構文

`fn(...)`型のフィールドに実際の値を渡す際の構文。パラメータは型注釈なしの識別子のみ(分解パターン不可)— 実際の型は宣言側の`fn(T0, T1, ...)`から**位置対応**で決まる。パラメータを取らない場合(`fn()`)は、クロージャを書かずベアパスの糖衣構文でも書ける:

```
||  式                       // パラメータ0個、式1つ
|param|  式                  // パラメータ1個、式1つ
|param, param2|  式          // パラメータ2個以上も可
|param, ...|  { 文; ... }    // 複数文のRustブロック本体
|param|  Type { .. }         // ネストした要素構築(値計算コールバック専用、後述)
<パス>                        // パラメータ0個の糖衣構文(`|| <パス>`と同義)
```

```rust
TabView {
    on_select: |index| vm.select_tab(index)     // 1引数、式1つ
    on_close: |index| {                          // 1引数、複数文ブロック
        vm.log_close(index);
        vm.close_tab(index);
    }
    on_new_tab: vm.new_tab                        // 0引数、ベアパスの糖衣
}
```

- `render_content: |item| DocumentView { doc: item }`のような「ネストした要素を返す」形は`#[param]`側の値計算コールバック専有の形で、`on_*`のような通知コールバック(イベントハンドラ)には使えない(要素を返しても配線先がない)
- ブロック本体`{ 文; ... }`は式1つの本体と違い、他のDSL式のような「`vm.field`は自動的にゲッター/アクション呼び出しになる」糖衣を持たない**素のRust**として解釈される — アクションを呼ぶ場合は`vm.close_tab(index)`のように明示的に`()`を書く(`vm.close_tab`だけだと、存在しないフィールドへのアクセスとして扱われコンパイルエラーになる)。`vm`のような参照先の解決(`self.vm`相当への書き換え)自体は式本体と同様に行われる
- クロージャ本体内の`vm.field`/`vm.action(args)`のような参照は、他のDSL式と同じ規則で解決される(コード生成側の詳細は`docs/elwindui_gui_framework_design.md`§7.2参照)

### `ControlTemplate<Self>`:テンプレート型フィールド(WinUI3方式`ControlTemplate`)

> **実装状況**: 設計のみ。本節が前提とする「値計算コールバックがネストした要素を構築する」機構自体(直前の`|param| Type { .. }`)がまだコード生成に実装されていないため、本節はそれに依存する形でさらに未実装。

`ControlTemplate<Self>`は、コンポーネント自身の視覚ツリーを実行時に丸ごと差し替え可能にする専用のフィールド型糖衣。WinUI3の`Control.Template`(`ContentPresenter`等を介した視覚ツリーの丸ごと差し替え、`Style`経由でインスタンス単位に再テンプレート化できる)に相当する。

```rust
component Control inherits UIElement {
    children: UIElementCollection,
    padding: Option<f32>,

    #[prop(default = None)]
    template: Option<ControlTemplate<Self>>,
}
```

- ジェネリック引数は常に文字通り`Self`のみを許す(コンポーネント自身の型)。それ以外を書くとエラー(14章ルール26)。
- 意味的には`Rc<dyn Fn(&Self) -> Rc<dyn UIElement>>`の糖衣だが、単なる`fn(&Self) -> Rc<dyn UIElement>`コールバック糖衣とは扱いが異なる専用の型として区別する:
  - **`prop`必須**(`#[param]`不可、14章ルール27)——直前の「値計算コールバックは`#[param]`側専有」という原則(§4冒頭)に対する**意図的な例外**。テンプレートは実行時に差し替えられて初めて意味があるため、実体化時固定の`#[param]`では目的を果たせない
  - 値が変わったとき、対応する`body`(下記)配下の視覚ツリーを丸ごと再構築するという、通常のプロパティ値の再代入とは異なる**構造的な**再同期が必要(`docs/elwindui_gui_framework_design.md`新設§5.7参照)

**値の書き方**は新しい構文を作らず、直前の「ネストした要素を構築する」値クロージャ構文(`|param| Type { .. }`)をそのまま使う:

```
template: |control| Grid {
    Rectangle { .. }
    control.content
}
```

パラメータ名は普通の識別子(型キーワードの`Self`はここでは使えない——値束縛名としては`control`のような通常の識別子を使う)。クロージャ内から`control.content`/`control.padding()`のように自分自身の他フィールドへ直接アクセスできる。これはWinUI3の`TemplateBinding`(リフレクションベース)の静的型付け版に相当し、既存の「`#[param]`フィールドへの名前付きアクセサ自動生成」(`docs/elwindui_builtins_spec.md`付録F補足)をそのまま使う。

**`body: <field>(Self)`**——`ControlTemplate<Self>`型のフィールドを、自分自身を渡して呼び出した結果を視覚ツリーのルートにする、という新しい`body`/`view`ルートの書き方。`field`名は`template`に限定せず、`ControlTemplate<Self>`型のフィールドなら任意の名前で使える一般規則(14章ルール28: `field`が同一component内の`ControlTemplate<Self>`型フィールドでない場合はエラー)。`builtin::Control`(`docs/elwindui_builtins_spec.md`付録F.9)の例:

```rust
view Control {
    match template {
        Some(t) => t(Self),
        None => /* 既存挙動: children をそのまま Visual 子要素にする */,
    }
}
```

`ControlTemplate<Self>`が返すのは常に単一ルート要素(WinUI3実物の`ControlTemplate`、および本DSLの「単一値フィールドの`if`/`match`は1要素に還元」ルール(§5)と同じ)。`Control.template`が`None`(既定)のときは現行どおり`children`を直接Visual子要素にする——挙動変更なし。

**再利用可能な名前付きテンプレート**は、その場限りのインライン値クロージャの代わりに、独立したRust関数として書いて使い回せる(`#[elwindui::template]`、下記「Rustファイル内での代替記法」参照)。裸パスで参照する:

```rust
Button { template: button_template }
```

`ControlTemplate<Self>`型フィールドへの裸パス代入は、既存の裸パス糖衣(直前、`fn()`型=0引数フィールド専用、`on_new_tab: vm.new_tab`が`|| vm.new_tab()`の糖衣になるもの)とは**意味が異なる**——「0引数で呼び出した結果」ではなく、`#[elwindui::template]`で定義された関数アイテムそのものを値として直接束縛する(14章ルール29)。`ControlTemplate<Self>`型フィールドはそもそも`fn(...)`糖衣とは別の専用型なので、既存の裸パス規則と文法上バッティングはしない。

**広く共有される既定値**(WinUI3の`Style`相当、複数コンポーネントに跨って既定テンプレートを一括変更する用途)は、新しい仕組みを作らず既存の`store`+`bind!`(`docs/elwindui_gui_framework_design.md`§7.1)をそのまま使う。詳細は同節を参照。

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

`#[content(field_name)]`(付録A)で指定した子要素の格納先フィールドがリスト型(`Vec<..>`/`ListExt<..>`)の場合、`if`/`match`/`for`のいずれも使える(前段落の入れ子ルールも同様)。フィールドが単一値型(例:`ContentControl`/`Window`の`content: Rc<dyn UIElement>`)の場合は`if`/`match`のみ使え、`for`は使えない(可変長のリストは単一の格納先に収まらないため)。単一値フィールド配下の`if`/`match`は、入れ子も含めたあらゆる分岐が最終的にちょうど1個の要素に還元できなければならない(1分岐に複数の裸の子要素を書くこともできない)。

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
  `.elwind` ファイルに同名の型が存在するというだけでは可視にならない(ただし`docs/elwindui_tool_codegen_design.md`が示すとおり、複数の
  `.elwind` ファイルが結局同じRustスコープに`include!`される場合は、その同じスコープ内では通常の
  Rustのファイル分割同様`use`は不要)。ローカル定義でも`use`解決でもない型参照は、Rustの「見つからない
  型」エラーと同様、静的検証エラーとなる
- ViewModelの参照(`docs/elwindui_gui_framework_design.md`§7.2)も同じ規則に従う。`viewmodel`を`.elwind`内でDSLネイティブに書いた場合も、
  `#[elwindui::viewmodel] mod foo { .. }`として通常のRustファイルに書いた場合も、参照側は必ずその実パス
  (前者なら`.elwind`ファイルが実際にコンパイルされ`include!`される先のパス、後者なら`mod foo`が実際に
  宣言されているRustパス、例: `crate::foo::Foo`)を`use`する。`elwindui::viewmodel::X`のような、どの
  モジュールにも実在しない架空の名前空間を`use`することはできない

---

## 13. 要素ツリーの探索(UIElement / visual_tree)

### 役割分担の方針

「子要素を持つ」という性質は既存の `{}` ネスト構文がそのまま表現しているため、**children専用の新しいDSL構文は追加しない**。ツリー走査専用の別トレイトは設けず、`docs/elwindui_gui_framework_design.md`§5で定義済みの `UIElement`(全要素が実装する唯一の共通トレイト)が `visual_children()`/`parent()` を通じてこの契約をそのまま担う。再帰探索アルゴリズム自体はDSLの文法ではなく、共通ランタイムライブラリ(`elwindui_core::visual_tree`)側の責務とする。

| 責務 | 担当 |
|---|---|
| 親子構造の宣言 | DSL構文(`{}` ネスト。追加構文は不要) |
| 動的生成された子要素(`if`/`for`/`match`の結果)をchildrenとして集約する規約 | コード生成器 |
| 全要素が親子を辿れるという契約(`visual_children()`/`parent()`) | `UIElement`(`docs/elwindui_gui_framework_design.md`§5.1a、コード生成器が全要素型に自動実装) |
| 再帰探索アルゴリズム(`visual_tree::find_all` 等) | 共通ランタイムライブラリ(DSLとは独立に拡張・最適化可能) |
| 特定要素への後からのアクセス | `#[id(...)]` アトリビュート |

### 共通トレイト(コード生成器が自動実装)

`children()`/`id()`だけのための別トレイトは無い。全要素型が既に実装している`UIElement`(`docs/elwindui_gui_framework_design.md`§5.1a)がその役割を兼ねる:

```rust
trait UIElement: AsAny {
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>>;
    fn parent(&self) -> Option<Rc<dyn UIElement>>;
    // ... 他多数(margin/alignment/measure/arrangeなど、`docs/elwindui_gui_framework_design.md`§5参照)
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
同様にバブルが機能する(`docs/elwindui_gui_framework_design.md`§5.10参照)。実装範囲はAppKit・WinUI3両バックエンドの`Button`のポインタ/タップ9イベント(§5.10)に加え、キーボード/フォーカス系5イベント——`on_key_down`/`on_key_up`/`on_text_input`(バブリング)、`on_got_focus`/`on_lost_focus`(非バブリング、`dispatch_direct`)——も`component UIElement`の`#[routed]`フィールドとして宣言されている(`docs/elwindui_gui_framework_design.md`§5.5/§8.1参照)。WinUI3側はWindows環境が無く未検証。

### 要素使用箇所への注釈:`#[shortcut(...)]`(キーボードショートカット)

`#[routed]`が**フィールド宣言**(全インスタンス共通の配線方式)に付くのに対し、`#[shortcut(...)]`は
`Button { ... }`という**要素の使用箇所**に付く注釈である——ショートカットは本質的にインスタンスごとの
決定(「このSaveボタンだけ`Ctrl+S`」)であり、`builtins.elwind`の`Button.on_click: fn()`という共有宣言
には付けられないため。構文上は`#[id(...)]`(前節)と同じ「要素の`{}`本体内で特定の行の直前に書く注釈」
という位置づけだが、`let`束縛ではなく通常の`属性名: 値`という属性行の直前に書く点が異なる。

```rust
Button {
    text: t!("notepad-menu-save")
    #[shortcut("Ctrl+S")]
    on_click: save_document()
}
```

`#[shortcut(...)]`が付けられるのは`#[routed]`なフィールド(`on_click`/`on_key_down`等)のみ。詳細な構文
(`winui3: "..."`/`appkit: "..."`によるバックエンド別指定、`scope: local`)・プラットフォーム変換規則
(macOSでの`Ctrl`→`Cmd`自動読み替え)・実行時の仕組み(`ShortcutRegistry`)は
`docs/elwindui_gui_framework_design.md`§8.1参照。実装範囲はAppKit・WinUI3両バックエンド(WinUI3側未検証)。

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
- **`#[id(...)]`は全てコンパイル時に確定している**ため、実行時に文字列で検索する仕組みは経由しない — 具象型を直接返す静的アクセサの方が`docs/elwindui_gui_framework_design.md`§7.2の「型消去を避け専用コードを生成する」方針に沿っており、ダウンキャストも不要になる
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

> **実装状況**: `crates/elwindui-codegen/src/validate.rs`は、既に実装済みの言語機能・ビルトインに対応するルール(概ね1〜8, 10〜13, 19, 25, 30〜31 — `#[param]`の静的性、`bind!`経由のstoreアクセス、`viewmodel`のV/VM分離、`#[shortcut(...)]`の妥当性など)を実際に検査する。一方、対応するビルトイン/機能自体が未実装なルール(9: `target::backend()`自体が存在しないため検査不能、14: `NavigationHost`未実装、15: `Dialog`未実装、16・17: `Transition`/`Effect`未実装、20: `#[async_computed]`未実装、21: `#[undoable]`未実装、22: `theme`未実装、23: `VirtualList`未実装、24: `on_foreground`等のモバイルライフサイクル未実装、26〜29: `ControlTemplate<Self>`/`#[elwindui::template]`未実装)は`validate.rs`にも対応する検査が存在しない。ルール18は`Command`機構撤廃(付録O.3〜O.5相当の仕組みが丸ごと廃止)に伴う欠番。

1. `#[param]` フィールドの初期化式に `bind!` / propの参照 / `#[computed]` が出現 → エラー
2. `#[param]` フィールドの初期化式に非純粋関数(`now()`, `random()` 等)が出現 → エラー(`env::*` / `once` 値は例外)
3. `#[computed]` フィールドへの外部代入 → エラー
4. enum値の裸文字列直書き(完全修飾でない参照) → エラー
5. `match` におけるenumメンバーの網羅漏れ(`_ =>` なし) → エラー
6. 制約(`#[range]`, `#[length]`, `#[pattern]` 等)付きフィールドへのリテラル値代入が制約違反 → ビルド時エラー、動的値の場合は実行時エラー
7. `external::*` 呼び出しがトップレベルの `once` 宣言以外の場所に出現 → エラー
8. importの循環・未解決パス → エラー
9. `#[overrides(builtin::X)]` が付いていない通常の`component`の`view`内に `native!` ブロック、または `target::backend()` の参照が出現 → エラー(`docs/elwindui_gui_framework_design.md`§4.1参照。独自部品はバックエンド共通実装に限定する)
10. `view`内に`Canvas`が含まれているが `#[accessible(...)]` が付与されていない → 警告(`docs/elwindui_gui_framework_design.md`§5.6参照)
11. `on_mount`/`on_unmount`ブロックの外で`#[param]`フィールドの再代入相当の操作が行われている → エラー(`docs/elwindui_gui_framework_design.md`§6.1参照。paramの不変性は生涯を通じて保証される)
12. `bind!`の参照先が`store`宣言(`docs/elwindui_gui_framework_design.md`§7.1)の型・フィールドとして存在しない → エラー
13. `store`フィールドへの`#[param]`側からの直接参照(`bind!`を介さない読み取り)→ エラー(`docs/elwindui_gui_framework_design.md`§7.1参照。storeへのアクセスは常に`bind!`を経由する)
14. `NavigationHost`内の`match route { ... }` がRoute enumの全メンバーを網羅していない(`_ =>`なし) → エラー(8章の網羅性検査と同じ仕組み、`docs/elwindui_builtins_spec.md`付録L.2参照)
15. `Dialog`/`Menu`等のオーバーレイ系ビルトインの外側(通常のcomponent)で`native!`/`target::backend()`が出現 → エラー(ルール9と同じ原則、`docs/elwindui_builtins_spec.md`付録M参照)
16. `Transition`/`KeyframeAnimation`(`docs/elwindui_builtins_spec.md`付録N.6)で存在しないイージング関数名、または範囲外のキーフレーム位置(`0.0..=1.0`外)が指定されている → エラー
17. `Effect`(`docs/elwindui_builtins_spec.md`付録N.3)のパラメータが対応バックエンドでサポートされない組み合わせ(例:GTK4未対応のエフェクト種別)である場合 → 警告(該当バックエンドではフォールバック描画に切り替わる旨を明示)
18. (欠番 — `Command`機構撤廃により削除。旧ルールは「`#[command]`が付与されたフィールドの型が`Command`でない → エラー」だったが、アクションはRustの`impl`ブロックの`fn`として自動検出されるようになり、対応する型検査自体が存在しなくなった)
19. `viewmodel`定義内に`view`ブロック、またはビルトイン要素(`Row`/`Text`等)への直接参照が存在する → エラー(`docs/elwindui_gui_framework_design.md`§7.2参照。ViewModelは表示ロジックを持たず、MVVMのV/VM分離を静的に強制する)
20. `#[async_computed]` が `viewmodel`/`store` 以外(通常の`component`のprop等)に付与されている → エラー(`docs/elwindui_gui_framework_design.md`§7.3参照。非同期状態はVM/Model層に閉じ込める)
21. `#[undoable]` が `viewmodel` の `#[observable]` フィールド以外(`store`や`component`のprop等)に付与されている → エラー(`docs/elwindui_gui_framework_design.md`§7.4参照)
22. `theme`の`variant`ブロックが`tokens{}`で宣言されていないトークン名を定義している、または`tokens{}`で宣言された一部のトークンを欠いている → エラー(`docs/elwindui_gui_framework_design.md`§8.5参照。全variant間でトークン集合の一致を保証する)
23. `VirtualList`に`key`が指定されていない状態で`items`の順序が変わる更新が行われる → 警告(`docs/elwindui_builtins_spec.md`付録Q参照。挿入位置ベースの再利用にフォールバックし、リコンサイル効率が低下する可能性がある)。一般の `for` は `Vec<Rc<T>>` のとき各要素の `Rc<T>` ポインタ同一性で子を再利用し、その他の collection は当該範囲を再構築する(`docs/elwindui_builtins_spec.md`付録Y参照)。`TabView` は `TabViewItem` を子として指定する。
24. `on_foreground`/`on_background`/`on_terminate`(`docs/elwindui_gui_framework_design.md`§6.2)が、アプリのエントリポイント(ルート)コンポーネント以外で宣言されている → 警告(OSレベルのライフサイクルは単一箇所への集約を推奨)
25. コールバック型のフィールドで `Rc<dyn Fn(...)>` / `Box<dyn Fn(...)>` のような型消去表現を直接使用している(`fn(...)` 糖衣構文を使っていない) → エラー(4章「コールバック型フィールド」参照)
26. `ControlTemplate<T>` の `T` が `Self` 以外 → エラー(4章「`ControlTemplate<Self>`」参照)
27. `ControlTemplate<Self>` 型フィールドに `#[param]` が付与されている → エラー(実行時差し替えができて初めて意味を持つため、常に`prop`でなければならない)
28. `body`/`view` ルートの `<field>(Self)` の `field` が、同一component内で宣言された `ControlTemplate<Self>` 型フィールドでない → エラー
29. `ControlTemplate<Self>` 型フィールドへの裸パス代入が、`#[elwindui::template]` で定義され、かつパラメータ型が厳密に `Self` と一致する関数を指していない → エラー(4章「`#[elwindui::template]`」参照)
30. `#[shortcut(...)]` が `#[routed]` でない属性に付与されている → エラー(4章「`#[shortcut(...)]`」参照。`on_click`等のコールバック属性以外に付けても意味を持たない)
31. `#[shortcut(...)]` に指定されたキー表記(修飾キー名/キー名)が不正 → エラー(`docs/elwindui_gui_framework_design.md`§8.1参照。`codegen::parse_shortcut_spec`と同じパーサーで検査するため、ここを通れば必ずコード生成もパースに成功する)
32. `elwindui::core::graphics::Brush`/`Color`(または`Option<..>`)型のフィールドへ文字列リテラルを代入する場合(例: `Rectangle { fill: "#3a3a3c" }`)、その文字列が`"#rrggbb"`/`"#rrggbbaa"`(`#`省略可)のいずれの形式にも一致しない → コード生成時エラー(`codegen::coerce_color_literal`。動的な`String`式には適用されない——`Brush`/`Color`型の値を直接渡す必要がある)

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

# 付録A. 名前空間とビルトインのオーバーライド規則

ユーザーが`Button`のようなビルトインプリミティブと同名のコンポーネントを定義し、バックエンドごとの実装を`native!`で明示的に書き下したい場合(`docs/elwindui_gui_framework_design.md`§3の応用)の名前解決規則を定める。**大原則として、暗黙のシャドーイングは一切許可しない。**

## A.1 ビルトインは予約名前空間に属する

```rust
builtin::Button
builtin::TextBlock
builtin::VerticalLayout
builtin::HorizontalLayout
builtin::TextArea
```

- これまで`Button { ... }`等と書いてきた記法は、`builtin::Button`への暗黙の`use`が常に効いている、という扱いにする
- ユーザーが同名の`component`を定義しても`builtin::X`自体は消えず、両者は別の完全修飾名を持つ

## A.2 衝突時の既定挙動:曖昧参照エラー

同一スコープに`builtin::X`とユーザー定義`X`が両方見える状態になった場合、暗黙の優先順位を付けず**静的エラー**とする。

```rust
component Button { ... }   // ユーザー定義

view Foo {
    Button { text: "OK" }   // エラー: builtin::Buttonとユーザー定義Buttonのどちらか曖昧
}
```

## A.3 意図の明示方法(1):別名での共存(推奨)

衝突を避ける最も単純な方法は、ビルトインと異なる名前を付けることである。

```rust
component CustomButton { ... }

view Foo {
    CustomButton { text: "OK" }   // 曖昧さなし
    Button { text: "Cancel" }     // builtin::Buttonがそのまま使われる
}
```

## A.4 意図の明示方法(2):`#[overrides(builtin::X)]`

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

## A.5 `#[overrides]`のスコープ規則

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

## A.6 ビルトインを明示的に指定する逃げ道

オーバーライドが有効なスコープ内でも、あえて元のビルトイン実装を使いたい場合に用いる。

```rust
view Foo {
    builtin::Button { text: "常に組み込み実装を使う" }
}
```

## A.7 静的検証ルールの追加

1. 同一スコープに`builtin::X`とユーザー定義`X`が両方見え、`#[overrides]`が付与されていない → 曖昧参照エラー
2. `#[overrides(builtin::X)]`が付いているが、ビルトイン`X`の必須フィールドを満たしていない → シグネチャ不一致エラー
3. `#[overrides]`の対象が存在しないビルトイン名を指している → 未解決参照エラー
4. 複数のコンポーネントが同じビルトインに対して`#[overrides]`を宣言し、同一スコープで両方が`use`されている → 多重オーバーライドエラー

## A.8 まとめ

| ケース | 挙動 |
|---|---|
| ユーザー定義コンポーネントが別名 | ビルトインと共存、曖昧さなし |
| 同名だが`#[overrides]`なし | 静的エラー(曖昧参照として拒否) |
| 同名で`#[overrides(builtin::X)]`あり | そのスコープ内でユーザー定義が優先、ビルトインは`builtin::X`で明示的にのみ参照可能 |
| シグネチャ不一致 | 静的エラー |

## A.9 `component`宣言レベルの属性:`#[embedded]`/`#[sealed]`/`#[native]`/`#[abstract]`/`#[content(field_name)]`

`#[overrides(builtin::X)]`(A.4)がユーザー定義コンポーネント側に付ける属性なのに対し、`#[embedded]`/`#[sealed]`/`#[native]`は`elwindui-codegen`自身の`.elwind`ソース(`BUILTIN_SHAPE_SOURCE`、`crates/elwindui-codegen/src/builtins.elwind`)が自分自身に付ける属性。`#[content(field_name)]`だけはビルトイン限定ではなく、ユーザー定義コンポーネントでも使える。`#[abstract]`もビルトイン限定ではない一般属性。いずれも`component`宣言の直前に、`inherits`の有無に関わらず0個以上任意の順序で書ける(`enum`/`viewmodel`/`view`には付けられない)。

- **`#[embedded]`** — このコンポーネントが`BUILTIN_SHAPE_SOURCE`自身の組み込み部品であることを明示する。`elwindui-codegen`は`BUILTIN_SHAPE_SOURCE`由来のモジュールを内部的に`is_builtin`フラグ付きで扱っており、`#[embedded]`が付いたコンポーネントがそれ以外の場所(利用者自身の`.elwind`ファイル)から来ていれば静的エラーになる。
- **`#[sealed]`** — このコンポーネントを`component X inherits Y`の`Y`(継承元)として指定できないようにする。具象的な末端形状(`Rectangle`/`Ellipse` — 継承したければ合成可能な`Shape`を使う)や、そもそも継承先を持たないネイティブ末端要素(`Button`/`TextArea`/`TabView`/`TabViewItem`)に付与する。
- **`#[native]`** — `inherits`元を持たず(base-less)、かつ`view`も持たないコンポーネントに、「実Rust実装は各バックエンドクレートが手書きする」ことを明示する。`inherits NativeControl`(A.1の1.)と`is_native == true`として扱われる点は同じだが、`NativeControl`という共有タグを経由しない——2つの使い分けは「実際にビジュアルツリーに`Rc<dyn UIElement>`として埋め込まれ、各バックエンドの`NativeControl`実装をバックエンド構造体の`base`として合成するか」で決まる(`docs/elwindui_gui_framework_design.md`§5.1a)。`Window`(実際のWinUI3の`Window`が`Control`ファミリーを経由せず`Object`を直接継承するのに対応)に加え、ビジュアルツリーに参加しない`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabViewItem`もこちらを使う。`#[native]`は`base`を持つコンポーネントや自前の`view`を持つコンポーネントには付けられず、`#[embedded]`と同様`BUILTIN_SHAPE_SOURCE`自身の宣言以外では使えない。
- **`#[abstract]`** — このコンポーネントを`view`内で直接インスタンス化できないようにする(`Type { .. }`という形で、属性値・クロージャ本体・裸のネスト子要素・単体の`view`ルートのどこに書いても静的エラー)。`component X inherits Y`の`Y`として指定するのは引き続き可能——むしろそれが本来の使い道で、`#[sealed]`のちょうど逆に位置する。唯一の例外は、`X`自身が`inherits`で名指ししている`#[abstract]`な`base`を、`X`自身の`view`の**ルート要素として**構築する場合(シェイプ合成、`docs/elwindui_builtins_spec.md`付録F.10の`Shape`の例。`validate::validate_inherits`が「ルート要素は`base`と一致しなければならない」を既に強制しているので、この一箇所だけ安全に許可される)。`builtins.elwind`の`UIElement`/`NativeControl`/`Layout`/`Shape`(いずれも「フィールドを持たない純粋なカテゴリタグ」、もしくは`Rectangle`/`Ellipse`が合成する土台)に付いており、直接使うことを意図した具象virtual builtin(`VerticalLayout`/`HorizontalLayout`/`Control`/`Grid`/`TextBlock`)には付かない。`codegen::generate_module`も`#[abstract]`なコンポーネントには`create_<snake case>(..)`/`new(..)`を一切生成しない。
- **`#[content(field_name)]`** — WinUI3の`ContentPropertyAttribute`相当。ある要素の`view`本体に「属性名を書かない裸のネスト子要素」(`Type { .. }`を`name: value`形式でなく直接`{}`内に書く)を渡した際、それがどのフィールドに束縛されるかを明示する。例:`MenuBarItem`は`#[content(submenu)]`を宣言しており、`MenuBarItem { text: "File", Menu { .. } }`の`Menu { .. }`は`submenu`フィールドに束縛される(`Window`/`ContentControl`/`TabViewItem`の`content`フィールドも同様に`#[content(content)]`を宣言している)。`field_name`は実在するフィールド名でなければならず(静的検証)、componentにつき最大1個。裸のネスト子要素があるのに`#[content(..)]`(または`children: Vec<..>`のようなリストフィールド)が無いcomponentにそれを渡すのはコード生成時エラーになる。

---

> 標準ビルトイン部品(`builtin::`名前空間のUI要素・`platform::`名前空間のOS機能アクセス)の仕様は
> `docs/elwindui_builtins_spec.md`(付録F・G・L・M・N・Q・T・X・Y)にまとめてある。バックエンド抽象化・
> `elwindui-core`ランタイム・ライフサイクル・Store/ViewModel/MVVM等のフレームワーク設計は
> `docs/elwindui_gui_framework_design.md`を参照。
