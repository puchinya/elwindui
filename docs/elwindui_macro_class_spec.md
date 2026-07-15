# `#[elwindui_macros::class]` マクロ仕様書

`crates/elwindui-macros/src/class.rs` が実装する属性マクロ `#[elwindui_macros::class]`
(`elwindui`ファサード経由では `#[elwindui::class]`)の完全な仕様。docs/elwindui_spec.md
付録H.2.1a が定めるRustクラス階層表現規約(trait+構造体合成によるRust上の疑似継承)を、
手書きコード(`elwindui-core`/各バックエンドクレート)とコード生成(`elwindui-codegen`)の
両方で実際に自動化する実装がこのマクロである。

> 付録H.2.1aの図・コード例は命名規約の一部が旧版のまま古くなっている。本書が現在の実装
> (構造体は素の`ClassName`、トレイトは`ClassNameExt`、祖先アクセサは`as_ui_element`/
> `__dyn_x`)の正になる。実装を疑うときは本書よりも常に`crates/elwindui-macros/src/class.rs`
> 自体を優先して確認すること。

---

## 1. 命名規則

- **構造体名**は常にソースに書いたとおりの素の識別子。`struct ClassName { .. }` は常に
  `ClassName`のままコンパイルされる——接尾辞の付加/削除は一切行わない。
- クラス自身の**トレイト名**(存在する場合)は常に `{ClassName}Ext`。Rustは同一モジュール内で
  構造体とトレイトが同じ裸名を共有できない(型名前空間が同じ)ため、接尾辞はトレイト側に付く。

---

## 2. 適用形態

このマクロは対象アイテムの形によって3通りの展開ロジックに分岐する。

### 2.1 `struct`/`impl`ペア(通常クラス・ルートクラス)

もっとも一般的な形。`struct ClassName { .. }`(`base`フィールドは書かない)と、別属性呼び出しの
`impl ClassName { .. }`(`for`無し)の**2回の独立した属性適用**として書く。

```rust
#[elwindui_macros::class(inherits = crate::ui::SuperClass)]
pub struct ClassName {
    // ClassName自身が宣言するフィールドのみ(baseは自動挿入)
}

#[elwindui_macros::class]
impl ClassName {
    // メソッド定義
}
```

`struct`側の属性が展開時に`#[class(..)]`引数を(クレート内プロセスグローバルな)ストアへ
保存し(`store_class_args`)、続く`impl`側の**引数なし** `#[elwindui_macros::class]` がそれを
読み出す(`load_class_args`)。したがって**`struct`が`impl`より前にソース上で宣言されている
ことが必須**。`impl`側に明示的に引数を書いた場合はそちらが常に優先される(ストアは無視される)。

`inherits = ..` を省略すると**ルートクラスモード**になる(このクラス階層で唯一祖先を持たない
クラス、実装では`UIElement`のみ)。詳細は§9。

**重要**: `inherits = ..`/`struct_only = ..`に渡す型は常に**クレートルート起点の完全修飾パス**
で書くこと(`crate::ui::SuperClass`/`elwindui_core::ui::SuperClass`など、裸名や`use`エイリアス
経由は不可)。理由と検証は§7。

### 2.2 `trait_only`(純粋インターフェース宣言)

このクレート内に対応する構造体を一切持たない、マーカー/インターフェーストレイトの宣言。
各具象実装は別クレート(バックエンド)が`struct_only`で個別に提供する(例:
`elwindui_core::ui::MenuItemExt`は各バックエンドの`MenuItem`構造体が実装する)。

```rust
#[elwindui_macros::class(trait_only, inherits = crate::ui::SuperClass)]
pub trait ClassName {
    fn some_method(&self, x: i32) -> bool;
}
```

`struct`/`impl`のペアは不要——**1回で完結する単独の属性適用**。マクロは`ClassName`を
`ClassNameExt`へリネームし、供給されたメソッドシグネチャ群を全て**デフォルトメソッド**へ
変換する(§5)。`trait_only`は**自分自身の`__elwindui_inherit_*!`トリオを生成しない**
(§8.6)——祖先チェーンへの接続はトレイト宣言自体の supertrait 境界だけで完結する。

### 2.3 `struct_only`(既存トレイトの具象実装)

「このクラスは新しい`{ClassName}Ext`トレイトを持たず、既存のトレイトをこの構造体が直接
実装する」ことを宣言する。バックエンドの実ネイティブ構造体(`elwindui-backend-appkit`の
`TextArea`/`Button`/`MenuItem`等)が典型例。

```rust
#[elwindui_macros::class(struct_only = elwindui_core::ui::TextAreaExt, inherits = crate::NativeControl)]
pub struct TextArea {
    handle: AnyView,
}

#[elwindui_macros::class]
impl TextArea {
    fn set_text(&self, text: &str) { .. }   // TextAreaExtの実メソッド、実体を持つ
}
```

`struct_only`クラスには「自分自身のトレイト」が無いため、他クラスがこれを`inherits = ..`先に
指定するとき、そのトレイトパスは`struct_only`の指定パスそのものになる(§6の`ancestor_own_trait`
がこれを解決する)。

---

## 3. `#[class(...)]` 引数一覧

