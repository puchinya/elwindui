# Class Model & `#[class]` Macro Guidelines

Guidelines for AI agents working with `elwindui-macros` (`#[elwindui_macros::class]`) or hand-written runtime class hierarchies in `elwindui-core` and backend crates.

## Specification Source of Truth

- **Authoritative spec**: [`docs/specs/macro_class_spec.md`](../specs/macro_class_spec.md).
- **Internal design**: [`docs/design/tools/class_macro_design.md`](../design/tools/class_macro_design.md).
- If source or design conflicts with `macro_class_spec.md`, do not change the spec without an approved contract change.

## Class Hierarchy Convention

For a class `Class` with parent `SuperClass`:

1. **Struct definition**:
   ```rust
   pub struct Class {
       base: SuperClass,
       // own fields...
   }
   ```
   Root classes (with no parent) omit the `base` field. Use bare struct names with no suffix.
2. **Trait definition**:
   ```rust
   pub trait ClassExt: SuperClassExt {
       // own methods...
   }
   ```
3. **Ancestor trait delegation**:
   `Class` must implement `ClassExt` and every ancestor trait in the hierarchy. Ancestor method implementations delegate to `self.base.method(...)`.
4. **Factory construction**:
   Always construct instances via a `create_class(...)` factory function. Never construct bare struct literals directly in external code.

## Macro Generated Code & IDE Support

- `#[elwindui_macros::class]` automates struct/trait generation, inheritance macro expansion (`__elwindui_inherit_*!`), and `rust-analyzer` shadow generation.
- Always verify changes to `#[class]` by running `rust-analyzer diagnostics .` in addition to `cargo check`.

## Override chains and removed class modes

`#[overridable]`/`#[overrides]` work at arbitrary descendant depth and across real crate boundaries through the ordinary class chain. Do not introduce a new, type-specific override workaround (inherent-method shadowing, UFCS, a second override system) — use the normal mechanism. `Window::show`/`hide`/`close` (`crates/elwindui-core/src/ui/controls/window.rs`) is the reference example.

The `trait_only`, `struct_only`, `abi_props` and `no_ancestor_forward` modes were removed by Issue #313 and are rejected by name. Do not reintroduce an interface/implementor split for a native control: declare it once in Core as a class-backed component and put the OS widget behind a backend peer (`docs/design/runtime/native_peer_design.md`).

Ancestor-trait resolution goes through a stable, per-class `__ElwindUIOwnExt` alias (see `class_macro_design.md`). Do not add a `{ConcreteName}Ext` compatibility alias; if resolution looks broken, fix it at the macro level.

Root mode (a class with no `inherits` at all, e.g. `UIElement`) has `as_ui_element(&self) -> &Self`, a required trait method hard-pinned to the root's concrete type; descendants forward it to `self.base.as_ui_element()`. Do not redesign `as_ui_element` itself (e.g. to return `&dyn RootExt`). Reference fixture: `crates/elwindui/tests/class_chain_cross_crate.rs` (cross-crate).

## Cross-crate generic ancestor resolution (PR #164 final remediation round, finding A5)

Whether a descendant's own `inherits = Ancestor<Args>` trailing generic arguments must reattach onto the ancestor's `{Name}Ext` trait depends on the ancestor's *shape*, not on any same-crate bookkeeping: reattach for a generic ordinary/root ancestor. Do not decide this from `same_crate_classes`/any other same-crate-only registry (it is empty in a separate `cargo build` process and so is wrong cross-crate by construction), and do not globally append or drop generic arguments independent of the specific ancestor being recursed through. The registry-independent replacement (`__ElwindUIOwnExtBound_<bare_name>` for supertrait-bound position, `__elwindui_apply_own_ext_<bare_name>` for dispatch-routing position — see `class_macro_design.md`'s own section on this) is per-ancestor and resolved at either proc-macro time (bound position) or macro-expansion time (dispatch position); if you need a third such decision point, follow the same split rather than introducing a new registry.

