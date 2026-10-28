//! Runtime proof for Issue #122: `if` / `match` / `for` written as the *direct* children of a
//! shape-composition (`inherits VerticalLayout`) component's own `body: view!` — never wrapped in an
//! explicit stored `VerticalLayout { .. }` — must compile and behave like the same dynamic regions
//! one level down.
//!
//! The composition root has no stored field of its own (docs/design/runtime/
//! component_lifecycle_design.md §4b: its only instance is `Self.base`), so each dynamic region's
//! content host must resolve to `self` itself rather than a non-existent `self.__verticallayout_N`
//! field (the original E0609). Every test below observes the composition root's *own*
//! `visual_children()`: their exact text/order proves that each region's content was applied to
//! `Self.base` at the correct offset, with no extra synthesized wrapper root in between.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::Cell;
use std::rc::{Rc, Weak};

use elwindui::core::ui::{TextBlock, UIElementExt};

/// Text of each direct visual child of `root`, in order. A child that is not a `TextBlock` is
/// reported by its mount-counting leaf name so the assertion still pins its exact position.
fn child_labels(root: &dyn UIElementExt) -> Vec<String> {
    root.visual_children()
        .iter()
        .map(|child| {
            if let Some(text) = child.as_any().downcast_ref::<TextBlock>() {
                text.text.borrow().clone()
            } else if child.as_any().downcast_ref::<ThenLeaf>().is_some() {
                "<then-leaf>".to_string()
            } else if child.as_any().downcast_ref::<ElseLeaf>().is_some() {
                "<else-leaf>".to_string()
            } else {
                "<other>".to_string()
            }
        })
        .collect()
}

fn labels(expected: &[&str]) -> Vec<String> {
    expected.iter().map(|label| label.to_string()).collect()
}

// ---------------------------------------------------------------------------
// T1. The Issue's original reproduction: root-direct `if` over a `#[param]`.
// ---------------------------------------------------------------------------

#[elwindui::component(inherits VerticalLayout)]
struct ShapeRootIfProbe {
    #[param]
    show: bool,

    body: view! {
        if show {
            TextBlock { text: "a" }
        } else {
            TextBlock { text: "b" }
        }
    },
}

#[elwindui::component]
impl ShapeRootIfProbe {}

#[test]
fn t1_root_direct_if_over_param_selects_the_matching_branch() {
    let shown = elwindui::new!(ShapeRootIfProbe(show: true));
    assert_eq!(child_labels(shown.as_ref()), labels(&["a"]));

    let hidden = elwindui::new!(ShapeRootIfProbe(show: false));
    assert_eq!(child_labels(hidden.as_ref()), labels(&["b"]));
}

// ---------------------------------------------------------------------------
// T2. Root-direct `if` over a mutable `#[prop]`, with mount-counting leaves (lazy-once).
// ---------------------------------------------------------------------------

thread_local! {
    static THEN_MOUNTS: Cell<u32> = const { Cell::new(0) };
    static ELSE_MOUNTS: Cell<u32> = const { Cell::new(0) };
}

#[elwindui::component(inherits VerticalLayout)]
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

#[elwindui::component(inherits VerticalLayout)]
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

#[elwindui::component(inherits VerticalLayout)]
struct ShapeRootToggle {
    #[prop(default = true)]
    show_then: bool,

    body: view! {
        if show_then {
            ThenLeaf {}
        } else {
            ElseLeaf {}
        }
    },
}

#[elwindui::component]
impl ShapeRootToggle {}

fn mounts() -> (u32, u32) {
    (THEN_MOUNTS.with(|c| c.get()), ELSE_MOUNTS.with(|c| c.get()))
}

#[test]
fn t2_root_direct_if_over_prop_switches_and_keeps_lazy_once() {
    THEN_MOUNTS.with(|c| c.set(0));
    ELSE_MOUNTS.with(|c| c.set(0));

    let toggle = ShapeRootToggle::new();
    assert_eq!(child_labels(toggle.as_ref()), labels(&["<then-leaf>"]));
    assert_eq!(
        mounts(),
        (1, 0),
        "only the active branch is built at startup"
    );
    let first_then = Rc::clone(&toggle.visual_children()[0]);

    toggle.set_show_then(false);
    assert_eq!(child_labels(toggle.as_ref()), labels(&["<else-leaf>"]));
    assert_eq!(
        mounts(),
        (1, 1),
        "the else branch materializes once on first reach"
    );
    assert!(
        first_then.visual_parent().is_none(),
        "the inactive branch is detached"
    );

    toggle.set_show_then(true);
    assert_eq!(child_labels(toggle.as_ref()), labels(&["<then-leaf>"]));
    assert_eq!(mounts(), (1, 1), "switching back reuses the cached branch");
    assert!(
        Rc::ptr_eq(&first_then, &toggle.visual_children()[0]),
        "the cached then-branch instance is re-attached, not rebuilt"
    );

    for _ in 0..4 {
        toggle.set_show_then(!toggle.show_then());
    }
    assert_eq!(child_labels(toggle.as_ref()), labels(&["<then-leaf>"]));
    assert_eq!(mounts(), (1, 1));
}

