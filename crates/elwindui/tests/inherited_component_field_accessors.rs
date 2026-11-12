//! Issue #166: a derived Component's own `view!` reads and writes fields declared by a
//! user-defined ancestor through one unambiguous accessor path (Model A — the declaring Component
//! owns storage, accessors and notification), compiled by real rustc.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::Point;
use elwindui::core::reactive::ObservableExt;
use elwindui::core::ui::UIElementExt;
use std::cell::RefCell;
use std::rc::Rc;

#[elwindui::component(inherits VerticalLayout)]
struct InheritBase {
    #[prop(default = "before".to_string())]
    heading: String,
    #[prop(default = 1)]
    count: i32,

    body: view! {
        #[id("base_label")]
        let base_label = TextBlock { text: heading };

        base_label
    },
}

#[elwindui::component]
impl InheritBase {}

/// One hop: reads `heading` from a DSL value position and from raw Rust (`format!`), writes it
/// from an event handler, and compound-assigns the inherited `count`.
#[elwindui::component(inherits crate::InheritBase)]
struct InheritDerived {
    #[prop(default = "own".to_string())]
    own_label: String,

    body: view! {
        #[id("read_label")]
        let read_label = TextBlock {
            text: heading,
            on_tapped: |_event| {
                heading = "after".to_string();
                count += 10;
            },
        };
        #[id("format_label")]
        let format_label = TextBlock { text: format!("{heading}/{}", count) };
        #[id("own_text")]
        let own_text = TextBlock { text: own_label };

        VerticalLayout {
            read_label
            format_label
            own_text
        }
    },
}

#[elwindui::component]
impl InheritDerived {}

#[elwindui::component(inherits crate::InheritBase)]
struct InheritMiddle {
    #[prop(default = 0)]
    middle_only: i32,
}

#[elwindui::component]
impl InheritMiddle {}

/// Multi-hop: `heading` is declared only on `InheritBase`.
#[elwindui::component(inherits crate::InheritMiddle)]
struct InheritLeaf {
    body: view! {
        #[id("leaf_label")]
        let leaf_label = TextBlock {
            text: heading,
            on_tapped: |_event| {
                heading = format!("{heading}!");
            },
        };

        leaf_label
    },
}

#[elwindui::component]
impl InheritLeaf {}

fn text_of(block: &elwindui::ui::TextBlock) -> String {
    block.text.borrow().clone()
}

fn tap(target: Rc<dyn UIElementExt>) {
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &target,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 1.0, y: 1.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );
}

#[test]
fn t1_derived_view_reads_inherited_prop() {
    let derived = InheritDerived::new();
    assert_eq!(derived.heading(), "before");
    assert_eq!(text_of(&derived.read_label()), "before");
    assert_eq!(text_of(&derived.format_label()), "before/1");
}

#[test]
fn t2_derived_handler_writes_inherited_prop_through_declaring_setter() {
    let derived = InheritDerived::new();
    let notified = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let sink = Rc::clone(&notified);
    let _subscription = ObservableExt::subscribe_property_changed(&derived.base, move |property| {
        sink.borrow_mut().push(property);
    });

    tap(derived.read_label());

    assert_eq!(derived.heading(), "after");
    assert_eq!(derived.base.heading(), "after");
    assert_eq!(derived.count(), 11);
    assert_eq!(*notified.borrow(), vec!["heading", "count"]);
    assert_eq!(text_of(&derived.read_label()), "after");
    assert_eq!(text_of(&derived.format_label()), "after/11");
}

#[test]
fn t2_external_inherited_write_resyncs_derived_view() {
    let derived = InheritDerived::new();
    derived.base.set_heading("external".to_string());
    assert_eq!(derived.heading(), "external");
    assert_eq!(text_of(&derived.read_label()), "external");
}

#[test]
fn t3_multi_hop_reads_and_writes_ancestor_prop() {
    let leaf = InheritLeaf::new();
    assert_eq!(text_of(&leaf.leaf_label()), "before");
    tap(leaf.leaf_label());
    assert_eq!(leaf.heading(), "before!");
    assert_eq!(leaf.base.base.heading(), "before!");
    assert_eq!(text_of(&leaf.leaf_label()), "before!");

    leaf.base.base.set_heading("from base".to_string());
    assert_eq!(text_of(&leaf.leaf_label()), "from base");
}

