# ElwindUIL DSL仕様書

Rust向けGUIフレームワーク(Elwind)のための宣言的レイアウト記述言語(ElwindUIL)の構文・意味論の仕様書。
Rustの構文・慣習に寄せることで学習コストを下げつつ、機械可読性・事前検証性を重視した設計。

本書はDSLの構文・静的検証ルールのみを対象とする。バックエンド抽象化・`elwindui-core`ランタイム・ライフサイクル・Store/ViewModel/MVVM等のGUIフレームワーク本体の設計は `docs/design/gui_framework_design.md`、標準UI要素(`Window`/`Button`等)の個別リファレンスとOS機能(ファイルダイアログ等)は `docs/specs/builtins_spec.md`、コード生成・LSP・プレビュー・ホットリロード等のツールチェーンは `docs/design/tools/*.md` を参照。

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

## 2. 基本構造 ✅

ElwindUILは通常のRustファイル中で属性マクロ`#[elwindui::component]`を使って書く。**これが唯一サポートされる記法であり、Rustのソースファイル以外の独自テキスト形式は存在しない。** 要素はRustの構造体リテラルに似た記法で記述し、ネストがそのまま親子関係になる。

```rust
#[elwindui::component]
struct Greeting {
    body: view! {
        HorizontalLayout {
            TextBlock { text: "Hello" }
            Button { text: "OK" }
        }
    }
}

#[elwindui::component]
impl Greeting {}
```

- 属性は `key: value` 形式
- カンマ・改行はどちらも区切りとして等価
- 単純な識別子・リテラルの参照は `${}` 不要。演算や結合を含む式のみ `format!` 等を使う
- `view!`は実在するマクロとして展開されるわけではない——`#[elwindui::component]`が`struct`全体を丸ごと別のコードへ置き換えるため、`view!`呼び出しのトークンはRustの型位置(`syn::Type::Macro`として構文的に妥当)を借りた記法にすぎず、生のDSLテキストとして読み出されて`elwindui-codegen`の既存パーサへそのままかけられる
- **`#[elwindui::component] impl Name {}` は、メソッドが1つも無くても常に必須。** 型を実際に生成するのは`impl`側であり、`struct`側は宣言を登録するだけ(`#[class]`が`struct`側で引数を保存し`impl`側で生成するのと同じ分担、§3「メソッド継承とオーバーライド」参照)。省略すると`Name`という名前で参照可能な型が生成されない。本書の以降のコード例は、原則としてこの空`impl`を省略せずに示す

```rust
#[elwindui::component]
struct Label {
    #[prop]
    label: String,

    body: view! {
        VerticalLayout {
            TextBlock { text: label }                  // 単純参照
            TextBlock { text: format!("{label}!") }    // 式はformat!マクロで明示
        }
    }
}

#[elwindui::component]
impl Label {}
```

実装は`elwindui_macros::component`(`elwindui::component`として再エクスポート)。マクロの入出力と内部パイプラインは`docs/design/tools/codegen_design.md`を参照。

### 名前空間:Rustのクレート名前空間規則にそのまま従う ✅

上記の`HorizontalLayout`/`TextBlock`/`Button`のような標準UI要素(ビルトイン)は、DSL専用の疑似名前空間を持たない。実体は通常のRustアイテムであり、名前解決はRustのクレート名前空間規則にそのまま従う。正準パスは常に **`elwindui::ui::<Name>`** である:

- バックエンド非依存のビルトイン(`VerticalLayout`/`HorizontalLayout`/`TextBlock`/`Control`/`ContentControl`/`Grid`/`Rectangle`/`Ellipse`/`Image`ほか)は`elwindui-core`クレートの`ui`モジュールで宣言される
- ネイティブビルトイン(`Window`/`Button`/`TextArea`/`TabView`ほか)の実体は、有効化中バックエンドのクレート(`elwindui-backend-appkit`/`-winui3`/`-gtk4`。`elwindui`クレートのCargoフィーチャ`backend-appkit`/`backend-winui3`/`backend-gtk4`で選択)にある

`elwindui`ファサードクレート(`crates/elwindui/src/lib.rs`)がこの両方を1つの`elwindui::ui`モジュールへ束ねる:

```rust
pub mod ui {
    #[cfg(all(target_os = "macos", feature = "backend-appkit"))]
    pub use elwindui_backend_appkit::*;
    #[cfg(all(target_os = "windows", feature = "backend-winui3"))]
    pub use elwindui_backend_winui3::*;
    #[cfg(all(target_os = "linux", feature = "backend-gtk4"))]
    pub use elwindui_backend_gtk4::*;
    pub use elwindui_core::ui::*;
}
```

このため**利用側から見えるパスはバックエンドに関わらず常に同一の`elwindui::ui::X`になる**。`view! { .. }`の中で`HorizontalLayout { .. }`のように裸の名前で書けるのは、コード生成器が生成コードの先頭に`use elwindui::ui::*;`を自動的に挿入するためであり(`elwindui-codegen`の実装詳細)、DSL文法が特別な名前解決規則を持っているわけではない——これも通常のRustの`use`と同じ仕組みである。

ユーザー定義のcomponent・`enum`・viewmodelにはこの自動`use`は効かない。これらは通常のRustアイテムとして自分のファイル・モジュールに存在し、他のファイルから参照する場合は通常のRustの`use`/可視性規則にそのまま従う(§11参照)。同様に、`view!`の外——自分で書く通常のRustコード、例えば`main`関数やcomponentの`impl`ブロック——からビルトインの拡張トレイト(インスタンスメソッド呼び出しに必要な`WindowExt`等)を使う場合は、その都度Rustの通常の`use`で明示的にインポートする:

```rust
use elwindui::ui::WindowExt;   // .set_title(..) 等のインスタンスメソッドを呼ぶために必要

window.set_title("Hello");
```

OS機能(ファイルダイアログ等)も同様に実在するモジュールパスを持つ。正準パスは**`elwindui::platform::<機能>`**で、こちらも自動`use`の対象外なので明示的にインポートする:

```rust
use elwindui::platform;

if let Some(path) = platform::file_dialog::open().await {
    // ...
}
```

---

## 3. component と view 🚧

コンポーネントは **`component`(データ定義)** と **`view`(描画ロジック)** の2つの役割に分離する。Rustの `struct` と `impl` の関係に対応する概念的な区分であり、構文上は1つの`struct`定義の中に共存する(§2参照)。

| | component | view |
|---|---|---|
| 役割 | 状態(フィールド)の定義 | 状態→見た目の写像 |
| 対応するRust概念 | `struct Foo { ... }` | `impl Foo { fn view(&self) -> Rc<dyn UIElement> }` |
| 書く内容 | 型・制約・初期値のみ | `if`/`for`/`match`による要素ツリーの組み立て |
| 変更頻度 | 低い(型は安定) | 高い(レイアウト調整で頻繁に変わる) |

**`body: view! { .. }` フィールドを持つcomponentは、必ず`inherits`(次項)で何らかのbaseを指定する。** base無しで自分自身の視覚ツリーを一から組み立てるcomponentは現状サポートされない——`view`を持つcomponentは常に何らかの合成可能なbase(`VerticalLayout`/`HorizontalLayout`/`Control`等、または他のユーザー定義component)の上に構築する。子要素を並べたいだけの単純なcomponentは、`inherits VerticalLayout`/`inherits HorizontalLayout`(次項の2番目のケース、シェイプ合成)を使うのが最も基本的な書き方になる——この場合`view`の中身がそのままそのレイアウトの子要素になるため、ラッパー要素を書く必要もない。`view!`を持たないcomponent(データ定義のみ、§4参照)にはこの制約はない。

```rust
#[elwindui::component(inherits VerticalLayout)]
struct VolumeControl {
    #[param(default = Orientation::Horizontal)]
    orientation: Orientation,

    #[prop(default = 50.0)]
    volume: f32,

    #[computed(expr = volume.to_string() + "%")]
    label: String,

    body: view! {
        match orientation {
            Orientation::Horizontal => {
                HorizontalLayout { Slider { value <=> volume }, TextBlock { text: label } }
            }
            Orientation::Vertical => {
                VerticalLayout { Slider { value <=> volume }, TextBlock { text: label } }
            }
        }
    }
}

#[elwindui::component]
impl VolumeControl {}
```

- 通常のRust `struct`フィールドには`field: Ty = expr`という初期化式の構文が無いため、デフォルト値・算出式は`#[prop(default = expr)]`/`#[computed(expr = expr)]`/`#[attached(default = expr)]`のように対応する属性自身の名前付き引数として渡す——`viewmodel`側の`#[observable(default = expr)]`/`#[computed(expr = expr)]`(`docs/design/gui_framework_design.md`§7.2)と同じ仕組み(`crates/elwindui-codegen/src/attr_frontend.rs`が両方の`struct`フロントエンドで共有)
- `#[param]`フィールドも`#[prop]`と同様に`#[param(default = expr)]`でデフォルト値を持てる(`#[prop(default = expr)]`と同じ`parser::parse_initializer`を経由する。ただし`#[param]`は静的評価式だけを許可し、リアクティブなフィールド参照は拒否する——§4参照)
- `component`自身の`#[prop(default=...)]`/`#[computed(...)]`フィールドを、その同じcomponentの`view`から裸の識別子で参照できる(`text: label`)——コード生成側は、`volume`が変わるたびに`label`を再計算して該当するビューノードだけを再同期する専用の通知機構を生成する。ただし依存関係の検出は`#[computed(expr = ...)]`の式を実際に`syn`で走査して見つかったフィールド参照に限られるため、`format!("{volume}%")`のようなマクロの不透明なトークン列に隠れた参照は検出できない——`volume.to_string() + "%"`のように、参照したいフィールドを実際の`syn::Expr`として現れる形で書く必要がある
- `match`の条件式(`orientation`)は裸のフィールド参照のみで、`if orientation == Orientation::Horizontal`のような比較式はDSLの`if`/`match`条件文法では扱えない(現状`if`/`match`の条件式パーサは裸のパス参照程度しか受け付けない)——enumによる分岐は比較演算ではなく`match`(または真偽値`#[prop]`に対する`if`)を使う

