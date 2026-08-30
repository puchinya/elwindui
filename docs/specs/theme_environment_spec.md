# ElwindUI Theme and Environment Specification

## 1. Scope

本仕様はEnvironment value、lookup、override、および Environment valuesをまとめて設定するPresetとしてのThemeの公開contractを定義する。runtimeの実現方式は [`../design/runtime/theme_environment_design.md`](../design/runtime/theme_environment_design.md) を参照する。

## 2. Environment

- Environment valueはUI subtreeへ公開値を提供し、最も近い有効なoverrideから解決される。
- lookupは公開UIの親子関係を保持し、backend補助hostの存在によって観測結果が変わってはならない。
- valueの設定・clearによってeffective valueが変わったsubtreeは、影響種別に応じて再同期される。
- Environment値は値型ではなく `EnvironmentKey` で識別する。`EnvironmentKey` は `type Value: Clone + 'static` と `fn default_value() -> Self::Value` を持つ。同じ `Value` 型を複数のKeyで共有してよい。
- `#[elwindui::environment_key(name, value, default)]` はKey型を生成する公開定義方法である。DSL側の消費経路(`#[environment(name)]` field attribute、`EnvironmentScope`)は [`dsl_spec.md`](dsl_spec.md) を参照。宣言元とは別のcrateからも、完全修飾クレートパス構文(Issue #129)で同じ消費経路から参照できる——実現方式は [`../design/tools/environment_key_macro_design.md`](../design/tools/environment_key_macro_design.md) を参照。
- 共有 `EnvironmentContext`(`Clone`)が型付きlookupを提供する: `get<K: EnvironmentKey>(&self) -> K::Value`、`set<K>(&self, value: K::Value)`、`derive(&self) -> EnvironmentContext`。内部storageはKeyの型消去(`TypeId`等)を用いてよいが、文字列によるruntime lookupは提供しない。
- Environment entryはreactive cellとして保持する。overrideされていないKeyは親のcellをそのまま共有し、overrideされたKeyのみ新しいcellを持つ——`derive()`はこの共有・分岐を行う。
- Environment解決はVisual Treeへのattachに依存しない。Componentは`mount`時に確立された`EnvironmentContext`を用いて自身の`#[environment(name)]` fieldを解決し、その後にbody/`view!`を評価してUIElementを生成する。child Componentへは、`mount`時に確立された(または`EnvironmentScope`が派生させた)contextを明示的に伝播してから、そのchildをmountする。
- `template: template_view!(|alias: Self| { ... })`を宣言するControl-derived componentは、mount時に確立したEnvironment contextを使い、最初のtemplate適用時に型付き`Option<ControlTemplate<Component>>`を一度だけ解決する。最初の適用前のKey変更はその適用時に反映され、適用後のKey変更はmount済みControlを再テンプレート化しない。詳細は[`control_template_spec.md`](control_template_spec.md)を参照する。
- `EnvironmentScope` はUIElement・Render nodeを生成しない。親Environmentをderiveし、指定したKeyのみ上書きした派生Environmentをchildrenの`mount`へ渡す。
- `context_popup`（`ui_spec.md`）の内容は、owner要素の有効なEnvironmentから`derive()`したpopup専用のEnvironmentContextで構築される。owner自身のEnvironmentは変更しない。この派生Contextには`crate::ui::popup::PopupDismissActionKey`（`Value = Option<PopupDismissAction>`）が`ContextMenuService::open_custom_popup`によって`Some(..)`として設定され、popup content内から宣言的にpopupを閉じる手段を提供する——詳細は[`../design/runtime/popup_context_menu_design.md`](../design/runtime/popup_context_menu_design.md) §6を参照。`PopupDismissActionKey`の**既定値**（`EnvironmentKey::default_value()`）は`None`であり、フレームワークのDSL管理経路（popup機構自体）はpopup scopeの外側でこれを`Some(..)`にすることはない。ただし、これは「popup外では常にNone」という絶対的な保証ではない——低レベルの型付きRust API（`EnvironmentContext::set::<PopupDismissActionKey>(..)`）そのものにはアクセス制御が導入されておらず、任意のRustコードが明示的に`Some(..)`を設定することは可能である。今回の制約はDSL（`#[environment(name)]`/`EnvironmentScope`/`#[elwindui::theme]`）が対応する解決関数のみに適用される: `PopupDismissActionKey`は`#[environment(popup_dismiss)]`による**読み取り**は可能だが、`EnvironmentScope { popup_dismiss: .. }`や`#[elwindui::theme]`フィールドによる**書き込みはできない**（`dsl_spec.md`§4/§13ルール34–36）——`ContextMenuService::open_custom_popup`のみが実際にアクティブな`PopupDismissAction`を設定できるフレームワーク管理値であり、DSL側から上書き可能な通常のEnvironment値ではない。同名のユーザー定義Key（`#[elwindui::environment_key(name = popup_dismiss, ..)]`）を同一crate内で宣言した場合は、そのユーザーKeyが優先され通常通り読み書き可能になる（他の組み込みKey名のシャドーイングと同じ挙動）。
- EnvironmentとThemeの責務は分離する。Theme(§3–§6)はEnvironmentのlookup/継承機構を再定義せず、`EnvironmentContext`のoverride経路を呼び出すのみである。

## 2a. Typed ControlTemplate Environment slots

ControlTemplate selection uses a generic typed Environment slot rather than a
per-control key declaration. For every `C: ControlExt + 'static`, the framework
provides the equivalent of:

```rust
#[doc(hidden)]
pub struct ControlTemplateEnvironment<C: ControlExt + 'static>(
    std::marker::PhantomData<fn() -> C>,
);

impl<C: ControlExt + 'static> EnvironmentKey
    for ControlTemplateEnvironment<C>
{
    type Value = Option<ControlTemplate<C>>;

    fn default_value() -> Self::Value { None }
}
```

The ergonomic API is:

```rust
environment.set_control_template::<C>(Some(template));
environment.set_control_template::<C>(None);
```

and the hidden read path is `__control_template::<C>()`. `Some` overrides the
component's `template: template_view!` default. `None` is an explicit local
entry that shadows an ancestor value and selects the default; it does not
remove the local entry. Lookup is exact by target type/TypeId: a
`ControlTemplate<Base>` never satisfies a `ControlTemplate<Derived>` slot.
There is no string lookup, registry, reflection, covariance, or runtime
re-templating subscription. The same derived-context cell inheritance and
shadowing rules as other Environment keys apply.

## 3. Theme

- Themeはresource containerでもtoken lookup systemでもない。Environment valuesをまとめて設定する **Preset** である(Issue #96)。
- UIコードはTheme型を直接参照しない。Themeが設定したEnvironment値(`#[environment(name)]`)またはsemantic valueを参照する。
- Theme適用は概念的に「Environment overrideの一括適用」である: `trait Theme { fn apply(&self, env: &EnvironmentContext); }`。`EnvironmentContext::set`(§2)を直接呼び出す以外の専用overrides型を公開APIに導入しない。

## 4. Theme definition

- `#[elwindui::theme] struct Name { #[theme(value = expr)] field: Type, .. }` はTheme Presetを生成する公開定義方法である。
- 各fieldの識別子は、**書き込み可能なEnvironment Key解決規則**（`component_frontend::lookup_writable_environment_key`）で解決される: (1) 同一crate内で先に宣言された `#[elwindui::environment_key(name = <field識別子>, ..)]` のKey、(2) 上記に該当しなければ書き込み可能な組み込みKey——Semantic Styleの組み込みKey名(§7)のみ、宣言不要でframework Keyへ解決される。それ以外の解決できないfield名はcompile-time errorとなる([`dsl_spec.md`](dsl_spec.md) §13ルール36参照)。この解決規則は`#[environment(name)]`（読み取り、§2、`component_frontend::lookup_environment_key`）とは**異なる**——読み取り側は上記に加えて`popup_dismiss`（§2）にも解決されるが、Theme・`EnvironmentScope`（§2）はいずれも書き込み可能な集合のみを用いるため`popup_dismiss`には解決されない。
- `value` 式の型はそのKeyの `Value` 型と一致しなければならない。不一致はcompile-time errorとなる。
- Themeはvariantを持たない。異なる見た目ごとに別個の `#[elwindui::theme]` 型(またはインスタンス)を定義する。「切り替え」は同一 `EnvironmentContext` へ別のTheme instanceを適用することであり、1つの型が持つvariant selectionではない。

## 5. Theme application boundary

- Themeはapplication-level(`EnvironmentContext::application_environment()`、§2の一種)へ適用できる。
- Window単位のTheme override(旧`Window.theme`)は提供しない。任意のsubtree単位でのTheme適用は `EnvironmentScope` (§2) が実装され次第、その仕組み経由で提供される。
- Theme適用はEnvironmentの通常のoverride・通知経路(§2)をそのまま用いる。Theme専用のrevision counterや変更影響分類は存在しない。

## 6. Application to UI properties

- Theme適用の効果はEnvironment値のoverrideとして観測される。公開propertyの意味はTheme適用によって変化しない。
- NativeControlの既定外観(背景色・フォント等)はThemeの適用対象ではない。ElwindUIはNativeControlへ既定のnative外観以外の値を自動供給しない(#96時点)。個別のNative Control外観のEnvironment経由での上書きは別仕様(Native Style / Control Style)の対象である。

## 7. Semantic Style

具体的な[`Brush`](graphics_spec.md#5-brush)と、UI上の意味を表すbrush指定を分離する。

```rust
pub enum BrushStyle {
    Value(Brush),
    Primary,
    Secondary,
    Tertiary,
    Foreground,
    Background,
    WindowBackground,
    Tint,
    Selection,
    Separator,
    Placeholder,
    Link,
    PlatformDefault,
}
```

- `BrushStyle::Value` は指定された具体的な`Brush`をそのまま表す。
- 各semantic variantは同名のframework組み込みEnvironment Keyから現在値を読む。組み込みKeyの`Value`型は`BrushStyle`、未override時のdefaultは`BrushStyle::PlatformDefault`である。
- 組み込みKey名は `primary` / `secondary` / `tertiary` / `foreground` / `background` / `window_background` / `tint` / `selection` / `separator` / `placeholder` / `link` とする。これらは `#[environment(name)]`、`EnvironmentScope`、`#[elwindui::theme]` から同一crate内のKey宣言なしで利用できる。
- Environment値は別のsemantic variantを参照してよい。解決は`Value`または`PlatformDefault`へ到達するまで再帰し、循環参照はpanic・無限再帰ではなく`PlatformDefault`へ解決する。
- `BrushStyle::resolve(&EnvironmentContext) -> ResolvedValue<Brush>` は、引数で明示されたeffective Environmentだけを読む。application-globalなambient lookupは行わない。

## 8. ResolvedValue and Brush properties

```rust
pub enum ResolvedValue<T> {
    Value(T),
    PlatformDefault,
}
```

- `ResolvedValue::PlatformDefault` はcoreで固定値へmaterializeしない。Native propertyではbackend/toolkitのclear/default経路へ渡し、self-drawn propertyではそのproperty固有の既定(clear)状態へ戻す。
- `view!` の既存Brush系property `foreground` / `background` / `fill` / `stroke` は`BrushStyle`を受け付け、Componentのmount-time effective Environmentから解決する。
- semantic Brush propertyを含むmount済みComponentは組み込みsemantic Keyの変更を購読し、同じeffective Environmentからpropertyを再解決する。`EnvironmentScope`内ではその派生Contextを用いる。
- 既存の`Brush`、`Color`、16進文字列カラー指定は`BrushStyle::Value`相当として引き続き利用できる。
- Rust setterの既存concrete `Brush` contractは変更しない。Semantic Styleからconcrete setter/clearへの変換はDSL codegen境界で行う。

## 9. 使用例

組み込みKeyは事前宣言せず、そのままTheme fieldと`view!`から利用できる。

```rust
use elwindui::core::environment::application_environment;
use elwindui::core::graphics::{Brush, Color};
use elwindui::core::theme::{BrushStyle, Theme};

#[elwindui::theme]
struct LightTheme {
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::white())))]
    background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::black())))]
    foreground: BrushStyle,
    // semantic role同士をaliasできる。
    #[theme(value = BrushStyle::Foreground)]
    primary: BrushStyle,
}

#[elwindui::component(inherits VerticalLayout)]
struct MyView {
    body: view! {
        VerticalLayout {
            background: BrushStyle::Background,
            TextBlock {
                text: "Hello",
                foreground: BrushStyle::Foreground,
            }
            Rectangle {
                fill: BrushStyle::Primary,
                stroke: BrushStyle::Separator,
            }
        }
    },
}

#[elwindui::component]
impl MyView {}

fn build_view() -> std::rc::Rc<MyView> {
    LightTheme.apply(&application_environment());
    MyView::new()
}
```

subtreeだけを上書きする場合は`EnvironmentScope`を用いる。外側のThemeを変更しても、scope内の`primary` overrideは維持される。

```rust
body: view! {
    EnvironmentScope {
        primary: BrushStyle::Value("#ff0066".into()),
        TextBlock {
            text: "Scoped accent",
            foreground: BrushStyle::Primary,
        }
    }
}
```
