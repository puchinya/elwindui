# Icon Source Design

規範仕様: [`../../specs/ui_spec.md`](../../specs/ui_spec.md) §9 Menu, [`../../specs/graphics_spec.md`](../../specs/graphics_spec.md) §9 Icons

関連 Issue: #170 (`feat: add backend-neutral icons to Menu and Context Menu`)、実装: PR #171(review remediation delta 含む)、依存元: #152 / PR #154 / PR #156（Context Menu / PopupSurface 基盤）

## 1. Ownership

`IconSource` / `SystemIcon` は `elwindui_core::graphics`(`crates/elwindui-core/src/graphics/icon.rs`)が所有する generic value type であり、`MenuItem` 専用ではない。将来 Button / Toolbar 等が同じ semantic icon model を再利用できるよう、`ui::controls::menu_item` ではなく `graphics` モジュールに配置する。ただし、この Issue の scope は `MenuItem.icon` のみであり、他 control への icon property 追加は行わない。

```rust
#[derive(Debug, Clone)]
pub enum IconSource {
    System(SystemIcon),
    Image(ImageSource),
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemIcon {
    Add, Remove, Delete, Edit, Copy, Cut, Paste,
    Undo, Redo, Search, Settings, Refresh,
}
```

`IconSource::Image` は既存の `ImageSource`(`Raster(BitmapImage)` / `Vector(VectorImage)`)をラップするのみで、新しい bitmap/vector resource abstraction は導入しない。

## 2. `SystemIcon` common-set invariant

`SystemIcon` は backend-native な識別子（SF Symbol 名、WinUI `Symbol` 名、GTK icon 名等）を一切公開しない、意味ベースの `#[non_exhaustive]` enum である。初期 12 variant とそのマッピングは以下に固定する。

| `SystemIcon` | WinUI 3 `Symbol` | AppKit SF Symbol | GTK4 / freedesktop semantic name(reservation) |
|---|---|---|---|
| `Add` | `Symbol::Add` | `plus` | `list-add` |
| `Remove` | `Symbol::Remove` | `minus` | `list-remove` |
| `Delete` | `Symbol::Delete` | `trash` | `edit-delete` |
| `Edit` | `Symbol::Edit` | `pencil` | `document-edit` |
| `Copy` | `Symbol::Copy` | `doc.on.doc` | `edit-copy` |
| `Cut` | `Symbol::Cut` | `scissors` | `edit-cut` |
| `Paste` | `Symbol::Paste` | `doc.on.clipboard` | `edit-paste` |
| `Undo` | `Symbol::Undo` | `arrow.uturn.backward` | `edit-undo` |
| `Redo` | `Symbol::Redo` | `arrow.uturn.forward` | `edit-redo` |
| `Search` | `Symbol::Find` | `magnifyingglass` | `edit-find` |
| `Settings` | `Symbol::Setting` | `gearshape` | `preferences-system` |
| `Refresh` | `Symbol::Refresh` | `arrow.clockwise` | `view-refresh` |

このリストにない variant は、全対応 backend で同一の意味を持つことを design で確認するまで追加しない。GTK4 の named-icon 列は runtime 実装が無い現時点でも invariant を破らないための reservation であり、実装義務ではない。

## 3. Native vs Custom rendering

`ContextMenuPresentation::Native`(および `MenuBarItem.submenu`)と `ContextMenuPresentation::Custom` は同じ `MenuItem.icon` を読むが、変換経路が異なる。

```text
MenuItem.icon: Option<IconSource>
    │
    ├── Native / MenuBar submenu (backend InnerMenuItem)
    │       IconSource::System(icon)  -> backend native system icon
    │           AppKit:  SF Symbol name -> NSImage -> NSMenuItem.image
    │           WinUI3:  Symbol        -> SymbolIcon -> MenuFlyoutItem.Icon
    │       IconSource::Image(source) -> backend が既存の image/vector decode path を再利用し
    │                                    native 16x16 相当の icon へ変換
    │
    └── Custom (Core ContextMenuPresenter::build_menu_view)
            leading slot に既存の `elwindui::ui::Image` コントロールを配置
            IconSource::System(icon)  -> system_icon_vector(icon, color) -> Image.source
            IconSource::Image(source) -> Image.source にそのまま設定
```

Custom presentation は backend-neutral な `UIElement` ツリーのままであり、`NSImageView`/`SymbolIcon` 等の backend native 型を注入しない。Native presentation では ElwindUI の canonical fallback artwork ではなく OS/toolkit のシステムアイコンを優先する。

