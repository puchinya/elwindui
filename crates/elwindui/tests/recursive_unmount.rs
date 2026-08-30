//! Integration tests for Issue #126: Generic Component Unmount / Recursive Subtree Teardown.
//!
//! Verifies:
//! - Child-first teardown ordering (descendant on_unmount -> parent on_unmount)
//! - Exactly-once on_unmount execution (idempotency & double unmount)
//! - Reentrancy safety from inside on_unmount
//! - Subscription detachment (property-changed and environment live-updates)
//! - No leak / Weak reference drops to 0
//! - Deep hierarchy (3+ levels)
//! - Dynamic region (if) subtree unmount

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use elwindui::core::ui::{ControlExt as _, UIElementExt as _};

thread_local! {
    static UNMOUNT_EVENTS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static CHILD_PROP_CHANGED_COUNT: Cell<u32> = const { Cell::new(0) };
    static ENV_CHANGED_COUNT: Cell<u32> = const { Cell::new(0) };
}

fn record_unmount(name: &'static str) {
    UNMOUNT_EVENTS.with(|events| events.borrow_mut().push(name));
}

fn get_unmount_events() -> Vec<&'static str> {
    UNMOUNT_EVENTS.with(|events| events.borrow().clone())
}

fn clear_unmount_events() {
    UNMOUNT_EVENTS.with(|events| events.borrow_mut().clear());
}

// ---------------------------------------------------------------------------
// 1. Nested hierarchy components
// ---------------------------------------------------------------------------

#[elwindui::component(inherits ContentControl)]
struct LeafChild {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("LeafChild");
        }
        TextBlock { text: "leaf" }
    }),
}

#[elwindui::component]
impl LeafChild {}

#[elwindui::component(inherits VerticalLayout)]
struct MiddleParent {
    body: view! {
        on_unmount {
            record_unmount("MiddleParent");
        }
        #[id("leaf")]
        let leaf = LeafChild { };

        leaf
    },
}

#[elwindui::component]
impl MiddleParent {}

#[elwindui::component(inherits VerticalLayout)]
struct TopContainer {
    body: view! {
        on_unmount {
            record_unmount("TopContainer");
        }
        #[id("middle")]
        let middle = MiddleParent { };

        middle
    },
}

#[elwindui::component]
impl TopContainer {}

#[test]
fn test_recursive_unmount_child_first_order() {
    clear_unmount_events();

    let top = TopContainer::new();
    let middle = top.middle();
    let leaf = middle.leaf();
    assert!(leaf.apply_template());
    assert_eq!(get_unmount_events().len(), 0);

    top.unmount();

    assert_eq!(
        get_unmount_events(),
        vec!["LeafChild", "MiddleParent", "TopContainer"],
        "Unmount must execute in child-first order"
    );

    // Double unmount must be safe no-op (idempotent)
    top.unmount();
    assert_eq!(
        get_unmount_events(),
        vec!["LeafChild", "MiddleParent", "TopContainer"],
        "Second unmount must be a no-op"
    );
}

// ---------------------------------------------------------------------------
// 2. Subscription cancellation test
// ---------------------------------------------------------------------------

#[elwindui::component(inherits ContentControl)]
struct SubscribingChild {
    #[prop]
    label: String,

    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("SubscribingChild");
        }
        on_update(label): {
            CHILD_PROP_CHANGED_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: label }
    }),
}

#[elwindui::component]
impl SubscribingChild {}

#[elwindui::component(inherits VerticalLayout)]
struct SubscribingParent {
    #[prop]
    text: String,

    body: view! {
        on_unmount {
            record_unmount("SubscribingParent");
        }
        #[id("child")]
        let child = SubscribingChild { label: text };

        child
    },
}

#[elwindui::component]
impl SubscribingParent {}