| 引数 | 意味 |
|---|---|
| `inherits = Type` | 直近の祖先の**構造体**型(完全修飾パス必須)。`base: Type`フィールドが自動挿入される。省略するとルートクラスモード(`struct_only`と併用時は「祖先なしの独立クラス」、例: `Window`)。 |
| `struct_only = Path` | 既存のトレイトパスを直接実装(§2.3、完全修飾パス必須)。新規`{ClassName}Ext`は生成されない。 |
| `trait_only` | `trait ClassName { .. }`への単独適用(§2.2)。 |
| `abstract_class` | このクラス自身は直接インスタンス化しない(`new`を自動生成しない、`new`を手書きするとコンパイルエラー)。サブクラスが`Self::construct()`を呼ぶための土台としてのみ存在する(例: `Layout`)。 |
| `sealed` | 継承禁止クラス。他クラスが`inherits = ..`でこれを指定すると、生成された`__elwindui_check_not_sealed_{Name}!`マクロが存在しないため`E0433`(マクロが見つからない)で失敗する(§8.5)。 |
| `no_ancestor_forward` | `struct_only`と併用時のみ意味を持つ。対象トレイトが`__dyn_x`規約に従っていない手書きトレイト(実メソッドを持つ、`#[class]`が関知しないもの——例: `NativeTabView`の`struct_only = crate::TabView`)であることを示す。このクラス**自身の**トレイトの`impl`生成のみをスキップし、それより先の祖先への到達性には影響しない(§8.4)。 |

---

## 4. メソッドタグ(`impl ClassName { .. }`内の各メソッドへの属性)

`impl ClassName { .. }`内の各`fn`は、レシーバの有無と付与されたタグによって最終的に
異なる`impl`ブロックへ振り分けられる。**`#[ancestor]`のような「どの祖先向けか」を明示する
タグは存在しない**——ルーティングは全てメソッド名をキーにしたマクロ側のマッチングで解決する
(§8.3)。

| タグ | 効果 |
|---|---|
| (無し、`&self`あり) | `ClassNameExt`(このクラス自身のトレイト)の一部として実装される。 |
| (無し、レシーバ無し) | コンストラクタ(`new`/`construct`)として`impl ClassName { .. }`(プレーン、コンストラクタ用ブロック)に振り分けられる。 |
| `#[inherent]` | トレイト振り分けから完全に除外し、プレーンな`impl ClassName { .. }`(コンストラクタと同じブロック)へ入れる。トレイトに属さないヘルパー(`into_any_view`/`set_on_text_change`のようなバックエンド固有の便宜メソッド)に使う。 |
| `#[overridable]` | このクラス自身の(タグ無しの)メソッドに付与し、「将来の子孫がこのメソッドを`#[overrides]`で上書きしてよい」と宣言する(§8)。 |
| `#[overrides]` | 子孫側で、**いずれかの祖先**(hopの深さを問わない)が`#[overridable]`宣言したメソッドを上書きしていることを明示する(§8)。 |

---

## 5. `__dyn_x`アクセサ方式(祖先メソッドの自動継承)

`{ClassName}Ext`は自分自身の**必須(デフォルト無し)メソッドを1つだけ**持つ:

```rust
fn __dyn_x(&self) -> &dyn ClassNameExt;
```

(`__dyn_x`は`dyn_accessor_ident`が`ClassName`のスネークケースから
`__dyn_control`/`__dyn_content_control`のように機械的に導出する。)

クラス自身が宣言した他の全メソッド(`own_methods`のうち`#[inherent]`でないもの)は、この
`__dyn_x`アクセサ経由で呼び出す**デフォルトメソッド**としてトレイト宣言自体に埋め込まれる:

```rust
fn padding(&self) -> Option<f32> {
    ContentControlExt::padding(self.__dyn_content_control())
}
```

ドット呼び出し(`self.__dyn_x().method()`)ではなく完全修飾呼び出し(`TraitName::method(recv,
..)`)を使うのは重要な点——サブクラスが祖先と同名の別概念のメソッドを宣言する場合(例:
`ContentControl::padding(&self) -> Option<f32>`と`Control::padding(&self) -> f32`)、
`ContentControlExt: ControlExt`という継承関係の下でドット呼び出しは名前が曖昧(E0034、戻り値型
では曖昧性解消されない)になるため。

**宣言クラス自身**(`ClassName`本体)は、この`{ClassName}Ext`をトリビアルに実装する——
`__dyn_x`は`self`を返す**反射的**な実装、他のメソッドは元々ユーザーが書いた**実**ボディで
明示的に上書きする(デフォルトへは決して委譲しない——委譲すると`__dyn_x`が`self`を返す以上、
無限再帰になる)。

**dyn互換性の要件**: `{ClassName}Ext`の全メソッドはオブジェクトセーフ(ジェネリックメソッド
不可、`Self`値渡し不可等)である必要がある。

---

## 6. 祖先チェーンへの参加(`impl`側の`prelude`)

`#[class(inherits = Parent)]`を持つクラス`X`の`impl`展開は、hop数やクレート境界に関係なく
常に以下を行う(`expand_impl`の`prelude`構築):

1. `Parent`の`__elwindui_check_not_sealed_Parent!()`を呼び、`Parent`が`#[sealed]`でないことを
   検証する。
2. `Parent`自身の`__elwindui_inherit_Parent!`(または、`X`が`struct_only`で`Parent`と全く同じ
   トレイトを実装している「ラッパーのラッパー」の場合は`__elwindui_inherit_Parent_skip!`)を、
   `X`自身の`#[overrides]`メソッド群を渡して呼び出す。