This applies identically at *every* recursive hop, not only the immediate `inherits = ..` edge (PR #164 final remediation round, finding A6): a class's own generated classify macros are reused for every future descendant depth, so a class's own "forward past me into whatever I inherit" metadata (`AncestorRoute` in `class.rs`) must stay just as unresolved as the immediate hop's own metadata — routed through a fixed, once-per-class continuation macro that asks the *next* ancestor's own policy macro fresh, at the point some real descendant chain actually reaches that hop. An earlier revision only fixed the immediate hop this way and left the recursive case baking a same-crate-registry-derived guess as a literal token — wrong for any ancestor beyond the immediate one that isn't in the same crate as the class generating the recursive metadata.

## `Window::mount_override`/`unmount_override` (Issue #162, PR #165 review remediation A6)

`Window` declares two `#[overridable]` hooks — `mount_override(&self, environment)` and `unmount_override(&self)` — reached through the ordinary `#[overridable]`/`#[overrides]` mechanism above, exactly like any other class-managed interface method. They are **framework-reserved**, not a second DSL-author-facing lifecycle surface: a user-authored `#[overrides] fn mount_override`/`unmount_override` on a `#[elwindui::component] impl` is rejected with a diagnostic pointing at `on_mount`/`on_unmount` (`docs/specs/dsl_spec.md` §13 rule 38) — those two remain the only lifecycle hooks a DSL author ever writes. `mount()`/`unmount()` themselves stay fixed, non-overridable framework algorithms; `mount_override`/`unmount_override` exist purely so the framework has a normal override point to reach through, spliced into the generated `mount()`/`unmount()` at fixed points (`docs/design/runtime/component_lifecycle_design.md` §4i).

The generated host-composition (`inherits Window`) component's own `#[overrides] mount_override`/`unmount_override` are ordinary generated methods, reached through the same ancestor-forwarding chain as `show`/`hide`/`close` — no inherent-method/UFCS workaround. The two are **not** symmetric in ordering (PR #165 rereview remediation round 2, A7 — an earlier revision of this paragraph incorrectly claimed both "chain into `self.base` first, then install/clear"):

```text
mount_override:   self.base.mount_override(..) first, THEN install the close-request handler
unmount_override: clear the close-request handler FIRST, then self.base.unmount_override()
```

`unmount_override` clears the handler *before* forwarding to the backend deliberately — the backend's native close-request storage must never keep pointing at a closure this component is about to tear down while `self.base.unmount_override()` (which closes any active popup, see below) is still running. Both directions use `#[doc(hidden)] WindowLifecycleHost::set_close_request_handler`, called through UFCS the same way `self.base.show()` etc. already are.

**Native close callback contract**: the installed `Rc<dyn Fn() -> bool>` closure captures only a type-erased `Weak<dyn Any>` (this component's own `__self_weak`, cloned — never a strong `Rc<Self>`), and its return value has a fixed meaning both backends' native close affordance (AppKit `windowShouldClose:`, WinUI3 `AppWindow.Closing`) must honor identically:

```text
true  -> the framework accepted the close request and is now handling it through its own
         lifecycle (the generated WindowExt::close()) -- veto the original native close attempt
false -> the generated owner is already gone -- allow the native default close to proceed
```

Cancelling the native close unconditionally regardless of this return value is a contract violation (PR #165 finding A1) — it strands the native window open with no framework owner left to ever close it. Both backends extract this as a pure, unit-tested decision function (`elwindui-backend-appkit::inner::window::should_allow_native_close`, `elwindui-backend-winui3::inner::window::decide_native_close`/`should_veto_native_close`) rather than inlining the branch directly into the native callback, so the decision logic is testable without constructing a real native window (which needs the main thread, unavailable in any `#[test]` harness — see `crates/elwindui/tests/window_mount_hide_close.rs`'s own type-check-only convention for why). WinUI3's own native close event additionally needs a `framework_initiated_close` reentrancy guard AppKit does not (`NSWindow::close` never consults `windowShouldClose:` at all — Apple's documented contract — while `AppWindow.Closing` fires for *any* close, including the framework's own).

`unmount_override` closes the Window's own active custom popup/context-menu surface (`TreeHost::close_active_popup`) *before* its own content unmounts, and clears the close-request handler *before* forwarding to the backend's own `unmount_override` — a stale handler must never remain reachable once the Window starts tearing down.

## `__self_weak` in `Window::construct()` (Issue #254)

Core `Window` (a concrete root class, Issue #313) references `__self_weak` directly inside `construct()`'s own body — `self_weak: __self_weak.clone()` — rather than through the post-construction `on_constructed`/`owner_rc()` idiom other native controls use (`Window` has no `visual_collection`). It hands that `Weak` to `NativePeerFactory::create_window`, and the backend `WindowPeer`'s `InnerWindow` upgrades it on first show. `__self_weak`'s type is `Weak<dyn WindowExt>` — for a bare `Window::new()` this points at that `Rc<Window>` itself; for a generated `component X inherits Window` (host composition), it points at the *outer* generated component's own `Rc`, coerced upward through the same ordinary supertrait-bound mechanism `#[class]` already uses elsewhere (§"Cross-crate generic ancestor resolution" above) — never a separate, inner `Rc<Window>`.

`InnerWindow` stores this as a plain `Weak<dyn WindowExt>` field and hands it to the backend's own `crate::app::retain_window` on first `show()`, which is what becomes the strong lifetime authority for the final owner. This is a distinct mechanism from `WindowLifecycleHost`'s close-request handler above (which also captures a `Weak`, but for the unrelated purpose of routing a native close *request* into the generated `close()` lifecycle) — do not conflate the two, and do not add a second `Weak`-storing field where one already suffices for both purposes' respective call sites. See `docs/design/runtime/component_lifecycle_design.md` §4j for the full cross-backend retention invariant.
