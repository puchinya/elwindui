# ElwindUI Graphics Specification

本仕様書は、ElwindUI のグラフィックス描画サブシステムにおける公開値型（Value Types）、幾何データ構造、ブラシ・カラーモデル、および画像描画意味論を定義する規範仕様（Normative Specification）である。

---

## 1. Scope

本書は `elwindui::core::graphics` モジュールが公開するグラフィックス表現型（Color, Brush, Path, Image, VectorImage 等）の型定義および公開意味論を規定する。

描画コマンドの保持構造（RenderTree, RenderGroup）、フィンガープリント、各バックエンド（CoreGraphics / Win2D / Cairo）への再生・再描画キャッシュ戦略は実装設計であり、[GUI Framework Design](../design/gui_framework_design.md) §5.7 の対象である。

---

## 2. Public API and Canonical Paths

グラフィックス型は `elwindui::core::graphics` から提供される。主な公開型は以下の通りである。

```rust
use elwindui::core::graphics::{
    Color, Brush, LinearGradientBrush, RadialGradientBrush, ImageBrush,
    StrokeStyle, Path, PathBuilder, Image, ImageData, VectorImage, ...
};
```

---

## 3. Coordinate and unit conventions

- **座標系**: 左上を原点 `(0.0, 0.0)` とし、右方向を正の X 軸、下方向を正の Y 軸とする。
- **単位**: 原則論理ピクセル（Device-Independent Pixels / DIPs）単位とする。
- **角度**: 円弧描画や回転等の角度指定は原則ラジアン（radians）または度（degrees）明記に従う。

---

## 4. Color

### `elwindui::core::graphics::Color`

RGBA 各要素を 8bit 整数 (`u8`) または 32bit 浮動小数点数 (`f32`) で保持する色表現。

#### Representation

```rust
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}
```

#### Parsing

16進数カラーコード文字列（例: `"#FF0000"`, `"#8000FF00"`）および色名からのパース（`Color::parse` / `FromStr`）をサポートする。

---

## 5. Brush

描画領域の塗りつぶし表現の抽象化。単色、グラデーション、パターン画像塗りをサポートする。

### 5.1 Solid Color Brush

単色塗り表現。`Color` 値から暗黙変換または直接指定される。

### 5.2 Linear Gradient Brush (`LinearGradientBrush`)

始点と終点のベクトルに沿って色が連続的に変化する線形グラデーション。

#### Properties

| Name | Type | Description |
|---|---|---|
| `start_point` | `Point` | グラデーション始点 |
| `end_point` | `Point` | グラデーション終点 |
| `stops` | `Vec<GradientStop>` | 色と位置（0.0 ..= 1.0）のリスト |
| `mapping_mode` | `BrushMappingMode` | 座標系（`Absolute`, `RelativeToBoundingBox`） |
| `spread_method` | `GradientSpreadMethod` | 領域外の拡張方式（`Pad`, `Reflect`, `Repeat`） |

### 5.3 Radial Gradient Brush (`RadialGradientBrush`)

中心点から外周方向へ放射状に色が変化する円形グラデーション。

#### Properties

| Name | Type | Description |
|---|---|---|
| `center` | `Point` | 放射の中心点 |
| `gradient_origin` | `Point` | グラデーションの起点 |
| `radius_x` | `f32` | X軸方向の半径 |
| `radius_y` | `f32` | Y軸方向の半径 |
| `stops` | `Vec<GradientStop>` | 色と位置のリスト |
| `spread_method` | `GradientSpreadMethod` | 領域外の拡張方式 |

### 5.4 Image Brush (`ImageBrush`)

画像パターンを用いて塗りつぶすブラシ。

#### Properties

| Name | Type | Description |
|---|---|---|
| `image` | `Image` | パターン画像 |
| `stretch` | `Stretch` | フィット伸長方式（`None`, `Fill`, `Uniform`, `UniformToFill`） |
| `tile_mode` | `TileMode` | タイリング方式（`None`, `FlipX`, `FlipY`, `FlipXY`, `Tile`） |
| `alignment_x` | `AlignmentX` | 水平揃え（`Left`, `Center`, `Right`） |
| `alignment_y` | `AlignmentY` | 垂直揃え（`Top`, `Center`, `Bottom`） |

---

## 6. Stroke

ストローク（輪郭線）のスタイル表現。

### `elwindui::core::graphics::StrokeStyle`

| Property | Type | Description |
|---|---|---|
| `width` | `f32` | 線幅 |
| `line_cap` | `LineCap` | 線の端点形状（`Flat`, `Square`, `Round`） |
| `line_join` | `LineJoin` | 線の結合形状（`Miter`, `Bevel`, `Round`） |
| `miter_limit` | `f32` | マイター結合の限界値 |
| `dash_array` | `Vec<f32>` | 破線のパターン配列 |
| `dash_offset` | `f32` | 破線パターンの開始オフセット |

---

## 7. Geometry

幾何パス表現とブーリアン演算の抽象化。

### 7.1 Path & PathBuilder

2次元ベクターパス構造。`PathBuilder` により命令的に構築される。

#### Path Commands

- `move_to(point)`: 開始点の移動
- `line_to(point)`: 直線の追加
- `quad_to(control, end)`: 2次ベジェ曲線の追加
- `cubic_to(control1, control2, end)`: 3次ベジェ曲線の追加
- `arc_to(segment)`: 円弧の追加
- `close()`: パスを閉じる

### 7.2 FillRule

パスの内部判定ルール。

- `EvenOdd`: 奇偶ルール
- `NonZero`: 非ゼロ巻き数ルール

### 7.3 Geometry Combination (`GeometryCombineMode`)

複数のパスに対する幾何学ブーリアン演算。

- `Union`: 和集合
- `Intersect`: 積集合
- `Exclude`: 差集合（A - B）
- `Xor`: 排他的論理和

---

## 8. Image

ラスタ画像データのメモリ表現とフォーマット。

### `elwindui::core::graphics::Image` & `ImageData`

| Property | Type | Description |
|---|---|---|
| `width` | `u32` | ピクセル幅 |
| `height` | `u32` | ピクセル高さ |
| `format` | `ImageFormat` | ピクセルフォーマット（`Rgba8`, `Bgra8` 等） |
| `alpha_mode` | `AlphaMode` | アルファ乗算モード（`Straight`, `Premultiplied`） |

#### ImageFit

描画領域に対する整列・拡大縮小方式（`None`, `Fill`, `Contain`, `Cover`）。

---

## 9. Vector graphics

SVG等のベクター文書の保持・レンダリング構造。

### `elwindui::core::graphics::VectorImage`

解像度非依存のベクターシーングラフを保持するオブジェクト。

#### Concepts
- **`VectorNode`**: パスノード、グループノード、テキストノードなどの階層要素
- **`VectorPaint`**: ベクター用の塗り・ストローク定義
- **`VectorFilter`**: ドロップシドウ、ガウシアンブラー等のベクターフィルター効果
- **`PreserveAspectRatio`**: アスペクト比維持の揃えとスライスルール

---

## 10. Related specifications

- [UI Specification](ui_spec.md) - 本グラフィックス型を利用するUI要素仕様
- [DSL Specification](dsl_spec.md) - DSLにおける属性指定ルール