この呼び出しが、`X: ParentExt`(または`Parent`が`struct_only`ならその指定トレイト)の`impl`を
実際に生成する——`X`自身の展開コードが祖先の実装詳細を一切知らなくてよい。

`Parent`の`own_trait`(このクラスを`inherits = ..`先に指定する誰かが使うべきトレイト)は
`ancestor_own_trait`が解決する: 同一クレート内の`struct_only`登録があればその実パスを、
無ければ`{Parent}Ext`という命名規約のフォールバックを使う(`NativeTabView`の
`struct_only = crate::TabView`のような、命名規約に従わない手書きトレイトのケースをカバー
するため)。

---

## 7. 完全修飾パスの要件と検証

`inherits = ..`/`struct_only = ..`に渡す型パスは、指す先が同一クレートでも常に
**クレートルート起点の完全パス**で書く必要がある(例: `inherits = UIElement`ではなく
`inherits = crate::ui::UIElement`)。理由:

- このパスのトークン列は、このクラスが生成する`__elwindui_inherit_*!`マクロ本体へ
  **リテラルのまま埋め込まれる**(`$OwnTrait`/`$OwnConcrete`として)。このマクロは
  **別のモジュール・別のクレートから展開される可能性がある**——`macro_rules!`内の型パス解決は
  マクロ名の解決と同様、**呼び出し元のスコープ**で行われる(定義元のスコープではない)ため、
  裸名や`use`エイリアス経由のパスはそこで解決できない。

`#[class]`自身がこれを検証する(`validate_fully_qualified_path`、`expand`/`expand_trait_only`
から呼ばれる):

1. パスのセグメント数が1(完全な裸名)なら即座に`compile_error!`。
2. セグメント数が2以上でも、末尾の裸名が同一クレート内の既知クラス(`same_crate_classes`に
   登録済み)であるにもかかわらず先頭セグメントが文字通り`crate`でない場合
   (例: `use crate as appkit;`によるローカルエイリアス`appkit::TextArea`)も`compile_error!`。

---

## 8. `__elwindui_inherit_*!`マクロトリオ——祖先チェーンの実体

hop-0(直近祖先)であろうとhop-N(祖先のそのまた祖先)であろうと、クレート境界を何回跨いで
いようと、祖先チェーンは**常に同一の仕組み**で解決される——hopの深さやクレート境界に応じた
特別扱いは無い。

### 8.1 3つのマクロ

`inherits`を持つ(または`struct_only`のみで祖先なしの)クラスは、`sealed`でない限り
`build_inherit_macros`が以下3つの`#[macro_export] macro_rules!`を生成する
(`bare_name` = クラスの裸名):

- **`__elwindui_inherit_{bare_name}!`**(entry): `$SubType:ty, $OwnTrait:path,
  $OwnConcrete:path; $($overrides:tt)*` を受け取り、蓄積用の空リスト2つを添えて
  `classify`へ渡す。子孫クラスが直接呼び出す入口。
- **`__elwindui_inherit_{bare_name}_skip!`**: `impl $OwnTrait for $SubType`を生成せずに
  そのまま次の祖先へ委譲するだけの入口。「ラッパーのラッパー」(`struct_only`の子孫が
  祖先と全く同じトレイトを実装している場合、`struct_only_collides_with`が検出)が使う。
- **`__elwindui_inherit_{bare_name}_classify!`**: `#[overrides]`メソッド群を
  「自分(`bare_name`)が`#[overridable]`宣言した名前と一致する」か「一致しない(祖先の
  さらに祖先向け)」かに1つずつ振り分ける、アキュムレータ式のtt-muncher。

### 8.2 呼び出し規約

- **直接呼び出し**(`expand_impl`の`prelude`など、アイテム位置に生成される通常のRustコード)
  では `inherit_macro_path` が経路を組み立てる: 対象が同一クレート内なら裸のマクロ名
  (`#ident`)、そうでなければ`inherit_macro_prefix`が導く完全パス。
- **マクロ本体内からの自己参照**(`classify`が自分自身を再帰呼び出しする等)では
  `inherit_macro_self_ref_path`を使う: 同一クレート内なら`$crate::#ident`
  (`macro_rules!`内で裸名は**呼び出し元**のスコープで解決されるため、`$crate::`修飾が必須)、
  そうでなければ`inherit_macro_prefix`が導く完全パス(この場合は元々クレートを跨ぐので
  `$crate`は使わない)。

### 8.3 `#[overridable]`/`#[overrides]`によるオーバーライドのルーティング

祖先クラスの`impl`内、タグ無しのメソッドに`#[overridable]`を付与する:

```rust
#[elwindui_macros::class]
impl OverridableBase {
    #[overridable]
    fn compute(&self, x: i32) -> i32 { x + self.value.get() }
}
```

子孫クラスの`impl`内では、`#[ancestor]`のような追加タグ無しで`#[overrides]`だけを付与する:

```rust
#[elwindui_macros::class]
impl OverridableDerived {
    #[overrides]
    fn compute(&self, x: i32) -> i32 { x * 10 }
}
```

子孫クラスの`prelude`は、この`compute`メソッドを`name => { fn compute(..) { .. } },`という
キー付きグループとして直近の祖先の`entry`マクロへ渡す。祖先の`classify`マクロは、自分が
`#[overridable]`宣言した名前の集合とリテラル一致するものだけを`impl $OwnTrait for $SubType`
へスプライスし、一致しないものはそのまま次の祖先の`classify`へ転送する——**どのクラスの
メソッドかを子孫側が明示する必要は一切ない**。メソッド名自体が常にキーになる。

