# elwindui GUIフレームワーク設計書

本ドキュメントは `docs/elwindui_spec.md`(ElwindUIL言語仕様書)を元に、**GUIフレームワーク本体**(言語コアモデル・バックエンド抽象化・ランタイム・標準ビルトイン部品・状態管理層)の設計を実装者向けに再構成したものである。仕様書の記述を単純に転記するのではなく、「誰が何を実装するのか」「各機能はどの静的検証ルールで守られているか」「各層はどう連携するか」という設計上の関心に沿って再編している。

## 本ドキュメントのスコープ

**対象(本ドキュメント)**: ElwindUIL言語のコアモデル(`component`/`view`/`param`/`prop`/`Element`)、バックエンド抽象化、`elwindui-core`ランタイム、標準ビルトイン部品、Store/ViewModel/MVVMなどの状態管理層、ナビゲーション・テーマ・エラーハンドリング等のUI機能拡張。

**対象外(ツール設計書を参照)**: `.elwind`→Rustのコード生成コンパイラ(`elwindui-codegen`)、LSP(`elwindui-languageserver`)、エディタ内プレビュー、ホットリロード機構。これらは `docs/elwindui_tool_*_design.md` 側の設計書を参照すること(仕様書 付録B に対応)。

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

ElwindUILは特定のGUIフレームワークに依存しない中間表現として設計されている(仕様書 付録A)。

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
┌──────────┬──────────┬──────────┬────────┬────────┐
│ WinUI3   │ AppKit   │ GTK4     │ egui   │ iced   │ …Uikit/Jetpack(付録W)
│ backend  │ backend  │ backend  │ backend│ backend│
└──────────┴──────────┴──────────┴────────┴────────┘
```

クレート構成(付録H.5):

```
elwindui-core           # Element, LayoutEngine, FocusManager, AccessibilityTree, InputRouter, Painter(共通・バックエンド非依存)
elwindui-backend-winui3 # elwindui-coreを実装 + windows-rsでネイティブAPIに橋渡し
elwindui-backend-appkit # 同上、objc2経由
elwindui-backend-gtk4   # 同上、gtk-rs経由
elwindui-backend-egui   # 同上 + accesskitでa11y補完
elwindui-backend-iced   # 同上 + accesskitでa11y補完
```

`.elwind`コンパイラが生成するコードは常に`elwindui-core`のトレイト境界に対して書かれ、実行時にどのバックエンドクレートがリンクされるかで実体が決まる。バックエンド指定は`#![backend(...)]`(ビルド設定)と`target::backend()`(式内定数、§3.3)の2つの窓口を持つ。

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

.elwindは論理的な要素ツリーを記述するのみで、egui/iced/druid系や各OSネイティブツールキットへの変換は「バックエンド」が担う(§1参照)。制約検証・enum網羅性検査・i18n解決などの言語機能はすべてバックエンド非依存のフロントエンド解析段階(ツール側の責務)で完結し、バックエンド選択に影響されない。

### 3.2 OSネイティブツールキットへの抽象化

Windows→**WinUI 3**(windows-rs経由)、macOS→**AppKit**(objc2経由)、Linux→**GTK4**という、OS標準ツールキットへコンパイル時に振り分ける。

```rust
#![backend(native)]   // ビルドターゲットに応じてOS標準ツールキットへ自動的に振り分ける
```

明示固定したい場合はRustの`cfg`属性の慣習に沿う:

```rust
#[cfg(target_os = "windows")]
#![backend(winui3)]
```

OSごとの見た目差はスタイル層に閉じ込める(`style { select(Button, backend == Backend::Winui3) { corner_radius: 4 } }`)。プラットフォーム固有機能へのエスケープハッチは`native!`ブロックを`#[cfg(backend = "...")]`と組み合わせて使う。

`prop`変更の反映方式は保持モード系ネイティブバックエンドでは「対応ネイティブAPIのプロパティ更新呼び出し」に、即時モード系(egui)では「毎フレーム再構築」になるが、`param`/`prop`/`Element`トレイトの意味自体はバックエンドを問わず共通である。

### 3.3 `target::backend()`(コンパイル時静的定数)

```rust
enum Backend {
    Winui3, Appkit, Gtk4, Egui, Iced,
    Uikit,      // iOS(付録W)
    Jetpack,    // Android(付録W)
}
```

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

