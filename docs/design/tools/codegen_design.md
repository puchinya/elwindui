# ElwindUIL コード生成ツール(`elwindui-codegen`)設計書

本書は、ElwindUILの`component`/`view`/`viewmodel`/`enum`定義をRustソースコードへ変換するコンパイラ本体(`elwindui-codegen`)の設計を定める。DSL構文そのものは`docs/specs/dsl_spec.md`、バックエンド抽象化・ランタイム等のフレームワーク設計は`docs/design/gui_framework_design.md`を正とし、本書では「コンパイラというツール」の入出力・内部パイプライン・起動方式・実装トレードオフに焦点を当てる。

入力は`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`という3つのRustプロシージャルマクロに限られる。`parser.rs`が解釈するテキスト構文は`parser.rs`/`validate.rs`/`codegen.rs`のテストフィクスチャ形式として内部に残るが、**サポートされる入力形式ではない**(`#[cfg(test)]`・`pub(crate)`で本番コードからは呼べない)。

ElwindUILツールチェーン全体(本書・LSP・プレビュー・ホットリロード)のアーキテクチャ概観は§7を参照。

---

## 1. スコープ

### 1.1 責務

- `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`が受け取るRustトークン列(`syn::ItemStruct`/`syn::ItemMod`/`syn::ItemEnum`)からの共通AST構築
- `docs/specs/dsl_spec.md`1〜14章・付録Aに定義された静的検証(13章の検証ルール一覧)の実行
- `#![backend(...)]` 指定・ビルドターゲットに基づく `target::backend()` の定数畳み込みと、非該当バックエンド分岐の静的除去(設計のみ、**未実装** — 下記実装状況の注を参照)
- バックエンド(WinUI 3/AppKit/GTK4)向けRustソースの生成
- 3つのプロシージャルマクロ(`component`/`viewmodel`/`dsl_enum`)への入力フロントエンドの提供

**実装状況の注**: 現時点の`elwindui-codegen`(`crates/elwindui-codegen/src/`)には`enum Backend`・`target::backend()`・`#![backend(...)]`属性のいずれも実装されていない。生成されるRustソースはバックエンド非依存(同一のソースがどのバックエンドクレートにもリンクできる)であり、実際にどのバックエンドを使うかは`elwindui`ファサードクレートのCargoフィーチャ(`backend-appkit`/`backend-winui3`/`backend-gtk4`)がどの`elwindui-backend-*`クレートをリンクするかだけで決まる(各バックエンドクレートが同名のビルトイン型を実装するため、コード生成器側でバックエンドごとに分岐する必要がない)。`docs/design/gui_framework_design.md`§3.3が定義する定数畳み込み・分岐除去の仕組みは将来のフォワードルッキング設計であり、以下の記述はその設計を示すものである。

### 1.2 非責務(他ツールが担当)

- エディタ内の増分診断・ホバー表示・補完 → `docs/design/tools/languageserver_design.md`(`elwindui-languageserver` LSP)
- 静的/インタラクティブプレビューのレンダリング → `docs/design/tools/preview_design.md`
- 実行中プロセスへの差分反映(dylib差し替え) → `docs/design/tools/hotreload_design.md`
- 言語仕様の意味論そのもの(param/prop/stateの区別、Element契約、Once/OneWay/TwoWay代入等) → `docs/design/gui_framework_design.md`

`elwindui-languageserver` はエディタ内診断のために `elwindui-codegen` と共通のパーサ・検証ロジックを再利用する想定だが、LSPプロセスとしての振る舞い(増分パース、プレビュー用インスタンス生成)は本書の対象外とする。

### 1.3 実装原則: イベント名・ペイロード型のハードコード禁止

`elwindui-codegen`は「AST(DSLが宣言した型情報)から機械的にRustコードを生成する」コンパイラであり、フレームワーク固有の属性名やペイロード型をコンパイラ自身に決め打ちで埋め込んではならない。特に`#[routed]`(4章、ルーティングイベント)のペイロード型は、常にそのDSLフィールド自身の`fn(T0, T1, ...)`宣言(`ast::Attr::Routed`)から`callback_param_types`で機械的に導出する——`codegen.rs`側に「このイベント名にはこの型」という固定テーブルを持たせてはならない。