## 4. Core canonical vector fallback

`system_icon_vector(icon: SystemIcon, color: Color) -> ImageSource`(`elwindui-core` 内部専用、非公開)は Custom presentation 専用の visual fallback であり、`SystemIcon` の public semantic identity ではない。

- `VectorImageBuilder`(既存 API)で構築する。intrinsic size `16x16`、viewBox `(0,0,16,16)`、monochrome。
- enabled item は menu foreground 系、disabled item は disabled foreground 系の色を使う。
- 12 variant 分の geometry は static data として構築し、フレームごとに再構築しない。key space が固定(12 個)であるため、bounded な static cache を許容する。unbounded なグローバル icon cache は導入しない。
- 新しい SVG parser や外部 icon package 依存は追加しない。既存の Core vector primitives のみを使う。

## 5. User-defined icon conversion

`IconSource::Image(ImageSource)` は既存の raster/vector decode・rasterize pipeline をそのまま再利用する。`ImageSource::Raster`/`Vector` のいずれも、AppKit・WinUI 3 両 backend で完全にサポートする(PR #171 review remediation で完了)。

### AppKit

`render/image.rs` の decode helper（raster、`resolve_cgimage`）と `render/vector/raster.rs` の `rasterize_vector_image_to_cgimage`(vector)を再利用し、16×16pt の `NSImage` を構築する。新しいデコーダは追加しない。

### WinUI 3

`ImageData::Encoded` は既存の `InMemoryRandomAccessStream`/`DataWriter` バイトストリームパターン(`render/composition/cache.rs`の`ImageSurfaceCache::surface_for`と同じ手法)で直接 `BitmapImage.SetSource` に渡す fast path を使う(再エンコード無し)。

`ImageData::Rgba8` および Win2D `CanvasBitmap` を保持する `ImageData::Backend` は、既存の `render::win2d_bitmap`(`render/win2d.rs`、通常の render command path が既に使っている変換関数)をそのまま再利用する——menu icon 専用の別実装は作らない。`ImageData::Backend` が Win2D 以外のハンドルを保持している場合は conversion failure として icon を省略する(§6 の failure semantics通り、§B1 参照)。

`ImageSource::Vector` は、既存の `render::emit_vector_image`/`replay_win2d_primitives`(vector scene を Win2D primitive へ変換・再生する既存パイプライン、`draw_vector_image_surface` と共有)をそのまま再利用し、`render::rasterize_vector_image_to_canvas_bitmap` で 32×32 の透明な `CanvasRenderTarget` へラスタライズする。第二の VectorScene traversal は作らない。

いずれの経路で得た `CanvasBitmap`/`CanvasRenderTarget` も、`canvas_bitmap_to_xaml_image_source`(`CanvasBitmap.SaveAsync` で in-memory PNG stream にエンコードし、`XamlBitmapImage.SetSource` に渡す——同じ `InMemoryRandomAccessStream` ブリッジ規約)経由で XAML image source に変換し、`xaml_image_source_to_icon_element` で `ImageIcon`/`IconElement` にラップする。encoded fast path も含め、全経路がこの同じ終端ロジックを共有する。

**未検証の注記**: `CanvasRenderTarget` のコンストラクタと `CanvasBitmap.SaveAsync` の正確な windows_bindgen 生成名は、このセッションに Windows ビルド環境が無いため実機確認できていない。既存の命名規則(`CanvasBitmap::LoadAsyncFromStream`/`CanvasBitmap::CreateFromBytes`)から推測した最善の呼び出しをコード内コメントで明記しており、初回 Windows ビルド時に確認・必要なら修正すること(アーキテクチャ変更ではなく機械的なスペル修正)。

### `ImageData::Backend` の扱い(PR #171 review remediation の決定)

元の contract では `ImageSource` を user icon に再利用するとしていたが、`ImageData::Backend` は型消去された backend-native resource であり、本質的に portable ではないという曖昧さがあった。これを次のように解決する。

- WinUI 3: `ImageData::Backend` のハンドルが Win2D `CanvasBitmap` へ downcast できれば(`win2d_bitmap` の既存 `downcast_ref::<CanvasBitmap>()` 経路)、通常の render command path と同じくそのまま使う。
- downcast できない(他 backend 由来などの)ハンドルは conversion failure として扱い、icon を省略する(§6 failure semantics)。panic しない。新しい cross-backend reflection や public trait は追加しない。
- AppKit も同様に、`BackendImageHandle` が扱えない場合は同じ decode helper の既存の失敗経路(`None` 返却)で icon を省略する——AppKit 側はこの Issue で新たに handle した動作ではなく、既存の `resolve_cgimage`/decode helper が元々持つ振る舞い。

## 6. Failure semantics

アイコンは menu action の補助情報であり、失敗を `MenuItem` failure に昇格させない。

- ユーザー定義アイコンの decode/rasterize 失敗: icon を表示しない。`text`/`enabled`/`shortcut`/`on_select` は維持する。panic しない。`MenuItem` を削除しない。
- ネイティブ `SystemIcon` lookup 失敗(例: AppKit の SF Symbol が実行環境に存在しない): icon を省略し、`MenuItem` は正常に残る。
- ただし、既知の `SystemIcon` variant に対して backend 側の mapping 自体が存在しない状態(§2 の表を網羅していない)は、実行時 fallback で隠してはならず、compile-time/design defect として扱う——mapping completeness はテストで保証する(`crates/elwindui-backend-appkit`/`crates/elwindui-backend-winui3` の pure mapping テスト)。

## 7. Backend state ownership

各 backend の `InnerMenuItem` は `icon: Rc<RefCell<Option<IconSource>>>` として semantic 値を保持する(`InnerMenuItem` は `Clone` されるため、clone 間で最新状態を共有する必要がある)。`set_icon()` の順序は「semantic state 更新 → live native MenuItem へ反映 → native 変換失敗時も semantic state は保持」に固定する。icon の変換・設定処理はユーザーコールバックへ再入しない(icon `RefCell` を、再入し得る native 呼び出しをまたいで borrow したままにしない)。

### WinUI 3 の realization ownership separation(PR #156 由来、変更しない)

`InnerMenu::create_flyout()` は Context Menu 表示のたびに新しい `MenuFlyoutItem` を生成し、text/enabled/shortcut/Click を手動コピーする既存設計を維持する。したがって icon 対応は以下の **両方** に必要である。

1. live `InnerMenuItem.xaml`(MenuBar submenu 側)への直接反映。
2. `create_flyout()` が生成する新規 `MenuFlyoutItem` への同じ icon の適用(snapshot)。

同一の XAML `MenuFlyoutItem` インスタンスを MenuBar と Context Menu で共有する設計変更は行わない。open 済みの flyout への live icon mutation は保証しない(次回 open 時に最新値が使われる)。

### WinUI 3 バインディング

`crates/elwindui-backend-winui3/build.rs` の `windows_bindgen` `--filter` allowlist に `Symbol`/`SymbolIcon`/`ImageIcon`/`IconElement`/`CanvasRenderTarget`/`CanvasBitmapFileFormat` を追加済み(生成済みバインディングファイルは手編集していない)。

### Vector rasterization のコード共有

`render/vector.rs` の `draw_vector_image_surface`(既存の `CompositionDrawingSurface` fallback path)と menu icon 用の `rasterize_vector_image_to_canvas_bitmap` は、どちらも同じ `replay_win2d_primitives`(Win2D primitive stream の解釈器)を呼ぶ。Vector scene の traversal(`emit_vector_image`)も共有する。menu icon 専用の第二の traversal/interpreter は存在しない。

## 8. Non-goals(この Issue で行わないこと)

`MenuBarItem.icon`、checked/radio menu item、submenu 拡張、separator 再設計、Toolbar/Button icon、GTK4 Menu 実装、任意 `UIElement` を icon として使う API、icon の色/テーマ API、icon animation/badge、icon-only menu item、新しい SVG parser/image decoder、`PopupSurface`/`ContextRequest`/keyboard navigation/shortcut 構文の再設計は、いずれもこの Issue の scope 外とする。

## 9. `BitmapImage` rename(付随作業)

Custom presentation の icon slot は既存の `elwindui::ui::Image`(UIElement control、`crates/elwindui-core/src/ui/controls/image.rs`)を再利用する。この control 名 `Image` と、既存の raster resource 型 `elwindui_core::graphics::Image`(`crates/elwindui-core/src/graphics/image.rs`)が同名で紛らわしいため、後者を `BitmapImage` にリネームする。`ImageBrush`/`ImageData`/`ImageId`/`Brush::Image` 等、`Image` を含む他の型名・variant 名は変更しない。値意味論は [Graphics Specification §8](../../specs/graphics_spec.md#8-image) を正とする。
