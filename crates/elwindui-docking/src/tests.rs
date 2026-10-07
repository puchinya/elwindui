use super::DockSize;
use super::DockTarget;
use super::Orientation;
use super::core::base::{Point, Size};
use super::core::environment::application_environment;
use super::core::focus::FocusTracker;
use super::core::input::{
    KeyModifiers, MouseButton, PointerDispatcher, PointerEventArgs, RawPointerEvent,
    RawPointerEventKind, RoutedEventArgs,
};
use super::core::layout::{GridLength, Visibility};
use super::core::ui::{
    ContentControlExt, Grid, GridExt, InvalidationKind, LayoutExt, Rectangle, RelayoutHost,
    TextBlock, TextBlockExt, UIElementExt, layout_root, unmount_subtree,
};
use super::core::visual_tree::find_all;
use super::docking_control::DockTabContextAction;
use super::id::{DockGroupId, DockItemId};
use super::model::{
    DefaultDockDefinition, DockLayoutModel, InternalDockGroupKey, InternalDockPlacement, Node,
    RootKind, SplitAddress, WeightedNode,
};
use super::placement::{DockLayoutError, DockPlacement, DockSide};
use super::runtime::{
    AutoHideOverlay, DockSplitView, DockSurfaceView, DockTargetOverlay, DragSession,
    DragSourceGeometry, DropPreview, FloatingHostFactory, FloatingHostRegistry, FloatingWindowHost,
    LatestOnlyQueue, ResolvedDockTarget, SurfaceRegistry, resolve_local_target_for_test,
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
    CustomGridSplitter, CustomGridSplitterExt, CustomTabView, CustomTabViewExt, CustomTabViewItem,
    GridResizeBehavior, GridResizeDirection, TabDragCompletedEventArgs, TabStripPosition,
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

struct SiblingMeasureRelayoutHost {
    sibling: Rc<DockingMeasureProbe>,
    dirty_groups: RefCell<Vec<u64>>,
}

impl RelayoutHost for SiblingMeasureRelayoutHost {
    fn request_relayout(&self, dirty_group_id: u64, _kind: InvalidationKind) {
        self.dirty_groups.borrow_mut().push(dirty_group_id);
    }
}

impl SiblingMeasureRelayoutHost {
    fn layout_dirty_sibling(&self) {
        let sibling_group = self.sibling.as_ui_element().render_group_id;
        let dirty_groups = self.dirty_groups.borrow_mut().drain(..).collect::<Vec<_>>();
        if dirty_groups.contains(&sibling_group) {
            self.sibling.measure(Size {
                width: 80.0,
                height: 40.0,
            });
        }
    }
}

struct FakeHostLog {
    events: RefCell<Vec<&'static str>>,
    close_count: Cell<usize>,
    bounds: Cell<Option<Rect>>,
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    close_handler: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    bounds_changed_handler: RefCell<Option<Rc<dyn Fn(Rect)>>>,
    invoke_bounds_changed_on_set: Cell<bool>,
}

impl FakeHostLog {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            events: RefCell::new(Vec::new()),
            close_count: Cell::new(0),
            bounds: Cell::new(None),
            content: RefCell::new(None),
            close_handler: RefCell::new(None),
            bounds_changed_handler: RefCell::new(None),
            invoke_bounds_changed_on_set: Cell::new(false),
        })
    }

    fn invoke_close(&self) -> bool {
        let handler = self.close_handler.borrow().clone();
        handler.map_or(false, |handler| handler())
    }

    fn invoke_bounds_changed(&self, bounds: Rect) {
        let handler = self.bounds_changed_handler.borrow().clone();
        if let Some(handler) = handler {
            handler(bounds);
        }
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
        if self.log.invoke_bounds_changed_on_set.get() {
            self.log.invoke_bounds_changed(bounds);
        }
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

    fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
        *self.log.bounds_changed_handler.borrow_mut() = handler;
    }
}

struct RecordingRelayoutHost {
    requests: RefCell<Vec<InvalidationKind>>,
    flushes: Cell<usize>,
}

impl RecordingRelayoutHost {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            requests: RefCell::new(Vec::new()),
            flushes: Cell::new(0),
        })
    }
}

impl RelayoutHost for RecordingRelayoutHost {
    fn request_relayout(&self, _dirty_group_id: u64, kind: InvalidationKind) {
        self.requests.borrow_mut().push(kind);
    }

    fn flush_interactive_relayout(&self) {
        self.flushes.set(self.flushes.get() + 1);
    }
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
        group_bounds: group.as_ref().map(|_| Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        }),
        group,
        preview_rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        },
        tab_insert_index: None,
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

#[test]
fn dock_group_defaults_to_compact_tab_widths() {
    assert!(DockGroup::new_group().compact_tabs_value());
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

fn mounted_bottom_documents_docking() -> Rc<DockingControl> {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", true);
    let (third, _) = authored_item("third", "Third", true);
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_tab_strip_position(TabStripPosition::Bottom);
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

fn mounted_capability_docking(
    first_can_float: bool,
    second_can_float: bool,
    first_can_dock: bool,
    second_can_dock: bool,
) -> Rc<DockingControl> {
    let (first, _) = authored_item("first", "First", true);
    first.set_can_float(first_can_float);
    first.set_can_dock(first_can_dock);
    let (second, _) = authored_item("second", "Second", true);
    second.set_can_float(second_can_float);
    second.set_can_dock(second_can_dock);
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

fn mounted_unequal_docking() -> Rc<DockingControl> {
    let (source, _) = authored_item("source", "Source", true);
    let (short, _) = authored_item("short", "A medium target header", true);
    let (long, _) = authored_item("long", "A very long target header", true);
    let (tail, _) = authored_item("tail", "C", true);
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_children(vec![source]);
    let tools = DockGroup::new_group();
    tools.set_id(group("tools"));
    tools.set_compact_tabs(true);
    tools.set_children(vec![short, long, tail]);
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

fn arranged_group_point(docking: &Rc<DockingControl>, group_id: &str) -> (Rc<CustomTabView>, Rect) {
    let view = docking
        .realization_for_test()
        .and_then(|realization| {
            realization
                .borrow()
                .group_for_test(&SnapshotGroupKey::Authored(group(group_id)))
        })
        .expect("authored group should have a retained view");
    let node: Rc<dyn UIElementExt> = view.clone();
    let bounds = SurfaceRegistry::bounds_in_host_root(&node).expect("group should be arranged");
    (view, bounds)
}

fn first_tab_in_group(docking: &Rc<DockingControl>, group_id: &str) -> Rc<dyn UIElementExt> {
    let (view, _) = arranged_group_point(docking, group_id);
    find_all::<CustomTabViewItem>(view.as_ref())
        .into_iter()
        .next()
        .expect("group should contain a retained tab item")
}

fn mounted_default_docking_with_keep_empty_tools() -> Rc<DockingControl> {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", true);
    let (third, _) = authored_item("third", "Third", true);
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_children(vec![first, second]);
    let tools = DockGroup::new_group();
    tools.set_id(group("tools"));
    tools.set_show_when_empty(true);
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
fn activation_selection_only_classifies_live_main_and_floating_items() {
    let model = default_model();
    assert!(model.activation_is_selection_only(&item("first")));

    let floating = model
        .with_item_moved(
            &item("first"),
            DockPlacement::Floating {
                bounds: Rect {
                    x: 40.0,
                    y: 50.0,
                    width: 320.0,
                    height: 240.0,
                },
            },
        )
        .unwrap();
    assert!(floating.activation_is_selection_only(&item("first")));

    let closed = model.with_item_closed(&item("first")).unwrap();
    assert!(!closed.activation_is_selection_only(&item("first")));

    let auto_hidden = model
        .with_item_moved(
            &item("first"),
            DockPlacement::AutoHide {
                side: DockSide::Left,
            },
        )
        .unwrap();
    assert!(!auto_hidden.activation_is_selection_only(&item("first")));
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
        Point { x: 30.0, y: 120.0 },
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
        Point { x: 30.0, y: 120.0 },
        vec![(
            SnapshotGroupKey::Authored(group("floating-group")),
            Rect {
                x: 80.0,
                y: 40.0,
                width: 240.0,
                height: 160.0,
            },
        )],
    );
    // Floating surfaces have no root-edge targets, like the WinUI.Dock reference.
    assert_eq!(floating_target, None);
}

#[test]
fn floating_surface_edge_has_no_root_target() {
    let target = resolve_local_target_for_test(
        RootKind::Floating(2),
        Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 240.0,
        },
        Point { x: 30.0, y: 120.0 },
        vec![(
            SnapshotGroupKey::Authored(group("main-group")),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 240.0,
            },
        )],
    );
    assert_eq!(target, None);
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
        Point { x: 260.0, y: 200.0 },
        vec![(
            key.clone(),
            Rect {
                x: 200.0,
                y: 120.0,
                width: 200.0,
                height: 160.0,
            },
        )],
    )
    .expect("the drawn SplitLeft compass cell should resolve");
    assert_eq!(left_tie.target, DockTarget::SplitLeft);

    // The group's former edge band no longer resolves: only drawn targets do.
    assert_eq!(
        resolve_local_target_for_test(
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
        ),
        None
    );
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
            Point { x: 160.0, y: 120.0 },
            DockTarget::SplitLeft,
            Rect {
                x: 80.0,
                y: 40.0,
                width: 120.0,
                height: 160.0,
            },
        ),
        (
            Point { x: 240.0, y: 120.0 },
            DockTarget::SplitRight,
            Rect {
                x: 200.0,
                y: 40.0,
                width: 120.0,
                height: 160.0,
            },
        ),
        (
            Point { x: 200.0, y: 80.0 },
            DockTarget::SplitTop,
            Rect {
                x: 80.0,
                y: 40.0,
                width: 240.0,
                height: 80.0,
            },
        ),
        (
            Point { x: 200.0, y: 160.0 },
            DockTarget::SplitBottom,
            Rect {
                x: 80.0,
                y: 120.0,
                width: 240.0,
                height: 80.0,
            },
        ),
        (
            Point { x: 18.0, y: 120.0 },
            DockTarget::DockLeft,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 240.0,
            },
        ),
        (
            Point { x: 382.0, y: 120.0 },
            DockTarget::DockRight,
            Rect {
                x: 200.0,
                y: 0.0,
                width: 200.0,
                height: 240.0,
            },
        ),
        (
            Point { x: 200.0, y: 18.0 },
            DockTarget::DockTop,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 120.0,
            },
        ),
        (
            Point { x: 200.0, y: 222.0 },
            DockTarget::DockBottom,
            Rect {
                x: 0.0,
                y: 120.0,
                width: 400.0,
                height: 120.0,
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
        assert_eq!(target.tab_insert_index, None);
    }
}

#[test]
fn repeated_drag_overlay_state_does_not_invalidate_layout() {
    let overlay = DockTargetOverlay::new();
    let group = Rect {
        x: 80.0,
        y: 40.0,
        width: 240.0,
        height: 160.0,
    };
    overlay.show(None, Some(group));
    let visual = overlay.visual();
    let size = Size {
        width: 400.0,
        height: 240.0,
    };
    super::core::ui::layout_root(&visual, size);
    assert!(visual.arranged_width().is_some());

    // A pointer move with the same hover state must not invalidate (or re-measure) the overlay.
    overlay.show(None, Some(group));
    assert!(visual.arranged_width().is_some());

    // A changed hover moves the compass with an arrange-only invalidation.
    overlay.show(None, Some(Rect { x: 120.0, ..group }));
    assert_eq!(visual.arranged_width(), None);
    assert!(visual.measured_size().is_some());

    let mut preview = DropPreview::new();
    let target = ResolvedDockTarget {
        root: RootKind::Main,
        target: DockTarget::Center,
        group: None,
        group_bounds: Some(group),
        preview_rect: group,
        tab_insert_index: None,
    };
    preview.show(&target);
    let preview_visual = preview.visual();
    super::core::ui::layout_root(&preview_visual, size);
    preview.show(&target);
    assert!(preview_visual.arranged_width().is_some());
}

#[test]
fn first_overlay_appearance_settles_in_one_layout_pass() {
    let overlay = DockTargetOverlay::new();
    let visual = overlay.visual();
    let size = Size {
        width: 400.0,
        height: 240.0,
    };
    // Root targets only, then the compass appears: each first appearance must leave layout
    // valid after one pass. Visibility flipped inside arrange would invalidate it again and make
    // the host rerun a whole-tree pass.
    overlay.show(None, None);
    super::core::ui::layout_root(&visual, size);
    assert!(visual.measured_size().is_some() && visual.arranged_width().is_some());
    overlay.show(
        None,
        Some(Rect {
            x: 80.0,
            y: 40.0,
            width: 240.0,
            height: 160.0,
        }),
    );
    super::core::ui::layout_root(&visual, size);
    assert!(visual.measured_size().is_some() && visual.arranged_width().is_some());
    assert!(
        overlay
            .button_rects()
            .iter()
            .all(|(_, rect)| rect.is_some_and(|rect| rect.width > 0.0))
    );
    assert_eq!(
        overlay.cross_background_for_test().visibility(),
        Visibility::Visible
    );

    // Leaving the group hides the compass before layout, again without a second pass.
    overlay.show(None, None);
    assert_eq!(
        overlay.cross_background_for_test().visibility(),
        Visibility::Collapsed
    );
    super::core::ui::layout_root(&visual, size);
    assert!(visual.measured_size().is_some() && visual.arranged_width().is_some());
}