存在しない/`#[overridable]`でないメソッドを`#[overrides]`した場合は、チェーンの末端
(§8.6の`terminal`チェック)まで転送されて`compile_error!`で失敗する。シグネチャの不一致は
生成された`impl`自体が通常のトレイト実装検査で検出する(E0050/E0053)。

### 8.4 `no_ancestor_forward`(`skip_own_impl`)

`struct_only`の対象が`__dyn_x`規約に従わない手書きトレイト(実メソッドを持つ、例:
`NativeTabView`の`struct_only = crate::TabView`)である場合、`no_ancestor_forward`を立てる。
効果は**このクラス自身のトリオが生成する`impl $OwnTrait for $SubType`ブロックだけ**を
省略すること(`fn #dyn_ident`を実装しようがない——対象トレイトにそのメソッドが無いため
`E0407`になる)。named accessorの生成、およびこのクラスがさらに祖先を持つ場合の遡上は
**通常通り継続する**——`no_ancestor_forward`なクラスの、そのまた祖先への到達性は一切
損なわれない。

### 8.5 `#[sealed]`

`sealed`でない限り、クラスは`__elwindui_check_not_sealed_{bare_name}!`という1つの
`macro_rules!`も追加生成する(`() => {};`のみの空no-op)。`sealed`なクラスはこの安全弁
マクロ自体を生成しない——`inherits = ..`でこれを指定しようとした子孫の`prelude`が
`#sealed_path!()`を呼ぼうとして`E0433`(マクロが見つからない)で失敗する。

### 8.6 チェーンの終端(祖先を持たないクラス)

祖先を持たないクラス(`inherits`を省略した`struct_only`クラス、または真のルートクラス
`UIElement`)は、`__elwindui_inherit_*!`トリオの生成時、共有の外部マクロへ委譲するのではなく
**自分自身の**ローカルな終端チェックマクロ(`__elwindui_inherit_{bare_name}_terminal!`)を
`$crate::`自己参照で生成・使用する:

```rust
macro_rules! __elwindui_inherit_{bare_name}_terminal {
    ($SubType:ty;) => {};
    ($SubType:ty; $($leftover:tt)+) => {
        compile_error!(concat!(
            "#[overrides]: no ancestor declared these methods #[overridable]: ",
            stringify!($($leftover)+)
        ));
    };
}
```

以前の設計では`UIElement`のみが1つの共有`__elwindui_inherit_terminal!`を`elwindui-core`に
生成し、他の全クラスがそれを(クレートを跨いで)自己参照していたが、この共有マクロへの参照
パスは「マクロを定義したクレート内での`Self`解決」に依存しており、**3クレート以上の連鎖**
(例: バックエンドクレートの`Window`が、消費者クレートの`inherits = ..`チェーンの中間に
現れる場合)で解決に失敗することが実機で判明したため、各クラスが自分専用のローカルな終端
チェックを持つ方式に変更した——`$crate::`は常に「このマクロを定義したクレート」を指すため、
クレート数に関係なく確実に解決する。

### 8.7 `trait_only`はトリオを生成しない

`trait_only`宣言(§2.2)は`prelude`を持たない——具象`struct`/`impl`が存在しないため、
「親の`impl`を組み立てる」処理自体が無い。supertrait境界(`ancestor_own_trait`経由の
`{Parent}Ext`)だけで祖先チェーンへの接続をRust自身のトレイト継承に委ねている。加えて、
このコードベース全体を確認した限り、`trait_only`宣言のリネーム前の裸名(`Window`/
`NativeControl`等)を`#[class(inherits = ..)]`のターゲットとして指している箇所は無い——
`struct_only`側は常にリネーム後の`{Name}Ext`名を直接指すため、`trait_only`自身のトリオは
元々誰にも呼ばれない。

これは同時に**クレート跨ぎの裸名衝突**も回避する: `trait_only`は「共通インターフェース宣言」
という性質上、複数のバックエンドクレートが**同じ裸名**を意図的に使い回す(`elwindui-core`の
`trait_only Window`と各バックエンドの`struct_only`実装`Window`構造体、など)。もし
`trait_only`側もトリオ+§9のラッパーモジュールを生成していたら、facadeが複数クレートの
`ui`名前空間をマージする際に`ambiguous glob re-exports`で衝突する(型自体はリネームにより
衝突しないが、生成される`__elwindui_macros_of_Window`のようなモジュール名は裸名ベースの
ままなので衝突する)。トリオを生成しないことで、この問題自体が発生しない。

---

## 9. クレート・モジュールを跨いだマクロ到達性

`#[macro_export]`が付いた`macro_rules!`は、**テキスト上どこに書かれていても定義元クレートの
ルートにしか実体を持たない**——型やトレイトと違い、そのマクロが実際に書かれたモジュールの
パス(`elwindui_core::ui::__elwindui_inherit_UIElement`のような)経由では、同一クレート内
からでも一切参照できない(実機で確認済み)。

