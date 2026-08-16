# Context Menu and PopupSurface Architecture

規範仕様: [`../../specs/ui_spec.md`](../../specs/ui_spec.md)

## 1. Overview

ElwindUI のコンテキストメニューおよびポップアップ基盤は、以下の2つの柱で構成される。

1. **プラットフォーム中立な `ContextRequest` による入力と処理の分離**:
   物理的なマウスボタン（右クリック）や特定のキーバインド（Shift+F10, Menuキー, Ctrl+Click等）を Core の発火条件として直接持たず、Backend による入力変換を経て platform-neutral な `ContextRequest` として `ContextMenuService` に集約する。
2. **Window 境界を越える汎用 `PopupSurface` 基盤**:
   メイン Window の client 領域に制約される `InWindowOverlay` とは異なり、OS の独立サーフェス（AppKit: borderless `NSPanel`/`NSWindow`, WinUI 3: `Popup` / secondary window）を利用して Window 外や `NativeControl` の前面に UIElement ツリーを表示・対話可能にする。

---

## 2. Layering & Responsibility Separation

```text
[OS / Toolkit Input Layer]
(macOS: secondary click, Ctrl+Click, NSView menuForEvent)
(Windows: secondary click, ContextRequested, Shift+F10, Menu key, UIA)
(GTK4: toolkit-defined context action)
        │
        ▼ (Backend Input Translation)
[ContextRequest] (Core: platform-neutral UI semantics)
        │
        ▼
[ContextMenuService] (Core: target resolution & nearest ancestor lookup)
        │
   ┌────┴──────────────────────────┐
   ▼                               ▼
[Native Menu Presentation]    [PopupSurface (Custom Render)]
(NSMenu / MenuFlyout)         (Custom Menu Presenter / Custom Context Popup)
                                   │
                                   ▼
                              [PopupHost] (Backend capability trait)
```

### 責務境界

- **Backend (OS Input Translation)**:
  - 各 OS / ツールキットの固有入力を検知し、`ContextRequest` (`source: ContextRequestSource`, `position: Option<Point>`) を生成して Core に渡す。
  - OS 固有のキー組み合わせ（Windows: Shift+F10/Menuキー, macOS: Ctrl+Click）やマウスボタン割り当て（左右反転設定等）を Backend 内で解釈する。
- **Core (`ContextMenuService` & Target Resolution)**:
  - `ContextRequestSource::Pointer`: 指定された `position` に基づき `hit_test` を行いターゲット要素を決定。
  - `ContextRequestSource::Keyboard`: `FocusTracker.focused()` から現在フォーカスを持つ要素を決定（ポインタ位置は破棄）。
  - `ContextRequestSource::Accessibility` / `NativeControl`: 通知されたターゲット要素または owner identity を直接利用。
  - ターゲット要素から `visual_parent` チェーンをルート方向へ探索し、**最も近い祖先（nearest ancestor）** に設定された `context_menu` または `context_popup` を解決する。
- **Presentation**:
  - `ContextMenuPresentation::Native`: `Menu` / `MenuItem` のセマンティックモデルを Backend のネイティブメニュー（`NSMenu` / `MenuFlyout`）に渡して表示。
  - `ContextMenuPresentation::Custom`: 標準 `Menu` モデルを `ContextMenuPresenter` により通常 UIElement ツリーとして構築し、`PopupSurface` 上で表示。
  - `context_popup`: 任意の UIElement を生成する `PopupContentTemplate` をターゲットの有効な `EnvironmentContext` で評価し、`PopupSurface` 上で表示。

---

## 3. Context Request Data Model

```rust
pub enum ContextRequestSource {
    Pointer,
    Keyboard,
    Accessibility,
    Other,
}

pub struct ContextRequest {
    pub source: ContextRequestSource,
    pub position: Option<Point>,
}
```

### Target & Ancestor Lookup Flow

```text
ContextRequest arrives
   │
   ├─ Source == Pointer  ──> hit_test(root, position) ──> target UIElement
   ├─ Source == Keyboard ──> FocusTracker.focused()    ──> target UIElement
   └─ NativeControl / UIA ─────────────────────────────> target UIElement
   │
   ▼
Resolve nearest context owner:
   current = target
   loop {
       if current has context_menu or context_popup {
           return Some((current, definition));
       }
       current = current.visual_parent();
       if current is None { return None; }
   }
```

---

## 4. PopupSurface & Host Contract

### InWindowOverlay vs PopupSurface

| 項目 | InWindowOverlay | PopupSurface |
|---|---|---|
| 描画領域 | メイン Window の client bounds 内のみ | Window 境界を越えてモニター全域に表示可能 |
| Z-order | 同一 RenderTree 内の最前面 | owner Window より前面かつ NativeControl より前面 |
| NativeControl との関係 | NativeControl の下（OS View の背後）に隠れる場合あり | NativeControl の上（前面）に確実に描画 |
| OS プリミティブ | 同一 NSView / XAML Panel 内のレイヤー | 独立した platform window/panel/surface |
| タスクバー表示 | 表示されない | 表示されない（tool/popup window 扱い） |
| Global Topmost | なし | なし（owner Window と Z-order 連動） |

### PopupHost Capability Trait