#[test]
fn t3_observable_ext_reports_ancestor_properties_on_descendant() {
    let leaf = InheritLeaf::new();
    let names = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let sink = Rc::clone(&names);
    let _subscription =
        ObservableExt::subscribe_property_changed(&*leaf, move |name| sink.borrow_mut().push(name));
    leaf.base.base.set_heading("x".to_string());
    leaf.base.set_middle_only(3);
    assert_eq!(*names.borrow(), vec!["heading", "middle_only"]);
}

#[test]
fn t4_derived_own_prop_is_unaffected() {
    let derived = InheritDerived::new();
    assert_eq!(text_of(&derived.own_text()), "own");
    derived.set_own_label("changed".to_string());
    assert_eq!(text_of(&derived.own_text()), "changed");
    assert_eq!(derived.heading(), "before");
}

// T6: DeferredView is a regression consumer of the general fix — a derived Component's
// `context_popup: view!` handler writes a Prop declared by its user-defined ancestor.
#[elwindui::component(inherits crate::InheritBase)]
struct InheritPopupDerived {
    body: view! {
        #[id("popup_target")]
        let popup_target = TextBlock {
            text: heading,
            context_popup: view! {
                TextBlock {
                    text: "popup",
                    on_tapped: |_event| {
                        heading = "from popup".to_string();
                    },
                }
            },
        };

        popup_target
    },
}

#[elwindui::component]
impl InheritPopupDerived {}

struct T6PopupHandle;

impl elwindui::core::ui::popup::PopupSurfaceHandle for T6PopupHandle {
    fn close(&self) {}
}

struct T6PopupHost {
    shown: RefCell<Vec<Rc<dyn UIElementExt>>>,
}

impl elwindui::core::ui::popup::PopupHost for T6PopupHost {
    fn show_popup(
        &self,
        request: elwindui::core::ui::popup::PopupRequest,
    ) -> Option<Rc<dyn elwindui::core::ui::popup::PopupSurfaceHandle>> {
        self.shown.borrow_mut().push(Rc::clone(&request.content));
        Some(Rc::new(T6PopupHandle))
    }
}

#[test]
fn t6_derived_context_popup_writes_inherited_prop() {
    use elwindui::core::base::Rect;
    use elwindui::core::ui::popup::{
        ContextMenuService, ContextRequest, PopupAnchor, ResolvedContextDefinition,
    };

    let derived = InheritPopupDerived::new();
    let target: Rc<dyn UIElementExt> = derived.popup_target();
    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template } = resolved.definition else {
        panic!("expected Popup definition");
    };
    let host = T6PopupHost {
        shown: RefCell::new(Vec::new()),
    };
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };
    let handle = ContextMenuService::open_custom_popup(
        &host,
        &resolved.owner,
        &template,
        &anchor,
        resolved.owner.effective_environment(),
        work_area,
    )
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0]);
    let inner = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());
    tap(inner);

    assert_eq!(derived.heading(), "from popup");
    assert_eq!(text_of(&derived.popup_target()), "from popup");

    elwindui::ui::unmount_subtree(&content);
    handle.close();
}

/// Per-property inherited resync: changing an unrelated inherited property must not re-apply an
/// attribute that depends only on `heading`. A sentinel written straight into the widget survives
/// a `count` change only if the `heading` attribute was not re-evaluated.
#[elwindui::component(inherits crate::InheritBase)]
struct InheritSelectiveDerived {
    body: view! {
        #[id("heading_view")]
        let heading_view = TextBlock { text: heading };
        #[id("count_view")]
        let count_view = TextBlock { text: format!("{count}") };

        VerticalLayout {
            heading_view
            count_view
        }
    },
}

#[elwindui::component]
impl InheritSelectiveDerived {}

