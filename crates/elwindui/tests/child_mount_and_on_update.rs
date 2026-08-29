//! CI-4 of #80 (docs/design/runtime/component_lifecycle_design.md §4b/§4c): plan-driven descendant
//! construction was moved from `construct()` (pre-`Rc`) into `__build_view()` (post-`Rc`), which
//! required every `stored` child field to become `OnceCell<Rc<ConcreteType>>` instead of a plain,
//! directly-owned field. `on_update(field, ...)` codegen was also implemented for the first time
//! (previously parsed nowhere, silently a no-op).

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static MOUNT_ORDER: Cell<u32> = const { Cell::new(0) };
    static CHILD_A_MOUNT_ORDER: Cell<u32> = const { Cell::new(0) };
    static CHILD_B_MOUNT_ORDER: Cell<u32> = const { Cell::new(0) };
    static PARENT_MOUNT_ORDER: Cell<u32> = const { Cell::new(0) };
}

fn next_order() -> u32 {
    MOUNT_ORDER.with(|c| {
        let v = c.get() + 1;
        c.set(v);
        v
    })
}

#[elwindui::component(inherits ContentControl)]
struct MountOrderChildA {
    template: template_view!(|templated_parent: Self| {
        on_mount {
            let n = next_order();
            CHILD_A_MOUNT_ORDER.with(|c| c.set(n));
        }
        TextBlock { text: "a" }
    }),
}

#[elwindui::component]
impl MountOrderChildA {}

#[elwindui::component(inherits ContentControl)]
struct MountOrderChildB {
    template: template_view!(|templated_parent: Self| {
        on_mount {
            let n = next_order();
            CHILD_B_MOUNT_ORDER.with(|c| c.set(n));
        }
        TextBlock { text: "b" }
    }),
}

#[elwindui::component]
impl MountOrderChildB {}

#[elwindui::component(inherits VerticalLayout)]
struct MountOrderParent {
    body: view! {
        on_mount {
            let n = next_order();
            PARENT_MOUNT_ORDER.with(|c| c.set(n));
        }
        #[id("child_a")]
        let child_a = MountOrderChildA { };
        #[id("child_b")]
        let child_b = MountOrderChildB { };

        child_a
        child_b
    },
}

#[elwindui::component]
impl MountOrderParent {}

#[test]
fn stored_children_build_before_parent_on_mount_and_named_accessors_work_immediately() {
    MOUNT_ORDER.with(|c| c.set(0));
    CHILD_A_MOUNT_ORDER.with(|c| c.set(0));
    CHILD_B_MOUNT_ORDER.with(|c| c.set(0));
    PARENT_MOUNT_ORDER.with(|c| c.set(0));

    let parent = MountOrderParent::new();

    // Both children's own `on_mount` fired strictly before the parent's own `on_mount` -- child
    // construction (now happening from the parent's post-`Rc` `__build_view()`, CI-4) still
    // completes, including each child's own build, before the parent's own build continues.
    let a = CHILD_A_MOUNT_ORDER.with(|c| c.get());
    let b = CHILD_B_MOUNT_ORDER.with(|c| c.get());
    let p = PARENT_MOUNT_ORDER.with(|c| c.get());
    assert!(
        a > 0 && b > 0 && p > 0,
        "all three should have fired: a={a} b={b} p={p}"
    );
    assert!(
        a < p,
        "child_a's on_mount ({a}) must fire before the parent's ({p})"
    );
    assert!(
        b < p,
        "child_b's on_mount ({b}) must fire before the parent's ({p})"
    );

    // `#[id(..)]` named accessors now read through `OnceCell<Rc<ConcreteType>>` instead of a plain
    // field (CI-4) -- these `.expect(..)`-panic internally if the cell was never populated, so a
    // successful, correctly-typed read here is a direct regression test for that storage change.
    let child_a: Rc<MountOrderChildA> = parent.child_a();
    let child_b: Rc<MountOrderChildB> = parent.child_b();
    let _ = (child_a, child_b);
}

thread_local! {
    static ON_UPDATE_COUNT: Cell<u32> = const { Cell::new(0) };
}

#[elwindui::component(inherits ContentControl)]
struct OnUpdateProbe {
    #[prop]
    label: String,

    template: template_view!(|templated_parent: Self| {
        on_update(label): {
            ON_UPDATE_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: label }
    }),
}

#[elwindui::component]
impl OnUpdateProbe {}

#[test]
fn on_update_fires_after_prop_change_but_not_on_initial_construction() {
    ON_UPDATE_COUNT.with(|c| c.set(0));

    let probe = elwindui::new!(OnUpdateProbe(label: "hello".to_string()));
    assert_eq!(
        ON_UPDATE_COUNT.with(|c| c.get()),
        0,
        "the initial construction-time value-set must not count as an on_update"
    );

    probe.set_label("world".to_string());
    assert_eq!(ON_UPDATE_COUNT.with(|c| c.get()), 1);

    probe.set_label("world again".to_string());
    assert_eq!(ON_UPDATE_COUNT.with(|c| c.get()), 2);
}