新しいルーティングイベントを追加する際の正しい手順は、対象コンポーネント(多くの場合、全コンポーネントに継承される共通`UIElement`)へ`#[routed] on_x: fn(PayloadType)`という実フィールドを`crates/elwindui-core/src/ui.rs`の`#[elwindui_macros::class]`宣言に足すことであり、`emit_wiring`等のコード生成側に`on_x`という名前を直接書き足すことではない。`Button.on_click`は現在も例外的にコンパイラ側の属性名マッチ(`emit_generic_on_click_routing`)で処理されている歴史的経緯があるが、これは新規追加すべきパターンではなく、将来的にはこの原則に沿って`UIElement`側の宣言フィールドへ統合されるべき負債として扱う。

---

## 2. 入出力

| | 内容 |
|---|---|
| 入力 | `#[elwindui::component(inherits Base)] struct Name { .. }`/`#[elwindui::viewmodel] mod foo { .. }`/`#[elwindui::dsl_enum] enum Name { .. }`(いずれもRustトークン列、`elwindui-macros`がproc-macro展開時に受け取る) |
| 出力 | コンパイル時に直接展開されるRustトークン列(バックエンド非依存) |
| 副作用 | 無し。中間ファイルを生成せず、マクロ展開位置にトークン列を直接埋め込む |

出力はproc-macro展開結果としてその場に埋め込まれるため、呼び出し側が別途取り込む操作は不要。

---

## 3. パイプライン

```
#[elwindui::component]/#[elwindui::viewmodel]/#[elwindui::dsl_enum]のRustトークン列
   │ ① フロントエンド変換(component_frontend.rs/attr_frontend.rs)
   ▼
共通AST(フレームワーク非依存の要素ツリー)
   │ ② 静的検証(言語仕様13章のルール一覧)
   ▼
検証済みAST
   │ ③ (設計上)target::backend() の定数畳み込み・非該当分岐の除去 ※未実装、下記参照
   ▼
バックエンド非依存AST
   │ ④ Rustコード生成(バックエンド非依存)
   ▼
Rustソース(WinUI3/AppKit/GTK4のいずれのバックエンドクレートにもリンク可能)
```

- **①フロントエンド変換**: `syn`で解析済みのRustトークン列(`ItemStruct`/`ItemMod`/`ItemEnum`)から共通ASTを構築する(`component_frontend::component_and_view_from_item_struct`/`attr_frontend::viewmodel_def_from_item_mod`/`component_frontend::enum_def_from_item_enum`)。`view! { .. }`型フィールドの中身だけは生のDSLテキストとして`parser::parse_view_body`にかけられる(§4.2参照)。他の同一クレート内`component`/`viewmodel`/`enum`の解決は、宣言順に populate される同一クレート内レジストリ(`component_frontend::same_crate_components`等)経由で行われる — `use`宣言によるモジュール解決自体は生成コードがRustコードである以上、最終的にはRustコンパイラに委譲される(§4.2脚注参照)。
- **②静的検証**: 言語仕様13章に列挙された検証ルール(`#[param]`静的評価式、`#[state]`競合、`<=>`の書き込み可能RHS、enum網羅性、制約違反、`native!`の出現位置など)をASTに対して実行する。違反はビルド時エラーとしてコンパイルを停止させる。
- **③定数畳み込み(未実装)**: `docs/design/gui_framework_design.md`§3.3は、`target::backend()`をビルド設定(Cargoのfeature/target triple)から一意に確定し、該当しない `match target::backend() { ... }` の腕や `#[cfg(backend = "...")]` 付き `native!` ブロックを生成対象から静的に除去する設計を定めているが、現在の`elwindui-codegen`にはこの段階が存在しない(`enum Backend`/`target::backend()`はコード中どこにも実装されていない)。実際には生成コードはバックエンドを問わず同一であり、この段階は素通りする。
- **④コード生成**: 検証済みASTから、バックエンドを問わず同一のRustコードを生成する。ビルトイン要素(`builtin::Window`/`Row`/`Text`等、`docs/specs/builtins_spec.md`付録F)は他コンポーネントと同じ`component`/`view`構文で書かれたリファレンス実装として同一パイプラインで処理される。生成コードが実際にどのバックエンドで動くかは、リンクされる`elwindui-backend-*`クレート(各バックエンドクレートが同名のビルトイン型を実装している)によって決まる——`docs/design/gui_framework_design.md`§1・§3が想定する「バックエンドごとに異なるコードを生成する」段階は現状ここには存在しない。

