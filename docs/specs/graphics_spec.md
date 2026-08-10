# ElwindUI Graphics Specification

本仕様書は、ElwindUI のグラフィックス描画サブシステムにおける公開値型（Value Types）、幾何データ構造、ブラシ・カラーモデル、および画像描画意味論を定義する規範仕様（Normative Specification）である。

---

## 1. Scope

本書は `elwindui::core::graphics` モジュールが公開するグラフィックス表現型（Color, Brush, Path, Image, VectorImage 等）の型定義および公開意味論を規定する。

描画コマンドの保持構造（`RenderTree`, `RenderGroup`, `RenderCommand`）の基本モデルは本書 §10 で規定する。各プラットフォームバックエンド（CoreGraphics / Win2D / Cairo）への具体的な描画命令再生（Replay）およびレイヤーキャッシュ戦略の詳細は [GUI Framework Design](../design/gui_framework_design.md) §5.7 を参照すること。

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

## 10. Retained render tree (`RenderTree`, `RenderGroup`, `RenderCommand`)

`RenderTree` は、UI 視覚ツリー（`UIElement` 階層）のレイアウト計算結果および描画コマンドを保持（Retained）し、ネイティブプラットフォームのレンダリング出力パス（CoreGraphics, Win2D, Cairo 等）へ引き渡す保持型グラフィックス描画データ構造である。

### 10.1 `RenderTree` and `RenderGroup` Architecture

- **`RenderTree`**:
  - 全体の保持型描画木のルートノード（`root: RenderGroup`）を保持する。
  - 要素 ID（`u64`）から木構造上のパス（`group_paths`）および対応する UI 要素（`visual_index: HashMap<u64, Weak<dyn UIElementExt>>`）への高速参照テーブルを管理する。
- **`RenderGroup`**:
  - 視覚ツリー上の 1 つの描画単位（Visual Node）。
  - **`id` (`u64`)**: 対応する UI 要素の一意識別子。
  - **`offset` (`Point`)**: 親 `RenderGroup` からのローカル相対位置オフセット。
  - **`size` (`Size`)**: 要素のローカル確定サイズ。
  - **`clip` (`Option<Rect>`)**: 要素に適用される局所クリッピング矩形。
  - **`commands` (`Vec<RenderCommand>`)**: 該当要素自身が出力する描画コマンド列。
  - **`children` (`Vec<RenderGroup>`)**: 子視覚要素の `RenderGroup` リスト。

### 10.2 Dirty Tracking & Generation Caching

- **`is_dirty` (`bool`)**:
  - 要素プロパティやサイズの変更に伴い、該当ノードが再描画・再作成を要することを示す一時フラグ（`mark_dirty`）。
- **`generation` (`u64`)**:
  - 描画コマンド列（`commands`）が更新・再記録されるたびにインクリメントされる世代番号。
  - バックエンドの描画キャッシュレイヤー（例: AppKit の `CALayer` キャッシュ構造）は、前回の描画再生（Replay）時の `generation` と比較することで、コマンドに変更がない場合にプラットフォームネイティブの描画リソース構築をスキップ・再利用する。

### 10.3 `RenderCommand` Primitive Set

`RenderGroup::commands` に保持されるバックエンド非依存の描画命令プリミティブ。

| Command Variant | Fields / Parameters | Description |
|---|---|---|
| `FillRect` | `rect`, `brush` | 矩形の塗りつぶし描画 |
| `StrokeRect` | `rect`, `brush`, `stroke` | 矩形の輪郭線描画 |
| `FillRoundedRect` | `rect`, `radii`, `brush` | 角丸矩形の塗りつぶし描画 |
| `StrokeRoundedRect` | `rect`, `radii`, `brush`, `stroke` | 角丸矩形の輪郭線描画 |
| `FillEllipse` | `rect`, `brush` | 楕円の塗りつぶし描画 |
| `StrokeEllipse` | `rect`, `brush`, `stroke` | 楕円の輪郭線描画 |
| `FillPath` | `path`, `brush`, `rule` | パス（ベクター図形）の塗りつぶし |
| `StrokePath` | `path`, `brush`, `stroke` | パス（ベクター図形）の輪郭線描画 |
| `DrawText` | `rect`, `text`, `style`, `alignment` | テキストグリフ列の指定領域内描画 |
| `DrawImage` | `rect`, `image`, `options` | ラスタ画像（`Image`）の描画 |
| `DrawVectorImage` | `rect`, `image`, `options` | ベクター画像（`VectorImage`）の描画 |
| `PushClip` / `PopClip` | `clip` (`Rect` / `RoundedRect` / `Path`) | クリッピング領域のスタック操作 |
| `NativeControl` | `bounds`, `owner_id` | ネイティブウィジェット（`NSView`, `FrameworkElement`）の埋め込み座標および所有要素 ID の定義 |

### 10.4 Reconcile & Resource Lifetime

- **ツリー同期（Reconcile）**:
  - UI 階層の変更やプロパティ更新後、`RenderTree::reconcile` により、最小の変更差分で `RenderGroup` の生成・削除・再記録・座標同期を行う。
- **リソースライフサイクル**:
  - `Visibility::Collapsed` や非選択タブなどの非アクティブノードでは、`UIElement` モデルは保持されたまま `RenderGroup` およびバックエンド描画リソースが解放され、アクティブ化時に最新のビューポートサイズで再作成される。

---

## 11. Related specifications

- [UI Specification](ui_spec.md) - 本グラフィックス型を利用するUI要素仕様
- [DSL Specification](dsl_spec.md) - DSLにおける属性指定ルール
