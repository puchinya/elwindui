# ElwindUIL DSL仕様書

Rust向けGUIフレームワーク(Elwind)のための宣言的レイアウト記述言語(ElwindUIL)の構文・意味論の仕様書。
Rustの構文・慣習に寄せることで学習コストを下げつつ、機械可読性・事前検証性を重視した設計。

本書はDSLの構文・静的検証ルールのみを対象とする。バックエンド抽象化・`elwindui-core`ランタイム・ライフサイクル・Store/ViewModel/MVVM等のGUIフレームワーク本体の設計は `docs/design/README.md`、標準UI要素(`Window`/`Button`等)は `docs/specs/ui_spec.md`、グラフィックス型・描画モデルは `docs/specs/graphics_spec.md`、OS機能(ファイルダイアログ等)は `docs/specs/platform_spec.md`、コード生成・LSP・プレビュー・ホットリロード等のツールチェーンは `docs/design/tools/*.md` を参照。

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
- `view!`は`#[elwindui::component]`の`body`型位置でだけ有効なDSL記法であり、通常のRustコードから単独macroとして呼び出すことはできない
- **`#[elwindui::component] impl Name {}` は、メソッドが1つも無くても常に必須。** 省略すると`Name`というcomponent型は成立しない。本書の以降のコード例は空`impl`も省略しない

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

### 名前空間:Rustのクレート名前空間規則にそのまま従う

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

## 3. component と view

コンポーネントは **`component`(データ定義)** と **`view`(描画ロジック)** の2つの役割に分離する。Rustの `struct` と `impl` の関係に対応する概念的な区分であり、構文上は1つの`struct`定義の中に共存する(§2参照)。

| | component | view |
|---|---|---|
| 役割 | 状態(フィールド)の定義 | 状態→見た目の写像 |
| 対応するRust概念 | `struct Foo { ... }` | `impl Foo { fn view(&self) -> Rc<dyn UIElementExt> }` |
| 書く内容 | 型・制約・初期値のみ | `if`/`for`/`match`による要素ツリーの組み立て |
| 変更頻度 | 低い(型は安定) | 高い(レイアウト調整で頻繁に変わる) |

**`body: view! { .. }` フィールドを持つcomponentは、必ず`inherits`(次項)で何らかのbaseを指定する。** これは単なる制限ではなく、`view!`の中身の書き込み先そのものがbaseに依存するためである——`view!`のトップレベルの属性設定(`padding: padding`のような`key: value`行)はbaseが持つ同名フィールドへの設定・バインディングであり、属性名を書かない裸のネスト子要素は、baseの**実効**`#[content(field_name)]` metadataが指定するフィールドへ lower される。destination の型が scalar なら `set_<field>(child)` に、collection なら collection surface への順序付き挿入になる。`Control`を直接継承するcomponentでは、内部 scalar `visual_root` destination を経由して単一の authored visual rootがprivateなtemplate-root経路へ接続されるが、これは公開`children` collectionではない。`inherits`で指定するbaseが無ければ、このどちらにも書き込み先が存在しない。したがってbase無しで自分自身の視覚ツリーを一から組み立てるcomponentは現状サポートされない——`view`を持つcomponentは常に何らかの合成可能なbase(`VerticalLayout`/`HorizontalLayout`/`Control`等、または他のユーザー定義component)の上に構築する。子要素を並べたいだけの単純なcomponentは、`inherits VerticalLayout`/`inherits HorizontalLayout`(次項の2番目のケース、シェイプ合成)を使うのが最も基本的な書き方になる——この場合`view`の中身がそのままそのレイアウトの子要素になるため、ラッパー要素を書く必要もない。`view!`を持たないcomponent(データ定義のみ、§4参照)にはこの制約はない。

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

- デフォルト値・算出式は`#[prop(default = expr)]`/`#[computed(expr = expr)]`/`#[attached(default = expr)]`のように対応する属性の名前付き引数として渡す。ViewModelの`#[observable(default = expr)]`/`#[computed(expr = expr)]`も同じ表記を使う
- `#[param]`も`#[param(default = expr)]`でデフォルト値を持てる。ただし静的評価式だけを許可し、リアクティブなフィールド参照は拒否する(§4参照)
- 同じcomponentの`#[prop]`/`#[computed]`フィールドは`view`から裸の識別子で参照でき、依存値の変更時に該当する算出値とviewだけが再同期される。依存fieldは通常の式として直接参照するか、`format!("{volume}%")`のような`format!`/`format_args!`のインラインキャプチャ(Rustの`{field}`記法)として参照する——どちらも依存として追跡される。`volume.to_string() + "%"`のようにfield参照が通常の式として現れる形も引き続き使える。それ以外の(`format!`/`format_args!`ではない)任意のmacro token内だけに隠れた参照は、依然として依存として扱われない
- `if`/`match`の条件式は裸のfield参照だけを受け付ける。`if orientation == Orientation::Horizontal`のような比較式は書けないため、enum分岐は`match`を使う

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

### `inherits`:WinUI3方式のクラス継承

