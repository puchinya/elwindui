use super::DockTarget;
use super::Orientation;
use super::core::base::{Point, Size};
use super::core::environment::application_environment;
use super::core::focus::FocusTracker;
use super::core::input::{
    KeyModifiers, MouseButton, PointerDispatcher, RawPointerEvent, RawPointerEventKind,
};
use super::core::layout::{GridLength, Visibility};
use super::core::ui::{
    ContentControlExt, Grid, GridExt, LayoutExt, Rectangle, TextBlock, TextBlockExt, UIElementExt,
    layout_root, unmount_subtree,
};
use super::core::visual_tree::find_all;
use super::docking_control::DockTabContextAction;
use super::id::{DockGroupId, DockItemId};
use super::model::{
    DefaultDockDefinition, DockLayoutModel, InternalDockGroupKey, InternalDockGroupPlacement,
    InternalDockPlacement, Node, RootKind, SplitAddress, WeightedNode,
};
use super::placement::{DockLayoutError, DockPlacement, DockSide};
use super::runtime::{
    AutoHideOverlay, DockSurfaceView, DragSession, DragSourceGeometry, DropPreview,
    FloatingHostFactory, FloatingHostRegistry, FloatingWindowHost, LatestOnlyQueue,
    ResolvedDockTarget, SurfaceRegistry, resolve_local_target_for_test,
};
use super::snapshot::{
    DockLayoutSnapshot, SnapshotAutoHideEntry, SnapshotFloatingRoot, SnapshotGroupKey,
    SnapshotNode, SnapshotOrientation, SnapshotRect, SnapshotReturnState, SnapshotWeightedNode,
};
use super::{
    DockGroup, DockGroupExt, DockItem, DockItemExt, DockSplitPanel, DockSplitPanelExt,
    DockingControl, DockingControlExt,
};
use elwindui_core::base::Rect;
use elwindui_custom_controls::{
    CustomSplitter, CustomTabView, CustomTabViewItem, TabDragCompletedEventArgs,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[elwindui_macros::class(inherits = elwindui_core::ui::UIElement)]
struct DockingMeasureProbe {
    measure_count: Cell<usize>,
    reported_size: Size,
}

#[elwindui_macros::class]
impl DockingMeasureProbe {
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        self.measure_count.set(self.measure_count.get() + 1);
        self.reported_size
    }

    fn construct(reported_size: Size) -> Self {
        Self {
            base: elwindui_core::ui::UIElement::construct(),
            measure_count: Cell::new(0),
            reported_size,
        }
    }
}

impl DockingMeasureProbe {
    fn reset_measure_count(&self) {
        self.measure_count.set(0);
    }

    fn measure_count(&self) -> usize {
        self.measure_count.get()
    }
}

struct FakeHostLog {
    events: RefCell<Vec<&'static str>>,
    close_count: Cell<usize>,
    bounds: Cell<Option<Rect>>,
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    close_handler: RefCell<Option<Rc<dyn Fn() -> bool>>>,
}

impl FakeHostLog {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            events: RefCell::new(Vec::new()),
            close_count: Cell::new(0),
            bounds: Cell::new(None),
            content: RefCell::new(None),
            close_handler: RefCell::new(None),
        })
    }

    fn invoke_close(&self) -> bool {
        let handler = self.close_handler.borrow().clone();
        handler.map_or(false, |handler| handler())
    }
}

struct FakeHost {
    log: Rc<FakeHostLog>,
}

impl FakeHost {
    fn new(log: Rc<FakeHostLog>) -> Rc<Self> {
        Rc::new(Self { log })
    }
}

impl FloatingWindowHost for FakeHost {
    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.log.events.borrow_mut().push("set_content");
        *self.log.content.borrow_mut() = Some(content);
    }

    fn set_bounds(&self, bounds: Rect) {
        self.log.events.borrow_mut().push("set_bounds");
        self.log.bounds.set(Some(bounds));
    }

    fn set_title(&self, _title: &str) {}

    fn show(&self) {
        self.log.events.borrow_mut().push("show");
    }

    fn activate(&self) {}

    fn close(&self) {
        self.log.events.borrow_mut().push("close");
        self.log.close_count.set(self.log.close_count.get() + 1);
    }

    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        self.log.events.borrow_mut().push(if handler.is_some() {
            "set_close_handler"
        } else {
            "clear_close_handler"
        });
        *self.log.close_handler.borrow_mut() = handler;
    }

    fn set_bounds_changed_handler(&self, _handler: Option<Rc<dyn Fn(Rect)>>) {}
}

fn fake_factory(
    hosts: Rc<RefCell<Vec<Rc<FakeHost>>>>,
    log: Rc<FakeHostLog>,
) -> FloatingHostFactory {
    Rc::new(move || {
        log.events.borrow_mut().push("create");
        let host = FakeHost::new(log.clone());
        hosts.borrow_mut().push(host.clone());
        Ok(host as Rc<dyn FloatingWindowHost>)
    })
}

fn individual_fake_factory(hosts: Rc<RefCell<Vec<Rc<FakeHost>>>>) -> FloatingHostFactory {
    Rc::new(move || {
        let log = FakeHostLog::new();
        let host = FakeHost::new(log);
        hosts.borrow_mut().push(host.clone());
        Ok(host as Rc<dyn FloatingWindowHost>)
    })
}

fn empty_auto_hide<T>() -> [Vec<T>; 4] {
    std::array::from_fn(|_| Vec::new())
}

fn assert_rect_eq(actual: Option<Rect>, expected: Rect) {
    assert_eq!(actual, Some(expected));
}

fn pointer_event(kind: RawPointerEventKind, position: Point) -> RawPointerEvent {
    RawPointerEvent {
        kind,
        position,
        screen_position: None,
        modifiers: KeyModifiers::default(),
        timestamp_ms: 0.0,
    }
}

fn pointer_event_with_screen(
    kind: RawPointerEventKind,
    position: Point,
    screen_position: Point,
) -> RawPointerEvent {
    RawPointerEvent {
        kind,
        position,
        screen_position: Some(screen_position),
        modifiers: KeyModifiers::default(),
        timestamp_ms: 0.0,
    }
}

fn item(value: &str) -> DockItemId {
    DockItemId::from(value)
}

fn group(value: &str) -> DockGroupId {
    DockGroupId::from(value)
}

fn test_drag(model: &DockLayoutModel, item: DockItemId) -> DragSession {
    DragSession::begin(
        model,
        item,
        RootKind::Main,
        DragSourceGeometry {
            source_root: RootKind::Main,
            source_bounds_host: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 240.0,
            },
            pointer_offset: super::core::base::Point { x: 10.0, y: 10.0 },
        },
    )
    .unwrap()
}

fn resolved_target(target: DockTarget, group: Option<SnapshotGroupKey>) -> ResolvedDockTarget {
    ResolvedDockTarget {
        root: RootKind::Main,
        target,
        group,
        preview_rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        },
    }
}

fn default_model() -> DockLayoutModel {
    let first = item("first");
    let second = item("second");
    let third = item("third");
    let root = Node::Split {
        orientation: Orientation::Horizontal,
        children: vec![
            WeightedNode {
                weight: 2.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("documents")),
                    items: vec![first, second],
                    selected: Some(item("first")),
                },
            },
            WeightedNode {
                weight: 1.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("tools")),
                    items: vec![third],
                    selected: Some(item("third")),
                },
            },
        ],
    };
    DockLayoutModel::from_default(DefaultDockDefinition::new(Some(root)))
}

fn snapshot_group_items(
    snapshot: &DockLayoutSnapshot,
    target: &SnapshotGroupKey,
) -> Option<Vec<DockItemId>> {
    fn find(node: &SnapshotNode, target: &SnapshotGroupKey) -> Option<Vec<DockItemId>> {
        match node {
            SnapshotNode::Group { group, items, .. } if group == target => Some(items.clone()),
            SnapshotNode::Group { .. } => None,
            SnapshotNode::Split { children, .. } => {
                children.iter().find_map(|child| find(&child.node, target))
            }
        }
    }
    snapshot
        .main_root
        .as_ref()
        .and_then(|root| find(root, target))
}

fn authored_item(
    id: &str,
    title: &str,
    can_close: bool,
) -> (Rc<DockItem>, Rc<super::core::ui::TextBlock>) {
    let page = super::core::ui::TextBlock::new();
    page.set_text(title);
    let dock_item = DockItem::new_item();
    dock_item.set_id(item(id));
    dock_item.set_title(title.to_owned());
    dock_item.set_can_close(can_close);
    dock_item.set_content(page.clone());
    (dock_item, page)
}

