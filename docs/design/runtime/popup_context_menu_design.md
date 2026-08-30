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
  - 各 OS / ツールキットの固有入力を検知し、`ContextRequest` (`source`, `local_position`, `screen_anchor`) を生成して Core に渡す。
  - `local_position`: TreeHost-local 論理座標（`hit_test` によるターゲット要素解決用）。
  - `screen_anchor`: デスクトップ screen 論理座標アンカー（`PopupAnchor::Point` または `PopupAnchor::Rect`、ポップアップ配置計算用）。
  - OS 固有のキー組み合わせ（Windows: Shift+F10/Menuキー, macOS: Ctrl+Click）やマウスボタン割り当て（左右反転設定等）を Backend 内で解釈する。
- **Core (`ContextMenuService` & Target Resolution)**:
  - `ContextRequestSource::Pointer`: 指定された `local_position` に基づき `hit_test` を行いターゲット要素を決定し、`screen_anchor` を配置アンカーとして利用。
  - `ContextRequestSource::Keyboard`: `FocusTracker.focused()` から現在フォーカスを持つ要素を決定し、Backend が算出した `screen_anchor` を配置アンカーとして利用。
  - `ContextRequestSource::Accessibility` / `NativeControl`: 通知されたターゲット要素または owner identity を直接利用し、`screen_anchor` を配置アンカーとして利用。
  - ターゲット要素から `visual_parent` チェーンをルート方向へ探索し、**最も近い祖先（nearest ancestor）** に設定された `context_menu` または `context_popup` を解決する。
  - `screen_anchor` が与えられない場合、ローカル `arranged_offset` を screen 座標と誤認してフォールバックすることは行わず、安全に `None` を返す。
- **Presentation**:
  - `ContextMenuPresentation::Native`: `Menu` / `MenuItem` のセマンティックモデルを Backend のネイティブメニュー（`NSMenu` / `MenuFlyout`）に渡して表示。
  - `ContextMenuPresentation::Custom`: 標準 `Menu` モデルを `ContextMenuPresenter` により通常 UIElement ツリーとして構築し、`PopupSurface` 上で表示。
  - `context_popup`: 任意の UIElement を生成する `ViewFactory`（汎用 deferred View factory 型、`docs/design/runtime/view_factory_design.md`）を、ターゲットの有効な `EnvironmentContext` から `derive()` した popup 専用コンテキストで評価し、`PopupSurface` 上で表示。
  - `MenuItem.icon`(`IconSource`、規範仕様: [UI Specification §9 Menu](../../specs/ui_spec.md)、値型設計: [Icon Source Design](icon_source_design.md)): 同じ `MenuItem.icon` が presentation mode に関わらず表示される。
    - `Native`: `MenuItem.icon` を backend のネイティブ icon slot（`NSMenuItem.image` / `MenuFlyoutItem.Icon`）に渡す。`SystemIcon` は OS/toolkit のネイティブシステムアイコンとして表示する。
    - `Custom`: `ContextMenuPresenter` が構築する行に既存の `elwindui::ui::Image` コントロールを leading icon slot として配置する。`IconSource::Image` はそのまま `Image.source` に設定し、`IconSource::System` は Core 内部の canonical monochrome vector fallback（`system_icon_vector`）を経由して `Image.source` に設定する——backend native icon をこの UIElement ツリーへ注入しない。

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
    pub local_position: Option<Point>,
    pub screen_anchor: Option<PopupAnchor>,
}
```

### Target & Ancestor Lookup Flow

```text
ContextRequest arrives
   │
   ├─ Source == Pointer  ──> hit_test(root, local_position) ──> target UIElement (anchor: screen_anchor)
   ├─ Source == Keyboard ──> FocusTracker.focused()         ──> target UIElement (anchor: screen_anchor)
   └─ NativeControl / UIA ──────────────────────────────────> target UIElement
   │
   ▼
Resolve nearest context owner:
       if current has context_menu or context_popup {
           return Some((current, definition));
       }
       current = current.visual_parent();
       if current is None { return None; }
   }
