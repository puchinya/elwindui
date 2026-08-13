# ElwindUI Theme and Environment Specification

## 1. Scope

本仕様はEnvironment value、lookup、override、および Environment valuesをまとめて設定するPresetとしてのThemeの公開contractを定義する。runtimeの実現方式は [`../design/runtime/theme_environment_design.md`](../design/runtime/theme_environment_design.md) を参照する。

## 2. Environment

- Environment valueはUI subtreeへ公開値を提供し、最も近い有効なoverrideから解決される。
- lookupは公開UIの親子関係を保持し、backend補助hostの存在によって観測結果が変わってはならない。
- valueの設定・clearによってeffective valueが変わったsubtreeは、影響種別に応じて再同期される。
- Environment値は値型ではなく `EnvironmentKey` で識別する。`EnvironmentKey` は `type Value: Clone + 'static` と `fn default_value() -> Self::Value` を持つ。同じ `Value` 型を複数のKeyで共有してよい。
- `#[elwindui::environment_key(name, value, default)]` はKey型を生成する公開定義方法である。DSL側の消費経路(`#[environment(name)]` field attribute、`EnvironmentScope`)は [`dsl_spec.md`](dsl_spec.md) を参照。
- 共有 `EnvironmentContext`(`Clone`)が型付きlookupを提供する: `get<K: EnvironmentKey>(&self) -> K::Value`、`set<K>(&self, value: K::Value)`、`derive(&self) -> EnvironmentContext`。内部storageはKeyの型消去(`TypeId`等)を用いてよいが、文字列によるruntime lookupは提供しない。
- Environment entryはreactive cellとして保持する。overrideされていないKeyは親のcellをそのまま共有し、overrideされたKeyのみ新しいcellを持つ——`derive()`はこの共有・分岐を行う。
- Environment解決はVisual Treeへのattachに依存しない。Component生成時に親から渡された `EnvironmentContext` を用いてbody/`view!`を評価し、child Componentの生成へそのcontextを伝播してからUIElementを生成し、最後にVisual Treeへattachする。
- `EnvironmentScope` はUIElement・Render nodeを生成しない。親Environmentをderiveし、指定したKeyのみ上書きした派生Environmentをchildrenの生成へ渡す。
- EnvironmentとThemeの責務は分離する。Theme(§3–§6)はEnvironmentのlookup/継承機構を再定義せず、`EnvironmentContext`のoverride経路を呼び出すのみである。

## 3. Theme

- Themeはresource containerでもtoken lookup systemでもない。Environment valuesをまとめて設定する **Preset** である(Issue #96)。
- UIコードはTheme型を直接参照しない。Themeが設定したEnvironment値(`#[environment(name)]`)またはsemantic valueを参照する。
- Theme適用は概念的に「Environment overrideの一括適用」である: `trait Theme { fn apply(&self, env: &EnvironmentContext); }`。`EnvironmentContext::set`(§2)を直接呼び出す以外の専用overrides型を公開APIに導入しない。

## 4. Theme definition

- `#[elwindui::theme] struct Name { #[theme(value = expr)] field: Type, .. }` はTheme Presetを生成する公開定義方法である。
- 各fieldの識別子は、同一crate内で先に宣言された `#[elwindui::environment_key(name = <field識別子>, ..)]` のKeyへ解決される。解決できないfield名はcompile-time errorとなる(`#[environment(name)]` と同じ解決規則、[`dsl_spec.md`](dsl_spec.md) §13参照)。
- `value` 式の型はそのKeyの `Value` 型と一致しなければならない。不一致はcompile-time errorとなる。
- Themeはvariantを持たない。異なる見た目ごとに別個の `#[elwindui::theme]` 型(またはインスタンス)を定義する。「切り替え」は同一 `EnvironmentContext` へ別のTheme instanceを適用することであり、1つの型が持つvariant selectionではない。

## 5. Theme application boundary

- Themeはapplication-level(`EnvironmentContext::application_environment()`、§2の一種)へ適用できる。
- Window単位のTheme override(旧`Window.theme`)は提供しない。任意のsubtree単位でのTheme適用は `EnvironmentScope` (§2) が実装され次第、その仕組み経由で提供される。
- Theme適用はEnvironmentの通常のoverride・通知経路(§2)をそのまま用いる。Theme専用のrevision counterや変更影響分類は存在しない。

## 6. Application to UI properties

- Theme適用の効果はEnvironment値のoverrideとして観測される。公開propertyの意味はTheme適用によって変化しない。
- NativeControlの既定外観(背景色・フォント等)はThemeの適用対象ではない。ElwindUIはNativeControlへ既定のnative外観以外の値を自動供給しない(#96時点)。個別のNative Control外観のEnvironment経由での上書きは別仕様(Native Style / Control Style)の対象である。