fn mounted_default_docking() -> Rc<DockingControl> {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", true);
    let (third, _) = authored_item("third", "Third", true);
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_children(vec![first, second]);
    let tools = DockGroup::new_group();
    tools.set_id(group("tools"));
    tools.set_children(vec![third]);
    let split = DockSplitPanel::new_panel();
    split.set_children(vec![
        documents as Rc<dyn UIElementExt>,
        tools as Rc<dyn UIElementExt>,
    ]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    docking
}

fn floating_auto_hide_snapshot_model() -> DockLayoutModel {
    let mut auto_hide = empty_auto_hide();
    auto_hide[DockSide::Left.index()].push(SnapshotAutoHideEntry {
        item: item("hidden"),
        open: false,
        return_state: SnapshotReturnState {
            group: SnapshotGroupKey::Generated(100),
            index: 0,
            floating_root: Some(0),
        },
    });
    DockLayoutModel::from_snapshot(DockLayoutSnapshot {
        version: DockLayoutSnapshot::VERSION,
        main_root: Some(SnapshotNode::Group {
            group: SnapshotGroupKey::Authored(group("main")),
            items: vec![item("main")],
            selected: Some(item("main")),
        }),
        floating_roots: vec![SnapshotFloatingRoot {
            bounds: SnapshotRect {
                x: 900.0,
                y: 100.0,
                width: 420.0,
                height: 260.0,
            },
            root: SnapshotNode::Group {
                group: SnapshotGroupKey::Generated(100),
                items: vec![item("stay")],
                selected: Some(item("stay")),
            },
        }],
        auto_hide,
        closed: Vec::new(),
        next_generated_group_id: 101,
        active_item: None,
    })
    .expect("valid floating auto-hide snapshot")
}

fn mounted_docking_with_items(items: Vec<Rc<DockItem>>) -> Rc<DockingControl> {
    let dock_group = DockGroup::new_group();
    dock_group.set_id(group("main"));
    dock_group.set_children(items);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(dock_group);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    docking
}

fn mounted_three_pane_docking() -> Rc<DockingControl> {
    let groups = (0..3)
        .map(|index| {
            let (dock_item, _) = authored_item(
                &format!("split-item-{index}"),
                &format!("Split item {index}"),
                true,
            );
            let dock_group = DockGroup::new_group();
            dock_group.set_id(group(&format!("split-group-{index}")));
            dock_group.set_children(vec![dock_item]);
            dock_group as Rc<dyn UIElementExt>
        })
        .collect::<Vec<_>>();
    let split = DockSplitPanel::new_panel();
    split.set_children(groups);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    docking
}

fn mounted_three_pane_probed_docking() -> (Rc<DockingControl>, Vec<Rc<DockingMeasureProbe>>) {
    let mut groups = Vec::new();
    let mut probes = Vec::new();
    for index in 0..3 {
        let probe = DockingMeasureProbe::new(Size {
            width: 80.0,
            height: 40.0,
        });
        let dock_item = DockItem::new_item();
        dock_item.set_id(item(&format!("probed-split-item-{index}")));
        dock_item.set_title(format!("Probed split item {index}"));
        dock_item.set_content(probe.clone());
        let dock_group = DockGroup::new_group();
        dock_group.set_id(group(&format!("probed-split-group-{index}")));
        dock_group.set_children(vec![dock_item]);
        groups.push(dock_group as Rc<dyn UIElementExt>);
        probes.push(probe);
    }
    let split = DockSplitPanel::new_panel();
    split.set_children(groups);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    (docking, probes)
}

fn floating_model_with_items(
    model: &DockLayoutModel,
    item_ids: &[&str],
    bounds: Rect,
) -> DockLayoutModel {
    assert!(!item_ids.is_empty());
    let mut result = model
        .with_item_moved(&item(item_ids[0]), DockPlacement::Floating { bounds })
        .expect("first item should float");
    let group = match result
        .snapshot()
        .floating_roots
        .first()
        .map(|root| &root.root)
    {
        Some(SnapshotNode::Group { group, .. }) => group.clone(),
        _ => panic!("floating placement should create a group root"),
    };
    for item_id in &item_ids[1..] {
        result = result
            .with_item_moved_internal(
                &item(item_id),
                InternalDockPlacement::Group {
                    group: group.clone().into(),
                    index: None,
                },
            )
            .expect("item should join the floating group");
    }
    result
}

#[test]
fn empty_default_initializes_and_reset_restores_the_authored_tree() {
    let model = default_model();
    assert!(!model.is_empty());
    assert!(model.contains_item(&item("first")));
    let changed = model
        .with_item_closed(&item("second"))
        .expect("close second");
    assert!(changed.is_item_closed(&item("second")));
    let reset = changed.with_reset().expect("reset attached default");
    assert!(!reset.is_item_closed(&item("second")));
    assert!(reset.contains_item(&item("second")));
}

#[test]
fn close_reopen_keeps_return_group_and_index() {
    let model = default_model();
    let closed = model.with_item_closed(&item("second")).unwrap();
    assert!(!closed.is_item_active(&item("second")));
    let reopened = closed.with_item_reopened(&item("second")).unwrap();
    assert!(!reopened.is_item_closed(&item("second")));
    let snapshot = serde_json::to_string(&reopened.snapshot()).unwrap();
    assert!(snapshot.contains("second"));
    assert!(snapshot.contains("documents"));
}

#[test]
fn activation_is_global_and_close_repairs_to_the_same_group() {
    let model = default_model();
    let active = model.with_item_activated(&item("second")).unwrap();
    assert_eq!(active.active_item(), Some(item("second")));
    assert!(!active.is_item_active(&item("first")));
    assert!(active.is_item_active(&item("second")));

    let closed = active.with_item_closed(&item("second")).unwrap();
    assert_eq!(closed.active_item(), Some(item("first")));
    assert!(!closed.is_item_active(&item("second")));
    assert!(closed.is_item_active(&item("first")));
}

#[test]
fn same_group_indexed_moves_insert_before_or_after_without_reversing_order() {
    let model = default_model();
    let moved_to_end = model
        .with_item_moved_internal(
            &item("first"),
            InternalDockPlacement::Group {
                group: InternalDockGroupKey::Authored(group("documents")),
                index: Some(2),
            },
        )
        .unwrap();
    assert_eq!(
        snapshot_group_items(
            &moved_to_end.snapshot(),
            &SnapshotGroupKey::Authored(group("documents")),
        ),
        Some(vec![item("second"), item("first")])
    );

    let moved_to_front = model
        .with_item_moved_internal(
            &item("second"),
            InternalDockPlacement::Group {
                group: InternalDockGroupKey::Authored(group("documents")),
                index: Some(0),
            },
        )
        .unwrap();
    assert_eq!(
        snapshot_group_items(
            &moved_to_front.snapshot(),
            &SnapshotGroupKey::Authored(group("documents")),
        ),
        Some(vec![item("second"), item("first")])
    );
}

#[test]
fn group_moves_keep_the_group_node_and_all_pages_together() {
    let model = default_model();
    let split = model
        .with_group_moved_internal(
            &InternalDockGroupKey::Authored(group("documents")),
            InternalDockGroupPlacement::SplitGroup {
                group: InternalDockGroupKey::Authored(group("tools")),
                side: DockSide::Right,
                weight: 1.0,
            },
        )
        .unwrap();
    assert!(matches!(
        split.snapshot().main_root,
        Some(SnapshotNode::Split { ref children, .. })
            if children.iter().any(|child| matches!(
                child.node,
                SnapshotNode::Group { group: SnapshotGroupKey::Authored(ref id), ref items, .. }
                    if id == &group("documents") && items == &vec![item("first"), item("second")]
            ))
    ));

    let merged = model
        .with_group_moved_internal(
            &InternalDockGroupKey::Authored(group("documents")),
            InternalDockGroupPlacement::Center {
                group: InternalDockGroupKey::Authored(group("tools")),
            },
        )
        .unwrap();
    assert_eq!(
        snapshot_group_items(
            &merged.snapshot(),
            &SnapshotGroupKey::Authored(group("tools")),
        ),
        Some(vec![item("third"), item("first"), item("second")])
    );
}

#[test]
fn snapshot_v2_round_trips_active_item_and_rejects_v1() {
    let model = default_model()
        .with_item_activated(&item("second"))
        .unwrap();
    let snapshot = model.snapshot();
    assert_eq!(snapshot.version(), DockLayoutSnapshot::VERSION);
    assert_eq!(snapshot.active_item, Some(item("second")));
    assert_eq!(
        DockLayoutModel::from_snapshot(snapshot.clone())
            .unwrap()
            .snapshot(),
        snapshot
    );

    let mut v1 = snapshot;
    v1.version = 1;
    assert!(matches!(
        DockLayoutModel::from_snapshot(v1),
        Err(DockLayoutError::UnknownSnapshotVersion { version: 1 })
    ));
}

#[test]
fn clear_layout_closes_everything_and_reset_restores_authored_default() {
    let model = default_model()
        .with_item_activated(&item("third"))
        .unwrap()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 20.0,
                    y: 30.0,
                    width: 400.0,
                    height: 300.0,
                },
            },
        )
        .unwrap();
    let cleared = model.with_cleared_layout().unwrap();
    assert_eq!(cleared.active_item(), None);
    assert!(cleared.snapshot().main_root.is_none());
    assert!(cleared.snapshot().floating_roots.is_empty());
    assert!(cleared.snapshot().auto_hide.iter().all(Vec::is_empty));
    assert!(cleared.is_item_closed(&item("first")));
    assert!(cleared.is_item_closed(&item("second")));
    assert!(cleared.is_item_closed(&item("third")));

    let reopened = default_model()
        .with_cleared_layout()
        .unwrap()
        .with_item_reopened(&item("first"))
        .unwrap()
        .with_item_reopened(&item("second"))
        .unwrap()
        .with_item_reopened(&item("third"))
        .unwrap();
    assert_eq!(
        snapshot_group_items(
            &reopened.snapshot(),
            &SnapshotGroupKey::Authored(group("documents"))
        ),
        Some(vec![item("first"), item("second")])
    );
    assert_eq!(
        snapshot_group_items(
            &reopened.snapshot(),
            &SnapshotGroupKey::Authored(group("tools"))
        ),
        Some(vec![item("third")])
    );

    let reset = cleared.with_reset().unwrap();
    assert!(reset.snapshot().main_root.is_some());
    assert!(reset.snapshot().closed.is_empty());
    assert_eq!(reset.active_item(), None);
}

#[test]
fn group_split_and_outer_edge_cover_all_four_sides() {
    let model = default_model();
    for side in DockSide::ALL {
        let split = model
            .with_item_moved(
                &item("first"),
                DockPlacement::SplitGroup {
                    group: group("tools"),
                    side,
                    weight: 2.0,
                },
            )
            .unwrap();
        assert!(split.contains_item(&item("first")));

        let edge = model
            .with_item_moved(
                &item("first"),
                DockPlacement::RootEdge { side, weight: 2.0 },
            )
            .unwrap();
        assert!(edge.contains_item(&item("first")));
        assert_eq!(edge.snapshot().version(), DockLayoutSnapshot::VERSION);
    }
}

#[test]
fn floating_and_auto_hide_are_value_transformations() {
    let model = default_model();
    let floating = model
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 20.0,
                    y: 30.0,
                    width: 400.0,
                    height: 300.0,
                },
            },
        )
        .unwrap();
    assert!(floating.contains_item(&item("first")));
    let auto_hidden = floating
        .with_item_moved(
            &item("first"),
            DockPlacement::AutoHide {
                side: DockSide::Right,
            },
        )
        .unwrap();
    assert!(auto_hidden.contains_item(&item("first")));
    assert!(!auto_hidden.is_item_closed(&item("first")));
    assert!(auto_hidden.is_item_auto_hidden(&item("first")));
    assert!(
        auto_hidden
            .with_item_activated(&item("first"))
            .unwrap()
            .is_item_active(&item("first"))
    );
    let restored = DockLayoutModel::from_snapshot(auto_hidden.snapshot()).unwrap();
    assert_eq!(restored.snapshot(), auto_hidden.snapshot());
    let unpinned = auto_hidden.with_item_unpinned(&item("first")).unwrap();
    assert!(!unpinned.is_item_auto_hidden(&item("first")));
    assert!(unpinned.is_item_active(&item("first")));
}

#[test]
fn invalid_programmatic_values_return_typed_errors() {
    let model = default_model();
    assert_eq!(
        model.with_item_moved(
            &item("first"),
            DockPlacement::RootEdge {
                side: DockSide::Left,
                weight: 0.0,
            },
        ),
        Err(DockLayoutError::InvalidWeight)
    );
    assert_eq!(
        model.with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: f32::NAN,
                    height: 2.0,
                },
            },
        ),
        Err(DockLayoutError::InvalidBounds)
    );
    assert_eq!(
        model.with_item_activated(&item("missing")),
        Err(DockLayoutError::UnknownItem(item("missing")))
    );
}

#[test]
fn snapshot_json_round_trip_omits_authored_defaults_and_rejects_unknown_version() {
    let model = default_model();
    let snapshot = model.snapshot();
    let json = serde_json::to_string(&snapshot).unwrap();
    let parsed: DockLayoutSnapshot = serde_json::from_str(&json).unwrap();
    let restored = DockLayoutModel::from_snapshot(parsed).unwrap();
    assert_eq!(restored.snapshot(), snapshot);

    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["version"] = serde_json::json!(99);
    let unknown: DockLayoutSnapshot = serde_json::from_value(value).unwrap();
    assert_eq!(
        DockLayoutModel::from_snapshot(unknown),
        Err(DockLayoutError::UnknownSnapshotVersion { version: 99 })
    );
    assert_eq!(
        DockLayoutModel::from_snapshot(snapshot)
            .unwrap()
            .with_reset(),
        Err(DockLayoutError::DefaultLayoutUnavailable)
    );
}

#[test]
fn ids_are_transparent_string_newtypes() {
    let item_json = serde_json::to_string(&item("doc")).unwrap();
    let group_json = serde_json::to_string(&group("group")).unwrap();
    assert_eq!(item_json, "\"doc\"");
    assert_eq!(group_json, "\"group\"");
    assert_eq!(DockItemId::from("doc").as_ref(), "doc");
    assert_eq!(DockGroupId::from("group").to_string(), "group");
}

#[test]
fn group_move_and_generated_empty_groups_normalize_without_losing_items() {
    let moved = default_model()
        .with_item_moved(
            &item("first"),
            DockPlacement::Group {
                group: group("tools"),
                index: Some(0),
            },
        )
        .unwrap();
    assert!(moved.contains_item(&item("first")));
    assert!(moved.is_item_active(&item("first")));

    let root = Node::Split {
        orientation: Orientation::Vertical,
        children: vec![
            WeightedNode {
                weight: 1.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("only")),
                    items: vec![item("only-item")],
                    selected: None,
                },
            },
            WeightedNode {
                weight: 1.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Generated(42),
                    items: Vec::new(),
                    selected: None,
                },
            },
        ],
    };
    let normalized = DockLayoutModel::from_default(DefaultDockDefinition::new(Some(root)));
    let json = serde_json::to_string(&normalized.snapshot()).unwrap();
    assert!(!json.contains("Generated"));
    assert!(!json.contains("Split"));
}