#[test]
fn root_and_group_target_visuals_are_retained_and_never_alias_highlights() {
    let overlay = DockTargetOverlay::new();
    assert_eq!(overlay.button_counts(), (5, 4));
    overlay.show(Some(DockTarget::DockLeft), None);
    assert_eq!(overlay.selected_target(), Some(DockTarget::DockLeft));
    let visual = overlay.visual();
    super::core::ui::layout_root(
        &visual,
        Size {
            width: 400.0,
            height: 240.0,
        },
    );
    let rects = overlay.button_rects();
    let left = rects
        .iter()
        .find(|(target, _)| *target == DockTarget::DockLeft)
        .and_then(|(_, rect)| *rect)
        .expect("root left target should be arranged");
    assert_eq!(
        left,
        Rect {
            x: 0.0,
            y: 102.0,
            width: 36.0,
            height: 36.0
        }
    );
    overlay.show(
        Some(DockTarget::SplitLeft),
        Some(Rect {
            x: 80.0,
            y: 30.0,
            width: 220.0,
            height: 160.0,
        }),
    );
    super::core::ui::layout_root(
        &visual,
        Size {
            width: 400.0,
            height: 240.0,
        },
    );
    assert_eq!(overlay.selected_target(), Some(DockTarget::SplitLeft));
    let compass_center = overlay
        .button_rects()
        .into_iter()
        .find(|(target, _)| *target == DockTarget::Center)
        .and_then(|(_, rect)| rect)
        .map(|rect| Point {
            x: rect.x + rect.width * 0.5,
            y: rect.y + rect.height * 0.5,
        })
        .expect("group center target should be arranged");
    assert_eq!(compass_center, Point { x: 190.0, y: 110.0 });
    assert_eq!(overlay.target_glyph_kinds().len(), 9);
    overlay.clear();
    assert_eq!(overlay.selected_target(), None);
}

#[test]
fn compass_backing_is_painted_before_any_theme_change() {
    use super::core::graphics::{IconSource, ImageSource, PathCommand, VectorNode};
    use super::core::ui::IconSourceElementExt;
    let overlay = DockTargetOverlay::new();
    let background = overlay.cross_background_for_test();
    assert_eq!(background.width(), Some(124.0));
    assert_eq!(background.height(), Some(124.0));
    let Some(IconSource::Image(ImageSource::Vector(image))) = background.icon_source() else {
        panic!("the compass must have its vector backing before theme refresh");
    };
    assert_eq!(
        image.intrinsic_size(),
        Size {
            width: 124.0,
            height: 124.0
        }
    );
    assert_eq!(image.root().children.len(), 1);
    let VectorNode::Path(contour) = &image.root().children[0] else {
        panic!("the backing must be one connected contour");
    };
    assert!(contour.fill.is_some());
    assert_eq!(contour.stroke.as_ref().unwrap().style.width, 1.0);
    assert_eq!(
        contour
            .path
            .commands()
            .iter()
            .filter(|command| matches!(command, PathCommand::QuadTo { .. }))
            .count(),
        12
    );
    assert_eq!(contour.path.commands().last(), Some(&PathCommand::Close));
}

#[test]
fn drop_preview_layer_arranges_the_rectangle_at_the_resolved_surface_rect() {
    let target = ResolvedDockTarget {
        root: RootKind::Floating(0),
        target: DockTarget::SplitRight,
        group: Some(SnapshotGroupKey::Generated(1)),
        group_bounds: Some(Rect {
            x: 100.0,
            y: 20.0,
            width: 200.0,
            height: 120.0,
        }),
        preview_rect: Rect {
            x: 137.0,
            y: 21.0,
            width: 163.0,
            height: 119.0,
        },
        tab_insert_index: None,
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
fn floating_bounds_use_the_default_extent_and_pointer_offset() {
    let source = DragSourceGeometry {
        source_root: RootKind::Main,
        source_bounds_host: Rect {
            x: 300.0,
            y: 200.0,
            width: 720.0,
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
            width: 400.0,
            height: 400.0,
        },
    );
}

#[test]
fn floating_bounds_keep_the_grab_point_inside_and_reject_missing_geometry() {
    let wide = DragSourceGeometry {
        source_root: RootKind::Floating(0),
        source_bounds_host: Rect {
            x: 0.0,
            y: 0.0,
            width: 900.0,
            height: 80.0,
        },
        pointer_offset: Point { x: 640.0, y: 20.0 },
    };
    assert_rect_eq(
        super::docking_control::floating_bounds_for_test(
            &wide,
            Point {
                x: 1000.0,
                y: 700.0,
            },
        ),
        Rect {
            x: 600.0,
            y: 680.0,
            width: 400.0,
            height: 400.0,
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
        group_bounds: None,
        preview_rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 105.0,
            height: 260.0,
        },
        tab_insert_index: None,
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
    let (_, page_host) = realization
        .borrow()
        .auto_hide_parts_for_test(&RootKind::Floating(0))
        .expect("floating auto-hide keeps its popup parts");
    let page_host_node: Rc<dyn UIElementExt> = page_host;
    let page_parent = hidden_page
        .visual_parent()
        .expect("auto-hide popup should own the page content");
    assert!(Rc::ptr_eq(&page_parent, &page_host_node));
    assert!(
        hidden_page
            .visual_parent()
            .is_some_and(|parent| parent.visual_parent().is_some())
    );
}

#[test]
fn auto_hide_strip_remains_visible_after_docking_model_commit() {
    let (main_item, _) = authored_item("main", "Main", true);
    let (hidden_item, _) = authored_item("hidden", "Hidden", true);
    let docking = mounted_docking_with_items(vec![main_item, hidden_item]);
    let model = docking.layout();
    let auto_hidden = model
        .with_item_moved(
            &item("hidden"),
            DockPlacement::AutoHide {
                side: DockSide::Right,
            },
        )
        .expect("auto-hide placement should be valid");

    docking.set_layout(auto_hidden);

    let realization = docking
        .realization_for_test()
        .expect("mounted docking has a realization");
    let (auto_hide_visual, _) = realization
        .borrow()
        .surface_chrome_for_test(&RootKind::Main)
        .expect("main surface has retained chrome");
    let auto_hide = auto_hide_visual
        .as_any()
        .downcast_ref::<Grid>()
        .expect("auto-hide chrome uses a Grid root");
    layout_root(
        &auto_hide_visual,
        Size {
            width: 944.0,
            height: 549.0,
        },
    );

    let right_strip = auto_hide.children().to_vec()[2].clone();
    assert_eq!(right_strip.visual_children().len(), 1);
    assert_eq!(right_strip.arranged_width(), Some(28.0));
    assert_eq!(right_strip.arranged_height(), Some(549.0));
    assert_eq!(
        right_strip.arranged_offset(),
        Some(Point { x: 916.0, y: 0.0 })
    );
}

#[test]
fn auto_hide_popup_uses_side_aware_opaque_panel_geometry() {
    let wrapper = CustomTabViewItem::new_item();
    let page = TextBlock::new();
    page.set_text("Popup body");
    wrapper.set_content(page.clone());
    let mut overlay = AutoHideOverlay::new();
    overlay.open(
        item("right"),
        DockSide::Right,
        Size {
            width: 900.0,
            height: 600.0,
        },
    );
    overlay.present_open_item(Some(wrapper.clone()), "Right document", true, true);

    let page_host: Rc<dyn UIElementExt> = overlay.page_host_for_test();
    let page_parent = page
        .visual_parent()
        .expect("popup should present the inherited page content");
    assert!(Rc::ptr_eq(&page_parent, &page_host));
    assert!(wrapper.visual_children().is_empty());

    let visual = overlay.visual();
    layout_root(
        &visual,
        Size {
            width: 900.0,
            height: 600.0,
        },
    );
    let pane = overlay.pane_for_test();
    assert_eq!(pane.arranged_width(), Some(300.0));
    assert_eq!(pane.arranged_height(), Some(600.0));
    assert_eq!(pane.arranged_offset(), Some(Point { x: 600.0, y: 0.0 }));
    let pin_button = overlay.pin_button_for_test();
    assert_eq!(
        pin_button.arranged_offset(),
        // 28-pixel reference pane buttons inside 12-pixel header insets, vertically centered.
        Some(Point { x: 232.0, y: 6.0 })
    );
}

#[test]
fn auto_hide_resize_callbacks_release_the_pane_and_its_content() {
    let mut overlay = AutoHideOverlay::new();
    let pane = Rc::downgrade(&overlay.pane_for_test());
    let grip = Rc::downgrade(&overlay.resize_grip_for_test());
    let wrapper = CustomTabViewItem::new_item();
    let page = Grid::new();
    let page_weak = Rc::downgrade(&page);
    wrapper.set_content(page);
    overlay.open(
        item("right"),
        DockSide::Right,
        Size {
            width: 900.0,
            height: 600.0,
        },
    );
    overlay.present_open_item(Some(wrapper.clone()), "Right document", true, true);
    drop(wrapper);
    drop(overlay);

    assert!(pane.upgrade().is_none());
    assert!(grip.upgrade().is_none());
    assert!(page_weak.upgrade().is_none());
}

#[test]
fn auto_hide_strip_markers_follow_the_four_side_orientations() {
    let overlay = AutoHideOverlay::new();
    let owner = std::rc::Weak::<DockingControl>::new();
    overlay.render_strips(
        [
            (0, item("left"), "Left document".to_owned(), None),
            (1, item("top"), "Top document".to_owned(), None),
            (2, item("right"), "Right document".to_owned(), None),
            (3, item("bottom"), "Bottom document".to_owned(), None),
        ]
        .into_iter(),
        &owner,
        RootKind::Main,
    );
    let visual = overlay.visual();
    layout_root(
        &visual,
        Size {
            width: 944.0,
            height: 549.0,
        },
    );
    assert_eq!(overlay.marker_count_for_test(), 4);
    let left = overlay.marker_size_for_test(&item("left")).unwrap();

    assert_eq!(left.width, 4.0);
    assert!(left.height > 24.0);
    let right = overlay.marker_size_for_test(&item("right")).unwrap();
    assert_eq!(right.width, 4.0);
    assert!(right.height > 24.0);
    let top = overlay.marker_size_for_test(&item("top")).unwrap();
    assert!(top.width > 24.0);
    assert_eq!(top.height, 4.0);
    let bottom = overlay.marker_size_for_test(&item("bottom")).unwrap();
    assert!(bottom.width > 24.0);
    assert_eq!(bottom.height, 4.0);
    let rotated_titles = find_all::<TextBlock>(visual.as_ref())
        .into_iter()
        .filter(|text| text.visual_transform().rotation != 0.0)
        .collect::<Vec<_>>();
    assert_eq!(rotated_titles.len(), 2);
    for text in rotated_titles {
        assert!(text.arranged_width().unwrap() > 24.0);
        assert_eq!(text.arranged_height(), Some(24.0));
        assert_eq!(
            text.visual_transform().rotation.abs(),
            std::f32::consts::FRAC_PI_2
        );
    }
    overlay.refresh_theme();
    for id in ["left", "top", "right", "bottom"] {
        assert_eq!(
            overlay.marker_fill_for_test(&item(id)),
            crate::runtime::themed_brush(crate::core::theme::BrushStyle::Separator)
        );
    }
}

#[test]
fn auto_hide_entries_measure_titles_only_in_normal_layout() {
    struct CountingTextBackend(Rc<std::cell::Cell<usize>>);
    impl crate::core::graphics::TextBackend for CountingTextBackend {
        fn default_text_style(&self) -> crate::core::graphics::ComputedTextStyle {
            crate::core::graphics::ComputedTextStyle::fallback()
        }
        fn measure_text(
            &self,
            request: &crate::core::graphics::TextMeasureRequest<'_>,
        ) -> crate::core::graphics::TextMeasureResult {
            self.0.set(self.0.get() + 1);
            crate::core::graphics::DummyTextBackend.measure_text(request)
        }
    }
    let calls = Rc::new(std::cell::Cell::new(0));
    crate::core::graphics::set_text_backend(Rc::new(CountingTextBackend(calls.clone())));

    let overlay = AutoHideOverlay::new();
    let owner = std::rc::Weak::<DockingControl>::new();
    overlay.render_strips(
        [
            (0, item("left"), "Left document".to_owned(), None),
            (1, item("top"), "Top document".to_owned(), None),
        ]
        .into_iter(),
        &owner,
        RootKind::Main,
    );
    // Building the entries measures nothing; the rail extent comes from the layout pass.
    assert_eq!(calls.get(), 0);
    let visual = overlay.visual();
    let size = Size {
        width: 944.0,
        height: 549.0,
    };
    layout_root(&visual, size);
    let titles = find_all::<TextBlock>(visual.as_ref())
        .into_iter()
        .filter(|text| text.measured_size().is_some())
        .collect::<Vec<_>>();
    let after_first_layout = calls.get();
    // One backend measurement per title: no construction-time pass and no second constraint.
    assert_eq!(after_first_layout, titles.len());
    layout_root(&visual, size);
    assert_eq!(calls.get(), after_first_layout);

    for (id, vertical) in [("left", true), ("top", false)] {
        let marker = overlay.marker_size_for_test(&item(id)).unwrap();
        let title = find_all::<TextBlock>(visual.as_ref())
            .into_iter()
            .find(|text| {
                text.as_any()
                    .downcast_ref::<TextBlock>()
                    .is_some_and(|text| {
                        *text.text.borrow()
                            == format!("{} document", if vertical { "Left" } else { "Top" })
                    })
            })
            .unwrap();
        let length = title.measured_size().unwrap().width.max(24.0);
        assert_eq!(
            if vertical {
                marker.height
            } else {
                marker.width
            },
            length
        );
        assert_eq!(title.arranged_width(), Some(length));
    }
    crate::core::graphics::clear_text_backend();
}

#[test]
fn auto_hide_resize_grips_cover_the_whole_inner_edge() {
    for side in DockSide::ALL {
        let mut overlay = AutoHideOverlay::new();
        let size = Size {
            width: 900.0,
            height: 600.0,
        };
        overlay.open(item("pane"), side, size);
        let visual = overlay.visual();
        layout_root(&visual, size);
        let grip = overlay.resize_grip_for_test();
        let bounds = SurfaceRegistry::bounds_in_host_root(&(grip as Rc<dyn UIElementExt>)).unwrap();
        let expected = match side {
            DockSide::Left => Rect {
                x: 294.0,
                y: 0.0,
                width: 6.0,
                height: 600.0,
            },
            DockSide::Right => Rect {
                x: 600.0,
                y: 0.0,
                width: 6.0,
                height: 600.0,
            },
            DockSide::Top => Rect {
                x: 0.0,
                y: 194.0,
                width: 900.0,
                height: 6.0,
            },
            DockSide::Bottom => Rect {
                x: 0.0,
                y: 400.0,
                width: 900.0,
                height: 6.0,
            },
        };
        assert_rect_eq(Some(bounds), expected);
    }
}

#[test]
fn auto_hide_remembers_initial_extent_on_close_and_when_another_item_opens() {
    let mut overlay = AutoHideOverlay::new();
    let size = Size {
        width: 900.0,
        height: 600.0,
    };
    overlay.open(item("first"), DockSide::Left, size);
    overlay.close();
    assert_eq!(
        overlay.remembered_extent_for_test(&item("first"), DockSide::Left),
        Some(300.0)
    );
    let larger = Size {
        width: 1200.0,
        height: 900.0,
    };
    overlay.open(item("first"), DockSide::Left, larger);
    let visual = overlay.visual();
    layout_root(&visual, larger);
    assert_eq!(overlay.pane_for_test().arranged_width(), Some(300.0));
    overlay.open(item("second"), DockSide::Top, size);
    overlay.open(item("third"), DockSide::Bottom, larger);
    assert_eq!(
        overlay.remembered_extent_for_test(&item("second"), DockSide::Top),
        Some(200.0)
    );
    assert_eq!(
        overlay.remembered_extent_for_test(&item("first"), DockSide::Left),
        Some(300.0)
    );
}

#[test]
fn auto_hide_extent_survives_surface_teardown_in_the_same_runtime_cache() {
    let cache = Rc::new(RefCell::new(Default::default()));
    {
        let mut first_surface = AutoHideOverlay::with_extent_cache(cache.clone());
        first_surface.open(
            item("document"),
            DockSide::Right,
            Size {
                width: 900.0,
                height: 600.0,
            },
        );
    }
    let mut second_surface = AutoHideOverlay::with_extent_cache(cache);
    second_surface.open(
        item("document"),
        DockSide::Left,
        Size {
            width: 1200.0,
            height: 900.0,
        },
    );
    let visual = second_surface.visual();
    layout_root(
        &visual,
        Size {
            width: 1200.0,
            height: 900.0,
        },
    );
    assert_eq!(second_surface.pane_for_test().arranged_width(), Some(300.0));
    assert_eq!(
        second_surface.remembered_extent_for_test(&item("document"), DockSide::Left),
        Some(300.0)
    );
    let mut new_runtime = AutoHideOverlay::new();
    new_runtime.open(
        item("document"),
        DockSide::Left,
        Size {
            width: 1200.0,
            height: 900.0,
        },
    );
    layout_root(
        &new_runtime.visual(),
        Size {
            width: 1200.0,
            height: 900.0,
        },
    );
    assert_eq!(new_runtime.pane_for_test().arranged_width(), Some(400.0));
}

#[test]
fn auto_hide_resizing_remembers_document_axis_extent_and_inverts_trailing_edges() {
    let mut overlay = AutoHideOverlay::new();
    let size = Size {
        width: 900.0,
        height: 600.0,
    };
    overlay.open(item("right"), DockSide::Right, size);
    let visual = overlay.visual();
    layout_root(&visual, size);
    let grip = overlay.resize_grip_for_test();
    let grip_node: Rc<dyn UIElementExt> = grip.clone();
    let bounds = SurfaceRegistry::bounds_in_host_root(&grip_node).unwrap();
    let start = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &visual,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), start),
    );
    dispatcher.handle(
        &visual,
        &focus,
        pointer_event(
            RawPointerEventKind::Moved,
            Point {
                x: start.x - 40.0,
                y: start.y,
            },
        ),
    );
    layout_root(&visual, size);
    assert_eq!(overlay.pane_for_test().arranged_width(), Some(340.0));
    dispatcher.handle(
        &visual,
        &focus,
        pointer_event(
            RawPointerEventKind::Released(MouseButton::Left),
            Point {
                x: start.x - 40.0,
                y: start.y,
            },
        ),
    );
    assert_eq!(
        overlay.remembered_extent_for_test(&item("right"), DockSide::Right),
        Some(340.0)
    );
    overlay.close();
    overlay.open(item("right"), DockSide::Right, size);
    layout_root(&visual, size);
    assert_eq!(overlay.pane_for_test().arranged_width(), Some(340.0));

    overlay.close();
    overlay.open(item("bottom"), DockSide::Bottom, size);
    layout_root(&visual, size);
    let grip = overlay.resize_grip_for_test();
    let grip_node: Rc<dyn UIElementExt> = grip;
    let bounds = SurfaceRegistry::bounds_in_host_root(&grip_node).unwrap();
    let start = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    dispatcher.handle(
        &visual,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), start),
    );
    dispatcher.handle(
        &visual,
        &focus,
        pointer_event(
            RawPointerEventKind::Moved,
            Point {
                x: start.x,
                y: start.y - 30.0,
            },
        ),
    );
    layout_root(&visual, size);
    assert_eq!(overlay.pane_for_test().arranged_height(), Some(230.0));
    dispatcher.handle(
        &visual,
        &focus,
        pointer_event(
            RawPointerEventKind::Released(MouseButton::Left),
            Point {
                x: start.x,
                y: start.y - 30.0,
            },
        ),
    );
    assert_eq!(
        overlay.remembered_extent_for_test(&item("bottom"), DockSide::Bottom),
        Some(230.0)
    );
}