動的`for`内の`property <=> item.field`は、安定したRc identityと解決可能な可変fieldを検証した後、通常の属性初期setter・item→widgetの`Subscription`・widget→itemの型付き`set_on_<property>_change`を生成する。どちらの購読もitemごとの`DynamicChild`が所有するため、`DynamicChildSlot::replace_rc_items`でitemが削除・置換されると一緒にDropされる。汎用Bindingオブジェクトやバックエンド別の経路は増やさない。

---

## 4. 起動方式

コード生成器はプロシージャルマクロとしてのみ呼び出される。

### 4.1 `component`/`viewmodel`/`dsl_enum`

`component`/`view`を通常のRust `struct`定義として書く。フィールドは`#[param]`/`#[prop]`等の属性を
伴う通常のフィールドとして、`view { .. }`要素ツリーは`view!`マクロ呼び出しを型に持つ1フィールド
として表現する(このフィールドは省略可能 — view無しコンポーネントになる)。

```rust
#[elwindui::component(inherits Window)]
struct NotepadWindow {
    #[bindable]
    vm: std::rc::Rc<NotepadViewModel>,

    body: view! {
        title: vm.window_title
        content: VerticalLayout {
            TextArea { text <=> vm.content }
        }
    }
}
```

- `view!`は実在するマクロではなく、一度も展開されない。`#[elwindui::component]`(属性マクロ)が
  `struct`全体を丸ごと別のコードへ置き換えるため、内側の`view!`呼び出しはRustが実際に展開する
  対象には現れない ── マクロ呼び出しがRustの*型*位置において構文的に妥当(`field: some_macro! {
  .. }`は`syn::Type::Macro`としてパースされる)であることを利用したトリック。`view!`のトークンは
  生のDSLテキストとして読み出され、既存のパーサ(`crates/elwindui-codegen/
  src/parser.rs`)へそのままかけられる。
- 中間ファイルを生成せず、コンパイル時にトークン列として直接展開する。`view!`の中身自体は、
  rust-analyzer自身による補完・型チェックの対象にはならない(`elwindui-languageserver`側で
  `vm.field`補完のみ別途提供、`docs/design/tools/languageserver_design.md`参照)。
- viewmodelは`#[elwindui::viewmodel] mod foo { struct Foo { .. } impl Foo { .. } }`という、`mod`で
  `struct`+`impl`を束ねた形で書く(1回のマクロ展開が受け取れる項目は1つだけなので、両方を一緒に
  渡すために`mod`で包む — 展開後は`mod`自体は消え、中の`struct`/`impl`がそのまま元の位置に現れる)。
  `impl`ブロックの`fn`/`async fn`はすべて自動的にviewmodelアクションとして検出される。
- 同一クレート内の他の`component`/`viewmodel`/`enum`を`view!`から参照する(フィールド型/
  `match`)には、それらが**このアイテムより前に**宣言されている必要がある — 各マクロ展開は自分の
  同一クレート内レジストリ(`component_frontend::same_crate_components`/`same_crate_viewmodels`/
  `same_crate_enums`)に自分自身を登録するため、後方参照は解決できない(宣言順依存)。
- プレーンなRust `enum`は、`#[elwindui::dsl_enum]`を付けない限り`view!`の`match`網羅性検査から
  見えない(nothing about a bare `enum` marks it as DSL-relevant to any proc-macro) —
  `#[elwindui::dsl_enum] enum Name { A, B, C }`は本体を無変更のまま透過しつつ、上記のレジストリへ
  登録するopt-inの属性。

