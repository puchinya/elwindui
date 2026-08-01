# ElwindUIL 実装状況ドキュメント

`docs/elwindui_dsl_spec.md`・`docs/elwindui_builtins_spec.md`・`docs/elwindui_gui_framework_design.md`・`docs/elwindui_tool_*_design.md`は仕様/設計書であり、将来実装予定のフォワードルッキングな内容を含む。本ドキュメントは「実際に`crates/`配下に何が実装済みで、何が未着手か」を横断的に一覧化したもの。各仕様書ファイル自身にも該当箇所へ実装状況の注記を追加済みなので、詳細な文脈はそちらを参照すること。

このドキュメントは2026年時点のワークスペース(`cargo build --workspace`が通る状態)を実地調査して作成した。実装は日々変化するため、内容が古くなったと思われる場合は`crates/`を直接確認すること。

---

## 1. クレート別実装状況

| クレート | 行数(目安) | 状況 |
|---|---|---|
| `elwindui-core` | 約4200行 | 実装済み。`UIElement`クラス階層(`#[elwindui_macros::class]`)、WinUI3準拠のMeasure/Arrange(`measure`/`arrange`/`measure_override`/`arrange_override`)、retained `RenderTree`/`RenderContext`、ルーティングイベント(`dispatch_routed`/`dispatch_direct`/`hit_test`、`ClipToBounds`/透明背景パススルー/`IsHitTestVisible`対応済み)、ポインタ/タップ入力(`elwindui_core::input::PointerDispatcher`)、キーボード/フォーカス入力(`elwindui_core::input::KeyboardDispatcher`/`ShortcutRegistry`、`elwindui_core::focus::FocusTracker`、`UIElementExt::focus()`/`FocusHost`)が実働。`graphics`モジュール(`src/graphics/`)は`Color`/`Brush`(単色・線形/放射グラデーション・画像)/`StrokeStyle`/`Path`・`PathBuilder`(cubic正規化、`contains`/`stroked_contains`はbounding-box近似ではなく実装済みのwinding-number/線分距離判定、真偽演算`combine`は`flo_curves`クレート採用)/`Image`/`RenderCommand`(Fill/Stroke×Rect・RoundedRect・Ellipse、DrawLine、Fill/StrokePath、DrawImage、Text、Push/Pop×Clip・Transform・Opacity)が実装済み——`Rectangle`/`Ellipse`の`fill`/`stroke`、`TextBlock`の`color`は`Option<String>`(hex文字列)から`Option<Brush>`/`Option<Color>`へ移行済み(`.elwind`側の`fill: "#3a3a3c"`のような文字列リテラル構文は`elwindui-codegen`がコード生成時に検証・変換するため無変更で動作する)。**SVGベクター画像対応**(2026-07-20追加): `VectorImage`/`VectorImageBuilder`/`VectorGroup`/`VectorNode`/`VectorPathNode`/`VectorRasterNode`/`VectorPaint`/`VectorPattern`/`VectorClipPath`/`VectorMask`/`VectorFilter`(17種filter primitive全型)/`ImageSource`(`Raster(Image)`/`Vector(VectorImage)`)/`RenderCommand::DrawVectorImage`/`RenderContext::draw_vector_image`が`src/graphics/{vector_image,vector_scene,vector_filter}.rs`に実装済み。`elwindui-core`自体はusvg/SVGファイル形式に一切依存しない(読み込みは新規crate`elwindui-svg`が担当)。`AffineTransform::invert()`(公開API)も同時に追加。`AccessibilityNode`トレイトは**型定義のみ**で実装(`impl`)がテスト用ダミー1つ以外に存在しない |
| `elwindui-svg`(2026-07-20新規) | 約900行 | 実装済み。`usvg 0.47`ベースのSVGファイル/文字列/bytesローダー(`SvgLoader`/`load_svg_file`/`load_svg_bytes`/`load_svg_str`)。usvgの静的機能(path/gradient/pattern/clipPath/mask/filter/text(グリフをpath化)/nested SVG/埋め込みラスター/data URL/SVGZ)を`elwindui_core::graphics::VectorImage`へ変換。リソース解決ポリシー(`SvgResourcePolicy::DenyExternal/DataUrlsOnly/SameDirectory/Custom`、パストラバーサル・シンボリックリンク脱出防止)、`SvgLimits`(ノード数/path command数/group深さ/filter primitive数/埋め込み画像バイト数/外部リソース数/nested SVG深さ/圧縮後バイト数の上限、SVGZ decompression bomb対策込み)を実装。SVGパースは`elwindui-core`から完全に隔離されており、バックエンドは`VectorImage`のみを参照する |
| `elwindui-codegen` | 約9300行 | 実装済み(コンパイラ本体)。2026-07-29〜31にかけて`.elwind`テキスト/`build.rs`経由の起動経路(`compile_dir`/`compile_dir_with_extra_viewmodels`)を完全削除し、`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`プロシージャルマクロ経路へ一本化した(Refs #14)。`#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }`という、`component`+`view`ペアを1つのRust `struct`として書ける形式(`component_frontend.rs`)が本番の唯一の入力経路 ── `view!`は実在するマクロではなく、`.elwind` DSLテキストとして読み出されるだけの型位置マクロ呼び出し。`view!`フィールドは省略可能(view無しコンポーネント)、component単位属性(`#[embedded]`/`#[sealed]`/`#[native]`/`#[abstract_]`/`#[text_style]`/`#[content(field)]`)・`#[param(default = ...)]`もこの形式で書ける。`builtins.elwind`(旧・ビルトイン形状の二重管理テキスト)は削除済み──ビルトイン25個の形状は`elwindui-core::ui`/各backendの`#[elwindui_macros::class]`宣言そのものが正であり、`__elwindui_shape_{Name}!`という`#[class]`が生成するmacro_rulesの遅延合成経由でクレート境界を越えて伝播する(専用のシンボルテーブルを持たない)。`crates/elwindui-codegen/src/testdata/builtins_dsl_text.txt`(旧`builtins.elwind`──受け入れ条件「リポジトリ内の`.elwind`ファイルが0個」を満たすため拡張子を変更済み)はテスト専用フィクスチャとして残存(本番コードは一切参照しない、内容がドリフトしても実害なし)。`parser::parse_module`(`.elwind`テキストのトップレベル`component`/`view`/`viewmodel`/`enum`/`use`構文)自体も残っている──`parser.rs`/`validate.rs`/`codegen.rs`/`component_frontend.rs`の計100件超のテストがこのテキスト形式をフィクスチャとして使っているため(書き換えは別タスク)が、`#[cfg(test)]`化・`pub(crate)`降格済み(本番コードからは呼べない)。`view!`本体を解析する`parse_view_body`/`parse_initializer`は無条件の本番APIのまま。`#[elwindui::dsl_enum] enum Name { .. }`(新設)はプレーンなRust enumを`view!`のmatch網羅性検査に載せるためのopt-in属性で、enum本体はそのまま透過する |
| `elwindui-macros` | - | 実装済み。`#[class(inherits/implements/supertrait/abstract_class/sealed)]` + `#[inherent]`/`#[ancestor]`によるクラス階層生成マクロ。`docs/elwindui_gui_framework_design.md`§5.1aの記述と一致。各`{ClassName}Ext`トレイト(root/ordinary/`trait_only`いずれの宣言でも)に`into_<name>_node(self: Rc<Self>) -> Rc<dyn {ClassName}Ext>`というアップキャストのデフォルトメソッドが自動生成される(2026-08-01追加)。祖先ごとにその祖先自身のトレイト宣言側で1つ生成され、Rustの通常のデフォルトメソッド継承(supertraitチェーン経由)で子孫に伝播するため、`#[class]`のancestor-forwarding機構(`build_inherit_macros`)側の特別対応は不要。`UIElement`を祖先に持たないクラス(`MenuItem`/`Window`等、`trait_only`かつ`inherits`無し)には対応する`into_ui_element_node`が生成されない、という形で「汎用UIElement子として埋め込めない」制約が自然に効く |
| `elwindui-i18n` | 58行+マクロ | 実装済み。Fluentベースのランタイム(`t!`, `declare!`マクロ)。ただしビルド時の`.ftl`静的検証(未翻訳キー検出・引数名整合性チェック)は`elwindui-codegen`側に存在せず未実装 |
| `elwindui-languageserver` | 約700行 | 部分実装。2026-08-01に`.elwind`ディレクトリ単位のモデルから単一`.rs`ファイル単位のモデルへ retarget(`component_frontend::modules_from_file`が`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`を実マクロ展開せずに読み出す)。診断(`elwindui-codegen`の`component_frontend`/`validate`を再利用、`syn::parse_file`失敗時は実際の行・列付き)、メンバー補完(`vm.field`)が実働。シンタックスハイライト(semantic tokens)は`.elwind`専用トークナイザを実装していたが、対象が実Rustファイルになったため削除済み(rust-analyzer自身のハイライトで足りる)。旧モデルにあったディレクトリ横断のクロスファイル解決(`bind!`/`vm.field`を別ファイルのviewmodelと突き合わせる)は無い ── 実マクロ展開経路自体、宣言順に依存する同一クレート内レジストリにしか頼っておらず、実コンパイルを行わないLSPには再現不能なため。hover、生成コードプレビュー、オフスクリーンレンダリングと連携したインスタンス生成パイプラインは未実装 |
| `elwindui-hotreload` | 32行 | スタブのみ。`param`/`prop`差分からremount/patchを判定する純粋関数(`decide_reload_action`)だけが存在し、`hot-lib-reloader`統合・実際のdylib差し替えは未実装 |
| `elwindui-test` | 79行 | 部分実装。`render_tree`(`UIElement`ツリーの、各ノードを`type_name()`でラベル付けしたインデントダンプ)のみ実装。`render_canvas_snapshot`/`assert_image_snapshot!`は未実装(`canvas.rs`はdocコメントのみのスタブ) |
| `elwindui-backend-appkit` | 約7900行 | 実装済み・実機検証済み。本機で`cargo build`/実行/スクリーンショット確認済みの唯一のバックエンド。描画replay(`host/replay.rs`の`replay_group`/`replay_commands`)は`RenderCommand`ごとに`CAShapeLayer`(fill/stroke/dash/cap/join/miter/nonzero-evenodd)・`CAGradientLayer`(実グラデーション、`try_add_gradient_fill_layer`)・`CATextLayer`・画像用`CALayer`を組み立てるCALayer合成方式(`NSView.draw(_:)`+`CGContext`直接記述ではない)。clipは`CAShapeLayer`マスクによる実ピクセル単位([`docs/elwindui_implementation_status.md`旧版の「bounding-box交差判定のみ」という記述は誤り・要修正済み——`clip_mask_layer`が実パス形状マスクを構築する])。**SVGベクター画像描画**(`render/vector/`、2026-07-20追加、同日中に2回目の追加実装あり): `RenderCommand::DrawVectorImage`をフル実装。group transform/opacity/clip/mask(alpha・luminance、オフスクリーンraster化)/blend-mode(`CALayer.compositingFilter`+Core Image blend filter)/filter graph(Core Imageへマッピング、`GaussianBlur`/`Offset`/`Merge`/`Composite`(Over/In/Out/Atop/**Xor**/**Arithmetic**、後者2つは`CISourceOutCompositing`の組み合わせと`CIMultiplyCompositing`+`CIColorMatrix`+`CIAdditionCompositing`の組み合わせで追加実装)/`Blend`/`Flood`/`ColorMatrix`/`Morphology`/`ConvolveMatrix`(3x3/5x5)/`DropShadow`/`ComponentTransfer`(Linear限定)/`Image`/**`Tile`**(`CIAffineTile`で追加実装)primitiveは実装、`Turbulence`/`DiffuseLighting`/`SpecularLighting`/`DisplacementMap`/非3x3・5x5の`ConvolveMatrix`は非推奨`CIKernel`文字列コンパイルAPIが必要なためユーザーの判断で対象外のまま、明示的diagnostic(`report_unsupported`、silent skipしない)で入力をそのまま通す)、path塗り(単色・グラデーション、任意の回転/拡大縮小変換下でも正しく動作——既存`try_add_gradient_fill_layer`の純平行移動限定を`position`/`bounds`/`affineTransform`方式で解消)、pattern(**真の無限タイリング**——塗り対象の境界を覆うタイル格子を計算し、`ImageBrush`タイル塗りの`add_tiled_image_layers`と同じ「`CALayer`を複数敷き詰める」技法を回転/拡大縮小対応の形へ一般化して適用、`render/paint.rs`の`add_tiled_image_layers`参照)、埋め込みラスター画像を実装。SVG読み込み(`elwindui-svg`/`usvg`)への依存はproduction経路に一切無い(dev-dependency経由のgolden testのみ)。オフスクリーン`CGBitmapContext`+`CALayer.renderInContext`によるgolden-imageテスト: `testsupport/golden.rs`(4シーン)に加え、`testsupport/svg_golden.rs`(`resvg`参照描画とのサンプル点比較、8シーン: 単色/線形グラデーション/groupopacity/clipPath/patternタイリング/feComposite Xor/feComposite Arithmetic/feTile)を追加 |
| `elwindui-backend-winui3` | 約8200行 | 実装コードあり。appkitと同じ層構成(`native_ui`/`inner`/`host`/`render`/`ffi`/`app`/`platform`、2026-07-25にファイル単位で揃えた)を持つ。Win2Dコマンドリスト(`render/win2d.rs`)+ `Microsoft.UI.Composition`のretainedレンダラ(`render/composition/`)+ SVGベクター描画(`render/vector.rs`)を実装済み。`build.rs`(576行)が`windows-bindgen`でWinRT projectionを生成し、`cpp/app_host.cpp`(C++/WinRT)が`Application`のcomposable-class集約を担う(windows-rsが未対応のため、microsoft/windows-rs#3404)。**フォント/テキストスタイル経路はWindows上でビルドおよび単一Applicationホスト回帰テストを完了済み。その他の機能の全体的な実機検証は引き続き未完了** |
| `elwindui-backend-gtk4` | 19行 | 未着手。`src/lib.rs`が19行のみのスタブ(`init()`と、`startup`をそのまま呼ぶだけの`application::run`)で、`native_ui`/`inner`/`host`/`render`/`platform`は一切存在せず、`gtk4` crateへの依存すら無い。`RenderCommand`を扱うコード自体が無いため`DrawVectorImage`も未着手のまま(既存の他コマンドと同列) |
| `elwindui`(ファサード) | 68行 | 実装済み。`backend-appkit`/`backend-winui3`/`backend-gtk4`のCargoフィーチャで`core`/`i18n`/`backend`/`ui`を再エクスポートする。`svg`フィーチャ(2026-07-20追加)で`elwindui::svg`として`elwindui-svg`を再エクスポート |
| プレビューツール(`elwindui-preview`相当) | - | **ワークスペースに存在しない**。`docs/elwindui_tool_preview_design.md`は100%未着手のフォワードルッキング設計 |

---

## 2. バックエンド対応状況

| バックエンド | 状況 |
|---|---|
| AppKit(macOS) | 実装済み・実機検証済み |
| WinUI3(Windows) | 実装コードあり。フォント/テキストスタイルはWindowsでビルド・実行テスト済み、他機能の全体的な実機検証は未完了 |
| GTK4(Linux) | 未着手(2行のスタブのみ) |
| Uikit(iOS)/Jetpack(Android)(`docs/elwindui_gui_framework_design.md`§8.8) | 設計のみ、コード無し |

現在のバックエンド候補は上記のネイティブ3種(+将来のモバイル2種)のみ。

**フォント/テキストスタイル(2026-07-26追加)**: `docs/elwindui_font_status.md`が単一の真実の源。AppKitは実装・実機検証済み(`NSFontDescriptor`/`NSAttributedString`ベース)。WinUI3は生成バインディング型で実装し、Windowsでのビルドと単一Applicationホスト回帰テストを完了済み。GTK4はバックエンド自体が未着手のため対象外。

**重要な設計と実装の乖離**: `docs/elwindui_gui_framework_design.md`§3.3が説明する`enum Backend` + `target::backend()`(コンパイル時定数、`match`網羅性検査による新バックエンド追加時の安全弁)は**コード中のどこにも実体が存在しない**。実際のバックエンド選択は`elwindui`ファサードクレートのCargoフィーチャフラグ(`backend-appkit`/`backend-winui3`/`backend-gtk4`)による`#[cfg(feature = ...)]`のみで行われている。これに伴い、`native!`/`match target::backend()`をビルトイン限定にする14章ルール9、`NavigationHost`の`Route`網羅性ルール14、オーバーレイ系ビルトインの分岐制限ルール15なども、前提となる仕組み自体が無いため検証しようがない。

---

## 3. ビルトインウィジェット実装状況

`crates/elwindui-core/src/ui.rs`と各`elwindui-backend-*`crateの`#[elwindui_macros::class]`宣言を正とする(旧`builtins.elwind`は2026-07-31に本番から削除済み──§1の`elwindui-codegen`行参照)。詳細な分類ツリーは`docs/elwindui_builtins_spec.md`冒頭を参照。

### 実装済み(`.elwind`宣言 + バックエンド実体あり)

`Window` / `VerticalLayout` / `HorizontalLayout`(`Row`/`Column`という名称ではない) / `Shape`(抽象) / `Rectangle` / `Ellipse` / `Image`(2026-07-20新規) / `Control` / `ContentControl` / `Grid` / `TextArea` / `TextBox`(NativeControl拡充Phase 1で新規、AppKit検証済み・WinUI3未検証・GTK4未着手) / `PasswordBox`(同上) / `ScrollView`(同上、ネストした`TreeHostView`/`TreeHostPanel`による`ElwinduiContentRoot`構造) / `Button` / `TextBlock` / `MenuBar` / `MenuBarItem` / `Menu` / `MenuItem` / `TabView` / `TabViewItem` — TextBox/PasswordBox/ScrollViewの詳細は`docs/elwindui_nativecontrol_expansion_status.md`参照

- `Image`は`Rectangle`/`Ellipse`と同じくバックエンド非依存の自己描画builtin(`elwindui_core::ui::Image`、ネイティブウィジェットを持たない)。`source: Option<ImageSource>`(`Raster(Image)`/`Vector(VectorImage)`)、`stretch: Option<Stretch>`、`rasterize: Option<VectorRasterizeMode>`(2026-07-20新規、`Vector`ソースのみ有効。§5のSVG節参照)。ヒットテストは`Shape`同様bounding-box精度のみ(`UIElement::hit_test_content`が点を受け取らない既存シグネチャのため、path形状ベースの精密ヒットテストは別タスク)。
- `Menu`/`MenuItem`は`MenuBarItem.submenu`経由での利用は実装済みだが、任意要素に`context_menu`属性で汎用的に付けるコンテキストメニュー機構は未実装。
- `tooltip`共通属性も未実装。
- `Control`の`template: Option<ControlTemplate<Self>>`(WinUI3の`Control.Template`相当の視覚ツリー実行時差し替え、`docs/elwindui_builtins_spec.md`付録F.9.1・`docs/elwindui_dsl_spec.md`§4・`docs/elwindui_gui_framework_design.md`§5.12)は**設計のみ・未実装**。`crates/elwindui-core/src/ui.rs`の`Control`構造体に対応フィールドは無く、現状は`children`をそのままVisual子要素にする挙動のみ実装済み。
- **フォント/テキストスタイル(2026-07-26実装)**: `Control`/`TextBlock`/各バックエンドの`NativeControl`に`font_family`/`font_size`/`font_weight`/`font_style`/`font_stretch`/`character_spacing`/`foreground`の7プロパティが実装済み(`#[text_style]` DSLコンポーネント属性、`docs/elwindui_dsl_spec.md`付録A)。プロパティ単位で独立にVisual Parent経由で継承される。`TextBlock`の旧`color: Option<Color>`は`foreground: Option<Brush>`に置き換わり廃止された(`color:`はもう使えない)。`TextBlock::measure_override`は旧`chars().count() * 8.0`固定近似を廃し、登録済み`TextBackend`(AppKitおよびWinUI3実装済み)による実測に置き換わった。WinUI3向けに動的な`FontFamily`/`Brush`を所有値として渡すコード生成も回帰テスト済み。詳細は`docs/elwindui_font_status.md`参照。

### 未実装(仕様のみ、`.elwind`宣言なし)

`Dropdown` / `Option`(付録F.5)、`Canvas`(付録G)、`NavigationHost` / `Route`(付録L)、`Dialog`(付録M.1)、`Tooltip`(付録M.3)、`VirtualList`(付録Q)

### `platform::`名前空間(付録T)

| 機能 | 状況 |
|---|---|
| `platform::file_dialog::open()`/`save()` | 実装済み(AppKit検証済み・WinUI3未検証・GTK4無し)。戻り値は`Option<PathBuf>`のみで、仕様書にあるファイルフィルタ指定引数は無い |
| `platform::clipboard::*` | 未実装(コード自体が存在しない) |
| ドラッグ&ドロップ(`draggable`/`on_drag_start`/`on_drop`) | 未実装 |

---

## 4. 言語コア機能の実装状況(`docs/elwindui_dsl_spec.md` §1〜14)

| 機能 | 状況 |
|---|---|
| `component`/`view`分離 | 実装済み |
| `param`/`prop`区別(`#[param]`、静的評価式制限) | 実装済み |
| 制御構文(`if`/`for`/`match`) | 実装済み。子要素位置の `if`/`else`(`else if`チェーン含む)・`match`・`for item in collection` は、親コンポーネント所有の透明な動的子範囲として `#[content(...)]` コレクションへ直接 insert/remove する。各範囲は前後の静的子要素と他の動的範囲を保持する。`for Vec<Rc<T>>`（および viewmodel 要素のリスト）は `Rc::ptr_eq` の identity で既存 item の子・購読を再利用し、他の collection は当該範囲のみを再構築する。`match` は validator が user enum の非網羅 arm をエラーにする。`if`/`match`の分岐内へのさらなる `if`/`match`/`for` の入れ子(`else if`含む)にも対応(`for`自身のbodyはリテラル要素のみのまま――入れ子非対応)。`#[content(...)]`フィールドが単一値型(`ContentControl`/`Window`の`content`等)の場合も`if`/`match`(`for`不可、全分岐が1要素に還元できる場合のみ)を書けるようになり、`inherits`の暗黙合成(下記)と組み合わせて view root 自体の動的化に相当する記法が書ける |
| `style{}`(横断的属性適用) | **未実装**。`elwindui-codegen`のASTに`Style`ノードが存在しない |
| 値制約(`#[range]`/`#[step]`/`#[length]`/`#[pattern]`/`#[format]`/`#[check]`) | `#[length]`のみ実装。他は未実装 |
| `enum`(`EnumName::values()`、`#[label(...)]`) | `EnumDef`はASTに存在(実装済み)。`values()`/`#[label]`によるi18nラベル付与の実装範囲は個別確認が必要 |
| `env::*` / `once` | **未実装**。`elwindui-codegen`にDSLキーワードとしての扱いが無い |
| `bind!` | 実装済み(`Initializer::Bind`) |
| `viewmodel`アクション(旧`#[command]`/`Command`型/`command!`マクロ、撤廃済み) | 実装済み。`#[elwindui::viewmodel]`のRustネイティブ`impl`ブロックの`fn`/`async fn`がそのまま自動検出されアクションになる(`Initializer::Action`、struct側の宣言は不要)。`.elwind`ネイティブ`viewmodel Name { ... }`構文にはアクションを宣言する手段が無い(`#[observable]`/`#[computed]`のみサポート) — アクションが必要な`viewmodel`は必ずRustネイティブ構文を使う |
| `on_*`イベント属性のクロージャ構文(`\|param, ...\| 式`/`{ .. }`) | 実装済み。対象フィールドの`fn(T0, T1, ...)`宣言から位置対応でパラメータ型を決める汎用機構(`codegen::emit_wiring`)。0引数ハンドラはベアパスの糖衣(`on_click: vm.save`)も書ける |
| 値計算コールバックがネストした要素を構築する構文(`\|param\| Type { .. }`、`VirtualList`の`render_item`・`ControlTemplate<Self>`が依存) | **未実装**。対応する`VirtualList`自体が未実装(§5参照)なため、この構文もコード生成に存在しない |
| `ControlTemplate<Self>`型フィールド・`body: <field>(Self)`・`#[elwindui::template]` | **未実装**。`docs/elwindui_dsl_spec.md`§4、§5参照 |
| i18n(Fluent、`t!`) | ランタイム(`elwindui-i18n`)は実装済み。ビルド時の`.ftl`静的検証(未翻訳キー検出・引数名整合性チェック)は未実装 |
| モジュール(`use`) | 生成先が実際のRustコードのため`use`解決自体はRustコンパイラに委譲される。循環参照・未解決パスの独自の機械的検出は未確認 |
| `visual_tree`モジュール(WinUI3の`VisualTreeHelper`相当。`get_children_count`/`get_child`/`get_parent`/`find_all`) | 実装済み。`UIElement::visual_children()`/`parent()`が本体の走査を担い、ランタイム文字列idによる検索(`find_by_id`相当)は`#[id(...)]`(静的アクセサ)と役割が重複するため未提供・提供予定なし |
| 14章 静的検証ルール(全29項目) | 部分実装。`crates/elwindui-codegen/src/validate.rs`がルール19(`viewmodel`内`view`参照禁止)を含む多くの言語機能バリデーションを実装しているが、前提機能自体が未実装のルール(9・14・15など、`target::backend()`依存。26〜29は`ControlTemplate<Self>`依存)は検証不能。ルール18(旧`#[command]`型検査)は`Command`機構撤廃に伴う欠番 |

---

## 5. UI機能拡張の実装状況

| 機能 | 参照先 | 状況 |
|---|---|---|
| ライフサイクルフック(`on_mount`/`on_unmount`/`on_update`) | `docs/elwindui_gui_framework_design.md`§6.1 | `on_mount`は実装・結線済み。`on_unmount`はパース・コード生成されるが、`elwindui-core::ui`に実際のツリー離脱(デタッチ)フックが無いため**呼び出されない** |
| `store`(グローバル状態) | `docs/elwindui_gui_framework_design.md`§7.1 | **未実装**。ASTに`Store`ノードが無い。`ControlTemplate<Self>`の広域既定値(WinUI3の`Style`代替、同節参照)もこれに依存するため未実装 |
| キーボード入力・フォーカス管理(`on_key_down`/`on_key_up`/`on_text_input`/`on_got_focus`/`on_lost_focus`、`tab_stop`/`focus_order`、`#[shortcut(...)]`、`UIElementExt::focus()`) | `docs/elwindui_gui_framework_design.md`§5.5/§8.1 | 実装済み(AppKit・WinUI3両バックエンド。WinUI3側は`elwindui-backend-winui3`が元々`#![cfg(target_os = "windows")]`ゲートのためこのマシンではコンパイル確認自体不可、未検証)。`#[focus(order/trap)]`という専用DSL属性は設計から変更——`tab_stop`/`focus_order`という普通の共通プロパティに置き換えた(§5.5参照)。自前描画系要素の自動フォーカス移譲(クリックでフォーカス)、方向キーでのフォーカス移動、ネイティブリーフ(`Button`/`TextArea`)自身の`on_key_down`/`on_got_focus`個別配線、IME変換中プレビュー表示は未実装 |
| ナビゲーション(`NavigationHost`/`Route`) | `docs/elwindui_builtins_spec.md`付録L | **未実装**(§3のビルトイン一覧参照) |
| ダイアログ/メニュー/ツールチップ | `docs/elwindui_builtins_spec.md`付録M | `Menu`/`MenuItem`本体は実装済み、`Dialog`/`Tooltip`および汎用`context_menu`/`tooltip`属性は未実装 |
| 描画拡張(Brush/Geometry/Effect/Transform/レイヤー合成/アニメーション) | `docs/elwindui_builtins_spec.md`付録N | 未実装。`Painter`基本セット(塗り・線・テキスト)のみ`elwindui-core`に存在、`Canvas`自体が未実装のため利用できない |
| MVVM(`viewmodel`/アクション) | `docs/elwindui_gui_framework_design.md`§7.2 | 実装済み。`#[observable]`/`#[computed]`と、`impl`ブロックの`fn`/`async fn`から自動検出されるアクションが動作し、`examples/notepad`のMVVM構成で実際に使われている |
| 非同期処理 | `docs/elwindui_gui_framework_design.md`§7.3 | 部分実装。`spawn`相当(`spawn_local`)は実装済みで`examples/notepad`が使用。`AsyncState<T>`/`#[async_computed]`/`task!`マクロは未実装 |
| リスト仮想化(`VirtualList`) | `docs/elwindui_builtins_spec.md`付録Q | 未実装 |
| テーマ/デザイントークン(`#[elwindui::theme_definition]` / `theme!`) | `docs/elwindui_theme_status.md` | Rust属性ベースの型付きtheme runtime、application/Window context、標準・独自token、variant/appearance、backend既定値へのclear、Layout背景、WinUI 3/AppKit adapterとappearance監視、`examples/theme-demo`、Windows UI Automation操作テストまで実装済み。GTK4、WinUI High Contrast通知、公開setterが未実装のnative state token適用は未対応 |
| エラーバウンダリ(`ErrorBoundary`) | `docs/elwindui_gui_framework_design.md`§8.6 | 未実装(`.elwind`宣言なし) |
| クリップボード/D&D/ファイルダイアログ | `docs/elwindui_builtins_spec.md`付録T | §3参照(file_dialogのみ実装) |
| Undo/Redo(`#[undoable]`) | `docs/elwindui_gui_framework_design.md`§7.4 | 未実装 |
| スナップショットテスト | `docs/elwindui_gui_framework_design.md`§9 | `render_tree`のみ実装。`render_canvas_snapshot`は未実装(§1参照) |
| モバイル対応(iOS/Android) | `docs/elwindui_gui_framework_design.md`§8.8 | 未実装(設計のみ) |
| フォント/テキストスタイル継承(`TextStyleOwner`、`font_*`/`foreground`) | `docs/elwindui_font_status.md` | 実装済み(AppKit実機検証済み、WinUI3はWindowsのビルド・単一Applicationホスト回帰テスト済み)。GTK4は未対応。DPI/表示スケール/テキストスケールの概念自体がelwindui-coreに無いため対応していない |
| SVGベクター画像対応(`VectorImage`/`elwindui-svg`/`builtin::Image`) | 本節末尾の指示書(`elwindui_svg_support_implementation_instructions.md`) | 実装済み(AppKit)。`elwindui-core`のコア型・新規crate`elwindui-svg`(usvgベースローダー、リソースセキュリティポリシー、SvgLimits)・AppKitの`vector_renderer.rs`(group/path/gradient/pattern/mask/blend/filter graph)・`builtin::Image`・`graphics-demo`のSVGタブ(実イラストで視覚検証済み)・golden/securityテストまで実装済み。WinUI3/GTK4は型のコンパイル整合性のみ(明示的unsupported、未実装)。**pattern塗りは真の無限タイリングに対応済み(2026-07-20追加実装)**。**`VectorRasterizeMode`(`builtin::Image`の`rasterize`属性)によるラスタライズ+キャッシュ描画モードを実装済み(2026-07-20追加、WinUI3の`SvgImageSource`に相当)——既定の`Auto`(描画時のピクセルサイズでラスタライズしキャッシュ)、`Fixed{pixel_width,pixel_height}`(WinUI3の`RasterizePixelWidth`/`RasterizePixelHeight`相当、固定サイズで一度だけラスタライズしその後のリサイズでは再生成しない=事前キャッシュ相当)、`Vector`(既存のライブ`CALayer`ツリー描画にオプトイン)の3モード。ラスタライズ自体は既存の`render_group`をオフスクリーン合成する形で再利用しているため、mask/pattern/filterなど既存機能はそのまま動作する。`Auto`は描画サイズの縮小方向には再ラスタライズせず既存の大きいビットマップをそのまま縮小表示し、拡大方向でも要求サイズが現在のキャッシュの1.5倍未満に収まる場合はキャッシュサイズの1.5倍で先読み的にラスタライズする(`vector_renderer.rs`の`auto_raster_target_size`)——ライブウィンドウリサイズのような連続的なサイズ変化で毎フレーム再ラスタライズが走るのを避けるため(2026-07-20追加)。**既知の制限: filter primitiveのうち`Turbulence`/`DiffuseLighting`/`SpecularLighting`/`DisplacementMap`/非3x3・5x5の`ConvolveMatrix`は、Appleが非推奨化した`CIKernel`文字列コンパイルAPIによるカスタムシェーダー実装が必要なため対象外(ユーザーの明示的判断、実装量・リスクとも大きいため)——明示的diagnosticで入力を素通しする近似のまま(`Tile`/`Composite`のXor・Arithmeticは既存のCore Image組み込みフィルターの組み合わせで2026-07-20に解消済み)。path形状ベースの精密ヒットテストは未実装(§3参照) |

---

## 6. ツールチェーン状況(`docs/elwindui_tool_*_design.md`)

| ツール | 状況 |
|---|---|
| `elwindui-codegen`(コード生成) | 実装済み。`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`プロシージャルマクロ経路に一本化済み(`.elwind`/`build.rs`経路は削除済み、§1参照)。バックエンド選択の定数畳み込み(`docs/elwindui_gui_framework_design.md`§3.3)は前提機能が無いため未実装。`#[elwindui::component]`/`#[elwindui::viewmodel]`と同系統のRust代替記法`#[elwindui::template]`(`docs/elwindui_tool_codegen_design.md`§4.2・`docs/elwindui_dsl_spec.md`§4参照)は設計のみ・未実装 |
| `elwindui-languageserver`(LSP) | 部分実装。単一`.rs`ファイル単位の診断・メンバー補完まで実働(2026-08-01 retarget、§1参照)。シンタックスハイライトは実Rustファイル化に伴い削除(rust-analyzer任せ)。hover・プレビュー用インスタンス生成パイプラインは未実装 |
| ホットリロード(`elwindui-hotreload`) | スタブのみ。remount/patch判定ロジックのみ存在、dylib差し替えは未実装 |
| リアルタイムプレビュー | **クレート自体が存在しない**。100%未着手 |

---

## 6.9 バックエンドcrateのファイル構成(2026-07-25 再編成)

`elwindui-backend-appkit`/`elwindui-backend-winui3`は、どちらも当初の
`lib.rs` + `inner.rs` + `native_ui.rs`という3ファイル構成に機能を積み増した結果、
`inner.rs`が4884行/3457行の「何でも入れ」になっていた。両者を同じ層構成へ再編成した。

| モジュール | 責務 | 依存してよい先 |
|---|---|---|
| `native_ui/` | 公開ファサード。builtinごとに`#[class]`1つ、`*Ext`トレイトを`inner`への委譲で実装 | `inner`, `host` |
| `inner/` | コントロール別の生プラグイン(`Inner`接頭辞)。1ファイル1コントロール系統 | `host`, `render`, `ffi` |
| `host/` | ツリーホストビュー。`layout_root`/`RenderTree`駆動、OSイベント→core入力 | `render`, `ffi` |
| `render/` | 描画のみ。`UIElement`/フォーカス/コントロールを一切知らない | `ffi` |
| `ffi/` | ツールキットとの境界。型消去ハンドル`AnyView` | (なし) |
| `app.rs` | Dispatcher、アプリデリゲート、イベントループ入口 | |
| `platform/` | UI要素ではないOSサービス(ファイルダイアログ) | |

依存は上から下への一方向のみ。再編成前は`inner ⇄ vector_renderer`の循環があった。

**プラットフォーム非依存ロジックの`elwindui-core`への移動**(両バックエンドで重複していたもの):
`base::Rect::union`/`intersect`、`graphics::fitted_image_rect` + `impl From<Stretch> for ImageFit`、
`input::ShortcutRegistry::collect_from_tree`、`ui::ChildList<T>`(`ListExt`実装の裏側の記憶域)。

**検証状況**: appkitは`cargo test`(332件)・`rust-analyzer diagnostics`(0エラー)・
notepad/graphics-demoのスクリーンショットまで確認済み。winui3は
`#![cfg(target_os = "windows")]`のため本機では空crateにコンパイルされ、**型検査すらされていない**
── 全ファイルが構文解析を通ること、層の依存方向、モジュール間参照の静的監査のみ実施。
Windows上でのビルド確認が済むまで未検証扱いとすること。

**未着手として残したもの**: `fitted_image_rect`は3つの変種(原点がdest相対か絶対か、入力が
`ImageFit`か`ImageDrawOptions`か`ImageBrush`か、退化サイズのガード有無)があり、統合は
「移動」ではなく挙動変更になるため、winui3側の2つは手つかずのまま。

### 6.9.1 ドキュメント参照の負債(未解決)

コードベース全体のdocコメントに**存在しないファイル**`docs/elwindui_spec.md`への参照が約134箇所
残っている(`AGENTS.md`を含む)。旧spec分割時に引き継がれなかったもので、内訳:

- 現行docsに実在する節: 付録A(→`elwindui_dsl_spec.md`)、付録T/F/G/L/M/N/Q/X/Y(→`elwindui_builtins_spec.md`) ── 計6箇所程度
- **どのdocsにも存在しない節**: 付録H(64箇所)、付録E(17箇所)、付録B/C/I/O/P/S/V ── 約95箇所

後者はファイル名を差し替えても参照先の節自体が無いため、機械的な修正ができない。付録Hは
`base.rs`等から`付録H.2.1a`として大量に参照されており(幾何プリミティブの定義と思われる)、
内容の行方を知る人による対応が必要。

---

## 7. 既知の主なギャップまとめ

- **GTK4バックエンドは事実上何も実装されていない**(2行のスタブ)。本ドキュメントの他の章で「WinUI3/AppKit/GTK4」と横並びで書かれている箇所の多くは、GTK4に関しては未着手であることに注意。
- **アクセシビリティは型定義のみ**で、`UIElement`ツリーにもバックエンドのネイティブAPI(`AutomationPeer`/`NSAccessibilityElement`/AT-SPI)にも未結線。フォーカス管理(`elwindui_core::focus::FocusTracker`)は実装済み(§5参照)——旧`AccessibilityNode`と並んで「型のみ」だった従来のフォーカス管理箇所はこの節では対象外になった。
- **ルーティングイベント(`#[routed]`)の実配線はAppKit・WinUI3両バックエンドで対応**(WinUI3側はこのマシンでは`elwindui-backend-winui3`自体がコンパイル確認不可のため未検証)。`Button`の実クリック(`on_click`)、共通`component UIElement`が宣言する9個のポインタ/タップイベント(`on_pointer_pressed`等、`elwindui_core::input::PointerDispatcher`)、5個のキーボード/フォーカスイベント(`on_key_down`等、`elwindui_core::input::KeyboardDispatcher`/`elwindui_core::focus::FocusTracker`)が自前描画系`UIElement`(`Layout`/`Control`/`Shape`/`TextBlock`系)で実配線済み——`Button`/`TextArea`/`TabView`等のネイティブリーフは別ウィジェットとして重なっているため、ポインタ/キーボードいずれも実際には発火しない(`on_click`のみ個別配線済み)。`hit_test`自体も`ClipToBounds`/透明背景パススルー/`IsHitTestVisible`(`UIElement::hit_test_visible`)対応済み。トンネリングイベント・`Canvas`固有のポインタイベント・明示的ポインタキャプチャAPIは未着手。
- **`store`/`viewmodel`のうち`viewmodel`(MVVM)は実装済みだが`store`(グローバル状態)は未実装**——`examples/notepad`のMVVMは`viewmodel`のみで構成されている。
- **`Backend` enum / `target::backend()`が存在しないため、これに依存する多くの静的検証ルール・ビルトイン(`NavigationHost`、ダイアログ/メニューのバックエンド分岐等)が「未実装」の根本原因になっている。** 将来この仕組みを実装する際は、影響範囲がドキュメント全体に及ぶことに留意する。
- **`Control.template`(WinUI3方式`ControlTemplate`、`docs/elwindui_dsl_spec.md`§4・`docs/elwindui_gui_framework_design.md`§5.12・`docs/elwindui_builtins_spec.md`付録F.9.1)は設計のみ・未実装。** 前提となる「値計算コールバックがネストした要素を構築する」構文(`VirtualList`の`render_item`と共通)自体も未実装のため、実装時はまずそちらから着手が必要。広域既定値(WinUI3の`Style`代替)は`store`(同じく未実装)への依存として設計されている。
- **フォント機能はGTK4未対応・DPI非対応。** WinUI3の7プロパティ、フォールバック列、既定フォント復帰、計測、ネイティブコントロール反映はWindowsで検証済み。`ScrollView`/`TabView`がホストする入れ子`TreeHostView`配下のコンテンツにはフォント継承が届かない(visualチェーンがそこで途切れるため)。