#[test]
fn auto_hide_light_dismiss_suppresses_model_reopen_until_explicit_activation() {
    let mut overlay = AutoHideOverlay::new();
    let surface_root = Grid::new();
    surface_root.set_background(Some(super::core::graphics::Color::TRANSPARENT.into()));
    surface_root.children().add(overlay.visual());
    overlay.bind_light_dismiss(&surface_root, &std::rc::Weak::new());
    let root: Rc<dyn UIElementExt> = surface_root.clone();
    let size = Size {
        width: 900.0,
        height: 600.0,
    };
    layout_root(&root, size);
    overlay.open(item("right"), DockSide::Right, size);
    layout_root(&root, size);
    let event = PointerEventArgs {
        position: Point { x: 450.0, y: 16.0 },
        screen_position: None,
        button: Some(MouseButton::Left),
        modifiers: KeyModifiers::default(),
    };
    super::core::ui::dispatch_routed(
        &root,
        "on_pointer_pressed",
        &event,
        &RoutedEventArgs::default(),
    );
    assert_eq!(overlay.current(), None);
    assert_eq!(
        overlay.open_from_model(item("right"), DockSide::Right, size),
        None
    );
    assert_eq!(overlay.current(), None);
    overlay.open(item("right"), DockSide::Right, size);
    assert_eq!(overlay.current(), Some(item("right")));
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
fn floating_bounds_move_callback_updates_the_current_root_once() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    docking.set_layout(floating_model_with_items(
        &docking.layout(),
        &["first"],
        Rect {
            x: 900.0,
            y: 100.0,
            width: 420.0,
            height: 260.0,
        },
    ));
    let host = hosts.borrow()[0].clone();
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    host.log.events.borrow_mut().clear();
    host.log.invoke_bounds_changed(Rect {
        x: 940.0,
        y: 130.0,
        width: 420.0,
        height: 260.0,
    });

    let bounds = docking.layout().snapshot().floating_roots[0].bounds;
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (940.0, 130.0, 420.0, 260.0)
    );
    assert_eq!(changes.get(), 1);
    assert!(!host.log.events.borrow().contains(&"set_bounds"));
    assert!(!host.log.events.borrow().contains(&"set_content"));
}

#[test]
fn floating_bounds_resize_callback_updates_the_current_root_once() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    docking.set_layout(floating_model_with_items(
        &docking.layout(),
        &["first"],
        Rect {
            x: 900.0,
            y: 100.0,
            width: 420.0,
            height: 260.0,
        },
    ));
    let host = hosts.borrow()[0].clone();
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    host.log.invoke_bounds_changed(Rect {
        x: 900.0,
        y: 100.0,
        width: 560.0,
        height: 340.0,
    });

    let bounds = docking.layout().snapshot().floating_roots[0].bounds;
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (900.0, 100.0, 560.0, 340.0)
    );
    assert_eq!(changes.get(), 1);
}

#[test]
fn floating_bounds_callback_survives_earlier_root_reindex() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let first = docking
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
    let two_floating = first
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
    let surviving = hosts.borrow()[1].clone();
    let remaining = docking.layout().with_item_closed(&item("first")).unwrap();
    docking.set_layout(remaining);
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    surviving.log.invoke_bounds_changed(Rect {
        x: 1510.0,
        y: 180.0,
        width: 500.0,
        height: 300.0,
    });

    let bounds = docking.layout().snapshot().floating_roots[0].bounds;
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (1510.0, 180.0, 500.0, 300.0)
    );
    assert_eq!(changes.get(), 1);
}

#[test]
fn floating_bounds_equal_native_echo_does_not_publish_again() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    docking.set_layout(floating_model_with_items(
        &docking.layout(),
        &["first"],
        Rect {
            x: 900.0,
            y: 100.0,
            width: 420.0,
            height: 260.0,
        },
    ));
    let host = hosts.borrow()[0].clone();
    let changes = Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));
    host.log.invoke_bounds_changed(Rect {
        x: 900.0,
        y: 100.0,
        width: 420.0,
        height: 260.0,
    });

    assert_eq!(changes.get(), 0);
}

