# Native peer design (Issue #313)

Related specifications: [`../../specs/ui_spec.md`](../../specs/ui_spec.md),
[`../../specs/dsl_spec.md`](../../specs/dsl_spec.md),
[`../../specs/macro_class_spec.md`](../../specs/macro_class_spec.md).
Related designs: [`native_control_design.md`](native_control_design.md),
[`ui_tree_design.md`](ui_tree_design.md),
[`component_lifecycle_design.md`](component_lifecycle_design.md) §4g/§4i/§4j,
[`rendering_design.md`](rendering_design.md).
Tracking: [#313](https://github.com/puchinya/elwindui/issues/313). Status: **approved target architecture,
migration in progress**. Current-state progress is in `docs/status/`, not here.

## 1. Problem and target

Before #313, each native builtin exists twice. Core declares an interface with
`#[component(trait_only)] trait` (or `#[class(trait_only)]`), and WinUI3 and AppKit each declare a concrete
`struct_only` implementor. `Button` and `Window` also add `abi_props` so the backend constructor ABI can find
Core's property schema. The facade re-exports backend concrete types and their constructor ABIs. This
affects 19 builtins: `NativeControl`, `Button`, `Window`, `CheckBox`, `RadioButton`, `Slider`,
`ToggleSwitch`, `TextBox`, `TextArea`, `PasswordBox`, `Dropdown`, `DropdownItem`, `Menu`, `MenuBar`,
`MenuItem`, `MenuBarItem`, `ScrollView`, `TabView` and `TabViewItem`.

Target: **Core declares each native builtin once, as an ordinary concrete Component.** A backend implements
only a *peer*: a native object that follows a Core-defined Rust trait. Backend peers are created through a
factory that the backend registers. The `trait_only`, `struct_only` and `abi_props` modes are then removed.

Invariants kept from the existing design:

- Core has no dependency on a backend crate and no `#[cfg(target_os = ..)]` control implementation.
- The `Rc` Visual tree, weak parent links and `Window`-is-not-a-`UIElement` stay as they are.
- `try_as_native_control() -> Option<&dyn Any>` and `record_group_commands<H>` /
  `RenderTree::{new, reconcile}<H>` in `ui/engine.rs` stay unchanged. These are the real implementation of
  what older docs call `collect_render_items<H>`.
- Native handles are never public DSL constructor arguments.
- Self-drawn Core types (`VerticalLayout`, `TextBlock`, `Shape`, custom controls, etc.) do not get peers.

## 2. Decisions (user-approved 2026-11-19)

| # | Decision |
|---|---|
| D1 | Peers are created **lazily at mount**. Construction stores only Core logical state. `Window` realizes on first `show()` or first operation that needs native state. |
| D2 | The peer contract uses **typed per-control Rust traits**: a common `NativePeer` plus `ButtonPeer`, `TextBoxPeer`, etc. A generic property-key mapper is rejected. |
| D3 | A missing factory, the wrong thread, or an unsupported backend/control causes a **diagnostic panic**. There is no silent fallback. Native backend init failure stays `elwindui::init() -> Result`. |
| D4 | Delivery is **staged PRs under #313**, with no child Issues. Partial PRs use `Refs #313`; the final removal PR uses `Closes #313`. |

## 3. Module layout and dependency DAG

```text
elwindui-macros  <-  elwindui-core  <-  elwindui-backend-{winui3,appkit,gtk4}  <-  elwindui (facade)
                     ui::native            native_ui::*Peer impls + Factory
                     ui::controls::*       (no public Component structs)
```

- `elwindui_core::ui::native` (new) owns the peer traits, `NativePeerFactory`, the registry and the
  diagnostic helpers.
- `elwindui_core::ui::controls::*` owns the concrete Component structs. They are written as class-backed
  `#[component] struct` + `#[component] impl` (the PR #312 form), with `#[prop]`/`#[content]` declared once.
- Backends keep `native_ui` (now peer implementations), `inner`, `host`, `render` and `ffi`. The
  `native_ui -> inner -> host -> render -> ffi` layering is unchanged.
- The facade `elwindui::ui` re-exports Core's controls and constructor ABIs (`__elwindui_ctor_*`) from Core.
  It stops glob-re-exporting backend `native_ui`. Backend-specific `application`, `platform` and the
  `backend` alias stay as they are.

## 4. Peer contract

```rust
// elwindui_core::ui::native (control-specific traits are added per stage PR)
pub trait NativePeer {
    /// Type-erased native handle for render/host reconciliation (backend `AnyView`).
    fn handle(&self) -> &dyn Any;
    fn measure(&self, available: Size) -> Size;
    fn apply_background(&self, background: Option<&Brush>) -> bool; // false = native failure
    fn set_tooltip(&self, tooltip: Option<&str>);
    fn supports_text_style(&self) -> bool;
    fn apply_text_style(&self, style: &CascadedTextStyle) -> bool;  // false = retry later
    /// Re-push the text style on every measure (AppKit `NSButton` loses attributed titles).
    fn refresh_text_style_on_measure(&self) -> bool { false }
    /// Detach native targets/subscriptions. Idempotent. After it, no callback may reach Core.
    fn dispose(&self) {}
}

pub trait ButtonPeer: NativePeer {
    fn set_text(&self, text: &str);
    fn set_enabled(&self, enabled: bool);
    fn set_role(&self, role: ButtonRole);
    fn set_is_default(&self, is_default: bool);
    fn set_on_click(&self, callback: Option<Box<dyn Fn()>>);
}
// TextBoxPeer, CheckBoxPeer, ... ScrollViewPeer, TabViewPeer, MenuBarPeer, ... WindowPeer
```

Rules:

- `NativePeer` covers only the existing `NativeControl` surface (background, tooltip, text style, measure,
  handle, dispose). Accessibility semantics stay Core-owned intrinsic semantics on `UIElement`; the
  backend host projects them as before, so peers have no accessibility method. Control-specific traits add only that control's public properties and
  OS→Core notifications.
- Peer traits are plain Rust traits, not `#[component(trait_only)]`. They are `pub` in `elwindui-core` only
  because backends live in other crates. They are a backend-implementer surface, not a DSL surface.
- Each peer setter receives the **Core logical value**. Theme, semantic brush and text-style resolution
  stay in Core; backends only translate values to native ones.
- Natural size comes from `NativePeer::measure`. Arrange still applies bounds through host reconciliation.

### 4.1 Factory and registry

```rust
pub trait NativePeerFactory {
    fn backend_name(&self) -> &'static str;
    fn create_button(&self) -> Rc<dyn ButtonPeer> { unsupported(self.backend_name(), "Button") }
    // one method per peer kind; default bodies panic via `unsupported`
    fn create_window(&self, owner: WindowPeerOwner) -> Rc<dyn WindowPeer> { unsupported(..) }
}
pub fn register_native_peer_factory(factory: Rc<dyn NativePeerFactory>);
pub fn clear_native_peer_factory();
pub fn has_native_peer_factory() -> bool;
/// Panics with the D3 diagnostic when this thread has no factory.
pub fn create_native_peer<R>(control: &'static str, create: impl FnOnce(&dyn NativePeerFactory) -> R) -> R;
```

- The registry is a **UI-thread thread-local**, following the `graphics/text.rs` `TEXT_BACKEND` precedent.
  The backend's `init()` registers its factory on the thread that calls `elwindui::init()`. A thread that
  never registered sees "no factory".
- Re-registration replaces the factory for that thread. This is allowed for tests and must not happen in
  production. Peers that already exist keep the factory that created them.
- Multiple Windows on the UI thread share the one factory.
- Tests isolate by registering a mock factory per test thread (libtest runs each test on its own thread).
- GTK4 registers a factory that overrides no `create_*` method, so every realization panics with an
  explicit "unsupported" diagnostic.

### 4.2 Failure boundary (D3)

| Condition | Behavior |
|---|---|
| Realization with no factory on this thread (before `init()`, or wrong thread) | `panic!` naming the control type, the thread id and "call elwindui::init() on the UI thread first" |
| Factory does not support the control | `panic!` naming the backend and control (`unsupported`) |
| Native object creation fails inside the backend | The backend panics with the native error (HRESULT / ObjC exception text). No retry, no fallback peer |
| Native backend initialization fails | Unchanged: `elwindui::init()` returns `Err(InitError)` and registers no factory |

Construction never panics for these reasons, because construction never touches the factory (D1).
Core-only unit tests can therefore build controls without any factory, as long as they are not mounted
into a host.

## 5. Core Component storage and data flow

- `NativeControl` becomes `#[component(inherits UIElement, abstract_class)] struct NativeControl` with
  `#[prop] background`, `#[prop] tooltip`, a private `peer: OnceCell<Rc<dyn NativePeer>>` (set at most
  once, which also lets `try_as_native_control` return a borrow of the handle) and
  `self_weak: Weak<dyn NativeControlExt>` captured from `__self_weak` in `construct`.
- `NativeControl::construct(peer: Option<Rc<dyn NativePeer>>)` is internal (no constructor inputs).
  `None` is the lazy path. `Some(peer)` is the transitional eager path for controls not yet migrated
  (their backend `struct_only` implementors wrap their handle in a `HandlePeer`).
- Realization reaches the most-derived control's `#[doc(hidden)] #[overridable] fn __create_native_peer`
  through `self_weak`, because a method running on the base storage cannot dispatch to an override by
  itself.
- Each concrete control stores its own typed slot, for example `peer: RefCell<Option<Rc<dyn ButtonPeer>>>`.
  At realization it gives the same `Rc` (upcast) to the `NativeControl` slot. There is exactly one logical
  peer per Component.
- **Setter flow (one path per property):** Core setter → store the logical value → if a peer exists, push
  the value to the peer → invalidate measure where the property affects size. Clearing a property pushes
  the cleared or PlatformDefault value. Props without a native effect never reach the peer.
- **Two-way properties (OS→Core):** the peer reports through a callback that Core installed at
  realization. Core compares with the stored value. If the value changed, Core stores it and raises the
  `on_change`/binding notification once. Core does not push it back to the peer, so there is no echo.
  A programmatic setter pushes to the peer and does not raise the OS change notification. A backend whose
  native widget fires a change event for programmatic sets must suppress it.
- **Routed events:** a native activation (for example a Button click, or a UIA/AX `Activate` action) calls
  the Core-installed callback once. That callback dispatches the routed event (`dispatch_routed(.., "on_click", ..)`).
  User `#[routed] on_click` handlers stay on the Core routed-event table (`register_routed_handler`).
- `try_as_native_control` on `NativeControl` returns `peer.handle()` when the peer is realized, and `None`
  otherwise. `record_group_commands<H>` keeps its `downcast_ref::<H>()`. A wrong `H` stays a silent
  non-match there, which is current behavior. A peer whose `handle()` is not the backend's `AnyView`
  is a backend bug and is covered by a debug assertion in the backend host.

## 6. Lifecycle and ownership

```text
Created --mount (host bindings installed)--> Realized --unmount--> Realized(detached) --remount--> (same peer)
   |                                              |
   +--------------------- Drop ------------------+--> dispose() (exactly once)
```

- **Realization entry** is the idempotent `NativeControlExt::__native_peer()`, called at the start of
  `measure_override` and `try_as_native_control`. Both run only for a control that takes part in hosted
  layout or render collection, i.e. a mounted control under a backend host, so this is D1's "at mount"
  realization without a separate mount hook. A mount hook was rejected because Visual-parent attach also
  happens in host-less trees (Core unit tests, detached subtrees), which would then need a factory.
  The backend `__IntoAnyView::into_any_view` (below) also realizes, because it hands the handle out.
- Realization order: factory `create_*` → push every stored logical property (base first, then
  control-specific) → install OS→Core callbacks → accessibility semantics → `invalidate_measure`.
- **Unmount** detaches the native widget from its host (host reconciliation) but **keeps the peer**.
  Remount, show/hide, Collapsed/inactive and TabView page deactivation never re-create a peer.
- **Drop** of the Core Component calls `dispose()` once, which unregisters native targets and
  callback-registry entries. Dropping the peer `Rc` then releases the native object.
- **No ownership cycles:** every callback that Core gives a peer captures `Weak<Self>` (from
  `__self_weak`), never `Rc`. After the owner drops, `upgrade()` fails and the callback is a no-op. A
  native event that arrives after `dispose()` is dropped by the backend (the registry entry is gone) and
  by Core (failed upgrade). This replaces the current Button `on_constructed`, which captures `owner_rc()`
  strongly and, on WinUI3, never unregisters its global UI-callback entry.
- **Reentrancy:** Core never holds a `RefCell` borrow across a peer call or a user callback. Peer
  callbacks borrow, clone what they need, release the borrow, then dispatch. The existing TabView
  close/select discipline applies to all peers.

## 7. Window

- `Window` stays a non-`UIElement` root. Core owns `#[component] struct Window` with title, menu bar,
  content, transparency, always-on-top and one-time initial bounds as logical state, and
  `peer: RefCell<Option<Rc<dyn WindowPeer>>>`.
- `WindowPeer` is a separate trait, not a `NativePeer`: `show`, `hide`, `close`, `set_title`,
  `set_menu_bar(Option<Rc<dyn MenuBarPeer>>)`, `set_content(Option<Rc<dyn UIElementExt>>)`,
  bounds getters/setters, `set_close_request_handler`, `set_bounds_changed_handler`, `activate`,
  `close_active_popup`.
- Realization happens on first `show()` or on the first getter that needs native state (bounds). Before
  that, getters return the logical values.
- `WindowLifecycleHost` moves onto `WindowPeer`. The close-request contract (§4i) is unchanged: a
  `Weak` owner, `bool` = accepted or owner gone, mount installs the handler after chaining, and unmount
  clears it before chaining.
- The retention registry stays backend-local (§4j). The factory receives a `WindowPeerOwner` that wraps
  `Weak<dyn WindowExt>` for the most-derived owner. The backend's `app::retain_window` upgrades it on
  first show and releases it on native close. The WinUI3 `framework_initiated_close` reentrancy guard
  and the AppKit `windowShouldClose:`/`windowWillClose:` split stay in the backend `WindowPeer`.
- Generated host-composition Windows keep `#[overrides] show/hide/close/mount_override/unmount_override`.
  They now chain to Core `Window` (an ordinary class-backed base) instead of a backend `struct_only`.

## 8. Backend-managed hierarchies

- `ScrollView`: Core owns `content`. `ScrollViewPeer::set_content(Option<Rc<dyn UIElementExt>>, owner: Weak<..>)`
  hosts it in the backend's nested `TreeHost`. The peer's unmount hook (`unmount_hosted_tree`) is
  installed by Core at realization through `add_unmount_hook` and calls `peer.unmount_content()`.
- `TabView`/`TabViewItem`: Core owns `children: Vec<Rc<TabViewItem>>`, `selected_index` and the item
  props. `TabViewItem` is a Core class-backed non-`UIElement` value. The peer receives the full ordered
  item list (`set_items`) plus per-item header/closable updates. Per-tab `TreeHost`s, active-host
  switching, `rebuilding` select suppression and the `Weak` wiring move into the backend peer unchanged.
  Close/select/new-tab notifications reach Core through `Weak` callbacks.
- `Menu`/`MenuBar`/`MenuItem`/`MenuBarItem` and `Dropdown`/`DropdownItem`: Core owns the logical item
  lists (`ListExt` stays on the Core type). Peers reconcile by `Rc::ptr_eq` (existing algorithm) through
  `set_items(&[Rc<dyn MenuItemPeerSource>])`. Item peers are realized by their parent peer, not by mount,
  because items are not `UIElement`s. `Menu::create_flyout` for context menus becomes a `MenuPeer` method.
- The generic public properties of these types (`enabled`, `selected_index`, etc.) follow §5 unchanged.

## 9. Macro, constructor ABI and migration order

- Core controls use the existing class-backed `#[component] struct` + `#[component] impl` with a
  zero-argument `construct()`. Their constructor ABI `__elwindui_ctor_<Name>` is therefore exported by
  `elwindui-core`, together with `__elwindui_props_<Name>`. `abi_props` is no longer needed because the
  props macro and the ABI belong to the same declaring class.
- A generated component whose view root is a native builtin keeps its `into_any_view(self: Rc<Self>) ->
  elwindui::backend::AnyView`. Core-owned controls cannot name `AnyView`, so each backend exports a
  `#[doc(hidden)] trait __IntoAnyView` implemented for `Rc<T: NativeControlExt>`, and the generated method
  brings it into scope. Inherent `into_any_view` methods (unmigrated controls, generated components)
  still win method resolution.
- `elwindui::ui::Button` keeps resolving as a **Builtin** path in codegen (`dsl_type_origin`). Qualified
  paths, Cargo-renamed facades, same/cross-crate inheritance, required-input forwarding, multi-hop and
  rust-analyzer shadow keep using the PR #312 class-backed registry (`register_class_backed_constructor`).
  Only the declaring crate changes from backend to Core.
- Removal order (one stage per PR, every stage keeps the full gate green):
  1. Peer infrastructure + `NativeControl` + `Button` (both backends, testsupport mock factory, facade ctor
     from Core).
  2. `Window`.
  3. `CheckBox`, `RadioButton`, `Slider`, `ToggleSwitch`, `TextBox`, `TextArea`, `PasswordBox`.
  4. `Dropdown`(+`Item`), `Menu`/`MenuBar`/`MenuItem`/`MenuBarItem`, `ScrollView`, `TabView`(+`Item`).
  5. Remove `trait_only`/`struct_only`/`abi_props` from `#[component]` and `#[class]` parsing and
     expansion. Using them becomes a compile error that names the mode and points to this design. Also
     migrate `bridge-fixture-{iface,backend}`, `class_bridge_cross_crate.rs`, `testsupport.rs` and docs,
     and add negative trybuild cases.
- Compatibility: user code that only uses `elwindui::ui::*`, `view!`, `new!`, `#[component(inherits ..)]` and
  `#[overrides]` keeps compiling. User code that names a backend concrete type
  (`elwindui::backend::Button`) or that authored its own `trait_only`/`struct_only` classes breaks. That
  is accepted and recorded in the spec migration note.

## 10. Test strategy

- **Core unit tests with a mock factory** (counters per peer method):
  - one `create_*` per Component across mount/unmount/remount/show/hide/layout;
  - property push order and values;
  - two-way echo suppression;
  - a single routed event per native activation;
  - Drop → `dispose()` once, `Weak` cannot upgrade, stale callback is a no-op;
  - `#[should_panic(expected = ..)]` for no factory, a different thread and an unsupported control.
- **Window:** a mock `WindowPeer` covers first-show retention, hide/re-show state, cancelled native close,
  reentrant close, accepted close release and popup teardown order.
- **Construction/inheritance regressions:** existing `class_backed_component.rs`, cross-crate fixtures,
  `new!`/`view!`/template, Cargo alias, multi-hop, generated→handwritten→generated, `#[overridable]`.
- **Removal:** trybuild compile-fail cases for each removed mode in `crates/elwindui/tests/ui/`.
- **Native hosts:** WinUI3 E2E on a Windows host and AppKit E2E on a macOS host, following
  `docs/agents/testing.md` host rules. A host that cannot be run is reported `BLOCKED` and is not
  counted as passed.

## 11. Alternatives rejected

- **Eager peer creation at construct:** requires a factory for every Core unit test and for any
  construction before `init()`, and keeps native resources for never-mounted controls.
- **Property-key mapper (`set_property(PropId, Value)`):** moves type errors to runtime and needs a
  dynamic value enum that duplicates the `#[prop]` schema.
- **Result-returning realization:** realization happens inside mount/measure, which has no error channel.
  Threading `Result` through those paths would change many signatures for a programming error.
- **Per-OS Core implementations behind `cfg`:** explicitly forbidden by #313. It would also reintroduce
  backend types into Core.