`match target::backend() { ... }`は`Backend`の全メンバー網羅を要求される(§2.3の網羅性検査と同じ仕組み)。**新しいバックエンド(`Uikit`/`Jetpack`等)を追加すると、既存の全ビルトインリファレンス実装が非網羅エラーになる** — これは仕様の欠陥ではなく、「新バックエンド追加時にどのビルトインが未対応かを機械的に洗い出す」安全弁として意図された挙動である。

コード生成時、`target::backend()`はビルド設定から得た値へ定数畳み込みされ、該当しない分岐は生成対象から静的に除去される。

### 3.4 名前空間とビルトインのオーバーライド規則

ビルトインは予約名前空間`builtin::*`に属する(`Row { ... }`は`builtin::Row`への暗黙の`use`が常に効いている扱い)。**大原則: 暗黙のシャドーイングは一切許可しない。**

| ケース | 挙動 |
|---|---|
| ユーザー定義コンポーネントが別名 | ビルトインと共存、曖昧さなし(推奨) |
| 同名だが`#[overrides]`なし | 静的エラー(曖昧参照として拒否) |
| 同名で`#[overrides(builtin::X)]`あり | そのスコープ内でユーザー定義が優先、ビルトインは`builtin::X`で明示参照可能 |
| シグネチャ不一致 | 静的エラー(置き換え先の必須フィールドを満たさない) |

`#[overrides]`の効力はそのコンポーネントを`use`で取り込んだファイル内でのみ有効(プロジェクト全体を暗黙に汚染しない)。複数コンポーネントが同じビルトインに対し`#[overrides]`を宣言し同一スコープで両方`use`された場合は多重オーバーライドエラー。

---

## 4. 標準ビルトイン部品

`Window`/`Column`/`Row`/`Text`/`TextArea`/`Dropdown`等は`builtin`名前空間に属し、コード生成器が標準実装として提供する。内部実装は他コンポーネントと同じ`component`/`view`構文で表現でき、`match target::backend()`(§3.3)による網羅性検査と`native!`エスケープハッチがそのまま適用される。

**代表的な実装パターン(`Stack` → `Column`/`Row`)**: 共通の`Stack`部品に`orientation`を渡して処理を委譲し、`Column`/`Row`はその薄いラッパーとして定義する。

```rust
component Stack { #[param] orientation: Orientation, #[param] spacing: number = 0, children: Vec<Element> }
component Column { children: Vec<Element> }
view Column { Stack { orientation: Orientation::Vertical, children } }
```

`Rect`(§付録F.6)はegui/iced backend向けの、ネイティブAPIを持たないbackendのための最小コンテナ要素で、`Button`の`#[overrides]`実装がこれを利用する例が仕様書に示されている。

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
Element ツリー(§2.11)
        │
        ▼
┌─────────────────────────────────────────┐
│ ElwindUIL Core Runtime(elwindui-core)      │
│  ├─ LayoutEngine      (制約ベースのMeasure/Arrange) │
│  ├─ FocusManager      (フォーカス移動・トラップ)     │
│  ├─ AccessibilityTree (UIAツリー相当)              │
│  ├─ InputRouter       (ヒットテスト・イベント配送)   │
│  └─ Painter           (§5.4参照)                  │
└─────────────────────────────────────────┘
        │
        ▼
各バックエンド実装(WinUI3/AppKit/GTK4/egui/iced)
```

ネイティブ系バックエンドはOS標準機構に極力委譲し、egui/icedのような非ネイティブ系バックエンドはCore Runtimeの共通実装(または`accesskit`のような橋渡しクレート)に依存する。

### 5.1 レイアウトエンジン

WinUI3の`Measure`/`Arrange`2パス方式を採用する。

```rust
trait LayoutNode {
    fn measure(&self, available: Size) -> Size;
    fn arrange(&mut self, final_rect: Rect);
}
```

`Stack`や`Canvas`を含む全ビルトインがこのトレイトを実装する。`.elwind`側の`width`/`height`/`spacing`等の属性がそのままMeasure/Arrangeの入力になり、新しい構文は不要。**レイアウト計算は`elwindui-core`内の共通実装(1つのRustクレート)で一元化され**、バックエンドは計算結果(確定した矩形座標)を受け取ってネイティブAPIに反映するだけ、という役割分担にする。これにより全バックエンドで同一のレイアウト結果が保証される。

### 5.2 フォーカス管理

```rust
trait FocusManager {
    fn move_focus(&mut self, direction: FocusDirection) -> Option<ElementId>;
    fn set_focus(&mut self, id: ElementId);
    fn focused(&self) -> Option<ElementId>;
    fn trap_focus(&mut self, scope: ElementId);
}
```

`.elwind`側は`#[focus(order: 1)]`/`#[focus(trap: true)]`属性で参加する。Tab移動順序・方向キー移動はCore Runtimeが共通ロジックとして提供し、ネイティブ系バックエンドはOS標準のフォーカス機構(WinUI3の`FocusManager`、AppKitの`NSResponder`チェーン、GTK4の`gtk_widget_grab_focus`)にCore Runtime側を正としてミラー同期する。`Dialog`(§8.3)は既定で`#[focus(trap: true)]`が自動適用される。

