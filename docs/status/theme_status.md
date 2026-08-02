# ElwindUIテーマ実装状況

最終更新: 2026-07-26

## 1. 正規のRust構文

テーマは`.elwind`へ追加せず、Rust属性と式マクロだけで定義・参照する。

```rust
#[elwindui::theme_definition(
    extends = SystemTheme,
    variants(Default, Ocean)
)]
struct AppTheme {
    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(0, 80, 120))
    )]
    layout_background: Brush,

    #[theme(default = Brush::Solid(Color::rgb(39, 103, 216)))]
    brand: Brush,
}

// view! 内
background: theme!(AppTheme::layout_background)
```

当初案の外側属性`#[elwindui::theme]`と参照マクロ`theme!`は、Rustの同一macro
namespaceで同名のattribute macroとfunction-like macroを同時公開できないため採用しない。
ユーザー確認に基づき、外側属性を`theme_definition`、field属性を`theme`、参照を
`theme!`とする構文を正式仕様とした。

## 2. 実装済み

- 型付き`ThemeToken<T>`、`ThemeValue<T>::Value/PlatformDefault`、
  `ThemeController<T>`、`ThemeHandle`、`ThemeContext`。
- `ThemePreference::{System, Light, Dark}`、
  `ThemeAppearance::{Light, Dark, HighContrast}`、
  `ThemeChangeImpact::{Paint, Measure, NativeStyle}`。
- application既定テーマと`Window.theme`によるWindow単位の上書き。visual parentを通じて
  WindowのThemeContextを解決する。
- `#[elwindui::theme_definition]`によるvariant enum、controller、型付きtokenの生成。
  未知・重複variant、独自tokenのdefault欠落をコンパイルエラーにする。
- ElwindUIの型階層に沿った標準token manifest。`panel`、`surface`、`input`は標準名に
  含めない。独自token名にはこの制約を適用しない。
- 具象標準tokenがtheme structに未宣言の場合の基底token fallback。具象tokenに
  `platform_default`が明示された場合は基底tokenへ進まず、そこでplatform既定へ戻る。
- `theme!`を使う属性だけをtheme revision時に再同期するコード生成。
  `Value`はsetter、`PlatformDefault`は`clear_*`経路へ展開する。
- テキストcascadeを`CascadedTextStyle`（未解決値を`Option`で保持）と
  `ComputedTextStyle`（描画用の具体値）へ分離。
- Layout背景。未指定時は透明であり、指定時はarranged boundsへ子要素より先に描画する。
- WinUI 3のfont/foreground/backgroundの`SetValue`相当と`ClearValue`復帰。
  `RequestedTheme`反映、`ActualThemeChanged`購読。
- AppKitのシステムfont、`NSColor.labelColor`、layer背景への復帰、および
  Aqua/DarkAqua/System appearance要求。
- 公開APIおよびマクロ生成される公開型・variant・tokenの英語rustdoc。
- `examples/theme-demo`。appearanceとvariantを独立して切り替え、
  Layout、Shape、TextBlock、native control、Menu、TabView、独自`brand` tokenを確認できる。

## 3. 値の優先順位

実装上の解決順は次の通り。

1. 要素の明示値
2. visual parentからcascadeした値
3. 要素固有theme token
4. ElwindUI基底型のtheme token
5. backend既定値

`PlatformDefault`は具体値へ早期変換しない。WinUI 3ではDependencyPropertyを
`ClearValue`し、AppKitではdynamic system color/system fontへ戻す。

## 4. 検証

- `cargo test --workspace`（2026-07-26、Windows、全件成功）。
- core/codegen/macro単体テスト。
- WinUI 3単一Applicationホストテスト（text style round-tripとClearValue復帰、15件成功）。
- `cargo build -p theme-demo`、`cargo check -p font-demo -p theme-demo`。
- `cargo doc -p elwindui-core -p elwindui-macros -p elwindui --features backend-winui3
  --no-deps`（broken intra-doc linkなし）。
- `tools/test-theme-demo-uia.py`によるWindows UI Automationテスト。
  Ocean → Solarized → Default → Dark → Light → Systemを`InvokePattern`で操作し、
  variant/appearanceラベル、revision増加、disabled control、nested TabViewを検証する。
- `rust-analyzer diagnostics .`は実行済みだが、`#[class]`／`#[viewmodel]`／`#[component]`
  の既知の解析差異により、既存exampleと新規`theme-demo`のマクロ呼び出し位置で
  `Weak<{unknown}>`／`Rc<{unknown}>`のE0282を報告する。rustcによる同じ生成コードの
  workspace build/testは成功している。既存のclass macro解析設計は
  `docs/elwindui_macro_class_spec.md` §15を参照。

## 5. 残る制限

- GTK4 backend本体がstubのため、theme traitの消費実装も未実装。
- AppKitのappearance監視コードは実装済みだが、このWindows環境ではmacOS実機ビルド・
  Aqua／DarkAqua切替の操作検証は行っていない。
- WinUI 3のHigh Contrast判定は型とtoken値を保持できるが、OSの
  `AccessibilitySettings.HighContrastChanged`との接続は未実装。
- Shadow/Elevation、Animation/Easing、theme別画像・iconは共通runtime型が未実装のため
  token対応を先行追加していない。
- selection/caret/focusなど、現時点でElwindUIの公開setterが存在しないnative state tokenは
  manifestとfallback規則を定義済みだが、個別DependencyPropertyへの明示適用は未実装。