#[test]
fn test_unmount_cancels_property_subscriptions() {
    clear_unmount_events();
    CHILD_PROP_CHANGED_COUNT.with(|c| c.set(0));

    let parent = elwindui::new!(SubscribingParent(text: "initial".to_string()));
    assert!(parent.child().apply_template());
    let initial_count = CHILD_PROP_CHANGED_COUNT.with(|c| c.get());

    parent.set_text("second".to_string());
    assert_eq!(
        CHILD_PROP_CHANGED_COUNT.with(|c| c.get()),
        initial_count + 1,
        "Subscription should receive property change while mounted"
    );

    parent.unmount();
    assert_eq!(
        get_unmount_events(),
        vec!["SubscribingChild", "SubscribingParent"]
    );

    // After unmount, mutations to parent property must not trigger child update
    parent.set_text("third".to_string());
    assert_eq!(
        CHILD_PROP_CHANGED_COUNT.with(|c| c.get()),
        initial_count + 1,
        "Subscription must be detached after unmount"
    );
}

// ---------------------------------------------------------------------------
// 3. Environment listener release & Weak lifetime verification
// ---------------------------------------------------------------------------

#[elwindui::environment_key(
    name = recursive_unmount_theme_color,
    value = String,
    default = String::from("light")
)]
pub struct RecursiveUnmountThemeColor;

#[elwindui::component(inherits ContentControl)]
struct EnvChild {
    #[environment(recursive_unmount_theme_color)]
    theme: String,

    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("EnvChild");
        }
        on_update(theme): {
            ENV_CHANGED_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: theme }
    }),
}

#[elwindui::component]
impl EnvChild {}

#[elwindui::component(inherits VerticalLayout)]
struct EnvParent {
    body: view! {
        on_unmount {
            record_unmount("EnvParent");
        }
        #[id("env_child")]
        let env_child = EnvChild { };
        env_child
    },
}

#[elwindui::component]
impl EnvParent {}

#[test]
fn test_environment_listener_released_and_weak_drops() {
    clear_unmount_events();
    ENV_CHANGED_COUNT.with(|c| c.set(0));

    let env = elwindui::core::environment::application_environment();
    env.set::<RecursiveUnmountThemeColor>("light".to_string());

    let parent = EnvParent::new();
    assert!(parent.env_child().apply_template());
    let child_weak: Weak<EnvChild> = Rc::downgrade(&parent.env_child());
    let parent_weak: Weak<EnvParent> = Rc::downgrade(&parent);

    assert!(child_weak.upgrade().is_some());
    assert!(parent_weak.upgrade().is_some());

    env.set::<RecursiveUnmountThemeColor>("dark".to_string());
    assert_eq!(ENV_CHANGED_COUNT.with(|c| c.get()), 1);

    parent.unmount();

    // After unmount, environment changes must not fire listener
    env.set::<RecursiveUnmountThemeColor>("blue".to_string());
    assert_eq!(
        ENV_CHANGED_COUNT.with(|c| c.get()),
        1,
        "Environment listener must be detached after unmount"
    );

    // Drop external strong Rc
    drop(parent);

    // Neither child nor parent should be retained by Environment or subscriptions
    assert!(
        child_weak.upgrade().is_none(),
        "Child component must be dropped when external Rc is released"
    );
    assert!(
        parent_weak.upgrade().is_none(),
        "Parent component must be dropped when external Rc is released"
    );
}

// ---------------------------------------------------------------------------
// 4. Reentrancy safety test
// ---------------------------------------------------------------------------

thread_local! {
    static REENTRANT_SELF: RefCell<Option<Rc<ReentrantComponent>>> = const { RefCell::new(None) };
}

#[elwindui::component(inherits ContentControl)]
struct ReentrantComponent {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("ReentrantComponent:start");
            // Call unmount reentrantly from inside on_unmount
            REENTRANT_SELF.with(|cell| {
                if let Some(r) = cell.borrow().as_ref() {
                    r.unmount();
                }
            });
            record_unmount("ReentrantComponent:end");
        }
        TextBlock { text: "reentrant" }
    }),
}

#[elwindui::component]
impl ReentrantComponent {}

#[test]
fn test_reentrant_unmount_is_safe() {
    clear_unmount_events();

    let comp = ReentrantComponent::new();
    assert!(comp.apply_template());
    REENTRANT_SELF.with(|cell| *cell.borrow_mut() = Some(comp.clone()));

    comp.unmount();

    REENTRANT_SELF.with(|cell| *cell.borrow_mut() = None);

    assert_eq!(
        get_unmount_events(),
        vec!["ReentrantComponent:start", "ReentrantComponent:end"],
        "Reentrant unmount must be ignored by idempotency guard without recursion or panic"
    );
}