### 5.3 アクセシビリティ

```rust
trait AccessibilityNode {
    fn role(&self) -> AccessibilityRole;
    fn label(&self) -> String;
    fn state(&self) -> AccessibilityState;
    fn children(&self) -> Vec<&dyn AccessibilityNode>;
}
```

ビルトイン部品は既定roleを自動付与するため通常追記不要だが、`Canvas`ベースの独自部品(§5.4)は意味情報を持たないため`#[accessible(role:, label:, ...)]`の明示を推奨し、付けない場合14章ルール10により静的警告となる。バックエンド実装義務はWinUI3=`AutomationPeer`、AppKit=`NSAccessibilityElement`、GTK4=`Atk`/AT-SPI、egui/iced=`accesskit`クレート経由。

### 5.4 独自描画部品(Canvas / Painter)

グラフ・ゲージ等「ピクセル単位で自分で描く」部品は宣言的な`view`構文の対象外とし、`Canvas`ビルトイン+命令的な`Painter`描画コードの組み合わせとして扱う。レイアウトは引き続き`.elwind`で宣言的に書き、描画内容は`Painter`を受け取るRust関数として書く。

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

`builtin::Canvas`自身は他ビルトイン同様`match target::backend()`で各backendのネイティブ描画コンテキストを`Painter`実装でラップし`on_paint`に渡す(バックエンド分岐が許されるのは`builtin`定義のみという§4.1の原則がここでも維持される)。

描画コードは`.elwind`の外、通常のRustファイル(`src/painters/*.rs`)に分離する。推奨ディレクトリ構成:

```
src/
├── ui/       # .elwind本体(レイアウト定義)
├── painters/ # 描画ロジック(通常のRust、バックエンド共通実装)
└── logic/    # on_click等の業務ロジック
```

`Canvas`の`prop`が変わると通常の`prop`更新ルール(§2.2)で再描画がトリガーされる。毎フレーム再描画したい場合は`#[animated]`を付け、その内部でのみ非純粋関数呼び出し(`elapsed_time()`等)が許可される(14章ルール2の例外)。クリック・ドラッグは`on_pointer_down`/`on_pointer_move`で扱い、座標系は論理ピクセルに統一してバックエンド側が実ピクセル変換を担う。

`Canvas`と`Row`/`Column`等の既存部品は同じ`Element`ツリー・`LayoutNode`として自然に混在できる(§2.11・§5.1が支えている)。

### 5.5 描画機能の拡張(Composition相当のビジュアル効果)

`Painter`の基本セット(塗り・線・テキスト)を拡張し、WinUI3の`Win2D`/`Composition`相当の機能を提供する。いずれも`elwindui-core`に属し、バックエンド差異は`builtin::Canvas`内部にのみ許可される(§4.1原則の継続)。

| 機能 | 型/メソッド | 備考 |
|---|---|---|
| ブラシ(単色/グラデーション/画像/Acrylic) | `Brush` enum + `fill_rect_brush`/`stroke_path_brush` | GTK4/eguiはAcrylic/Blur非対応時、単色フォールバック+静的警告(14章ルール17) |
| ジオメトリ(ベジエ・弧) | `Path` + `StrokeStyle`(cap/join/dash) | |
| エフェクト(シャドウ・ブラー・色調補正) | `Effect` enum + `#[effect(...)]` | オフスクリーンサーフェスへレンダリング後に適用 |
| 変形(移動・回転・拡縮・スキュー) | `Transform` enum + `push_transform`/`pop_transform` | スタック方式、ネスト可 |
| レイヤー合成・クリップ・ブレンド | `begin_layer`/`end_layer`/`clip_rect`/`clip_path` + `BlendMode` | エフェクトの基盤機構 |
| 暗黙アニメーション | `#[transition(duration, easing)]` | propに付与、値変化時に自動補間描画 |
| キーフレームアニメーション | `KeyframeAnimation::new().add(t, v).easing(...).sample(t)` | `Canvas`内での手続き的制御、位置は`0.0..=1.0`(範囲外は14章ルール16でエラー) |
| リッチテキスト | `TextRun` + `draw_rich_text` | 複数書式混在テキストの1回描画 |