// ---------------------------------------------------------------------------
// T3. Root-direct exhaustive `match` over an enum `#[prop]`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeRootMode {
    #[default]
    Loading,
    Error,
    Ok,
}

#[elwindui::component(inherits VerticalLayout)]
struct ShapeRootMatch {
    #[prop(default = ShapeRootMode::Loading)]
    mode: ShapeRootMode,

    body: view! {
        match mode {
            ShapeRootMode::Loading => TextBlock { text: "loading" },
            ShapeRootMode::Error => TextBlock { text: "error" },
            ShapeRootMode::Ok => TextBlock { text: "ok" },
        }
    },
}

#[elwindui::component]
impl ShapeRootMatch {}

#[test]
fn t3_root_direct_match_shows_only_the_active_arm() {
    let probe = ShapeRootMatch::new();
    assert_eq!(child_labels(probe.as_ref()), labels(&["loading"]));

    for (mode, expected) in [
        (ShapeRootMode::Error, "error"),
        (ShapeRootMode::Ok, "ok"),
        (ShapeRootMode::Loading, "loading"),
        (ShapeRootMode::Ok, "ok"),
    ] {
        probe.set_mode(mode);
        assert_eq!(
            child_labels(probe.as_ref()),
            labels(&[expected]),
            "after switching to {mode:?} only that arm's child may remain"
        );
    }
}

// ---------------------------------------------------------------------------
// T4 / T5 / T8. Root-direct `for` over `Vec<Rc<Item>>`.
// ---------------------------------------------------------------------------

#[elwindui::viewmodel]
mod shape_root_row {
    struct ShapeRootRow {
        #[observable(default = String::new())]
        label: String,
    }
}

fn row(label: &str) -> Rc<ShapeRootRow> {
    let row = ShapeRootRow::new();
    row.set_label(label.to_string());
    row
}

#[elwindui::component(inherits VerticalLayout)]
struct ShapeRootFor {
    #[prop(default = Vec::new())]
    rows: Vec<Rc<ShapeRootRow>>,

    body: view! {
        for row in rows {
            TextBlock { text: row.label }
        }
    },
}

#[elwindui::component]
impl ShapeRootFor {}

#[test]
fn t4_root_direct_for_tracks_length_and_order() {
    let probe = ShapeRootFor::new();
    assert_eq!(child_labels(probe.as_ref()), Vec::<String>::new());

    let (a, b, c, d) = (row("a"), row("b"), row("c"), row("d"));
    probe.set_rows(vec![a.clone(), b.clone(), c.clone()]);
    assert_eq!(child_labels(probe.as_ref()), labels(&["a", "b", "c"]));

    probe.set_rows(vec![a.clone(), b.clone(), c.clone(), d.clone()]);
    assert_eq!(child_labels(probe.as_ref()), labels(&["a", "b", "c", "d"]));

    probe.set_rows(vec![a.clone(), c.clone(), d.clone()]);
    assert_eq!(child_labels(probe.as_ref()), labels(&["a", "c", "d"]));

    probe.set_rows(vec![d.clone(), a.clone(), c.clone()]);
    assert_eq!(child_labels(probe.as_ref()), labels(&["d", "a", "c"]));

    probe.set_rows(Vec::new());
    assert_eq!(child_labels(probe.as_ref()), Vec::<String>::new());
}

#[test]
fn t5_root_direct_for_keeps_rc_identity_and_releases_removed_items() {
    let probe = ShapeRootFor::new();
    let (kept, removed) = (row("kept"), row("removed"));
    probe.set_rows(vec![kept.clone(), removed.clone()]);
    let kept_widget = Rc::clone(&probe.visual_children()[0]);
    let removed_widget: Weak<dyn UIElementExt> = Rc::downgrade(&probe.visual_children()[1]);

    // An item-property change updates the existing widget in place.
    kept.set_label("kept-updated".to_string());
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["kept-updated", "removed"])
    );

    let fresh = row("fresh");
    probe.set_rows(vec![fresh.clone(), kept.clone()]);
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["fresh", "kept-updated"])
    );
    assert!(
        Rc::ptr_eq(&kept_widget, &probe.visual_children()[1]),
        "a retained Rc item keeps its widget instance across reconciliation"
    );
    assert!(
        removed_widget.upgrade().is_none(),
        "a removed item's widget is dropped"
    );
    assert_eq!(
        Rc::strong_count(&removed),
        1,
        "a removed item is no longer retained by the dynamic slot or its subscription"
    );

    // The removed item's subscription is gone: mutating it must not resurrect a child.
    removed.set_label("ignored".to_string());
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["fresh", "kept-updated"])
    );
}