// ---------------------------------------------------------------------------
// 5. Dynamic if subtree unmount
// ---------------------------------------------------------------------------

#[elwindui::viewmodel]
mod dynamic_switch_view_model {
    struct DynamicSwitchViewModel {
        #[observable(default = true)]
        show_child: bool,
    }
}

#[elwindui::component(inherits ContentControl)]
struct DynamicIfChild {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("DynamicIfChild");
        }
        TextBlock { text: "dynamic if child" }
    }),
}

#[elwindui::component]
impl DynamicIfChild {}

#[elwindui::component(inherits ContentControl)]
struct DynamicIfHost {
    #[bindable]
    vm: std::rc::Rc<DynamicSwitchViewModel>,

    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            if vm.show_child {
                DynamicIfChild { }
            }
        }
    }),
}

#[elwindui::component]
impl DynamicIfHost {}

#[test]
fn test_dynamic_if_branch_removal_triggers_unmount() {
    clear_unmount_events();

    let vm = DynamicSwitchViewModel::new();
    let host = elwindui::new!(DynamicIfHost(vm: vm.clone()));
    assert!(host.apply_template());
    let child = elwindui::core::visual_tree::find_all::<DynamicIfChild>(host.as_ref())
        .into_iter()
        .next()
        .expect("active dynamic if child");
    let child = child
        .as_any()
        .downcast_ref::<DynamicIfChild>()
        .expect("dynamic if child has its concrete type");
    assert!(child.apply_template());

    assert_eq!(get_unmount_events().len(), 0);

    // Switch branch from true to false
    vm.set_show_child(false);

    assert_eq!(
        get_unmount_events(),
        vec!["DynamicIfChild"],
        "Dynamic if removal must trigger unmount of removed branch component"
    );
}

// ---------------------------------------------------------------------------
// 6. Direct component hierarchy recursive unmount test
// ---------------------------------------------------------------------------

#[elwindui::component(inherits ContentControl)]
struct PlainChild {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("PlainChild");
        }
        TextBlock { text: "plain child" }
    }),
}

#[elwindui::component]
impl PlainChild {}

#[elwindui::component(inherits ContentControl)]
struct PlainParent {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("PlainParent");
        }
        VerticalLayout {
            PlainChild { }
        }
    }),
}

#[elwindui::component]
impl PlainParent {}

#[test]
fn test_plain_component_recursive_unmount() {
    clear_unmount_events();

    let parent = PlainParent::new();
    assert!(parent.apply_template());
    let child = elwindui::core::visual_tree::find_all::<PlainChild>(parent.as_ref())
        .into_iter()
        .next()
        .expect("plain child");
    let child = child
        .as_any()
        .downcast_ref::<PlainChild>()
        .expect("plain child has its concrete type");
    assert!(child.apply_template());
    assert_eq!(get_unmount_events().len(), 0);

    parent.unmount();
    assert_eq!(
        get_unmount_events(),
        vec!["PlainChild", "PlainParent"],
        "Parent unmount must recursively unmount descendant child in child-first order"
    );

    // Second unmount must be a no-op (idempotency)
    parent.unmount();
    assert_eq!(
        get_unmount_events(),
        vec!["PlainChild", "PlainParent"],
        "Second unmount must be a no-op"
    );
}

// ---------------------------------------------------------------------------
// 7. Dynamic for removal test
// ---------------------------------------------------------------------------

#[elwindui::viewmodel]
mod dynamic_item_view_model {
    struct ItemViewModel {
        #[observable(default = String::new())]
        name: String,
    }
}

#[elwindui::viewmodel]
mod dynamic_for_view_model {
    use super::ItemViewModel;

    struct DynamicForViewModel {
        #[observable(default = Vec::new())]
        items: Vec<ItemViewModel>,
    }
}

#[elwindui::component(inherits ContentControl)]
struct DynamicForChild {
    #[bindable]
    vm: std::rc::Rc<ItemViewModel>,

    template: template_view!(|templated_parent: Self| {
        on_unmount {
            if self.vm().name() == "A" {
                record_unmount("DynamicForChild:A");
            } else if self.vm().name() == "B" {
                record_unmount("DynamicForChild:B");
            } else if self.vm().name() == "C" {
                record_unmount("DynamicForChild:C");
            }
        }
        TextBlock { text: vm.name }
    }),
}