存在しないイージング関数名や範囲外キーフレーム位置は14章ルール16で静的エラーとなる。

### 5.6 クレート構成のまとめ

§1のクレート一覧を参照。`elwindui-core`が`Element`/`LayoutEngine`/`FocusManager`/`AccessibilityTree`/`InputRouter`/`Painter`という共通・バックエンド非依存な基盤を持ち、各`elwindui-backend-*`クレートがこれを実装してネイティブAPIへ橋渡しする、という構成が全体を貫く設計原則である。

### 5.7 ルーティングイベント(WinUI3スタイル)

WinUI3の`RoutedEvent`に倣い、`#[routed]`属性(`#[two_way]`と同じ、`.elwind`のコールバック型フィールドに付与するアトリビュート)を付けたイベントは、発生元の要素から祖先へバブルする。対象は`on_click`のような入力系イベントに限られ、`TabView`の`on_select(usize)`のようなウィジェット固有の型付きペイロードを持つコールバックはルーティング対象外(既存の直接配線のまま)。

```rust
// crates/elwindui-codegen/src/builtins.elwind (Button)
component Button inherits NativeControl {
    #[routed]
    on_click: fn(),
}
```

**木構造は`Box`ではなく`Rc`で、本物の親ポインタを持つ**。`Element`ツリー(`Rc<dyn UIElement>`)の各ノードは`UIElementBase.parent: RefCell<Option<Weak<dyn UIElement>>>`という親への弱参照を持ち、要素が木に組み込まれる瞬間(`elwindui_core::tree::new_element`)に必ず設定される。これにより`dispatch_routed`は要素からルートまで単純に`parent()`を辿るだけでバブルでき、静的な`.elwind`構造でも、`TabView`の`items_source`/`item_template`のように実行時に動的組み立てられた木でも同じように機能する(木を毎回探索し直す必要がない)。

```rust
// elwindui-core
pub fn dispatch_routed<T: 'static>(target: &Rc<dyn UIElement>, name: &str, payload: &T, args: &RoutedEventArgs);
pub fn hit_test(root: &Rc<dyn UIElement>, available: Size, at: Point) -> Option<Rc<dyn UIElement>>;
```

`hit_test`は座標から最深(最前面)の要素を1つ返すだけで、経路は返さない — バブルは戻り値から`dispatch_routed`するだけで済む(親ポインタが経路計算を代替する)。`RoutedEventArgs { handled: Cell<bool> }`にハンドラが`handled`を立てると、そこで伝播が止まる。

ハンドラ本体は`UIElementBase.routed_handlers`(イベント名で引く型消去レジストリ、`Rc<RefCell<HashMap<&'static str, Vec<Box<dyn Any>>>>>`)に登録される。`Button`のようなネイティブウィジェットは、自分自身の構築時点(まだ`NativeControl`ラッパーが存在しない、木構築はboundary-up)に自分自身の`routed_handlers`へ登録し、`elwindui-codegen`の`into_node_if_needed`がラップ時に同じ`Rc`を共有する。実際のネイティブクリック配線(`NSButton`のtarget-action等)は、木とネイティブハンドルの両方が同時にスコープ内にある唯一の場所である各バックエンドの`relayout`(`TreeHostView`/`TreeHostPanel`)が担う。

現時点の実装範囲は、AppKitバックエンドの`Button`のみ(検証済み)。トンネリング(`Preview*`)、`Canvas`上のポインタイベント、WinUI3バックエンドでの実配線は将来の課題として残る。

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

- `on_mount`/`on_unmount`はツリーへの組み込み直後/除去直前に一度だけ実行される
- `on_update(field, ...)`は指定propまたは`#[computed]`の変化毎に発火。無引数の`on_update: { ... }`は任意prop変化で発火(濫用注意)
- これらは通常のRustコードブロックであり、`#[param]`静的評価式(§2.2)とは別の実行コンテキストのため非純粋関数呼び出し制限は適用されない
- **ただし`#[param]`フィールドへの代入はライフサイクルフック内でも禁止**される(14章ルール11)。`#[param]`の「実体化時のみ確定・以後不変」という原則はフックの内側でも一貫する