#[test]
fn floating_bounds_callback_reentrant_during_native_sync_does_not_panic() {
    let docking = mounted_default_docking();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    docking.set_layout(floating_model_with_items(
        &docking.layout(),
        &["first"],
        Rect {
            x: 900.0,
            y: 100.0,
            width: 420.0,
            height: 260.0,
        },
    ));
    let host = hosts.borrow()[0].clone();
    host.log.invoke_bounds_changed_on_set.set(true);

    docking.set_layout(floating_model_with_items(
        &docking.layout(),
        &["first"],
        Rect {
            x: 940.0,
            y: 130.0,
            width: 560.0,
            height: 340.0,
        },
    ));

    let bounds = docking.layout().snapshot().floating_roots[0].bounds;
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (940.0, 130.0, 560.0, 340.0)
    );
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

    assert!(!hosts.borrow()[1].log.invoke_close());
    assert!(docking.layout().is_item_closed(&item("second")));
    assert_eq!(changes.get(), 1);
    assert_eq!(hosts.borrow()[1].log.close_count.get(), 0);
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

    assert!(!host.log.invoke_close());
    assert!(docking.layout().is_item_closed(&item("first")));
    assert!(docking.layout().is_item_closed(&item("second")));
    assert_eq!(changes.get(), 1);
    assert_eq!(host.log.close_count.get(), 0);
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
fn floating_native_close_removes_empty_authored_root_and_closes_one_host() {
    let docking = mounted_default_docking_with_keep_empty_tools();
    let hosts = Rc::new(RefCell::new(Vec::new()));
    docking.install_floating_host_factory_for_test(individual_fake_factory(hosts.clone()));
    let floating = docking
        .layout()
        .with_item_moved(
            &item("third"),
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

    assert!(!host.log.invoke_close());
    assert!(docking.layout().is_item_closed(&item("third")));
    assert!(docking.layout().snapshot().floating_roots.is_empty());
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
fn drag_commit_uses_the_resolved_center_insertion_index() {
    let model = default_model();
    let mut drag = test_drag(&model, item("first"));
    let mut target = resolved_target(
        DockTarget::Center,
        Some(SnapshotGroupKey::Authored(group("tools"))),
    );
    target.tab_insert_index = Some(0);
    drag.preview(&target, 1.0).unwrap();
    let committed = drag.commit().expect("preview should commit once");
    assert_eq!(
        snapshot_group_items(
            &committed.snapshot(),
            &SnapshotGroupKey::Authored(group("tools")),
        ),
        Some(vec![item("first"), item("third")])
    );
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
    let surface_size = Size {
        width: 900.0,
        height: 600.0,
    };
    assert_eq!(overlay.open(item("a"), DockSide::Left, surface_size), None);
    assert_eq!(overlay.current(), Some(item("a")));
    assert_eq!(
        overlay.open(item("b"), DockSide::Right, surface_size),
        Some(item("a"))
    );
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
fn authored_show_when_empty_group_keeps_its_empty_tab_view_without_hint_text() {
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

    // Like WinUI.Dock's `ShowWhenEmpty`, the empty group keeps its (empty) tab view and draws
    // no hint text of its own.
    let visible_text = find_all::<TextBlock>(docking.as_ref())
        .into_iter()
        .filter(|text| text.visibility() == Visibility::Visible)
        .filter_map(|text| {
            text.as_any()
                .downcast_ref::<TextBlock>()
                .map(|text| text.text.borrow().clone())
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    assert!(visible_text.is_empty(), "{visible_text:?}");
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
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );

    let splitters = find_all::<CustomGridSplitter>(docking.as_ref());
    assert_eq!(splitters.len(), 2);
    let split_view = find_all::<DockSplitView>(docking.as_ref())
        .into_iter()
        .next()
        .expect("split runtime should include the full-span handle layer");
    let split_view = split_view
        .as_any()
        .downcast_ref::<DockSplitView>()
        .expect("split handle layer should retain its control type");
    let split_grid = split_view
        .track_grid_for_test()
        .expect("split handle layer should retain its track Grid");
    assert_eq!(split_grid.columns.borrow().len(), 3);
    assert_eq!(split_grid.column_constraints.borrow().len(), 3);
    assert_eq!(split_grid.resolved_column_sizes().len(), 3);

    let children = split_grid.visual_children();
    assert_eq!(children.len(), 3);
    let panes = [&children[0], &children[1], &children[2]];
    for pair in panes.windows(2) {
        let previous_right =
            pair[0].arranged_offset().unwrap().x + pair[0].arranged_width().unwrap();
        let next_left = pair[1].arranged_offset().unwrap().x;
        assert!(
            ((next_left - previous_right) - 12.0).abs() < 0.01,
            "pane tracks should be separated by the reference 12 px Grid spacing"
        );
    }
    let track_sizes = split_grid.resolved_column_sizes();
    for (index, splitter) in splitters.iter().enumerate() {
        let splitter = splitter
            .as_any()
            .downcast_ref::<CustomGridSplitter>()
            .expect("runtime splitter should retain its control type");
        assert_eq!(splitter.resize_direction(), GridResizeDirection::Columns);
        assert_eq!(
            splitter.resize_behavior(),
            GridResizeBehavior::PreviousAndCurrent
        );
        assert_eq!(splitter.width(), Some(12.0));
        assert_eq!(
            splitter
                .as_ui_element()
                .get_attached::<i32>("Grid", "column", -1),
            (index + 1) as i32
        );
        assert_eq!(
            splitter
                .arranged_offset()
                .expect("splitter should be arranged")
                .x,
            track_sizes.iter().take(index + 1).sum::<f32>() + (index + 1) as f32 * 12.0
        );
        assert_eq!(splitter.arranged_width(), Some(12.0));
        assert_eq!(splitter.arranged_height(), Some(420.0));
        assert_eq!(splitter.visual_transform().translation.x, -12.0);

        // The splitter template is a full-size state background followed by the grip.
        let grip = splitter
            .visual_children()
            .into_iter()
            .next()
            .and_then(|root| root.visual_children().get(1).cloned())
            .expect("splitter should expose its grip visual");
        assert_eq!(grip.arranged_width(), Some(4.0));
        assert_eq!(grip.arranged_height(), Some(24.0));
        assert_eq!(grip.arranged_offset().unwrap().x, 4.0);
        assert_eq!(grip.arranged_offset().unwrap().y, 198.0);
    }
}

#[test]
fn selected_document_without_activation_has_no_active_chrome() {
    let docking = mounted_default_docking();
    assert_eq!(docking.layout().selected_item_id(), Some(item("first")));
    assert_eq!(docking.layout().active_item(), None);
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );

    let visible_markers = visible_active_document_markers(&docking);
    assert!(visible_markers.is_empty());
    let realization = docking.realization_for_test().unwrap();
    assert_eq!(realization.borrow().active_group_chrome_count_for_test(), 0);
}

#[test]
fn content_header_pin_click_auto_hides_the_item_while_the_press_activates_it() {
    let docking = mounted_bottom_documents_docking();
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 900.0,
        height: 600.0,
    };
    layout_root(&root, size);
    let header = docking
        .realization_for_test()
        .unwrap()
        .borrow()
        .group_content_header_for_test(&SnapshotGroupKey::Authored(group("documents")))
        .unwrap();
    let pin = header.visual_children()[1].clone();
    let bounds = SurfaceRegistry::bounds_in_host_root(&pin).unwrap();
    let center = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), center),
    );
    // The press activates the group, so its accent frame appears before the release; the
    // frame must not take the release away from the pin button.
    assert_eq!(docking.layout().active_item(), Some(item("first")));
    layout_root(&root, size);
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Released(MouseButton::Left), center),
    );
    assert!(docking.layout().is_item_auto_hidden(&item("first")));
    // The active Document left its group, so no docked group keeps an active frame.
    layout_root(&root, size);
    let realization = docking.realization_for_test().unwrap();
    assert!(
        realization
            .borrow()
            .visible_active_frames_for_test()
            .is_empty()
    );
    assert_eq!(realization.borrow().active_group_chrome_count_for_test(), 0);
}

#[test]
fn single_item_bottom_group_pin_click_auto_hides_its_item() {
    let (doc, _) = authored_item("doc", "Doc", true);
    let (errors, _) = authored_item("errors", "Error List", false);
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_children(vec![doc]);
    let error_group = DockGroup::new_group();
    error_group.set_id(group("errors"));
    error_group.set_tab_strip_position(TabStripPosition::Bottom);
    error_group.set_children(vec![errors]);
    let split = DockSplitPanel::new_panel();
    split.set_orientation(Orientation::Vertical);
    split.set_children(vec![
        documents as Rc<dyn UIElementExt>,
        error_group as Rc<dyn UIElementExt>,
    ]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 900.0,
        height: 600.0,
    };
    layout_root(&root, size);
    // Another group is active first, as in the demo after any earlier interaction.
    docking.handle_group_selected(SnapshotGroupKey::Authored(group("documents")), 0);
    layout_root(&root, size);
    let header = docking
        .realization_for_test()
        .unwrap()
        .borrow()
        .group_content_header_for_test(&SnapshotGroupKey::Authored(group("errors")))
        .unwrap();
    let pin = header
        .visual_children()
        .into_iter()
        .skip(1)
        .find(|child| child.visibility() == Visibility::Visible)
        .expect("visible pin action");
    let bounds = SurfaceRegistry::bounds_in_host_root(&pin).unwrap();
    let center = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), center),
    );
    layout_root(&root, size);
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Released(MouseButton::Left), center),
    );
    assert!(docking.layout().is_item_auto_hidden(&item("errors")));
}

#[test]
fn group_created_hook_chooses_the_presentation_of_runtime_created_groups() {
    let docking = mounted_default_docking();
    let asked = Rc::new(RefCell::new(Vec::new()));
    {
        let asked = asked.clone();
        docking.set_on_group_created(Box::new(move |args| {
            asked.borrow_mut().push(args.item.clone());
            crate::DockGroupOptions {
                tab_strip_position: TabStripPosition::Bottom,
                compact_tabs: true,
            }
        }));
    }
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 720.0,
        height: 420.0,
    };
    layout_root(&root, size);
    // Authored groups never ask the hook.
    assert!(asked.borrow().is_empty());

    let moved = docking
        .layout()
        .with_item_moved(
            &item("third"),
            DockPlacement::RootEdge {
                side: DockSide::Left,
                weight: 1.0,
            },
        )
        .unwrap();
    docking.set_layout(moved);
    layout_root(&root, size);
    assert_eq!(*asked.borrow(), vec![item("third")]);
    let views = find_all::<CustomTabView>(docking.as_ref());
    let generated = views
        .iter()
        .filter_map(|view| view.as_any().downcast_ref::<CustomTabView>())
        .find(|view| {
            view.children()
                .to_vec()
                .iter()
                .any(|tab| tab.header() == "Third")
        })
        .expect("the moved item has a runtime-created group");
    assert_eq!(generated.tab_strip_position(), TabStripPosition::Bottom);
    assert!(generated.compact());

    // The answer is kept for the group; later relayouts do not ask again.
    docking.set_layout(
        docking
            .layout()
            .with_item_activated(&item("first"))
            .unwrap(),
    );
    layout_root(&root, size);
    assert_eq!(asked.borrow().len(), 1);

    // A snapshot carries the generated group's identity, not its presentation answer.
    // A fresh runtime asks its own hook and can choose a different presentation.
    let restored = mounted_default_docking();
    let restored_asked = Rc::new(RefCell::new(Vec::new()));
    {
        let restored_asked = restored_asked.clone();
        restored.set_on_group_created(Box::new(move |args| {
            restored_asked.borrow_mut().push(args.item.clone());
            crate::DockGroupOptions::default()
        }));
    }
    restored.set_layout(DockLayoutModel::from_snapshot(docking.layout().snapshot()).unwrap());
    let restored_root: Rc<dyn UIElementExt> = restored.clone();
    layout_root(&restored_root, size);
    assert_eq!(*restored_asked.borrow(), vec![item("third")]);
    let views = find_all::<CustomTabView>(restored.as_ref());
    let generated = views
        .iter()
        .filter_map(|view| view.as_any().downcast_ref::<CustomTabView>())
        .find(|view| {
            view.children()
                .to_vec()
                .iter()
                .any(|tab| tab.header() == "Third")
        })
        .expect("restored generated group");
    assert_eq!(generated.tab_strip_position(), TabStripPosition::Top);
    assert!(!generated.compact());
}

#[test]
fn light_dismiss_press_outside_an_open_pane_does_not_reach_the_content_below() {
    let docking = mounted_default_docking();
    let pinned = docking
        .layout()
        .with_item_moved(
            &item("third"),
            DockPlacement::AutoHide {
                side: DockSide::Right,
            },
        )
        .unwrap()
        .with_item_activated(&item("third"))
        .unwrap();
    docking.set_layout(pinned);
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 720.0,
        height: 420.0,
    };
    layout_root(&root, size);
    assert_eq!(docking.layout().active_item(), Some(item("third")));

    // Press on the documents group, outside the open right pane.
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    let point = Point { x: 80.0, y: 200.0 };
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), point),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Released(MouseButton::Left), point),
    );
    // Like WinUI.Dock's light-dismiss popup, the press only dismisses the pane: the documents
    // group underneath is not activated.
    assert_eq!(docking.layout().active_item(), None);
    layout_root(&root, size);
    assert_eq!(
        docking
            .realization_for_test()
            .unwrap()
            .borrow()
            .open_auto_hide_item_on(&RootKind::Main),
        None
    );
}

#[test]
fn closing_the_active_document_activates_only_its_own_group_selection() {
    let docking = mounted_default_docking();
    // "first" and "second" share the documents group; "third" is alone in the tools group.
    let model = docking.layout();
    let closed = model
        .with_item_activated(&item("first"))
        .unwrap()
        .with_item_closed(&item("first"))
        .unwrap();
    assert_eq!(closed.active_item(), Some(item("second")));

    // WinUI.Dock leaves no active Document when the closed one's group becomes empty; it never
    // activates a Document of another group.
    let emptied = model
        .with_item_activated(&item("third"))
        .unwrap()
        .with_item_closed(&item("third"))
        .unwrap();
    assert_eq!(emptied.active_item(), None);
}