一方、マクロ定義の**すぐ後で明示的に`pub use`による自己再エクスポート**を書けば、そのモジュール
パス経由でも解決できるようになる。`build_inherit_macros`/`expand_impl`/`expand_trait_only`は、
生成した各マクロ(`entry`/`skip`/`classify`/`sealed_check`)についてこれを行う——ただし
**このクラス自身の宣言箇所と同じスコープに直接**ではなく、専用のラッパーモジュール
`__elwindui_macros_of_{bare_name}`(`macro_reexport_mod_ident`)の中に置く:

```rust
#[doc(hidden)]
#[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
pub mod __elwindui_macros_of_Window {
    pub use crate::__elwindui_inherit_Window;
    pub use crate::__elwindui_inherit_Window_skip;
    pub use crate::__elwindui_inherit_Window_classify;
    pub use crate::__elwindui_check_not_sealed_Window;
}
```

専用モジュールに逃がしているのは、`#[macro_export]`による強制クレートルート配置と、クラス
自身がまさにそのクレートルート直下(例: `elwindui-backend-appkit`の`lib.rs`直下の生の構造体)
に宣言されているケースとの間で、同一スコープ内に同じ名前を2重定義してしまう(`E0255`)衝突を
確実に避けるため。

`path_module_prefix`は、祖先の`inherits =`/`struct_only =`に書かれた完全パスの末尾を
`__elwindui_macros_of_{末尾の裸名}`に置き換えたパスを、マクロの到達経路として使う——型自身が
到達できる経路(facadeの`pub mod ui { pub use elwindui_core::ui::*; pub use
elwindui_backend_appkit::*; }`のような複数クレートを束ねる再エクスポートも含む)を、マクロも
そのまま辿れる。これにより、祖先がどのクレートに属していても`elwindui-macros`側が
そのクレート名を決め打ちで知る必要が一切ない。

---

## 10. `$crate::`自己参照と`macro_expanded_macro_exports_accessed_by_absolute_paths`lint

`build_inherit_macros`が生成する`entry`/`skip`/`classify`マクロ同士の内部参照
(例: `entry`が`classify`を呼ぶ、`classify`が自分自身を再帰呼び出しする)は、呼び出し元が
どのクレートであっても常に「定義元クレートの`classify`」を指す必要がある。裸の参照
(`#classify_ident!`)は**呼び出し元**のスコープで解決される(定義元のスコープではない)ため、
`$crate::#classify_ident!`のような修飾が必須。

`$crate::`修飾は、**同一クレート内**で自己参照するケース(例: `elwindui-core`自身の中で
`Layout`が`UIElement`を継承する場合)では`crate::`に展開されるため、rustcの
`macro_expanded_macro_exports_accessed_by_absolute_paths`(マクロ展開由来の
`#[macro_export]`マクロを絶対パスで参照することの制限、rust-lang/rust#52234、デフォルトで
`deny`)に抵触する。この lint はマクロ定義側の`#[allow(...)]`では抑制できず(実機確認済み)、
**`#[class]`を使うクレートのクレートルート**(`lib.rs`/`main.rs`の先頭、`elwindui-codegen`が
生成したコードを`include!`するファイルも含む)で
`#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]`を明示する必要がある——
将来この lint が(警告文の通り)hard errorに格上げされた場合はこの仕組み自体の再設計が
必要になる、という既知のリスク。

---

## 11. マクロ本体に埋め込む`crate::`パスの書き換え(`rewrite_crate_segment`)

このクラスが**自分自身の将来の子孫のために**生成する`__elwindui_inherit_*!`トリオ
(§8)には、「このクラス自身がさらに祖先を持つ場合、その祖先の完全修飾トレイト/具象パス
(`next_trait`/`next_concrete`)」がリテラルトークンとして埋め込まれる。このパスの先頭が
`crate`キーワードだった場合(同一クレート内の祖先を指す場合、§7の要件により必ずこの形)、
**そのままでは正しく解決できない**——`crate`キーワードのhygieneは「そのトークンが元々
書かれたクレート」に紐づくが、このトークン列は**別のマクロの生成ボディへ埋め込まれ**、
さらにその生成ボディが**3クレート目から**マクロ呼び出しの連鎖を経て展開される場合、
定義元クレートへ確実には解決されない(実機で確認済み: `elwindui-core`の`ContentControl`
——`inherits = crate::ui::Control`——が、`notepad`から3段のマクロ呼び出しを経て
展開されたときに解決に失敗した)。

対処として、`next_trait`/`next_concrete`を組み立てる箇所(`expand_impl`の最終
`inherit_macros`ブロック)では、`rewrite_crate_segment`がトークン列の先頭が文字通り`crate`
であれば`$crate`(macro_rulesの正規のクレート内自己参照メタ変数)に書き換えてから埋め込む。
`$crate::`は`crate::`と違い、どこで展開されても常に定義元クレートへ正しく解決される。

---

## 12. ルートクラスモード

`inherits`を省略した`struct`/`impl`ペア(実装上`UIElement`のみが該当)は特別扱いになる:

- ユーザーが書いた全メソッドのうち、`#[overridable]`が付いていないものは、
  `{ClassName}Ext`トレイト宣言に**デフォルトメソッド**として直接その実ボディが埋め込まれる。
- `#[overridable]`が付いたもの(`visual_children`/`measure_override`/`arrange_override`/
  `paint`/`try_as_native_control`)は、`__dyn_x`と同じ発想のディスパッチを経由する——ただし
  `as_ui_element`(下記)を`__dyn_x`アクセサに転用できないため、ルートクラスは`__dyn_x`
  そのものに加えて**専用のディスパッチ用アクセサ**(`dyn_accessor_ident`の通常形)も持つ。