#[test]
fn duplicate_live_references_are_deterministically_deduplicated() {
    let duplicate = item("duplicate");
    let root = Node::Split {
        orientation: Orientation::Horizontal,
        children: vec![
            WeightedNode {
                weight: 1.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("first-group")),
                    items: vec![duplicate.clone()],
                    selected: Some(duplicate.clone()),
                },
            },
            WeightedNode {
                weight: 1.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("second-group")),
                    items: vec![duplicate.clone()],
                    selected: Some(duplicate),
                },
            },
        ],
    };
    let normalized = DockLayoutModel::from_default(DefaultDockDefinition::new(Some(root)));
    let json = serde_json::to_string(&normalized.snapshot()).unwrap();
    assert_eq!(json.matches(r#""selected":"duplicate""#).count(), 1);
}

#[test]
fn source_updates_are_latest_only_and_equal_values_are_silent() {
    let model = default_model();
    let mut queue = LatestOnlyQueue::new();
    assert_eq!(queue.request(&model, model.clone()), None);
    let first = model.with_item_closed(&item("first")).unwrap();
    assert_eq!(queue.request(&model, first.clone()), Some(first.clone()));
    let second = model.with_item_closed(&item("second")).unwrap();
    assert_eq!(queue.request(&model, second.clone()), None);
    assert_eq!(queue.finish(), Some(second));
    assert_eq!(queue.finish(), None);
}

#[test]
fn public_root_edge_targets_main_even_when_a_floating_root_exists() {
    let model = default_model()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .unwrap();
    let floating_before = model.snapshot().floating_roots[0].root.clone();
    let next = model
        .with_item_moved(
            &item("second"),
            DockPlacement::RootEdge {
                side: DockSide::Left,
                weight: 1.0,
            },
        )
        .unwrap();
    assert_eq!(next.snapshot().floating_roots[0].root, floating_before);
    assert!(matches!(
        next.snapshot().main_root,
        Some(SnapshotNode::Split { .. })
    ));
}

#[test]
fn private_root_edge_can_target_a_floating_root() {
    let model = default_model()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .unwrap();
    let main_before = model.snapshot().main_root.clone();
    let next = model
        .with_item_moved_internal(
            &item("second"),
            super::model::InternalDockPlacement::RootEdge {
                root: RootKind::Floating(0),
                side: DockSide::Right,
                weight: 1.0,
            },
        )
        .unwrap();
    // Moving the source item out of the main root legitimately removes it from that root. The
    // assertion is about the private edge target: it must not add the item to a new main edge.
    assert_ne!(next.snapshot().main_root, main_before);
    fn has_generated(node: &SnapshotNode) -> bool {
        match node {
            SnapshotNode::Group { group, .. } => {
                matches!(group, SnapshotGroupKey::Generated(_))
            }
            SnapshotNode::Split { children, .. } => {
                children.iter().any(|child| has_generated(&child.node))
            }
        }
    }
    assert!(
        !next
            .snapshot()
            .main_root
            .as_ref()
            .is_some_and(has_generated)
    );
    assert!(matches!(
        next.snapshot().floating_roots[0].root,
        SnapshotNode::Split { .. }
    ));
}

#[test]
fn private_root_edge_rejects_an_invalid_floating_root_without_mutation() {
    let model = default_model();
    let error = model
        .with_item_moved_internal(
            &item("first"),
            super::model::InternalDockPlacement::RootEdge {
                root: RootKind::Floating(999),
                side: DockSide::Left,
                weight: 1.0,
            },
        )
        .unwrap_err();
    assert_eq!(error, DockLayoutError::InvalidFloatingRoot { index: 999 });
    assert_eq!(model, default_model());
}

#[test]
fn surface_registry_converts_host_root_points_after_nested_surface_offsets() {
    let host = Grid::new();
    host.set_rows(vec![GridLength::Fixed(18.0), GridLength::Fixed(80.0)]);
    host.set_columns(vec![GridLength::Fixed(25.0), GridLength::Fixed(100.0)]);
    let surface = Grid::new();
    surface.set_attached("Grid", "row", 1i32);
    surface.set_attached("Grid", "column", 1i32);
    host.children().add(surface.clone());
    let host_node: Rc<dyn UIElementExt> = host.clone();
    layout_root(
        &host_node,
        Size {
            width: 300.0,
            height: 300.0,
        },
    );
    let surface_node: Rc<dyn UIElementExt> = surface.clone();
    assert_rect_eq(
        SurfaceRegistry::bounds_in_host_root(&surface_node),
        Rect {
            x: 25.0,
            y: 18.0,
            width: 100.0,
            height: 80.0,
        },
    );
    assert_eq!(
        SurfaceRegistry::host_root_to_surface_local(&surface_node, Point { x: 40.0, y: 30.0 },),
        Some(Point { x: 15.0, y: 12.0 })
    );
}

struct OffsetCoordinateHost {
    screen_origin: Point,
}

impl super::core::ui::CoordinateHost for OffsetCoordinateHost {
    fn root_to_screen(&self, point: Point) -> Option<Point> {
        Some(Point {
            x: point.x + self.screen_origin.x,
            y: point.y + self.screen_origin.y,
        })
    }

    fn screen_to_root(&self, point: Point) -> Option<Point> {
        Some(Point {
            x: point.x - self.screen_origin.x,
            y: point.y - self.screen_origin.y,
        })
    }
}

#[test]
fn floating_surface_screen_conversion_subtracts_surface_origin() {
    let surface = Grid::new();
    surface.set_width(240.0);
    surface.set_height(160.0);
    surface.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 900.0, y: 100.0 },
    })));
    surface.arrange(Rect {
        x: 20.0,
        y: 30.0,
        width: 240.0,
        height: 160.0,
    });
    let surface_node: Rc<dyn UIElementExt> = surface;
    let host_root = surface_node
        .screen_to_root(Point { x: 950.0, y: 160.0 })
        .expect("coordinate host should convert screen coordinates");
    assert_eq!(host_root, Point { x: 50.0, y: 60.0 });
    assert_eq!(
        SurfaceRegistry::host_root_to_surface_local(&surface_node, host_root),
        Some(Point { x: 30.0, y: 30.0 })
    );
}

#[test]
fn local_target_resolution_is_restricted_to_the_drag_source_root_without_screen_position() {
    let main_target = resolve_local_target_for_test(
        RootKind::Main,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 240.0,
        },
        Point { x: 3.0, y: 120.0 },
        vec![(
            SnapshotGroupKey::Authored(group("main-group")),
            Rect {
                x: 80.0,
                y: 40.0,
                width: 240.0,
                height: 160.0,
            },
        )],
    )
    .expect("main source surface should resolve");
    assert_eq!(main_target.root, RootKind::Main);
    assert_eq!(main_target.target, DockTarget::DockLeft);

    let floating_target = resolve_local_target_for_test(
        RootKind::Floating(0),
        Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 240.0,
        },
        Point { x: 3.0, y: 120.0 },
        vec![(
            SnapshotGroupKey::Authored(group("floating-group")),
            Rect {
                x: 80.0,
                y: 40.0,
                width: 240.0,
                height: 160.0,
            },
        )],
    )
    .expect("floating source surface should resolve");
    assert_eq!(floating_target.root, RootKind::Floating(0));
    assert_eq!(floating_target.target, DockTarget::DockLeft);
}

#[test]
fn resolved_target_selects_floating_outer_edge_and_ignores_wrong_surface_groups() {
    let target = resolve_local_target_for_test(
        RootKind::Floating(2),
        Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 240.0,
        },
        Point { x: 3.0, y: 120.0 },
        vec![(
            SnapshotGroupKey::Authored(group("main-group")),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 240.0,
            },
        )],
    )
    .expect("outer edge should resolve before any group");
    assert_eq!(target.root, RootKind::Floating(2));
    assert_eq!(target.target, DockTarget::DockLeft);
    assert_eq!(target.group, None);
}

#[test]
fn resolved_target_uses_smallest_group_and_deterministic_edge_order() {
    let key = SnapshotGroupKey::Generated(7);
    let center = resolve_local_target_for_test(
        RootKind::Floating(0),
        Rect {
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: 400.0,
        },
        Point { x: 300.0, y: 200.0 },
        vec![
            (
                SnapshotGroupKey::Authored(group("outer")),
                Rect {
                    x: 20.0,
                    y: 20.0,
                    width: 560.0,
                    height: 360.0,
                },
            ),
            (
                key.clone(),
                Rect {
                    x: 200.0,
                    y: 120.0,
                    width: 200.0,
                    height: 160.0,
                },
            ),
        ],
    )
    .expect("center group should resolve");
    assert_eq!(center.group, Some(key.clone()));
    assert_eq!(center.target, DockTarget::Center);
    assert_eq!(center.root, RootKind::Floating(0));

    let left_tie = resolve_local_target_for_test(
        RootKind::Main,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: 400.0,
        },
        Point { x: 205.0, y: 125.0 },
        vec![(
            key,
            Rect {
                x: 200.0,
                y: 120.0,
                width: 200.0,
                height: 160.0,
            },
        )],
    )
    .expect("group edge should resolve");
    assert_eq!(left_tie.target, DockTarget::SplitLeft);
}

#[test]
fn preview_geometry_matches_all_nine_resolved_targets() {
    let surface = Rect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 240.0,
    };
    let group_bounds = Rect {
        x: 80.0,
        y: 40.0,
        width: 240.0,
        height: 160.0,
    };
    let expected = [
        (
            Point { x: 200.0, y: 120.0 },
            DockTarget::Center,
            Rect {
                x: 80.0,
                y: 40.0,
                width: 240.0,
                height: 160.0,
            },
        ),
        (
            Point { x: 82.0, y: 120.0 },
            DockTarget::SplitLeft,
            Rect {
                x: 80.0,
                y: 40.0,
                width: 120.0,
                height: 160.0,
            },
        ),
        (
            Point { x: 318.0, y: 120.0 },
            DockTarget::SplitRight,
            Rect {
                x: 200.0,
                y: 40.0,
                width: 120.0,
                height: 160.0,
            },
        ),
        (
            Point { x: 200.0, y: 42.0 },
            DockTarget::SplitTop,
            Rect {
                x: 80.0,
                y: 40.0,
                width: 240.0,
                height: 80.0,
            },
        ),
        (
            Point { x: 200.0, y: 198.0 },
            DockTarget::SplitBottom,
            Rect {
                x: 80.0,
                y: 120.0,
                width: 240.0,
                height: 80.0,
            },
        ),
        (
            Point { x: 2.0, y: 120.0 },
            DockTarget::DockLeft,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 240.0,
            },
        ),
        (
            Point { x: 398.0, y: 120.0 },
            DockTarget::DockRight,
            Rect {
                x: 300.0,
                y: 0.0,
                width: 100.0,
                height: 240.0,
            },
        ),
        (
            Point { x: 200.0, y: 2.0 },
            DockTarget::DockTop,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 60.0,
            },
        ),
        (
            Point { x: 200.0, y: 238.0 },
            DockTarget::DockBottom,
            Rect {
                x: 0.0,
                y: 180.0,
                width: 400.0,
                height: 60.0,
            },
        ),
    ];
    for (point, target_kind, expected_rect) in expected {
        let target = resolve_local_target_for_test(
            RootKind::Main,
            surface,
            point,
            vec![(SnapshotGroupKey::Generated(1), group_bounds)],
        )
        .expect("target should resolve");
        assert_eq!(target.target, target_kind);
        assert_eq!(target.preview_rect, expected_rect);
    }
}

#[test]
fn drop_preview_layer_arranges_the_rectangle_at_the_resolved_surface_rect() {
    let target = ResolvedDockTarget {
        root: RootKind::Floating(0),
        target: DockTarget::SplitRight,
        group: Some(SnapshotGroupKey::Generated(1)),
        preview_rect: Rect {
            x: 137.0,
            y: 21.0,
            width: 163.0,
            height: 119.0,
        },
    };
    let mut preview = DropPreview::new();
    preview.show(&target);
    let layer = preview.layer();
    layer.measure(Size {
        width: 400.0,
        height: 240.0,
    });
    layer.arrange(Rect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 240.0,
    });
    let rectangles = find_all::<Rectangle>(layer.as_ref());
    assert_eq!(rectangles.len(), 1);
    let rectangle = rectangles[0]
        .as_any()
        .downcast_ref::<Rectangle>()
        .expect("preview child is a Rectangle");
    assert_eq!(rectangle.visibility(), Visibility::Visible);
    assert_rect_eq(
        rectangle
            .arranged_offset()
            .zip(rectangle.arranged_width().zip(rectangle.arranged_height()))
            .map(|(offset, (width, height))| Rect {
                x: offset.x,
                y: offset.y,
                width,
                height,
            }),
        target.preview_rect,
    );

    preview.clear();
    assert_eq!(rectangle.visibility(), Visibility::Collapsed);
}

#[test]
fn floating_bounds_preserve_source_size_and_pointer_offset() {
    let source = DragSourceGeometry {
        source_root: RootKind::Main,
        source_bounds_host: Rect {
            x: 300.0,
            y: 200.0,
            width: 420.0,
            height: 260.0,
        },
        pointer_offset: Point { x: 40.0, y: 20.0 },
    };
    assert_rect_eq(
        super::docking_control::floating_bounds_for_test(
            &source,
            Point {
                x: 1000.0,
                y: 700.0,
            },
        ),
        Rect {
            x: 960.0,
            y: 680.0,
            width: 420.0,
            height: 260.0,
        },
    );
}