コード生成器は各バックエンドのライフサイクル(WinUI3の`Loaded`/`Unloaded`、AppKitの`viewDidAppear`/`viewWillDisappear`、GTK4の`realize`/`unrealize`、egui/iced初回フレーム検出)にこれらのフックをマッピングする。リスト仮想化(§8.4)でリサイクルされる要素は、プール再利用時に`on_mount`を再発火させず`prop`更新のみで反映する点に注意。

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

### 7.2 ViewModel / Command(MVVM)

WinUI3/WPF由来のMVVMパターンを、**新しい実行時機構を作らず**`#[computed]`(§2.2)と`store`(§7.1)の仕組みを再利用して導入する。

| MVVMの層 | ElwindUILでの対応 |
|---|---|
| Model | 通常のRust構造体、または`store` |
| ViewModel | `viewmodel`(本節) |
| View | 既存の`component`/`view`。ViewModelを`#[inject]`で受け取り表示のみ担当 |

```rust
viewmodel NotepadViewModel {
    #[observable]
    #[length(0..=100000)]
    content: String = String::new(),

    #[computed]
    char_count: i32 = content.chars().count() as i32,

    #[command(can_execute: state != SaveState::Saving)]
    save: Command = command!(|| {
        state = SaveState::Saving;
        document::save(&content);
        state = SaveState::Saved;
    }),
}
```

- `viewmodel`は`store`と同じフィールド構文を再利用する(新しい式構文は導入しない)
- `#[observable]`は「実行時に変化しView側へ伝播する」フィールド修飾子(propに相当)
- `viewmodel`は`view`ブロックを持てず、ビルトイン要素への参照が内部に出現すると14章ルール19により静的エラーとなる(V/VM分離が構文レベルで強制される)
- `Command`型は`can_execute`(`#[computed]`と同じ静的依存関係抽出で自動再評価)を持ち、`command!(|| { ... })`マクロでロジックを包む(`bind!`と同じ慣習)
- View側はViewModelを`#[param] #[inject]`で受け取り(§7.1の`#[scoped]`+`#[inject]`と同じ注入パターン)、双方向編集フィールドは`bind!(vm.field, TwoWay)`でpropに写し取り、読み取り専用表示(`vm.window_title`等)は`view`式中で直接参照してよい(14章ルール13の対象外 — ルール13は`#[param]`初期化式への直接参照のみを禁止)

**低オーバーヘッドな内部表現**: 依存関係はコンパイル時に静的抽出し(`#[computed]`と同一の仕組み)、動的な購読リスト(`Vec<Box<dyn Fn()>>`)は持たない。`Copy`可能な型は`Cell<T>`、非`Copy`型のみ`RefCell`で保持し、`Command`の本体は具体的なクロージャ型として単相化する(`dyn Trait`を使わない)。複雑な相互依存で静的解析が困難な場合のみ、`elwindui-core`が提供する汎用リアクティブグラフ(スロットマップ+世代インデックスの`SignalId`)にフォールバックする。

`viewmodel`はバックエンドを一切起動せず通常の`#[test]`で単体テスト可能(§9参照)。

`store`との関係:

| | `store` | `viewmodel` |
|---|---|---|
| 目的 | アプリ全体で共有される永続的/半永続的データ | 特定View向けの表示用データと操作 |
| インスタンス | 既定でシングルトン(`#[scoped]`で複数化可) | 常にView単位、`#[inject]`で注入 |
| Command | 持たない(素のRustロジック関数を直接呼ぶ) | `Command`型で保持、`can_execute`込みで公開 |

### 7.3 非同期処理

新しい実行モデルは導入せず、既存の`#[computed]`・`Command`を非同期版に拡張する。

```rust
enum AsyncState<T> { Idle, Loading, Success(T), Error(String) }

viewmodel DocumentViewModel {
    #[observable]
    file_path: String,

    #[async_computed]
    content: AsyncState<String> = task!(async { fs::read_to_string(&file_path).await }),
}
```

- `AsyncState<T>`は通常のenumとして網羅性検査の対象(`match`でIdle/Loading/Success/Errorの処理漏れを静的検出)
- `#[async_computed]`は`#[computed]`の非同期版。`#[observable]`依存が変化すると自動再実行され、実行中は`Loading`
- `#[async_computed]`/`#[command(async, ...)]`が`viewmodel`/`store`以外に付与された場合は静的エラー(14章ルール20) — 非同期状態はVM/Model層に閉じ込め、`component`の`#[param]`静的評価式を汚染しない
- `#[command(async, can_execute: ...)]`は実行中自動的に`can_execute`が`false`扱いになる(多重実行防止)。`#[command(async, cancellable)]`で`vm.command.cancel()`を提供
- `elwindui-core`はホストの非同期ランタイムを直接指定せず`spawn(fut)`という薄い抽象を提供し、各バックエンドがWinUI3の`DispatcherQueue`/AppKitの`DispatchQueue.main`/GTK4の`glib::MainContext`/egui・icedのホストランタイムに橋渡しする