**呼び出し側:**

`view!`の中で`Card { title: "売上", value: 12000 }`と書くDSL要素構築の糖衣構文は、`view!`の**外**にある通常のRustコード(例えば`main`関数)からは使えない——生成された`Card`は普通のRust structであり、フィールドが`pub`公開されるとは限らないため、`view!`外からは`#[param]`/初期値を持たない`#[prop]`をコンストラクタ引数の順で渡す通常の関連関数`Card::new(..)`を呼ぶ:

```rust
#[elwindui::component(inherits VerticalLayout)]
struct Card {
    #[prop]
    title: String,
    #[prop]
    value: i32,

    body: view! {
        TextBlock { text: title }
        TextBlock { text: format!("{}", value) }
    }
}

#[elwindui::component]
impl Card {}

let sales_card = Card::new("売上".to_string(), 12000);
```

- インスタンス命名は専用記号を使わず、Rustの `let` 束縛をそのまま使う
- `view!`の**中**からは、DSL要素構築の糖衣構文で書ける

```rust
#[elwindui::component(inherits VerticalLayout)]
struct Dashboard {
    #[prop]
    title: String,
    #[prop]
    value: i32,

    body: view! {
        Card { title: title, value: value }
    }
}

#[elwindui::component]
impl Dashboard {}
```

**実装状況の注**: 「属性名と変数名が一致する場合のショートハンド」(`Card { title, value }`のように`title: title`を省略する記法)は設計されているが、現状の`elwindui-codegen`では動作しない(`` `title` does not refer to an earlier `let` binding in this view ``という内部エラーで失敗する)。動作するまでは`title: title`のように毎回フルスペルで書くこと。

### `inherits`:WinUI3方式のクラス継承

