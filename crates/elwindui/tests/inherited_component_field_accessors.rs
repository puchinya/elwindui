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

// Cross-crate (D6): the declaring ancestor lives in another crate. The consumer macro does not
// classify `heading`; the defining crate's shape macro / rustc resolve it.
use elwindui_external_component_fixture::ExternalInheritBaseExt as _;

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInheritBase)]
struct CrossCrateDerived {
    body: view! {
        #[id("cross_label")]
        let cross_label = TextBlock {
            text: heading,
            on_tapped: |_event| {
                heading = format!("{heading}+tap");
            },
        };

        cross_label
    },
}

#[elwindui::component]
impl CrossCrateDerived {}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInheritMiddle)]
struct CrossCrateLeaf {
    body: view! {
        #[id("leaf_cross_label")]
        let leaf_cross_label = TextBlock {
            text: format!("[{heading}]"),
            on_tapped: |_event| {
                heading = "leaf".to_string();
            },
        };

        leaf_cross_label
    },
}

#[elwindui::component]
impl CrossCrateLeaf {}

#[test]
fn cross_crate_derived_reads_and_writes_external_ancestor_prop() {
    let derived = CrossCrateDerived::new();
    assert_eq!(text_of(&derived.cross_label()), "external-before");
    tap(derived.cross_label());
    assert_eq!(derived.base.heading(), "external-before+tap");
    assert_eq!(text_of(&derived.cross_label()), "external-before+tap");
}

#[test]
fn cross_crate_multi_hop_through_external_middle() {
    let leaf = CrossCrateLeaf::new();
    assert_eq!(text_of(&leaf.leaf_cross_label()), "[external-before]");
    tap(leaf.leaf_cross_label());
    assert_eq!(leaf.base.base.heading(), "leaf");
    assert_eq!(text_of(&leaf.leaf_cross_label()), "[leaf]");
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

// View-less local middle over a qualified external generated base: it is itself an inheritable
// composition hop, and `heading` (declared in the external crate) stays reachable two hops down.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInheritBase)]
struct LocalViewlessMiddle {
    #[prop(default = 5)]
    local_only: i32,
}

#[elwindui::component]
impl LocalViewlessMiddle {}

#[elwindui::component(inherits crate::LocalViewlessMiddle)]
struct LocalViewlessLeaf {
    body: view! {
        #[id("viewless_leaf_label")]
        let viewless_leaf_label = TextBlock {
            text: format!("{heading}/{local_only}"),
            on_tapped: |_event| {
                heading = "viewless".to_string();
                local_only += 1;
            },
        };

        viewless_leaf_label
    },
}

#[elwindui::component]
impl LocalViewlessLeaf {}

#[test]
fn cross_crate_multi_hop_through_local_viewless_middle() {
    let middle = LocalViewlessMiddle::new();
    assert_eq!(middle.heading(), "external-before");
    assert_eq!(middle.local_only(), 5);

    let leaf = LocalViewlessLeaf::new();
    assert_eq!(text_of(&leaf.viewless_leaf_label()), "external-before/5");
    tap(leaf.viewless_leaf_label());
    assert_eq!(leaf.base.heading(), "viewless");
    assert_eq!(leaf.base.local_only(), 6);
    assert_eq!(text_of(&leaf.viewless_leaf_label()), "viewless/6");
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