```

---

## 4. PopupSurface & Host Contract

### Coordinate Layers

| レイヤー | 原点 / 単位 | 用途 |
|---|---|---|
| **TreeHost-local Logical DIP** | View / Canvas 左上 (0,0), Y-down | 入力ルーティング、ヒットテスト、レイアウト |
| **Core Screen Logical DIP** | デスクトップ仮想スクリーン空間（Y-down、マルチモニター配置により負の X/Y 座標を取り得る） | ポップアップ配置計算 (`calculate_popup_placement`)、アンカー座標 |
| **OS Physical Pixels** | ディスプレイ物理座標系 | Windows `ContentCoordinateConverter`, `AppWindow.Position`, `DisplayArea.OuterBounds`/`WorkArea` 等のネイティブ境界 |
| **WinUI XAML Local DIP** | XamlRoot / Window Client 左上 (0,0), Y-down | `Popup.HorizontalOffset` / `VerticalOffset` の設定 |
| **AppKit Native Screen** | Primary Screen 左下 (0,0), Y-up | `NSWindow.setFrame`, `NSScreen.frame` のネイティブ境界 |

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
    /// `None` if the backend could not show the popup (e.g. WinUI3 coordinate conversion or
    /// `Popup` construction failure). A backend must never return a handle wrapping a nonexistent
    /// native surface — the caller (`ContextMenuService::open_custom_popup`/`open_custom_menu`)
    /// unmounts `request.content` itself when this is `None`.
    fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>>;
}

pub trait PopupSurfaceHandle {
    fn close(&self);
}
```

`PopupHost::show_popup` became fallible in Issue #161's second review pass (previously `-> Rc<dyn PopupSurfaceHandle>`, unconditionally). Before that, WinUI3's `InnerPopupSurface::show` could already fail, but `WinUI3PopupHost::show_popup` papered over it with a handle wrapping `surface: Option<Rc<InnerPopupSurface>>` — Core saw a successfully-opened popup even when no native surface existed. `WinUI3PopupHandle` no longer has that `Option` layer: it always wraps a live `Rc<InnerPopupSurface>`, and `WinUI3PopupHost::show_popup` returns `None` directly on failure.

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

`context_popup` の内容は、ターゲット要素の mount 時点ではなく、**表示要求（open）が発生した時点**で構築される。`ViewFactory::build`（`docs/design/runtime/view_factory_design.md`）は owner を `Weak` としてのみ捕捉し、owner が既に解放されていれば（factory クロージャを一切呼ばずに）`None` を返す — その場合 popup は表示されない（`ContextMenuService::open_custom_popup` も `Option<Rc<dyn PopupSurfaceHandle>>` を返す）。

`ContextMenuService::open_custom_popup` は、owner の有効な `EnvironmentContext` から `derive()` した popup 専用の `EnvironmentContext` を作る。この派生コンテキストに `PopupDismissAction`（`crate::ui::popup::PopupDismissActionKey`）を設定してから `ViewFactory::build` に渡す。owner 自身の Environment は変更しない。`PopupDismissActionKey` は `open_custom_popup` のみが設定するフレームワーク管理値であり、DSL 側（`EnvironmentScope`/`#[elwindui::theme]`）からは書き込めない——`#[environment(popup_dismiss)]` による読み取りのみが可能（`component_frontend::lookup_environment_key` は読み取り専用の resolver、`lookup_writable_environment_key` は `popup_dismiss` を含まない、`docs/specs/dsl_spec.md` §4/§13 参照）。ただの「上書き可能な通常の Environment 値」ではない点に注意。

`PopupDismissAction` は内部で `PopupDismissState`（`Building` → `Open(Weak<PopupSurfaceHandle>)` / `Dismissed`、`open_custom_popup` に private）という状態を持つ。これは build/mount 中（ネイティブサーフェスがまだ存在しない段階、将来 #162 の生成 Component root の `on_mount` を含む）に dismiss が呼ばれるケースを正しく扱うために必要——単純な weak-handle-slot だけでは、`show_popup` が返る前に dismiss された場合、そのリクエストが握りつぶされてしまう。