#[test]
fn active_group_frame_is_drawn_in_the_accent_color() {
    let docking = mounted_default_docking();
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 720.0,
        height: 420.0,
    };
    layout_root(&root, size);
    let realization = docking.realization_for_test().unwrap();
    assert!(
        realization
            .borrow()
            .visible_active_frames_for_test()
            .is_empty()
    );

    // Without a theme Primary the frame still uses a visible accent (WinUI.Dock draws the
    // active group border in the system accent).
    activate_first_document(&docking);
    layout_root(&root, size);
    let frames = realization.borrow().visible_active_frames_for_test();
    assert_eq!(frames.len(), 1);
    match &frames[0] {
        Some(crate::core::graphics::Brush::Solid(color)) => assert!(color.a > 0),
        other => panic!("active frame stroke should be a visible accent, got {other:?}"),
    }
    let seams = realization.borrow().active_group_seams_for_test();
    assert_eq!(seams.len(), 1);
    assert_eq!(seams[0].y, 32.5);
    assert_eq!(seams[0].height, 0.0);
    assert!(seams[0].width > 0.0 && seams[0].width <= 200.0);
}

fn sized_group(name: &str) -> Rc<DockGroup> {
    let dock_item = DockItem::new_item();
    dock_item.set_id(item(&format!("{name}-item")));
    dock_item.set_title(name.to_owned());
    dock_item.set_content(super::core::ui::TextBlock::new());
    let dock_group = DockGroup::new_group();
    dock_group.set_id(group(name));
    dock_group.set_children(vec![dock_item]);
    dock_group
}

fn first_split_grid(docking: &Rc<DockingControl>) -> Rc<Grid> {
    let split_view = find_all::<DockSplitView>(docking.as_ref())
        .into_iter()
        .next()
        .expect("the split realizes a DockSplitView");
    split_view
        .as_any()
        .downcast_ref::<DockSplitView>()
        .expect("split view type")
        .track_grid_for_test()
        .expect("split view retains its track grid")
}

#[test]
fn authored_group_width_and_limits_become_fixed_constrained_tracks() {
    let documents = sized_group("documents");
    documents.set_dock_size(DockSize {
        min_width: Some(120.0),
        ..DockSize::default()
    });
    let tools = sized_group("tools");
    tools.set_dock_size(DockSize::width(200.0));
    let split = DockSplitPanel::new_panel();
    split.set_orientation(Orientation::Horizontal);
    split.set_children(vec![
        documents as Rc<dyn UIElementExt>,
        tools as Rc<dyn UIElementExt>,
    ]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    for width in [900.0, 400.0] {
        layout_root(
            &root,
            Size {
                width,
                height: 300.0,
            },
        );
        let grid = first_split_grid(&docking);
        assert_eq!(grid.columns.borrow()[1], GridLength::Fixed(200.0));
        assert!(matches!(grid.columns.borrow()[0], GridLength::Star(_)));
        assert_eq!(grid.column_constraints.borrow()[0].min, Some(120.0));
        let sizes = grid.resolved_column_sizes();
        // The authored tool width holds while the star document column absorbs the change.
        assert_eq!(sizes[1], 200.0, "window width {width}");
        assert!((sizes[0] - (width - 200.0 - 12.0)).abs() < 0.5);
    }
}

#[test]
fn splitter_resized_fixed_track_keeps_its_new_extent_for_the_runtime() {
    let tools = sized_group("tools");
    tools.set_dock_size(DockSize::width(200.0));
    let split = DockSplitPanel::new_panel();
    split.set_orientation(Orientation::Horizontal);
    split.set_children(vec![
        sized_group("documents") as Rc<dyn UIElementExt>,
        tools as Rc<dyn UIElementExt>,
    ]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    let viewport = Size {
        width: 900.0,
        height: 300.0,
    };
    layout_root(&root, viewport);
    let grid = first_split_grid(&docking);
    let realization = docking.realization_for_test().unwrap();
    let address = SplitAddress {
        root: RootKind::Main,
        path: Vec::new(),
    };
    assert!(realization.borrow_mut().begin_splitter(
        &docking.layout(),
        address,
        0,
        grid.clone(),
        crate::Orientation::Horizontal,
    ));
    // The splitter owns the live Grid mutation; emulate a 60 px drag into the fixed track.
    grid.set_columns(vec![GridLength::Star(1.0), GridLength::Fixed(260.0)]);
    let next = realization
        .borrow_mut()
        .finish_splitter(false, -60.0)
        .expect("a completed drag publishes a model");
    docking.set_layout(next);
    layout_root(&root, viewport);
    assert_eq!(
        first_split_grid(&docking).columns.borrow()[1],
        GridLength::Fixed(260.0)
    );
}

#[test]
fn split_panel_height_applies_to_a_perpendicular_split_of_groups() {
    let top = sized_group("top");
    let bottom_split = DockSplitPanel::new_panel();
    bottom_split.set_orientation(Orientation::Horizontal);
    bottom_split.set_dock_size(DockSize::height(200.0));
    bottom_split.set_children(vec![
        sized_group("left") as Rc<dyn UIElementExt>,
        sized_group("right") as Rc<dyn UIElementExt>,
    ]);
    let split = DockSplitPanel::new_panel();
    split.set_orientation(Orientation::Vertical);
    split.set_children(vec![
        top as Rc<dyn UIElementExt>,
        bottom_split as Rc<dyn UIElementExt>,
    ]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 600.0,
            height: 500.0,
        },
    );
    let grid = first_split_grid(&docking);
    assert_eq!(grid.rows.borrow()[1], GridLength::Fixed(200.0));
    assert!(matches!(grid.rows.borrow()[0], GridLength::Star(_)));
}

#[test]
fn vertical_runtime_split_centers_row_grip_in_the_gutter() {
    let groups = (0..2)
        .map(|index| {
            let dock_item = DockItem::new_item();
            dock_item.set_id(item(&format!("vertical-split-item-{index}")));
            dock_item.set_title(format!("Vertical split item {index}"));
            dock_item.set_content(super::core::ui::TextBlock::new());

            let dock_group = DockGroup::new_group();
            dock_group.set_id(group(&format!("vertical-split-group-{index}")));
            dock_group.set_children(vec![dock_item]);
            dock_group
        })
        .collect::<Vec<_>>();
    let split = DockSplitPanel::new_panel();
    split.set_orientation(Orientation::Vertical);
    split.set_children(
        groups
            .into_iter()
            .map(|group| group as Rc<dyn UIElementExt>)
            .collect(),
    );

    let docking = DockingControl::__new_unmounted();
    docking.set_content(split);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );

    let split_view = find_all::<DockSplitView>(docking.as_ref())
        .into_iter()
        .next()
        .expect("vertical split should include the full-span handle layer");
    let split_view = split_view
        .as_any()
        .downcast_ref::<DockSplitView>()
        .expect("split handle layer should retain its control type");
    let split_grid = split_view
        .track_grid_for_test()
        .expect("split handle layer should retain its track Grid");
    assert_eq!(split_grid.rows.borrow().len(), 2);
    let panes = split_grid.visual_children();
    let previous_bottom =
        panes[0].arranged_offset().unwrap().y + panes[0].arranged_height().unwrap();
    let next_top = panes[1].arranged_offset().unwrap().y;
    assert!(((next_top - previous_bottom) - 12.0).abs() < 0.01);

    let splitter = find_all::<CustomGridSplitter>(docking.as_ref())
        .into_iter()
        .next()
        .expect("two-pane vertical split should have one splitter");
    let splitter = splitter
        .as_any()
        .downcast_ref::<CustomGridSplitter>()
        .expect("runtime splitter should retain its control type");
    assert_eq!(splitter.resize_direction(), GridResizeDirection::Rows);
    assert_eq!(splitter.height(), Some(12.0));
    assert_eq!(splitter.arranged_width(), Some(720.0));
    assert_eq!(splitter.arranged_height(), Some(12.0));
    let row_sizes = split_grid.resolved_row_sizes();
    assert!((row_sizes[0] + 12.0 - splitter.arranged_offset().unwrap().y).abs() < 0.01);
    assert_eq!(splitter.visual_transform().translation.y, -12.0);

    // The splitter template is a full-size state background followed by the grip.
    let grip = splitter
        .visual_children()
        .into_iter()
        .next()
        .and_then(|root| root.visual_children().get(1).cloned())
        .expect("splitter should expose its grip visual");
    assert_eq!(grip.arranged_width(), Some(24.0));
    assert_eq!(grip.arranged_height(), Some(4.0));
    assert_eq!(grip.arranged_offset().unwrap().x, 348.0);
    assert_eq!(grip.arranged_offset().unwrap().y, 4.0);
}

#[test]
fn active_document_marker_is_inside_the_tab_header_before_its_title() {
    let docking = mounted_default_docking();
    let active = docking
        .layout()
        .with_item_activated(&item("first"))
        .expect("a live document should be activatable");
    docking.set_layout(active);
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );

    let visible_markers = visible_active_document_markers(&docking);

    assert_eq!(visible_markers.len(), 1);
    let (item, marker) = &visible_markers[0];
    let item_bounds = SurfaceRegistry::bounds_in_host_root(item).expect("tab should be arranged");
    let marker_bounds =
        SurfaceRegistry::bounds_in_host_root(marker).expect("active marker should be arranged");
    assert!(
        (marker_bounds.x - (item_bounds.x + 12.0)).abs() < 0.01,
        "item={item_bounds:?}, marker={marker_bounds:?}"
    );
    assert!(marker_bounds.y >= item_bounds.y);
    assert!(marker_bounds.y + marker_bounds.height <= item_bounds.y + item_bounds.height);

    let title = find_all::<TextBlock>(item.as_ref())
        .into_iter()
        .find(|text| {
            text.as_any()
                .downcast_ref::<TextBlock>()
                .is_some_and(|text| text.text.borrow().as_str() == "First")
        })
        .expect("active tab title should remain visible");
    let title_bounds = SurfaceRegistry::bounds_in_host_root(&title).expect("title should arrange");
    assert!(title_bounds.x > marker_bounds.x + marker_bounds.width);
    let realization = docking.realization_for_test().unwrap();
    assert_eq!(realization.borrow().active_group_chrome_count_for_test(), 1);
}

#[test]
fn selecting_a_different_document_moves_the_active_marker_with_it() {
    let docking = mounted_default_docking();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let realization = docking.realization_for_test().unwrap();
    let group = SnapshotGroupKey::Authored(group("documents"));
    let view = realization
        .borrow()
        .group_for_test(&group)
        .expect("document group should be realized");

    assert!(view.select_index(1));

    assert_eq!(docking.layout().active_item(), Some(item("second")));
    let markers = visible_active_document_markers(&docking);
    assert_eq!(markers.len(), 1);
    let active_title = find_all::<TextBlock>(markers[0].0.as_ref())
        .into_iter()
        .find_map(|text| {
            text.as_any()
                .downcast_ref::<TextBlock>()
                .map(|text| text.text.borrow().clone())
        });
    assert_eq!(active_title.as_deref(), Some("Second"));
}

#[test]
fn pressing_the_selected_tab_of_another_group_activates_its_document() {
    let docking = mounted_default_docking();
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 720.0,
        height: 420.0,
    };
    layout_root(&root, size);
    let realization = docking.realization_for_test().unwrap();
    let documents = realization
        .borrow()
        .group_for_test(&SnapshotGroupKey::Authored(group("documents")))
        .expect("document group should be realized");
    let tools = realization
        .borrow()
        .group_for_test(&SnapshotGroupKey::Authored(group("tools")))
        .expect("tool group should be realized");
    assert!(documents.select_index(1));
    assert_eq!(docking.layout().active_item(), Some(item("second")));
    assert_eq!(tools.selected_index(), 0);

    // "Third" is already selected in its own group, so the press changes no selection; it must
    // still activate the Document and move the single active marker (WinUI.Dock activates on
    // every tab press).
    let press = PointerEventArgs {
        position: Point { x: 10.0, y: 10.0 },
        screen_position: None,
        button: Some(MouseButton::Left),
        modifiers: KeyModifiers::default(),
    };
    tools.forward_item_pointer_event(
        0,
        elwindui_custom_controls::TabItemPointerEvent::Pressed(press),
    );
    assert_eq!(docking.layout().active_item(), Some(item("third")));
    layout_root(&root, size);
    let markers = visible_active_document_markers(&docking);
    assert_eq!(markers.len(), 1);
    let active_title = find_all::<TextBlock>(markers[0].0.as_ref())
        .into_iter()
        .find_map(|text| {
            text.as_any()
                .downcast_ref::<TextBlock>()
                .map(|text| text.text.borrow().clone())
        });
    assert_eq!(active_title.as_deref(), Some("Third"));
}