#[elwindui::component]
impl DynamicForChild {}

#[elwindui::component(inherits ContentControl)]
struct DynamicForHost {
    #[bindable]
    vm: std::rc::Rc<DynamicForViewModel>,

    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            for item in vm.items {
                DynamicForChild { vm: item }
            }
        }
    }),
}

#[elwindui::component]
impl DynamicForHost {}

#[test]
fn test_dynamic_for_removal_triggers_unmount() {
    clear_unmount_events();

    let item_a = ItemViewModel::new();
    item_a.set_name("A".to_string());
    let item_b = ItemViewModel::new();
    item_b.set_name("B".to_string());
    let item_c = ItemViewModel::new();
    item_c.set_name("C".to_string());

    let vm = DynamicForViewModel::new();
    vm.items_push(item_a.clone());
    vm.items_push(item_b.clone());
    vm.items_push(item_c.clone());

    let host = elwindui::new!(DynamicForHost(vm: vm.clone()));
    assert!(host.apply_template());
    for child in elwindui::core::visual_tree::find_all::<DynamicForChild>(host.as_ref()) {
        let child = child
            .as_any()
            .downcast_ref::<DynamicForChild>()
            .expect("dynamic for child has its concrete type");
        assert!(child.apply_template());
    }

    assert_eq!(get_unmount_events().len(), 0);

    // Remove C then B
    vm.items_remove(2);
    vm.items_remove(1);

    let events = get_unmount_events();
    assert!(
        events.contains(&"DynamicForChild:B"),
        "Removed item B must be unmounted: {events:?}"
    );
    assert!(
        events.contains(&"DynamicForChild:C"),
        "Removed item C must be unmounted: {events:?}"
    );
    assert!(
        !events.contains(&"DynamicForChild:A"),
        "Retained item A must not be unmounted: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// 8. Dynamic match removal test
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicMatchTab {
    First,
    Second,
}

#[elwindui::viewmodel]
mod dynamic_match_view_model {
    use super::DynamicMatchTab;
    struct DynamicMatchViewModel {
        #[observable(default = DynamicMatchTab::First)]
        tab: DynamicMatchTab,
    }
}

#[elwindui::component(inherits ContentControl)]
struct MatchFirstChild {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("MatchFirstChild");
        }
        TextBlock { text: "first tab" }
    }),
}

#[elwindui::component]
impl MatchFirstChild {}

#[elwindui::component(inherits ContentControl)]
struct MatchSecondChild {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("MatchSecondChild");
        }
        TextBlock { text: "second tab" }
    }),
}

#[elwindui::component]
impl MatchSecondChild {}

#[elwindui::component(inherits ContentControl)]
struct DynamicMatchHost {
    #[bindable]
    vm: std::rc::Rc<DynamicMatchViewModel>,

    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            match vm.tab {
                DynamicMatchTab::First => {
                    MatchFirstChild { }
                }
                DynamicMatchTab::Second => {
                    MatchSecondChild { }
                }
            }
        }
    }),
}

#[elwindui::component]
impl DynamicMatchHost {}

#[test]
fn test_dynamic_match_branch_removal_triggers_unmount() {
    clear_unmount_events();

    let vm = DynamicMatchViewModel::new();
    let host = elwindui::new!(DynamicMatchHost(vm: vm.clone()));
    assert!(host.apply_template());
    let child = elwindui::core::visual_tree::find_all::<MatchFirstChild>(host.as_ref())
        .into_iter()
        .next()
        .expect("active match child");
    let child = child
        .as_any()
        .downcast_ref::<MatchFirstChild>()
        .expect("match child has its concrete type");
    assert!(child.apply_template());

    assert_eq!(get_unmount_events().len(), 0);

    // Switch branch from First to Second
    vm.set_tab(DynamicMatchTab::Second);

    assert_eq!(
        get_unmount_events(),
        vec!["MatchFirstChild"],
        "Old match branch component must be unmounted when switching branches"
    );
}

// ---------------------------------------------------------------------------
// 9. Teardown ordering: tree connection is intact during on_unmount
// ---------------------------------------------------------------------------