`Building` から `Open` への遷移は `host.show_popup` が返った**後**、一度の可変借用の下でアトミックに行う——`show_popup` 自体の中(バックエンドのネイティブ「表示」呼び出しが同期的に再入するケース)で dismiss が呼ばれた場合、状態は `show_popup` が返る前に `Building` → `Dismissed` へ遷移している。この遷移後の状態を確認せずに無条件で `Open` を代入すると、`Dismissed` が `Open` で上書きされ、dismiss 要求が失われる。正しい遷移は: 直前の状態が `Building` なら `Open(Weak(handle))` へ、`Dismissed` なら `Dismissed` のまま据え置いた上でこの `handle` 自体を即座に `close()` して `None` を返す（`Open` は決して経由しない）。

### Build / Mount / Unmount Sequence

```text
Context Request
   │
   ▼
Resolve owner (nearest-ancestor context_popup/context_menu target)
   │
   ▼
Capture effective EnvironmentContext from owner, then derive() a popup-scoped EnvironmentContext
   │
   ▼
Install PopupDismissAction into the popup-scoped EnvironmentContext (PopupDismissState::Building)
   │
   ▼
ViewFactory::build(ViewBuildContext { owner: Weak<owner>, environment: popup-scoped })
   │  -> None if owner already dropped (enforced by ViewFactory::build itself, factory never runs):
   │     abort, nothing shown, nothing to unmount
   ▼
[content built; PopupDismissAction may already have been called during this step]
   │
   ├─ dismissed during build (PopupDismissState::Dismissed) ──> unmount_subtree(content) ──> abort,
   │                                                              return None, popup never shown
   ▼ (still Building)
Measure content, compute placement
   │
   ▼
PopupHost::show_popup(request) -> Option<Rc<dyn PopupSurfaceHandle>>
   │  (dismiss() may fire from *inside* this call — a backend's native "show" can reenter
   │   synchronously; state moves Building -> Dismissed here if so, still before any handle exists)
   │
   ├─ None (backend show failed, e.g. WinUI3 coordinate conversion) ──> PopupDismissState::Dismissed
   │                                                                     ──> unmount_subtree(content)
   │                                                                     ──> return None
   ▼ Some(handle)
Atomic check-and-transition on PopupDismissState (single mutable borrow):
   ├─ was Building  ──> PopupDismissState::Open(Weak::downgrade(handle)) ──> return Some(handle)
   │                     (dismiss() past this point upgrades and calls handle.close())
   └─ was Dismissed ──> stays Dismissed (never Open) ──> handle.close() ──> return None
      (dismiss() fired during show_popup itself, above; the handle it raced against is still
      closed here rather than left open or silently dropped)
   │
   ▼ Dismissal
   │
   ├─ ElwindUI-controlled close (PopupDismissAction, item selection, popup replacement, explicit
   │  PopupSurfaceHandle::close(), Drop, AppKit light-dismiss [own NSEvent monitors, ElwindUI-driven])
   │      │
   │      ▼
   │  internal close guard (idempotent, exactly-once — `begin_close`/`is_open`)
   │      │
   │      ▼
   │  unmount_subtree(content root) — child-first: on_unmount hooks, subscription cancellation
   │      │
   │      ▼
   │  native close/visibility detach (AppKit: removeChildWindow/orderOut; WinUI3: SetIsOpen(false))
   │      │
   │      ▼
   │  ElwindUI host clear/detach (TreeHost::clear_tree() — native resource release only)
   │
   └─ backend-native post-dismiss notification (WinUI3 Popup.Closed only — toolkit already changed
      visibility/IsOpen before ElwindUI is ever notified; see §7's WinUI3 subsection)
          │
          ▼
      internal close guard (same exactly-once guard as the ElwindUI-controlled path)
          │
          ▼
      unmount_subtree(content root) — child-first: on_unmount hooks, subscription cancellation
          │
          ▼
      ElwindUI host clear/detach (TreeHost::clear_tree()) — no further native visibility call here;
          the toolkit already performed its own close
   │
   ▼ (either branch)
Backend host releases its own strong reference to the content root (`InnerPopupSurface::content:
   RefCell<Option<Rc<..>>>`, taken during the close guard above) — a closed PopupSurfaceHandle must
   not keep the already-unmounted content subtree alive just because the handle itself is still
   reachable (e.g. via a host's active_popup field until replaced/dropped)
   │
   ▼
Restore focus to original target if necessary
```

