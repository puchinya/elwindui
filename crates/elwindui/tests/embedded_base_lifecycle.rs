//! Issue #166 / PR #311 (A/P1-1, A/P1-2): a generated base embedded as a derived Component's
//! `base` value keeps its own lifecycle, subscriptions and wiring, and nothing the generated view
//! stores keeps the outermost Component alive.
//!
//! - Ownership: event handlers, `for` item handlers and subscriptions capture only a weak owner,
//!   so the Component is released when its last strong `Rc` goes away, with or without an
//!   explicit `unmount_subtree` first, standalone or embedded.
//! - Lifecycle: an embedded base runs its own `on_mount` / `on_update` / `on_unmount` exactly once
//!   each — `on_mount` from the base up, `on_unmount` from the most-derived level down.
//! - Subscriptions: the base's `#[bindable]` viewmodel keeps updating its own view while embedded,
//!   and every subscription is released on unmount. (A required `#[bindable]` base field reaches a
//!   derived constructor only through a view-less derived Component, which forwards the base's
//!   constructor inputs; a derived Component with its own view cannot supply it.)

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use elwindui::core::ui::UIElementExt;

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(entry: impl Into<String>) {
    LOG.with(|log| log.borrow_mut().push(entry.into()));
}

fn take_log() -> Vec<String> {
    LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn text_of(block: &elwindui::ui::TextBlock) -> String {
    block.text.borrow().clone()
}

fn tap(target: Rc<dyn UIElementExt>) {
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &target,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: elwindui::core::base::Point { x: 1.0, y: 1.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );
}

fn child_texts(root: &dyn UIElementExt) -> Vec<String> {
    root.visual_children()
        .iter()
        .filter_map(|child| {
            child
                .as_any()
                .downcast_ref::<elwindui::ui::TextBlock>()
                .map(text_of)
        })
        .collect()
}

#[elwindui::viewmodel]
mod lifecycle_view_model {
    struct LifecycleLabels {
        #[observable(default = "vm-a".to_string())]
        label: String,
    }
}

#[elwindui::viewmodel]
mod lifecycle_row {
    struct LifecycleRow {
        #[observable(default = String::new())]
        label: String,
    }
}

fn row(label: &str) -> Rc<LifecycleRow> {
    let row = LifecycleRow::new();
    row.set_label(label.to_string());
    row
}

// ---------------------------------------------------------------------------
// Fixtures: a base with every kind of stored callback and every lifecycle hook, embedded once
// (`LifecycleDerived`) and twice (`LifecycleLeaf`).
// ---------------------------------------------------------------------------

#[elwindui::component(inherits VerticalLayout)]
struct LifecycleBase {
    #[prop(default = 0)]
    count: i32,
    #[prop(default = Vec::new())]
    rows: Vec<Rc<LifecycleRow>>,

    body: view! {
        on_mount {
            log("base mount");
        }
        on_update(count): {
            log(format!("base update {count}"));
        }
        on_unmount {
            log("base unmount");
        }

        #[id("base_count")]
        let base_count = TextBlock {
            text: format!("count {count}"),
            on_tapped: |_event| {
                count += 1;
            },
        };
        base_count
        for item in rows {
            TextBlock {
                text: item.label,
                on_tapped: |_event| {
                    count += 10;
                },
            }
        }
    },
}

#[elwindui::component]
impl LifecycleBase {}

#[elwindui::component(inherits crate::LifecycleBase)]
struct LifecycleDerived {
    #[prop(default = 0)]
    derived_taps: i32,

    body: view! {
        on_mount {
            log("derived mount");
        }
        on_update(derived_taps): {
            log(format!("derived update {derived_taps}"));
        }
        on_unmount {
            log("derived unmount");
        }

        #[id("derived_text")]
        let derived_text = TextBlock {
            text: format!("derived {derived_taps}/{count}"),
            on_tapped: |_event| {
                derived_taps += 1;
            },
        };
        derived_text
    },
}

#[elwindui::component]
impl LifecycleDerived {}

#[elwindui::component(inherits crate::LifecycleDerived)]
struct LifecycleLeaf {
    body: view! {
        on_mount {
            log("leaf mount");
        }
        on_unmount {
            log("leaf unmount");
        }

        TextBlock { text: "leaf" }
    },
}

#[elwindui::component]
impl LifecycleLeaf {}

// A base with a required `#[bindable]` viewmodel, embedded through a view-less derived level.
#[elwindui::component(inherits VerticalLayout)]
struct BindableBase {
    #[bindable]
    vm: Rc<LifecycleLabels>,

    body: view! {
        on_mount {
            log("bindable mount");
        }
        on_unmount {
            log("bindable unmount");
        }

        #[id("vm_text")]
        let vm_text = TextBlock { text: vm.label };
        vm_text
    },
}

#[elwindui::component]
impl BindableBase {}

#[elwindui::component(inherits crate::BindableBase)]
struct ViewlessBindableDerived {}

#[elwindui::component]
impl ViewlessBindableDerived {}

// ---------------------------------------------------------------------------
// A/P1-1: no strong owner cycle.
// ---------------------------------------------------------------------------

#[test]
fn standalone_component_with_handlers_is_released_on_drop() {
    let base = LifecycleBase::new();
    let item = row("r1");
    base.set_rows(vec![item.clone()]);
    tap(base.base_count());
    tap(Rc::clone(&base.visual_children()[1]));
    assert_eq!(base.count(), 11);

    let weak: Weak<LifecycleBase> = Rc::downgrade(&base);
    drop(base);
    assert!(weak.upgrade().is_none(), "the standalone owner is released");
    assert_eq!(
        Rc::strong_count(&item),
        1,
        "no `for` slot outlives the owner"
    );
}

#[test]
fn embedded_base_with_handlers_releases_the_owner_on_drop() {
    let derived = LifecycleDerived::new();
    let item = row("r1");
    derived.base.set_rows(vec![item.clone()]);
    let base_count = derived.base.base_count();
    let derived_text = derived.derived_text();
    let item_widget = Rc::clone(&derived.visual_children()[1]);
    tap(base_count.clone());
    tap(derived_text.clone());
    tap(Rc::clone(&item_widget));
    assert_eq!(derived.base.count(), 11);
    assert_eq!(derived.derived_taps(), 1);

    let weak: Weak<LifecycleDerived> = Rc::downgrade(&derived);
    drop(derived);
    assert!(
        weak.upgrade().is_none(),
        "neither the base's nor the derived level's stored callbacks keep the owner"
    );
    assert_eq!(
        Rc::strong_count(&item),
        1,
        "no `for` slot outlives the owner"
    );

    // Widgets kept alive past their owner: their handlers find no owner and do nothing.
    tap(base_count);
    tap(derived_text);
    tap(item_widget);
}

#[test]
fn embedded_base_is_released_after_explicit_unmount() {
    let leaf = LifecycleLeaf::new();
    leaf.base.base.set_rows(vec![row("r1")]);
    let base_count = leaf.base.base.base_count();
    let root: Rc<dyn UIElementExt> = leaf.clone();
    take_log();

    elwindui::core::ui::unmount_subtree(&root);
    drop(root);
    assert_eq!(
        take_log(),
        strings(&["leaf unmount", "derived unmount", "base unmount"]),
        "unmount runs from the most-derived level down, once each"
    );

    // Unmounted: no hook runs again and no view re-syncs.
    leaf.base.base.set_count(5);
    assert_eq!(take_log(), Vec::<String>::new());
    assert_eq!(text_of(&leaf.base.base.base_count()), "count 0");

    let weak: Weak<LifecycleLeaf> = Rc::downgrade(&leaf);
    drop(leaf);
    assert!(weak.upgrade().is_none(), "the unmounted owner is released");
    tap(base_count);
}

// ---------------------------------------------------------------------------
// A/P1-2: lifecycle hooks and subscriptions of an embedded base.
// ---------------------------------------------------------------------------

#[test]
fn embedded_base_runs_its_own_hooks_in_order_once_each() {
    take_log();
    let derived = LifecycleDerived::new();
    assert_eq!(
        take_log(),
        strings(&["base mount", "derived mount"]),
        "on_mount runs from the base up, once each"
    );

    // The base's `on_update(count)` fires for a write from the base's own handler and from
    // outside; the derived `on_update` only for its own field.
    tap(derived.base.base_count());
    derived.base.set_count(7);
    tap(derived.derived_text());
    assert_eq!(
        take_log(),
        strings(&["base update 1", "base update 7", "derived update 1"])
    );
    assert_eq!(text_of(&derived.derived_text()), "derived 1/7");
    assert_eq!(text_of(&derived.base.base_count()), "count 7");

    derived.unmount();
    assert_eq!(take_log(), strings(&["derived unmount", "base unmount"]));
    derived.unmount();
    assert_eq!(take_log(), Vec::<String>::new(), "unmount is idempotent");
}

#[test]
fn twice_embedded_base_runs_every_level_once() {
    take_log();
    let leaf = LifecycleLeaf::new();
    assert_eq!(
        take_log(),
        strings(&["base mount", "derived mount", "leaf mount"])
    );
    tap(leaf.base.base.base_count());
    assert_eq!(take_log(), strings(&["base update 1"]));
    leaf.unmount();
    assert_eq!(
        take_log(),
        strings(&["leaf unmount", "derived unmount", "base unmount"])
    );
}

#[test]
fn embedded_base_for_item_handlers_reach_the_base() {
    let derived = LifecycleDerived::new();
    let (a, b) = (row("a"), row("b"));
    derived.base.set_rows(vec![a.clone(), b.clone()]);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["count 0", "a", "b", "derived 0/0"])
    );
    tap(Rc::clone(&derived.visual_children()[2]));
    assert_eq!(derived.base.count(), 10);
    assert_eq!(text_of(&derived.derived_text()), "derived 0/10");
}