### 7.4 Undo/Redo

編集操作のUndo/Redoを`viewmodel`フィールドへの共通仕組みとして提供する。

```rust
viewmodel NotepadViewModel {
    #[observable]
    #[undoable(coalesce: 500ms)]
    content: String = String::new(),
}
```

- `#[undoable]`は`viewmodel`の`#[observable]`フィールドにのみ付与できる(14章ルール21) — Undo単位は「1つのViewの編集セッション」に紐づくため、アプリ全体共有の`store`や`component`の`prop`には意味を持たない
- `#[undoable]`フィールドが1つ以上ある`viewmodel`には`undo`/`redo: Command`と`can_undo`/`can_redo`が自動追加される
- `coalesce: 500ms`で連続入力を1つのUndoエントリにまとめる(`#[transition(duration:...)]`と同じ「時間指定アトリビュート」の慣習)

---

## 8. UI機能拡張ビルトイン

### 8.1 キーボード入力・ショートカット

要素単位: `on_key_down`/`on_key_up`(物理キー)、`on_text_input`(IME確定後の文字列)。フォーカスを持つ要素のみ受信する(§5.2の`FocusManager`と連動)。

グローバル: `#[shortcut("Ctrl+S")]`はプラットフォーム非依存の修飾キー表記で、コード生成時にmacOS向けビルドで`Ctrl`→`Cmd`へ自動読み替えされる。個別OS割り当ては`#[shortcut(winui3: "Ctrl+S", appkit: "Cmd+S")]`のように複数指定可能。既定ではフォーカス無関係にウィンドウ内で発火し、`scope: local`でフォーカス時のみに限定できる。

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

`match current_route { ... }`は`Route`の全メンバー網羅を要求される(14章ルール14、§2.3と同じ仕組み)。遷移操作は`navigate!(route)`(遷移+履歴push)/`navigate_back!()`(履歴を1つ戻す)。`NavigationHost`はビルトインのため内部で`match target::backend()`を持つ(WinUI3=`Frame`、AppKit=`contentViewController`差し替え、GTK4=`gtk::Stack`、egui/iced=内部状態による表示切り替え)。§4.1の原則通り、通常のcomponentはこの分岐を書けない。

### 8.3 ダイアログ・ポップアップ・メニュー

- `Dialog`: モーダル。`#[focus(trap: true)](§5.2)`が自動適用され、`on_dismiss`はEsc・外側クリック・明示的な閉じるボタンいずれからも発火
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
- `item_height`固定なら§5.1のMeasureパスをスキップし定数時間で表示範囲を計算、`estimated_item_height`のみなら初回`measure`で実測しキャッシュ
- 画面外に出た`Element`はプールに戻し再利用する。再利用インスタンスでは`on_mount`(§6.1)は初回プール生成時のみ発火し、以降は`prop`更新のみ行う
- `key`未指定で順序が変わる更新を行うと挿入位置ベースの再利用にフォールバックし、14章ルール23により静的警告

### 8.5 テーマ/デザイントークン

`style{}`(§2.4)は個別属性の上書きに留まるため、カラーパレット・スペーシング・タイポグラフィを一元管理する`theme`構文を用意する。

```rust
theme AppTheme {
    tokens { color primary; color background; color text; spacing unit; font body; font heading }
    variant Light { primary: "#2ecc71"; background: "#ffffff"; ... }
    variant Dark  { primary: "#27ae60"; background: "#111111"; ... }
}
```

- 全`variant`は`tokens{}`宣言のトークンを過不足なく持たねばならない(14章ルール22) — 「ダークモードだけ特定の色が未定義」という事故を静的に防ぐ
- 参照は`AppTheme.token名`という`.`アクセス(`env::*`やstoreフィールド参照と同じ慣習)。`style{}`からも`Painter`/`Brush`(§5.5)からも同じ記法で参照可能
- 実行時切り替えはファイル単位アトリビュート`#![theme(AppTheme, variant: bind!(AppSettings.theme_mode, OneWay))]`で宣言し、storeの変化に応じて`AppTheme.*`参照箇所が自動再評価される(既存のprop差分更新の仕組みに乗る)

### 8.6 エラーハンドリング(エラーバウンダリ)

