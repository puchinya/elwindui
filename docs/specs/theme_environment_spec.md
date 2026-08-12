# ElwindUI Theme and Environment Specification

## 1. Scope

本仕様はEnvironment value、Theme definition、variant、token、appearance、lookup、override、platform defaultの公開contractを定義する。runtimeの実現方式は [`../design/runtime/theme_environment_design.md`](../design/runtime/theme_environment_design.md) を参照する。

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
- EnvironmentとThemeの責務は分離する。Themeの解決方式(§3–§7)はEnvironmentのlookup/継承機構を再定義しない。

## 3. Theme definition

- `#[elwindui::theme_definition]` はvariant enum、controller、型付きtokenを生成する公開定義方法である。
- 未知variant、重複variant、必要なdefaultを持たないcustom tokenはcompile-time errorとなる。
- standard tokenはElwindUIの公開型階層に沿う。custom tokenにはstandard tokenの命名制約を適用しない。
- `theme!` はtoken参照をproperty valueとして利用するDSL接続点である。

## 4. Theme values and lookup

- `ThemeValue<T>::Value(T)` は解決済みの具体値を表す。
- `ThemeValue<T>::PlatformDefault` は共通層で具体値を決めず、対象backendの既定値へ戻すことを表す。
- application themeが既定contextであり、`Window.theme` はそのWindow subtreeを上書きする。
- concrete standard tokenがvariantにない場合は対応するbase tokenへfallbackする。
- concrete tokenに `platform_default` が明示されている場合、base tokenへ進まずその地点でplatform defaultへ解決する。

## 5. Appearance and variant

- theme variantとOS appearance preferenceは独立した軸である。
- `ThemePreference` は `System`、`Light`、`Dark` を表す。
- `ThemeAppearance` は少なくとも `Light`、`Dark`、`HighContrast` を表す。
- `System` は現在のOS appearanceへ追従し、観測可能なappearance変更をsubtreeへ通知する。

## 6. Application to UI properties

- `theme!` の `Value` は通常のsetter semanticsで適用される。
- `PlatformDefault` は対応するclear/reset経路で適用される。
- Theme値は公開propertyの意味を変更しない。Styleはproperty集合、Themeはその値の供給源である。
- 自前描画要素とNativeControlは、同じtokenが適用可能な公開propertyについて同じ解決結果を観測する。
- native APIが公開setterを持たないstateはTheme適用対象にならない。その対応状況はstatusに記録する。

## 7. Change impact

Theme変更はpropertyごとの影響に従い、paint、measure、native styleの必要な範囲だけを更新する。値が変化していないpropertyを無条件に再構築してはならない。