#[test]
fn embedded_base_bindable_viewmodel_updates_its_own_view_until_unmount() {
    take_log();
    let vm = LifecycleLabels::new();
    let derived = elwindui::new!(ViewlessBindableDerived(vm: Rc::clone(&vm)));
    assert_eq!(
        take_log(),
        strings(&["bindable mount"]),
        "the hook runs once, not twice"
    );
    assert_eq!(text_of(&derived.base.vm_text()), "vm-a");
    vm.set_label("vm-b".to_string());
    assert_eq!(text_of(&derived.base.vm_text()), "vm-b");

    derived.unmount();
    assert_eq!(take_log(), strings(&["bindable unmount"]));
    vm.set_label("vm-c".to_string());
    assert_eq!(
        text_of(&derived.base.vm_text()),
        "vm-b",
        "no update after unmount"
    );

    let weak: Weak<ViewlessBindableDerived> = Rc::downgrade(&derived);
    drop(derived);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        Rc::strong_count(&vm),
        1,
        "no subscription keeps the viewmodel"
    );
}

#[test]
fn embedded_bindable_owner_is_released_without_unmount() {
    let vm = LifecycleLabels::new();
    let derived = elwindui::new!(ViewlessBindableDerived(vm: Rc::clone(&vm)));
    let weak: Weak<ViewlessBindableDerived> = Rc::downgrade(&derived);
    drop(derived);
    assert!(weak.upgrade().is_none());
    assert_eq!(Rc::strong_count(&vm), 1);
    vm.set_label("after-drop".to_string());
}
