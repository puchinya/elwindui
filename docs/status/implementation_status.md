# ElwindUIL 実装状況

`docs/specs/`・`docs/design/`配下は仕様書/設計書であり、将来実装予定のフォワードルッキングな内容を含む。本ドキュメントは「実際に`crates/`配下に何が実装済みで、何が未着手か」を横断的に一覧化したもの。

実装は日々変化する。内容が古いと思われる場合は`crates/`を直接確認すること。ドキュメント全体の構成と読み方は`docs/README.md`を参照。

**バッジの意味**: ✅ 実装済み(少なくとも1バックエンドで実機検証済み) / 🚧 部分実装 / 📋 仕様のみ(コード無し)
**バックエンド列**: ✅ 実装・実機検証済み / ⚠️ 実装コードはあるが未検証 / ✖ 未着手

---

## 1. クレート別実装状況

| クレート | 行数 | 状況 |
|---|---:|---|
| `elwindui-core` | 13,620 | ✅ `UIElement`クラス階層(`#[elwindui_macros::class]`)、WinUI3準拠のMeasure/Arrange(`measure`/`arrange`/`measure_override`/`arrange_override`)、retained `RenderTree`/`RenderContext`、ルーティングイベント(`dispatch_routed`/`dispatch_direct`/`hit_test`、`ClipToBounds`/透明背景パススルー/`IsHitTestVisible`対応)、ポインタ/タップ入力(`input::PointerDispatcher`)、キーボード/フォーカス入力(`input::KeyboardDispatcher`/`input::ShortcutRegistry`、`focus::FocusTracker`、`UIElementExt::focus()`/`FocusHost`)が実働。`graphics`モジュールは`Color`/`Brush`(単色・線形/放射グラデーション・画像)/`StrokeStyle`/`Path`・`PathBuilder`(cubic正規化、`contains`/`stroked_contains`はwinding-number/線分距離判定、真偽演算`combine`は`flo_curves`)/`Image`/`RenderCommand`(Fill/Stroke×Rect・RoundedRect・Ellipse、DrawLine、Fill/StrokePath、DrawImage、Text、Push/Pop×Clip・Transform・Opacity)。SVGベクター型は`graphics/{vector_image,vector_scene,vector_filter}.rs`に`VectorImage`/`VectorImageBuilder`/`VectorGroup`/`VectorNode`/`VectorPathNode`/`VectorRasterNode`/`VectorPaint`/`VectorPattern`/`VectorClipPath`/`VectorMask`/`VectorFilter`(17種filter primitive全型)/`ImageSource`(`Raster`/`Vector`)/`RenderCommand::DrawVectorImage`。usvg/SVGファイル形式には一切依存しない。`AccessibilityNode`トレイトは**型定義のみ**で、テスト用ダミー以外の実装が存在しない |
| `elwindui-svg` | 1,769 | ✅ `usvg 0.47`ベースのSVGローダー(`SvgLoader`/`load_svg_file`/`load_svg_bytes`/`load_svg_str`)。usvgの静的機能(path/gradient/pattern/clipPath/mask/filter/text(グリフをpath化)/nested SVG/埋め込みラスター/data URL/SVGZ)を`elwindui_core::graphics::VectorImage`へ変換。リソース解決ポリシー(`SvgResourcePolicy::DenyExternal/DataUrlsOnly/SameDirectory/Custom`、パストラバーサル・シンボリックリンク脱出防止)、`SvgLimits`(ノード数/path command数/group深さ/filter primitive数/埋め込み画像バイト数/外部リソース数/nested SVG深さ/圧縮後バイト数の上限、SVGZ decompression bomb対策)。SVGパースは`elwindui-core`から隔離されており、バックエンドは`VectorImage`のみを参照する |
| `elwindui-codegen` | 18,161 | ✅ コンパイラ本体。`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`プロシージャルマクロが唯一の入力経路。`#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }`という、`component`+`view`ペアを1つのRust `struct`として書く形式(`component_frontend.rs`)——`view!`は実在するマクロではなく、DSLテキストとして読み出される型位置マクロ呼び出し。`view!`フィールドは省略可能(view無しコンポーネント)、component単位属性(`#[embedded]`/`#[sealed]`/`#[native]`/`#[abstract_]`/`#[text_style]`/`#[content(field)]`)・`#[param(default = ...)]`もこの形式で書ける。ビルトイン25個の形状は`elwindui-core::ui`/各backendの`#[elwindui_macros::class]`宣言そのものが正であり、`__elwindui_shape_{Name}!`という`#[class]`生成のmacro_rulesの遅延合成経由でクレート境界を越えて伝播する(専用のシンボルテーブルを持たない)。`parser::parse_module`(トップレベル`component`/`view`/`viewmodel`/`enum`/`use`のテキスト構文)は`#[cfg(test)]`・`pub(crate)`でテストフィクスチャ専用として残る(本番コードからは呼べない)。`view!`本体を解析する`parse_view_body`/`parse_initializer`は本番APIのまま。`#[elwindui::dsl_enum] enum Name { .. }`はプレーンなRust enumを`view!`のmatch網羅性検査に載せるopt-in属性で、enum本体はそのまま透過する |
| `elwindui-macros` | 4,057 | ✅ `#[class(inherits/implements/supertrait/abstract_class/sealed)]` + `#[inherent]`/`#[ancestor]`によるクラス階層生成マクロ。仕様は`docs/specs/macro_class_spec.md`が正。各`{ClassName}Ext`トレイト(root/ordinary/`trait_only`いずれの宣言でも)に`into_<name>_node(self: Rc<Self>) -> Rc<dyn {ClassName}Ext>`というアップキャストのデフォルトメソッドが生成される。祖先ごとにその祖先自身のトレイト宣言側で1つ生成され、Rustの通常のデフォルトメソッド継承(supertraitチェーン経由)で子孫に伝播する。`UIElement`を祖先に持たないクラス(`MenuItem`/`Window`等、`trait_only`かつ`inherits`無し)には`into_ui_element_node`が生成されず、「汎用UIElement子として埋め込めない」制約が型として効く |
| `elwindui-i18n` | 65 | 🚧 Fluentベースのランタイム(`t!`, `declare!`マクロ)は実働。ビルド時の`.ftl`静的検証(未翻訳キー検出・引数名整合性チェック)は`elwindui-codegen`側に存在せず未実装 |
| `elwindui-languageserver` | 1,095 | 🚧 単一`.rs`ファイル単位のモデル(`component_frontend::modules_from_file`が`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`を実マクロ展開せずに読み出す)。診断(`elwindui-codegen`の`component_frontend`/`validate`を再利用、`syn::parse_file`失敗時は実際の行・列付き)、メンバー補完(`vm.field`)が実働。シンタックスハイライト(semantic tokens)は`view! { .. }`マクロ本体だけに限定して提供(`src/semantic_tokens.rs`、`proc-macro2`の`span-locations`で実ソース上のバイト範囲を特定しその範囲内だけトークナイズ——rust-analyzer自身のRustハイライトとは非重複)。ディレクトリ横断のクロスファイル解決は無い——実マクロ展開経路自体が宣言順に依存する同一クレート内レジストリにしか頼っておらず、実コンパイルを行わないLSPには再現不能なため。hover、生成コードプレビュー、オフスクリーンレンダリング連携のインスタンス生成パイプラインは未実装 |
| `elwindui-hotreload` | 32 | 📋 スタブのみ。`param`/`prop`差分からremount/patchを判定する純粋関数(`decide_reload_action`)だけが存在し、`hot-lib-reloader`統合・実際のdylib差し替えは未実装 |
| `elwindui-test` | 48 | 🚧 `render_tree`(`UIElement`ツリーの、各ノードを`type_name()`でラベル付けしたインデントダンプ)のみ実装。`render_canvas_snapshot`/`assert_image_snapshot!`は未実装(`canvas.rs`はdocコメントのみのスタブ) |
| `elwindui-backend-appkit` | 10,017 | ✅ 本機で`cargo build`/実行/スクリーンショット確認済みの唯一のバックエンド。描画replay(`host/replay.rs`の`replay_group`/`replay_commands`)は`RenderCommand`ごとに`CAShapeLayer`(fill/stroke/dash/cap/join/miter/nonzero-evenodd)・`CAGradientLayer`(`try_add_gradient_fill_layer`)・`CATextLayer`・画像用`CALayer`を組み立てるCALayer合成方式(`NSView.draw(_:)`+`CGContext`直接記述ではない)。clipは`CAShapeLayer`マスクによる実パス形状単位(`clip_mask_layer`)。SVGベクター描画(`render/vector/`)は`RenderCommand::DrawVectorImage`をフル実装: group transform/opacity/clip/mask(alpha・luminance、オフスクリーンraster化)/blend-mode(`CALayer.compositingFilter`+Core Image blend filter)/filter graph(Core Imageへマッピング)/path塗り(単色・グラデーション、任意の回転/拡大縮小変換下でも正しく動作)/pattern(塗り対象の境界を覆うタイル格子を計算する無限タイリング、`render/paint.rs`の`add_tiled_image_layers`と同じ技法を回転/拡大縮小対応へ一般化)/埋め込みラスター画像。SVG読み込み(`elwindui-svg`/`usvg`)への依存はproduction経路に無い(dev-dependency経由のgolden testのみ)。オフスクリーン`CGBitmapContext`+`CALayer.renderInContext`によるgolden-imageテスト(`testsupport/golden.rs`、`testsupport/svg_golden.rs`は`resvg`参照描画とのサンプル点比較) |
| `elwindui-backend-winui3` | 8,517 | ⚠️ 実装コードあり。appkitと同じ層構成を持つ。Win2Dコマンドリスト(`render/win2d.rs`)+`Microsoft.UI.Composition`のretainedレンダラ(`render/composition/`)+SVGベクター描画(`render/vector.rs`)。`build.rs`が`windows-bindgen`でWinRT projectionを生成し、`cpp/app_host.cpp`(C++/WinRT)が`Application`のcomposable-class集約を担う(windows-rsが未対応のため、microsoft/windows-rs#3404)。フォント/テキストスタイル経路はWindows上でビルドおよび単一Applicationホスト回帰テストを完了している。**その他の機能の実機検証は未完了** |
| `elwindui-backend-gtk4` | 19 | ✖ `src/lib.rs`が19行のスタブ(`init()`と、`startup`をそのまま呼ぶだけの`application::run`)のみ。`native_ui`/`inner`/`host`/`render`/`platform`は存在せず、`gtk4` crateへの依存も無い。`RenderCommand`を扱うコード自体が無い |
| `elwindui`(ファサード) | 139 | ✅ Cargoフィーチャ`backend-appkit`/`backend-winui3`/`backend-gtk4`で`core`/`i18n`/`backend`/`ui`を再エクスポートする。`svg`フィーチャで`elwindui::svg`として`elwindui-svg`を再エクスポート。`default = []` |
| プレビューツール | — | 📋 **ワークスペースに存在しない**。`docs/design/tools/preview_design.md`は100%未着手のフォワードルッキング設計 |

---

## 2. サンプルアプリ(`examples/`)

| 名前 | 行数 | 用途 |
|---|---:|---|
| `graphics-demo` | 846 | `elwindui_core::graphics`の標準ビジュアル検証ツール。`TabView`で機能領域ごとにタブを分ける。`graphics`変更時は再実行してスクリーンショットを確認する |
| `notepad` | 384 | MVVM構成(`viewmodel`・`spawn_local`・`platform::file_dialog`・`MenuBar`)の総合サンプル |
| `theme-demo` | 371 | テーマ/デザイントークン、variant/appearance切り替えの検証 |
| `controls-demo` | 257 | `TextBox`/`PasswordBox`/`ScrollView`などのネイティブコントロール検証 |
| `font-demo` | 198 | フォント/テキストスタイルの継承と解決順序の検証 |
| `viewmodel-attr-demo` | 44 | `#[elwindui::viewmodel]`の最小サンプル |

---

## 3. バックエンド対応状況

| バックエンド | 状況 |
|---|---|
| AppKit(macOS) | ✅ 実装・実機検証済み |
| WinUI3(Windows) | ⚠️ 実装コードあり。フォント/テキストスタイルはWindowsでビルド・実行テスト済み、他機能の実機検証は未完了 |
| GTK4(Linux) | ✖ 未着手(19行のスタブのみ) |
| UIKit(iOS)/Jetpack(Android) | 📋 設計のみ、コード無し(`docs/design/gui_framework_design.md` §8.8) |

バックエンド候補は上記のネイティブ3種(+将来のモバイル2種)のみ。

**設計と実装の乖離**: `docs/design/gui_framework_design.md` §3.3が説明する`enum Backend` + `target::backend()`(コンパイル時定数、`match`網羅性検査による新バックエンド追加時の安全弁)は**コード中のどこにも実体が存在しない**。実際のバックエンド選択は`elwindui`ファサードクレートのCargoフィーチャフラグによる`#[cfg(feature = ...)]`のみで行われる。これに伴い、`native!`/`match target::backend()`をビルトイン限定にする静的検証ルール9、`NavigationHost`の`Route`網羅性ルール14、オーバーレイ系ビルトインの分岐制限ルール15も、前提となる仕組みが無いため検証しようがない。

---

## 4. 機能 × バックエンド マトリクス

| 機能 | AppKit | WinUI3 | GTK4 | 仕様/設計 |
|---|:---:|:---:|:---:|---|
| `Window` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.1 |
| `VerticalLayout` / `HorizontalLayout` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.2 |
| `TextBlock` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.3 |
| `TextArea` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.4 |
| `Dropdown` / `DropdownItem` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.5 |
| `Rectangle` / `Ellipse` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.6 |
| `Control` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.9 |
| `ContentControl` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.10 |
| `Grid` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.11 |
| `TextBox` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.12 |
| `PasswordBox` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.13 |
| `ScrollView` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.14 |
| `Button` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.15 |
| `CheckBox` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.16 |
| `RadioButton` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.17 |
| `ToggleSwitch` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.18 |
| `Slider` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` F.19 |
| `Image`(ラスター/ベクター) | ✅ | ✖ | ✖ | **仕様書に節が無い**(§5・§7参照) |
| `MenuBar` / `MenuBarItem` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` 付録X |
| `Menu` / `MenuItem` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` 付録M.2 |
| `TabView` / `TabViewItem` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` 付録Y |
| `Canvas` / `Painter` | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録G |
| 描画拡張(Brush/Geometry/Effect/Transform) | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録N |
| `NavigationHost` / `Route` | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録L |
| `Dialog` | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録M.1 |
| `tooltip`属性(`NativeControl`派生の全ネイティブ葉) | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` 付録M.3 |
| `tooltip`属性(自前描画要素) / 汎用`context_menu`属性 | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録M.2/M.3 |
| `VirtualList` | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録Q |
| `platform::file_dialog` | ✅ | ⚠️ | ✖ | `docs/specs/builtins_spec.md` 付録T.2 |
| `platform::clipboard` | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録T.1 |
| ドラッグ&ドロップ | ✖ | ✖ | ✖ | `docs/specs/builtins_spec.md` 付録T.3 |
| SVGベクター描画 | ✅ | ✖ | ✖ | 本ドキュメント §7 |
| フォント/テキストスタイル | ✅ | ⚠️ | ✖ | `docs/status/font_status.md` |
| テーマ/デザイントークン | ✅ | ✅ | ✖ | `docs/status/theme_status.md` |
| ポインタ/タップ入力 | ✅ | ⚠️ | ✖ | `docs/design/gui_framework_design.md` §5.10 |
| キーボード/フォーカス | ✅ | ⚠️ | ✖ | `docs/design/gui_framework_design.md` §5.5 / §8.1 |
| アクセシビリティ | ✖ | ✖ | ✖ | `docs/design/gui_framework_design.md` §5.6 |

`platform::file_dialog`の戻り値は`Option<PathBuf>`のみで、仕様書にあるファイルフィルタ指定引数は無い。

---

## 5. ビルトイン実装状況

`crates/elwindui-core/src/ui.rs`と各`elwindui-backend-*`crateの`#[elwindui_macros::class]`宣言を正とする。詳細な分類ツリーは`docs/specs/builtins_spec.md`冒頭を参照。

**`elwindui-core`が持つバックエンド非依存ビルトイン(12)**: `UIElement` / `Layout`(抽象) / `VerticalLayout` / `HorizontalLayout` / `Shape`(抽象) / `Rectangle` / `Ellipse` / `Image` / `TextBlock` / `Control` / `ContentControl` / `Grid`

**各バックエンドが持つネイティブビルトイン(18)**: `NativeControl`(抽象) / `Window` / `Button` / `CheckBox` / `RadioButton` / `ToggleSwitch` / `Dropdown` / `Slider` / `TextArea` / `TextBox` / `PasswordBox` / `ScrollView` / `MenuBar` / `MenuBarItem` / `Menu` / `MenuItem` / `TabView` / `TabViewItem`

補足:

- `Image`は`Rectangle`/`Ellipse`と同じくバックエンド非依存の自己描画builtin(ネイティブウィジェットを持たない)。`source: Option<ImageSource>`(`Raster(Image)`/`Vector(VectorImage)`)、`stretch: Option<Stretch>`、`rasterize: Option<VectorRasterizeMode>`(`Vector`ソースのみ有効、§7参照)。ヒットテストは`Shape`同様bounding-box精度のみ(`UIElement::hit_test_content`が点を受け取らないシグネチャのため、path形状ベースの精密ヒットテストは別タスク)
- `Menu`/`MenuItem`は`MenuBarItem.submenu`経由での利用のみ実装済み。任意要素に`context_menu`属性で付ける汎用コンテキストメニュー機構は未実装
- `tooltip`共通属性は`NativeControl`に宣言され、そこから派生する全ネイティブ葉で実装済み。自前描画要素(`TextBlock`/`Shape`/レイアウト)では未実装
- `Control`の`template: Option<ControlTemplate<Self>>`(WinUI3の`Control.Template`相当の視覚ツリー実行時差し替え、`docs/specs/builtins_spec.md` 付録F.9.1・`docs/specs/dsl_spec.md` §4・`docs/design/gui_framework_design.md` §5.12)は📋設計のみ。`crates/elwindui-core/src/ui.rs`の`Control`構造体に対応フィールドは無く、`children`をそのままVisual子要素にする挙動のみ実装されている
- `Control`/`TextBlock`/各バックエンドの`NativeControl`は`font_family`/`font_size`/`font_weight`/`font_style`/`font_stretch`/`character_spacing`/`foreground`の7プロパティを持ち(`#[text_style]` DSLコンポーネント属性、`docs/specs/dsl_spec.md` 付録A)、プロパティ単位で独立にVisual Parent経由で継承される。`TextBlock::measure_override`は登録済み`TextBackend`(AppKit・WinUI3実装済み)による実測を行う。詳細は`docs/status/font_status.md`参照

---

## 6. 言語コア機能の実装状況(`docs/specs/dsl_spec.md` §1〜14)

| 機能 | 状況 |
|---|---|
| `component`/`view`分離 | ✅ |
| `param`/`prop`区別(`#[param]`、静的評価式制限) | ✅ |
| 制御構文(`if`/`for`/`match`) | ✅ 子要素位置の`if`/`else`(`else if`チェーン含む)・`match`・`for item in collection`は、親コンポーネント所有の透明な動的子範囲として`#[content(...)]`コレクションへ直接insert/removeする。各範囲は前後の静的子要素と他の動的範囲を保持する。`for Vec<Rc<T>>`(およびviewmodel要素のリスト)は`Rc::ptr_eq`のidentityで既存itemの子・購読を再利用し、他のcollectionは当該範囲のみを再構築する。`match`はvalidatorがuser enumの非網羅armをエラーにする。`if`/`match`の分岐内へのさらなる`if`/`match`/`for`の入れ子(`else if`含む)にも対応(`for`自身のbodyはリテラル要素のみ——入れ子非対応)。`#[content(...)]`フィールドが単一値型(`ContentControl`/`Window`の`content`等)の場合も`if`/`match`(`for`不可、全分岐が1要素に還元できる場合のみ)を書ける |
| `style{}`(横断的属性適用) | 📋 **未実装**。`elwindui-codegen`のASTに`Style`ノードが存在しない |
| 値制約(`#[range]`/`#[step]`/`#[length]`/`#[pattern]`/`#[format]`/`#[check]`) | 🚧 `#[length]`のみ実装 |
| `enum`(`EnumName::values()`、`#[label(...)]`) | 🚧 `EnumDef`はASTに存在。`values()`/`#[label]`によるi18nラベル付与の実装範囲は個別確認が必要 |
| `env::*` / `once` | 📋 **未実装**。`elwindui-codegen`にDSLキーワードとしての扱いが無い |
| `bind!` | ✅ (`Initializer::Bind`) |
| `viewmodel`アクション | ✅ `#[elwindui::viewmodel]`のRustネイティブ`impl`ブロックの`fn`/`async fn`がそのまま自動検出されアクションになる(`Initializer::Action`、struct側の宣言は不要)。テキスト構文の`viewmodel Name { ... }`にはアクションを宣言する手段が無い(`#[observable]`/`#[computed]`のみ)——アクションが必要な`viewmodel`はRustネイティブ構文を使う |
| `on_*`イベント属性のクロージャ構文(`\|param, ...\| 式`/`{ .. }`) | ✅ 対象フィールドの`fn(T0, T1, ...)`宣言から位置対応でパラメータ型を決める汎用機構(`codegen::emit_wiring`)。0引数ハンドラはベアパスの糖衣(`on_click: vm.save`)も書ける |
| 値計算コールバックがネストした要素を構築する構文(`\|param\| Type { .. }`) | 📋 **未実装**。これに依存する`VirtualList`の`render_item`・`ControlTemplate<Self>`も未実装 |
| `ControlTemplate<Self>`型フィールド・`body: <field>(Self)`・`#[elwindui::template]` | 📋 **未実装** |
| メソッド継承(`#[overridable]`/`#[overrides]`/`base::name(..)`) | ✅ `#[elwindui::component] struct X { .. }` + `#[elwindui::component] impl X { .. }`の2枚組で提供する(`impl`はメソッドが無くても必須——型を生成するのは`impl`側)。`inherits <ユーザー定義コンポーネント>`(完全修飾パスで記述、`docs/specs/dsl_spec.md` §3)経由でエンドツーエンド動作確認済み(`examples/inheritance-demo`)。継承・オーバーライドは仕様どおり1階層(直接の`inherits`先)のみ保証。基底自身が`on_*`配線・bindable・`on_mount`を持つ場合の既知の制約は下記§10参照 |
| i18n(Fluent、`t!`) | 🚧 ランタイム(`elwindui-i18n`)は実装済み。ビルド時の`.ftl`静的検証は未実装 |
| モジュール(`use`) | ✅ 生成先が実際のRustコードのため`use`解決自体はRustコンパイラに委譲される。循環参照・未解決パスの独自の機械的検出は未確認 |
| `visual_tree`モジュール(`get_children_count`/`get_child`/`get_parent`/`find_all`) | ✅ `UIElement::visual_children()`/`parent()`が走査を担う。ランタイム文字列idによる検索(`find_by_id`相当)は`#[id(...)]`(静的アクセサ)と役割が重複するため未提供・提供予定なし |
| 静的検証ルール(全29項目) | 🚧 `crates/elwindui-codegen/src/validate.rs`がルール19(`viewmodel`内`view`参照禁止)を含む多くの言語機能バリデーションを実装。前提機能自体が未実装のルール(9・14・15など`target::backend()`依存、26〜29は`ControlTemplate<Self>`依存)は検証不能。ルール18は`Command`機構が存在しないための欠番 |

---

## 7. UI機能拡張の実装状況

| 機能 | 参照先 | 状況 |
|---|---|---|
| ライフサイクルフック | `docs/design/gui_framework_design.md` §6.1 | 🚧 `on_mount`は実装・結線済み。`on_unmount`はパース・コード生成されるが、`elwindui-core::ui`にツリー離脱(デタッチ)フックが無いため**呼び出されない** |
| `store`(グローバル状態) | `docs/design/gui_framework_design.md` §7.1 | 📋 **未実装**。ASTに`Store`ノードが無い。`ControlTemplate<Self>`の広域既定値もこれに依存する |
| キーボード入力・フォーカス管理 | `docs/design/gui_framework_design.md` §5.5 / §8.1 | 🚧 AppKit・WinUI3両バックエンドで実装(WinUI3は`#![cfg(target_os = "windows")]`ゲートのため本機ではコンパイル確認自体不可)。`#[focus(order/trap)]`という専用DSL属性ではなく`tab_stop`/`focus_order`という共通プロパティとして提供する。自前描画系要素の自動フォーカス移譲(クリックでフォーカス)、方向キーでのフォーカス移動、ネイティブリーフ(`Button`/`TextArea`)自身の`on_key_down`/`on_got_focus`個別配線、IME変換中プレビュー表示は未実装 |
| ナビゲーション(`NavigationHost`/`Route`) | `docs/specs/builtins_spec.md` 付録L | 📋 **未実装** |
| ダイアログ/メニュー/ツールチップ | `docs/specs/builtins_spec.md` 付録M | 🚧 `Menu`/`MenuItem`本体と`tooltip`属性(ネイティブ葉のみ)は実装済み。`Dialog`、自前描画要素の`tooltip`、汎用`context_menu`属性は未実装 |
| 描画拡張(Brush/Geometry/Effect/Transform/レイヤー合成/アニメーション) | `docs/specs/builtins_spec.md` 付録N | 📋 未実装。`Painter`基本セット(塗り・線・テキスト)のみ`elwindui-core`に存在し、`Canvas`自体が未実装のため利用できない |
| MVVM(`viewmodel`/アクション) | `docs/design/gui_framework_design.md` §7.2 | ✅ `#[observable]`/`#[computed]`と、`impl`ブロックの`fn`/`async fn`から自動検出されるアクションが動作し、`examples/notepad`のMVVM構成で使われている |
| 非同期処理 | `docs/design/gui_framework_design.md` §7.3 | 🚧 `spawn`相当(`spawn_local`)は実装済みで`examples/notepad`が使用。`AsyncState<T>`/`#[async_computed]`/`task!`マクロは未実装 |
| リスト仮想化(`VirtualList`) | `docs/specs/builtins_spec.md` 付録Q | 📋 未実装 |
| テーマ/デザイントークン | `docs/status/theme_status.md` | ✅ Rust属性ベースの型付きtheme runtime、application/Window context、標準・独自token、variant/appearance、backend既定値へのclear、Layout背景、WinUI 3/AppKit adapterとappearance監視、`examples/theme-demo`、Windows UI Automation操作テストまで実装済み。GTK4、WinUI High Contrast通知、公開setterが未実装のnative state token適用は未対応 |
| エラーバウンダリ(`ErrorBoundary`) | `docs/design/gui_framework_design.md` §8.6 | 📋 未実装 |
| クリップボード/D&D/ファイルダイアログ | `docs/specs/builtins_spec.md` 付録T | 🚧 `file_dialog`のみ実装(§4参照) |
| Undo/Redo(`#[undoable]`) | `docs/design/gui_framework_design.md` §7.4 | 📋 未実装 |
| スナップショットテスト | `docs/design/gui_framework_design.md` §9 | 🚧 `render_tree`のみ実装。`render_canvas_snapshot`は未実装 |
| モバイル対応(iOS/Android) | `docs/design/gui_framework_design.md` §8.8 | 📋 未実装(設計のみ) |
| フォント/テキストスタイル継承 | `docs/status/font_status.md` | ✅ AppKit実機検証済み、WinUI3はWindowsのビルド・単一Applicationホスト回帰テスト済み。GTK4は未対応。DPI/表示スケール/テキストスケールの概念自体が`elwindui-core`に無いため対応していない |
| SVGベクター画像対応 | §1の`elwindui-svg`/`elwindui-backend-appkit`行 | ✅ AppKitのみ。`elwindui-core`のコア型・`elwindui-svg`(usvgベースローダー、リソースセキュリティポリシー、`SvgLimits`)・AppKitの`render/vector/`(group/path/gradient/pattern/mask/blend/filter graph)・`builtin::Image`・`graphics-demo`のSVGタブ・golden/securityテストまで動作する。WinUI3/GTK4は型のコンパイル整合性のみ(明示的unsupported)。`VectorRasterizeMode`による3モード——`Auto`(描画時のピクセルサイズでラスタライズしキャッシュ)、`Fixed{pixel_width,pixel_height}`(WinUI3の`RasterizePixelWidth`/`RasterizePixelHeight`相当、固定サイズで一度だけラスタライズしその後のリサイズでは再生成しない)、`Vector`(ライブ`CALayer`ツリー描画にオプトイン)。ラスタライズは既存の`render_group`をオフスクリーン合成する形で再利用するため、mask/pattern/filterはそのまま動作する。`Auto`は縮小方向には再ラスタライズせず既存の大きいビットマップを縮小表示し、拡大方向でも要求サイズが現在のキャッシュの1.5倍未満に収まる場合はキャッシュサイズの1.5倍で先読み的にラスタライズする(`auto_raster_target_size`)——ライブウィンドウリサイズのような連続的なサイズ変化で毎フレーム再ラスタライズが走るのを避けるため。**既知の制限**: filter primitiveのうち`Turbulence`/`DiffuseLighting`/`SpecularLighting`/`DisplacementMap`/非3x3・5x5の`ConvolveMatrix`は、Appleが非推奨化した`CIKernel`文字列コンパイルAPIによるカスタムシェーダー実装が必要なため対象外(ユーザーの明示的判断)——明示的diagnostic(`report_unsupported`、silent skipしない)で入力を素通しする近似のまま。path形状ベースの精密ヒットテストは未実装 |

---

## 8. ツールチェーン状況

| ツール | 状況 |
|---|---|
| `elwindui-codegen`(コード生成) | ✅ バックエンド選択の定数畳み込み(`docs/design/gui_framework_design.md` §3.3)は前提機能が無いため未実装。`#[elwindui::template]`(`docs/design/tools/codegen_design.md` §4・`docs/specs/dsl_spec.md` §4)は📋設計のみ |
| `elwindui-languageserver`(LSP) | 🚧 単一`.rs`ファイル単位の診断・メンバー補完まで実働。シンタックスハイライトは`view! { .. }`マクロ本体に限定して実装済み。hover・プレビュー用インスタンス生成パイプラインは未実装 |
| ホットリロード(`elwindui-hotreload`) | 📋 スタブのみ。remount/patch判定ロジックのみ存在し、dylib差し替えは未実装 |
| リアルタイムプレビュー | 📋 **クレート自体が存在しない**。100%未着手 |

---

## 9. バックエンドcrateのファイル構成

`elwindui-backend-appkit`/`elwindui-backend-winui3`は同じ層構成を持ち、依存は上から下への一方向のみ。

| モジュール | 責務 | 依存してよい先 |
|---|---|---|
| `native_ui/` | 公開ファサード。builtinごとに`#[class]`1つ、`*Ext`トレイトを`inner`への委譲で実装 | `inner`, `host` |
| `inner/` | コントロール別の生プラグイン(`Inner`接頭辞)。1ファイル1コントロール系統 | `host`, `render`, `ffi` |
| `host/` | ツリーホストビュー。`layout_root`/`RenderTree`駆動、OSイベント→core入力 | `render`, `ffi` |
| `render/` | 描画のみ。`UIElement`/フォーカス/コントロールを一切知らない | `ffi` |
| `ffi.rs` | ツールキットとの境界。型消去ハンドル`AnyView` | (なし) |
| `app.rs` | Dispatcher、アプリデリゲート、イベントループ入口 | |
| `platform/` | UI要素ではないOSサービス(ファイルダイアログ) | |

両バックエンドで`native_ui/`・`inner/`・`host/`は同じファイル名で対応する:

- `native_ui/`: `button.rs` `control.rs` `menu.rs` `scroll_view.rs` `tab_view.rs` `text.rs` `window.rs`
- `inner/`: `button.rs` `menu.rs` `scroll_view.rs` `tab_view.rs` `text.rs` `window.rs`
- `host/`: `event.rs` `replay.rs`

バックエンド固有の追加モジュール:

- appkit: `render/`は`geometry.rs` `image.rs` `layer.rs` `paint.rs` `path.rs` `text.rs` `vector/`。`testsupport/`にgolden-imageテスト基盤
- winui3: `render/`は`composition/` `text.rs` `vector.rs` `win2d.rs`。`bindings.rs`が`build.rs`生成のWinRT projection

**プラットフォーム非依存ロジックの`elwindui-core`への集約**: `base::Rect::union`/`intersect`、`graphics::fitted_image_rect` + `impl From<Stretch> for ImageFit`、`input::ShortcutRegistry::collect_from_tree`、`ui::ChildList<T>`(`ListExt`実装の裏側の記憶域)。

**未統合として残るもの**: `fitted_image_rect`は3つの変種(原点がdest相対か絶対か、入力が`ImageFit`か`ImageDrawOptions`か`ImageBrush`か、退化サイズのガード有無)があり、統合は「移動」ではなく挙動変更になるため、winui3側の2つは`elwindui-core`へ寄せていない。

**検証状況**: appkitは`cargo test`・`rust-analyzer diagnostics`(0エラー)・notepad/graphics-demoのスクリーンショットまで確認済み。winui3は`#![cfg(target_os = "windows")]`のため本機では空crateにコンパイルされ、**型検査すらされない**——全ファイルが構文解析を通ること、層の依存方向、モジュール間参照の静的監査のみ実施している。Windows上でのビルド確認が済むまで未検証扱いとすること。

---

## 10. 既知の主なギャップ

- **GTK4バックエンドは事実上何も実装されていない**(19行のスタブ)。本ドキュメントの他の章で「WinUI3/AppKit/GTK4」と横並びで書かれている箇所の多くは、GTK4に関しては未着手である
- **アクセシビリティは型定義のみ**で、`UIElement`ツリーにもバックエンドのネイティブAPI(`AutomationPeer`/`NSAccessibilityElement`/AT-SPI)にも未結線
- **ルーティングイベント(`#[routed]`)の実配線はAppKit・WinUI3両バックエンドで対応**(WinUI3はこのマシンでコンパイル確認不可のため未検証)。`Button`の実クリック(`on_click`)、共通`component UIElement`が宣言する9個のポインタ/タップイベント(`input::PointerDispatcher`)、5個のキーボード/フォーカスイベント(`input::KeyboardDispatcher`/`focus::FocusTracker`)が自前描画系`UIElement`(`Layout`/`Control`/`Shape`/`TextBlock`系)で実配線済み——`Button`/`TextArea`/`TabView`等のネイティブリーフは別ウィジェットとして重なっているため、ポインタ/キーボードいずれも実際には発火しない(`on_click`のみ個別配線済み)。`hit_test`は`ClipToBounds`/透明背景パススルー/`IsHitTestVisible`に対応済み。トンネリングイベント・`Canvas`固有のポインタイベント・明示的ポインタキャプチャAPIは未着手
- **`store`(グローバル状態)が未実装**——`viewmodel`(MVVM)は実装済みで、`examples/notepad`のMVVMは`viewmodel`のみで構成されている
- **`Backend` enum / `target::backend()`が存在しない**ため、これに依存する多くの静的検証ルール・ビルトイン(`NavigationHost`、ダイアログ/メニューのバックエンド分岐等)が未実装の根本原因になっている。将来この仕組みを実装する際は、影響範囲がドキュメント全体に及ぶことに留意する
- **`Control.template`(`ControlTemplate`)は設計のみ・未実装。** 前提となる「値計算コールバックがネストした要素を構築する」構文(`VirtualList`の`render_item`と共通)も未実装のため、実装時はまずそちらから着手が必要。広域既定値は`store`(同じく未実装)への依存として設計されている
- **フォント機能はGTK4未対応・DPI非対応。** `ScrollView`/`TabView`がホストする入れ子`TreeHostView`配下のコンテンツにはフォント継承が届かない(visualチェーンがそこで途切れるため)
- **合成された(`inherits`の)ユーザー定義基底が、自身に`on_*`配線・bindableフィールド・`on_mount`を持つ場合、それらが埋め込み先で失われる。** `codegen::generate_view`が生成する`on_constructed`はこれらの配線を`Rc::downgrade(&this)`でクロージャへcaptureするため、`this: Rc<Self>`の実体が要る——通常は`__self_weak`(仕様上「構築中の最派生オブジェクトへの弱参照」、`docs/specs/macro_class_spec.md` §13.3)を`Self`へdowncastして得るが、この基底が*別の*生成コンポーネントの合成`base:`フィールドとして埋め込まれている場合(`inherits <ユーザー定義コンポーネント>`)、最派生オブジェクトは埋め込み先(外側)のコンポーネントであり、downcastは意図どおり失敗する。`&self`だけで足りる`resync()`/`__refresh_dynamic_regions()`/content配線は埋め込み時も正しく動く。修正には`__self_weak`と並ぶ型付きweak参照の追加か、これらのクロージャのdowncastを呼び出し時まで遅延させる設計変更が要る。単純な基底(配線・bindable・on_mountを持たない、`examples/inheritance-demo`の`LabeledPanel`など)は問題なく動作する
- **`builtin::Image`は実装済みだが仕様書に節が無い。** `docs/specs/builtins_spec.md` 付録Fに`Image`の項目が存在しないため、プロパティ(`source`/`stretch`/`rasterize`)の規範的な定義は`crates/elwindui-core/src/ui.rs`の宣言のみが正になっている