#[test]
fn inherited_change_resyncs_only_dependent_attributes() {
    let derived = InheritSelectiveDerived::new();
    *derived.heading_view().text.borrow_mut() = "sentinel".to_string();

    derived.base.set_count(42);
    assert_eq!(text_of(&derived.count_view()), "42");
    assert_eq!(
        text_of(&derived.heading_view()),
        "sentinel",
        "a `count` change must not re-apply the `heading`-only attribute"
    );

    derived.base.set_heading("selective".to_string());
    assert_eq!(text_of(&derived.heading_view()), "selective");
}

/// A value type that supports only compound assignment (`AddAssign<i32>`) — no `Add`, no `Copy`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meters(pub i32);

impl std::fmt::Display for Meters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::AddAssign<i32> for Meters {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs;
    }
}

#[elwindui::component(inherits VerticalLayout)]
struct OpAssignBase {
    #[prop(default = crate::Meters(1))]
    distance: crate::Meters,

    body: view! {
        TextBlock { text: format!("{distance}") }
    },
}

#[elwindui::component]
impl OpAssignBase {}

/// Receives a `Meters` through a DSL value position, which makes `distance` a DSL-known inherited
/// field of the derived view (the direct `base.distance()`/`base.set_distance(..)` route).
#[elwindui::component(inherits VerticalLayout)]
struct MetersView {
    #[prop(default = crate::Meters(0))]
    value: crate::Meters,

    body: view! {
        TextBlock { text: format!("{value}") }
    },
}

#[elwindui::component]
impl MetersView {}

/// `distance += 2` on an inherited Prop whose type implements only `AddAssign<i32>` — through the
/// direct DSL-known route (`OpAssignDerived`, `MetersView { value: distance }`) and through the
/// shape-macro route (`OpAssignRawDerived`, raw-Rust-only reference).
#[elwindui::component(inherits crate::OpAssignBase)]
struct OpAssignDerived {
    body: view! {
        #[id("op_label")]
        let op_label = TextBlock {
            text: format!("{distance}"),
            on_tapped: |_event| {
                distance += 2;
            },
        };
        #[id("meters_view")]
        let meters_view = crate::MetersView { value: distance };

        VerticalLayout {
            op_label
            meters_view
        }
    },
}

#[elwindui::component]
impl OpAssignDerived {}

#[elwindui::component(inherits crate::OpAssignBase)]
struct OpAssignRawDerived {
    body: view! {
        #[id("raw_op_label")]
        let raw_op_label = TextBlock {
            text: format!("{distance}"),
            on_tapped: |_event| {
                distance += 2;
            },
        };

        raw_op_label
    },
}

#[elwindui::component]
impl OpAssignRawDerived {}

#[test]
fn compound_assignment_on_inherited_prop_keeps_add_assign_requirement() {
    let derived = OpAssignDerived::new();
    tap(derived.op_label());
    assert_eq!(derived.base.distance(), Meters(3));
    assert_eq!(text_of(&derived.op_label()), "3");
    assert_eq!(derived.meters_view().value(), Meters(3));

    let raw = OpAssignRawDerived::new();
    tap(raw.raw_op_label());
    assert_eq!(raw.base.distance(), Meters(3));
    assert_eq!(text_of(&raw.raw_op_label()), "3");
}

#[test]
fn compound_assignment_fallback_preserves_original_operator_for_undeclared_names() {
    // A name no ancestor declares expands back to the authored `name op= value` verbatim, so a
    // local whose type implements only `AddAssign<i32>` still compiles and mutates in place.
    let base = OpAssignBase::new();
    let mut walked = Meters(10);
    crate::__elwindui_props_OpAssignBase!(@inherited_op_assign base, walked, +=, 5);
    assert_eq!(walked, Meters(15));
    // A declared property through the same arm keeps `AddAssign` too.
    crate::__elwindui_props_OpAssignBase!(@inherited_op_assign base, distance, +=, 4);
    assert_eq!(base.distance(), Meters(5));
}

// ---------------------------------------------------------------------------
// Dynamic regions (`if` / `match` / `for`) over ancestor-declared Props.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum DynMode {
    #[default]
    First,
    Second,
}

#[elwindui::viewmodel]
mod inherited_dyn_row {
    struct InheritedDynRow {
        #[observable(default = String::new())]
        label: String,
    }
}