- `as_ui_element(&self) -> &ClassName` は必須(デフォルト無し)シグネチャとしてマクロ自身が
  合成する——ユーザーがこれを手書きするとコンパイルエラーになる。**具象型を返す**直接
  フィールドアクセス用のアクセサであり、`#[overridable]`メソッドのディスパッチには使えない
  (常に「今いる具象型」を返すため、途中の子孫のオーバーライドを飛び越してしまう)。
- `ClassName`自身はこのトレイトを`as_ui_element(&self) -> &Self { self }`という反射的実装で
  実装する。
- ルートクラスの生成トレイトは常に`: base::AsAny`を継承境界に持つ。

---

## 13. コンストラクタ自動生成(`construct` → `new`)

`&self`を持たない(レシーバ無し)メソッドで、名前が厳密に`construct`かつ戻り値`Self`のものを
`impl ClassName { .. }`内に書くと、マクロは自動的に対になる`new`を生成する:

```rust
fn construct(padding: Option<f32>) -> Self { .. }
// ↓ 自動生成される
pub fn new(padding: Option<f32>) -> std::rc::Rc<Self> {
    std::rc::Rc::new(Self::construct(padding))
}
```

- `construct`と`new`の両方を手書きすると、手書きの`new`が常に優先される(自動生成は
  発生しない)——`Rc::new(Self::construct(..))`だけでは足りない後処理(親ポインタ配線・
  イベント配線・初回`resync()`呼び出し等)が必要なクラス向け。
- `abstract_class`が立っているクラスは`new`を自動生成しない——`construct`自体は定義してよく、
  具象サブクラスが自分の`base`フィールドを組み立てる際に呼ぶための土台として機能する
  (例: `Layout::construct()`)。

---

## 14. 既知の制限・注意点

- `__dyn_x`方式は全ての`{ClassName}Ext`メソッドがdyn互換であることを要求する(§5末尾)。
- ジェネリッククラス(`NativeControl<H>`)は`trait_only`/`struct_only`の組み合わせでのみ
  使われており、`inherits = ..`先として他クラスから指定されることは無い——
  `build_dyn_default_methods`等がジェネリクスを完全にはサポートしていない(ターボフィッシュの
  付け忘れ等)ため、仮に将来ジェネリッククラスを`inherits`先にする場合は動作確認が別途必要。
- §10の`macro_expanded_macro_exports_accessed_by_absolute_paths` lintは現状「警告」だが、
  将来hard errorへ格上げされた場合、`$crate::`自己参照に依存するこの仕組み全体の再設計が
  必要になる。

### `AsAny`/`.as_any()`利用時の既知の罠(「as-any hack」)

`#[class]`自体のバグではないが、`struct_only`クラスが典型的に書く
`arg.as_any().downcast_ref::<ConcreteType>()`という具象型復元パターン
(`elwindui_core::base::AsAny`、`Rc<dyn XExt>`引数を受け取るsetterで多用される)には、
Rust自体のよく知られた罠が存在する。`AsAny`のブランケット実装
(`impl<T: Any> AsAny for T`)は、`Any`を実装する**あらゆる`'static`かつ`Sized`な型**に
適用される——これには`Menu`のような具象型だけでなく、`Rc<dyn MenuExt>`という**スマート
ポインタ自体**も含まれる(`Rc`は常に`Sized`)。

呼び出し先を書いたファイルで`use elwindui_core::base::AsAny;`を**直接**importしていると、
メソッド解決は`submenu.as_any()`の探索を`submenu`自身の型(`Rc<dyn MenuExt>`)から始め、
そこで(ブランケット実装経由で)`AsAny`が見つかった時点で確定してしまう——`dyn MenuExt`へ
derefする前に停止するため、返る`&dyn Any`は`Menu`ではなく`Rc<dyn MenuExt>`自身を指す
(同一アドレスでも`TypeId`が異なり、`downcast_ref`は常に`None`を返す)。

**回避策**: `AsAny`トレイトを直接importしない。`MenuExt`のような各`{Name}Ext`トレイトは
既に`AsAny`をsupertraitとして持っているため、`{Name}Ext`(または`UIElementExt`等)だけを
importしていれば、`.as_any()`はそのトレイトのsupertrait経由でのみ解決可能になり、
`dyn {Name}Ext`自身の(具象型ごとに正しい)vtableスロットへ確実に到達する。
`elwindui-backend-appkit/src/native_ui.rs`はこの理由で`AsAny`を意図的にimportしていない
(該当箇所のコメント参照)。

---

## 15. IDE解析(rust-analyzer)対応とデバッガ対応

### 15.1 背景: `class_arg_store`のソース順依存とrust-analyzer

§2.1で述べた通り、`struct ClassName`の`#[class(..)]`引数は`impl ClassName`側が読みに行く
プロセスグローバルなstore(`class_arg_store`、`store_class_args`/`load_class_args`)を経由する。
このstoreは**「同一クレート内では属性マクロが常にソース順に1プロセスで展開される」という
rustcの前提**に依存しており(`class.rs`冒頭のモジュールdocコメント)、rustcでのコンパイルでは
常に成立する。