thread_local! {
    static HAD_PARENT_DURING_UNMOUNT: Cell<bool> = const { Cell::new(false) };
}

#[elwindui::component(inherits ContentControl)]
struct TreeConnectionChild {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            use elwindui::core::ui::UIElementExt;
            if self.visual_parent().is_some() || self.parent().is_some() {
                HAD_PARENT_DURING_UNMOUNT.with(|c| c.set(true));
            }
        }
        TextBlock { text: "tree check" }
    }),
}

#[elwindui::component]
impl TreeConnectionChild {}

#[elwindui::component(inherits ContentControl)]
struct TreeConnectionParent {
    template: template_view!(|templated_parent: Self| { TreeConnectionChild {} }),
}

#[elwindui::component]
impl TreeConnectionParent {}

#[test]
fn test_on_unmount_runs_before_visual_and_logical_detach() {
    HAD_PARENT_DURING_UNMOUNT.with(|c| c.set(false));

    let parent = TreeConnectionParent::new();
    assert!(parent.apply_template());
    let child = elwindui::core::visual_tree::find_all::<TreeConnectionChild>(parent.as_ref())
        .into_iter()
        .next()
        .expect("tree connection child");
    let child = child
        .as_any()
        .downcast_ref::<TreeConnectionChild>()
        .expect("tree connection child has its concrete type");
    assert!(child.apply_template());
    assert!(!HAD_PARENT_DURING_UNMOUNT.with(|c| c.get()));

    parent.unmount();

    assert!(
        HAD_PARENT_DURING_UNMOUNT.with(|c| c.get()),
        "on_unmount must execute before parent/visual tree connection is detached"
    );
}

// ---------------------------------------------------------------------------
// 10. Ancestor reentrant unmount safety and child-first ordering
// ---------------------------------------------------------------------------

thread_local! {
    static ANCESTOR_PARENT_REF: RefCell<Option<Rc<AncestorReentrantParent>>> = const { RefCell::new(None) };
}

#[elwindui::component(inherits ContentControl)]
struct AncestorReentrantChild {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("Child:start");
            if let Some(parent) = ANCESTOR_PARENT_REF.with(|r| r.borrow().clone()) {
                parent.unmount();
            }
            record_unmount("Child:end");
        }
        TextBlock { text: "child" }
    }),
}

#[elwindui::component]
impl AncestorReentrantChild {}

#[elwindui::component(inherits ContentControl)]
struct AncestorReentrantParent {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("Parent");
        }
        AncestorReentrantChild { }
    }),
}

#[elwindui::component]
impl AncestorReentrantParent {}

#[test]
fn test_ancestor_reentrant_unmount_is_safe_and_preserves_child_first_order() {
    clear_unmount_events();

    let parent = AncestorReentrantParent::new();
    assert!(parent.apply_template());
    let child = elwindui::core::visual_tree::find_all::<AncestorReentrantChild>(parent.as_ref())
        .into_iter()
        .next()
        .expect("ancestor reentrant child");
    let child = child
        .as_any()
        .downcast_ref::<AncestorReentrantChild>()
        .expect("ancestor reentrant child has its concrete type");
    assert!(child.apply_template());
    ANCESTOR_PARENT_REF.with(|r| *r.borrow_mut() = Some(parent.clone()));

    assert_eq!(get_unmount_events().len(), 0);

    parent.unmount();

    assert_eq!(
        get_unmount_events(),
        vec!["Child:start", "Child:end", "Parent"],
        "Child-first ordering must be strictly preserved under ancestor reentrant unmount"
    );

    // Double unmount is a safe no-op
    parent.unmount();
    assert_eq!(
        get_unmount_events(),
        vec!["Child:start", "Child:end", "Parent"]
    );
}

// ---------------------------------------------------------------------------
// 11. Unmount on Created Component (before mount) must not run on_unmount
// ---------------------------------------------------------------------------

thread_local! {
    static CREATED_UNMOUNT_COUNT: Cell<u32> = const { Cell::new(0) };
    static CREATED_MOUNT_COUNT: Cell<u32> = const { Cell::new(0) };
}

