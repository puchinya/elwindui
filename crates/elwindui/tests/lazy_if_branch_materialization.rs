//! End-to-end runtime proof for Issue #52's lazy-once `if` branch materialization
//! (`elwindui-codegen/src/codegen.rs`'s `lazy_branch_plan`/`emit_lazy_leaf_value`): an `if`/`else`
//! branch made only of childless literal elements must construct its widget subtree at most once,
//! only once it actually becomes the active branch — never both branches unconditionally at
//! startup (the old eager-construction behavior), and never a second time on switching back to an
//! already-materialized branch.
//!
//! `ThenLeaf`/`ElseLeaf` each record their own construction in a thread-local counter from
//! `on_mount` (fires exactly once, from `on_constructed`, the same call that reaches
//! `__refresh_dynamic_regions` — see `codegen.rs`'s own `on_constructed` doc comment), giving a
//! direct, unambiguous signal that is meaningless to fake by any codegen shortcut short of
//! actually constructing the leaf.
//!
//! The toggling condition (`show_then`) is deliberately `ToggleHost`'s own mutable `#[prop]`
//! field, not an injected `#[bindable]` viewmodel property — mirrors `examples/notepad`'s real,
//! screenshot-verified `CustomCheckBox` (`is_checked`, toggled via `on_tapped`). A *bind-owner*-
//! driven `if`/`match` condition (e.g. `if vm.show { .. }` for a `#[bindable] vm`) turns out to
//! have a separate, pre-existing gap unrelated to lazy-once materialization:
//! `property_resync_methods_for` only scans each dynamic node's own *attributes* for which
//! bind-owner properties they depend on, never a dynamic region's own `condition`/`match value`
//! expression — so `__resync_<owner>` never fires `__refresh_dynamic_regions()` off of a condition-
//! only dependency. An own-field condition doesn't hit this gap: `component_self_subscription`
//! calls `__refresh_dynamic_regions()` unconditionally on every own-property change, regardless of
//! which attribute/condition actually depends on it.

// Required in the crate root of anything using `#[elwindui_macros::class]` (which every
// `inherits`-carrying component becomes) — see docs/specs/macro_class_spec.md §10 and
// `examples/viewmodel-attr-demo/src/main.rs`'s identical top-of-file comment.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::Cell;

thread_local! {
    static THEN_MOUNTS: Cell<u32> = Cell::new(0);
    static ELSE_MOUNTS: Cell<u32> = Cell::new(0);
}

#[elwindui::component(inherits ContentControl)]
struct ThenLeaf {
    body: view! {
        on_mount {
            THEN_MOUNTS.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "then" }
    },
}

#[elwindui::component]
impl ThenLeaf {}

#[elwindui::component(inherits ContentControl)]
struct ElseLeaf {
    body: view! {
        on_mount {
            ELSE_MOUNTS.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "else" }
    },
}

#[elwindui::component]
impl ElseLeaf {}

#[elwindui::component(inherits ContentControl)]
struct ToggleHost {
    #[prop(default = true)]
    show_then: bool,

    body: view! {
        VerticalLayout {
            if show_then {
                ThenLeaf {}
            } else {
                ElseLeaf {}
            }
        }
    },
}

#[elwindui::component]
impl ToggleHost {
    #[overridable]
    fn flip(&self) {
        self.set_show_then(!self.show_then());
    }
}

#[test]
fn unreached_branch_is_never_constructed_and_switching_materializes_once() {
    let host = ToggleHost::new();

    assert_eq!(
        THEN_MOUNTS.with(|c| c.get()),
        1,
        "the initially-active `then` branch must construct exactly once at startup"
    );
    assert_eq!(
        ELSE_MOUNTS.with(|c| c.get()),
        0,
        "the inactive `else` branch must never construct while unreached"
    );

    host.flip();
    assert_eq!(
        ELSE_MOUNTS.with(|c| c.get()),
        1,
        "switching to the `else` branch must materialize it exactly once"
    );
    assert_eq!(
        THEN_MOUNTS.with(|c| c.get()),
        1,
        "switching away from `then` must not reconstruct or drop the already-cached instance"
    );

    host.flip();
    assert_eq!(
        THEN_MOUNTS.with(|c| c.get()),
        1,
        "switching back to `then` must reuse its cached instance, not rebuild it"
    );
    assert_eq!(
        ELSE_MOUNTS.with(|c| c.get()),
        1,
        "switching away from `else` must not reconstruct or drop its cached instance either"
    );

    host.flip();
    host.flip();
    assert_eq!(THEN_MOUNTS.with(|c| c.get()), 1);
    assert_eq!(ELSE_MOUNTS.with(|c| c.get()), 1);
}
