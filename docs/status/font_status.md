# フォント/テキストスタイル機能 実装状況

ユーザー提供の「elwindui Font実装指示書」に基づき、WinUI 3のフォントプロパティ・継承・計測・描画動作を基準とした、バックエンド非依存のフォント機能を実装した。本ドキュメントはその実装状況の単一の真実の源であり、`docs/elwindui_nativecontrol_expansion_status.md`と同じ運用方針(マイルストーンごとに更新、完了の誇張をしない)を踏襲する。

---

## 0. 完了条件に対する要約

指示書§35の完了条件はおおむね満たしている。AppKitは実装・実機テスト検証済み、WinUI3はWindows上でビルドと単一ApplicationホストによるXAML round-tripテストまで検証済み、GTK4は設計のみで実装なし。elwindui-core自体にはDPI/表示スケール/テキストスケールの概念が存在せず(`TextMeasureRequest::scale`は常に`1.0`、計測・レイアウトへの影響なし)、この点は今後も対応しない設計判断だが、AppKitバックエンドの*描画解像度*(Retinaでのラスタライズ)は`render::add_sublayer_scaled`によるバックエンド内部の`contentsScale`伝播で対応済み(Issue #18)——「計測はcoreの責務・解像度はバックエンドの責務」という分離。詳細は §6/§9 の未対応事項一覧を参照。

---

## 1. 共通フォントモデル

`crates/elwindui-core/src/graphics/text.rs`(新規)に実装。

| 型 | 役割 |
|---|---|
| `FontFamily` | カンマ区切りフォールバックリストを保持する `Arc<str>` ラッパー。`FontFamily::system()` が「バックエンド既定に委ねる」センチネル(`"system-ui"`)——共通層に `"Yu Gothic UI"` 等の具体名を書かない(指示書§16) |
| `FontWeight(pub u16)` | **数値ニュータイプ**(enumではない)。`THIN`(100)〜`BLACK`(900)の名前付き定数。WinUI3の `FontWeight` 自体が `u16`、AppKitの `NSFontWeightTrait` は連続値の `f32` であり、可変フォントの中間値(450/550等)を表現するために数値型を採用——CLAUDE.mdの「enumが唯一の値集合機構」ルールに近接するため、ユーザーに確認の上で決定した意図的な例外 |
| `FontStyle` | `Normal`/`Italic`/`Oblique` の enum |
| `FontStretch` | `UltraCondensed`〜`UltraExpanded` の9段階 enum。`percent()` で50.0〜200.0のCSS/DirectWrite準拠パーセンテージに変換 |
| `TextStyleProperty` | 7プロパティを識別する enum(`ALL: [Self; 7]`、変更通知・clear APIで使用) |
| `TextStyleValues` | ローカル値。全フィールド `Option<T>`——`None` = 未設定 = 継承 |
| `TextStyleStorage` | 各クラスが実フィールドとして持つ型。`RefCell<TextStyleValues>` + getter/setter(setterは値が実際に変わったかを`bool`で返す)+ `resolve_onto(&ComputedTextStyle) -> ComputedTextStyle`(プロパティ単位のオーバーレイ) |
| `ComputedTextStyle` | 全7項目解決済み。`Option`なし。計測・描画はこれだけを消費する。`fallback()` は旧`TextBlock::measure_override`の近似値(16.0pt相当)を再現し、既存テストの数値が変わらないようにしてある |
| `TextBackend` トレイト + `set_text_backend`/`text_backend`/`clear_text_backend` | 計測シームの登録機構(下記§4) |
| `DummyTextBackend` | 未登録時の決定的フォールバック。elwindui-core単体テストはこれで動く |

`RenderCommand::Text`は `font: Font`(ZST)と`color: Option<Color>`の2フィールドを廃止し、`style: ComputedTextStyle` 1本に統合した(`graphics::command.rs`)。`RenderContext::draw_text`のシグネチャも `color: Option<Color>` → `style: &ComputedTextStyle` に変更。

---

## 2. 値の解決順序とプロパティ単位の継承

1. 自身のローカル値(`TextStyleStorage`)
2. `Visual Parent` を辿った、最も近い `TextStyleOwner` 実装要素の解決済み値(そこで既に親のさらに上まで畳み込み済み)
3. 登録されているバックエンドの `TextBackend::default_text_style()`

7プロパティは常に独立に解決される(`TextStyleStorage::resolve_onto`)——1個だけローカル設定しても他はそのまま親から継承される。`Grid`/`Layout`/`VerticalLayout`/`HorizontalLayout`/`Shape`/`Image` は `TextStyleOwner` を実装しない(＝`UIElement::as_text_style_owner()` の既定 `None` のまま)ため、これらを何段挟んでも継承は切れない(指示書§11)。

### `TextStyleOwner` トレイト

`crates/elwindui-core/src/ui.rs` に手書きで定義した通常の(`#[class]`管理下ではない)トレイト。`Control`/`TextBlock`/各バックエンドの `NativeControl` の3クラスだけが実装する。これら3つは単一継承チェーン上の兄弟(`Control`/`TextBlock`は`UIElement`を直接継承、`NativeControl`も`UIElement`を直接継承)であり、`inherits=`チェーンに組み込めないため、`AsAny`/`RelayoutHost`と同じ「直交する能力トレイト」の形にした。

7プロパティそれぞれに `get`/`set`/`clear` のデフォルトメソッドを持つ。**setter引数の型は6項目が裸の値(`f32`/enum/`FontFamily`)、`foreground`だけが`Option<Brush>`** ——`UIElement::set_width(&self, width: f32)` 等、既存の「Option<T>宣言のDSLフィールドでも実setterは裸値を取り、未設定は『setterを一度も呼ばない』かclear_xで表現する」という規約(house convention)を6項目に踏襲し、`foreground`だけは`Shape::set_fill(&self, fill: Option<Brush>)`の規約に合わせた。値を明示的に`None`として渡す経路は存在しない(指示書§26の「値だけで既定値かローカル値か判定しない」を型で保証)。

### `as_text_style_owner()` と継承ウォーク

```rust
// UIElement (root class)
#[overridable]
fn as_text_style_owner(&self) -> Option<&dyn TextStyleOwner> { None }
```

`try_as_native_control`と全く同じ形(ルートクラスの`#[overridable]`、`Control`/`TextBlock`/各バックエンドの`NativeControl`が`#[overrides]`で`Some(self)`を返す)。`#[class]`マクロの変更は不要だった。

継承ウォーク本体は `ui.rs` の自由関数 `inherited_text_style(base: &UIElement) -> ComputedTextStyle`。`request_relayout`と同じ「最初の一段は生の`visual_parent`フィールドを読み、以降は`Rc<dyn UIElementExt>`の trait メソッドを呼ぶ」形を踏襲している。

### `inheritance_parent(kind: InheritanceParentKind)`

指示書§14通り、**呼び出し側がどちらの木を辿るかを指定する**形にした(boolではなくenum)。

```rust
pub enum InheritanceParentKind { Visual, Logical }

#[overridable]
fn inheritance_parent(&self, kind: InheritanceParentKind) -> Option<Rc<dyn UIElementExt>> {
    match kind {
        InheritanceParentKind::Visual  => self.visual_parent(),
        InheritanceParentKind::Logical => self.parent().or_else(|| self.visual_parent()),
    }
}
```

フォント解決は常に `InheritanceParentKind::Visual` を渡す(指示書§13:「フォント継承をLogical Treeで行ってはならない」)。`Logical`はLogical Parentが無ければVisualへフォールバックする(WinUI3の`GetInheritanceParentInternal()`と同じ挙動)。ルートクラスの`#[overridable]`なので、Popup/Portal/ControlTemplateのような要素が将来オーバーライドできる窓口として機能する(実配線は未対応、§9参照)。

---

## 3. `ContentControl`のような合成コンポーネントでの扱い

`ContentControl`(および将来のユーザー定義 `component X inherits Control`)は `TextStyleOwner` を**直接実装しない**——実装しているのは埋め込まれた `base: Control` フィールドの方だけである。このため `elwindui-codegen` が生成するsetter呼び出しは、`TextStyleOwner::set_font_size(&*(receiver), ..)` のようなUFCSではなく、**必ず `UIElementExt::as_text_style_owner(&*(receiver))` を経由**する:

```rust
elwindui::core::ui::UIElementExt::as_text_style_owner(&*(receiver))
    .expect("...")
    .set_font_size(value);
```

`as_text_style_owner`は`#[class]`の祖先転送チェーンの一部(`UIElement`で`#[overridable]`宣言)なので、`ContentControl`が自身でオーバーライドしなくても`Control`の実装へ正しく転送される。これにより `ContentControl` に `impl TextStyleOwner` を個別に書く必要がない。回帰ガードは `crates/elwindui-core/src/ui.rs` の `content_control_inherits_text_style_from_its_base_control` テストと、`crates/elwindui-codegen/src/codegen.rs` の `content_control_declares_seven_text_style_fields_via_control_base` テスト。

---

## 4. 計測シーム(`TextBackend`)

```rust
pub trait TextBackend {
    fn default_text_style(&self) -> ComputedTextStyle;
    fn measure_text(&self, req: &TextMeasureRequest<'_>) -> TextMeasureResult;
}
pub fn set_text_backend(backend: Rc<dyn TextBackend>);
pub fn text_backend() -> Rc<dyn TextBackend>; // 未登録なら DummyTextBackend
```

`thread_local` + `Rc`(既存のシングルメインスレッド前提——`invalidate_host`/`AnyView`と同じ形——に合わせた)。各バックエンドの `init()` が自分の `TextBackend` 実装を登録する。`TextBlock::measure_override` はこの `text_backend()` を呼んで実測する(以前の `chars().count() * 8.0` 固定近似を廃止)。`DummyTextBackend` は意図的にこの旧近似値と同じ数値(0.5×font_size/文字、行高=font_size、既定font_size=16.0)を返すため、既存の core 単体テストの数値は変更していない。

`measure`と`render`は同じ `resolved_text_style()` を(キャッシュせず)毎回再解決するため、「Measureで使ったフォント == Renderで使ったフォント」が構造的に保証される(指示書§21/§22)。

---

## 5. DSL: `#[text_style]` コンポーネント属性

`crates/elwindui-codegen/src/text_style.rs`(新規)に7フィールドの定義を集約。`.elwind`パーサ(`parser.rs`)が `#[text_style]` を認識すると、パース時点でこの7フィールドをコンポーネント自身のフィールドより前に注入する。

`builtins.elwind` で `#[text_style]` を付けたのは **`Control`・`TextBlock`・`NativeControl`** の3箇所(個別のBu tton/TextArea/TextBox/PasswordBoxには付けていない)。理由: `elwindui-codegen`の`emit_field_setter_call`はUFCS経由のディスパッチを行うが、`Button`に付けた場合`declaring_types["font_size"] == "Button"`になり`ButtonExt`をスコープに入れて呼ぼうとして失敗する(実際のRust実装は`NativeControl`が持つ)。`NativeControl`に付けることで`ScrollView`/`TabView`にも7プロパティが付くが、これはWinUI3の`Control`派生全体がフォントプロパティを持つのと同じ挙動であり許容している(ユーザー確認済み)。

`TextBlock`の旧 `color: Option<Color>` フィールドは廃止し、`foreground`(`Option<Brush>`)に統合した。DSLでの `color:` は使えなくなり、`foreground:` を使う(`examples/notepad-inline`の唯一の使用箇所を更新済み)。

`foreground: "#3a3a3c"` のようなhexリテラルは既存の `coerce_color_literal` 機構がそのまま処理する——`#[text_style]`の`foreground`型文字列を`Shape.fill`と完全に同じ `Option<elwindui::core::graphics::Brush>` にしたため、codegen側の変更は不要だった。

### `resolve_effective_fields`/`resolve_field_declaring_types` の免除

`ContentControl`のような自前の`view`を持つコンポーネントは、baseの継承フィールドのうち「`#[routed]`か、`UIElement`直宣言か、view内でbare参照されているもの」だけを引き継ぐ既存フィルタがある。このフィルタに`Attr::TextStyle`も免除対象として追加しなければ、`ContentControl`は7プロパティを黙って失う(コンパイルエラーにはならず、実行時にsetterが存在しないだけ)——これが今回の実装で最も踏みやすい落とし穴だった。

---

## 6. バックエンド別対応状況

| プロパティ | AppKit | WinUI3 | GTK4 |
|---|---|---|---|
| `font_family` | ✅ 実装・テスト済み | ✅ フォールバック列・system復帰を含め実行テスト済み | ❌ 未実装(バックエンド自体が20行スタブ) |
| `font_size` | ✅ | ✅ | ❌ |
| `font_weight` | ✅ | ✅ | ❌ |
| `font_style`(italic) | ✅ | ✅ | ❌ |
| `font_stretch` | ✅ | ✅ | ❌ |
| `character_spacing` | ✅(TextBlock/Button/TextField。TextView/TextAreaの編集可能テキストは未対応、後述) | ✅ | ❌ |
| `foreground` | ✅(Solidは正確。Gradient/Imageはフラット色へ縮退) | ✅(Solid Brush) | ❌ |
| 実測(`measure_text`) | ✅ `NSAttributedString.boundingRectWithSize:options:context:` | ✅ XAML `TextBlock`スクラッチ計測 | ❌ |
| 既定フォント取得 | ✅ `NSFont::systemFontOfSize(NSFont::systemFontSize())` | ✅ `XamlAutoFontFamily()` | ❌ |

### AppKit実装詳細(`crates/elwindui-backend-appkit/src/render/text.rs`)

- `ns_font(&ComputedTextStyle) -> Retained<NSFont>`: システムファミリ(`FontFamily::is_system()`)は`NSFont::systemFontOfSize`起点、指定ファミリは`NSFontDescriptor::fontDescriptorWithFamily`起点。どちらも`NSFontTraitsAttribute`(weight/width)と`symbolicTraits`(italic)を`fontDescriptorByAddingAttributes`/`fontDescriptorWithSymbolicTraits`で追加してから`NSFont::fontWithDescriptor_size`で実体化。後者が`nil`を返すフォント記述子では italic 化前の記述子を使い、解決に失敗したら`NSFont::systemFontOfSize`にフォールバックする(指示書§31:フォント不在でクラッシュしない)。
- `secure_text_font(&ComputedTextStyle) -> Retained<NSFont>`: `NSSecureTextField`専用。パスワードマスクはAppKit内部グリフで描画されるため、指定された`FontFamily`とフォールバック列を使わず、システムフォント基準でsize/weight/stretchを適用する。descriptor経由の italic 合成は内部マスクグリフを欠落させるため、PasswordBoxでは安全側に倒して適用しない。これによりマスク文字が missing glyph になることを防ぐ。
- 指定ファミリ名が存在しない場合、AppKit自身は「最も近い」フォントへ黙って置換するため、実体化したフォントの`familyName()`が要求名と一致するか確認し、一致しなければ次のフォールバック候補(`FontFamily::families()`の次のカンマ区切り要素)へ進む。
- weight変換(`nsfont_weight`): `FontWeight(u16)`の100〜900を、Appleの`NSFontWeight*`名前付き定数(UltraLight=-0.8 〜 Black=0.62)に対応する9点の間で**線形補間**する。可変フォントの中間値(450等)もなめらかに変換できるのがこの補間の狙い——`FontWeight`を数値型にした直接の理由。
- stretch変換(`nsfont_width`): `FontStretch::percent()`(50〜200%)を`NSFontWidthTrait`(-1.0〜1.0)へ線形変換。
- `character_spacing`(1/1000 em)→ `NSKernAttributeName = spacing / 1000.0 * font_size`(ポイント換算はAppKit側で1度だけ行う。WinUI3は逆に無変換で渡せる——`CharacterSpacing`自体が1/1000 em単位のため)。
- 描画は`CATextLayer.setString:`に`NSAttributedString`を渡す方式に変更した。**`NSAttributedString`を設定すると`CATextLayer`自身の`font`/`fontSize`/`foregroundColor`/`alignmentMode`は無視される**ため、旧来の`setFontSize(14.0)`等の呼び出しは削除した(残すと「黙って死んでいる第二の真実の源」になるため)。整列(`TextAlignment`)は`NSMutableParagraphStyle`経由でアトリビュートに含める。
- Retina対応: `CATextLayer`自体は`contentsScale`を自前で設定しない(2026-08、Issue #18)。`host::replay`が`CATextLayer`を`layer`へ追加する際、`render::add_sublayer_scaled`が親`layer`の`contentsScale`(`TreeHostView::backing_scale_factor`——`NSWindow.backingScaleFactor`起点、未アタッチ時は`NSScreen.mainScreen`にフォールバック)を再帰的に子へ伝播する共通機構を通す。以前は`text_layer.setContentsScale(layer.contentsScale())`を個別に呼んでいたが、`layer`(`RenderGroup`のコンテナ`CALayer`)自体が`CALayer::new()`直後で常に`1.0`のままだったため実質無効だった——このバグがRetinaでのテキストぼやけの直接原因。
- Gradient/Image foregroundは`first_gradient_stop_color`によるフラット色へ縮退させている(`render::paint::apply_fill`の既存の同種の縮退と同じ精神)。マスクベースのグラデーション文字描画(`try_add_gradient_fill_layer`の`CATextLayer`マスク化)は本パスでは実装していない——未対応事項として§9に記録。

### WinUI3実装・検証詳細(`crates/elwindui-backend-winui3/src/render/text.rs`)

- `windows-bindgen`が生成した`Windows::UI::Text`の型を`bindings::winui_text`として一元利用し、XAML側の`FontWeight`/`FontStyle`/`FontStretch`と型を揃えた。数値`FontWeight(u16)`は値を失わずに変換する。
- `FontFamily`は先頭候補だけを抜き出さず、`"Consolas, Segoe UI"`のようなカンマ区切り列全体を`XamlFontFamily::CreateInstanceWithName`へ渡す。`FontFamily::system()`は適用のたびに`XamlAutoFontFamily()`へ変換して設定するため、名前付きフォントから既定フォントへ切り替えても以前の値が残らない。
- `TextBlock`描画、スクラッチ`TextBlock`による計測、ネイティブ`Button`/`TextBox`/`PasswordBox`/`TextArea`は同じ`apply_text_style_to_control`経路で7項目を設定する。ネイティブコントロールの適用済みスタイルキャッシュは、WinRT呼び出しが成功したときだけ更新する。
- Windows上の単一`Application`ホスト回帰テストで、7項目のXAML round-trip、フォールバック列、名前付き→system切替、サイズと字間による計測変化、存在しないフォントの安全なフォールバックを検証している。

### ネイティブコントロールへの反映(pull方式)

`native_ui::control::NativeControl`(AppKitの`Button`/`TextArea`/`TextBox`/`PasswordBox`/`ScrollView`/`TabView`共通の基底)に`text_style: TextStyleStorage`と`applied: RefCell<Option<ComputedTextStyle>>`を追加した。`measure_override`の中で毎回`sync_text_style()`を呼び、解決済みスタイルが前回と異なる場合だけ`AnyView::apply_text_style`(→`AppKitHandle::apply_text_style`)を叩く——**push方式ではなくpull方式**にした理由:

1. `#[class]`マクロは`struct_only`クラス(`NativeControl`はこの形)で`#[overridable]`が正しく動かない既知の制約がある——根本原因と、マクロを修正しない方針をユーザーと確認した経緯は`docs/elwindui_macro_class_spec.md`§14に詳述した。要約: `struct_only`は既存の(他クレートで宣言済みの)トレイトを実装するだけで自分専用の新トレイトを持たないため、`ordinary`/`root`クラスのように`#[overridable]`メソッドごとのアクセサを自由に追加できない。マクロへ補助トレイト自動生成を足す案も検討したが、`Button`/`TextArea`など子孫も`struct_only`であるため祖先転送チェーンへの組み込みが複数箇所に波及し、既存の複雑さに見合わないと判断——**マクロは修正せず、基底クラスから最派生オブジェクトへ直接プッシュする経路を作らない**設計(下記のpull方式)を採用した。
2. `UIElementExt::measure`は毎レイアウトパスで無条件に`measure_override`を再実行するため、プル方式でも取りこぼしがない。

`AppKitHandle`トレイト(`ffi.rs`)に`apply_text_style`/`supports_text_style`のデフォルト実装(no-op/false)を追加し、`Retained<NSButton>`/`Retained<NSTextField>`/`Retained<NSScrollView>`の3つの実ハンドル型に実装した(`Retained<NSStackView>`は既定no-opのまま)。

| ハンドル | font | foreground | character_spacing |
|---|---|---|---|
| `NSButton` | `setFont:` | `character_spacing != 0`のときだけ`setAttributedTitle:`経由(通常の`title`/`setFont:`ペアではカーニング/前景色を表現できないため。頻繁に発生しない限り、ベゼルスタイルの既定ティントを崩さない) | 同上 |
| `NSTextField`(TextBox) | 指定FontFamilyとフォールバックを含む`setFont:` | `setTextColor:` | 未対応(編集可能な`stringValue`への属性付け替えは編集中の挙動と衝突するため、意図的に対象外) |
| `NSSecureTextField`(PasswordBox) | システムフォント基準の`setFont:`（size/weight/stretch。italic は内部マスク保護のため適用しない） | `setTextColor:` | 未対応。内部マスクグリフを守るためFontFamilyとcharacter_spacingは意図的に無視する |
| `NSScrollView`(TextArea) | `documentView()`を`NSTextView`へdowncastして`setFont:` | `setTextColor:` | 未対応(同上の理由。`NSLayoutManager`の一時属性か`textStorage`全体への属性適用が必要で、本パスの対象外) |

`InnerTextArea::default_width`/`default_height`は構築時に一度だけフォントメトリクスから算出され、以後フォントが変わっても再計算されないバグがあった。`Cell<f32>`化し、`native_ui::TextArea::measure_override`から`sync_text_style()`の直後に`InnerTextArea::refresh_default_size()`を呼ぶよう修正した。

### Cargo.toml変更

`objc2-app-kit`に`NSAttributedString`/`NSStringDrawing`/`NSParagraphStyle`/`NSFontDescriptor`/`NSColor`を追加。`objc2-foundation`に`NSAttributedString`を追加。**`objc2-core-text`は追加していない**——`NSFontDescriptor`のtraits辞書だけでweight/width/italicを表現できるため。

---

## 7. TextBlockの計測・描画

`crates/elwindui-core/src/ui.rs`の`TextBlock`:

- 構造体: `color: RefCell<Option<Color>>` を削除し `text_style: TextStyleStorage` を追加。
- `measure_override`: `resolved_text_style()` → `text_backend().measure_text(&TextMeasureRequest { .. })` で実測。`wrapping`は常に`TextWrapping::NoWrap`(下記未対応事項)、`scale`は常に`1.0`(DPI概念なし)。
- `render`: 同じく`resolved_text_style()`を(measureの結果をキャッシュせず)再解決してから`context.draw_text(..)`へ渡す。1レイアウトパス内では状態が変化しないため2回の解決結果は必ず一致する。

---

## 8. 変更通知・無効化

`TextStyleOwner::on_text_style_property_changed`の既定実装:

```text
Foreground の変更   -> invalidate()          (再描画のみ)
それ以外6項目の変更 -> invalidate_measure()  (再計測+再描画)
```

各`TextStyleStorage`のsetterは値が実際に変わったときだけ`true`を返すため、同値の再設定では通知・無効化とも発生しない(指示書§23)。キャッシュ機構(世代番号等)は実装していない——`UIElementExt::measure`が毎パス無条件に`measure_override`を再実行する既存の設計により、`clip_to_bounds`と同様、キャッシュ無しの再帰解決で正しさを保てるため、v1では正しさを優先し最適化は先送りにした(指示書§24の「初期実装では正確な無効化を優先」の指示通り)。

---

## 9. 未対応事項(明示)

- **GTK4バックエンド全体** — `crates/elwindui-backend-gtk4`はgtk4/pango依存すら無い20行のスタブ。フォント対応の抽象化(`TextBackend`等)はGTK4実装可能な形にしてあるが、実装コードは書いていない。
- **WinUI3の適用粒度** — 本機能はWindows上で検証済みだが、elwindui のツリーはXAMLツリーそのものではない(`Control`/`Grid`はXAMLピアを持たない仮想ビルトイン、ネイティブリーフは`Canvas`のフラットな子)。そのため指示書§18の「未設定のプロパティはDependencyPropertyを設定しない」は文字通りには実装できず、解決済み`ComputedTextStyle`を常に適用する設計を採る(§10参照)。
- **DPI / 表示スケール / テキストスケール(計測・レイアウト層)** — elwindui-coreにこの概念自体が存在しない。`TextMeasureRequest::scale`は常に`1.0`(意図的にこのまま——スケールをcoreの計測へ持ち込むとモニタごとにレイアウトサイズが変わってしまう)。該当する指示書§32の動的変更テスト(29・30番)は実施不能。**なお2026-08(Issue #18)、AppKitバックエンドの*描画解像度*側は別途対応した**——`render::add_sublayer_scaled`/`render::layer`が`CALayer.contentsScale`を`TreeHostView::backing_scale_factor`から再帰的に伝播し、`TextBlock`を含む全描画がRetinaで正しい解像度になる。これは計測に影響しないバックエンド内部のラスタライズ詳細であり、上記の「未対応」の対象外。
- **`TextBlock.text_wrapping`** — 決定した7プロパティの範囲外のため、DSLフィールドとしては追加していない。`TextMeasureRequest`に`wrapping: TextWrapping`の枠は用意済みなので、追加時のシグネチャ変更は不要。
- **`Brush::Image`のforeground** — AppKit側はフラット色へのフォールバックのみ(グラデーションと同じ縮退)。
- **`ScrollView`/`TabView`のホスト済みコンテンツへの継承** — これらは別の`TreeHostView`インスタンスにコンテンツをホストするため、visualチェーンがそこで途切れる。`InheritanceParentKind`を要素ごとにオーバーライド可能にする仕組み(§2)は将来この問題を解く手段になるが、ホスト済みルートに実際の配線をする作業は本実装のスコープ外。
- **Popup/ControlTemplateの継承境界** — `inheritance_parent(kind)`が`#[overridable]`である設計上は拡張可能だが、実際にオーバーライドする要素は存在しない(指示書§28で要求されている「テンプレート内部のフォント継承」も同様に未配線)。
- **`ComputedTextStyle`の世代番号キャッシュ** — 正しさを優先し、v1では未実装(§8参照)。
- **AppKitの`NSTextField`/`NSTextView`(TextBox/PasswordBox/TextArea)のcharacter_spacing** — 上記§6の表の通り、編集可能なネイティブテキストへのカーニング適用は対象外。
- **`CATextLayer`実グリフレンダリングのピクセルゴールデンテスト** — §11参照。非メインスレッド(`cargo test`のワーカースレッド)でのデッドロックリスクが確認されたため撤去した。計測ベースの単体テストで代替。

---

## 10. 指示書からの意図的な逸脱(要確認事項・ユーザー承認済み)

| 項目 | 指示書の記述 | 採用した実装 | 理由 |
|---|---|---|---|
| `FontWeight`の型 | 記述なし(WinUI3の`FontWeight`型を参考にとだけ言及) | `enum`ではなく数値ニュータイプ`FontWeight(pub u16)` | CLAUDE.mdの「enumが唯一の値集合機構」ルールに近接するため確認の上で決定。可変フォント中間値の表現に必要 |
| WinUI3 §18 | 「未設定のローカル値はDependencyPropertyを設定しない」 | 解決済み値を常にpush、「XAML既定値と異なる値だけ書く」と再解釈 | elwindui のツリー構造がXAMLツリーと一致しないため文字通りの実装は誤動作する |
| `#[text_style]`の付与位置 | 記述なし(個別プロパティの実装を指示) | `Button`等の個別リーフではなく`NativeControl`に付与 | `emit_field_setter_call`のUFCSディスパッチが`declaring_types`ベースであるため、個別リーフに付けると`E0599`になる |

---

## 11. テスト

新規テスト(既存テストは全て変更なしで継続グリーン):

- `crates/elwindui-core/src/graphics/text.rs`: 値型・`TextStyleStorage`・`DummyTextBackend`の単体テスト(14件)
- `crates/elwindui-core/src/ui.rs`: 継承・`TextStyleOwner`・`inheritance_parent`・invalidateの単体テスト(12件、`content_control_inherits_text_style_from_its_base_control`を含む)
- `crates/elwindui-codegen/src/parser.rs`: `#[text_style]`のパース・注入・拒否ルール(3件)
- `crates/elwindui-codegen/src/validate.rs`: builtin外拒否・重複フィールド拒否(2件)
- `crates/elwindui-codegen/src/codegen.rs`: setterディスパッチ・hexリテラル変換・`ContentControl`の`declaring_types`、動的`FontFamily`/`Brush`の所有値・`Some(Brush)`展開
- `crates/elwindui-backend-appkit/src/render/text.rs`: `ns_font`のサイズ/weight/italic/フォールバック、計測の伸長・折り返し・カーニング(9件、実機で実行・検証済み)
- `crates/elwindui-backend-winui3/src/render/text.rs`: 数値weight、全style/stretch変換の単体テスト
- `crates/elwindui-backend-winui3/src/inner/button.rs`: 単一WinUI `Application`ホストでの7項目XAML round-trip、フォールバック列、system復帰、計測変化、不在フォントの回帰テスト
- `examples/font-demo`: System / Display / Monoの3プロファイルを実行時に切り替え、`TextBlock`と`Button`/`TextBox`/`PasswordBox`/`TextArea`への7項目の反映を確認するためのデモ
- `crates/elwindui-backend-appkit/src/testsupport/golden.rs`のピクセルレベルインク被覆率ゴールデン(既定/bold+large/kerned)は**実装したが撤去した**——`CATextLayer`による実際のグリフラスタライズ(`renderInContext:`)が、`cargo test`のワーカースレッド(実際のAppKitメインスレッドではないことを`MainThreadMarker::new()`が`None`を返すことで実証確認済み)上で断続的にデッドロックしたため。`NSAttributedString.boundingRectWithSize:options:context:`によるテキスト**計測**(グリフを実際にラスタライズしない)は同じ非メインスレッド環境で毎回安定して高速動作しており、`render/text.rs`側の単体テストがこの問題の影響を受けないのはこの違いによる。本番コードは`elwindui-backend-appkit::app::run`経由で常に実際のメインスレッド+動作中のランループ上で実行されるため実害は無いが、テストハーネス自体の制約としてピクセルゴールデンは断念した。実機での見た目確認は`tools/macos-ui-driver`でのスクリーンショット検証(§0参照)で代替する。

`cargo build --workspace`/`cargo test --workspace`は全てグリーン。`rust-analyzer diagnostics .`も実行済み(詳細は本ドキュメントの更新履歴、または実装コミットのコミットメッセージを参照)。