#[elwindui::component(inherits ContentControl)]
struct UnmountedBeforeMountComponent {
    template: template_view!(|templated_parent: Self| {
        on_mount {
            CREATED_MOUNT_COUNT.with(|c| c.set(c.get() + 1));
        }
        on_unmount {
            CREATED_UNMOUNT_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "unmounted before mount" }
    }),
}

#[elwindui::component]
impl UnmountedBeforeMountComponent {}

#[test]
fn test_unmount_on_created_component_does_not_trigger_on_unmount() {
    CREATED_MOUNT_COUNT.with(|c| c.set(0));
    CREATED_UNMOUNT_COUNT.with(|c| c.set(0));

    let component = UnmountedBeforeMountComponent::__new_unmounted();
    assert_eq!(CREATED_MOUNT_COUNT.with(|c| c.get()), 0);
    assert_eq!(CREATED_UNMOUNT_COUNT.with(|c| c.get()), 0);

    // Unmount before mount: Created -> Unmounted
    component.unmount();
    assert_eq!(CREATED_MOUNT_COUNT.with(|c| c.get()), 0);
    assert_eq!(
        CREATED_UNMOUNT_COUNT.with(|c| c.get()),
        0,
        "on_unmount must not run for a component that was never mounted"
    );

    // Repeated unmount on Unmounted state is a no-op
    component.unmount();
    assert_eq!(CREATED_UNMOUNT_COUNT.with(|c| c.get()), 0);
}

// ---------------------------------------------------------------------------
// 12. Intermediate ancestor reentrant unmount from subtree traversal
// ---------------------------------------------------------------------------

thread_local! {
    static INTERMEDIATE_PARENT_REF: RefCell<Option<Rc<IntermediateParentComponent>>> = const { RefCell::new(None) };
}

#[elwindui::component(inherits ContentControl)]
struct IntermediateChildComponent {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("Child:start");
            if let Some(parent) = INTERMEDIATE_PARENT_REF.with(|r| r.borrow().clone()) {
                parent.unmount();
            }
            record_unmount("Child:end");
        }
        TextBlock { text: "child" }
    }),
}

#[elwindui::component]
impl IntermediateChildComponent {}

#[elwindui::component(inherits ContentControl)]
struct IntermediateParentComponent {
    template: template_view!(|templated_parent: Self| {
        on_mount {
            INTERMEDIATE_PARENT_REF.with(|r| *r.borrow_mut() = Some(this.clone()));
        }
        on_unmount {
            record_unmount("Parent");
        }
        IntermediateChildComponent { }
    }),
}

#[elwindui::component]
impl IntermediateParentComponent {}

#[elwindui::component(inherits ContentControl)]
struct IntermediateGrandParentComponent {
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            record_unmount("GrandParent");
        }
        IntermediateParentComponent { }
    }),
}

#[elwindui::component]
impl IntermediateGrandParentComponent {}

#[test]
fn test_intermediate_ancestor_reentry_from_subtree_traversal_is_safe() {
    clear_unmount_events();
    INTERMEDIATE_PARENT_REF.with(|r| *r.borrow_mut() = None);

    let grand_parent = IntermediateGrandParentComponent::new();
    assert!(grand_parent.apply_template());
    let parent =
        elwindui::core::visual_tree::find_all::<IntermediateParentComponent>(grand_parent.as_ref())
            .into_iter()
            .next()
            .expect("intermediate parent");
    let parent = parent
        .as_any()
        .downcast_ref::<IntermediateParentComponent>()
        .expect("intermediate parent has its concrete type");
    assert!(parent.apply_template());
    let child = elwindui::core::visual_tree::find_all::<IntermediateChildComponent>(parent)
        .into_iter()
        .next()
        .expect("intermediate child");
    let child = child
        .as_any()
        .downcast_ref::<IntermediateChildComponent>()
        .expect("intermediate child has its concrete type");
    assert!(child.apply_template());
    assert!(INTERMEDIATE_PARENT_REF.with(|r| r.borrow().is_some()));
    assert_eq!(get_unmount_events().len(), 0);

    grand_parent.unmount();

    assert_eq!(
        get_unmount_events(),
        vec!["Child:start", "Child:end", "Parent", "GrandParent"],
        "Intermediate parent must be marked Unmounting during subtree descent so child reentrant unmount is a safe no-op"
    );
}