#[test]
fn pressing_inside_a_document_activates_it_even_when_the_content_handles_the_press() {
    let (first, _) = authored_item("first", "First", true);
    let (third, third_page) = authored_item("third", "Third", true);
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_children(vec![first]);
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
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    activate_first_document(&docking);

    // The page consumes the press itself (like a button inside a tool window).
    third_page.register_routed_handler::<PointerEventArgs>(
        "on_pointer_pressed",
        Box::new(|_, args| args.handled.set(true)),
    );
    let press = PointerEventArgs {
        position: Point { x: 10.0, y: 10.0 },
        screen_position: None,
        button: Some(MouseButton::Left),
        modifiers: KeyModifiers::default(),
    };
    let page: Rc<dyn UIElementExt> = third_page.clone();
    super::core::ui::dispatch_routed(
        &page,
        "on_pointer_pressed",
        &press,
        &RoutedEventArgs::default(),
    );
    assert_eq!(docking.layout().active_item(), Some(item("third")));

    // A right press does not activate.
    activate_first_document(&docking);
    let right = PointerEventArgs {
        button: Some(MouseButton::Right),
        ..press
    };
    super::core::ui::dispatch_routed(
        &page,
        "on_pointer_pressed",
        &right,
        &RoutedEventArgs::default(),
    );
    assert_eq!(docking.layout().active_item(), Some(item("first")));
}

fn activate_first_document(docking: &DockingControl) {
    docking.handle_group_selected(SnapshotGroupKey::Authored(group("documents")), 0);
    assert_eq!(docking.layout().active_item(), Some(item("first")));
}

fn visible_active_document_markers(
    docking: &DockingControl,
) -> Vec<(Rc<dyn UIElementExt>, Rc<dyn UIElementExt>)> {
    let mut visible_markers = Vec::new();
    for wrapper in find_all::<CustomTabViewItem>(docking) {
        let Some(item) = wrapper.as_any().downcast_ref::<CustomTabViewItem>() else {
            continue;
        };
        for marker in find_all::<Rectangle>(item) {
            if marker.width() == Some(4.0)
                && marker.height() == Some(16.0)
                && marker.visibility() == Visibility::Visible
            {
                visible_markers.push((wrapper.clone(), marker));
            }
        }
    }
    visible_markers
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
    let theme_refreshes_before_selection = docking
        .realization_for_test()
        .expect("mounted docking has a realization")
        .borrow()
        .theme_refresh_count_for_test();
    let second_bounds =
        SurfaceRegistry::bounds_in_host_root(&(wrappers[1].clone() as Rc<dyn UIElementExt>))
            .expect("second tab should be arranged");
    let second_center = Point {
        x: second_bounds.x + 24.0,
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
            .theme_refresh_count_for_test(),
        theme_refreshes_before_selection,
        "selection-only changes must not refresh retained Docking theme state"
    );
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
fn selection_does_not_remeasure_an_unrelated_sibling() {
    let (first_page, second_page) = (TextBlock::new(), TextBlock::new());
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

    let probe = DockingMeasureProbe::new(Size {
        width: 80.0,
        height: 40.0,
    });
    let host = Grid::new();
    host.set_rows(vec![GridLength::Star(1.0), GridLength::Fixed(60.0)]);
    docking.set_attached("Grid", "row", 0i32);
    probe.set_attached("Grid", "row", 1i32);
    host.children().add(docking.clone());
    host.children().add(probe.clone());
    let root: Rc<dyn UIElementExt> = host.clone();
    let size = Size {
        width: 720.0,
        height: 420.0,
    };
    layout_root(&root, size);
    probe.reset_measure_count();
    let relayout_host = Rc::new(SiblingMeasureRelayoutHost {
        sibling: probe.clone(),
        dirty_groups: RefCell::new(Vec::new()),
    });
    host.set_invalidate_host(Some(relayout_host.clone()));

    let wrappers = find_all::<CustomTabViewItem>(docking.as_ref());
    let second_bounds =
        SurfaceRegistry::bounds_in_host_root(&(wrappers[1].clone() as Rc<dyn UIElementExt>))
            .expect("second tab should be arranged");
    let second_center = Point {
        x: second_bounds.x + 24.0,
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
    relayout_host.layout_dirty_sibling();

    assert!(docking.layout().is_item_active(&item("second")));
    assert_eq!(
        probe.measure_count(),
        0,
        "selection must not recursively invalidate an unrelated sibling"
    );
}

#[test]
fn runtime_theme_refresh_gate_is_idempotent() {
    let docking = mounted_default_docking();
    let realization = docking
        .realization_for_test()
        .expect("mounted docking has a realization");
    let initial_refreshes = realization.borrow().theme_refresh_count_for_test();

    docking.refresh_runtime_theme_for_test(false);
    assert_eq!(
        realization.borrow().theme_refresh_count_for_test(),
        initial_refreshes,
        "an unchanged theme signature must not refresh retained runtime chrome"
    );

    docking.refresh_runtime_theme_for_test(true);
    assert_eq!(
        realization.borrow().theme_refresh_count_for_test(),
        initial_refreshes + 1,
        "a changed BrushStyle signature must refresh exactly once"
    );

    docking.refresh_runtime_theme_for_test(true);
    assert_eq!(
        realization.borrow().theme_refresh_count_for_test(),
        initial_refreshes + 1,
        "repeating the changed signature must not refresh again"
    );
}

#[test]
fn runtime_theme_signature_resets_and_reinitializes_with_runtime_lifecycle() {
    let docking = mounted_default_docking();
    assert!(docking.has_runtime_theme_signature_for_test());

    let root: Rc<dyn UIElementExt> = docking.clone();
    unmount_subtree(&root);
    assert!(!docking.has_runtime_theme_signature_for_test());

    let remounted = mounted_default_docking();
    assert!(remounted.has_runtime_theme_signature_for_test());
}

#[test]
fn actual_tab_pointer_path_starts_and_cancels_docking_drag_after_four_pixels() {
    let docking = mounted_default_docking();
    // A press now activates the pressed Document (WinUI.Dock); start from it active so the
    // gesture under test is the only model change.
    activate_first_document(&docking);
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
        x: bounds.x + 24.0,
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
fn top_tab_strip_blank_space_does_not_start_a_group_drag() {
    let docking = mounted_default_docking();
    let original = docking.layout();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 900.0,
            height: 600.0,
        },
    );
    let (documents, group_bounds) = arranged_group_point(&docking, "documents");
    assert_eq!(documents.tab_strip_position(), TabStripPosition::Top);
    let last_tab_boundary = documents
        .tab_insertion_boundary(2)
        .expect("two document tabs have an ending boundary");
    let tab_strip_right = last_tab_boundary.x + last_tab_boundary.width;
    assert!(
        tab_strip_right < group_bounds.width,
        "fixture needs blank strip area"
    );
    let header = docking
        .realization_for_test()
        .and_then(|realization| {
            realization
                .borrow()
                .group_content_header_for_test(&SnapshotGroupKey::Authored(group("documents")))
        })
        .expect("the group host retains its optional content header");
    assert_eq!(header.visibility(), Visibility::Collapsed);

    let start = Point {
        x: group_bounds.x + (tab_strip_right + group_bounds.width) * 0.5,
        y: group_bounds.y + last_tab_boundary.y + last_tab_boundary.height * 0.5,
    };
    let (_, target_bounds) = arranged_group_point(&docking, "tools");
    let target = Point {
        x: target_bounds.x + target_bounds.width * 0.5,
        y: target_bounds.y + target_bounds.height * 0.5,
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
        pointer_event(RawPointerEventKind::Moved, target),
    );
    let realization = docking.realization_for_test().unwrap();
    assert!(!realization.borrow().active_drag_for_test());
    assert_eq!(docking.layout(), original);
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Released(MouseButton::Left), target),
    );
    assert_eq!(docking.layout(), original);
}

#[test]
fn bottom_header_selection_updates_title_and_capabilities_without_reconciliation() {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", false);
    second.set_can_pin(false);
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_tab_strip_position(TabStripPosition::Bottom);
    documents.set_children(vec![first, second]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(documents);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 400.0,
            height: 300.0,
        },
    );
    let realization = docking.realization_for_test().unwrap();
    let key = SnapshotGroupKey::Authored(group("documents"));
    let header = realization
        .borrow()
        .group_content_header_for_test(&key)
        .unwrap();
    let view = realization.borrow().group_for_test(&key).unwrap();
    let reconciles = realization.borrow().full_reconcile_count_for_test();
    let title = header.visual_children()[0].clone();
    let actions = header.visual_children();

    assert!(view.select_index(1));
    assert_eq!(
        title
            .as_any()
            .downcast_ref::<TextBlock>()
            .unwrap()
            .text
            .borrow()
            .as_str(),
        "Second"
    );
    assert_eq!(actions[1].visibility(), Visibility::Collapsed);
    assert_eq!(actions[2].visibility(), Visibility::Collapsed);
    assert_eq!(
        realization.borrow().full_reconcile_count_for_test(),
        reconciles
    );

    assert!(view.select_index(0));
    assert_eq!(
        title
            .as_any()
            .downcast_ref::<TextBlock>()
            .unwrap()
            .text
            .borrow()
            .as_str(),
        "First"
    );
    assert_eq!(actions[1].visibility(), Visibility::Visible);
    assert_eq!(actions[2].visibility(), Visibility::Visible);
    assert_eq!(
        realization.borrow().full_reconcile_count_for_test(),
        reconciles
    );
    assert!(Rc::ptr_eq(&title, &header.visual_children()[0]));

    let before_right_release = docking.layout();
    let right_release = PointerEventArgs {
        position: Point { x: 10.0, y: 10.0 },
        screen_position: None,
        button: Some(MouseButton::Right),
        modifiers: KeyModifiers::default(),
    };
    for action in &actions[1..=2] {
        super::core::ui::dispatch_routed(
            action,
            "on_pointer_released",
            &right_release,
            &RoutedEventArgs::default(),
        );
    }
    assert_eq!(docking.layout(), before_right_release);

    // Moving the selected last tab away leaves the remaining page and its header in sync.
    assert!(view.select_index(1));
    let title_updates = Rc::new(RefCell::new(Vec::new()));
    struct TitleUpdateHost(Rc<RefCell<Vec<u64>>>);
    impl RelayoutHost for TitleUpdateHost {
        fn request_relayout(&self, id: u64, _kind: InvalidationKind) {
            self.0.borrow_mut().push(id);
        }
    }
    root.set_invalidate_host(Some(Rc::new(TitleUpdateHost(title_updates.clone()))));
    let next = {
        let mut current = realization.borrow_mut();
        current
            .begin_drag(&docking.layout(), item("second"), Point { x: 0.0, y: 0.0 })
            .unwrap();
        let target = current
            .target_for_drop(None, Point { x: 18.0, y: 150.0 })
            .unwrap();
        assert_eq!(target.target, DockTarget::DockLeft);
        current.preview_drag(&target, 1.0).unwrap();
        current.finish_drag(true).unwrap()
    };
    docking.set_layout(next);
    layout_root(
        &root,
        Size {
            width: 400.0,
            height: 300.0,
        },
    );
    assert_eq!(
        title
            .as_any()
            .downcast_ref::<TextBlock>()
            .unwrap()
            .text
            .borrow()
            .as_str(),
        "First"
    );
    assert_eq!(view.children().len(), 1);
    assert!(
        title_updates.borrow().contains(&title.render_group_id()),
        "the reattached header must notify the host to replace its retained text commands"
    );
}

#[test]
fn content_header_packs_visible_actions_at_the_trailing_edge() {
    let (document, _) = authored_item("fixed", "Error List", false);
    let tools = DockGroup::new_group();
    tools.set_id(group("tools"));
    tools.set_tab_strip_position(TabStripPosition::Bottom);
    tools.set_children(vec![document]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(tools);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 400.0,
            height: 300.0,
        },
    );
    let header = docking
        .realization_for_test()
        .unwrap()
        .borrow()
        .group_content_header_for_test(&SnapshotGroupKey::Authored(group("tools")))
        .unwrap();
    let header_bounds =
        SurfaceRegistry::bounds_in_host_root(&(header.clone() as Rc<dyn UIElementExt>)).unwrap();
    let visible: Vec<_> = header
        .visual_children()
        .into_iter()
        .skip(1)
        .filter(|action| action.visibility() == Visibility::Visible)
        .collect();
    // The item cannot close, so only the pin is visible, and it sits at the trailing inset.
    assert_eq!(visible.len(), 1);
    let pin = SurfaceRegistry::bounds_in_host_root(&visible[0]).unwrap();
    assert!(
        (pin.x + pin.width - (header_bounds.x + header_bounds.width - 8.0)).abs() < 0.01,
        "pin {pin:?} should end at the header's trailing inset {header_bounds:?}"
    );
}