fn dyn_row(label: &str) -> Rc<InheritedDynRow> {
    let row = InheritedDynRow::new();
    row.set_label(label.to_string());
    row
}

/// Base: declares every Prop the derived regions depend on, plus an unrelated one.
#[elwindui::component(inherits VerticalLayout)]
struct DynamicRegionBase {
    #[prop(default = false)]
    show: bool,
    #[prop(default = DynMode::First)]
    mode: DynMode,
    #[prop(default = Vec::new())]
    rows: Vec<Rc<InheritedDynRow>>,
    #[prop(default = 0)]
    unrelated: i32,

    body: view! {
        TextBlock { text: "base" }
    },
}

#[elwindui::component]
impl DynamicRegionBase {}

#[elwindui::component(inherits crate::DynamicRegionBase)]
struct DynamicIfDerived {
    body: view! {
        #[id("if_region")]
        let if_region = VerticalLayout {
            if show {
                TextBlock { text: "shown" }
            } else {
                TextBlock { text: "hidden" }
            }
        };

        if_region
    },
}

#[elwindui::component]
impl DynamicIfDerived {}

#[elwindui::component(inherits crate::DynamicRegionBase)]
struct DynamicMatchForDerived {
    body: view! {
        #[id("regions")]
        let regions = VerticalLayout {
            match mode {
                DynMode::First => TextBlock { text: "first" },
                DynMode::Second => TextBlock { text: "second" },
            }
            for row in rows {
                TextBlock { text: row.label }
            }
        };

        regions
    },
}

#[elwindui::component]
impl DynamicMatchForDerived {}

fn child_texts(root: &dyn UIElementExt) -> Vec<String> {
    root.visual_children()
        .iter()
        .map(|child| {
            child
                .as_any()
                .downcast_ref::<elwindui::ui::TextBlock>()
                .map(text_of)
                .unwrap_or_else(|| "<other>".to_string())
        })
        .collect()
}

fn strings(expected: &[&str]) -> Vec<String> {
    expected.iter().map(|s| s.to_string()).collect()
}

#[test]
fn inherited_prop_drives_derived_if_region() {
    let derived = DynamicIfDerived::new();
    let names = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let sink = Rc::clone(&names);
    let _subscription = ObservableExt::subscribe_property_changed(&*derived, move |name| {
        sink.borrow_mut().push(name)
    });
    let region = derived.if_region();
    let region = region.as_ref();

    assert_eq!(child_texts(region), strings(&["hidden"]));
    derived.base.set_show(true);
    assert_eq!(child_texts(region), strings(&["shown"]));
    derived.base.set_show(false);
    assert_eq!(child_texts(region), strings(&["hidden"]));
    assert_eq!(
        *names.borrow(),
        vec!["show", "show"],
        "each inherited write is reported exactly once, under the declaring property's name"
    );

    // An unrelated inherited Prop must not touch the region: the visible child keeps its identity
    // and a sentinel written straight into it survives.
    let before = region.visual_children();
    let visible = before[0]
        .as_any()
        .downcast_ref::<elwindui::ui::TextBlock>()
        .expect("if branch child is a TextBlock");
    *visible.text.borrow_mut() = "sentinel".to_string();
    derived.base.set_unrelated(7);
    let after = region.visual_children();
    assert_eq!(after.len(), 1);
    assert!(Rc::ptr_eq(&before[0], &after[0]));
    assert_eq!(child_texts(region), strings(&["sentinel"]));
}

#[test]
fn inherited_prop_drives_derived_match_and_for_regions() {
    let derived = DynamicMatchForDerived::new();
    let regions = derived.regions();
    let regions = regions.as_ref();
    assert_eq!(child_texts(regions), strings(&["first"]));

    derived.base.set_mode(DynMode::Second);
    assert_eq!(child_texts(regions), strings(&["second"]));

    let (a, b, c) = (dyn_row("a"), dyn_row("b"), dyn_row("c"));
    derived.base.set_rows(vec![a.clone(), b.clone()]);
    assert_eq!(child_texts(regions), strings(&["second", "a", "b"]));
    derived.base.set_rows(vec![c.clone(), a.clone()]);
    assert_eq!(child_texts(regions), strings(&["second", "c", "a"]));

    derived.base.set_mode(DynMode::First);
    assert_eq!(child_texts(regions), strings(&["first", "c", "a"]));

    let before = regions.visual_children();
    derived.base.set_unrelated(1);
    let after = regions.visual_children();
    assert_eq!(before.len(), after.len());
    assert!(
        before.iter().zip(&after).all(|(x, y)| Rc::ptr_eq(x, y)),
        "an unrelated inherited Prop must not rebuild any dynamic region"
    );

    derived.base.set_rows(Vec::new());
    assert_eq!(child_texts(regions), strings(&["first"]));
}