いずれの入力形式でも②〜④のパイプライン(静的検証・定数畳み込み・コード生成)は共通の内部実装
(`elwindui-codegen`本体)を呼び出すのみとし、`component`/`viewmodel`/`dsl_enum`いずれから来た
アイテムかによってコンパイラの検証結果や生成コードの意味が変わることはない。

**実装状況の注**: `elwindui_macros::component`/`elwindui_macros::viewmodel`/`elwindui_macros::
dsl_enum`(`crates/elwindui-macros/src/lib.rs`、`elwindui::component`/`elwindui::viewmodel`/
`elwindui::dsl_enum`として再エクスポート)として実装され、ワークスペース内の全example(`notepad`
含む)が利用する。`component`は`struct`に付与する属性マクロで、`view`要素ツリーは`view!`型
フィールド(`crates/elwindui-codegen/src/component_frontend.rs`が処理)として書く。`component`/
`viewmodel`と同系統のもう1つのRust代替記法として、単一の`fn`に付与する`#[elwindui::template]`
(再利用可能な名前付き`ControlTemplate<Self>`定義、`docs/specs/dsl_spec.md`§4「`#[elwindui::
template]`」参照)が設計されている——**設計のみ・未実装**。

---

## 5. 他ツールとの連携点

- **`elwindui-languageserver`(LSP)**: エディタ内診断のため、本コンパイラのパーサ・検証ロジック(①②)を共有ライブラリとして呼び出す想定。ただし増分パースやプレビュー用インスタンス生成はLSP側の責務であり、本書では扱わない。
- **プレビューツール**: 静的プレビュー(`docs/design/tools/preview_design.md`のレベル①)は「componentを既定値でインスタンス化しオフスクリーンレンダリングする」処理であり、コンパイラが生成したコード(またはLSPが保持する検証済みAST)を利用する。生成コード自体の変更は不要。
- **ホットリロードツール**: `#[param]`変更時は再マウント、prop変更のみの場合は差分更新という区別(`docs/design/tools/hotreload_design.md`)は、コンパイラが出力するコードが`param`/`prop`の区別を保ったまま生成されていることが前提となる。コンパイラ側で両者を混同しないコード生成を保証する。

---

## 6. ツールチェーン全体アーキテクチャ

ソースファイルの保存から、エディタ内診断・プレビュー・実行中アプリへの反映までを横断する、ElwindUILツールチェーン全体の構成。

```
┌──────────────────────────────────────────────┐
│ エディタ(VSCode等)                             │
│  ┌──────────────┐  ┌─────────────────────────┐ │
│  │ .rsエディタ   │  │ プレビューパネル(WebView) │ │
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

本コンパイラ(`elwindui-codegen`)自体はこの図の中心には現れないが、LSPが再利用するフロントエンド変換・静的検証ロジック(§3の①②)、および実際の`cargo build`時に`component`/`viewmodel`/`dsl_enum`をRustコードへ変換する処理の両方を提供し、上記の全経路の土台になっている。ここに挙げるツール群(本書・LSP・プレビュー・ホットリロード)はいずれも、DSLの言語仕様(`component`/`view`/`param`/`prop`/`Element`トレイト等、`docs/specs/dsl_spec.md`・`docs/design/gui_framework_design.md`)自体を変更せずに構築できるツールチェーン層として位置づける。

---

## 7. まとめ

| 要件 | 対応 |
|---|---|
| `component`/`viewmodel`/`dsl_enum` → Rust変換 | フロントエンド変換→共通AST→静的検証→定数畳み込み→バックエンド別コード生成の4段パイプライン |
| 起動方式 | `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`プロシージャルマクロのみ |
| 静的検証 | 言語仕様13章のルール一覧をASTに対して実行し、違反はビルド時エラー |
| バックエンド分岐の除去 | `target::backend()`の定数畳み込みにより非該当分岐を静的除去(**未実装**。現状は生成コードがバックエンド非依存で、リンクする`elwindui-backend-*`クレートの選択のみでバックエンドが決まる) |
| 他ツールとの関係 | LanguageServer/preview/hotreloadは本コンパイラの解析結果・生成コードを利用する側であり、検証ロジックの二重実装を避ける |