rust-analyzerのマクロ展開は需要駆動・インクリメンタルな計算であり、この「ソース順に展開される」
という保証を持たない。`impl ClassName`側の展開が、ペアの`struct ClassName`側の展開より先に
rust-analyzer内部で評価されると、`load_class_args`は`None`を返す——このとき(この節の変更が入る
前の実装では)`expand_impl`は`compile_error!`**だけ**を返して展開全体を打ち切っていたため、
そのクラスが書いたメソッドも`new`も含めて何一つrust-analyzerから見えなくなり、結果として
「メンバー関数が補完候補に出ない」「`new()`の戻り値型が不明」という症状になっていた
(実際にユーザーから報告された症状と一致)。

### 15.2 `#[cfg(rust_analyzer)]`shadow(`build_rust_analyzer_shadow`)

`rust_analyzer`は rust-analyzer自身が`#[cfg(...)]`を評価する際にtrueとして扱う公式のcfgフラグ
であり、実際の`cargo build`/`cargo check`(rustc本体)では**決してtrueにならない**。`#[class]`
はこれを使い、`load_class_args`が`None`を返した場合(`expand_impl`、§15.1の失敗ケース)に限って、
以下の2つを**両方**出力するようになった:

- `#[cfg(rust_analyzer)]`限定の`impl ClassName { .. }`(`build_rust_analyzer_shadow`が構築)——
  `item.items`を`own_methods`/`override_methods`/`ctor_methods`へ分類する処理(store・`args`を
  一切参照しない、`expand_impl`冒頭のループ)の結果だけから組み立てる、**このクラス自身が
  宣言したメソッドとコンストラクタだけを平たいinherent methodとして持つ自己完結ブロック**。
  `store`の成否に依存しないため必ず成功する。`construct`があり手書きの`new`が無ければ
  (`abstract_class`かどうかはこの分岐では不明なため無視した簡略版のルールで)`new`も
  自動生成する。
- `#[cfg(not(rust_analyzer))]`限定の、従来通りの`compile_error!`——rustcでの本物のコンパイル
  では、struct宣言順の誤り(またはstructを書き忘れている)は今まで通り検出される。

**この失敗ケース以外では何も変わらない**——`load_class_args`が成功する通常のケース(圧倒的
多数)では、`trait_decl`/`trait_impl`/`ctor_block`/`__elwindui_inherit_*!`トリオ等の実生成物は
**一切変更されておらず、`cfg`で隠されてもいない**。実装当初は成功ケースも含めて実生成物全体を
`#[cfg(not(rust_analyzer))]`で隠し、rust-analyzerには常にこのshadowだけを見せる設計を試したが、
以下の理由で撤回した:

1. **`#[cfg(...)]`は直後の1アイテムにしか掛からない**。`trait_decl`/`trait_impl`/`ctor_block`/
   `__elwindui_inherit_*!`トリオ(最大5アイテム)のように複数の独立したトップレベルアイテムを
   連結したストリームの先頭に`#[cfg(not(rust_analyzer))]`を1つ書いても、2つ目以降のアイテムは
   無条件にコンパイルされてしまう(`shadow`と衝突し、`E0034`等で実ビルド自体が壊れた)。
   これを`mod { .. }`でラップしてアイテムひとつに畳もうとしたが、`inherit_macro_path`(§8.2)が
   同一クレート内の祖先マクロを**裸名**で呼ぶ設計(`$crate::`やパス修飾を使わない)に依存して
   おり、生成物を1段深いモジュールへ移すとその裸名解決が壊れる(`cannot find macro ... in this
   scope`)。裸名呼び出し規約自体を変える(=より大きな設計変更)よりも、そもそも成功ケースを
   隠す必要性自体を見直す方を選んだ。
2. **`{ClassName}Ext`という型名そのものを消してしまうと実害が大きい**。このコードベースには
   `Rc<dyn UIElementExt>`/`T: UIElementExt`のように`{Name}Ext`を**型として**直接書いている
   箇所が多数ある(メソッド呼び出しの補完だけの問題ではない)。成功ケースの`trait_decl`ごと
   隠すと、rust-analyzer上でこれら**型位置の参照が軒並み解決不能になる**——元の(store失敗時
   だけ発生する)問題よりずっと広範囲な退行になってしまう。

### 15.3 `expand_struct`側の`Deref` shadow

`expand_struct`(§2.1の`struct`側展開)は`class_arg_store`を一切参照しない、常に単独で成功する
関数である。`inherits = ..`がある場合、`#[cfg(rust_analyzer)]`限定で

```rust
impl std::ops::Deref for ClassName {
    type Target = <inheritsで指定した祖先の型>;
    fn deref(&self) -> &Self::Target { &self.base }
}
```

を追加で出力する。祖先の各クラスも同じ仕組みで自分の`base`へのDerefを持つため、rust-analyzerの
オートデリファレンスは`self.base.base. ...`と祖先チェーンをそのまま辿れる——15.2の失敗ケース
(祖先が分からずshadowにsupertrait境界を書けない場合)でも、祖先の`#[inherent]`ヘルパーや
公開フィールドへは、この`Deref`経由でなおアクセス可能になる。ストア非依存で常に成功するため、
15.1のリスクとは無関係に安全に追加できる。

### 15.4 Span保持とデバッガのブレークポイント