`view`内の予期しないエラーでアプリ全体をクラッシュさせず、該当部分だけフォールバック表示に切り替える。

```rust
ErrorBoundary {
    fallback: |err| Text { text: t!("error-fallback", message: err.to_string()), color: "#e74c3c" }
    NotepadWindow { }
}
```

- `view`構築・`#[computed]`評価・`Canvas`の`on_paint`実行中のエラーを捕捉し`fallback`に置き換える。ネスト可能で内側の`ErrorBoundary`が捕捉範囲を限定する
- 内部的には`catch_unwind`相当の仕組みで囲むが、ネイティブAPI呼び出し(COM/Objective-C/GObject)を跨ぐパニックは言語境界でUB化する恐れがあるため、ネイティブ呼び出し部分は`Result`化を必須としcatch_unwindは純粋Rustロジックの範囲に留める(ベストエフォート方針)
- 同期`Command`のエラーは`#[command(catches: ErrorType)]`で`viewmodel`の`last_error`相当フィールドに自動格納(§7.3の非同期版と対になる同期パターン)
- 未捕捉時は`elwindui-core`既定のフォールバック画面(デバッグ=詳細スタック、リリース=簡潔メッセージ)でクラッシュを防止する

### 8.7 クリップボード・ドラッグ&ドロップ・ファイルダイアログ

OS機能へのアクセスをGUI要素ではなく`platform::`名前空間の関数として提供する(`env::*`/`external::*`と同じ「明示的な入口」の思想)。

```rust
platform::clipboard::write_text(&content);
let text: Option<String> = platform::clipboard::read_text();
```

ファイルダイアログは本質的に非同期(ユーザー操作待ち)なので常に`Future`を返し、§7.3の`#[command(async)]`パターンと組み合わせる。ドラッグ&ドロップは`draggable: bool`/`on_drag_start`/`on_drop`を任意のビルトイン要素が持てる共通属性として提供する。

| 機能 | WinUI3 | AppKit | GTK4 | egui/iced |
|---|---|---|---|---|
| クリップボード | `Clipboard`/`DataPackage` | `NSPasteboard` | `Gdk::Clipboard` | `arboard`クレート経由 |
| ファイルダイアログ | `FileOpenPicker`/`FileSavePicker` | `NSOpenPanel`/`NSSavePanel` | `gtk::FileChooserNative` | `rfd`クレート経由 |
| D&D | `DragDrop`イベント | `NSDraggingDestination` | `Gtk::DropTarget` | Canvas内ヒットテストで独自実装 |

### 8.8 モバイル対応(iOS / Android)

§3.3の`Backend` enumに`Uikit`(iOS)/`Jetpack`(Android)を追加し、既存バックエンド抽象化をそのまま拡張する。バリアント追加に伴う既存ビルトインの網羅性エラー(§3.3参照)は、各`builtin`定義に対応する`native!`腕を追加することで解消する。

- **画面サイズ・向き・セーフエリア**: 実行中に変化しうる値であるため`env::*`を拡張せず、§7.1と同じ`store`の仕組みを使ったビルトインStoreとして提供する(`store platform::Device { orientation, safe_area, window_size }`)。参照は通常のstoreと同じく`bind!`経由必須(14章ルール13)
- **セーフエリアのレイアウト反映**: `Window`ビルトインは既定で`respects_safe_area: true`を持ち、§5.1のレイアウトエンジンがセーフエリアを差し引いて利用可能領域を計算する
- **タッチジェスチャー**: `on_swipe`/`on_pinch`/`on_long_press`を任意のビルトイン要素の共通属性として一般化(§5.4の`on_pointer_down`等の拡張)。デスクトップ系backendはマウス操作からの近似にフォールバック
- **OSレベルライフサイクル**: §6.2参照
- **DPI対応**: 論理ピクセル座標の方針(§5.4)を継承。`Image::asset("icon")`がDPI別バリアント(`icon@1x/@2x/@3x.png`)を実行環境のスケールファクタから自動解決
- **パーミッション**: `platform::permissions::request(Permission::Camera).await`が直接`PermissionStatus`を返す(§8.7の`platform::`名前空間+§7.3の非同期パターンの組み合わせ)

---

## 9. テスト支援

§7.2の`viewmodel`単体テスト(バックエンド非起動)に加え、`view`が組み立てる要素ツリー・描画結果を検証するスナップショットテストを提供する。

