use super::DockTarget;
use super::Orientation;
use super::core::environment::application_environment;
use super::core::layout::Visibility;
use super::core::ui::{ContentControlExt, TextBlockExt, UIElementExt};
use super::core::visual_tree::find_all;
use super::id::{DockGroupId, DockItemId};
use super::model::{
    DefaultDockDefinition, DockLayoutModel, InternalDockGroupKey, Node, RootKind, SplitAddress,
    WeightedNode,
};
use super::placement::{DockLayoutError, DockPlacement, DockSide};
use super::runtime::{AutoHideOverlay, DragSession, DropPreview, LatestOnlyQueue};
use super::snapshot::{
    DockLayoutSnapshot, SnapshotGroupKey, SnapshotNode, SnapshotOrientation, SnapshotWeightedNode,
};
use super::{
    DockGroup, DockGroupExt, DockItem, DockItemExt, DockSplitPanel, DockSplitPanelExt,
    DockingControl, DockingControlExt,
};
use elwindui_core::base::Rect;
use elwindui_custom_controls::{CustomSplitter, CustomTabView, CustomTabViewItem};
use std::cell::Cell;

fn item(value: &str) -> DockItemId {
    DockItemId::from(value)
}

fn group(value: &str) -> DockGroupId {
    DockGroupId::from(value)
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
        assert_eq!(edge.snapshot().version(), 1);
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
fn drag_preview_commit_and_capture_loss_are_transactional() {
    let model = default_model();
    let mut drag = DragSession::begin(&model, item("first")).unwrap();
    let preview = drag
        .preview(
            DockTarget::Center,
            Some(SnapshotGroupKey::Authored(group("tools"))),
            1.0,
        )
        .unwrap()
        .clone();
    assert_ne!(preview, model);
    assert_eq!(drag.capture_lost(), model);
    assert!(drag.commit().is_none());

    let mut committed = DragSession::begin(&model, item("first")).unwrap();
    committed.preview(DockTarget::DockRight, None, 1.0).unwrap();
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
        let mut drag = DragSession::begin(&model, item("first")).unwrap();
        assert!(drag.preview(target, group.clone(), 1.0).is_ok());
    }
    for target in [
        DockTarget::DockLeft,
        DockTarget::DockTop,
        DockTarget::DockRight,
        DockTarget::DockBottom,
    ] {
        let mut drag = DragSession::begin(&model, item("first")).unwrap();
        assert!(drag.preview(target, None, 1.0).is_ok());
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

    let mut drag = DragSession::begin(&model, item("second")).unwrap();
    let preview = drag
        .preview(
            DockTarget::Center,
            Some(SnapshotGroupKey::Generated(*generated)),
            1.0,
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
    preview.set_target(DockTarget::SplitBottom);
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
    let changes = std::rc::Rc::new(Cell::new(0));
    let changes_for_callback = changes.clone();
    docking.set_on_layout_change(Box::new(move |_| {
        changes_for_callback.set(changes_for_callback.get() + 1);
    }));

    assert!(tab.select_index(1));
    assert!(docking.layout().is_item_active(&item("second")));
    assert_eq!(changes.get(), 1);

    assert!(tab.request_close(1));
    assert!(docking.layout().is_item_closed(&item("second")));
    assert_eq!(changes.get(), 2);
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