調査の結果、`own_methods`/`ctor_methods`/`override_methods`(ユーザーが書いた実メソッド本体)は
一貫して元の`syn::ImplItemFn`をそのまま`quote! { #f }`でspliceしており、文字列化や再構築を
一切経ていない——**実ビルド側は元々Spanを保持しており、デバッガでのブレークポイント配置は
この変更以前から成立している**(この節を追加する過程で新たに保証したのではなく、既存の実装が
既にこの性質を満たしていることを確認した)。15.2の shadow も同じ`ImplItemFn`をそのまま
spliceするため、rust-analyzer上のホバー/定義ジャンプも実ソース行を指す。

唯一の既知の例外は`inherits =`/`struct_only =`の型パスで、`store_class_args`/`load_class_args`
を経由する際に一度`.to_string()`で文字列化され`syn::parse_str`で再パースされる
(`class.rs`の`StoredClassArgs`まわり)——このトークンは`call_site` spanになり、生成された
supertrait境界から「定義へジャンプ」した際に元の記述位置ではなくマクロ呼び出し位置に飛ぶ、
という軽微な影響のみを持つ。メソッド本体やブレークポイントには一切関係しないため、今回は
対応スコープ外とし、既知の限定的な制約としてここに記録するに留める。

### 15.5 検証方法

rust-analyzer本体(`rustup component add rust-analyzer`で導入可能)の`rust-analyzer diagnostics
<path>`サブコマンドは、エディタのLSPセッションを起動せずに実解析エンジンをワークスペース全体へ
バッチ実行し、実際にIDE上で出るのと同じ診断を出力する——本番のrust-analyzerによる検証はこれで
行う。加えて、`rust_analyzer`cfgが立ったときのコードが型として成立するかは
`RUSTFLAGS="--cfg rust_analyzer" cargo check --workspace`
でも代替検証できる(このcfgはrust-analyzer以外では立たないため、通常の`cargo build`/`cargo test`
には一切影響しない)。15.2のshadow経路(store失敗ケース)はrustcでの通常コンパイルでは構造上
決して踏まれない(structは常にimplより先に展開されるため)ので、この2つのコマンドだけでは
検証できず、`crates/elwindui-macros/src/class.rs`の`rust_analyzer_shadow_tests`
(`#[cfg(test)]`)が`expand_impl`をstruct無しで直接呼び、返された`TokenStream2`が
`syn::parse2::<syn::File>`で構文的に妥当であることを検証している。

### 15.6 `same_crate_classes`のクレート跨ぎ汚染と`compiling_crate_key`

`rust-analyzer diagnostics .`を実際にこのワークスペース全体に対して実行したところ、15.2〜15.4
とは別の、今回のセッションより前から存在するバグが見つかった: `same_crate_classes`
(`register_same_crate_class`/`validate_fully_qualified_path`/`inherit_macro_prefix`/
`struct_only_collides_with`/`ancestor_own_trait`/`ancestor_is_no_ancestor_forward`が使う、
「このbare名は同一クレート内で`#[class]`宣言されているか」を記録するプロセスグローバルな
`OnceLock`レジストリ、§8参照)が、`class_arg_store`と同じ「1プロセス=1クレートのコンパイル」
という前提に依存していた。

通常の`cargo build`は1クレートのコンパイルごとに別プロセスが立つためこのレジストリは自然に
リセットされるが、rust-analyzerは**ワークスペース全体を1つの長寿命proc-macro-srvプロセスで
解析する**。そのため`elwindui-core`を解析した際に登録された`Window`/`ContentControl`/
`NativeControl`等のbare名が、同一セッション内で後から解析される**別クレート**(`notepad`/
`elwindui-backend-appkit`)の解析にそのまま漏れ出し、`notepad`側の正当なクレート跨ぎ参照
`elwindui::ui::Window`が「同一クレート内のクラスなのに`crate::`で始まっていない」という
誤検知エラーになり、そこから`unresolved macro __elwindui_inherit_NativeControl!`や複数の
`E0599`(no method)エラーへ連鎖していた。

**対処**: `compiling_crate_key()`(`class.rs`)を追加し、環境変数`CARGO_CRATE_NAME`
(無ければ`CARGO_PKG_NAME`)を都度読んで「今まさにコンパイル中のクレート」を識別する——これは
実cargoだけでなくrust-analyzer自身のproc-macro-srvプロトコルも、プロセスが使い回されている
場合であっても各マクロ展開リクエストごとに新しく設定する値であるため、rust-analyzer上でも
正しく機能する。`class_arg_store`/`same_crate_classes`の両方のキーを、bare名/クラス名単体から
`(compiling_crate_key(), 名前)`のタプルへ変更した——これにより、同じ長寿命プロセス内で複数
クレートが処理されても、レジストリの中身がクレートごとに正しく分離される。

`rust-analyzer diagnostics .`で再検証した結果、この対処後はワークスペース全体で`#[class]`
関連のエラーは0件になった(唯一残る`Error`級診断は`notepad-inline`の`elwindui::component!`
——`#[class]`とは別のプロシージャルマクロで、このセッションの変更とは無関係、未調査)。

**教訓**: この種のプロセスグローバルな状態を今後このモジュールに追加する場合は、必ず
`compiling_crate_key()`でスコープすること——単一クレートを個別にビルド/チェックしただけでは
この種の漏れは決して再現しない(2つ以上の相互依存クレートが同じproc-macro-srvプロセス内で
解析されて初めて顕在化する)ため、`rust-analyzer diagnostics`をワークスペース全体に対して
実行しない限り気づけない。
