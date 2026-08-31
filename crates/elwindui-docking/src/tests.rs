use super::DockTarget;
use super::Orientation;
use super::id::{DockGroupId, DockItemId};
use super::model::{
    DefaultDockDefinition, DockLayoutModel, InternalDockGroupKey, Node, WeightedNode,
};
use super::placement::{DockLayoutError, DockPlacement, DockSide};
use super::runtime::{AutoHideOverlay, DragSession, DropPreview, LatestOnlyQueue};
use super::snapshot::{
    DockLayoutSnapshot, SnapshotGroupKey, SnapshotNode, SnapshotOrientation, SnapshotWeightedNode,
};
use elwindui_core::base::Rect;

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
    assert!(
        auto_hidden
            .with_item_activated(&item("first"))
            .unwrap()
            .is_item_active(&item("first"))
    );
    let restored = DockLayoutModel::from_snapshot(auto_hidden.snapshot()).unwrap();
    assert_eq!(restored.snapshot(), auto_hidden.snapshot());
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
        .preview(DockTarget::Center, Some(group("tools")), 1.0)
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