#[test]
fn floating_bounds_apply_only_the_minimum_size_and_reject_missing_geometry() {
    let small = DragSourceGeometry {
        source_root: RootKind::Floating(0),
        source_bounds_host: Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        },
        pointer_offset: Point { x: 40.0, y: 20.0 },
    };
    assert_rect_eq(
        super::docking_control::floating_bounds_for_test(
            &small,
            Point {
                x: 1000.0,
                y: 700.0,
            },
        ),
        Rect {
            x: 960.0,
            y: 680.0,
            width: 160.0,
            height: 120.0,
        },
    );

    let unavailable = DragSourceGeometry {
        source_root: RootKind::Main,
        source_bounds_host: Rect {
            x: 0.0,
            y: 0.0,
            width: f32::NAN,
            height: 200.0,
        },
        pointer_offset: Point { x: 1.0, y: 1.0 },
    };
    assert_eq!(
        super::docking_control::floating_bounds_for_test(
            &unavailable,
            Point { x: 100.0, y: 100.0 },
        ),
        None
    );
}

#[test]
fn floating_host_prepare_failure_keeps_the_committed_registry_empty() {
    let factory: FloatingHostFactory = Rc::new(|| {
        Err(DockLayoutError::FloatingHostUnavailable {
            reason: "test factory failure".to_owned(),
        })
    });
    let registry = FloatingHostRegistry::with_factory(factory);
    let surface = DockSurfaceView::empty_surface();
    let owner = std::rc::Weak::<DockingControl>::new();
    let error = match registry.prepare_new(
        surface,
        Rect {
            x: 10.0,
            y: 20.0,
            width: 320.0,
            height: 220.0,
        },
        &owner,
    ) {
        Ok(_) => panic!("failing factory must not prepare a host"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        DockLayoutError::FloatingHostUnavailable {
            reason: "test factory failure".to_owned()
        }
    );
    assert_eq!(registry.host_count(), 0);
}

#[test]
fn floating_prepare_failure_keeps_docking_model_and_wrapper_parent_unchanged() {
    let docking = mounted_default_docking();
    let failing_factory: FloatingHostFactory = Rc::new(|| {
        Err(DockLayoutError::FloatingHostUnavailable {
            reason: "interactive test failure".to_owned(),
        })
    });
    docking.install_floating_host_factory_for_test(failing_factory);
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let wrapper = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a stable wrapper");
    let wrapper_node: Rc<dyn UIElementExt> = wrapper.clone();
    let original_parent = wrapper_node
        .visual_parent()
        .expect("wrapper should have an owner before prepare");
    let original_model = docking.layout();
    let realization = docking.realization_for_test().unwrap();
    realization
        .borrow_mut()
        .begin_drag(&original_model, item("first"), Point { x: 100.0, y: 100.0 })
        .expect("source geometry should be available");

    let error = match realization.borrow_mut().prepare_floating_host(Rect {
        x: 900.0,
        y: 100.0,
        width: 420.0,
        height: 260.0,
    }) {
        Ok(_) => panic!("failing host factory must abort preparation"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DockLayoutError::FloatingHostUnavailable { .. }
    ));
    assert_eq!(docking.layout(), original_model);
    assert!(
        wrapper_node
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &original_parent))
    );
    assert_eq!(realization.borrow().floating_host_count_for_test(), 0);
    realization.borrow_mut().finish_drag(false);
}

#[test]
fn late_reconcile_plan_failure_preserves_the_committed_runtime_projection() {
    let docking = mounted_default_docking();
    let tab = find_all::<CustomTabView>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should be visible");
    assert!(tab.apply_template());
    let wrapper = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a stable wrapper");
    let wrapper_node: Rc<dyn UIElementExt> = wrapper.clone();
    let original_parent = wrapper_node
        .visual_parent()
        .expect("wrapper should have an owner before planning");
    let original_model = docking.layout();
    let candidate = original_model
        .with_item_closed(&item("first"))
        .expect("candidate should be valid");
    let realization = docking.realization_for_test().unwrap();
    let original_surface = realization
        .borrow()
        .surface_for_test(&RootKind::Main)
        .expect("main surface should be retained");
    let original_group = realization
        .borrow()
        .group_for_test(&SnapshotGroupKey::Authored(group("documents")))
        .expect("authored group should be retained");
    let original_root = realization
        .borrow()
        .main_runtime_root_for_test()
        .expect("main runtime root should be retained");
    let original_owner = realization
        .borrow()
        .owner_for_test(&item("first"))
        .expect("wrapper should have a committed owner");
    let original_reconciles = realization.borrow().full_reconcile_count_for_test();
    let original_surface_count = realization.borrow().surface_registry_count_for_test();

    let result = {
        let mut realization = realization.borrow_mut();
        realization.fail_after_reconcile_plan_for_test();
        realization.apply_staged(&candidate)
    };

    assert!(matches!(
        result,
        Err(DockLayoutError::InvalidSnapshot { .. })
    ));
    assert_eq!(docking.layout(), original_model);
    assert!(
        wrapper_node
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &original_parent))
    );
    let realization = docking.realization_for_test().unwrap();
    assert!(Rc::ptr_eq(
        &realization
            .borrow()
            .surface_for_test(&RootKind::Main)
            .unwrap(),
        &original_surface
    ));
    assert!(Rc::ptr_eq(
        &realization
            .borrow()
            .group_for_test(&SnapshotGroupKey::Authored(group("documents")))
            .unwrap(),
        &original_group
    ));
    assert!(Rc::ptr_eq(
        &realization.borrow().main_runtime_root_for_test().unwrap(),
        &original_root
    ));
    assert_eq!(
        realization.borrow().owner_for_test(&item("first")),
        Some(original_owner)
    );
    assert_eq!(
        realization.borrow().full_reconcile_count_for_test(),
        original_reconciles
    );
    assert_eq!(
        realization.borrow().surface_registry_count_for_test(),
        original_surface_count
    );
}

#[test]
fn late_plan_failure_does_not_touch_an_existing_floating_host() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let first_floating = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .expect("first floating root should be valid");
    docking.set_layout(first_floating);
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let host_a = hosts.borrow()[0].clone();
    let original_bounds = host_a.log.bounds.get();
    let original_content = host_a.log.content.borrow().clone();
    let original_handler = host_a.log.close_handler.borrow().clone();
    host_a.log.events.borrow_mut().clear();
    let wrapper = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime should contain a stable wrapper");
    let wrapper_node: Rc<dyn UIElementExt> = wrapper;
    let original_parent = wrapper_node
        .visual_parent()
        .expect("wrapper should be hosted");
    let original_model = docking.layout();
    let candidate = original_model
        .with_item_moved(
            &item("second"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 1400.0,
                    y: 100.0,
                    width: 360.0,
                    height: 220.0,
                },
            },
        )
        .expect("second floating root should be valid");
    let realization = docking.realization_for_test().unwrap();
    let result = {
        let mut realization = realization.borrow_mut();
        realization.fail_after_reconcile_plan_for_test();
        realization.apply_staged(&candidate)
    };

    assert!(result.is_err());
    assert_eq!(docking.layout(), original_model);
    assert!(
        wrapper_node
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &original_parent))
    );
    assert!(host_a.log.events.borrow().is_empty());
    assert_eq!(host_a.log.bounds.get(), original_bounds);
    assert!(
        host_a
            .log
            .content
            .borrow()
            .as_ref()
            .zip(original_content.as_ref())
            .is_some_and(|(current, original)| Rc::ptr_eq(current, original))
    );
    assert!(
        host_a
            .log
            .close_handler
            .borrow()
            .as_ref()
            .zip(original_handler.as_ref())
            .is_some_and(|(current, original)| Rc::ptr_eq(current, original))
    );
    assert_eq!(realization.borrow().floating_host_count_for_test(), 1);
    assert_eq!(hosts.borrow().len(), 2);
    assert_eq!(hosts.borrow()[1].log.close_count.get(), 1);
    assert!(
        !hosts.borrow()[1]
            .log
            .events
            .borrow()
            .iter()
            .any(|event| *event == "show")
    );
}

#[test]
fn floating_host_prepare_commit_show_is_staged_and_ordered() {
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    let factory = fake_factory(hosts.clone(), log.clone());
    let mut registry = FloatingHostRegistry::with_factory(factory);
    let surface = DockSurfaceView::empty_surface();
    let owner = std::rc::Weak::<DockingControl>::new();
    let prepared = registry
        .prepare_new(
            surface,
            Rect {
                x: 10.0,
                y: 20.0,
                width: 320.0,
                height: 220.0,
            },
            &owner,
        )
        .expect("fake host should prepare");
    assert_eq!(
        *log.events.borrow(),
        vec!["create", "set_bounds", "set_content", "set_close_handler"]
    );
    assert_eq!(registry.host_count(), 0);
    let id = registry.commit_prepared(prepared, 0);
    assert_eq!(registry.host_count(), 1);
    registry.show(id);
    assert_eq!(log.events.borrow().last(), Some(&"show"));
}

#[test]
fn aborting_a_prepared_floating_host_clears_handler_and_closes_it() {
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    let factory = fake_factory(hosts.clone(), log.clone());
    let registry = FloatingHostRegistry::with_factory(factory);
    let owner = std::rc::Weak::<DockingControl>::new();
    let prepared = registry
        .prepare_new(
            DockSurfaceView::empty_surface(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 220.0,
            },
            &owner,
        )
        .expect("fake host should prepare");
    prepared.abort();
    assert_eq!(registry.host_count(), 0);
    assert_eq!(log.close_count.get(), 1);
    assert_eq!(
        *log.events.borrow(),
        vec![
            "create",
            "set_bounds",
            "set_content",
            "set_close_handler",
            "clear_close_handler",
            "close",
        ]
    );
}

#[test]
fn floating_surface_runtime_keeps_its_chrome_across_reconciliation() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts, log));
    let floating = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .expect("floating candidate");
    docking.set_layout(floating);

    let realization = docking
        .realization_for_test()
        .expect("mounted docking has a realization");
    let first_surfaces = {
        let realization = realization.borrow();
        (
            realization.surface_for_test(&RootKind::Main).unwrap(),
            realization
                .surface_for_test(&RootKind::Floating(0))
                .unwrap(),
            realization
                .surface_chrome_for_test(&RootKind::Floating(0))
                .unwrap(),
        )
    };
    realization
        .borrow_mut()
        .reconcile_for_test(&docking.layout())
        .expect("equal model should reconcile");
    let second_surfaces = {
        let realization = realization.borrow();
        (
            realization.surface_for_test(&RootKind::Main).unwrap(),
            realization
                .surface_for_test(&RootKind::Floating(0))
                .unwrap(),
            realization
                .surface_chrome_for_test(&RootKind::Floating(0))
                .unwrap(),
        )
    };
    assert!(Rc::ptr_eq(&first_surfaces.0, &second_surfaces.0));
    assert!(Rc::ptr_eq(&first_surfaces.1, &second_surfaces.1));
    assert!(Rc::ptr_eq(&first_surfaces.2.0, &second_surfaces.2.0));
    assert!(Rc::ptr_eq(&first_surfaces.2.1, &second_surfaces.2.1));

    let target = ResolvedDockTarget {
        root: RootKind::Floating(0),
        target: DockTarget::DockLeft,
        group: None,
        preview_rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 105.0,
            height: 260.0,
        },
    };
    realization.borrow_mut().show_preview_for_test(target);
    assert_eq!(realization.borrow().preview_for_test(&RootKind::Main), None);
    assert_eq!(
        realization
            .borrow()
            .preview_for_test(&RootKind::Floating(0))
            .map(|(target, _)| target),
        Some(DockTarget::DockLeft)
    );
    realization.borrow_mut().clear_drag_target();
    assert_eq!(
        realization
            .borrow()
            .preview_for_test(&RootKind::Floating(0)),
        None
    );
}