#[test]
fn t8_repeated_updates_then_owner_drop_release_every_subscription() {
    let probe = ShapeRootFor::new();
    let items: Vec<_> = (0..4).map(|i| row(&format!("r{i}"))).collect();
    for round in 0..32 {
        let mut current = items.clone();
        current.rotate_left(round % items.len());
        current.truncate(1 + round % items.len());
        probe.set_rows(current.clone());
        items[round % items.len()].set_label(format!("r{}-{round}", round % items.len()));
        let expected: Vec<String> = current.iter().map(|item| item.label()).collect();
        assert_eq!(child_labels(probe.as_ref()), expected, "round {round}");
    }

    let weak_probe = Rc::downgrade(&probe);
    drop(probe);
    assert!(weak_probe.upgrade().is_none(), "the owner is fully dropped");
    for item in &items {
        assert_eq!(
            Rc::strong_count(item),
            1,
            "no slot or subscription outlives the owner"
        );
        // No stale callback may fire into the dropped owner.
        item.set_label("after-drop".to_string());
    }
}

// ---------------------------------------------------------------------------
// T6. Static children interleaved with root-direct dynamic regions, plus a nested dynamic region.
// ---------------------------------------------------------------------------

#[elwindui::component(inherits VerticalLayout)]
struct ShapeRootMixed {
    #[prop(default = true)]
    show: bool,
    #[prop(default = ShapeRootMode::Loading)]
    mode: ShapeRootMode,
    #[prop(default = Vec::new())]
    rows: Vec<Rc<ShapeRootRow>>,

    body: view! {
        TextBlock { text: "head" }
        if show {
            match mode {
                ShapeRootMode::Loading => TextBlock { text: "loading" },
                ShapeRootMode::Error => TextBlock { text: "error" },
                ShapeRootMode::Ok => TextBlock { text: "ok" },
            }
        } else {
            TextBlock { text: "hidden" }
        }
        TextBlock { text: "middle" }
        for row in rows {
            TextBlock { text: row.label }
        }
        TextBlock { text: "tail" }
    },
}

#[elwindui::component]
impl ShapeRootMixed {}

#[test]
fn t6_static_and_root_direct_dynamic_siblings_keep_their_offsets() {
    let probe = ShapeRootMixed::new();
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["head", "loading", "middle", "tail"])
    );

    probe.set_rows(vec![row("x"), row("y")]);
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["head", "loading", "middle", "x", "y", "tail"])
    );

    probe.set_mode(ShapeRootMode::Error);
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["head", "error", "middle", "x", "y", "tail"])
    );

    probe.set_show(false);
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["head", "hidden", "middle", "x", "y", "tail"])
    );

    probe.set_mode(ShapeRootMode::Ok);
    probe.set_show(true);
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["head", "ok", "middle", "x", "y", "tail"])
    );

    probe.set_rows(Vec::new());
    assert_eq!(
        child_labels(probe.as_ref()),
        labels(&["head", "ok", "middle", "tail"])
    );
}

// ---------------------------------------------------------------------------
// T7. The same `for` reconciliation one level down, under an explicit stored wrapper, so the
// DynamicChildSlot / item-observer fixes are proven independent of the composition root.
// ---------------------------------------------------------------------------

#[elwindui::component(inherits VerticalLayout)]
struct WrappedFor {
    #[prop(default = Vec::new())]
    rows: Vec<Rc<ShapeRootRow>>,

    body: view! {
        TextBlock { text: "head" }
        VerticalLayout {
            for row in rows {
                TextBlock { text: row.label }
            }
        }
    },
}

#[elwindui::component]
impl WrappedFor {}

#[test]
fn t7_wrapped_for_matches_the_root_direct_form() {
    let probe = WrappedFor::new();
    let inner = || Rc::clone(&probe.visual_children()[1]);
    let (a, b, c) = (row("a"), row("b"), row("c"));
    probe.set_rows(vec![a.clone(), b.clone(), c.clone()]);
    assert_eq!(child_labels(inner().as_ref()), labels(&["a", "b", "c"]));

    probe.set_rows(vec![c.clone(), a.clone()]);
    assert_eq!(child_labels(inner().as_ref()), labels(&["c", "a"]));

    a.set_label("a2".to_string());
    assert_eq!(child_labels(inner().as_ref()), labels(&["c", "a2"]));
    assert_eq!(child_labels(probe.as_ref()), labels(&["head", "<other>"]));
    assert_eq!(Rc::strong_count(&b), 1);
}