#[test]
fn bottom_content_header_wraps_title_and_centers_trailing_actions() {
    let (document, page) = authored_item(
        "wrapped",
        "A long selected document title that needs multiple lines",
        true,
    );
    let documents = DockGroup::new_group();
    documents.set_id(group("documents"));
    documents.set_tab_strip_position(TabStripPosition::Bottom);
    documents.set_children(vec![document]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(documents);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 180.0,
            height: 400.0,
        },
    );
    let header = docking
        .realization_for_test()
        .unwrap()
        .borrow()
        .group_content_header_for_test(&SnapshotGroupKey::Authored(group("documents")))
        .unwrap();
    let header_bounds =
        SurfaceRegistry::bounds_in_host_root(&(header.clone() as Rc<dyn UIElementExt>)).unwrap();
    assert!(header_bounds.height > 40.0, "{header_bounds:?}");
    let title = header.visual_children()[0].clone();
    assert_eq!(
        title
            .as_any()
            .downcast_ref::<TextBlock>()
            .unwrap()
            .text_wrapping(),
        crate::core::graphics::TextWrapping::Wrap
    );
    let title_bounds = SurfaceRegistry::bounds_in_host_root(&title).unwrap();
    for action in header.visual_children().into_iter().skip(1) {
        let bounds = SurfaceRegistry::bounds_in_host_root(&action).unwrap();
        assert_eq!(bounds.width, 24.0);
        assert_eq!(bounds.height, 24.0);
        assert!(bounds.x >= title_bounds.x + title_bounds.width);
        assert!((bounds.y + 12.0 - (header_bounds.y + header_bounds.height * 0.5)).abs() < 0.01);
    }
    assert_eq!(page.arranged_height(), Some(400.0 - header_bounds.height));
}

#[test]
fn content_header_actions_follow_width_after_auto_hide_strip_appears() {
    let (kept, kept_page) = authored_item("kept", "Kept", true);
    let (hidden, _) = authored_item("hidden", "Hidden", true);
    let tools = DockGroup::new_group();
    tools.set_id(group("tools"));
    tools.set_tab_strip_position(TabStripPosition::Bottom);
    tools.set_children(vec![kept, hidden]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(tools);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    let viewport = Size {
        width: 400.0,
        height: 300.0,
    };
    layout_root(&root, viewport);

    let auto_hidden = docking
        .layout()
        .with_item_moved(
            &item("hidden"),
            DockPlacement::AutoHide {
                side: DockSide::Right,
            },
        )
        .expect("auto-hide placement should be valid");
    docking.set_layout(auto_hidden);
    layout_root(&root, viewport);

    let header = docking
        .realization_for_test()
        .unwrap()
        .borrow()
        .group_content_header_for_test(&SnapshotGroupKey::Authored(group("tools")))
        .unwrap();
    let header_bounds =
        SurfaceRegistry::bounds_in_host_root(&(header.clone() as Rc<dyn UIElementExt>)).unwrap();
    assert!(
        header_bounds.x + header_bounds.width <= 400.0 - 28.0 + 0.01,
        "header must yield the right strip: {header_bounds:?}"
    );
    assert_eq!(kept_page.arranged_width(), Some(400.0 - 28.0));
    for action in header.visual_children().into_iter().skip(1) {
        if action.visibility() != Visibility::Visible {
            continue;
        }
        let bounds = SurfaceRegistry::bounds_in_host_root(&action).unwrap();
        assert!(
            bounds.x + bounds.width <= header_bounds.x + header_bounds.width + 0.01,
            "action {bounds:?} must stay inside header {header_bounds:?}"
        );
    }
}

#[test]
fn group_body_hover_keeps_the_compass_without_resolving_a_target() {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", true);
    let docking = mounted_docking_with_items(vec![first, second]);
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 600.0,
            height: 400.0,
        },
    );
    let realization = docking.realization_for_test().unwrap();
    realization
        .borrow_mut()
        .begin_drag(&docking.layout(), item("second"), Point { x: 0.0, y: 0.0 })
        .expect("drag should begin");

    let body = realization
        .borrow()
        .resolve_drop(None, Point { x: 300.0, y: 120.0 })
        .expect("the pointer is over the main surface");
    assert_eq!(body.root, RootKind::Main);
    assert!(body.hovered_group.is_some());
    assert_eq!(body.target, None);

    let hovered = body.hovered_group.unwrap();
    let center = Point {
        x: hovered.x + hovered.width * 0.5,
        y: hovered.y + hovered.height * 0.5,
    };
    let on_cell = realization
        .borrow()
        .resolve_drop(None, center)
        .and_then(|resolution| resolution.target)
        .expect("the drawn Center cell resolves");
    assert_eq!(on_cell.target, DockTarget::Center);
}

#[test]
fn bottom_group_compass_centers_on_the_frame_including_its_content_header() {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", true);
    let tools = DockGroup::new_group();
    tools.set_id(group("tools"));
    tools.set_tab_strip_position(TabStripPosition::Bottom);
    tools.set_children(vec![first, second]);
    let docking = DockingControl::__new_unmounted();
    docking.set_content(tools);
    docking.mount(application_environment());
    assert!(docking.apply_template());
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 400.0,
            height: 300.0,
        },
    );
    let realization = docking.realization_for_test().unwrap();
    let header = realization
        .borrow()
        .group_content_header_for_test(&SnapshotGroupKey::Authored(group("tools")))
        .unwrap();
    let header_bounds =
        SurfaceRegistry::bounds_in_host_root(&(header as Rc<dyn UIElementExt>)).unwrap();
    realization
        .borrow_mut()
        .begin_drag(&docking.layout(), item("second"), Point { x: 0.0, y: 0.0 })
        .expect("drag should begin");
    let resolution = realization
        .borrow()
        .resolve_drop(None, Point { x: 200.0, y: 150.0 })
        .expect("pointer is over the surface");
    let frame = resolution.hovered_group.expect("the group is hovered");
    assert_eq!(
        frame.y, header_bounds.y,
        "frame starts at the content header"
    );
    assert_eq!(frame.y + frame.height * 0.5, 150.0);
    assert_eq!(
        resolution.target.map(|target| target.target),
        Some(DockTarget::Center)
    );
}

#[test]
fn dragged_tab_leaves_its_strip_until_the_drag_ends() {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", true);
    let docking = mounted_docking_with_items(vec![first, second]);
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 600.0,
            height: 400.0,
        },
    );
    let realization = docking.realization_for_test().unwrap();
    let wrapper = realization
        .borrow()
        .wrapper_for_test(&item("second"))
        .expect("authored item has a wrapper");
    // Drag the selected Document so the group must present its neighbour meanwhile.
    docking.handle_group_selected(SnapshotGroupKey::Authored(group("main")), 1);
    layout_root(
        &root,
        Size {
            width: 600.0,
            height: 400.0,
        },
    );
    let view_node = find_all::<CustomTabView>(docking.as_ref())
        .into_iter()
        .next()
        .expect("group view");
    let view = view_node
        .as_any()
        .downcast_ref::<CustomTabView>()
        .expect("group view type");
    assert_eq!(view.selected_index(), 1);
    realization
        .borrow_mut()
        .begin_drag(&docking.layout(), item("second"), Point { x: 0.0, y: 0.0 })
        .expect("drag should begin");
    assert_eq!(wrapper.visibility(), Visibility::Collapsed);
    assert_eq!(
        view.selected_index(),
        0,
        "the neighbour is shown during the drag"
    );
    // The hidden header gives up its strip width, so the remaining tab closes up at the start.
    layout_root(
        &root,
        Size {
            width: 600.0,
            height: 400.0,
        },
    );
    let first_wrapper = realization
        .borrow()
        .wrapper_for_test(&item("first"))
        .unwrap();
    assert_eq!(wrapper.arranged_width().unwrap_or(0.0), 0.0);
    assert!(first_wrapper.arranged_offset().unwrap().x <= 8.0);
    let original = docking.layout();
    assert_eq!(
        realization.borrow_mut().finish_drag(false),
        Some(original),
        "a canceled drag leaves the model unchanged"
    );
    assert_eq!(wrapper.visibility(), Visibility::Visible);
    assert_eq!(view.selected_index(), 1);
}

#[test]
fn committed_drop_activates_the_moved_document() {
    let (first, _) = authored_item("first", "First", true);
    let (second, _) = authored_item("second", "Second", true);
    let docking = mounted_docking_with_items(vec![first, second]);
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 600.0,
            height: 400.0,
        },
    );
    assert_ne!(docking.layout().active_item(), Some(item("second")));
    let realization = docking.realization_for_test().unwrap();
    let mut current = realization.borrow_mut();
    current
        .begin_drag(&docking.layout(), item("second"), Point { x: 0.0, y: 0.0 })
        .expect("drag should begin");
    let target = current
        .target_for_drop(None, Point { x: 34.0, y: 200.0 })
        .expect("root DockLeft target resolves where it is drawn");
    assert_eq!(target.target, DockTarget::DockLeft);
    current.preview_drag(&target, 1.0).unwrap();
    let next = current.finish_drag(true).expect("drop commits");
    assert_eq!(next.active_item(), Some(item("second")));
}

#[test]
fn auto_hide_dismissal_clears_activity_and_unpin_docks_to_the_same_root_edge() {
    let (main_item, _) = authored_item("main", "Main", true);
    let (hidden_item, _) = authored_item("hidden", "Hidden", true);
    let docking = mounted_docking_with_items(vec![main_item, hidden_item]);
    let root: Rc<dyn UIElementExt> = docking.clone();
    let viewport = Size {
        width: 600.0,
        height: 400.0,
    };
    layout_root(&root, viewport);
    let realization = docking.realization_for_test().unwrap();
    // A wide group pins to the nearer of Top/Bottom by the reference shape rule.
    assert_eq!(
        realization.borrow().pin_side(&item("hidden")),
        Some(DockSide::Top)
    );

    let auto_hidden = docking
        .layout()
        .with_item_moved(
            &item("hidden"),
            DockPlacement::AutoHide {
                side: DockSide::Right,
            },
        )
        .unwrap();
    docking.set_layout(auto_hidden);
    layout_root(&root, viewport);

    docking.handle_auto_hide_open(RootKind::Main, item("hidden"));
    assert_eq!(docking.layout().active_item(), Some(item("hidden")));
    docking.handle_auto_hide_dismissed(item("hidden"));
    assert_eq!(docking.layout().active_item(), None);
    assert!(docking.layout().is_item_auto_hidden(&item("hidden")));

    docking.handle_auto_hide_open(RootKind::Main, item("hidden"));
    layout_root(&root, viewport);
    // Reopening after a light dismissal presents the pane again.
    assert_eq!(
        realization.borrow().open_auto_hide_item_on(&RootKind::Main),
        Some(item("hidden"))
    );
    docking.handle_pin_gesture(RootKind::Main);
    let model = docking.layout();
    assert!(!model.is_item_auto_hidden(&item("hidden")));
    assert_eq!(model.active_item(), Some(item("hidden")));
    let Some(SnapshotNode::Split {
        orientation,
        children,
    }) = model.snapshot().main_root
    else {
        panic!("unpinning wraps the root in a root-edge split");
    };
    assert_eq!(orientation, SnapshotOrientation::Horizontal);
    assert!(matches!(
        &children.last().unwrap().node,
        SnapshotNode::Group { items, .. } if items == &vec![item("hidden")]
    ));
    // The unpinned side becomes the item's preferred pin side.
    layout_root(&root, viewport);
    assert_eq!(
        realization.borrow().pin_side(&item("hidden")),
        Some(DockSide::Right)
    );
}

#[test]
fn auto_hide_pane_opened_through_the_control_uses_one_third_of_the_center() {
    let (main_item, _) = authored_item("main", "Main", true);
    let (hidden_item, _) = authored_item("hidden", "Hidden", true);
    let docking = mounted_docking_with_items(vec![main_item, hidden_item]);
    let root: Rc<dyn UIElementExt> = docking.clone();
    let viewport = Size {
        width: 928.0,
        height: 549.0,
    };
    layout_root(&root, viewport);
    let auto_hidden = docking
        .layout()
        .with_item_moved(
            &item("hidden"),
            DockPlacement::AutoHide {
                side: DockSide::Right,
            },
        )
        .unwrap();
    docking.set_layout(auto_hidden);
    layout_root(&root, viewport);
    docking.handle_auto_hide_open(RootKind::Main, item("hidden"));
    layout_root(&root, viewport);
    let (pane, _) = docking
        .realization_for_test()
        .unwrap()
        .borrow()
        .auto_hide_parts_for_test(&RootKind::Main)
        .unwrap();
    let width = pane.arranged_width().expect("open pane is arranged");
    assert!(
        (width - (928.0 - 28.0) / 3.0).abs() < 1.0,
        "pane width {width} should be one third of the usable center"
    );

    let body = pane.visual_children()[0].clone();
    let header = body.visual_children()[0].clone();
    let actions = header.visual_children();
    let before_right_release = docking.layout();
    let mut release = PointerEventArgs {
        position: Point { x: 10.0, y: 10.0 },
        screen_position: None,
        button: Some(MouseButton::Right),
        modifiers: KeyModifiers::default(),
    };
    for action in &actions[1..=2] {
        super::core::ui::dispatch_routed(
            action,
            "on_pointer_released",
            &release,
            &RoutedEventArgs::default(),
        );
    }
    assert_eq!(docking.layout(), before_right_release);
    release.button = Some(MouseButton::Left);
    super::core::ui::dispatch_routed(
        &actions[2],
        "on_pointer_released",
        &release,
        &RoutedEventArgs::default(),
    );
    assert!(docking.layout().is_item_closed(&item("hidden")));
    assert!(!docking.layout().is_item_closed(&item("main")));
}