#[test]
fn floating_surface_renders_and_presents_its_own_auto_hide_entries() {
    let (main_item, _) = authored_item("main", "Main", true);
    let (stay_item, _) = authored_item("stay", "Stay", true);
    let (hidden_item, hidden_page) = authored_item("hidden", "Hidden", true);
    let docking = mounted_docking_with_items(vec![main_item, stay_item, hidden_item]);
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts, log));
    docking.set_layout(floating_auto_hide_snapshot_model());

    let realization = docking
        .realization_for_test()
        .expect("mounted docking has a realization");
    let auto_hide_visual = realization
        .borrow()
        .surface_chrome_for_test(&RootKind::Floating(0))
        .expect("floating surface has retained chrome")
        .0;
    let auto_hide = auto_hide_visual
        .as_any()
        .downcast_ref::<Grid>()
        .expect("auto-hide chrome uses a Grid root");
    let left_strip = auto_hide.children().to_vec()[0].clone();
    assert_eq!(left_strip.visual_children().len(), 1);

    realization
        .borrow_mut()
        .open_auto_hide_on(RootKind::Floating(0), item("hidden"));
    assert_eq!(
        realization
            .borrow()
            .open_auto_hide_item_on(&RootKind::Floating(0)),
        Some(item("hidden"))
    );
    assert_eq!(
        realization.borrow().open_auto_hide_item_on(&RootKind::Main),
        None
    );
    assert!(
        hidden_page
            .visual_parent()
            .is_some_and(|parent| parent.visual_parent().is_some())
    );
}

#[test]
fn floating_host_ids_follow_their_surfaces_when_an_earlier_root_is_removed() {
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let mut registry = FloatingHostRegistry::with_factory(individual_fake_factory(hosts.clone()));
    let owner = std::rc::Weak::<DockingControl>::new();
    let surface_a = DockSurfaceView::empty_surface();
    let surface_b = DockSurfaceView::empty_surface();
    registry
        .sync(
            &[
                (
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 320.0,
                        height: 220.0,
                    },
                    surface_a,
                ),
                (
                    Rect {
                        x: 400.0,
                        y: 0.0,
                        width: 320.0,
                        height: 220.0,
                    },
                    surface_b.clone(),
                ),
            ],
            &owner,
        )
        .expect("fake hosts should synchronize");
    let ids = registry.host_ids();
    assert_eq!(ids.len(), 2);
    registry
        .sync(
            &[(
                Rect {
                    x: 400.0,
                    y: 0.0,
                    width: 320.0,
                    height: 220.0,
                },
                surface_b,
            )],
            &owner,
        )
        .expect("remaining host should synchronize");
    assert_eq!(registry.root_index_for_host(ids[1]), Some(0));
    assert_eq!(hosts.borrow()[0].log.close_count.get(), 1);
    assert_eq!(hosts.borrow()[1].log.close_count.get(), 0);
}

#[test]
fn floating_host_sync_preparation_leaves_existing_hosts_untouched() {
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let mut registry = FloatingHostRegistry::with_factory(individual_fake_factory(hosts.clone()));
    let owner = std::rc::Weak::<DockingControl>::new();
    let surface_a = DockSurfaceView::empty_surface();
    let surface_b = DockSurfaceView::empty_surface();
    registry
        .sync(
            &[(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 320.0,
                    height: 220.0,
                },
                surface_a.clone(),
            )],
            &owner,
        )
        .expect("initial host should synchronize");
    let host_a = hosts.borrow()[0].clone();
    let host_a_id = registry.host_ids()[0];
    let original_bounds = host_a.log.bounds.get();
    let original_content = host_a.log.content.borrow().clone();
    let original_handler = host_a.log.close_handler.borrow().clone();
    host_a.log.events.borrow_mut().clear();

    let prepared = registry
        .prepare_sync(
            &[
                (
                    Rect {
                        x: 40.0,
                        y: 50.0,
                        width: 420.0,
                        height: 260.0,
                    },
                    surface_a,
                ),
                (
                    Rect {
                        x: 600.0,
                        y: 80.0,
                        width: 300.0,
                        height: 200.0,
                    },
                    surface_b,
                ),
            ],
            &owner,
        )
        .expect("host sync should prepare");

    assert!(host_a.log.events.borrow().is_empty());
    assert_eq!(host_a.log.bounds.get(), original_bounds);
    assert!(
        host_a
            .log
            .content
            .borrow()
            .as_ref()
            .zip(original_content.as_ref())
            .is_some_and(|(current, original)| Rc::ptr_eq(current, original))
    );
    assert!(
        host_a
            .log
            .close_handler
            .borrow()
            .as_ref()
            .zip(original_handler.as_ref())
            .is_some_and(|(current, original)| Rc::ptr_eq(current, original))
    );
    assert_eq!(registry.root_index_for_host(host_a_id), Some(0));
    assert_eq!(registry.host_count(), 1);

    let staged = hosts.borrow()[1].clone();
    assert_eq!(
        *staged.log.events.borrow(),
        vec!["set_bounds", "set_content", "set_close_handler"]
    );
    prepared.abort();
    assert_eq!(staged.log.close_count.get(), 1);
    assert!(
        !staged
            .log
            .events
            .borrow()
            .iter()
            .any(|event| *event == "show")
    );
    assert_eq!(registry.host_count(), 1);
    assert_eq!(registry.root_index_for_host(host_a_id), Some(0));
}

#[test]
fn floating_native_close_uses_stable_host_identity_after_root_reindex() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let first_floating = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .unwrap();
    let two_floating = first_floating
        .with_item_moved(
            &item("second"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 1400.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .unwrap();
    docking.set_layout(two_floating);
    assert_eq!(hosts.borrow().len(), 2);

    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    let remaining_after_first_close = docking.layout().with_item_closed(&item("first")).unwrap();
    docking.set_layout(remaining_after_first_close);
    assert_eq!(hosts.borrow()[0].log.close_count.get(), 1);

    assert!(hosts.borrow()[1].log.invoke_close());
    assert!(docking.layout().is_item_closed(&item("second")));
    assert_eq!(changes.get(), 1);
    assert_eq!(hosts.borrow()[1].log.close_count.get(), 1);
}

#[test]
fn floating_native_close_veto_keeps_window_and_model_when_any_item_is_not_closeable() {
    let (allowed, _) = authored_item("allowed", "Allowed", true);
    let (blocked, _) = authored_item("blocked", "Blocked", false);
    let docking = mounted_docking_with_items(vec![allowed, blocked]);
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let floating = floating_model_with_items(
        &docking.layout(),
        &["allowed", "blocked"],
        Rect {
            x: 900.0,
            y: 100.0,
            width: 420.0,
            height: 260.0,
        },
    );
    docking.set_layout(floating);
    let committed = docking.layout();
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    let host = hosts.borrow()[0].clone();

    assert!(host.log.invoke_close());
    assert_eq!(docking.layout(), committed);
    assert_eq!(changes.get(), 0);
    assert_eq!(host.log.close_count.get(), 0);
    assert!(host.log.close_handler.borrow().is_some());
    assert_eq!(
        docking
            .realization_for_test()
            .unwrap()
            .borrow()
            .floating_host_count_for_test(),
        1
    );
}

#[test]
fn floating_native_close_commits_all_items_once_and_closes_one_host() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let floating = floating_model_with_items(
        &docking.layout(),
        &["first", "second"],
        Rect {
            x: 900.0,
            y: 100.0,
            width: 420.0,
            height: 260.0,
        },
    );
    docking.set_layout(floating);
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    let host = hosts.borrow()[0].clone();

    assert!(host.log.invoke_close());
    assert!(docking.layout().is_item_closed(&item("first")));
    assert!(docking.layout().is_item_closed(&item("second")));
    assert_eq!(changes.get(), 1);
    assert_eq!(host.log.close_count.get(), 1);
    assert!(host.log.close_handler.borrow().is_none());
    assert_eq!(
        docking
            .realization_for_test()
            .unwrap()
            .borrow()
            .floating_host_count_for_test(),
        0
    );
}

#[test]
fn redocking_the_last_floating_item_closes_its_host_once() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let floating = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .expect("item should float");
    docking.set_layout(floating);
    let host = hosts.borrow()[0].clone();
    let redocked = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::RootEdge {
                side: DockSide::Left,
                weight: 1.0,
            },
        )
        .expect("item should redock to the main root");
    docking.set_layout(redocked);

    assert!(docking.layout().snapshot().floating_roots.is_empty());
    assert_eq!(host.log.close_count.get(), 1);
    assert!(host.log.close_handler.borrow().is_none());
    assert_eq!(
        docking
            .realization_for_test()
            .unwrap()
            .borrow()
            .floating_host_count_for_test(),
        0
    );
}

#[test]
fn drag_preview_commit_and_capture_loss_are_transactional() {
    let model = default_model();
    let mut drag = test_drag(&model, item("first"));
    let target = resolved_target(
        DockTarget::Center,
        Some(SnapshotGroupKey::Authored(group("tools"))),
    );
    drag.preview(&target, 1.0).unwrap();
    let preview = model
        .with_item_moved(
            &item("first"),
            DockPlacement::Group {
                group: group("tools"),
                index: None,
            },
        )
        .unwrap();
    assert_ne!(preview, model);
    assert_eq!(drag.capture_lost(), model);
    assert!(drag.commit().is_none());

    let mut committed = test_drag(&model, item("first"));
    committed
        .preview(&resolved_target(DockTarget::DockRight, None), 1.0)
        .unwrap();
    assert!(committed.commit().is_some());
    assert!(committed.commit().is_none());
}

#[test]
fn drag_target_conversion_covers_center_split_and_outer_edges() {
    let model = default_model();
    let group = Some(SnapshotGroupKey::Authored(group("tools")));
    for target in [
        DockTarget::Center,
        DockTarget::SplitLeft,
        DockTarget::SplitTop,
        DockTarget::SplitRight,
        DockTarget::SplitBottom,
    ] {
        let mut drag = test_drag(&model, item("first"));
        assert!(
            drag.preview(&resolved_target(target, group.clone()), 1.0)
                .is_ok()
        );
    }
    for target in [
        DockTarget::DockLeft,
        DockTarget::DockTop,
        DockTarget::DockRight,
        DockTarget::DockBottom,
    ] {
        let mut drag = test_drag(&model, item("first"));
        assert!(drag.preview(&resolved_target(target, None), 1.0).is_ok());
    }
}

#[test]
fn drag_preview_can_target_a_generated_runtime_group() {
    let model = default_model()
        .with_item_moved(
            &item("first"),
            DockPlacement::RootEdge {
                side: DockSide::Left,
                weight: 1.0,
            },
        )
        .unwrap();
    let SnapshotNode::Split { children, .. } = model.snapshot().main_root.unwrap() else {
        panic!("expected generated root split");
    };
    let SnapshotNode::Group {
        group: SnapshotGroupKey::Generated(generated),
        ..
    } = &children[0].node
    else {
        panic!("expected generated leading group");
    };

    let mut drag = test_drag(&model, item("second"));
    drag.preview(
        &resolved_target(
            DockTarget::Center,
            Some(SnapshotGroupKey::Generated(*generated)),
        ),
        1.0,
    )
    .unwrap();
    let preview = model
        .with_item_moved_internal(
            &item("second"),
            super::model::InternalDockPlacement::Group {
                group: InternalDockGroupKey::Generated(*generated),
                index: None,
            },
        )
        .unwrap();
    let SnapshotNode::Split { children, .. } = preview.snapshot().main_root.unwrap() else {
        panic!("expected root split");
    };
    let SnapshotNode::Group { items, .. } = &children[0].node else {
        panic!("expected generated group");
    };
    assert!(items.contains(&item("second")));
}

#[test]
fn auto_hide_overlay_keeps_one_open_item_and_preview_clears() {
    let mut overlay = AutoHideOverlay::default();
    assert_eq!(overlay.open(item("a")), None);
    assert_eq!(overlay.current(), Some(&item("a")));
    assert_eq!(overlay.open(item("b")), Some(item("a")));
    assert_eq!(overlay.close(), Some(item("b")));
    assert_eq!(overlay.current(), None);

    let mut preview = DropPreview::new();
    preview.show(&resolved_target(DockTarget::SplitBottom, None));
    assert_eq!(preview.target(), Some(DockTarget::SplitBottom));
    preview.clear();
    assert_eq!(preview.target(), None);
}