### PopupHost Capability Trait

```rust
pub enum PopupFocusPolicy {
    None,
    Root,
    FirstFocusable,
}

pub enum PopupDismissPolicy {
    LightDismiss,
    Explicit,
}

pub struct PopupRequest {
    pub content: Rc<dyn UIElementExt>,
    pub position: Point,
    pub size: Size,
    pub focus_policy: PopupFocusPolicy,
    pub dismiss_policy: PopupDismissPolicy,
}

pub trait PopupHost {
    fn show_popup(&self, request: PopupRequest) -> Rc<dyn PopupSurfaceHandle>;
}

pub trait PopupSurfaceHandle {
    fn close(&self);
}
```

---

## 5. Placement & Coordinate Calculation

Placement 計算は Core 内の純粋関数（Pure logic）として実装され、Backend ごとに実装を複製しない。

### Pure Placement Logic

Core 座標系は **Top-Left (0, 0), +x: right, +y: down** に統一される。

```text
1. target anchor (Point or Rect in Core top-left screen coordinates)
2. desired popup content size (from measure pass)
3. monitor work area bounds (Rect in Core top-left, excluding dock/menubar/taskbar)
4. calculate ideal placement (Below / AutoFlip / Above / Right / Left)
5. test bounds against monitor work area:
   - if bottom overflows monitor work area -> flip upward (y = anchor.y - popup.height)
   - if right overflows monitor work area -> flip leftward (x = anchor.x - popup.width)
6. clamp within monitor work area bounds
```

### Coordinate Conversion Boundaries

- **Core Screen Coordinates**: プライマリモニター左上 (0, 0) を基準とした論理ピクセル座標（Top-Left, Y-down）。
- **AppKit Backend**: Core の Top-Left 座標系と AppKit の Bottom-Left (Y-up) 座標系との変換を Backend 境界で行う（`appkit_y = primary_screen_height - (core_y + height)`）。
- **WinUI 3 Backend**: Top-Left / Y-down セマンティクスをそのまま利用し、XamlRoot / Canvas 経由で配置。

---

## 6. Lifecycle & Environment Inheritance

### Build / Mount / Unmount Sequence

```text
Context Request
   │
   ▼
Capture effective EnvironmentContext from target element
   │
   ▼
Build Popup content using PopupContentTemplate(environment)
   │
   ▼
Create / Configure PopupSurface via PopupHost
   │
   ▼
Mount content tree into Popup's TreeHost (register RelayoutHost, FocusHost)
   │
   ▼
Show PopupSurface (apply calculated screen position)
   │
   ▼ [Interaction / Dismissal event: Outside Click, Escape, Selection, Window Move]
   │
Close PopupSurface
   │
   ▼
Unmount content tree (cancel subscriptions, clear Visual parent)
   │
   ▼
Restore focus to original target if necessary
```

---

## 7. Backend Implementation Strategies

### AppKit Backend
- **Native Context Menu**: `NSMenu` を `NSMenu::popUpContextMenu:withEvent:forView:` または `popUpMenuPositioningItem:atLocation:inView:` でポップアップ表示。
- **PopupSurface**: `NSPanel`（`NSWindowStyleMaskBorderless`, `NSFloatingWindowLevel`, `isFloatingPanel = true`, `hasShadow = true`）。
  - ContentView として `TreeHostView` を配置し、メイン Window と全く同じ layout / render / input / focus パイプラインを再利用。
  - `NSEvent::addLocalMonitorForEventsMatchingMask:` または `makeFirstResponder` を用いて Outside Click を検出し、自動 dismiss。

### WinUI 3 Backend
- **Native Context Menu**: `MenuFlyout` の `ShowAt(target_element, point)` を使用。
- **PopupSurface**: `Microsoft.UI.Xaml.Controls.Primitives.Popup` を使用して `TreeHostPanel` をホスト。
- **Coordinate Conversion**: Canvas ローカル座標を `TransformToVisual` により XamlRoot / Screen 座標へ変換。
- **Work Area**: `Microsoft.UI.Windowing.DisplayArea::GetFromPoint` を用いて実モニターの WorkArea を動的に取得。
- **Focus & Lifetime**: `PopupFocusPolicy::Root` にて開いた popup の root UIElement にフォーカスを設定。`TreeHostPanel` / `InnerPopupSurface` が `active_popup` として handle を保持し、新規 popup open 時に既存 popup を安全にクローズ。
- **Menu Realization Ownership**: `Menu` / `MenuItem` は論理セマンティックモデルであり、WinUI 3 ネイティブ XAML `MenuFlyoutItemBase` は同時に1つの親コレクション（MenuBarItem または MenuFlyout）にのみ所属する。
- Outside pointer press および `ProcessKeyboardAccelerators` (Escape) で dismiss。
- **GUI 実機検証**: Windows 実機環境での描画・マルチモニター・DPI・タッチ操作の検証は Issue [#157](https://github.com/puchinya/elwindui/issues/157) にて管理。

### GTK4 Backend
- GTK4 は現在 placeholder / stub 実装であるため、`PopupHost` および context menu の公開 API contract を整合させ、未実装であることを `docs/status/backend_status.md` および `control_status.md` に明記する。
