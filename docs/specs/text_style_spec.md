# ElwindUI Text Style Specification

## 1. Scope

本仕様は、テキストを表示するUI要素とNativeControlから観測できるfont、foreground、継承、clear/reset、fallbackのcontractを定義する。内部storageとbackend変換は [`../design/runtime/text_design.md`](../design/runtime/text_design.md) を参照する。

## 2. Public values

- `FontFamily` はカンマ区切りのfallback family listを表す。`FontFamily::system()` / `system-ui` はbackendのsystem font cascadeへ委ねる値であり、特定OSのfamily名を意味しない。
- `FontWeight(u16)` は100～900の標準定数を持ち、中間weightも表現できる。
- `FontStyle` は `Normal`、`Italic`、`Oblique` を表す。
- `FontStretch` は `UltraCondensed` から `UltraExpanded` までの9段階を表す。
- `foreground` は `Brush` である。backendがbrush種別を完全には表現できない場合の現在の対応はstatusに記録する。

## 3. Common properties

`#[text_style]` を持つ要素は次の共通propertyを公開する。

| Property | Meaning |
|---|---|
| `font_family` | family fallback list |
| `font_size` | logical point size |
| `font_weight` | numeric font weight |
| `font_style` | normal / italic / oblique |
| `font_stretch` | width variation |
| `character_spacing` | character advance adjustment |
| `foreground` | text brush |

`TextBlock` はこれらに加えて `text` と `text_alignment` を持つ。各control固有のpropertyは [`ui_spec.md`](ui_spec.md) を正本とする。

## 4. Inheritance and local values

- 各propertyは独立に継承される。子にlocal valueがあるpropertyだけが祖先値を上書きする。
- local valueがない場合、logical content relationを優先し、必要な場合にvisual parentを通じて最も近いtext-style ownerを探索する。
- `ContentControl`のcontentはcontrolからtext styleを継承できる。backend用の補助hostは公開上の継承境界を変更してはならない。
- 祖先にも値がないpropertyは、backend-independentなfallbackから解決される。ただしsystem font/foregroundを要求する値はbackend既定へ委ねる。

## 5. Clear, reset, and platform default

- propertyをclearするとlocal valueだけが削除され、継承値またはfallbackが再び有効になる。
- `theme!` が `PlatformDefault` に解決された場合、backendの対応propertyをclearし、ネイティブ既定値へ戻す。
- clearは他のtext propertyのlocal valueを変更しない。
- Theme変更やancestor変更によってeffective valueが変わった場合、表示・計測へ反映されなければならない。

## 6. Measurement and rendering

- 同じeffective text styleと制約に対し、measurementとrenderingは同じfont semanticsを使う。
- `font_size`、family、weight、style、stretch、character spacingの変更は、寸法が変わり得るため再計測を要求する。
- `foreground`だけの変更はpaint更新で足りる。
- secure text controlはOSのsecure-entry behaviorとsystem glyph cascadeを保持しなければならない。通常text renderer用のfont synthesisによってmask glyphを欠損させてはならない。