#[test]
fn auto_hide_activation_keeps_entries_and_switches_the_single_open_overlay() {
    let model = default_model();
    let first = model
        .with_item_moved(
            &item("first"),
            DockPlacement::AutoHide {
                side: DockSide::Left,
            },
        )
        .unwrap();
    let both = first
        .with_item_moved(
            &item("second"),
            DockPlacement::AutoHide {
                side: DockSide::Left,
            },
        )
        .unwrap();
    let open_first = both.with_item_activated(&item("first")).unwrap();
    let open_second = open_first.with_item_activated(&item("second")).unwrap();
    let entries = &open_second.snapshot().auto_hide[DockSide::Left.index()];
    assert_eq!(entries.len(), 2);
    assert!(!entries[0].open);
    assert!(entries[1].open);
}

#[test]
fn restored_generated_group_allocator_advances_past_restored_ids() {
    let snapshot = DockLayoutSnapshot {
        version: DockLayoutSnapshot::VERSION,
        main_root: Some(SnapshotNode::Split {
            orientation: SnapshotOrientation::Horizontal,
            children: vec![SnapshotWeightedNode {
                weight: 1.0,
                node: SnapshotNode::Group {
                    group: SnapshotGroupKey::Generated(42),
                    items: vec![item("generated-item")],
                    selected: Some(item("generated-item")),
                },
            }],
        }),
        floating_roots: Vec::new(),
        auto_hide: std::array::from_fn(|_| Vec::new()),
        closed: Vec::new(),
        next_generated_group_id: 1,
        active_item: None,
    };
    let model = DockLayoutModel::from_snapshot(snapshot).unwrap();
    let moved = model
        .with_item_moved(
            &item("generated-item"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
            },
        )
        .unwrap();
    let json = serde_json::to_string(&moved.snapshot()).unwrap();
    assert!(json.contains(r#""Generated":43"#));
}

#[test]
fn removed_authored_groups_relocate_live_and_return_state_to_current_defaults() {
    let replacement_root = Node::Group {
        group: InternalDockGroupKey::Authored(group("replacement")),
        items: vec![item("first"), item("second"), item("third")],
        selected: Some(item("first")),
    };
    let replacement = DefaultDockDefinition::new(Some(replacement_root));

    let repaired = default_model().attach_default(replacement.clone());
    let repaired_json = serde_json::to_string(&repaired.snapshot()).unwrap();
    assert!(repaired_json.contains("replacement"));
    assert!(!repaired_json.contains("documents"));
    assert!(!repaired_json.contains("tools"));
    assert!(repaired.contains_item(&item("first")));
    assert!(repaired.contains_item(&item("third")));

    let closed = default_model().with_item_closed(&item("first")).unwrap();
    let repaired_closed = closed.attach_default(replacement);
    let reopened = repaired_closed.with_item_reopened(&item("first")).unwrap();
    let reopened_json = serde_json::to_string(&reopened.snapshot()).unwrap();
    assert!(reopened_json.contains("replacement"));
    assert!(!reopened_json.contains("documents"));
}

#[test]
fn adjacent_split_resize_changes_only_the_two_boundary_weights() {
    let root = Node::Split {
        orientation: Orientation::Horizontal,
        children: vec![
            WeightedNode {
                weight: 1.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("one")),
                    items: vec![item("one")],
                    selected: Some(item("one")),
                },
            },
            WeightedNode {
                weight: 2.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("two")),
                    items: vec![item("two")],
                    selected: Some(item("two")),
                },
            },
            WeightedNode {
                weight: 3.0,
                node: Node::Group {
                    group: InternalDockGroupKey::Authored(group("three")),
                    items: vec![item("three")],
                    selected: Some(item("three")),
                },
            },
        ],
    };
    let model = DockLayoutModel::from_default(DefaultDockDefinition::new(Some(root)));
    let address = SplitAddress {
        root: RootKind::Main,
        path: Vec::new(),
    };
    let resized = model
        .with_adjacent_split_weights(&address, 1, 30.0, 300.0)
        .expect("split boundary");
    let snapshot = resized.snapshot();
    let SnapshotNode::Split { children, .. } = snapshot.main_root.unwrap() else {
        panic!("expected split");
    };
    assert!((children[0].weight - (1.0 / 6.0)).abs() < 0.001);
    assert!((children[1].weight - (5.0 / 12.0)).abs() < 0.001);
    assert!((children[2].weight - (5.0 / 12.0)).abs() < 0.001);
}

#[test]
fn authored_docking_declaration_is_collapsed_and_runtime_wrapper_is_visible() {
    let page = super::core::ui::TextBlock::new();
    page.set_text("stable page");
    let dock_item = DockItem::new_item();
    dock_item.set_id(item("page"));
    dock_item.set_title("Page".to_string());
    dock_item.set_content(page.clone());

    let dock_group = DockGroup::new_group();
    dock_group.set_id(group("main"));
    dock_group.set_children(vec![dock_item]);

    let docking = DockingControl::__new_unmounted();
    docking.set_content(dock_group);
    docking.mount(application_environment());
    assert!(docking.apply_template());

    let presenters = find_all::<super::core::ui::ContentPresenter>(docking.as_ref());
    assert!(
        presenters
            .iter()
            .any(|presenter| presenter.visibility() == Visibility::Collapsed)
    );
    let tabs = find_all::<CustomTabView>(docking.as_ref());
    assert_eq!(tabs.len(), 1);
    let tab = tabs[0]
        .as_any()
        .downcast_ref::<CustomTabView>()
        .expect("runtime group is a CustomTabView");
    assert!(tab.apply_template());
    assert!(
        page.visual_parent()
            .is_some_and(|parent| parent.visual_parent().is_some())
    );
    assert!(
        page.as_ui_element()
            .parent
            .borrow()
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
            .is_some_and(|parent| parent.as_any().is::<CustomTabViewItem>())
    );
}

#[test]
fn authored_show_when_empty_group_keeps_chrome_and_non_interactive_drop_hint() {
    let (dock_item, _) = authored_item("empty-item", "Empty item", true);
    let dock_group = DockGroup::new_group();
    dock_group.set_id(group("empty-group"));
    dock_group.set_show_when_empty(true);
    dock_group.set_children(vec![dock_item]);

    let docking = DockingControl::__new_unmounted();
    docking.set_content(dock_group);
    docking.mount(application_environment());
    assert!(docking.apply_template());

    let cleared = docking
        .layout()
        .with_cleared_layout()
        .expect("clear should preserve the authored default metadata");
    docking.set_layout(cleared);

    let hints = find_all::<TextBlock>(docking.as_ref())
        .into_iter()
        .filter(|hint| {
            hint.as_any()
                .downcast_ref::<TextBlock>()
                .is_some_and(|hint| hint.text.borrow().as_str() == "Drop here")
        })
        .collect::<Vec<_>>();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].visibility(), Visibility::Visible);
    assert!(!hints[0].hit_test_visible());
    assert!(
        find_all::<CustomTabView>(docking.as_ref())
            .into_iter()
            .any(|view| view.visibility() == Visibility::Visible)
    );
}

#[test]
fn tab_context_indexed_close_actions_commit_once_and_preserve_protected_items() {
    let docking = mounted_default_docking();
    let callback_count = Rc::new(Cell::new(0));
    let callback_count_for_handler = callback_count.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        callback_count_for_handler.set(callback_count_for_handler.get() + 1);
    }));

    docking.handle_tab_context_action(item("second"), DockTabContextAction::CloseOthers);
    let after_others = docking.layout();
    assert!(after_others.is_item_closed(&item("first")));
    assert!(!after_others.is_item_closed(&item("third")));
    assert!(!after_others.is_item_closed(&item("second")));
    assert_eq!(callback_count.get(), 1);

    let docking = mounted_default_docking();
    docking.handle_tab_context_action(item("second"), DockTabContextAction::CloseTabsToLeft);
    let after_left = docking.layout();
    assert!(after_left.is_item_closed(&item("first")));
    assert!(!after_left.is_item_closed(&item("second")));
    assert!(!after_left.is_item_closed(&item("third")));

    let docking = mounted_default_docking();
    docking.handle_tab_context_action(item("first"), DockTabContextAction::CloseTabsToRight);
    let after_right = docking.layout();
    assert!(!after_right.is_item_closed(&item("first")));
    assert!(after_right.is_item_closed(&item("second")));
    assert!(!after_right.is_item_closed(&item("third")));
}

#[test]
fn three_pane_runtime_split_realizes_two_splitters() {
    let groups = (0..3)
        .map(|index| {
            let dock_item = DockItem::new_item();
            dock_item.set_id(item(&format!("split-item-{index}")));
            dock_item.set_title(format!("Split item {index}"));
            dock_item.set_content(super::core::ui::TextBlock::new());

            let dock_group = DockGroup::new_group();
            dock_group.set_id(group(&format!("split-group-{index}")));
            dock_group.set_children(vec![dock_item]);
            dock_group
        })
        .collect::<Vec<_>>();
    let split = DockSplitPanel::new_panel();
    split.set_children(
        groups
            .into_iter()
            .map(|group| group as std::rc::Rc<dyn UIElementExt>)
            .collect(),
    );

    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());

    let splitters = find_all::<CustomSplitter>(docking.as_ref());
    assert_eq!(splitters.len(), 2);
}

#[test]
fn retained_group_callbacks_commit_selection_and_close_once() {
    let first_page = super::core::ui::TextBlock::new();
    let second_page = super::core::ui::TextBlock::new();
    let first = DockItem::new_item();
    first.set_id(item("first"));
    first.set_title("First".to_string());
    first.set_content(first_page);
    let second = DockItem::new_item();
    second.set_id(item("second"));
    second.set_title("Second".to_string());
    second.set_content(second_page);

    let dock_group = DockGroup::new_group();
    dock_group.set_id(group("main"));
    dock_group.set_children(vec![first, second]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(dock_group);
    docking.mount(application_environment());
    assert!(docking.apply_template());

    let tabs = find_all::<CustomTabView>(docking.as_ref());
    assert_eq!(tabs.len(), 1);
    let tab = tabs[0]
        .as_any()
        .downcast_ref::<CustomTabView>()
        .expect("runtime group is a CustomTabView");
    assert!(tab.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let wrappers = find_all::<CustomTabViewItem>(docking.as_ref());
    assert_eq!(wrappers.len(), 2);
    let first_wrapper_parent = (wrappers[0].clone() as Rc<dyn UIElementExt>)
        .visual_parent()
        .expect("first wrapper should be hosted");
    let second_wrapper_parent = (wrappers[1].clone() as Rc<dyn UIElementExt>)
        .visual_parent()
        .expect("second wrapper should be hosted");
    let changes = std::rc::Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));

    let full_reconciles_before_selection = docking
        .realization_for_test()
        .expect("mounted docking has a realization")
        .borrow()
        .full_reconcile_count_for_test();
    let second_bounds =
        SurfaceRegistry::bounds_in_host_root(&(wrappers[1].clone() as Rc<dyn UIElementExt>))
            .expect("second tab should be arranged");
    let second_center = Point {
        x: second_bounds.x + second_bounds.width * 0.5,
        y: second_bounds.y + second_bounds.height * 0.5,
    };
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(
            RawPointerEventKind::Pressed(MouseButton::Left),
            second_center,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(
            RawPointerEventKind::Released(MouseButton::Left),
            second_center,
        ),
    );
    assert!(docking.layout().is_item_active(&item("second")));
    assert_eq!(changes.get(), 1);
    assert_eq!(
        docking
            .realization_for_test()
            .unwrap()
            .borrow()
            .full_reconcile_count_for_test(),
        full_reconciles_before_selection,
        "selection-only changes must not reconcile the retained Dock runtime"
    );
    assert!(
        wrappers[0]
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &first_wrapper_parent))
    );
    assert!(
        wrappers[1]
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &second_wrapper_parent))
    );

    assert!(tab.request_close(1));
    assert!(docking.layout().is_item_closed(&item("second")));
    assert_eq!(changes.get(), 2);
}

#[test]
fn actual_tab_pointer_path_starts_and_cancels_docking_drag_after_four_pixels() {
    let docking = mounted_default_docking();
    let original = docking.layout();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let tab_item = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a tab item");
    let tab_node: Rc<dyn UIElementExt> = tab_item;
    let bounds = SurfaceRegistry::bounds_in_host_root(&tab_node)
        .expect("arranged tab item should have host-root bounds");
    assert!(bounds.width > 0.0 && bounds.height > 0.0);
    let start = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), start),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(
            RawPointerEventKind::Moved,
            Point {
                x: start.x + 6.0,
                y: start.y,
            },
        ),
    );
    let realization = docking
        .realization_for_test()
        .expect("mounted docking has a realization");
    assert!(realization.borrow().active_drag_for_test());
    assert_eq!(docking.layout(), original);

    assert!(dispatcher.cancel());
    assert!(!realization.borrow().active_drag_for_test());
    assert_eq!(realization.borrow().preview_for_test(&RootKind::Main), None);
    assert_eq!(docking.layout(), original);
}

