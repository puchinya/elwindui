//! End-to-end runtime proof for Issue #58: a `#[bindable]` viewmodel property referenced only in an
//! `if` **condition** (never in a sibling attribute) must still switch the active branch when that
//! property changes — for a `component X inherits ContentControl` host exactly like the real
//! `#[elwindui::component]` components in `examples/`, not just the codegen-string-check tests.
//!
//! Two independent root causes both had to be fixed for this to work (`elwindui-codegen/src/
//! codegen.rs`):
//!
//! 1. `property_resync_methods_for`'s per-bind-owner `__resync_<owner>(&self, property)` collected
//!    its `match property { .. }` arms by scanning only `PlannedNode::attributes` (via
//!    `collect_view_expr_owner_properties`), never a dynamic region's own `DynamicPlan::If.condition`
//!    / `Match.value` / `For.collection`. A bind-owner property referenced *only* there fell through
//!    the generated `_ => {}` catch-all, so no arm at all called `self.__refresh_dynamic_regions()` —
//!    even though the `vm.subscribe_property_changed(..)` subscription itself was correctly wired and
//!    `__resync_vm` was actually being called.
//! 2. Separately, `generate_view`'s `is_composed` branch (any component whose `inherits` base
//!    ultimately resolves to a real native widget — i.e. almost every real `#[elwindui::component]`)
//!    built its bind-owner resync arms with `include_refresh: false`, on the mistaken premise that
//!    something else already calls `__refresh_dynamic_regions()` on a bind owner's property change
//!    for a composed component. Nothing does — `component_self_subscription` (the *own*-`#[prop]`-
//!    field path) calls it unconditionally regardless of `is_composed`, but the bind-owner path did
//!    not mirror that. So even a bind-owner property that *did* get an arm (fix 1) still never
//!    switched the branch in any composed component. Fixed by reusing the same
//!    `include_refresh: true` build for both the composed and non-composed cases.
//!
//! `ThenLeaf`/`ElseLeaf` mirror `lazy_if_branch_materialization.rs`'s thread-local-counter proof:
//! each records its own construction from `on_mount`, giving a direct signal that only a real branch
//! switch can produce.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::Cell;
use std::rc::Rc;

use elwindui::core::ui::UIElementExt as _;

thread_local! {
    static THEN_MOUNTS: Cell<u32> = Cell::new(0);
    static ELSE_MOUNTS: Cell<u32> = Cell::new(0);
}

#[elwindui::viewmodel]
mod toggle_view_model {
    struct ToggleViewModel {
        #[observable(default = true)]
        show_then: bool,
    }
}

#[elwindui::component(inherits ContentControl)]
struct ThenLeaf {
    template: template_view!(|templated_parent: Self| {
        on_mount {
            THEN_MOUNTS.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "then" }
    }),
}

#[elwindui::component]
impl ThenLeaf {}

#[elwindui::component(inherits ContentControl)]
struct ElseLeaf {
    template: template_view!(|templated_parent: Self| {
        on_mount {
            ELSE_MOUNTS.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "else" }
    }),
}

#[elwindui::component]
impl ElseLeaf {}

#[elwindui::component(inherits ContentControl)]
struct BindOwnerToggleHost {
    #[bindable]
    vm: Rc<ToggleViewModel>,

    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            if vm.show_then {
                ThenLeaf {}
            } else {
                ElseLeaf {}
            }
        }
    }),
}

#[elwindui::component]
impl BindOwnerToggleHost {}

#[test]
fn bind_owner_driven_if_condition_switches_on_property_change() {
    let vm = ToggleViewModel::new();
    let host = elwindui::new!(BindOwnerToggleHost(vm: Rc::clone(&vm)));
    assert!(host.apply_template());

    assert_eq!(
        THEN_MOUNTS.with(|c| c.get()),
        1,
        "the initially-active `then` branch must construct at startup"
    );
    assert_eq!(
        ELSE_MOUNTS.with(|c| c.get()),
        0,
        "the inactive `else` branch must not construct while unreached"
    );

    vm.set_show_then(false);
    assert_eq!(
        ELSE_MOUNTS.with(|c| c.get()),
        1,
        "a `#[bindable]` vm property used only in the `if` condition must still switch the branch \
         on change (issue #58)"
    );

    vm.set_show_then(true);
    assert_eq!(
        THEN_MOUNTS.with(|c| c.get()),
        1,
        "switching back to `then` must not reconstruct it"
    );

    drop(host);
}