#[test]
fn bottom_content_header_drags_only_the_selected_document() {
    let docking = mounted_bottom_documents_docking();
    // A press now activates the pressed Document (WinUI.Dock); start from it active so the
    // gesture under test is the only model change.
    activate_first_document(&docking);
    let original = docking.layout();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 900.0,
            height: 600.0,
        },
    );
    let header = docking
        .realization_for_test()
        .and_then(|realization| {
            realization
                .borrow()
                .group_content_header_for_test(&SnapshotGroupKey::Authored(group("documents")))
        })
        .expect("bottom group should have a selected-document content header");
    assert_eq!(header.visibility(), Visibility::Visible);
    let header_node: Rc<dyn UIElementExt> = header;
    let header_bounds = SurfaceRegistry::bounds_in_host_root(&header_node)
        .expect("bottom content header should be arranged");
    let start = Point {
        x: header_bounds.x + header_bounds.width * 0.5,
        y: header_bounds.y + header_bounds.height * 0.5,
    };
    let (_, target_bounds) = arranged_group_point(&docking, "tools");
    let target = Point {
        x: target_bounds.x + target_bounds.width * 0.5,
        y: target_bounds.y + target_bounds.height * 0.5,
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
        pointer_event(RawPointerEventKind::Moved, target),
    );
    let realization = docking.realization_for_test().unwrap();
    assert!(realization.borrow().active_drag_for_test());
    assert_eq!(
        docking.layout(),
        original,
        "drag preview must not commit early"
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Released(MouseButton::Left), target),
    );
    assert_ne!(docking.layout(), original);
    let snapshot = docking.layout().snapshot();
    assert_eq!(
        snapshot_group_items(&snapshot, &SnapshotGroupKey::Authored(group("documents"))),
        Some(vec![item("second")]),
        "only the selected Document should leave the source group"
    );
    let tools = snapshot_group_items(&snapshot, &SnapshotGroupKey::Authored(group("tools")))
        .expect("target group should remain authored");
    assert!(tools.contains(&item("first")));
    assert!(tools.contains(&item("third")));
}

#[test]
fn actual_tab_pointer_path_commits_a_root_edge_drop_once() {
    let docking = mounted_default_docking();
    // A press now activates the pressed Document (WinUI.Dock); start from it active so the
    // gesture under test is the only model change.
    activate_first_document(&docking);
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
        x: bounds.x + 24.0,
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
        x: main_surface_bounds.x + 18.0,
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
fn actual_marker_boundary_matches_resolved_index_and_committed_order() {
    let docking = mounted_unequal_docking();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let (target_view, target_bounds) = arranged_group_point(&docking, "tools");
    let first_boundary = target_view
        .tab_insertion_boundary(0)
        .expect("first insertion boundary should be retained");
    let midpoint_boundary = target_view
        .tab_insertion_boundary(1)
        .expect("midpoint insertion boundary should be retained");
    let second_boundary = target_view
        .tab_insertion_boundary(2)
        .expect("second insertion boundary should be retained");
    assert!(second_boundary.x > first_boundary.x);
    let target = Point {
        x: target_bounds.x + midpoint_boundary.x + 1.0,
        y: target_bounds.y + midpoint_boundary.y + midpoint_boundary.height * 0.5,
    };
    assert_eq!(
        target_view.tab_insertion_index_at(Point {
            x: midpoint_boundary.x + 1.0,
            y: midpoint_boundary.y + midpoint_boundary.height * 0.5,
        }),
        Some(1)
    );
    let source = first_tab_in_group(&docking, "documents");
    let source_node: Rc<dyn UIElementExt> = source;
    let source_bounds = SurfaceRegistry::bounds_in_host_root(&source_node).unwrap();
    let start = Point {
        x: source_bounds.x + 24.0,
        y: source_bounds.y + source_bounds.height * 0.5,
    };
    let original = docking.layout();
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
        pointer_event(RawPointerEventKind::Moved, target),
    );

    let realization = docking.realization_for_test().unwrap();
    let resolved = realization
        .borrow()
        .target_for_drop(None, target)
        .expect("center target should resolve from the actual pointer path");
    assert_eq!(resolved.target, DockTarget::Center);
    assert_eq!(resolved.tab_insert_index, Some(1));
    let marker = realization
        .borrow()
        .insertion_marker_for_test(&RootKind::Main)
        .expect("the retained insertion marker should be visible");
    assert_eq!(
        marker,
        Rect {
            x: target_bounds.x + midpoint_boundary.x,
            y: target_bounds.y + midpoint_boundary.y,
            width: 2.0,
            height: midpoint_boundary.height,
        }
    );

    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Released(MouseButton::Left), target),
    );
    assert_ne!(docking.layout(), original);
    assert_eq!(
        snapshot_group_items(
            &docking.layout().snapshot(),
            &SnapshotGroupKey::Authored(group("tools"))
        ),
        Some(vec![
            item("short"),
            item("source"),
            item("long"),
            item("tail")
        ])
    );
}

#[test]
fn actual_item_pointer_path_respects_float_capability() {
    let docking = mounted_capability_docking(false, true, true, true);
    // A press now activates the pressed Document (WinUI.Dock); start from it active so the
    // gesture under test is the only model change.
    activate_first_document(&docking);
    let original = docking.layout();
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
    let source = first_tab_in_group(&docking, "documents");
    let source_node: Rc<dyn UIElementExt> = source;
    let bounds = SurfaceRegistry::bounds_in_host_root(&source_node).unwrap();
    let start = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let release = Point {
        x: 1000.0,
        y: 700.0,
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
        pointer_event(RawPointerEventKind::Moved, release),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event_with_screen(
            RawPointerEventKind::Released(MouseButton::Left),
            release,
            release,
        ),
    );

    assert_eq!(docking.layout(), original);
    assert_eq!(changes.get(), 0);
    assert_eq!(docking.layout().snapshot().floating_roots.len(), 0);
}

#[test]
fn actual_item_pointer_path_respects_dock_capability() {
    let docking = mounted_capability_docking(true, true, false, true);
    // A press now activates the pressed Document (WinUI.Dock); start from it active so the
    // gesture under test is the only model change.
    activate_first_document(&docking);
    let original = docking.layout();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let source = first_tab_in_group(&docking, "documents");
    let source_node: Rc<dyn UIElementExt> = source;
    let bounds = SurfaceRegistry::bounds_in_host_root(&source_node).unwrap();
    let start = Point {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.5,
    };
    let release = Point { x: 2.0, y: 210.0 };
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
        pointer_event(RawPointerEventKind::Moved, release),
    );
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Released(MouseButton::Left), release),
    );

    assert_eq!(docking.layout(), original);
    assert_eq!(changes.get(), 0);
}

#[test]
fn actual_tab_float_prepare_failure_leaves_model_and_wrapper_parent_unchanged() {
    let docking = mounted_default_docking();
    // A press now activates the pressed Document (WinUI.Dock); start from it active so the
    // gesture under test is the only model change.
    activate_first_document(&docking);
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
    // A press now activates the pressed Document (WinUI.Dock); start from it active so the
    // gesture under test is the only model change.
    activate_first_document(&docking);
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
        x: tab_bounds.x + 24.0,
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
        width: 400.0,
        height: 400.0,
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
fn screen_drop_on_floating_outer_edge_finds_the_floating_surface_without_a_root_target() {
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
    let resolution = realization
        .borrow()
        .resolve_drop(Some(Point { x: 934.0, y: 230.0 }), Point { x: 0.0, y: 0.0 })
        .expect("floating surface should contain the screen point");
    assert_eq!(resolution.root, RootKind::Floating(0));
    // The floating edge draws no root target, so it does not resolve one.
    assert_eq!(resolution.target, None);
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
fn actual_splitter_pointer_path_lets_grid_own_preview_and_commits_once_or_restores_on_cancel() {
    let (docking, _probes) = mounted_three_pane_probed_docking();
    let original = docking.layout();
    let root: Rc<dyn UIElementExt> = docking.clone();
    layout_root(
        &root,
        Size {
            width: 720.0,
            height: 420.0,
        },
    );
    let splitter = find_all::<CustomGridSplitter>(docking.as_ref())
        .into_iter()
        .next()
        .expect("three-pane runtime split should contain a splitter");
    let splitter_node: Rc<dyn UIElementExt> = splitter.clone();
    let mut bounds = SurfaceRegistry::bounds_in_host_root(&splitter_node)
        .expect("arranged splitter should have host-root bounds");
    let translation = splitter.presentation_visual_transform().translation;
    bounds.x += translation.x;
    bounds.y += translation.y;
    let split_view = find_all::<DockSplitView>(docking.as_ref())
        .into_iter()
        .find_map(|view| {
            view.as_any()
                .downcast_ref::<DockSplitView>()
                .and_then(DockSplitView::track_grid_for_test)
        })
        .expect("split runtime should retain its track Grid");
    let grid = split_view.as_ref();
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
    let relayout = RecordingRelayoutHost::new();
    root.as_ui_element()
        .set_invalidate_host(Some(relayout.clone() as Rc<dyn RelayoutHost>));
    let dispatcher = PointerDispatcher::new();
    let focus = FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        pointer_event(RawPointerEventKind::Pressed(MouseButton::Left), start),
    );
    for step in 1..=30 {
        dispatcher.handle(
            &root,
            &focus,
            pointer_event(
                RawPointerEventKind::Moved,
                Point {
                    x: start.x + 36.0 * step as f32 / 30.0,
                    y: start.y,
                },
            ),
        );
    }
    assert!(realization.borrow().active_splitter_for_test());
    assert_eq!(docking.layout(), original);
    assert_ne!(*grid.columns.borrow(), original_tracks);
    assert_eq!(relayout.flushes.get(), 30);
    let measure_requests = relayout
        .requests
        .borrow()
        .iter()
        .filter(|kind| **kind == InvalidationKind::Measure)
        .count();
    assert_eq!(measure_requests, 30);
    assert!(
        relayout
            .requests
            .borrow()
            .iter()
            .all(|kind| matches!(kind, InvalidationKind::Measure | InvalidationKind::Render))
    );
    assert_eq!(
        realization.borrow().full_reconcile_count_for_test(),
        full_reconciles_before_drag,
        "splitter preview must not reconcile the Dock runtime"
    );
    let preview_tracks = grid.columns.borrow().clone();
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
    assert_eq!(
        *grid.columns.borrow(),
        preview_tracks,
        "splitter release must not jump away from the previewed tracks"
    );
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
    let canceled_splitter = find_all::<CustomGridSplitter>(canceled.as_ref())
        .into_iter()
        .next()
        .expect("canceled split should contain a splitter");
    let canceled_node: Rc<dyn UIElementExt> = canceled_splitter.clone();
    let mut canceled_bounds = SurfaceRegistry::bounds_in_host_root(&canceled_node).unwrap();
    let translation = canceled_splitter
        .presentation_visual_transform()
        .translation;
    canceled_bounds.x += translation.x;
    canceled_bounds.y += translation.y;
    let canceled_start = Point {
        x: canceled_bounds.x + canceled_bounds.width * 0.5,
        y: canceled_bounds.y + canceled_bounds.height * 0.5,
    };
    let canceled_grid = find_all::<DockSplitView>(canceled.as_ref())
        .into_iter()
        .find_map(|view| {
            view.as_any()
                .downcast_ref::<DockSplitView>()
                .and_then(DockSplitView::track_grid_for_test)
        })
        .expect("canceled split should retain its track Grid");
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
fn splitter_preview_remeasures_retained_children_for_live_layout() {
    let (docking, probes) = mounted_three_pane_probed_docking();
    let root: Rc<dyn UIElementExt> = docking.clone();
    let size = Size {
        width: 720.0,
        height: 420.0,
    };
    layout_root(&root, size);
    let splitter = find_all::<CustomGridSplitter>(docking.as_ref())
        .into_iter()
        .next()
        .expect("probed split should contain a splitter");
    let splitter_node: Rc<dyn UIElementExt> = splitter.clone();
    let mut splitter_bounds = SurfaceRegistry::bounds_in_host_root(&splitter_node)
        .expect("splitter should have arranged bounds");
    let translation = splitter.presentation_visual_transform().translation;
    splitter_bounds.x += translation.x;
    splitter_bounds.y += translation.y;
    let grid = find_all::<DockSplitView>(docking.as_ref())
        .into_iter()
        .find_map(|view| {
            view.as_any()
                .downcast_ref::<DockSplitView>()
                .and_then(DockSplitView::track_grid_for_test)
        })
        .expect("split runtime should retain its track Grid");
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
    assert!(probes.iter().all(|probe| probe.measure_count() > 0));
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