#[test]
fn actual_tab_pointer_path_commits_a_root_edge_drop_once() {
    let docking = mounted_default_docking();
    let original = docking.layout();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let tab_item = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a tab item");
    let tab_node: Rc<dyn UIElementExt> = tab_item;
    let bounds = SurfaceRegistry::bounds_in_host_root(&tab_node)
        .expect("arranged tab item should have host-root bounds");
    let start = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), start),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(
            RawPointerEventKind::Moved,
            Point {
                x: start.x + 6.0,
                y: start.y,
            },
        ),
    );
    let realization = docking.realization_for_test().unwrap();
    assert!(realization.borrow().active_drag_for_test());
    let main_surface = realization
        .borrow()
        .surface_for_test(&RootKind::Main)
        .expect("main surface");
    let main_surface_node: Rc<dyn UIElementExt> = main_surface;
    let main_surface_bounds = SurfaceRegistry::bounds_in_host_root(&main_surface_node)
        .expect("main surface should have arranged bounds");
    let release_position = Point {
        x: main_surface_bounds.x + 2.0,
        y: main_surface_bounds.y + main_surface_bounds.height * 0.5,
    };
    let release_target = realization
        .borrow()
        .target_for_drop(None, release_position)
        .expect("main outer edge should resolve during the actual drag");
    assert_eq!(release_target.root, RootKind::Main);
    assert_eq!(release_target.target, DockTarget::DockLeft);
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(
            RawPointerEventKind::Released(MouseButton::Left),
            release_position,
        ),
    );

    assert_ne!(docking.layout(), original);
    assert_eq!(changes.get(), 1);
    assert!(
        !docking
            .realization_for_test()
            .unwrap()
            .borrow()
            .active_drag_for_test()
    );
    assert!(matches!(
        docking.layout().snapshot().main_root,
        Some(SnapshotNode::Split { .. })
    ));
}

#[test]
fn actual_tab_float_prepare_failure_leaves_model_and_wrapper_parent_unchanged() {
    let docking = mounted_default_docking();
    let failing_factory: FloatingHostFactory = Rc::new(|| {
        Err(DockLayoutError::FloatingHostUnavailable {
            reason: "actual drag host failure".to_owned(),
        })
    });
    docking.install_floating_host_factory_for_test(failing_factory);
    let root: Rc<dyn UIElementExt> = docking.clone();
    root.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 0.0, y: 0.0 },
    })));
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let tab_item = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a tab item");
    let tab_node: Rc<dyn UIElementExt> = tab_item;
    let tab_bounds = SurfaceRegistry::bounds_in_host_root(&tab_node).unwrap();
    let start = Point {
        x: tab_bounds.x + tab_bounds.width * 0.5,
        y: tab_bounds.y + tab_bounds.height * 0.5,
    };
    let wrapper_parent = tab_node.visual_parent().expect("tab should have an owner");
    let original = docking.layout();
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event_with_screen(
            RawPointerEventKind::Pressed(MouseButton::Left),
            start,
            start,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event_with_screen(
            RawPointerEventKind::Moved,
            Point {
                x: start.x + 6.0,
                y: start.y,
            },
            Point {
                x: start.x + 6.0,
                y: start.y,
            },
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event_with_screen(
            RawPointerEventKind::Released(MouseButton::Left),
            Point {
                x: 1000.0,
                y: 700.0,
            },
            Point {
                x: 1000.0,
                y: 700.0,
            },
        ),
    );

    assert_eq!(docking.layout(), original);
    assert_eq!(changes.get(), 0);
    assert!(
        tab_node
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &wrapper_parent))
    );
    assert_eq!(
        docking
            .realization_for_test()
            .unwrap()
            .borrow()
            .floating_host_count_for_test(),
        0
    );
}

#[test]
fn actual_tab_float_late_plan_failure_aborts_prepared_host_without_commit() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts.clone(), log.clone()));
    let root: Rc<dyn UIElementExt> = docking.clone();
    root.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 0.0, y: 0.0 },
    })));
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let tab_item = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a tab item");
    let tab_node: Rc<dyn UIElementExt> = tab_item;
    let source_bounds = SurfaceRegistry::bounds_in_host_root(&tab_node).unwrap();
    let start = Point {
        x: source_bounds.x + source_bounds.width * 0.5,
        y: source_bounds.y + source_bounds.height * 0.5,
    };
    let wrapper_parent = tab_node.visual_parent().expect("tab should have an owner");
    let original = docking.layout();
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));

    let realization = docking.realization_for_test().unwrap();
    realization
        .borrow_mut()
        .begin_drag(&original, item("first"), start)
        .expect("source geometry should be available");
    realization
        .borrow_mut()
        .fail_after_reconcile_plan_for_test();
    docking.handle_tab_drag_completed(
        SnapshotGroupKey::Authored(group("documents")),
        TabDragCompletedEventArgs {
            index: 0,
            position: Point {
                x: 1000.0,
                y: 700.0,
            },
            screen_position: Some(Point {
                x: 1000.0,
                y: 700.0,
            }),
            canceled: false,
        },
    );

    assert_eq!(docking.layout(), original);
    assert_eq!(changes.get(), 0);
    assert!(
        tab_node
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &wrapper_parent))
    );
    assert_eq!(realization.borrow().floating_host_count_for_test(), 0);
    assert_eq!(log.close_count.get(), 1);
    assert_eq!(
        *log.events.borrow(),
        vec![
            "create",
            "set_bounds",
            "set_content",
            "set_close_handler",
            "clear_close_handler",
            "close",
        ]
    );
}

#[test]
fn actual_tab_float_uses_source_geometry_and_shows_after_commit() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts.clone(), log.clone()));
    let root: Rc<dyn UIElementExt> = docking.clone();
    root.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 0.0, y: 0.0 },
    })));
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let tab_item = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a tab item");
    let tab_node: Rc<dyn UIElementExt> = tab_item;
    let tab_bounds = SurfaceRegistry::bounds_in_host_root(&tab_node).unwrap();
    let start = Point {
        x: tab_bounds.x + tab_bounds.width * 0.5,
        y: tab_bounds.y + tab_bounds.height * 0.5,
    };
    let source_bounds = find_all::<CustomTabView>(docking.as_ref())
        .iter()
        .filter_map(|group| {
            let node: Rc<dyn UIElementExt> = group.clone();
            SurfaceRegistry::bounds_in_host_root(&node)
        })
        .find(|bounds| {
            start.x >= bounds.x
                && start.y >= bounds.y
                && start.x <= bounds.x + bounds.width
                && start.y <= bounds.y + bounds.height
        })
        .expect("source group should contain the pressed tab");
    let expected = Rect {
        x: 1000.0 - (start.x - source_bounds.x),
        y: 700.0 - (start.y - source_bounds.y),
        width: source_bounds.width.max(160.0),
        height: source_bounds.height.max(120.0),
    };
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    let callback_model = Rc::new(RefCell::new(None));
    let callback_model_for_callback = callback_model.clone();
    let log_for_callback = log.clone();
    docking.set_on_layout_change(Box::new(move |model| {
        changes_for_callback.set(changes_for_callback.get() + 1);
        *callback_model_for_callback.borrow_mut() = Some(model);
        log_for_callback.events.borrow_mut().push("layout_callback");
    }));
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event_with_screen(
            RawPointerEventKind::Pressed(MouseButton::Left),
            start,
            start,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event_with_screen(
            RawPointerEventKind::Moved,
            Point {
                x: start.x + 6.0,
                y: start.y,
            },
            Point {
                x: start.x + 6.0,
                y: start.y,
            },
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event_with_screen(
            RawPointerEventKind::Released(MouseButton::Left),
            Point {
                x: 1000.0,
                y: 700.0,
            },
            Point {
                x: 1000.0,
                y: 700.0,
            },
        ),
    );

    let host = hosts.borrow()[0].clone();
    assert_eq!(docking.layout().snapshot().floating_roots.len(), 1);
    assert_eq!(changes.get(), 1);
    assert_eq!(
        callback_model
            .borrow()
            .as_ref()
            .map(|model| model.snapshot().floating_roots.len()),
        Some(1)
    );
    assert_eq!(host.log.bounds.get(), Some(expected));
    assert_eq!(
        *host.log.events.borrow(),
        vec![
            "create",
            "set_bounds",
            "set_content",
            "set_close_handler",
            "layout_callback",
            "show"
        ]
    );
}

#[test]
fn user_callback_unmount_aborts_staged_host_after_runtime_commit() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts, log.clone()));
    let root: Rc<dyn UIElementExt> = docking.clone();
    root.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 0.0, y: 0.0 },
    })));
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let tab_item = find_all::<CustomTabViewItem>(docking.as_ref())
        .into_iter()
        .next()
        .expect("runtime group should contain a tab item");
    let tab_node: Rc<dyn UIElementExt> = tab_item;
    let source_bounds = SurfaceRegistry::bounds_in_host_root(&tab_node).unwrap();
    let start = Point {
        x: source_bounds.x + source_bounds.width * 0.5,
        y: source_bounds.y + source_bounds.height * 0.5,
    };
    let original = docking.layout();
    let weak_root = Rc::downgrade(&root);
    let log_for_callback = log.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        log_for_callback.events.borrow_mut().push("layout_callback");
        if let Some(root) = weak_root.upgrade() {
            unmount_subtree(&root);
        }
    }));

    let realization = docking.realization_for_test().unwrap();
    realization
        .borrow_mut()
        .begin_drag(&original, item("first"), start)
        .expect("source geometry should be available");
    docking.handle_tab_drag_completed(
        SnapshotGroupKey::Authored(group("documents")),
        TabDragCompletedEventArgs {
            index: 0,
            position: Point {
                x: 1000.0,
                y: 700.0,
            },
            screen_position: Some(Point {
                x: 1000.0,
                y: 700.0,
            }),
            canceled: false,
        },
    );

    assert_eq!(
        *log.events.borrow(),
        vec![
            "create",
            "set_bounds",
            "set_content",
            "set_close_handler",
            "layout_callback",
            "clear_close_handler",
            "close",
        ]
    );
    assert_eq!(log.close_count.get(), 1);
    assert_eq!(realization.borrow().floating_host_count_for_test(), 0);
    assert!(!log.events.borrow().iter().any(|event| *event == "show"));
}

#[test]
fn screen_drop_on_floating_outer_edge_resolves_the_floating_root() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts, log));
    let model = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .unwrap();
    docking.set_layout(model);
    let root: Rc<dyn UIElementExt> = docking.clone();
    root.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 0.0, y: 0.0 },
    })));
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let realization = docking.realization_for_test().unwrap();
    let floating_surface = realization
        .borrow()
        .surface_for_test(&RootKind::Floating(0))
        .unwrap();
    floating_surface.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 900.0, y: 100.0 },
    })));
    floating_surface.arrange(Rect {
        x: 0.0,
        y: 0.0,
        width: 420.0,
        height: 260.0,
    });
    realization
        .borrow_mut()
        .begin_drag(
            &docking.layout(),
            item("second"),
            Point { x: 100.0, y: 100.0 },
        )
        .expect("main item should begin a drag");
    let target = realization
        .borrow()
        .target_for_drop(Some(Point { x: 902.0, y: 220.0 }), Point { x: 0.0, y: 0.0 })
        .expect("floating surface should contain the screen point");
    assert_eq!(target.root, RootKind::Floating(0));
    assert_eq!(target.target, DockTarget::DockLeft);
    assert_eq!(target.group, None);
}