`#[elwindui::component(inherits Base)] struct Name { ... }` は `Base` を4通りに解決する(単なる構造的契約ではなく、WinUI3/C#の`Control → ContentControl → Button`と同じ実継承):

1. **`Base`が`NativeControl`マーカー** — 純粋なカテゴリタグ(フィールド継承なし)。ネイティブ実装を持つ末端要素(`Button`等)であることを示すのみ。
2. **`Base`が`view`を持たないプリミティブ形状ファミリー**(例:`Control`/`Rectangle`)、または**`Base`自身が既にシェイプ合成されているDSLコンポーネント**、または**`Base`が`view`を持たないネイティブ実装のホスト**(例:`Window`) — `Base`の`#[param]`/propフィールドを**再宣言なしに自動継承**し、さらに`Name`自身の`view`の中身は**常に暗黙に`Base`自身の属性・子要素**になる(ラッパー要素は書かない——`Base { ... }`という入れ子は書かず、`Base`の属性・子要素を`view`の`{}`直下にそのまま書く)。シェイプ合成/ホスト合成(`docs/specs/ui_spec.md`参照)。
3. **`Base`が自前の`view`を持つ、それ自体は合成されていない論理コンポーネント**(builtinでもユーザー定義でも) — フィールドに加えて`view`(テンプレート)も継承する。`Name`が独自の`view`を書かなければ`Base`のテンプレートをそのまま(WinUI3の既定`ControlTemplate`のように)引き継ぎ、書けば**完全なテンプレート上書き**になる(ルート要素の型に制約はない)。
4. **`Base`がネイティブ実装のみの末端要素**(例:`Button`) — 継承不可。生成されるRustコードを持たないため、委譲先が存在しない。

`Base`の書き方は、それが組み込み(builtin)かユーザー定義かで異なる:組み込みは裸の名前(`inherits Control`/`inherits ContentControl`のように)、ユーザー定義コンポーネントはクレートルート起点の完全修飾パス(`inherits crate::ui::LabeledPanel`)で書く。これは`#[elwindui_macros::class]`の`inherits = ..`引数が同一クレート内でも常に完全修飾パスを要求するのと同じ理由による(`docs/specs/macro_class_spec.md`§7)——生成される`__elwindui_inherit_*!`マクロ連鎖が別モジュールから展開される可能性があるため、裸名は解決できない。ユーザー定義の`Base`を裸名で書くと静的エラーになる。あわせて、`Base`を公開するモジュールは名前を列挙した再エクスポートではなく、必ずグロブ再エクスポート(`pub use some_module::*;`)にすること——`#[class]`は`Base`と同じ位置に伴走する`__elwindui_macros_of_{Base}`エイリアスを生成するため、名前を列挙した再エクスポートではそれが取り残される。

```rust
#[elwindui::component(inherits Control)]
struct ContentControl {
    content: std::rc::Rc<dyn UIElementExt>,
    // padding は Control から自動的に継承される — 再宣言不要、self.padding() がそのまま使える

    body: view! {
        // `Control { .. }` というラッパーは書かない — `view!`の中身が Control の属性と
        // private template-root 用の単一 visual root になる
        padding: padding
        content
    }
}

#[elwindui::component]
impl ContentControl {}
```

ここで`Control`は`elwindui::ui::Control`(ビルトイン、裸名で参照)で、`ContentControl`は上記の例で定義しているユーザー自身のcomponent名——ビルトインの同名`ContentControl`(`docs/specs/ui_spec.md`参照)と衝突しない。ローカルに定義された`ContentControl`は、`elwindui::ui::*`の自動`use`(§2)より常に優先して解決される(Rustの通常の名前解決が、同一スコープのグロブ`use`よりローカル定義を優先するのと同じ)。

`view`の中身が暗黙に`Base`自身になるかどうかは、`Base`が実際に合成可能(2番目のケースに当てはまるか)によって決まり、`Name`自身がラッパーを書くかどうかでは選べない――合成可能な`Base`を持つ`component`の`view`は常にこの形で書く。`Control`のshape compositionだけは公開collectionではなくprivate template rootへ単一rootを接続する。3番目のケース(合成されていない論理コンポーネントの継承)だけが、今まで通り「独自のルート要素を持つ完全なテンプレート上書き」になる。

継承したフィールドは、派生component自身の`view`が**同名のまま裸で参照**している場合のみ、派生側の実効フィールド(＝コンストラクタ引数)になる。リテラル値で上書きしている場合(例:`Rectangle { fill: "#3a3a3c" }`)や、そもそも参照していない場合は、その基底フィールドは派生側の公開APIには現れない。

**メソッド継承とオーバーライド**(C#の`virtual`/`override`/`base.Method()`相当)。

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
- `base::name(...)` — オーバーライドした本体から基底実装を呼び出す(C#の`base.Method()`相当)。同じ書き方で`on_mount`/`on_unmount`(`docs/design/runtime/ui_tree_design.md`)内から基底のライフサイクルフックを呼ぶこともできる
- 継承・オーバーライドは1階層(直接の`inherits`先)のみ保証される。2階層以上に渡る`base::`連鎖は対象外
- `impl`側の`#[elwindui::component]`は**引数なし**で書く。`#[class]`と同じく、対応する`struct`が同一ソース上で先に宣言されている必要がある
- **`impl`ブロックはメソッドが無くても必須**。型を生成するのは`impl`側であり、`struct`側は宣言を登録するだけである(`#[class]`が`struct`側で引数を保存し`impl`側で生成するのと同じ分担)。メソッドを持たないコンポーネントも`#[elwindui::component] impl Name {}`を書く
- `fn`には`#[overridable]`か`#[overrides]`のいずれかが必須。`&self`レシーバ・プレーンな識別子引数のみで、ジェネリクス・`where`句・`async`/`unsafe`・トレイト`impl`は受け付けない(通常の`impl`ブロックに書く)
- `#[elwindui_macros::class]`(ビルトインのRustクラス階層マクロ、`docs/specs/macro_class_spec.md`§8.3)にも同じ`#[overridable]`/`#[overrides]`があるが、そちらは`elwindui-core`/バックエンドが手書きするRustクラス階層に対する仕組みで、こちらはコンポーネント継承チェーン上のメソッドオーバーライド(AST上の`MethodDef`)である。属性名と意味論を意図的に揃えてあるが、実装もスコープも独立している

`#[computed]`フィールドも同様に、基底の同名フィールドを`#[overrides]`なしで再宣言するとエラーになり、`#[overrides]`を付けると上書きとして扱われる(型は基底と一致していなければならない)。

### Lifecycle hooks

`view!`本体のroot要素より前に、component lifecycleへ対応するhookを書ける。

```rust
body: view! {
    on_mount: { load_document(); }
    on_update(content): { mark_unsaved(); }
    on_unmount: { save_draft(); }

    Window { }
}
```

- `on_mount`はcomponentがUI treeへ初めて接続された直後に一度だけ実行する
- `on_unmount`はcomponentがUI treeから切り離される直前に一度だけ実行する
- `on_update(field, ...)`は列挙した`#[prop]`または`#[computed]`のいずれかが変更された後に実行する
- 引数なしの`on_update`は任意の`#[prop]`変更を監視する
- 初期構築時の値設定は`on_update`として数えない
- hookは通常のRust blockであり、副作用を実行できる
- 派生componentは`base::on_mount()`と`base::on_unmount()`を明示して直接の基底hookを呼べる
- `#[param]`はhook内でも再代入できない

attach/detach、subscription、backend resourceの内部順序は
[`ui_tree_design.md`](../design/runtime/ui_tree_design.md)を参照する。

### `viewmodel`と`store`:宣言構文

`viewmodel`と`store`は、`component`とは別のアイテム種別で、実Rustの属性マクロを`mod`に付けて宣言する。`mod`内に`struct`(必須、1つ)と`impl`(任意、1つ)を書く:

```rust
#[elwindui::viewmodel]
mod counter_vm {
    pub struct Counter {
        #[observable(default = 0i32)]
        count: i32,

        #[computed(expr = count * 2)]
        doubled: i32,

        #[async_computed(expr = fetch_remote_total(count))]
        remote_total: i64,
    }

    impl Counter {
        fn increment(&self) { count = count + 1; }
    }
}

#[elwindui::store]
mod counter_store {
    pub struct CounterStore {
        #[observable(default = 0i32)]
        count: i32,
    }

    impl CounterStore {
        fn increment(&self) { count = count + 1; }
    }
}
```

`store`は`viewmodel`と同一のフィールド文法を再利用する:

- `#[observable(default = expr)]`:実行時に読み書き可能な状態。setterは変更後に型付き`PropertyChanged`(§9参照)を発火する。
- `#[computed(expr = expr)]`:`component`の`#[computed]`と同じ意味——依存する`#[observable]`/`#[computed]`フィールドの変化に同期的に追従する読み取り専用の算出値。
- `#[async_computed(expr = expr)]`:`expr`は`Result<T, E>`(`E: std::fmt::Display`)を返す非同期式で、依存フィールドが変化するたびに`spawn_local`(`docs/design/runtime/state_management_design.md`「Async work」参照)へ再スポーンされる。生成されるgetterの戻り値型は宣言した値型`T`ではなく`elwindui::core::reactive::AsyncComputed<T>`(`Loading`/`Ready(T)`/`Failed(String)`)——フィールド宣言自体は`T`のままでよく、ラップはコード生成が行う。再スポーンは直前の実行中タスクをフィールド単位の世代カウンタで supersede するだけで、真のキャンセルではない(詳細は設計ドキュメント参照)。`viewmodel`/`store`以外への付与は§13ルール20により拒否される。
- `impl`ブロック内の`fn`/`async fn`はそのままaction(§13で別途言及)として自動検出され、追加の属性は不要。

**`store`固有の性質——プロセス全体のシングルトン**: `store`型のインスタンスは`component`が保持する`viewmodel`(`#[bindable]`)とは異なり、どのcomponentにも所有されない。`TypeName::instance() -> Rc<TypeName>`が、初回呼び出し時に遅延構築される共有インスタンスを返す(`docs/design/runtime/state_management_design.md`「Stores」参照)。`view!`内からは所有フィールドを介さず、型名で修飾した裸参照`TypeName.field`で直接参照する想定である:

```rust
body: view! {
    TextBlock { text: CounterStore.count }
    Button { on_click: || CounterStore::instance().increment() }
}
```

このパスは§13ルール12(参照先フィールドの存在検証)・ルール13(`#[param]`側からの直接参照の禁止)の対象になる。**現状の実装状況**: `store`宣言・シングルトンアクセス(`TypeName::instance()`)・`#[async_computed]`は実装済みだが、`view!`内での`TypeName.field`裸参照とその自動購読コード生成、および対応するルール12/13の検証はまだ実装されていない——`docs/status/implementation_status.md`を参照。それまでは、storeのフィールドは`TypeName::instance().field()`という通常のRust呼び出しとして(actionや`on_click`等の中で)読み書きする。

---

## 4. componentフィールドの種類(param/prop/state/computed)

component の各フィールドには、実体化時に固定されるか実行時に変更できるか・値がどこから来るかを表すアトリビュートを付けられる。**アトリビュートを何も付けなければ既定で`#[prop]`(実行時に読み書き可能)になる**——`#[prop]`は明示的に書いてもよいが、省略時のデフォルトでもある。

| | `#[param]` | `#[prop]`(既定) | `#[state]` | `#[computed]` |
|---|---|---|---|---|
| 変更可能性 | 実体化時のみ、以後イミュータブル | 実行時いつでも変更可 | 実行時いつでも変更可(component自身のロジックから) | 不可——依存フィールドの変化に応じて自動再計算される読み取り専用値 |
| 使える式 | 静的評価式のみ(リテラル・他paramの参照・純粋関数・`env::*`) | 静的評価式に加え、他prop/state/computed/bindable ownerを参照するリアクティブ式 | `default`は静的評価式のみ(初期値)。以後の値は`view`/イベントハンドラからの代入で決まる | `#[computed(expr = ...)]`の式(依存する他フィールドへの参照を含む) |
| 主な用途 | 構造分岐(`if`/`for`の条件)、レイアウト決定 | 表示内容・状態の動的更新 | componentが内部だけで使う非公開の状態(外部の構築引数・公開APIには現れない、§9参照) | 他フィールドから導出される算出値(例:合計金額、書式化した文字列) |
| 実行時アクセス | `instance.field()`(getterのみ。代入相当の操作はコンパイルエラー) | `instance.field()`/`instance.set_field(value)`(後述) | component内部の`self.field()`/`self.set_field(value)` | `instance.field()`(getterのみ) |

### `#[param]`:実体化時に固定

`#[param]`フィールドの初期化式には**静的評価式**のみを書ける。

**許可される要素:**

- リテラル(数値・文字列・真偽値・配列)
- 四則演算・比較・三項演算子相当の `if` 式
- 組み込み純粋関数(`min`, `max`, `round` など)
- 同一コンポーネント内の他の `#[param]` フィールドへの参照
- `env::*`(動的定数、§8)

**禁止される要素:**

- リアクティブな`#[prop]`/`#[state]`/`#[bindable]`プロパティの参照
- prop(`#[param]`が付いていないフィールド)の参照
- 非純粋関数(`now()`, `random()` など)の直接呼び出し

### `#[prop]`(既定):実行時に読み書き可能

**`#[prop]`フィールドへのアクセスは、コード生成器が自動生成する`pub fn <field>(&self) -> T`(getter)/`pub fn set_<field>(&self, value: T)`(setter)を通じて行う**——フィールド自体は`struct`上に公開されず、この2メソッドが外部・内部を問わず唯一のアクセス経路になる。setterは値を書き換えるだけでなく、そのcomponent専用の型付き通知(`{Component}Property`、viewmodelの`PropertyChanged`と同じ設計、§9参照)を発火し、依存する`view`の該当箇所・依存する`#[computed]`フィールドだけを再同期する。

setterが生成されるかどうかは、**そのcomponentが`view`を持つかどうか**で分かれる:

- **`view`を持つcomponent**(`body: view! { .. }`フィールドがある)では、`#[prop]`フィールドはdefault値の有無に関わらず**常に**getter+setterの両方を持つ——default値の無い必須フィールド(`new(..)`のコンストラクタ引数)であっても、実体化後にsetterで書き換えられる
- **`view`を持たないcomponent**(データ定義のみ)では、再同期すべき`view`が無いぶん簡略化されており、default値を持つフィールド(`#[prop(default = expr)]`)または`Option<T>`型のフィールドだけがsetterを持つ。default値の無い必須の非`Option`フィールドはgetterのみになる(`#[param]`と同じ形)

### `#[state]`:component専用の非公開状態

`#[state(default = expr)]`はcomponent専用の非公開リアクティブ状態。`default`は必須で、コンストラクタ引数・公開getter/setter・props APIには現れない——外部から与えることも読み取ることもできない、component自身の`view`/イベントハンドラの中だけで完結する状態を表す(§9のTwoWayバインディングの例`VolumeSlider`を参照)。runtime上のアクセス経路・再同期の仕組みは`#[prop]`と同じ(component専用の型付き通知経由)で、公開されるかどうかだけが異なる。

### `#[computed]`:算出値

`#[computed]` を付けたフィールドは依存する他フィールドの変化に応じて自動再評価される読み取り専用の算出値。外部からの代入は静的エラーとなる。

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

### `#[bindable]`:viewmodelの保持

`#[bindable]`はcomponentがviewmodelを保持するための専用アトリビュートで、**指定できる型はviewmodel(`#[elwindui::viewmodel]`で定義された型)に限られる**——viewmodel以外の型を指定すると、生成されるコードがviewmodel専用のPropertyChanged購読の仕組みを満たせずコンパイルエラーになる。実体化時に一度だけ固定される(以後差し替え不可)という点は`#[param]`と同様だが、`#[bindable]`自身が値を書き換えるわけではなく、保持しているviewmodelの中のフィールドが変化した際に依存する`view`部分を自動再同期させるための購読を張る(`docs/design/runtime/state_management_design.md`参照)。

### `#[environment(name)]`:継承されるUI context値の参照

親から継承される型付きの `Environment` 値(`docs/specs/theme_environment_spec.md`参照)を読み取るためのフィールドアトリビュート。`name` は `#[elwindui::environment_key]`(同spec参照)で定義されたEnvironment Keyの名前か、フレームワーク組み込みのEnvironment Key名(下記)のいずれかを指す。

```rust
#[elwindui::component]
struct SettingsView {
    #[environment(locale)]
    locale: Locale,

    body: view! {
        TextBlock {
            text: format!("{}", locale.identifier())
        }
    }
}

#[elwindui::component]
impl SettingsView {}
```

`name`は次の2形式を取る(Issue #129):

- bare識別子(`locale`): 次の優先順位で解決される(`component_frontend::lookup_environment_key`)。
  1. 宣言元と**同一crate内**で先に宣言された`#[elwindui::environment_key(name = locale, ..)]`のKey(従来通り、ユーザー/ライブラリ定義)。
  2. 上記に該当がなければ、**フレームワーク組み込みのEnvironment Key**——`#[elwindui::environment_key]`宣言なしに常に解決可能な固定名の集合。現在の組み込みKeyは、Semantic Style Brush Key群(`primary`/`secondary`/`tertiary`/`foreground`/`background`/`window_background`/`tint`/`selection`/`separator`/`placeholder`/`link`、`Value = BrushStyle`、`theme_environment_spec.md`§7)と、`popup_dismiss`(`Value = Option<PopupDismissAction>`——Keyの既定値は`None`、`ContextMenuService::open_custom_popup`がpopup-scoped Environmentへ`Some(..)`を設定する。DSL管理経路（popup機構自体）はpopup scopeの外側で`Some(..)`を設定しない。低レベルの型付きRust API（`EnvironmentContext::set`）による明示的な上書きは制限されない——詳細は`theme_environment_spec.md`§2参照)。
  3. どちらにも該当しなければ、コード生成時の`compile_error!`(13章ルール34)。
  - ユーザー定義Keyと組み込みKeyの名前が衝突した場合、ユーザー定義Keyが優先される(組み込みKeyへは決してフォールバックしない、フレームワーク自身が同名を再定義することはない)。
- 完全修飾クレートパス(`some_crate::locale`): 宣言元crateが`pub`にエクスポートしたKeyへ、**クレート境界を越えて**解決される。パスの先頭部分(最終セグメントを除く全体)は、呼び出し側で解決可能な任意のcrateパス(依存クレート名、`use`で導入したエイリアス、ネストしたモジュールパスなど)でよい——`#[elwindui::environment_key]`は宣言元クレートのルートへ常にエクスポートされるため、Key構造体自身がどのモジュールに置かれているかとは無関係である。組み込みKeyには完全修飾形式は存在しない(常にbare識別子でのみ参照する)。

**読み取り（read）専用の解決規則であることに注意**: 上記の bare識別子解決順序(`component_frontend::lookup_environment_key`)は `#[environment(name)]` フィールド(値の**読み取り**)専用であり、Environment値を**書き込む**DSL構文(`EnvironmentScope { key: value }`、5章、および `#[elwindui::theme]`、`docs/specs/theme_environment_spec.md`§3/§4)は別の解決関数(`component_frontend::lookup_writable_environment_key`)を用いる。書き込み側の優先順位は次の2段階のみである。

1. 宣言元と同一crate内で先に宣言された`#[elwindui::environment_key(name = ..)]`のKey(read側と同じ、ユーザー定義Keyは常に書き込み可能)。
2. Semantic Style Brush Key群(`theme_environment_spec.md`§7)。

**`popup_dismiss` はこの書き込み側の解決には含まれない**——`ContextMenuService::open_custom_popup`のみがpopup-scoped Environmentへ`PopupDismissAction`を設定でき、`EnvironmentScope`や`#[elwindui::theme]`経由でフレームワークの現在のdismissアクションを上書きすることはできない(`docs/specs/theme_environment_spec.md`§2参照)。同名のユーザー定義Key(`#[elwindui::environment_key(name = popup_dismiss, ..)]`)を同一crate内で宣言した場合は、その優先順位(ルール1)により通常のユーザーKeyとして書き込み可能になる——これは他の組み込みKey名のシャドーイングと同じ挙動であり、`popup_dismiss`という名前文字列自体を特別扱いして拒否するわけではない。

```rust
#[elwindui::component]
struct SettingsView {
    #[environment(locale_crate::locale)]
    locale: Locale,

    body: view! {
        TextBlock {
            text: format!("{}", locale.identifier())
        }
    }
}

#[elwindui::component]
impl SettingsView {}
```

両形式は排他的であり、bareで見つからない場合に完全修飾形式へフォールバックする、といった動作はしない。完全修飾形式で参照したKeyが宣言元crateに存在しない場合、`elwindui-codegen`はコード生成時点でこれを検出できない(別crateがどのマクロをエクスポートしているかはproc-macro展開からは分からない)——実際にコンパイルされた時点で`rustc`自身の「マクロが見つからない」エラーとして検出される(bare識別子形式の`compile_error!`とは異なる。13章ルール34参照)。`value`の型不一致はいずれの形式でも通常の`rustc`型エラーとして検出される。実装は`docs/design/tools/environment_key_macro_design.md`を参照。

`#[environment]`フィールドは:

- 読み取り専用(`instance.field()`のgetterのみ。setter相当の操作は静的エラー)
- リアクティブ——参照元のEnvironment値が変化すると、依存する`view`の該当箇所だけが再同期される(§9「変更の伝搬」と同じ仕組み)
- 実体化時に固定される値ではなく親から継承される値であり、コンストラクタ引数にはならない
- public propertyではなく、component自身のstateでもなく、TwoWay Bindingのtargetにもならない

`#[param]`/`#[prop]`/`#[state]`/`#[bindable]`との併用は静的エラー(13章ルール33)。

`#[param]`初期化式中の`env::*`(§8「動的定数」)とは別の仕組みである。`env::*`は実体化時に一度だけ評価される静的なOS/実行環境定数(`env::os()`等)で、以後変化せず継承もされない。`#[environment(name)]`は親から継承され実行時に変化しうるリアクティブなUI context値であり、`EnvironmentScope`によってsubtreeごとに上書きできる(§5「`EnvironmentScope`」参照)。

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
  その`view`ルートがネイティブでない場合(`into_node()`経由で`Rc<dyn UIElementExt>`として取り出せる場合)
  ——いずれも構築直後に`elwindui-codegen`の`emit_common_ui_element_setters`/`emit_construction`が
  `(erased).base().set_attached::<T>(..)`を呼ぶことで反映される(`docs/design/runtime/ui_tree_design.md`)。`view`ルート自身が
  ネイティブに解決するユーザー定義component(`inherits NativeControl`を宣言せず`Button`等を
  ラップするようなケース)への設定は、`.base()`へ到達する手段自体がまだ無く、引き続き未対応
  ——将来の拡張課題

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
実行時型消去は発生しない(`docs/design/runtime/state_management_design.md`の「型消去を避け専用コードを生成する」方針と同じ)。

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
- クロージャ本体内の`vm.field`/`vm.action(args)`のような参照は、他のDSL式と同じ規則で解決される(コード生成側の詳細は`docs/design/runtime/state_management_design.md`参照)

### `ControlTemplate<C>`:mount-timeのtyped template

normative contractは[`control_template_spec.md`](control_template_spec.md)に分離する。
初期版はinstance propertyを持たず、`#[component(template = key)]`が指定したEnvironment Keyから
mount時に一度だけ選択する。Keyが`None`なら`body: view!`をdefault templateとして構築する。

```rust
#[elwindui::component(inherits Control, template = rounded_panel_template)]
struct RoundedPanel {
    body: view! { /* default template */ },
}

#[elwindui::control_template(target = RoundedPanel)]
struct CompactRoundedPanelTemplate {
    body: view! {
        VerticalLayout {
            TextBlock { text: templated_parent.label }
            ContentPresenter {}
        }
    },
}
```

- `templated_parent`はtarget型へ静的に型付けされ、getter、TwoWay setter、PropertyChanged resyncを既存生成経路で利用する。
- `ContentPresenter`は`ContentControl`のlogical contentをVisual表示する。template内では静的に0個または1個だけ許可し、dynamic region内では使えない。
- replaceable body内の`#[id(...)]`は`TemplatePart`契約がない初期版では禁止する。
- `NativeControl`、非`Control` target、Key型不一致は生成Rustのtrait bound・型一致でコンパイル時に拒否する。
- per-instance `template:`、mount後の再テンプレート化、`TemplatePart`、`VisualState`は対象外である。

### `view! { .. }`を属性値とする糖衣構文(deferred view、Issue #162)

`ViewTemplate`または`Option<ViewTemplate>`型のフィールド(現時点では`context_popup`のみ、`docs/specs/ui_spec.md`参照——実際にどちらの型で受けるかは宣言側が決め、代入側は宣言された型を`elwindui::core::ui::DeferredViewAssignmentTarget::from_view_template`経由で型主導に変換する)には、通常の式の代わりに裸の`view! { .. }`ブロックを直接書ける。これは新しい構築時ではなく**評価が遅延される**View — 実際に構築されるのは、そのフィールドの用途が定める「開かれた」時点(`context_popup`ならpopupが開かれた瞬間)である。

```rust
#[elwindui::component]
struct DocumentTabs {
    #[bindable]
    vm: std::rc::Rc<DocumentTabsViewModel>,

    body: view! {
        TabView {
            context_popup: view! {
                VerticalLayout {
                    TextBlock { text: vm.selected_label }
                    Button {
                        text: "Close"
                        on_click: vm.close_tab
                    }
                }
            }
        }
    }
}
```

- ブロック内は通常の`view!`本体と全く同じ文法(`on_mount`/`lets`/`if`/`match`/`for`を含む)であり、専用の制限は無い。
- ブロック内の裸の識別子は、そのブロックを**字句的に囲むComponent自身**の以下の範囲に対して、通常の`view!`本体と同じ名前解決規則で解決される——`ControlTemplate`の`templated_parent.foo`のような明示的修飾は不要であり、また使えない:
  - enclosing Component自身の`field`/`state`/`param`/`computed`/`environment`値(裸の一段名、例: `selected_label`)。
  - enclosing Component自身の`#[bindable]`ownerを経由した二段パス(例: `vm.selected_label`、`vm.close_tab`)。
  - enclosing Component自身の書込み可能な(`prop`/`state`の)field/state値への代入——生成されたsetterへ経路付けられる。
  - 上記のいずれにも該当しない裸の名前(モジュール定数、`None`、通常の自由関数呼び出しなど)は、通常のRustの名前としてそのまま残る。
  - enclosing Component自身の**任意のメソッド**が暗黙に`self`扱いされるわけではない——ブロック内で明示的な`self`を書いた場合、それは(ブロックがlowerされる)隠しComponent自身を指す、通常のRustの`self`のままである。
- 評価(構築)は宣言時点ではなく、そのフィールドの用途が定める時点で行われ、その時点でのenclosing Componentの現在値を読む。enclosing Componentが既に解放されている場合は何も構築されず、フィールドの実行時型(`ViewTemplate`)が`None`相当を返す。
- **ライブ更新**: ブロックが実際に開かれて(popupなら表示されて)いる間、上記のenclosing Component自身のfield/state/computed/environment値、および`#[bindable]` ownerを経由した値は、通常のComponent自身の`view!`本体が持つのと同じ購読・resync規則(`PropertyChanged`)に従ってライブ更新される——ブロックが閉じられれば、対応する購読も解放される。
- 生成コードはmacro展開時にこのブロックを独立した隠しComponent/View pairへlowerし、既存の`view!`構築pipelineへ委譲する——実行時に新しい束縛機構は導入しない。詳細は`docs/design/tools/codegen_design.md`§3.35、`docs/design/runtime/popup_context_menu_design.md`の該当節を参照。

---

## 5. 制御構文

Rust標準の制御構文(`for`/`if`/`match`)をそのまま採用し、専用のテンプレートディレクティブは設けない。`view!`の中でこれらを書いた位置は「動的領域」として扱われ、参照しているプロパティの値に応じて実際に生成されるUI要素が変わる——値が変化するたびに、その領域だけが差し替えられ、componentの`view`全体が再構築されることはない(§9「変更の伝搬」参照)。

| 構文 | 用途 | 条件/対象の書き方 | 生成されるUI要素 |
|---|---|---|---|
| `for item in collection` | コレクションの要素ごとに繰り返す | `collection`は裸のプロパティ参照 | 各itemごとに1組ずつ |
| `if cond { .. } else { .. }` | 真偽値による条件分岐 | `cond`は裸の真偽値プロパティ参照(比較式は不可) | 選択された1分岐のみ |
| `match value { .. }` | enumによる分岐(網羅性検査つき) | `value`は裸のenumプロパティ参照 | 選択された1分岐のみ |

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

### `for`:繰り返し

`collection`の各要素ごとに、繰り返されるテンプレート(bodyの子要素)からUI要素が1組ずつ生成される。`for`自身のbody(繰り返される側のテンプレート)は**リテラル要素のみ**で、その中にさらに`if`/`match`/`for`を入れ子にすることはできない(各item は使い捨てのローカル構造体であり、入れ子の動的領域を持つ永続状態を持たないため)。

`collection`が変わった際にどの要素を作り直し、どの要素を使い回すかは、対象コレクションの型によって決まる:

- **`collection`が`Vec<Rc<T>>`型の場合**(またはbodyがitemを子componentの`#[bindable]`へ束縛している場合)、`Rc` identityをitem identityとして使う。同じ`Rc`実体は対応するUI要素・購読を再利用し、削除されたitemの要素は破棄する
- **それ以外の(`Rc`でラップされていない)コレクションの場合**、識別可能な安定したidが無いため、`for`が再評価されるたびにその範囲のUI要素を丸ごと作り直す(既存の要素は再利用されない)
- 13章ルール23も参照(`VirtualList`の`key`未指定時の挙動を含む、より詳しい規則)

### `if`/`match`:条件分岐

`match` は列挙体の全メンバーを網羅していれば `_ =>` を省略できる。網羅されていない場合はコンパイルエラーとなる(Rustの`match`と同じ挙動)。

`if`/`match`の条件式は裸のプロパティ参照のみを受け付ける——`if is_admin`のような真偽値フィールドの参照や`match status`のような列挙体フィールドの参照はできるが、`if orientation == Orientation::Horizontal`のような比較式は書けない(§3参照)。enumによる分岐は常に`match`を使う。

`if`/`match`の各分岐(`else if`チェーンを含む)には、さらに`if`/`match`/`for`を入れ子で書ける——`else if`は`else`ブロックの中にネストした`if`が1つある形として扱われる。

実際にUI要素として構築されるのは、選択された1つの分岐だけである——選択されなかった分岐は構築されない。条件/`match`対象の値が変わって選択される分岐が切り替わると、それまで構築されていた要素は破棄され、新しく選択された分岐が構築される(分岐間で要素が使い回されることはない)。

### 子要素の格納先フィールドによる制約

子要素の格納先フィールド(付録A `#[content(field_name)]`参照)がリスト型(`Vec<..>`/`ListExt<..>`)の場合、`if`/`match`/`for`のいずれも使える(前節の入れ子ルールも同様)。フィールドが単一値型(例:`ContentControl`/`Window`の`content: Rc<dyn UIElementExt>`)の場合は`if`/`match`のみ使え、`for`は使えない(可変長のリストは単一の格納先に収まらないため)。単一値フィールド配下の`if`/`match`は、入れ子も含めたあらゆる分岐が最終的にちょうど1個の要素に還元できなければならない(1分岐に複数の裸の子要素を書くこともできない)。

### `EnvironmentScope`:subtreeへのEnvironment override

`view!`内で`EnvironmentScope { key: value ... }`と書くと、その内側のchildrenに限り指定したEnvironment Key(`docs/specs/theme_environment_spec.md`参照)の値を上書きできる。`key`は`#[elwindui::environment_key]`で定義されたKeyの名前、`value`はそのKeyの`Value`型に一致する式。

```rust
body: view! {
    EnvironmentScope {
        locale: Locale::new("en-US")

        SettingsView {}
    }
}
```

`#[environment(name)]`と同様、`key`は同一crate内のbare識別子(`locale`)に加えて、クレート境界を越えた完全修飾形式(Issue #129)も取れる:

```rust
body: view! {
    EnvironmentScope {
        locale_crate::locale: Locale::new("en-US")

        SettingsView {}
    }
}
```

ただしこちらは`key: value`が付属プロパティ構文(`Owner::field: value`、§3)の構文をそのまま流用しているため、`#[environment(name)]`の完全修飾形式(任意の深さの`syn::Path`を受け付ける)と異なり、**必ずちょうど1個の`::`**(クレート名/エイリアス + キー名)でなければならない——多段モジュールパスは書けない(`docs/design/tools/environment_key_macro_design.md`参照)。

- `EnvironmentScope`自身はUI要素・Render nodeを生成しない——親のEnvironmentをderiveし、指定したKeyだけを上書きした派生Environmentをchildrenの`mount`に渡すだけである(構築時ではない——`docs/design/runtime/component_lifecycle_design.md`参照)
- 上書きされなかったKeyは親のEnvironment値をそのまま参照する(`docs/specs/theme_environment_spec.md`の継承規則を参照)
- `EnvironmentScope`は入れ子にできる——内側の`EnvironmentScope`は自身を囲む(外側の)`EnvironmentScope`の派生Environmentからさらにderiveする(コンポーネント自身の`__mount_environment`から直接deriveするわけではない)
- `EnvironmentScope`の直下、または`EnvironmentScope`直下の`if`/`match`分岐内に書かれた裸の要素は、いずれもscope付きmountの対象になる。`if`/`match`分岐が(分岐内が子要素を持たない単一の裸要素のみで構成されるなど)本来ならlazy-once化(初回到達時まで構築を遅延)の対象になる形であっても、`EnvironmentScope`の内側にある場合は常に即時(eager)構築される——lazy化された枝は`__build_view()`とは別の生成メソッド(`__refresh_dynamic_regions`)から後で構築されるため、scopeが導出した`EnvironmentContext`を保持するローカル変数を参照できないための制約であり、mount先のEnvironmentが誤る(scopeが適用されない)ことはない
- `EnvironmentScope`の直下に`for`を直接書いた場合は現時点で未対応——`for`の各要素は`__build_view()`より後まで存続する専用のrendererから随時構築されるため、(`EnvironmentScope`が存在しない場合と同様に)通常の非scope経路でmount・構築される——既知の制限

---

## 6. 値制約(アトリビュートによる数式的表現)

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

## 7. 列挙体(enum)

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

- 値の参照は `EnumName::Member` というパス形式で書く(裸の文字列リテラルの直書きは型不一致として静的エラー)。**パスの綴り方は通常のRustの`use`と同じ**——`use`で導入した短い名前をそのまま`Member`側に使ってよく、クレートを跨いだフルパスを書く必要はない。`#[elwindui::dsl_enum]`は本体を変更しない通常のRust `enum`であり、名前解決は通常のRustコンパイラ規則に従う(§11参照)
- `EnumName::values()` で全メンバーを列挙可能(`for`との組み合わせで選択UIを自動生成できる)
- `#[label(...)]` アトリビュートで多言語表示名を付与でき、`member.label()` で現在ロケールの文字列を取得する
- `match` と組み合わせることで、全メンバーを処理しているかどうかの網羅性検査が働く。この検査は二層構造になっている:同一クレート内で`#[elwindui::dsl_enum]`登録済みのenumについては`elwindui-codegen`自身がマクロ展開時に早期エラーを出す(同一クレート内限定——プロセスごとに別のrustc起動になる実際の`cargo build`では、この早期検査を別クレートのenumへ拡張する手段が原理的に存在しない、`docs/design/tools/codegen_design.md`参照)。それとは独立に、生成される`match`は常に合成的な`_ =>`を持たない素のRust `match`として出力されるため、**別クレートから`use`したenumも含め、非網羅的な`match`は最終的に必ずrustc自身が`E0004`として検出する**——早期検査が効かない場合でも、コンパイルが通ってしまうことはない

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

## 8. 動的定数(env)

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

`env::*`は実体化時に一度だけ確定する静的なOS/実行環境定数であり、親から継承されたり実行時に変化したりはしない。UI階層に沿って継承され、実行時に変化しリアクティブに再同期される値は`Environment`(4章「`#[environment(name)]`」、`docs/specs/theme_environment_spec.md`参照)を使う。

---

## 9. データバインディング

**データバインディング**とは、UI要素の属性値をcomponentの状態(フィールド)に結び付け、状態が変化したときにUIの表示を自動的に追従させる仕組みである。手続き的に「値が変わったら該当するUI要素を書き換える」というコードを書く必要はなく、`view!`の中で属性値をフィールド参照として書くだけでよい——コード生成器が依存関係を静的に解析し、状態変化を検知して該当箇所だけを再同期するコードを生成する(§5の制御構文がどうUI要素を生成するか、後述の「変更の伝搬」も参照)。

バインディングには向きがあり、componentの属性値(`property: value`の`value`部分)の**書き方**によってどの向きになるかが決まる: UIへ一方向に反映するだけのもの(OneWay)、初期値を一度だけ渡すもの(Once)、UIでの入力をcomponentの状態へ書き戻すもの(TwoWay)の3種類がある:

| 書き方 | 更新タイミング |
|---|---|
| `property: expr` | 依存解析による自動分類(下記)——OneWay(依存先が変わるたびに再評価)またはOnce(初期化時に一度だけ) |
| `property: once!(expr)` | 常にOnce(初期化時に一度だけ評価。依存解析自体を行わない) |
| `property <=> target` | TwoWay(値の変更が双方向に伝わる。書き方は明示的) |

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

上の例では、`Slider`の`value`はTwoWay(`volume`の変化がスライダーの表示位置に伝わり、スライダー操作も`volume`に書き戻る)、1つ目の`TextBlock`はOneWay(`volume`が変わるたびに再評価)、2つ目の`TextBlock`はOnce(構築時の`volume`の値でメッセージを1回だけ組み立て、以後`volume`が変わっても更新されない)になる。

### `property: expr` の自動分類

依存解析によって、`expr`の中に**リアクティブな参照**が見つかるかどうかで自動的に分類する:

- **見つかれば OneWay**(依存先が変わるたびに再評価): 自componentの`#[prop]`/`#[state]`/`#[computed]`フィールドへの裸参照、または`#[bindable]`ownerに対する`owner.property`という形の参照
- **見つからなければ Once**(初期化時に一度だけ評価): リテラル・`#[param]`フィールドの参照のみで構成される式

この判定は式を実際に構文木として走査して行う——`format!(..)`/`format_args!(..)`/`vec!(..)`の引数の中は再帰的に見るが、それ以外のマクロ呼び出しは中身を検査できないため、依存の有無に関わらず**安全側に倒してOneWay扱い**にする(§3の`#[computed(expr = ...)]`の依存検出にも同じ不透明マクロの制約がある)。

### `once!(expr)`:明示的なOnce

`property: once!(expr)`は上記の自動分類そのものを行わず、依存収集を抑止して初期化時のスナップショットとして一度だけ評価する。`expr`の中にリアクティブな参照があっても無視される——「初期表示時の値を固定表示したいが、式の中身自体はリアクティブなフィールドを使って書きたい」場合に使う。

### `property <=> target`:明示的なTwoWay

`<=>`の右辺(`target`)に書けるのは次のいずれかに限る:

- 同一componentの`#[prop]`/`#[state]`フィールド(`<=>`は`view`を持つcomponentの中でしか書けないため、§4のとおりこれらは常に書き込み可能)
- `#[bindable]` ownerに対する`owner.property`(直接参照。式は不可)
- 安定した`for` itemの`item.property`——ただしitemの列が明示的な`Vec<Rc<T>>`または`#[observable] Vec<T>`として生成されるviewmodel collectionである場合に限る(§5の`for`要素再利用の仕組みが安定したidを提供できることが前提)。この場合`property`は対象の`#[prop]`または`#[observable]`でなければならない

`<=>`の widget→model 方向(ユーザー操作で値が変わったときの書き戻し)は、対応するsetterを呼ぶだけでよい——別途component全体の再同期を呼んではならない(setter自体が必要な範囲の再同期を担うため、二重に呼ぶと無駄な更新が発生する)。

### 変更の伝搬:2つの仕組み

OneWayの再評価・TwoWayの model→widget 方向の反映は、参照先がどこにあるかによって2つの異なる仕組みで実現される。いずれも**componentの`view`全体を再構築せず、依存する属性・動的領域だけを更新する**という結果は同じだが、実現方法が違う:

1. **同一component自身の`#[prop]`/`#[state]`/`#[computed]`フィールドの変更**: これらのフィールドを変更するsetterは、コンパイル時に静的決定された、そのcomponent専用の型付き通知(`{Component}Property`——viewmodelの`PropertyChanged`と同型の設計)を直接発火し、依存する`view`の該当箇所・依存する`#[computed]`だけを再計算する。ランタイムの購読オブジェクトを経由しない——`elwindui-codegen`が`view`の式を静的解析して依存関係を洗い出し済みだからこそ、setterから直接呼び出せる
2. **`#[bindable]`で保持しているviewmodelの変更**: viewmodel側は別の`#[elwindui::viewmodel]`マクロ展開で生成されるため、component側はコンパイル時にその内部構造を知らない。そのため`#[observable]`のsetterは代入後に型付き`PropertyChanged`を発火し、component側は`Subscription`によるランタイム購読でそれを受け取り、該当する属性・動的領域だけを更新する。表示領域が破棄されると`Subscription`はDropにより解除される。例えば`TextArea { text <=> doc.content }`の入力は、その`TextArea`と`doc.content`に依存する表示だけを更新し、親の`TabView`の`children`コレクションを再同期しない

`for` itemのTwoWayコールバックも(2)と同じ`DynamicChild`の寿命に属し、itemが置換・削除されると購読とともに解放される。`for`/`if`/`match`の構造変更(§5参照)は親view全体ではなく対応する動的領域だけを差し替える。依存プロパティを静的に特定できない任意のRust式はビルド時エラーとなり、必要な計算は`#[computed]`または解析可能なprop参照へ分解する。

---

## 10. 多言語対応(i18n)

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

- ビルド時に `.ftl` を静的パースし、DSL内で参照している `t!("key", ...)` のメッセージIDが**全`available`言語で定義されているか**を機械的に検証する(未翻訳キーの検出)
- `t!` に渡す引数名は、対応する `.ftl` メッセージ内の `{ $引数名 }` と一致していなければ静的エラーとする

---

## 11. モジュール(import)

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
- ViewModelの参照([`../design/runtime/state_management_design.md`](../design/runtime/state_management_design.md))も同じ規則に従う。参照側は必ずその定義が
  実際にコンパイルされる実Rustパス(`#[elwindui::viewmodel] mod foo { .. }`が実際に宣言されている
  パス、例: `crate::foo::Foo`)を`use`する
- enumの参照(§7)も同じ規則に従う——通常のRustの`use`で導入した名前をそのまま使える

---

## 12. 要素ツリーの探索と要素への注釈

### 役割分担の方針

「子要素を持つ」という性質は既存の `{}` ネスト構文がそのまま表現するため、children専用の別DSL構文は追加しない。ツリー探索の内部方式は [`../design/runtime/ui_tree_design.md`](../design/runtime/ui_tree_design.md) を参照する。

| 責務 | 担当 |
|---|---|
| 親子構造の宣言 | DSL構文(`{}` ネスト。追加構文は不要) |
| 動的生成された子要素(`if`/`for`/`match`の結果)をchildrenとして集約する規約 | コード生成器 |
| 全要素が親子を辿れるという契約(`visual_children()`/`parent()`) | `UIElementExt`(`docs/design/runtime/ui_tree_design.md`、コード生成器が全要素型に自動実装) |
| 再帰探索アルゴリズム(`visual_tree::find_all` 等) | 共通ランタイムライブラリ(DSLとは独立に拡張・最適化可能) |
| 特定要素への後からのアクセス | `#[id(...)]` アトリビュート |

### 共通トレイト(コード生成器が自動実装)

`children()`/`id()`だけのための別トレイトは無い。全要素型が既に実装している`UIElementExt`(`docs/design/runtime/ui_tree_design.md`)がその役割を兼ねる:

```rust
trait UIElementExt: AsAny {
    fn visual_children(&self) -> Vec<Rc<dyn UIElementExt>>;
    fn parent(&self) -> Option<Rc<dyn UIElementExt>>;
    // ... 他多数(margin/alignment/measure/arrangeなど。内部方式は`docs/design/runtime/ui_tree_design.md`参照)
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

`elwindui_core::visual_tree`は、WinUI3の`VisualTreeHelper`に相当する自由関数群を提供する。`UIElementExt`自体が既に`visual_children()`/`parent()`を持つため、木の走査そのものはこのモジュールを経由しなくても行えるが、`visual_tree`は(a) WinUI3に近い呼び出し形(`visual_tree::get_child(elem, i)`)と、(b) `UIElementExt`単体には無い型ベースの再帰収集(`find_all`)をまとめて提供する。

```rust
pub fn get_children_count(element: &dyn UIElementExt) -> usize;
pub fn get_child(element: &dyn UIElementExt, index: usize) -> Option<Rc<dyn UIElementExt>>;
pub fn get_parent(element: &dyn UIElementExt) -> Option<Rc<dyn UIElementExt>>; // UIElementExt::parentのラップ

// 型による再帰探索(該当する型の要素をすべて収集。WinUI3のVisualTreeHelperには無い拡張)
pub fn find_all<T: 'static>(root: &dyn UIElementExt) -> Vec<Rc<dyn UIElementExt>> {
    // visual_children() を再帰的に辿り、as_any().downcast_ref::<T>()が成功するものを収集する
    ...
}
```

- idによる文字列検索(`find_by_id`相当)は無い。理由は次項参照 — ランタイムidを保持する要素が存在しない
- 探索方式(深さ優先/幅優先)やキャッシュ戦略の変更は、**DSLの構文を一切変えずに**ライブラリ側の実装更新だけで完結する
- DSL側が保証するのは「`UIElementExt` トレイトを介してツリー全体に到達可能である」という契約のみ

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
- **`#[id(...)]`は全てコンパイル時に確定している**ため、実行時に文字列で検索する仕組みは経由しない — 具象型を直接返す静的アクセサの方が`docs/design/runtime/state_management_design.md`の「型消去を避け専用コードを生成する」方針に沿っており、ダウンキャストも不要になる
- **ランタイム文字列idによる検索は意図的に提供しない**。名前付きアクセスは`#[id(...)]`に統一し、型付きfieldとして静的に解決する

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

配送は発生元からancestorへ順に進み、`RoutedEventArgs.handled`が`true`になった時点で停止する。
`for`などで動的に構築された要素も、同じlogical parent chainに従って配送される。
`on_key_down`、`on_key_up`、`on_text_input`はbubbleし、`on_got_focus`、`on_lost_focus`は
対象要素へ直接配送する。treeとinputの内部構造は
[`ui_tree_design.md`](../design/runtime/ui_tree_design.md)と
[`input_focus_design.md`](../design/runtime/input_focus_design.md)を参照する。

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

`#[shortcut(...)]`が付けられるのは`#[routed]`なフィールド(`on_click`/`on_key_down`等)のみ。
`winui3: "..."`と`appkit: "..."`でbackend別の表記を指定でき、`scope: local`は登録範囲を
対象subtreeへ限定する。共通表記の`Ctrl`はmacOSでは`Cmd`へ変換する。登録と配送の内部構造は
[`input_focus_design.md`](../design/runtime/input_focus_design.md)を参照する。

---

## 13. 静的検証ルール一覧

コンパイラ/リンタが実行前に検出すべき項目:

1. `#[param]` フィールドの初期化式にリアクティブなprop/state/computed/bindable owner参照が出現 → エラー
2. `#[param]` フィールドの初期化式に非純粋関数(`now()`, `random()` 等)が出現 → エラー(`env::*` は例外)
3. `#[computed]` フィールドへの外部代入 → エラー
4. enum値の裸文字列直書き(完全修飾でない参照) → エラー
5. `match` におけるenumメンバーの網羅漏れ(`_ =>` なし) → エラー
6. 制約(`#[range]`, `#[length]`, `#[pattern]` 等)付きフィールドへのリテラル値代入が制約違反 → ビルド時エラー、動的値の場合は実行時エラー
7. (欠番 — `once` 宣言の廃止に伴い、`external::*` を許可する場所自体が無くなったため)
8. importの循環・未解決パス → エラー
9. (欠番 — `native!` / `target::backend()` 構文の廃止に伴い不要)
10. `view`内に`Canvas`が含まれているが `#[accessible(...)]` が付与されていない → 警告(`docs/design/runtime/input_focus_design.md`参照)
11. `on_mount`/`on_update`/`on_unmount`を含むあらゆる実行contextで`#[param]`フィールドの再代入相当の操作が行われている → エラー([`ui_tree_design.md`](../design/runtime/ui_tree_design.md)参照。paramの不変性は生涯を通じて保証される)
12. リアクティブ属性式または`<=>`の参照先が`store`宣言(`docs/design/runtime/state_management_design.md`)の型・フィールドとして存在しない → エラー
13. `store`/`viewmodel`フィールドへの`#[param]`側からの直接参照 → エラー(`docs/design/runtime/state_management_design.md`、`docs/agents/codegen.md`参照。store/viewmodelはViewのリアクティブ属性式または明示的な`<=>`から参照する)
14. `NavigationHost`内の`match route { ... }` がRoute enumの全メンバーを網羅していない(`_ =>`なし) → エラー(7章の網羅性検査と同じ仕組み、`docs/specs/ui_spec.md`参照)
15. (欠番 — `native!` / `target::backend()` 構文の廃止に伴い不要)
16. `Transition`/`KeyframeAnimation`(`docs/specs/ui_spec.md`参照)で存在しないイージング関数名、または範囲外のキーフレーム位置(`0.0..=1.0`外)が指定されている → エラー
17. `Effect`(`docs/specs/ui_spec.md`参照)のパラメータが対応バックエンドでサポートされない組み合わせ(例:GTK4未対応のエフェクト種別)である場合 → 警告(該当バックエンドではフォールバック描画に切り替わる旨を明示)
18. (欠番 — アクションはRustの`impl`ブロックの`fn`として自動検出されるため、対応する型検査が存在しない)
19. `viewmodel`定義内に`view`ブロック、またはビルトイン要素(`Row`/`Text`等)への直接参照が存在する → エラー(`docs/design/runtime/state_management_design.md`参照。ViewModelは表示ロジックを持たず、MVVMのV/VM分離を静的に強制する)
20. `#[async_computed]` が `viewmodel`/`store` 以外(通常の`component`のprop等)に付与されている → エラー(`docs/design/runtime/state_management_design.md`参照。非同期状態はVM/Model層に閉じ込める)
21. (欠番 — ElwindUIは宣言的なundo/redo(`#[undoable]`)を提供しないため。SwiftUIの`UndoManager`同様、必要であればアプリ側がhostレベルの仕組みに自分で配線する)
22. (欠番 — Themeがtoken/variantモデルからEnvironment上のPresetモデルへ再定義されたことに伴い(#96)、`tokens{}`/`variant`ブロック自体が存在しなくなったため不要)
23. `VirtualList`に`key`が指定されていない状態で`items`の順序が変わる更新が行われる → 警告(`docs/specs/ui_spec.md`参照。挿入位置ベースの再利用にフォールバックし、リコンサイル効率が低下する可能性がある)。一般の `for` は `Vec<Rc<T>>` のとき各要素の `Rc<T>` ポインタ同一性で子を再利用し、その他の collection は当該範囲を再構築する(`docs/specs/ui_spec.md`参照)。`TabView` は `TabViewItem` を子として指定する。
24. `on_foreground`/`on_background`/`on_terminate`(`docs/design/runtime/ui_tree_design.md`)が、アプリのエントリポイント(ルート)コンポーネント以外で宣言されている → 警告(OSレベルのライフサイクルは単一箇所への集約を推奨)
25. コールバック型のフィールドで `Rc<dyn Fn(...)>` / `Box<dyn Fn(...)>` のような型消去表現を直接使用している(`fn(...)` 糖衣構文を使っていない) → エラー(4章「コールバック型フィールド」参照)
26. `#[control_template(target = T)]`の`T`が`ControlExt`を実装しない(`NativeControl`を含む) → 生成Rustのtrait bound error
27. `#[component(template = key)]`のKeyが未宣言、または`EnvironmentKey::Value`が`Option<ControlTemplate<Component>>`と一致しない → エラー
28. template-enabled default bodyまたは`#[control_template]` bodyが欠落・重複する、あるいは`#[id(...)]`を含む → エラー
29. replaceable templateが複数の`ContentPresenter`を含む、またはdynamic region内に`ContentPresenter`を含む → エラー
30. `#[shortcut(...)]` が `#[routed]` でない属性に付与されている → エラー(12章「`#[shortcut(...)]`」参照。`on_click`等のコールバック属性以外に付けても意味を持たない)
31. `#[shortcut(...)]` に指定されたキー表記(修飾キー名/キー名)が不正 → エラー(`docs/design/runtime/input_focus_design.md`参照。`codegen::parse_shortcut_spec`と同じパーサーで検査するため、ここを通れば必ずコード生成もパースに成功する)
32. `elwindui::core::graphics::Brush`/`Color`(または`Option<..>`)型のフィールドへ文字列リテラルを代入する場合(例: `Rectangle { fill: "#3a3a3c" }`)、その文字列が`"#rrggbb"`/`"#rrggbbaa"`(`#`省略可)のいずれの形式にも一致しない → コード生成時エラー(`codegen::coerce_color_literal`。動的な`String`式には適用されない——`Brush`/`Color`型の値を直接渡す必要がある)。`foreground`/`background`/`fill`/`stroke`は`BrushStyle`も受け付け、effective Environmentから解決した後に同じsetter/clear contractへ接続する。
33. `#[environment(...)]` が同一フィールドの `#[param]`/`#[prop]`/`#[state]`/`#[bindable]` と併用されている → エラー(4章「`#[environment(name)]`」参照)
34. `#[environment(name)]` の `name` が、解決可能な `#[elwindui::environment_key]` 定義または組み込みEnvironment Key名(Semantic Style Key、`theme_environment_spec.md`§7、または`popup_dismiss`、同spec§2)を持たない → エラー。bare識別子(同一crate内解決または組み込みKey fallback)の場合はコード生成時の`compile_error!`。完全修飾クレートパス(Issue #129、クレート境界を越えた解決)の場合は、生成コードが実際にコンパイルされた時点の`rustc`自身の「マクロが見つからない」エラー——proc-macro展開からは他クレートが何をエクスポートしているか分からないため、`elwindui-codegen`側の早期検査は原理的に行えない(`docs/design/tools/environment_key_macro_design.md`参照)。
35. `EnvironmentScope { key: value .. }` の `key` が、解決可能な `#[elwindui::environment_key]` 定義または**書き込み可能な**組み込みEnvironment Key名(Semantic Style Key、`theme_environment_spec.md`§7——ルール34の読み取り専用集合とは別で、`popup_dismiss`を含まない。4章「`#[environment(name)]`」の書き込み側解決規則を参照)を持たない、または `value` の型がそのKeyの `Value` 型と一致しない → エラー(5章「`EnvironmentScope`」参照)。名前解決エラーの検出方式(コード生成時`compile_error!` vs 実コンパイル時`rustc`エラー)はルール34と同じ、bare/完全修飾の別で決まる。`value`の型不一致はどちらの形式でも常に通常の`rustc`型エラー。`popup_dismiss`を`EnvironmentScope`で上書きしようとした場合は、書き込み可能な集合に含まれないため、bare識別子の「未解決」と同じコード生成時`compile_error!`となる。
36. `#[elwindui::theme] struct Name { #[theme(value = ..)] field: Type, .. }` の `field` の識別子が、解決可能な `#[elwindui::environment_key]` 定義または**書き込み可能な**組み込みsemantic Key名を持たない → コード生成時エラー(`docs/specs/theme_environment_spec.md`§2/§3/§4/§7参照。ルール35の`EnvironmentScope`と同じ書き込み側解決規則——`#[environment(name)]`の読み取り側解決規則とは異なり、`popup_dismiss`は含まない)
37. `view! { .. }`(deferred view、本章「`view! { .. }`を属性値とする糖衣構文」参照)が代入されるフィールドが`ViewTemplate`/`Option<ViewTemplate>`型でない、または`TwoWay`/`once!(..)`で代入されている → エラー(deferred viewはOneWayの一方向構築のみ許可する。書き戻し先が存在せず、`once!`のような構築時一回評価とも意味的に異なる——実際の評価は宣言時ではなく用途が定める時点まで遅延される)
38. `mount_override`/`unmount_override`という名前の`fn`をユーザーが`#[overrides]`(またはそれ以外の形)で定義している → エラー(`docs/design/runtime/component_lifecycle_design.md`§4i参照。この2つはWindowのバックエンド実装がフレームワーク内部のライフサイクルフックへ接続するために予約された名前であり、DSL作者向けのライフサイクルフックではない——同じ目的には`on_mount`/`on_unmount`を使う)

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

# 付録A. component宣言レベルの属性

`component`宣言の直前に、`inherits`の有無に関わらず0個以上任意の順序で書ける4つの属性(`enum`/`viewmodel`/`view`には付けられない)。ビルトイン25個の宣言は`elwindui-core::ui`/各`elwindui-backend-*`crateの`#[elwindui_macros::class]`宣言そのものであり、これらの属性はそちらのRust宣言に直接付いている。

- **`#[sealed]`** — このコンポーネントを`component X inherits Y`の`Y`(継承元)として指定できないようにする。具象的な末端形状(`Rectangle`/`Ellipse` — 継承したければ合成可能な`Shape`を使う)や、そもそも継承先を持たないネイティブ末端要素(`Button`/`TextArea`/`TabView`/`TabViewItem`)に付与する。
- **`#[abstract]`(Rust形式:`#[abstract_]`)** — componentを`view`内で直接instantiateできなくする。`inherits`先には指定できる。唯一の例外として、descendant自身が名指ししたabstract baseを、そのdescendantの`view`ルートとして構築する形を許可する。abstract componentには公開constructorを生成しない。
- **`#[text_style]`**([`text_style_spec.md`](text_style_spec.md)) — 共通text-style propertyをcomponentへ追加する。同名fieldを自前で宣言したcomponentへの付与は静的エラーとなる。共有基底で宣言したpropertyはdescendantから利用できる。
- **`#[content(field_name)]`** — WinUI3の`ContentPropertyAttribute`相当。ある要素の`view`本体に「属性名を書かない裸のネスト子要素」(`Type { .. }`を`name: value`形式でなく直接`{}`内に書く)を渡した際、それがどのフィールドに束縛されるかを明示する。実効 metadata と field 型により lowering が決まり、scalar field はちょうど一つの child を `set_<field>(child)` へ、collection field は宣言された collection surface へ順序を保って渡す。例:`MenuBarItem`は`#[content(submenu)]`を宣言しており、`MenuBarItem { text: "File", Menu { .. } }`の`Menu { .. }`は`submenu`フィールドに束縛される(`Window`/`ContentControl`/`TabViewItem`の`content`フィールドも同様に`#[content(content)]`を宣言している)。`Control`は内部 scalar `#[content(visual_root)]` destinationを持つが、これは公開 collectionではなくprivate template-root ownershipへ委譲するためのDSL/codegen surfaceである。派生component自身の`#[content(...)]`は継承した destination を置き換えるため、将来の typed `CustomTabView::children` は`Control::visual_root`と衝突しない。lowering は型名`Control`を特別扱いせず、実効metadataとfield型だけから決まる。`field_name`は実在するフィールド名でなければならず(静的検証)、componentにつき最大1個。裸のネスト子要素があるのに実効`#[content(..)]`(または専用の children/items surface)が無いcomponentにそれを渡すのは静的診断になる。この属性はビルトイン限定ではなく、ユーザー定義コンポーネントでも使える。

`#[sealed]`/`#[abstract_]`はユーザー定義componentでも利用できる。`#[text_style]`はtext-style owner contractを実装するcomponentで利用する。

---

> 標準UI要素は [`ui_spec.md`](ui_spec.md)、text styleは [`text_style_spec.md`](text_style_spec.md)、graphics valueは [`graphics_spec.md`](graphics_spec.md)、OS serviceは [`platform_spec.md`](platform_spec.md) を参照。内部実装は [`../design/README.md`](../design/README.md) から対象designを選ぶ。