// ---------------------------------------------------------------------------
// Root-direct dynamic regions over a user-defined base (with and without its own view). The
// base's own children come first in the shared collection; the derived Component's root
// children (static and dynamic) follow them in authored order.
// ---------------------------------------------------------------------------

#[elwindui::component(inherits VerticalLayout)]
struct ViewlessRegionBase {
    #[prop(default = false)]
    show: bool,
    #[prop(default = Vec::new())]
    rows: Vec<Rc<InheritedDynRow>>,
}

#[elwindui::component]
impl ViewlessRegionBase {}

#[elwindui::component(inherits crate::DynamicRegionBase)]
struct RootIfOverViewBase {
    body: view! {
        TextBlock { text: "static-before" }
        if show {
            TextBlock { text: "shown" }
        } else {
            TextBlock { text: "hidden" }
        }
        TextBlock { text: "static-after" }
    },
}

#[elwindui::component]
impl RootIfOverViewBase {}

#[elwindui::component(inherits crate::DynamicRegionBase)]
struct RootMatchForOverViewBase {
    body: view! {
        match mode {
            DynMode::First => TextBlock { text: "first" },
            DynMode::Second => TextBlock { text: "second" },
        }
        for row in rows {
            TextBlock { text: row.label }
        }
    },
}

#[elwindui::component]
impl RootMatchForOverViewBase {}

#[elwindui::component(inherits crate::ViewlessRegionBase)]
struct RootIfForOverViewlessBase {
    body: view! {
        if show {
            TextBlock { text: "shown" }
        }
        for row in rows {
            TextBlock { text: row.label }
        }
    },
}

#[elwindui::component]
impl RootIfForOverViewlessBase {}

#[test]
fn root_direct_if_over_base_with_view_follows_inherited_prop() {
    let derived = RootIfOverViewBase::new();
    let names = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let sink = Rc::clone(&names);
    let _subscription = ObservableExt::subscribe_property_changed(&*derived, move |name| {
        sink.borrow_mut().push(name)
    });
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["base", "static-before", "hidden", "static-after"])
    );
    derived.base.set_show(true);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["base", "static-before", "shown", "static-after"])
    );
    derived.base.set_show(false);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["base", "static-before", "hidden", "static-after"])
    );
    assert_eq!(*names.borrow(), vec!["show", "show"]);

    let before = derived.visual_children();
    derived.base.set_unrelated(3);
    let after = derived.visual_children();
    assert_eq!(before.len(), after.len());
    assert!(before.iter().zip(&after).all(|(x, y)| Rc::ptr_eq(x, y)));
}

#[test]
fn root_direct_match_and_for_over_base_with_view_follow_inherited_props() {
    let derived = RootMatchForOverViewBase::new();
    assert_eq!(child_texts(derived.as_ref()), strings(&["base", "first"]));
    derived.base.set_mode(DynMode::Second);
    let (a, b) = (dyn_row("a"), dyn_row("b"));
    derived.base.set_rows(vec![a.clone(), b.clone()]);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["base", "second", "a", "b"])
    );
    derived.base.set_rows(vec![b.clone()]);
    derived.base.set_mode(DynMode::First);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["base", "first", "b"])
    );
    derived.base.set_rows(Vec::new());
    assert_eq!(child_texts(derived.as_ref()), strings(&["base", "first"]));
}