`#[elwindui::component(inherits Base)] struct Name { ... }` は `Base` を4通りに解決する(単なる構造的契約ではなく、WinUI3/C#の`Control → ContentControl → Button`と同じ実継承):

1. **`Base`が`NativeControl`マーカー** — 純粋なカテゴリタグ(フィールド継承なし)。ネイティブ実装を持つ末端要素(`Button`等)であることを示すのみ。
2. **`Base`が`view`を持たないプリミティブ形状ファミリー**(例:`Control`/`Rectangle`)、または**`Base`自身が既にシェイプ合成されているDSLコンポーネント**、または**`Base`が`view`を持たないネイティブ実装のホスト**(例:`Window`) — `Base`の`#[param]`/propフィールドを**再宣言なしに自動継承**し、さらに`Name`自身の`view`の中身は**常に暗黙に`Base`自身の属性・子要素**になる(ラッパー要素は書かない——`Base { ... }`という入れ子は書かず、`Base`の属性・子要素を`view`の`{}`直下にそのまま書く)。シェイプ合成/ホスト合成(`docs/specs/builtins_spec.md`付録F.10参照)。
3. **`Base`が自前の`view`を持つ、それ自体は合成されていない論理コンポーネント**(builtinでもユーザー定義でも) — フィールドに加えて`view`(テンプレート)も継承する。`Name`が独自の`view`を書かなければ`Base`のテンプレートをそのまま(WinUI3の既定`ControlTemplate`のように)引き継ぎ、書けば**完全なテンプレート上書き**になる(ルート要素の型に制約はない)。
4. **`Base`がネイティブ実装のみの末端要素**(例:`Button`) — 継承不可。生成されるRustコードを持たないため、委譲先が存在しない。

`Base`の書き方は、それが組み込み(builtin)かユーザー定義かで異なる:組み込みは裸の名前(`inherits Control`/`inherits ContentControl`のように)、ユーザー定義コンポーネントはクレートルート起点の完全修飾パス(`inherits crate::ui::LabeledPanel`)で書く。これは`#[elwindui_macros::class]`の`inherits = ..`引数が同一クレート内でも常に完全修飾パスを要求するのと同じ理由による(`docs/specs/macro_class_spec.md`§7)——生成される`__elwindui_inherit_*!`マクロ連鎖が別モジュールから展開される可能性があるため、裸名は解決できない。ユーザー定義の`Base`を裸名で書くと静的エラーになる。あわせて、`Base`を公開するモジュールは名前を列挙した再エクスポートではなく、必ずグロブ再エクスポート(`pub use some_module::*;`)にすること——`#[class]`は`Base`と同じ位置に伴走する`__elwindui_macros_of_{Base}`エイリアスを生成するため、名前を列挙した再エクスポートではそれが取り残される。

```rust
#[elwindui::component(inherits Control)]
struct ContentControl {
    content: std::rc::Rc<dyn UIElement>,
    // padding は Control から自動的に継承される — 再宣言不要、self.padding() がそのまま使える

    body: view! {
        // `Control { .. }` というラッパーは書かない — `view!`の中身がそのまま暗黙に Control の
        // 属性・子要素になる(2番目のケース)
        padding: padding
        content
    }
}

#[elwindui::component]
impl ContentControl {}
```

**実装状況の注**: `content`のような、`view!`の`{}`直下で自分自身の既存フィールドを**裸のまま**子要素として挿入する記法(`key:`を付けない、`#[id(...)]`束縛でもない単なる識別子1つの行)は、現状の`elwindui-codegen`では動作しない(`` `content` does not refer to an earlier `let` binding in this view ``という内部エラーになる)。`padding: padding`のような`key: value`形式の裸参照(属性値としての参照)は正常に動作する——影響があるのはbody直下に子要素として裸で置く場合のみ。
ここで`Control`は`elwindui::ui::Control`(ビルトイン、裸名で参照)で、`ContentControl`は上記の例で定義しているユーザー自身のcomponent名——ビルトインの同名`ContentControl`(`docs/specs/builtins_spec.md`付録F.10)と衝突しない。ローカルに定義された`ContentControl`は、`elwindui::ui::*`の自動`use`(§2)より常に優先して解決される(Rustの通常の名前解決が、同一スコープのグロブ`use`よりローカル定義を優先するのと同じ)。

`view`の中身が暗黙に`Base`自身になるかどうかは、`Base`が実際に合成可能(2番目のケースに当てはまるか)によって決まり、`Name`自身がラッパーを書くかどうかでは選べない――合成可能な`Base`を持つ`component`の`view`は常にこの形で書く。3番目のケース(合成されていない論理コンポーネントの継承)だけが、今まで通り「独自のルート要素を持つ完全なテンプレート上書き」になる。

継承したフィールドは、派生component自身の`view`が**同名のまま裸で参照**している場合のみ、派生側の実効フィールド(＝コンストラクタ引数)になる。リテラル値で上書きしている場合(例:`Rectangle { fill: "#3a3a3c" }`)や、そもそも参照していない場合は、その基底フィールドは派生側の公開APIには現れない。

**メソッド継承とオーバーライド**(C#の`virtual`/`override`/`base.Method()`相当)。🚧 **部分実装**。

メソッド本体は`struct`定義には置けないので、`#[elwindui_macros::class]`と同じ**`struct`/`impl`ペア**(`docs/specs/macro_class_spec.md`§2.1)の形を取る。属性名も`#[class]`と揃え、`#[overridable]`(オーバーライド可能な宣言)と`#[overrides]`(上書き)を使う:

```rust
#[elwindui::component]
struct Control {
    // フィールド宣言
}

#[elwindui::component]
impl Control {
    #[overridable]
    fn label(&self) -> String {
        "control".to_string()
    }
}

#[elwindui::component(inherits Control)]
struct ContentControl {
    // フィールド宣言
}

#[elwindui::component]
impl ContentControl {
    #[overrides]
    fn label(&self) -> String {
        format!("{}!", base::label())
    }
}
```

- `#[overridable] fn name(&self, ...) -> T { ... }` — 派生componentがオーバーライド可能なメソッドを宣言する
- `#[overrides] fn name(...) { ... }` — 基底の同名`#[overridable]`メソッドと同じシグネチャで上書きする(シグネチャ不一致は静的エラー)
- `base::name(...)` — オーバーライドした本体から基底実装を呼び出す(C#の`base.Method()`相当)。同じ書き方で`on_mount`/`on_unmount`(`docs/design/gui_framework_design.md`§6.1)内から基底のライフサイクルフックを呼ぶこともできる
- 継承・オーバーライドは1階層(直接の`inherits`先)のみ保証される。2階層以上に渡る`base::`連鎖は対象外
- `impl`側の`#[elwindui::component]`は**引数なし**で書く。`#[class]`と同じく、対応する`struct`が同一ソース上で先に宣言されている必要がある
- **`impl`ブロックはメソッドが無くても必須**。型を生成するのは`impl`側であり、`struct`側は宣言を登録するだけである(`#[class]`が`struct`側で引数を保存し`impl`側で生成するのと同じ分担)。メソッドを持たないコンポーネントも`#[elwindui::component] impl Name {}`を書く
- `fn`には`#[overridable]`か`#[overrides]`のいずれかが必須。`&self`レシーバ・プレーンな識別子引数のみで、ジェネリクス・`where`句・`async`/`unsafe`・トレイト`impl`は受け付けない(通常の`impl`ブロックに書く)
- `#[elwindui_macros::class]`(ビルトインのRustクラス階層マクロ、`docs/specs/macro_class_spec.md`§8.3)にも同じ`#[overridable]`/`#[overrides]`があるが、そちらは`elwindui-core`/バックエンドが手書きするRustクラス階層に対する仕組みで、こちらはコンポーネント継承チェーン上のメソッドオーバーライド(AST上の`MethodDef`)である。属性名と意味論を意図的に揃えてあるが、実装もスコープも独立している

`#[computed]`フィールドも同様に、基底の同名フィールドを`#[overrides]`なしで再宣言するとエラーになり、`#[overrides]`を付けると上書きとして扱われる(型は基底と一致していなければならない)。

### 添付プロパティ(`#[attached]`):WPF/WinUI3方式

あるcomponentが宣言し、**任意の別要素が自分自身に設定できる**プロパティ(WPFの`Grid.Row`/`Grid.Column`相当)。
宣言したcomponent自身のインスタンスデータにはならない——スキーマ宣言のみで、宣言したcomponent自身の
コンストラクタ/生成structには一切現れない。

```rust
#[elwindui::component]
struct Grid {
    #[param]
    rows: Vec<GridLength>,
    #[param]
    columns: Vec<GridLength>,
    #[param]
    children: Vec<AnyView>,

    #[attached(default = 0)]
    row: i32,
    #[attached(default = 0)]
    column: i32,
}

#[elwindui::component]
impl Grid {}
```

```rust
#[elwindui::component(inherits ContentControl)]
struct FormPanel {
    body: view! {
        Grid {
            rows: [GridLength::Auto, GridLength::Star(1.0)]
            columns: [GridLength::Fixed(120.0), GridLength::Star(1.0)]
            TextBlock { text: "Header", Grid::row: 0, Grid::column: 0 }
            Button { text: "Click", Grid::row: 1, Grid::column: 1 }
        }
    }
}

#[elwindui::component]
impl FormPanel {}
```

- `#[attached]`フィールドは初期値(デフォルト)必須——設定しなかった要素に適用される既定値を表す
- 設定側の構文は`Owner::field: value`(Rustのパス区切り`::`)——`{}`内で通常の属性と自由に混在できる
- `Owner`は静的には「`field`という名前の`#[attached]`フィールドを持つ既知のcomponentか」だけを検証する。
  設定先の要素が実際に`Owner`(例:`Grid`)の子孫であるかどうかは**検証しない**——WPF同様、対応する
  コンテナの外で設定しても静的エラーにはならず、単に無視される
- 実装は`(owner, field) -> Box<dyn Any>`の型消去された汎用バッグ(`UIElement::attached`)——
  `Grid`の`row`/`column`もこれ経由で格納される。オーナー自身が自分の宣言した型を知っている
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
  `(erased).base().set_attached::<T>(..)`を呼ぶことで反映される(`docs/design/gui_framework_design.md`§5.1a)。`view`ルート自身が
  ネイティブに解決するユーザー定義component(`inherits NativeControl`を宣言せず`Button`等を
  ラップするようなケース)への設定は、`.base()`へ到達する手段自体がまだ無く、引き続き未対応
  ——将来の拡張課題

---

## 4. componentフィールドの種類(param/prop/state/computed) ✅

**フィールドに`#[param]`/`#[state]`/`#[computed]`のいずれのアトリビュートも付けなければ、既定で`#[prop]`(実行時に読み書き可能)になる。** `#[prop]`は明示的に書いてもよいが、省略時のデフォルトでもある。

| | `#[param]` | `#[prop]`(既定) | `#[state]` | `#[computed]` |
|---|---|---|---|---|
| 変更可能性 | 実体化時のみ、以後イミュータブル | 実行時いつでも変更可 | 実行時いつでも変更可(component自身のロジックから) | 不可——依存フィールドの変化に応じて自動再計算される読み取り専用値 |
| 使える式 | 静的評価式のみ(リテラル・他paramの参照・純粋関数・`env::*`) | 静的評価式に加え、他prop/state/computed/bindable ownerを参照するリアクティブ式 | `default`は静的評価式のみ(初期値)。以後の値は`view`/イベントハンドラからの代入で決まる | `#[computed(expr = ...)]`の式(依存する他フィールドへの参照を含む) |
| 主な用途 | 構造分岐(`if`/`for`の条件)、レイアウト決定 | 表示内容・状態の動的更新 | componentが内部だけで使う非公開の状態(外部の構築引数・公開APIには現れない、§9参照) | 他フィールドから導出される算出値(例:合計金額、書式化した文字列) |
| 実行時アクセス | `instance.field()`(getterのみ。代入相当の操作はコンパイルエラー) | `instance.field()`/`instance.set_field(value)`(後述) | component内部の`self.field()`/`self.set_field(value)` | `instance.field()`(getterのみ) |

`#[computed]` を付けたフィールドは依存する他フィールドの変化に応じて自動再評価される読み取り専用の算出値。外部からの代入は静的エラーとなる。

**`#[prop]`フィールドへのアクセスは、コード生成器が自動生成する`pub fn <field>(&self) -> T`(getter)/`pub fn set_<field>(&self, value: T)`(setter)を通じて行う**——フィールド自体は`struct`上に公開されず、この2メソッドが外部・内部を問わず唯一のアクセス経路になる。ただし**setterが生成されるのはそのフィールドがdefault値を持つ場合のみ**(`#[prop(default = expr)]`、または`Option<T>`型で暗黙にdefaultが`None`になる場合)——default値を持たない`#[prop]`(型のみを書いた必須フィールド)は`#[param]`と同じく`new(..)`のコンストラクタ引数になり、getterしか生成されない。したがって「`#[prop]`なら常に実行時に書き換えられる」わけではなく、**default値の有無**が実際にsetterを持つかどうかを決める。

`#[bindable]`はcomponentがviewmodelを保持するための専用アトリビュートで、**指定できる型はviewmodel(`#[elwindui::viewmodel]`で定義された型)に限られる**——viewmodel以外の型を指定すると、生成されるコードがviewmodel専用のPropertyChanged購読の仕組みを満たせずコンパイルエラーになる。実体化時に一度だけ固定される(以後差し替え不可)という点は`#[param]`と同様だが、`#[bindable]`自身が値を書き換えるわけではなく、保持しているviewmodelの中のフィールドが変化した際に依存する`view`部分を自動再同期させるための購読を張る(`docs/design/gui_framework_design.md`§7.2参照)。

```rust
#[elwindui::component]
struct Cart {
    items: Vec<Item>,

    #[computed(expr = items.iter().map(|i| i.price * i.qty).sum())]
    total: f64,
}

#[elwindui::component]
impl Cart {}
```

**静的評価式に許可される要素(`#[param]`用):**

- リテラル(数値・文字列・真偽値・配列)
- 四則演算・比較・三項演算子相当の `if` 式
- 組み込み純粋関数(`min`, `max`, `round` など)
- 同一コンポーネント内の他の `#[param]` フィールドへの参照
- `env::*`(動的定数、§8)

**禁止される要素:**

- リアクティブな`#[prop]`/`#[state]`/`#[bindable]`プロパティの参照
- prop(`#[param]`が付いていないフィールド)の参照
- 非純粋関数(`now()`, `random()` など)の直接呼び出し

### コールバック型フィールド: `fn(...)` 糖衣構文

フィールドがコールバック(関数)型を持つ場合、`Rc<dyn Fn(...)>` や `Box<dyn Fn(...)>` のような
型消去表現をDSLソース上に直接書くことは禁止される(13章ルール25)。かわりに以下の糖衣構文を使う:

```
fn(引数型, ...)                // 戻り値なし、必須
fn(引数型, ...) -> 戻り値型      // 戻り値あり、必須
fn(引数型, ...)?                 // 省略可能。既定値は `= None` で明示する
```

この糖衣構文はコード生成時、フィールドを持つ`component`のインスタンス化ごとに単相化
(monomorphize)された具体的なクロージャ引数として展開され、`Box<dyn Fn>`/`Rc<dyn Fn>`のような
実行時型消去は発生しない(`docs/design/gui_framework_design.md`§7.2の「型消去を避け専用コードを生成する」方針と同じ)。

`fn(...)`型のフィールドの意味は`#[param]`の有無でそのまま決まり、コールバック専用の追加
アトリビュートは存在しない:

- **`#[param]`付き** = 実体化時に固定される値計算コールバック。静的評価式(その場で束縛された
  クロージャ)のみ許可される。例: `key: fn(&Item) -> usize`, `render_item: fn(&Item) -> View`。
- **`#[param]`無し(既定の`prop`)** = 実行時に差し替え可能な通知コールバック、いわゆる
  イベントハンドラ。例: `on_select: fn(usize)`, `on_close: fn(usize)`。

```rust
#[elwindui::component]
struct VirtualList {
    #[param]
    key: fn(&Item) -> usize,      // 値計算コールバック(paramなので実体化時固定)

    on_select: fn(usize),         // 通知コールバック(propなので実行時に発火・差し替え可)
}

#[elwindui::component]
impl VirtualList {}
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
#[elwindui::component(inherits ContentControl)]
struct DocumentTabs {
    #[bindable]
    vm: std::rc::Rc<DocumentViewModel>,

    body: view! {
        TabView {
            on_select: |index| vm.select_tab(index)     // 1引数、式1つ
            on_close: |index| {                          // 1引数、複数文ブロック
                vm.log_close(index);
                vm.close_tab(index);
            }
            on_new_tab: vm.new_tab                        // 0引数、ベアパスの糖衣
        }
    }
}

#[elwindui::component]
impl DocumentTabs {}
```

- `render_content: |item| DocumentView { doc: item }`のような「ネストした要素を返す」形は`#[param]`側の値計算コールバック専有の形で、`on_*`のような通知コールバック(イベントハンドラ)には使えない(要素を返しても配線先がない)
- ブロック本体`{ 文; ... }`は式1つの本体と違い、他のDSL式のような「`vm.field`は自動的にゲッター/アクション呼び出しになる」糖衣を持たない**素のRust**として解釈される — アクションを呼ぶ場合は`vm.close_tab(index)`のように明示的に`()`を書く(`vm.close_tab`だけだと、存在しないフィールドへのアクセスとして扱われコンパイルエラーになる)。`vm`のような参照先の解決(`self.vm`相当への書き換え)自体は式本体と同様に行われる
- クロージャ本体内の`vm.field`/`vm.action(args)`のような参照は、他のDSL式と同じ規則で解決される(コード生成側の詳細は`docs/design/gui_framework_design.md`§7.2参照)

### `ControlTemplate<Self>`:テンプレート型フィールド(WinUI3方式`ControlTemplate`)

> **実装状況**: 設計のみ。本節が前提とする「値計算コールバックがネストした要素を構築する」機構自体(直前の`|param| Type { .. }`)がまだコード生成に実装されていないため、本節はそれに依存する形でさらに未実装。

`ControlTemplate<Self>`は、コンポーネント自身の視覚ツリーを実行時に丸ごと差し替え可能にする専用のフィールド型糖衣。WinUI3の`Control.Template`(`ContentPresenter`等を介した視覚ツリーの丸ごと差し替え、`Style`経由でインスタンス単位に再テンプレート化できる)に相当する。

```rust
#[elwindui::component(inherits UIElement)]
struct Control {
    children: UIElementCollection,
    padding: Option<f32>,

    #[prop(default = None)]
    template: Option<ControlTemplate<Self>>,
}
```

- ジェネリック引数は常に文字通り`Self`のみを許す(コンポーネント自身の型)。それ以外を書くとエラー(13章ルール26)。
- 意味的には`Rc<dyn Fn(&Self) -> Rc<dyn UIElement>>`の糖衣だが、単なる`fn(&Self) -> Rc<dyn UIElement>`コールバック糖衣とは扱いが異なる専用の型として区別する:
  - **`prop`必須**(`#[param]`不可、13章ルール27)——直前の「値計算コールバックは`#[param]`側専有」という原則(§4冒頭)に対する**意図的な例外**。テンプレートは実行時に差し替えられて初めて意味があるため、実体化時固定の`#[param]`では目的を果たせない
  - 値が変わったとき、対応する`body`(下記)配下の視覚ツリーを丸ごと再構築するという、通常のプロパティ値の再代入とは異なる**構造的な**再同期が必要(`docs/design/gui_framework_design.md`新設§5.7参照)

**値の書き方**は新しい構文を作らず、直前の「ネストした要素を構築する」値クロージャ構文(`|param| Type { .. }`)をそのまま使う:

```
template: |control| Grid {
    Rectangle { .. }
    control.content
}
```

パラメータ名は普通の識別子(型キーワードの`Self`はここでは使えない——値束縛名としては`control`のような通常の識別子を使う)。クロージャ内から`control.content`/`control.padding()`のように自分自身の他フィールドへ直接アクセスできる。これはWinUI3の`TemplateBinding`(リフレクションベース)の静的型付け版に相当し、既存の「`#[param]`フィールドへの名前付きアクセサ自動生成」(`docs/specs/builtins_spec.md`付録F補足)をそのまま使う。

**`body: <field>(Self)`**——`ControlTemplate<Self>`型のフィールドを、自分自身を渡して呼び出した結果を視覚ツリーのルートにする、という新しい`body`/`view`ルートの書き方。`field`名は`template`に限定せず、`ControlTemplate<Self>`型のフィールドなら任意の名前で使える一般規則(13章ルール28: `field`が同一component内の`ControlTemplate<Self>`型フィールドでない場合はエラー)。`Control`(`docs/specs/builtins_spec.md`付録F.9)の例:

```rust
#[elwindui::component(inherits UIElement)]
struct Control {
    #[prop]
    template: Option<ControlTemplate<Self>>,

    body: view! {
        match template {
            Some(t) => t(Self),
            None => /* 既定挙動: children をそのまま Visual 子要素にする */,
        }
    }
}
```

`ControlTemplate<Self>`が返すのは常に単一ルート要素(WinUI3実物の`ControlTemplate`、および本DSLの「単一値フィールドの`if`/`match`は1要素に還元」ルール(§5)と同じ)。`Control.template`が`None`(既定)のときは現行どおり`children`を直接Visual子要素にする——挙動変更なし。

### `#[elwindui::template]`:再利用可能な名前付きテンプレート

> **実装状況**: 設計のみ・未実装。`#[elwindui::component]`/`#[elwindui::viewmodel]`と同系統だが、専用の`fn`向け属性マクロとしてはまだ存在しない。

`template: |control| Grid { .. }`のようなインライン値クロージャ(前節参照)はその場限り(1箇所)の書き方しかできない。複数のコンポーネントで同じテンプレートを使い回したい(WinUI3で`ControlTemplate`を`Style`リソースとして共有するのと同じ用途)場合のために、`#[elwindui::component]`(`struct`に付与)・`#[elwindui::viewmodel]`(`mod`に付与)と同系統の、**単一の`fn`に付与する新しい属性マクロ**を用意する:

```rust
#[elwindui::template]
fn button_template(inst: &Button) -> Rc<dyn UIElement> {
    HorizontalLayout {
        Rectangle { .. }
        inst.content
    }
}
```

- パラメータは必ず1個。型注釈は普通のRustとして必須(DSLの値クロージャと違い、これは生Rustの`fn`宣言なので型省略はできない)。戻り値の型は`Rc<dyn UIElement>`固定
- `#[elwindui::component]`(`elwindui_macros::component`、`crates/elwindui-macros/src/lib.rs`)と同じトリック——`fn`の本体ブロックをRustとして解釈させず、生のDSLテキストとして`elwindui-codegen`の既存パーサに渡し、パラメータ名(`inst`)を「テンプレート対象インスタンス」として束縛した状態でコード生成する想定(`elwindui-codegen`側に姉妹フロントエンドを追加する実装になる見込み)
- 値としての参照は裸パス(`template: button_template`)。これは`ControlTemplate<Self>`型フィールドへの裸パス代入の規則(前節参照——関数アイテムそのものを値として使う、既存の0引数呼び出し糖衣とは別の意味)に従う。パラメータ型が厳密にフィールドの`Self`と一致しない関数を指している場合はエラー(13章ルール29)
- `docs/design/tools/codegen_design.md`§4.1も参照(`component`/`viewmodel`と並ぶ3つ目のRust代替記法として言及)

```rust
#[elwindui::component]
struct Toolbar {
    body: view! {
        Button { template: button_template }
    }
}
```

**広く共有される既定値**(WinUI3の`Style`相当、複数コンポーネントに跨って既定テンプレートを一括変更する用途)は、新しい仕組みを作らず既存の`store`を`#[bindable]` ownerとして公開し、通常のリアクティブ属性式(`docs/design/gui_framework_design.md`§7.1)を使う。詳細は同節を参照。

---

## 5. 制御構文 ✅

Rust標準の制御構文をそのまま採用し、専用ディレクティブは設けない。

```rust
#[elwindui::component(inherits VerticalLayout)]
struct ItemList {
    #[prop]
    items: Vec<Item>,
    #[prop]
    is_admin: bool,
    #[prop]
    status: Status,

    body: view! {
        // 繰り返し
        for item in items {
            Card { title: item.name, value: item.value }
        }

        // 条件分岐(真偽値の裸のプロパティ参照)
        if is_admin {
            Button { text: "管理画面" }
        } else {
            TextBlock { text: "権限がありません" }
        }

        // 分岐(網羅性検査つき)
        match status {
            Status::Loading => TextBlock { text: "読み込み中…" },
            Status::Error   => TextBlock { text: "エラー", foreground: "#c0392b" },
            Status::Ok      => TextBlock { text: "OK" },
        }
    }
}

#[elwindui::component]
impl ItemList {}
```

`match` は列挙体の全メンバーを網羅していれば `_ =>` を省略できる。網羅されていない場合はコンパイルエラーとなる(Rustの`match`と同じ挙動)。

`if`/`match`の条件式は裸のプロパティ参照のみを受け付ける——`if is_admin`のような真偽値フィールドの参照や`match status`のような列挙体フィールドの参照はできるが、`if orientation == Orientation::Horizontal`のような比較式は書けない(§3参照)。enumによる分岐は常に`match`を使う。

`if`/`match`の各分岐(`else if`チェーンを含む)には、さらに`if`/`match`/`for`を入れ子で書ける——`else if`は`else`ブロックの中にネストした`if`が1つある形として扱われる。ただし`for`自身のbody(繰り返される側のテンプレート)はリテラル要素のみで、その中に`if`/`match`/`for`をさらに入れ子にすることはできない(各`for`項目は使い捨てのローカル構造体であり、入れ子の動的領域を持つ永続状態を持たないため)。

子要素の格納先フィールド(付録A `#[content(field_name)]`参照)がリスト型(`Vec<..>`/`ListExt<..>`)の場合、`if`/`match`/`for`のいずれも使える(前段落の入れ子ルールも同様)。フィールドが単一値型(例:`ContentControl`/`Window`の`content: Rc<dyn UIElement>`)の場合は`if`/`match`のみ使え、`for`は使えない(可変長のリストは単一の格納先に収まらないため)。単一値フィールド配下の`if`/`match`は、入れ子も含めたあらゆる分岐が最終的にちょうど1個の要素に還元できなければならない(1分岐に複数の裸の子要素を書くこともできない)。

### `if`/`match`/`for`が実際にUI要素を生成する仕組み

`if`/`match`/`for`が書かれた位置は「動的領域」として扱われ、依存するプロパティが変化するたびに、その領域だけが差し替えられる(§9「PropertyChangedと部分更新」参照)——親componentの`view`全体が再構築されることはない。

- **`if`/`match`**: 実際に選択された1つの分岐だけがUI要素として構築される。選択されなかった分岐は構築されない。条件/`match`対象の値が変わって選択される分岐が切り替わると、それまで構築されていた要素は破棄され、新しく選択された分岐が構築される——分岐間で要素が使い回されることはない。
- **`for`**: `items`の各要素ごとに、繰り返されるテンプレート(bodyの子要素)からUI要素が1組ずつ生成される。`items`が変わった際にどの要素を作り直しどの要素を使い回すかは、対象コレクションの型によって決まる:
  - **`items`が`Vec<Rc<T>>`型の場合**(または、`for`のbodyがitemを子componentの`#[bindable]`フィールドへ束縛している場合)、各itemの識別に`Rc`の指すヒープ確保の**アドレス値**(`Rc::as_ptr(item) as usize`)を使う。前回の描画で使われたitemと今回のitemが同じアドレス(=同一の`Rc`実体、`Rc::clone`で複製したものも含む)であれば、対応するUI要素・購読を**再生成せずそのまま使い回す**。前回存在して今回のitems列に存在しないアドレスの要素は破棄される。この仕組みは`elwindui_core::ui::DynamicChildSlot::replace_rc_items`が担う
  - **それ以外の(`Rc`でラップされていない)コレクションの場合**、識別可能な安定したidが無いため、`for`が再評価されるたびにその範囲のUI要素を丸ごと作り直す(既存の要素は再利用されない)
  - 13章ルール23も参照(`VirtualList`の`key`未指定時の挙動を含む、より詳しい規則)

---

## 6. 値制約(アトリビュートによる数式的表現) 🚧

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
#[elwindui::component]
struct LoginForm {
    #[length(3..=16)]
    #[pattern(r"^[a-zA-Z0-9_]+$")]
    username: String,

    #[format(email)]
    email: String,

    password: String,

    #[check(password == confirm_password, message = "パスワードが一致しません")]
    confirm_password: String,
}

#[elwindui::component]
impl LoginForm {}
```

**検証タイミング:**

- リテラル値による制約違反 → ビルド時静的エラー
- リアクティブ属性式等の動的値による制約違反 → 実行時エラー

---

## 7. 列挙体(enum) 🚧

値候補があるフィールドは共用体を書き捨てず、名前付き `enum` として定義する。Rustのenum構文をそのまま採用する。

```rust
#[elwindui::dsl_enum]
enum Orientation {
    Horizontal,
    Vertical,
}

#[elwindui::dsl_enum]
enum ThemeMode {
    #[label(t!("enum.theme.light"))]
    Light,
    #[label(t!("enum.theme.dark"))]
    Dark,
    Auto,
}

#[elwindui::dsl_enum]
enum LogLevel {
    Debug = 0,
    Info = 10,
    Warning = 20,
    Error = 30,
}
```

- 値の参照は `EnumName::Member` というパス形式で書く(裸の文字列リテラルの直書きは型不一致として静的エラー)。**パスの綴り方は通常のRustの`use`と同じ**——`use`で導入した短い名前をそのまま`Member`側に使ってよく、クレートを跨いだフルパスを書く必要はない。`#[elwindui::dsl_enum]`は本体をまったく変更せず透過するだけの真のRust `enum`であり、値参照は`syn::Expr`として生成コードへ逐語的に埋め込まれるため、名前解決は最終的に通常のRustコンパイラへ委譲される(§11参照)
- `EnumName::values()` で全メンバーを列挙可能(`for`との組み合わせで選択UIを自動生成できる)
- `#[label(...)]` アトリビュートで多言語表示名を付与でき、`member.label()` で現在ロケールの文字列を取得する
- `match` と組み合わせることで、全メンバーを処理しているかどうかの網羅性検査が働く。この検査は二層構造になっている:同一クレート内で`#[elwindui::dsl_enum]`登録済みのenumについては`elwindui-codegen`自身がマクロ展開時に早期エラーを出す(同一クレート内限定——プロセスごとに別のrustc起動になる実際の`cargo build`では、この早期検査を別クレートのenumへ拡張する手段が原理的に存在しない、`docs/design/tools/codegen_design.md`§4.2参照)。それとは独立に、生成される`match`は常に合成的な`_ =>`を持たない素のRust `match`として出力されるため、**別クレートから`use`したenumも含め、非網羅的な`match`は最終的に必ずrustc自身が`E0004`として検出する**——早期検査が効かない場合でも、コンパイルが通ってしまうことはない

**実装状況の注**: `#[elwindui::dsl_enum]`(`component_frontend::enum_def_from_item_enum`)は現状、`syn::Fields::Unit`(ペイロード無しの単純variant)であることの検証のみを行い、本体はそのまま無変更で透過する——`ThemeMode`例の`#[label(...)]`のようなvariant単位の属性は取り除かれずそのまま残るため、`#[label(...)]`自体が実Rustの認識済み属性として何らかの形で処理されない限り、この属性を使う`enum`は実際にはコンパイルが通らない。`#[label]`/`EnumName::values()`によるi18nラベル付与の実装自体が「実装範囲は個別確認が必要」という不確実な状態(`docs/status/implementation_status.md`参照)であり、`ThemeMode`の例は設計意図の説明であって動作確認済みのコード例ではない。また`#[elwindui::dsl_enum]`は本体を完全に無変更で透過するだけなので、`Orientation`/`ThemeMode`同士のような値比較(`==`)が必要な場合は`#[derive(PartialEq)]`等をDSL側の属性とは別に自分で付与する必要がある(match のパターンマッチ自体は`PartialEq`を要求しない)。

```rust
#[elwindui::component(inherits VerticalLayout)]
struct ThemeModeList {
    body: view! {
        for m in ThemeMode::values() {
            TextBlock { text: m.label() }
        }
    }
}

#[elwindui::component]
impl ThemeModeList {}
```

匿名の共用体型(`"a" | "b"` のようなインライン列挙)は採用しない。Rustに無名enumがないことと整合させ、値集合を扱う手段は常に名前付き `enum` に一本化する。

---

## 8. 動的定数(env) 📋

「実体化時に一度だけ確定し、以後は変化しない」値を扱うための仕組み。`#[param]` の静的評価式の例外として参照を許可する。

```rust
#[elwindui::component]
struct TitleBar {
    #[param(default = if env::os() == "macos" { "traffic-light" } else { "caption" })]
    style: String,
}
```

**組み込み `env` 関数(例):**

- `env::os()` — `"windows" | "macos" | "linux" | "ios" | "android"`
- `env::platform()` — `"desktop" | "mobile" | "web"`
- `env::locale()` — 実行環境の既定ロケール
- `env::direction()` — `"ltr" | "rtl"`

**実装状況の注**: `env::*`はコード生成器側に実装が全く無く、純粋に設計のみの機能(`docs/status/implementation_status.md`参照)。

---

## 9. データバインディング ✅

```rust
#[elwindui::component(inherits VerticalLayout)]
struct VolumeSlider {
    #[state(default = 50.0)]
    volume: f32,

    body: view! {
        Slider { value <=> volume }
        TextBlock { text: format!("{}%", volume) }
        TextBlock { text: once!(format!("initial: {}%", volume)) }
    }
}

#[elwindui::component]
impl VolumeSlider {}
```

- `property: expression`は依存解析により自動分類される。`#[state]`、可変`#[prop]`、
  `#[computed]`、直接の`#[bindable] owner.property`を含めばOneWay、定数・リテラル・
  `#[param]`だけなら初期化時の一度だけ評価する
- `property: once!(expression)`は依存収集を抑止し、初期化時のスナップショットとして一度だけ評価する
- `property <=> writable_target`は明示的なTwoWay。RHSは同一componentの可変`#[prop]`/
  `#[state]`、直接の`#[bindable] owner.property`、または安定した`for` itemの直接の
  `item.property`に限る。後者は明示的な`Vec<Rc<T>>`または`#[observable] Vec<T>`として
  生成されるviewmodel collectionだけで使え、propertyは`#[prop]`または`#[observable]`でなければならない
- `#[state(default = expr)]`はcomponent専用の非公開リアクティブ状態。defaultは必須で、
  コンストラクタ引数・公開getter/setter・props API・継承フィールドには現れない

### PropertyChanged と部分更新

`#[observable]` のsetterは代入後に型付き `PropertyChanged` を発火する。`view` は式から
静的に取得した依存プロパティだけを購読し、その属性または動的領域だけを更新する。従って
`TextArea { text <=> doc.content }` の入力はその `TextArea` と `doc.content` に依存する表示だけを
更新し、親の `TabView` の children コレクションを再同期しない。二方向バインディングのwidget→model側は
setterを呼ぶだけで、別途コンポーネント全体の再同期を呼んではならない。

購読は `Subscription` で表され、表示領域が破棄されるとDropにより解除される。`for` itemの
TwoWayコールバックも同じ`DynamicChild`の寿命に属し、itemが置換・削除されると購読とともに解放される。
`for`/`if`/`match`
の構造変更は親view全体ではなく対応する動的領域だけを差し替える。依存プロパティを静的に
特定できない任意Rust式はビルド時エラーとし、必要な計算は `#[computed]` または解析可能な
prop参照へ分解する。

---

## 10. 多言語対応(i18n) 🚧

翻訳文言は独自フォーマットを持たず、業界標準の **Fluent(.ftl)** をそのまま採用する。DSL側は `t!` マクロでFluentのメッセージIDを参照するだけで、複数形・性別分岐・日付/数値フォーマットはFluent自身の構文(`select`式、`NUMBER()`/`DATETIME()`関数)に委譲する。

```rust
#[elwindui::component(inherits VerticalLayout)]
struct OrderSummary {
    #[prop]
    n: i32,
    #[prop]
    price: f64,
    #[prop]
    order: Order,

    body: view! {
        TextBlock { text: t!("dashboard-title") }
        TextBlock { text: t!("cart-item-count", count: n) }
        TextBlock { text: t!("order-saved-at", time: order.created_at) }
        TextBlock { text: t!("item-price", price: price) }
    }
}

#[elwindui::component]
impl OrderSummary {}
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

```rust
#[elwindui::main]
fn main() {
    // 呼び出し元クレートの strings/en.ftl を読み込んでバンドルを初期化する。
    // `t!(..)` が評価されるより前に、一度だけ呼ぶ。
    elwindui::i18n::declare!();
    // ...
}
```

**実装状況の注**: 現在の`declare!()`は引数を取らず、`strings/en.ftl`・ロケール`"en"`に固定されている。
上記の`default`/`fallback`/`available`/`resources`に相当する設定機構は未実装。

- ビルド時に `.ftl` を静的パースし、DSL内で参照している `t!("key", ...)` のメッセージIDが**全`available`言語で定義されているか**を機械的に検証する(未翻訳キーの検出)
- `t!` に渡す引数名は、対応する `.ftl` メッセージ内の `{ $引数名 }` と一致していなければ静的エラーとする

---

## 11. モジュール(import) ✅

```rust
use components::card::Card;
use components::widgets::{StatCard, Badge};
use components::common_kit as UI;
use components::card::Card as ProductCard;
```

- Rustの `use` 構文と完全に一致させる。ビルトイン(§2)を除き、DSLが持つ独自の名前解決規則は存在しない
- 静的にimportを解決し、循環参照・未解決参照を機械的に検出できる
- `use` は対象アイテムの**実際のRustパス**へ解決される。ある型名がコンポーネント定義から参照可能なのは、
  (a) 同じファイル(=同じ実パスを持つモジュール)内でローカルに定義されている場合、または
  (b) その型の実パスを指す `use` がそのファイルにある場合、のいずれかに限る。ディレクトリ内の他の
  別のファイルに同名の型が存在するというだけでは可視にならない(ただし`docs/design/tools/codegen_design.md`が示すとおり、複数の
  定義が結局同じRustスコープに置かれる場合は、その同じスコープ内では通常の
  Rustのファイル分割同様`use`は不要)。ローカル定義でも`use`解決でもない型参照は、Rustの「見つからない
  型」エラーと同様、静的検証エラーとなる
- ViewModelの参照(`docs/design/gui_framework_design.md`§7.2)も同じ規則に従う。参照側は必ずその定義が
  実際にコンパイルされる実Rustパス(`#[elwindui::viewmodel] mod foo { .. }`が実際に宣言されている
  パス、例: `crate::foo::Foo`)を`use`する
- enumの参照(§7)も同じ規則に従う——通常のRustの`use`で導入した名前をそのまま使える

---

## 12. 要素ツリーの探索と要素への注釈 ✅

### 役割分担の方針

「子要素を持つ」という性質は既存の `{}` ネスト構文がそのまま表現しているため、**children専用の新しいDSL構文は追加しない**。ツリー走査専用の別トレイトは設けず、`docs/design/gui_framework_design.md`§5で定義済みの `UIElement`(全要素が実装する唯一の共通トレイト)が `visual_children()`/`parent()` を通じてこの契約をそのまま担う。再帰探索アルゴリズム自体はDSLの文法ではなく、共通ランタイムライブラリ(`elwindui_core::visual_tree`)側の責務とする。

| 責務 | 担当 |
|---|---|
| 親子構造の宣言 | DSL構文(`{}` ネスト。追加構文は不要) |
| 動的生成された子要素(`if`/`for`/`match`の結果)をchildrenとして集約する規約 | コード生成器 |
| 全要素が親子を辿れるという契約(`visual_children()`/`parent()`) | `UIElement`(`docs/design/gui_framework_design.md`§5.1a、コード生成器が全要素型に自動実装) |
| 再帰探索アルゴリズム(`visual_tree::find_all` 等) | 共通ランタイムライブラリ(DSLとは独立に拡張・最適化可能) |
| 特定要素への後からのアクセス | `#[id(...)]` アトリビュート |

### 共通トレイト(コード生成器が自動実装)

`children()`/`id()`だけのための別トレイトは無い。全要素型が既に実装している`UIElement`(`docs/design/gui_framework_design.md`§5.1a)がその役割を兼ねる:

```rust
trait UIElement: AsAny {
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>>;
    fn parent(&self) -> Option<Rc<dyn UIElement>>;
    // ... 他多数(margin/alignment/measure/arrangeなど、`docs/design/gui_framework_design.md`§5参照)
}
```

- `view` 内で `{}` ネストにより宣言された子要素は、そのままコード生成器によって `visual_children()` の返り値に詰められる
- `if` / `for` / `match` によって実行時に確定する子要素も、生成時にフラット化されて同じ `visual_children()` に集約される、という規約に統一する

```rust
#[elwindui::component(inherits HorizontalLayout)]
struct Toolbar {
    #[prop]
    show_save: bool,
    #[prop]
    extra_buttons: Vec<ButtonSpec>,

    body: view! {
        if show_save { ToolbarButton { text: "Save" } }
        for item in extra_buttons { ToolbarButton { text: item.label } }
    }
}

#[elwindui::component]
impl Toolbar {}
```

上記のように条件・繰り返しで生成された要素も、`Toolbar` インスタンスの `visual_children()` から一律に辿れる。

### 再帰探索API:`visual_tree`(共通ランタイムライブラリ、DSL非依存)

`elwindui_core::visual_tree`は、WinUI3の`VisualTreeHelper`に相当する自由関数群を提供する。`UIElement`自体が既に`visual_children()`/`parent()`を持つため、木の走査そのものはこのモジュールを経由しなくても行えるが、`visual_tree`は(a) WinUI3に近い呼び出し形(`visual_tree::get_child(elem, i)`)と、(b) `UIElement`単体には無い型ベースの再帰収集(`find_all`)をまとめて提供する。

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

- idによる文字列検索(`find_by_id`相当)は無い。理由は次項参照 — ランタイムidを保持する要素が存在しない
- 探索方式(深さ優先/幅優先)やキャッシュ戦略の変更は、**DSLの構文を一切変えずに**ライブラリ側の実装更新だけで完結する
- DSL側が保証するのは「`UIElement` トレイトを介してツリー全体に到達可能である」という契約のみ

### 特定要素への名前付きアクセス:`#[id(...)]`

`let` 束縛は同一 `view` 関数内でのみ有効なため、外部(Rustロジック側)から後で要素を参照したい場合は `#[id(...)]` アトリビュートを付与する。

```rust
#[elwindui::component(inherits VerticalLayout)]
struct DocumentView {
    #[prop]
    content: String,

    body: view! {
        #[id("editor")]
        let editor = TextArea { text: content };

        editor
        StatusBar { ... }
    }
}

#[elwindui::component]
impl DocumentView {}
```

- `#[id(...)]` を付けた `let` 束縛は、`{}` ネスト内で裸の識別子として(上記の`editor`のように)参照できる子要素になる
- 実装(`elwindui-codegen`)は`#[id(...)]`ごとに、その束縛の**具象Rust型をそのまま返す名前付きアクセサメソッド**(`pub fn <id>(&self) -> Rc<ConcreteType>`)を、その`view`を持つコンポーネント自身に生成する。`#[id(...)]`が付いた束縛は暗黙的に「実体化後も保持される」扱いになり(通常の子要素同様、動的な属性を持つ場合と同じ`stored`規約)、対応するフィールドから`.clone()`して返すだけの薄いメソッドになる
- **`#[id(...)]`は全てコンパイル時に確定している**ため、実行時に文字列で検索する仕組みは経由しない — 具象型を直接返す静的アクセサの方が`docs/design/gui_framework_design.md`§7.2の「型消去を避け専用コードを生成する」方針に沿っており、ダウンキャストも不要になる
- **ランタイム文字列idによる検索は意図的に提供しない**。`UIElement`自体はidを保持するフィールドを持たず、名前付きアクセスは`#[id(...)]`一本に統一する。これはWinUI3が`VisualTreeHelper`(構造的な木の走査のみ、前項)と`FrameworkElement.FindName`(名前引き)を明確に分離しているのと同じ役割分担であり、`FindName`相当は`#[id(...)]`が静的に担う

### 共通属性:`#[routed]`(ルーティングイベント、WinUI3スタイル)

コールバック型フィールド(`fn()`等)には`#[routed]`アトリビュートを付けられる。付けたイベントは
発生元の要素から祖先へバブルする(WinUI3の`RoutedEvent`相当)。対象は`Button`の
`on_click`のような入力系イベントに限られ、`TabView`の`on_select(usize)`のような
ウィジェット固有の型付きペイロードを持つコールバックはルーティング対象外(直接配線)。

```rust
#[elwindui::component(inherits NativeControl)]
struct Button {
    #[routed]
    on_click: fn(),
}

#[elwindui::component]
impl Button {}
```

ハンドラは要素自身の型消去レジストリ(`UIElementBase.routed_handlers`)にイベント名で登録され、
配送(`elwindui_core::ui::dispatch_routed`)は発生元要素から`UIElementBase.parent`(本物の親
ポインタ、要素が木に組み込まれる際に必ず設定される)を辿って祖先へバブルする。`RoutedEventArgs`の
`handled`フラグが立てられると、そこで伝播が止まる。親ポインタ方式のため、`for` のように
実行時に動的組み立てられた木でも、静的なDSL構造と
同様にバブルが機能する(`docs/design/gui_framework_design.md`§5.10参照)。実装範囲はAppKit・WinUI3両バックエンドの`Button`のポインタ/タップ9イベント(§5.10)に加え、キーボード/フォーカス系5イベント——`on_key_down`/`on_key_up`/`on_text_input`(バブリング)、`on_got_focus`/`on_lost_focus`(非バブリング、`dispatch_direct`)——も`component UIElement`の`#[routed]`フィールドとして宣言されている(`docs/design/gui_framework_design.md`§5.5/§8.1参照)。WinUI3側はWindows環境が無く未検証。

### 要素使用箇所への注釈:`#[shortcut(...)]`(キーボードショートカット)

`#[routed]`が**フィールド宣言**(全インスタンス共通の配線方式)に付くのに対し、`#[shortcut(...)]`は
`Button { ... }`という**要素の使用箇所**に付く注釈である——ショートカットは本質的にインスタンスごとの
決定(「このSaveボタンだけ`Ctrl+S`」)であり、`Button.on_click: fn()`という共有宣言
には付けられないため。構文上は`#[id(...)]`(前項)と同じ「要素の`{}`本体内で特定の行の直前に書く注釈」
という位置づけだが、`let`束縛ではなく通常の`属性名: 値`という属性行の直前に書く点が異なる。

```rust
#[elwindui::component(inherits ContentControl)]
struct SaveButton {
    body: view! {
        Button {
            text: t!("notepad-menu-save")
            #[shortcut("Ctrl+S")]
            on_click: save_document()
        }
    }
}

#[elwindui::component]
impl SaveButton {}
```

**実装状況の注**: 上記の`#[shortcut("Ctrl+S")]`を`view!`内の属性行の直前に置く記法は設計されているが、現状の`elwindui-codegen`のパーサはこの位置での`#[shortcut(...)]`を正しく処理できず、`view! { .. }`本体のパースエラーになる(内部エラーメッセージが`on_click`の直前で解析に失敗する)。動作するまでは`#[shortcut(...)]`を使わずに`on_click`だけを書くこと。

`#[shortcut(...)]`が付けられるのは`#[routed]`なフィールド(`on_click`/`on_key_down`等)のみ。詳細な構文
(`winui3: "..."`/`appkit: "..."`によるバックエンド別指定、`scope: local`)・プラットフォーム変換規則
(macOSでの`Ctrl`→`Cmd`自動読み替え)・実行時の仕組み(`ShortcutRegistry`)は
`docs/design/gui_framework_design.md`§8.1参照。実装範囲はAppKit・WinUI3両バックエンド(WinUI3側未検証)。

---

## 13. 静的検証ルール一覧 🚧

コンパイラ/リンタが実行前に検出すべき項目:

> **実装状況**: `crates/elwindui-codegen/src/validate.rs`は、既に実装済みの言語機能・ビルトインに対応するルール(概ね1〜8, 10〜13, 19, 25, 30〜31 — `#[param]`の静的性、リアクティブ属性式と`<=>`の参照先、`viewmodel`のV/VM分離、`#[shortcut(...)]`の妥当性など)を実際に検査する。一方、対応するビルトイン/機能自体が未実装なルール(9: `target::backend()`自体が存在しないため検査不能、14: `NavigationHost`未実装、15: `Dialog`未実装、16・17: `Transition`/`Effect`未実装、20: `#[async_computed]`未実装、21: `#[undoable]`未実装、22: テーマはDSLブロックではなくRust属性`#[elwindui::theme_definition]`として実装されているため対象外、23: `VirtualList`未実装、24: `on_foreground`等のモバイルライフサイクル未実装、26〜29: `ControlTemplate<Self>`/`#[elwindui::template]`未実装)は`validate.rs`にも対応する検査が存在しない。ルール18は欠番(アクションはRustの`impl`ブロックの`fn`として自動検出されるため、対応する型検査が存在しない)。

1. `#[param]` フィールドの初期化式にリアクティブなprop/state/computed/bindable owner参照が出現 → エラー
2. `#[param]` フィールドの初期化式に非純粋関数(`now()`, `random()` 等)が出現 → エラー(`env::*` は例外)
3. `#[computed]` フィールドへの外部代入 → エラー
4. enum値の裸文字列直書き(完全修飾でない参照) → エラー
5. `match` におけるenumメンバーの網羅漏れ(`_ =>` なし) → エラー
6. 制約(`#[range]`, `#[length]`, `#[pattern]` 等)付きフィールドへのリテラル値代入が制約違反 → ビルド時エラー、動的値の場合は実行時エラー
7. (欠番 — `once` 宣言の廃止に伴い、`external::*` を許可する場所自体が無くなったため)
8. importの循環・未解決パス → エラー
9. 通常の`component`の`view`内に `native!` ブロック、または `target::backend()` の参照が出現 → エラー(`docs/design/gui_framework_design.md`§4.1参照。独自部品はバックエンド共通実装(ビルトイン)に限定する)
10. `view`内に`Canvas`が含まれているが `#[accessible(...)]` が付与されていない → 警告(`docs/design/gui_framework_design.md`§5.6参照)
11. `on_mount`/`on_unmount`ブロックの外で`#[param]`フィールドの再代入相当の操作が行われている → エラー(`docs/design/gui_framework_design.md`§6.1参照。paramの不変性は生涯を通じて保証される)
12. リアクティブ属性式または`<=>`の参照先が`store`宣言(`docs/design/gui_framework_design.md`§7.1)の型・フィールドとして存在しない → エラー
13. `store`フィールドへの`#[param]`側からの直接参照 → エラー(`docs/design/gui_framework_design.md`§7.1参照。storeはViewのリアクティブ属性式から参照する)
14. `NavigationHost`内の`match route { ... }` がRoute enumの全メンバーを網羅していない(`_ =>`なし) → エラー(7章の網羅性検査と同じ仕組み、`docs/specs/builtins_spec.md`付録L.2参照)
15. `Dialog`/`Menu`等のオーバーレイ系ビルトインの外側(通常のcomponent)で`native!`/`target::backend()`が出現 → エラー(ルール9と同じ原則、`docs/specs/builtins_spec.md`付録M参照)
16. `Transition`/`KeyframeAnimation`(`docs/specs/builtins_spec.md`付録N.6)で存在しないイージング関数名、または範囲外のキーフレーム位置(`0.0..=1.0`外)が指定されている → エラー
17. `Effect`(`docs/specs/builtins_spec.md`付録N.3)のパラメータが対応バックエンドでサポートされない組み合わせ(例:GTK4未対応のエフェクト種別)である場合 → 警告(該当バックエンドではフォールバック描画に切り替わる旨を明示)
18. (欠番 — アクションはRustの`impl`ブロックの`fn`として自動検出されるため、対応する型検査が存在しない)
19. `viewmodel`定義内に`view`ブロック、またはビルトイン要素(`Row`/`Text`等)への直接参照が存在する → エラー(`docs/design/gui_framework_design.md`§7.2参照。ViewModelは表示ロジックを持たず、MVVMのV/VM分離を静的に強制する)
20. `#[async_computed]` が `viewmodel`/`store` 以外(通常の`component`のprop等)に付与されている → エラー(`docs/design/gui_framework_design.md`§7.3参照。非同期状態はVM/Model層に閉じ込める)
21. `#[undoable]` が `viewmodel` の `#[observable]` フィールド以外(`store`や`component`のprop等)に付与されている → エラー(`docs/design/gui_framework_design.md`§7.4参照)
22. `theme`の`variant`ブロックが`tokens{}`で宣言されていないトークン名を定義している、または`tokens{}`で宣言された一部のトークンを欠いている → エラー(`docs/design/gui_framework_design.md`§8.5参照。全variant間でトークン集合の一致を保証する)
23. `VirtualList`に`key`が指定されていない状態で`items`の順序が変わる更新が行われる → 警告(`docs/specs/builtins_spec.md`付録Q参照。挿入位置ベースの再利用にフォールバックし、リコンサイル効率が低下する可能性がある)。一般の `for` は `Vec<Rc<T>>` のとき各要素の `Rc<T>` ポインタ同一性で子を再利用し、その他の collection は当該範囲を再構築する(`docs/specs/builtins_spec.md`付録Y参照)。`TabView` は `TabViewItem` を子として指定する。
24. `on_foreground`/`on_background`/`on_terminate`(`docs/design/gui_framework_design.md`§6.2)が、アプリのエントリポイント(ルート)コンポーネント以外で宣言されている → 警告(OSレベルのライフサイクルは単一箇所への集約を推奨)
25. コールバック型のフィールドで `Rc<dyn Fn(...)>` / `Box<dyn Fn(...)>` のような型消去表現を直接使用している(`fn(...)` 糖衣構文を使っていない) → エラー(4章「コールバック型フィールド」参照)
26. `ControlTemplate<T>` の `T` が `Self` 以外 → エラー(4章「`ControlTemplate<Self>`」参照)
27. `ControlTemplate<Self>` 型フィールドに `#[param]` が付与されている → エラー(実行時差し替えができて初めて意味を持つため、常に`prop`でなければならない)
28. `body`/`view` ルートの `<field>(Self)` の `field` が、同一component内で宣言された `ControlTemplate<Self>` 型フィールドでない → エラー
29. `ControlTemplate<Self>` 型フィールドへの裸パス代入が、`#[elwindui::template]` で定義され、かつパラメータ型が厳密に `Self` と一致する関数を指していない → エラー(4章「`#[elwindui::template]`」参照)
30. `#[shortcut(...)]` が `#[routed]` でない属性に付与されている → エラー(12章「`#[shortcut(...)]`」参照。`on_click`等のコールバック属性以外に付けても意味を持たない)
31. `#[shortcut(...)]` に指定されたキー表記(修飾キー名/キー名)が不正 → エラー(`docs/design/gui_framework_design.md`§8.1参照。`codegen::parse_shortcut_spec`と同じパーサーで検査するため、ここを通れば必ずコード生成もパースに成功する)
32. `elwindui::core::graphics::Brush`/`Color`(または`Option<..>`)型のフィールドへ文字列リテラルを代入する場合(例: `Rectangle { fill: "#3a3a3c" }`)、その文字列が`"#rrggbb"`/`"#rrggbbaa"`(`#`省略可)のいずれの形式にも一致しない → コード生成時エラー(`codegen::coerce_color_literal`。動的な`String`式には適用されない——`Brush`/`Color`型の値を直接渡す必要がある)

---

## 14. 全体サンプル

```rust
use components::slider::Slider;

#[elwindui::dsl_enum]
enum Orientation {
    Horizontal,
    Vertical,
}

#[elwindui::component(inherits VerticalLayout)]
struct VolumeControl {
    #[param(default = Orientation::Horizontal)]
    orientation: Orientation,

    #[state(default = 50.0)]
    volume: f32,

    #[computed(expr = volume.to_string() + "%")]
    label: String,

    body: view! {
        let slider = Slider { value <=> volume };

        match orientation {
            Orientation::Horizontal => {
                HorizontalLayout { slider, TextBlock { text: label } }
            }
            Orientation::Vertical => {
                VerticalLayout { slider, TextBlock { text: label } }
            }
        }
    }
}

#[elwindui::component]
impl VolumeControl {}
```

---

# 付録A. component宣言レベルの属性 ✅

`component`宣言の直前に、`inherits`の有無に関わらず0個以上任意の順序で書ける4つの属性(`enum`/`viewmodel`/`view`には付けられない)。ビルトイン25個の宣言は`elwindui-core::ui`/各`elwindui-backend-*`crateの`#[elwindui_macros::class]`宣言そのものであり、これらの属性はそちらのRust宣言に直接付いている。

- **`#[sealed]`** — このコンポーネントを`component X inherits Y`の`Y`(継承元)として指定できないようにする。具象的な末端形状(`Rectangle`/`Ellipse` — 継承したければ合成可能な`Shape`を使う)や、そもそも継承先を持たないネイティブ末端要素(`Button`/`TextArea`/`TabView`/`TabViewItem`)に付与する。
- **`#[abstract]`(Rust形式:`#[abstract_]`)** — このコンポーネントを`view`内で直接インスタンス化できないようにする(`Type { .. }`という形で、属性値・クロージャ本体・裸のネスト子要素・単体の`view`ルートのどこに書いても静的エラー)。`component X inherits Y`の`Y`として指定するのは引き続き可能——むしろそれが本来の使い道で、`#[sealed]`のちょうど逆に位置する。唯一の例外は、`X`自身が`inherits`で名指ししている`#[abstract]`な`base`を、`X`自身の`view`の**ルート要素として**構築する場合(シェイプ合成、`docs/specs/builtins_spec.md`付録F.10の`Shape`の例。`validate::validate_inherits`が「ルート要素は`base`と一致しなければならない」を既に強制しているので、この一箇所だけ安全に許可される)。ビルトインの`UIElement`/`NativeControl`/`Layout`/`Shape`(いずれも「フィールドを持たない純粋なカテゴリタグ」、もしくは`Rectangle`/`Ellipse`が合成する土台)に付いており、直接使うことを意図した具象virtual builtin(`VerticalLayout`/`HorizontalLayout`/`Control`/`Grid`/`TextBlock`)には付かない。`codegen::generate_module`も`#[abstract]`なコンポーネントには`create_<snake case>(..)`/`new(..)`を一切生成しない。
- **`#[text_style]`**(`docs/status/font_status.md`参照) — フォント/テキストスタイルの7プロパティ(`font_family`/`font_size`/`font_weight`/`font_style`/`font_stretch`/`character_spacing`/`foreground`)を、このコンポーネント自身の宣言済みフィールドより前に注入する。実体は`elwindui_core::ui::TextStyleOwner`(手書きトレイト)が持つ`TextStyleStorage`——対応する実Rust実装が存在するビルトイン自身の宣言(`Control`/`TextBlock`/`NativeControl`)にのみ付けられる。個別のネイティブ末端要素(`Button`等)には付けない——コード生成側のディスパッチ規約(`docs/specs/builtins_spec.md`付録F参照)上、共有基底の`NativeControl`に付ける必要があるため。同名フィールドを自前で宣言しているコンポーネントに付けるのは静的エラー。
- **`#[content(field_name)]`** — WinUI3の`ContentPropertyAttribute`相当。ある要素の`view`本体に「属性名を書かない裸のネスト子要素」(`Type { .. }`を`name: value`形式でなく直接`{}`内に書く)を渡した際、それがどのフィールドに束縛されるかを明示する。例:`MenuBarItem`は`#[content(submenu)]`を宣言しており、`MenuBarItem { text: "File", Menu { .. } }`の`Menu { .. }`は`submenu`フィールドに束縛される(`Window`/`ContentControl`/`TabViewItem`の`content`フィールドも同様に`#[content(content)]`を宣言している)。`field_name`は実在するフィールド名でなければならず(静的検証)、componentにつき最大1個。裸のネスト子要素があるのに`#[content(..)]`(または`children: Vec<..>`のようなリストフィールド)が無いcomponentにそれを渡すのはコード生成時エラーになる。この属性はビルトイン限定ではなく、ユーザー定義コンポーネントでも使える。

`#[sealed]`/`#[abstract_]`は上記のとおりユーザー定義コンポーネントでも一般的に使える属性である。一方`#[text_style]`は対応する実Rust実装(`TextStyleStorage`)を持つビルトイン自身の宣言でのみ意味を持つ。

---

> 標準UI要素の個別リファレンス(`Window`/`Button`等)とOS機能アクセス(ファイルダイアログ等)の仕様は
> `docs/specs/builtins_spec.md`にまとめてある。バックエンド抽象化・
> `elwindui-core`ランタイム・ライフサイクル・Store/ViewModel/MVVM等のフレームワーク設計は
> `docs/design/gui_framework_design.md`を参照。