| テスト対象 | 手段 |
|---|---|
| ビジネスロジック・Commandの振る舞い | 通常の`#[test]` + `viewmodel`の直接操作(バックエンド起動不要) |
| 要素ツリーの構造(レイアウト・分岐結果) | `elwindui_test::render_tree` + `assert_snapshot!`(`insta`クレート等の慣習に合わせる) |
| Canvas等のピクセル単位の描画結果 | `elwindui_test::render_canvas_snapshot` + `assert_image_snapshot!` |

`render_canvas_snapshot`はツール設計書側のオフスクリーンレンダリング機能(プレビュー用)を再利用する。新しいBackend variant(テスト専用等)は追加せず既存backendのヘッドレスモードを用いることで、`match target::backend()`の網羅性検査(§2.3・§3.3)に影響を与えない設計になっている。

---

## 10. 静的検証ルール一覧(14章)と機能対応表

コンパイラ/リンタが実行前に検出すべき項目(ツール側の実装対象だが、各ルールがフレームワークのどの不変条件を守っているかは設計上重要なため一覧化する)。

| # | ルール概要 | 関連する本ドキュメントの節 |
|---|---|---|
| 1 | `#[param]`初期化式に`bind!`/propの参照/`#[computed]`が出現 → エラー | §2.2 |
| 2 | `#[param]`初期化式に非純粋関数(`now()`,`random()`等)が出現 → エラー(`env::*`/`once`は例外) | §2.2, §2.7, §5.4(`#[animated]`が例外) |
| 3 | `#[computed]`フィールドへの外部代入 → エラー | §2.2 |
| 4 | enum値の裸文字列直書き(完全修飾でない参照) → エラー | §2.6 |
| 5 | `match`におけるenumメンバーの網羅漏れ → エラー | §2.3 |
| 6 | 制約付きフィールドへの制約違反代入 → ビルド時/実行時エラー | §2.5 |
| 7 | `external::*`呼び出しが`once`宣言以外に出現 → エラー | §2.7 |
| 8 | importの循環・未解決パス → エラー | §2.10 |
| 9 | `#[overrides(builtin::X)]`のない通常componentに`native!`/`target::backend()`出現 → エラー | §3.4, §4.1 |
| 10 | `Canvas`を含む`view`に`#[accessible(...)]`なし → 警告 | §5.3, §5.4 |
| 11 | ライフサイクルフック外での`#[param]`相当の再代入 → エラー | §6.1 |
| 12 | `bind!`参照先が`store`宣言に存在しない → エラー | §7.1 |
| 13 | `store`/ViewModel/ビルトインStoreフィールドへの`#[param]`側からの直接参照 → エラー | §7.1, §7.2, §8.8 |
| 14 | `NavigationHost`内`match route`の非網羅 → エラー | §8.2 |
| 15 | ダイアログ/メニュー等オーバーレイ系ビルトイン外での`native!`/`target::backend()` → エラー | §8.3 |
| 16 | `Transition`/`KeyframeAnimation`の不正イージング名・範囲外キーフレーム → エラー | §5.5 |
| 17 | `Effect`のバックエンド非対応組み合わせ → 警告(フォールバック明示) | §5.5 |
| 18 | `#[command]`付与フィールドの型が`Command`でない → エラー | §7.2 |
| 19 | `viewmodel`定義内に`view`ブロック/ビルトイン要素への直接参照 → エラー | §7.2 |
| 20 | `#[async_computed]`/`#[command(async,...)]`が`viewmodel`/`store`以外に付与 → エラー | §7.3 |
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
| レイアウト計算(Measure/Arrange)の一元実装 | `elwindui-core`(§5.1) |
| ViewModelの依存関係静的抽出結果に基づく`Cell`/`RefCell`ベースの実行時更新 | コンパイラが生成するコード + `elwindui-core`の汎用リアクティブグラフ(フォールバック時のみ) |
| ネイティブAPIへの橋渡し(WinUI3/AppKit/GTK4/Uikit/Jetpack/egui/iced) | 各`elwindui-backend-*`クレート |
| コード生成・LSP診断・プレビュー・ホットリロード | ツール設計書(`docs/elwindui_tool_*_design.md`)側の対象 |

この分担の要点: **DSLの文法自体は「`Element`ツリーへの到達可能性」「param/propの静的/動的区別」「enum網羅性」といった契約のみを保証し、探索アルゴリズムやレイアウト計算・リアクティブ更新の実装詳細はすべて`elwindui-core`側の関心事として分離されている**。これにより、将来的にレイアウトエンジンの実装を差し替えたり新しいバックエンドを追加したりしても、DSLの構文自体は変更不要という設計になっている。