**Portable invariant, precisely**: `unmount_subtree` runs exactly once, before the ElwindUI popup
host detaches/clears the content tree and before the surface releases its own content ownership, on
*every* dismissal path on *every* backend. The *stronger* claim — unmount before any native
visibility/detach operation — holds for every ElwindUI-controlled close, but not for a backend-native
post-dismiss notification whose toolkit only reports the dismissal *after* changing visibility itself
(currently just WinUI3's `Popup.Closed`, see §7). `on_unmount` must never be documented or assumed to
require the native popup to still be visible/open while it runs.

**Declarative `context_popup: view! { .. }` DSL** (Issue [#162](https://github.com/puchinya/elwindui/issues/162),
split from [#161](https://github.com/puchinya/elwindui/issues/161), which owns the `ViewFactory`
runtime/backend foundation this design revision describes) is implemented. A `context_popup`
attribute may now be assigned a bare `view! { .. }` block — the same grammar/AST/codegen pipeline a
normal Component body uses — instead of an ordinary `ViewFactory`-typed expression. It desugars
entirely at macro-expansion time, so no new *runtime* binding system is introduced — but the desugar
itself runs in the middle of the pipeline, not before it: `validate::validate` runs first, against the
*original*, unlowered `ViewExpr::DeferredView` node (so it can validate bare names against the
enclosing lexical Component's own scope, `check_deferred_view_assignment`), and only *after* that does
`lower_deferred_views_in_module` extract it into the hidden Component/View pair described below,
before `codegen::build_symbol_table`/code emission ever run — see step 1's own ordering note.

1. **Lowering** (`elwindui-codegen::lower_deferred_views_in_module`, run once per enclosing module
   after `validate::validate` and before `codegen::build_symbol_table`): every `view! { .. }` block
   found in `context_popup` position is extracted into its own hidden, framework-synthesized
   `ComponentDef`/`ViewDef` pair — `ContentControl`-based, named
   `__ElwinduiViewFactoryInstanceFor<Owner>_<ordinal>` — carrying exactly one synthetic field,
   `#[param] __view_owner: Weak<Owner>` (`ViewDef::implicit_owner = Some(ImplicitOwnerDef {
   field_name: "__view_owner", readable_fields, writable_fields, reactive_fields, bindable_fields
   })` — PR #165 final/post-final rereview remediation, A2/A8/A9: an explicit schema, computed once
   from `Owner`'s own effective fields, not a bare owner-field-name string — see
   `docs/design/tools/codegen_design.md` §3.35 for the exact derivation rule). The original
   `context_popup` attribute value is replaced with a `ViewExpr::DeferredView` marker referencing
   the hidden component by name.
2. **Weak-owner codegen** (reusing, not duplicating, the template parent's declared-alias
   weak-owner mechanism — see `docs/design/runtime/view_factory_design.md` §3's "why `Weak`, never
   `Rc`" and `is_template_or_deferred_scope`): the hidden component's generated code treats
   `__view_owner` exactly like a template parent for bindable-owner and Environment-propagation
   purposes (`docs/agents/class-model.md`'s "no second binding system" principle — see also
   `synthesize_external_base_fields`'s explicit `implicit_owner.is_some()` exemption, which stops
   bare names resolved against the *enclosing* Component's own scope from being mistaken for
   unresolved external-builtin-base fields needing synthesis).
3. **Factory emission**: the `context_popup` site emits
   `ViewFactory::new(move |ctx| { .. })` (`docs/design/runtime/view_factory_design.md` §2 — the
   same `ViewFactory` every other deferred-view-typed field already uses). The closure recovers a
   weak reference to the *lexical owner* (`DeferredViewExpr::lexical_owner` — always the original
   source Component, at any nesting depth, PR #165 review remediation A3) one of two ways, chosen at
   the point this factory expression is emitted, not by the hidden component's own shape:
   - **Top-level** (`ctx.implicit_owner.is_none()` — this factory is being emitted inside the true
     lexical owner's own generated code): `self.__self_weak.borrow().upgrade().and_then(|rc|
     rc.downcast::<Self>().ok()).map(|rc| Rc::downgrade(&rc))` (the same idiom `__build_view`'s own
     `__most_derived` local already uses — *not* `Rc::downgrade(self)`, which assumes an ownership
     shape not every generated component has).
   - **Nested** (`ctx.implicit_owner.is_some()` — a `context_popup: view! { .. }` written inside
     *another* `context_popup: view! { .. }`'s own body, so this factory expression is emitted while
     generating the *outer* hidden Component's own code): `self.__view_owner.clone()` directly — the
     outer hidden Component's own `__view_owner` field already holds exactly the right
     `Weak<lexical_owner>` value (by the same A3 guarantee that keeps every nesting level's
     `lexical_owner` equal to the same original source Component), so it is reused rather than
     re-derived. The `__self_weak`-downcast approach above cannot work here: `self` at this emission
     point genuinely *is* an instance of the outer hidden Component's own type, not the lexical
     owner's, so `downcast::<lexical_owner>()` on it can never succeed.

   Either way, the recovered weak reference returns `None` immediately if that owner has already
   been dropped, and otherwise constructs a **fresh hidden-component instance on every popup open**
   (`__ElwinduiViewFactoryInstanceFor<Owner>_<ordinal>::__new_unmounted(owner_weak)`, then
   `mount(ctx.environment)`) — so bare names inside the `view! { .. }` block resolve directly against
   the enclosing Component's own schema-listed fields/state/params/computed/environment values and
   `#[bindable]`-owner-qualified paths (`ImplicitOwnerDef`'s `readable_fields`/`bindable_fields` —
   *not* arbitrary methods, and *not* every name in scope: a bare name outside that schema is left as
   ordinary Rust, and an explicit `self` inside the block still means the hidden Component itself,
   never the enclosing Component), read at the moment the popup is actually opened (not at the
   enclosing Component's own mount time). A writable schema field (`writable_fields`, `Prop`/`State`
   only) assigned to inside the block routes through the enclosing Component's own generated setter.
   Any reactive binding inside the block — including a direct bare schema field or a `#[bindable]`
   owner's own qualified path — live-updates for as long as the popup instance stays mounted, exactly
   as an ordinary nested `view!` region would (PR #165 post-final rereview remediation, A9: this
   applies uniformly to a direct bare reference and a `vm.field`-qualified one alike, each backed by
   its own real `ObservableExt` subscription — `codegen::implicit_bind_owners`).

Environment propagates from the hidden component into *its own* nested children the same way
`ControlTemplate`'s replaced body already does (`node.environment_scope`,
`is_template_or_deferred_scope` triggered by `implicit_owner.is_some()`) — without this, nested
Components inside a declarative popup would silently fall back to `application_environment()`
instead of inheriting the popup's actual mount-time Environment (including
`#[environment(popup_dismiss)]`), a correctness gap this design's own implementation found and fixed
in the same generalized mechanism rather than a `context_popup`-specific special case.

Today, popup content may still be authored via the low-level `ViewFactory::new(|ctx| ...)` API
directly when full manual control is wanted — see `docs/design/runtime/view_factory_design.md` §4
for exactly what that low-level API does and does not guarantee. Both forms share the same
`ViewFactory` runtime primitive, but they are not equivalent: the low-level API is just a raw
closure the author fills in by hand, with no help resolving or capturing an outer owner correctly
(it is entirely possible to hand-write one that captures a stale value or an accidental strong
`Rc`). The declarative `context_popup: view! { .. }` sugar additionally provides, at compile time,
lexical-owner resolution against the enclosing Component's own schema, disciplined weak-owner
capture (never an accidental strong reference), and the same binding/dependency-tracking codegen
(including live updates and subscription cleanup) an ordinary `view!` body gets — guarantees the raw
closure API cannot enforce on its own.

**Owner-`Window`-close interaction**: closing the owner `Window` while one of its declarative (or
manually-authored `ViewFactory`) popups is still open must close that popup — and run its
`unmount_subtree` teardown — *before* the Window's own content unmounts, not leave it to be
orphaned by the native surface disappearing out from under it. `Window::unmount_override`
(`docs/design/runtime/component_lifecycle_design.md`'s "Window mount_override/unmount_override
hooks") calls the backend's own `close_active_popup` (`TreeHostView`/`TreeHostPanel`, both a thin
`take()`-then-`close()` on the same `active_popup` slot §6's Build/Mount/Unmount Sequence already
tracks) at exactly this point, ahead of the owner's own content teardown — the same portable
invariant this section already establishes for popup dismissal in isolation now also holds across an
owning-Window close.

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
- **Coordinate Conversion**:
  - `canvas_to_screen_point`: Canvas ローカル DIP -> `TransformToVisual(XamlRoot.Content)` による Window Client Local DIP -> `ContentCoordinateConverter::ConvertLocalToScreen` (または `+ (AppWindow.Position / scale)` fallback) による Desktop Screen Logical DIP。
  - `screen_logical_to_xaml_local`: Desktop Screen Logical DIP -> Screen Physical Px -> `ContentCoordinateConverter::ConvertScreenToLocal` (または `- (AppWindow.Position / scale)` fallback) による XamlRoot / Window Client Local DIP -> `Popup.SetHorizontalOffset` / `SetVerticalOffset`。
  - 変換失敗時に screen 座標を XAML ローカルオフセットとして誤認・再利用することは禁止し、安全にポップアップ表示を中断（`Option::None`）する。
- **Work Area**: `DisplayArea::GetFromPoint` -> `DisplayArea::GetFromWindowId` -> 明示的 screen 変換済み XamlRoot bounds の優先順で取得し、`display_area_to_core_work_area` (`outer_x + work_x`, `outer_y + work_y`) により必ずグローバル Screen Logical Rect (`Option<Rect>`) として返す。未変換のローカル Rect をスクリーン Rect として偽装返却することは禁止。
- **Focus & Lifetime**: `PopupFocusPolicy::Root` にて開いた popup の root UIElement にフォーカスを設定。`TreeHostPanel` / `InnerPopupSurface` が `active_popup` として handle を保持し、新規 popup open 時に既存 popup を安全にクローズ。
- **Menu Realization Ownership**: `Menu` / `MenuItem` は論理セマンティックモデルであり、Context Menu 表示時は `InnerMenu::create_flyout` により専用の `MenuFlyoutItem` インスタンスを生成することで `MenuBarItem` とのネイティブインスタンス競合を回避。
- Outside pointer press および `ProcessKeyboardAccelerators` (Escape) で dismiss。
- **Native light-dismiss の close 経路（Issue #161 レビューで確定した例外）**: `Popup.IsLightDismissEnabled = true` の場合、outside pointer press や Escape での dismiss は WinUI 自身が検出して自動的に `Popup.IsOpen` を `false` にする——ElwindUI はこの遷移を制御も事前通知も受けない。`Popup.Closed` イベントは `IsOpen` が `false` になった**後**にのみ発火する（[`Popup`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.primitives.popup)・[`Popup.Closed`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.primitives.popup.closed) 参照）。したがって `InnerPopupSurface` は native-originated close 用に専用の内部ハンドラ（`on_native_closed`、`crates/elwindui-backend-winui3/src/inner/popup.rs`）を持つ: `Popup.Closed` はこのハンドラへルーティングされ、`unmount_subtree` と `TreeHostPanel::clear_tree()` のみを実行し、`SetIsOpen(false)` は呼ばない（WinUI が既に行っているため）。ElwindUI 主導の close（`PopupDismissAction` 等）が使う `close()` はこれとは別に、`unmount_subtree` を native visibility 変更（`SetIsOpen(false)`）より前に実行する、より強い順序を維持する。両者は同一の exactly-once guard（`begin_close`）を共有し、どちらの経路からも teardown は1回だけ実行される。**`Popup.Closed` を「close 前」に発火するイベントとして扱ってはならない** — 常に事後通知である。
- **GUI 実機検証**: Windows 実機環境での描画・マルチモニター・DPI・タッチ操作、および上記 native light-dismiss の順序保証の検証は Issue [#157](https://github.com/puchinya/elwindui/issues/157) にて管理。

### GTK4 Backend
- GTK4 は現在 placeholder / stub 実装であるため、`PopupHost` および context menu の公開 API contract を整合させ、未実装であることを `docs/status/backend_status.md` および `control_status.md` に明記する。