#[test]
fn screen_drop_on_floating_surface_uses_only_that_surface_group_for_center() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts, log));
    let model = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .unwrap();
    docking.set_layout(model);
    let root: Rc<dyn UIElementExt> = docking.clone();
    root.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 0.0, y: 0.0 },
    })));
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let realization = docking.realization_for_test().unwrap();
    let floating_surface = realization
        .borrow()
        .surface_for_test(&RootKind::Floating(0))
        .unwrap();
    floating_surface.set_coordinate_host(Some(Rc::new(OffsetCoordinateHost {
        screen_origin: Point { x: 900.0, y: 100.0 },
    })));
    assert!(floating_surface.apply_template());
    let floating_node: Rc<dyn UIElementExt> = floating_surface.clone();
    layout_root(
        &floating_node,
        Size {
            width: 420.0,
            height: 260.0,
        },
    );
    realization
        .borrow_mut()
        .begin_drag(
            &docking.layout(),
            item("second"),
            Point { x: 100.0, y: 100.0 },
        )
        .expect("main item should begin a drag");
    let target = realization
        .borrow()
        .target_for_drop(
            Some(Point {
                x: 1110.0,
                y: 230.0,
            }),
            Point { x: 0.0, y: 0.0 },
        )
        .expect("floating center should resolve");
    assert_eq!(target.root, RootKind::Floating(0));
    assert_eq!(target.target, DockTarget::Center);
    assert!(target.group.is_some());
}

#[test]
fn actual_splitter_pointer_path_previews_tracks_and_commits_once_or_restores_on_cancel() {
    let docking = mounted_three_pane_docking();
    let original = docking.layout();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let splitter = find_all::<CustomSplitter>(docking.as_ref())
        .into_iter()
        .next()
        .expect("three-pane runtime split should contain a splitter");
    let splitter_node: Rc<dyn UIElementExt> = splitter.clone();
    let bounds = SurfaceRegistry::bounds_in_host_root(&splitter_node)
        .expect("arranged splitter should have host-root bounds");
    let grid_node = splitter
        .visual_parent()
        .expect("splitter should have a parent");
    let grid = grid_node
        .as_any()
        .downcast_ref::<Grid>()
        .expect("splitter parent should be the retained split Grid");
    let original_tracks = grid.columns.borrow().clone();
    let start = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    let realization = docking
        .realization_for_test()
        .expect("mounted docking has a realization");
    let full_reconciles_before_drag = realization.borrow().full_reconcile_count_for_test();
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), start),
    );
    for step in 1..=20 {
        dispatcher.handle(
            &root,
            &focus,
            pointer_event(
                RawPointerEventKind::Moved,
                Point {
                    x: start.x + 36.0 * step as f32 / 20.0,
                    y: start.y,
                },
            ),
        );
    }
    assert!(realization.borrow().active_splitter_for_test());
    assert_eq!(docking.layout(), original);
    assert_ne!(*grid.columns.borrow(), original_tracks);
    assert_eq!(
        realization.borrow().full_reconcile_count_for_test(),
        full_reconciles_before_drag,
        "splitter preview must not reconcile the Dock runtime"
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(
            RawPointerEventKind::Released(MouseButton::Left),
            Point {
                x: start.x + 36.0,
                y: start.y,
            },
        ),
    );
    assert!(!realization.borrow().active_splitter_for_test());
    assert_ne!(docking.layout(), original);
    assert_eq!(changes.get(), 1);
    assert_eq!(
        realization.borrow().full_reconcile_count_for_test(),
        full_reconciles_before_drag,
        "splitter completion must use the value-only path"
    );

    let canceled = mounted_three_pane_docking();
    let canceled_original = canceled.layout();
    let canceled_root: Rc<dyn UIElementExt> = canceled.clone();
    layout_root(
        &canceled_root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let canceled_splitter = find_all::<CustomSplitter>(canceled.as_ref())
        .into_iter()
        .next()
        .expect("canceled split should contain a splitter");
    let canceled_node: Rc<dyn UIElementExt> = canceled_splitter.clone();
    let canceled_bounds = SurfaceRegistry::bounds_in_host_root(&canceled_node).unwrap();
    let canceled_start = Point {
        x: canceled_bounds.x + canceled_bounds.width * 0.5,
        y: canceled_bounds.y + canceled_bounds.height * 0.5,
    };
    let canceled_grid_node = canceled_splitter
        .visual_parent()
        .expect("canceled splitter should have a parent");
    let canceled_grid = canceled_grid_node.as_any().downcast_ref::<Grid>().unwrap();
    let canceled_tracks = canceled_grid.columns.borrow().clone();
    let canceled_realization = canceled.realization_for_test().unwrap();
    let canceled_full_reconciles = canceled_realization
        .borrow()
        .full_reconcile_count_for_test();
    let canceled_changes = Rc::new(Cell::new(0));
    let canceled_changes_for_callback = canceled_changes.clone();
    canceled.set_on_layout_change(Box::new(move |_| {
        canceled_changes_for_callback.set(canceled_changes_for_callback.get() + 1);
    }));
    let canceled_dispatcher = PointerDispatcher::new();
    let canceled_focus = FocusTracker::new();
    canceled_dispatcher.handle(
        &canceled_root,
        &canceled_focus,
        pointer_event(
            RawPointerEventKind::Pressed(MouseButton::Left),
            canceled_start,
        ),
    );
    canceled_dispatcher.handle(
        &canceled_root,
        &canceled_focus,
        pointer_event(
            RawPointerEventKind::Moved,
            Point {
                x: canceled_start.x + 36.0,
                y: canceled_start.y,
            },
        ),
    );
    assert_ne!(*canceled_grid.columns.borrow(), canceled_tracks);
    assert!(canceled_dispatcher.cancel());
    assert_eq!(canceled.layout(), canceled_original);
    assert_eq!(*canceled_grid.columns.borrow(), canceled_tracks);
    assert_eq!(canceled_changes.get(), 0);
    assert_eq!(
        canceled_realization
            .borrow()
            .full_reconcile_count_for_test(),
        canceled_full_reconciles
    );
}

#[test]
fn splitter_preview_arranges_retained_children_without_measuring_them() {
    let (docking, probes) = mounted_three_pane_probed_docking();
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 720.0,
        height: 420.0,
    };
    layout_root(&root, size);
    let splitter = find_all::<CustomSplitter>(docking.as_ref())
        .into_iter()
        .next()
        .expect("probed split should contain a splitter");
    let splitter_node: Rc<dyn UIElementExt> = splitter.clone();
    let splitter_bounds = SurfaceRegistry::bounds_in_host_root(&splitter_node)
        .expect("splitter should have arranged bounds");
    let grid_node = splitter
        .visual_parent()
        .expect("splitter should have a parent");
    let grid = grid_node
        .as_any()
        .downcast_ref::<Grid>()
        .expect("splitter parent should be the retained split Grid");
    let original_tracks = grid.columns.borrow().clone();
    let arranged_width = grid.arranged_width().expect("split grid width");
    let arranged_height = grid.arranged_height().expect("split grid height");
    let original_first_pane_width = grid.children().to_vec()[0]
        .arranged_width()
        .expect("first pane should have arranged width");
    for probe in &probes {
        probe.reset_measure_count();
    }

    let start = Point {
        x: splitter_bounds.x + splitter_bounds.width * 0.5,
        y: splitter_bounds.y + splitter_bounds.height * 0.5,
    };
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), start),
    );
    for step in 1..=20 {
        dispatcher.handle(
            &root,
            &focus,
            pointer_event(
                RawPointerEventKind::Moved,
                Point {
                    x: start.x + 36.0 * step as f32 / 20.0,
                    y: start.y,
                },
            ),
        );
    }

    grid.arrange(Rect {
        x: 0.0,
        y: 0.0,
        width: arranged_width,
        height: arranged_height,
    });

    assert_ne!(
        *grid.columns.borrow(),
        original_tracks,
        "preview columns={:?}",
        grid.columns.borrow()
    );
    assert_ne!(
        grid.children().to_vec()[0].arranged_width(),
        Some(original_first_pane_width),
        "arrange-only preview should move the pane boundary: columns={:?}",
        grid.columns.borrow(),
    );
    assert!(probes.iter().all(|probe| probe.measure_count() == 0));
    assert!(dispatcher.cancel());
}

#[test]
fn empty_initial_layout_publishes_once_and_source_assignment_does_not_echo() {
    let page = super::core::ui::TextBlock::new();
    let dock_item = DockItem::new_item();
    dock_item.set_id(item("source-item"));
    dock_item.set_title("Source item".to_string());
    dock_item.set_content(page);
    let dock_group = DockGroup::new_group();
    dock_group.set_id(group("source-group"));
    dock_group.set_children(vec![dock_item]);

    let docking = DockingControl::__new_unmounted();
    let changes = std::rc::Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    docking.set_content(dock_group);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    assert_eq!(changes.get(), 1);

    let source = docking
        .layout()
        .with_item_closed(&item("source-item"))
        .unwrap();
    docking.set_layout(source);
    assert!(docking.layout().is_item_closed(&item("source-item")));
    assert_eq!(changes.get(), 1);
}

#[test]
fn dynamic_authored_registration_adds_and_removes_items_with_one_publication_each() {
    let first_page = super::core::ui::TextBlock::new();
    let first = DockItem::new_item();
    first.set_id(item("dynamic-first"));
    first.set_title("First".to_string());
    first.set_content(first_page);
    let dock_group = DockGroup::new_group();
    dock_group.set_id(group("dynamic-group"));
    dock_group.set_children(vec![first.clone()]);

    let docking = DockingControl::__new_unmounted();
    docking.set_content(dock_group.clone());
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let changes = std::rc::Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));

    let second = DockItem::new_item();
    second.set_id(item("dynamic-second"));
    second.set_title("Second".to_string());
    second.set_content(super::core::ui::TextBlock::new());
    dock_group.set_children(vec![first, second]);
    assert!(docking.layout().contains_item(&item("dynamic-second")));
    assert_eq!(changes.get(), 1);

    dock_group.set_children(Vec::new());
    assert!(!docking.layout().contains_item(&item("dynamic-first")));
    assert!(!docking.layout().contains_item(&item("dynamic-second")));
    assert_eq!(changes.get(), 2);
}

#[test]
fn registration_refresh_publishes_before_stale_floating_host_sync() {
    let (first, _) = authored_item("registration-first", "First", true);
    let (second, _) = authored_item("registration-second", "Second", true);
    let dock_group = DockGroup::new_group();
    dock_group.set_id(group("registration-group"));
    dock_group.set_children(vec![first.clone(), second]);

    let docking = DockingControl::__new_unmounted();
    docking.set_content(dock_group.clone());
    docking.mount(application_environment());
    assert!(docking.apply_template());

    let hosts = Rc::new(RefCell::new(Vec::new()));
    let log = FakeHostLog::new();
    docking.install_floating_host_factory_for_test(fake_factory(hosts, log.clone()));
    let floating = docking
        .layout()
        .with_item_moved(
            &item("registration-first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .expect("item should float");
    docking.set_layout(floating);
    log.events.borrow_mut().clear();

    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    let log_for_callback = log.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
        log_for_callback.events.borrow_mut().push("layout_callback");
    }));

    dock_group.set_children(Vec::new());

    assert_eq!(changes.get(), 1);
    assert_eq!(
        *log.events.borrow(),
        vec!["layout_callback", "clear_close_handler", "close"]
    );
}

#[test]
fn docking_unmount_clears_floating_hosts_surfaces_and_weak_owner_callbacks() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let floating = docking
        .layout()
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 900.0,
                    y: 100.0,
                    width: 420.0,
                    height: 260.0,
                },
            },
        )
        .expect("item should float");
    docking.set_layout(floating);

    let realization = docking.realization_for_test().unwrap();
    let floating_surface = realization
        .borrow()
        .surface_for_test(&RootKind::Floating(0))
        .expect("floating surface should exist");
    let weak_surface = Rc::downgrade(&floating_surface);
    let host = hosts.borrow()[0].clone();
    let log = host.log.clone();
    let weak_docking = Rc::downgrade(&docking);
    let root: Rc<dyn UIElementExt> = docking.clone();

    unmount_subtree(&root);
    assert_eq!(log.close_count.get(), 1);
    assert!(log.close_handler.borrow().is_none());
    assert_eq!(realization.borrow().floating_host_count_for_test(), 0);
    assert_eq!(realization.borrow().surface_registry_count_for_test(), 0);

    // The fake host intentionally retains its content like a native host would until its close
    // teardown completes. Release that external probe before checking the runtime's weak graph.
    *log.content.borrow_mut() = None;
    hosts.borrow_mut().clear();
    drop(host);
    drop(floating_surface);
    drop(realization);
    drop(root);
    drop(docking);
    assert!(weak_docking.upgrade().is_none());
    assert!(weak_surface.upgrade().is_none());
}