#[test]
fn root_direct_if_and_for_over_viewless_base_follow_inherited_props() {
    // The view-less base composes over `VerticalLayout` like an empty `body`: it is a real UI
    // element whose props, content collection and accessors its derived Components inherit.
    let base = ViewlessRegionBase::new();
    assert!(!base.show());
    assert!(base.visual_children().is_empty());

    let derived = RootIfForOverViewlessBase::new();
    assert_eq!(child_texts(derived.as_ref()), Vec::<String>::new());
    derived.base.set_show(true);
    assert_eq!(child_texts(derived.as_ref()), strings(&["shown"]));
    let (a, b) = (dyn_row("a"), dyn_row("b"));
    derived.base.set_rows(vec![a.clone(), b.clone()]);
    assert_eq!(child_texts(derived.as_ref()), strings(&["shown", "a", "b"]));
    derived.base.set_show(false);
    assert_eq!(child_texts(derived.as_ref()), strings(&["a", "b"]));
    derived.base.set_rows(vec![b.clone(), a.clone()]);
    assert_eq!(child_texts(derived.as_ref()), strings(&["b", "a"]));
}

// ---------------------------------------------------------------------------
// A base's own view keeps working when it is embedded as a derived Component's `base`.
// ---------------------------------------------------------------------------

#[elwindui::component(inherits VerticalLayout)]
struct LivePrefixBase {
    #[prop(default = Vec::new())]
    base_rows: Vec<Rc<InheritedDynRow>>,
    #[prop(default = "base-label".to_string())]
    base_label: String,
    #[prop(default = 0)]
    base_taps: i32,

    body: view! {
        #[id("base_text")]
        let base_text = TextBlock {
            text: base_label,
            on_tapped: |_event| {
                base_taps += 1;
            },
        };
        base_text
        for row in base_rows {
            TextBlock { text: row.label }
        }
    },
}

#[elwindui::component]
impl LivePrefixBase {}

#[elwindui::component(inherits crate::LivePrefixBase)]
struct LivePrefixDerived {
    #[prop(default = false)]
    show_tail: bool,

    body: view! {
        TextBlock { text: "d-static" }
        if show_tail {
            TextBlock { text: "d-dyn" }
        }
    },
}

#[elwindui::component]
impl LivePrefixDerived {}

#[elwindui::component(inherits crate::LivePrefixDerived)]
struct LivePrefixLeaf {
    body: view! {
        TextBlock { text: "leaf" }
    },
}

#[elwindui::component]
impl LivePrefixLeaf {}

#[test]
fn embedded_base_view_keeps_its_own_bindings_regions_and_handlers() {
    let derived = LivePrefixDerived::new();
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["base-label", "d-static"])
    );

    // The base's own attribute binding re-syncs on its own setter.
    derived.base.set_base_label("relabeled".to_string());
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["relabeled", "d-static"])
    );

    // The base's own dynamic region refreshes, and the derived root region stays after it.
    let (a, b) = (dyn_row("a"), dyn_row("b"));
    derived.base.set_base_rows(vec![a.clone(), b.clone()]);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["relabeled", "a", "b", "d-static"])
    );
    derived.set_show_tail(true);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["relabeled", "a", "b", "d-static", "d-dyn"])
    );
    derived.base.set_base_rows(vec![b.clone()]);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["relabeled", "b", "d-static", "d-dyn"])
    );
    derived.set_show_tail(false);
    derived.set_show_tail(true);
    assert_eq!(
        child_texts(derived.as_ref()),
        strings(&["relabeled", "b", "d-static", "d-dyn"])
    );

    // The base's own event handler is wired.
    tap(derived.base.base_text());
    assert_eq!(derived.base.base_taps(), 1);
}

#[test]
fn twice_embedded_base_view_keeps_its_own_regions() {
    let leaf = LivePrefixLeaf::new();
    leaf.base.base.set_base_rows(vec![dyn_row("x")]);
    leaf.base.set_show_tail(true);
    assert_eq!(
        child_texts(leaf.as_ref()),
        strings(&["base-label", "x", "d-static", "d-dyn", "leaf"])
    );
    leaf.base.base.set_base_label("deep".to_string());
    assert_eq!(
        child_texts(leaf.as_ref()),
        strings(&["deep", "x", "d-static", "d-dyn", "leaf"])
    );
}
