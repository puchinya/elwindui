use elwindui_custom_controls::core::base::{Point, Size};
use elwindui_custom_controls::core::graphics::{IconSource, RenderCommand, RenderTree, SystemIcon};
use elwindui_custom_controls::core::input::{
    KeyModifiers, MouseButton, PointerEventArgs, RoutedEventArgs, TappedEventArgs,
};
use elwindui_custom_controls::core::ui::{
    ContentControlExt, InvalidationKind, ListExt, RelayoutHost, UIElementExt, dispatch_routed,
    layout_root,
};
use elwindui_custom_controls::{
    CloseButtonPresentation, CustomSplitter, CustomSplitterExt, CustomTabView, CustomTabViewExt,
    CustomTabViewItem, CustomTabViewItemExt, Orientation, SplitterDragCompleted, TabDragCompleted,
    TabStripPosition,
};
use std::cell::RefCell;
use std::rc::Rc;

fn pointer(position: Point, button: Option<MouseButton>) -> PointerEventArgs {
    PointerEventArgs {
        position,
        screen_position: Some(position),
        button,
        modifiers: KeyModifiers::default(),
    }
}

#[test]
fn tab_view_owns_ordered_items_and_exposes_public_presentation_properties() {
    let first = CustomTabViewItem::new_item();
    first.set_header("first".to_string());
    let second = CustomTabViewItem::new_item();
    second.set_header("second".to_string());

    let view = CustomTabView::new_view();
    view.set_tab_position(TabStripPosition::Bottom);
    view.set_close_button_presentation(CloseButtonPresentation::Always);
    view.set_children(vec![first.clone(), second.clone()]);

    assert_eq!(view.children().len(), 2);
    assert_eq!(view.children().to_vec()[0].header(), "first");
    assert_eq!(view.tab_position(), TabStripPosition::Bottom);
    assert_eq!(
        view.close_button_presentation(),
        CloseButtonPresentation::Always
    );
    assert!(second.visual_parent().is_some());
    let view_node: Rc<dyn UIElementExt> = view.clone();
    assert!(Rc::ptr_eq(
        &second.visual_parent().expect("tab parent"),
        &view_node
    ));

    let replacement = CustomTabViewItem::new_item();
    view.set_children(vec![replacement.clone()]);
    assert!(first.visual_parent().is_none());
    assert!(second.visual_parent().is_none());
    assert!(replacement.visual_parent().is_some());
}

#[test]
fn typed_children_list_surface_preserves_order_and_detaches_removed_items() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    let list: &dyn ListExt<dyn CustomTabViewItemExt> = view.children();
    let first_ext: Rc<dyn CustomTabViewItemExt> = first.clone();
    let second_ext: Rc<dyn CustomTabViewItemExt> = second.clone();

    list.add(first_ext.clone());
    list.add(second_ext.clone());
    assert_eq!(list.len(), 2);
    assert!(Rc::ptr_eq(&list.to_vec()[0], &first_ext));
    assert!(Rc::ptr_eq(&list.to_vec()[1], &second_ext));

    assert!(list.remove(&first_ext));
    assert_eq!(list.len(), 1);
    assert!(Rc::ptr_eq(&list.to_vec()[0], &second_ext));
}

#[test]
fn selected_index_is_two_way_and_equal_selection_is_a_noop() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![first, second]);

    let changes = Rc::new(RefCell::new(Vec::new()));
    let changes_for_callback = changes.clone();
    view.set_on_selected_index_changed(move |index| {
        changes_for_callback.borrow_mut().push(index);
    });

    assert!(view.select_index(1));
    assert!(!view.select_index(1));
    view.select_index(0);
    assert_eq!(view.selected_index(), 0);
    assert_eq!(&*changes.borrow(), &[1, 0]);
}

#[test]
fn close_request_is_gated_without_removing_the_item() {
    let closeable = CustomTabViewItem::new_item();
    let protected = CustomTabViewItem::new_item();
    protected.set_closable(false);
    let view = CustomTabView::new_view();
    view.set_children(vec![closeable, protected]);

    let requests = Rc::new(RefCell::new(Vec::new()));
    let requests_for_callback = requests.clone();
    view.set_on_close_requested(move |request| {
        requests_for_callback.borrow_mut().push(request.index);
    });

    assert!(view.request_close(0));
    assert!(!view.request_close(1));
    assert_eq!(&*requests.borrow(), &[0]);
    assert_eq!(view.children().len(), 2);
}

#[test]
fn tab_pointer_sequence_emits_drag_payloads_and_cancel() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);

    let completed = Rc::new(RefCell::new(Vec::<TabDragCompleted>::new()));
    let completed_for_callback = completed.clone();
    view.set_on_tab_drag_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
    }));

    let target: Rc<dyn UIElementExt> = item;
    assert!(
        view.as_ui_element()
            .routed_handlers
            .borrow()
            .contains_key("on_pointer_pressed")
    );
    assert!(Rc::ptr_eq(
        &target.visual_parent().expect("tab parent"),
        &(view.clone() as Rc<dyn UIElementExt>)
    ));
    let args = RoutedEventArgs::default();
    let press = pointer(Point { x: 1.0, y: 2.0 }, Some(MouseButton::Left));
    dispatch_routed(&target, "on_pointer_pressed", &press, &args);
    let moved = pointer(Point { x: 6.0, y: 2.0 }, None);
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &moved,
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_released",
        &pointer(Point { x: 8.0, y: 2.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    assert_eq!(completed.borrow().len(), 1);
    assert!(!completed.borrow()[0].canceled);

    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &press,
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 6.0, y: 2.0 }, None),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_canceled",
        &pointer(Point { x: 2.0, y: 3.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(completed.borrow().len(), 2);
    assert!(completed.borrow()[1].canceled);
}

#[test]
fn close_pointer_sequence_never_selects_the_tab() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_close_button_presentation(CloseButtonPresentation::Always);
    view.set_children(vec![first.clone(), second.clone()]);

    let selection = Rc::new(RefCell::new(Vec::new()));
    let selection_for_callback = selection.clone();
    view.set_on_selected_index_change(Box::new(move |index| {
        selection_for_callback.borrow_mut().push(index);
    }));
    let close_requests = Rc::new(RefCell::new(Vec::new()));
    let close_requests_for_callback = close_requests.clone();
    view.set_on_close_request(Box::new(move |index| {
        close_requests_for_callback.borrow_mut().push(index);
    }));

    // Empty labels make each tab's deterministic 46px header fit the 20px close slot. The
    // second tab therefore has a close point at x=62, y=16 after its direct arrange pass.
    let size = Size {
        width: 240.0,
        height: 120.0,
    };
    view.measure_override(size);
    view.arrange_override(size);
    let target: Rc<dyn UIElementExt> = second;
    let close_point = Point { x: 64.0, y: 16.0 };
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(close_point, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_released",
        &pointer(close_point, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    // A normal dispatcher may also produce a tap after release. It must not reintroduce a
    // second selection interpretation for the close rectangle.
    dispatch_routed(
        &target,
        "on_tapped",
        &TappedEventArgs {
            position: close_point,
            modifiers: KeyModifiers::default(),
        },
        &RoutedEventArgs::default(),
    );

    assert_eq!(&*close_requests.borrow(), &[1]);
    assert_eq!(view.selected_index(), 0);
    assert!(selection.borrow().is_empty());
    assert_eq!(view.children().len(), 2);
    let _ = first;
}

#[test]
fn pointer_over_hover_transitions_request_render_only() {
    struct RecordingHost {
        kinds: Rc<RefCell<Vec<InvalidationKind>>>,
    }
    impl RelayoutHost for RecordingHost {
        fn request_relayout(&self, _dirty_group_id: u64, kind: InvalidationKind) {
            self.kinds.borrow_mut().push(kind);
        }
    }

    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_close_button_presentation(CloseButtonPresentation::OnPointerOver);
    view.set_children(vec![item.clone()]);
    let kinds = Rc::new(RefCell::new(Vec::new()));
    view.as_ui_element()
        .set_invalidate_host(Some(Rc::new(RecordingHost {
            kinds: kinds.clone(),
        })));

    let target: Rc<dyn UIElementExt> = item;
    let args = pointer(Point { x: 1.0, y: 1.0 }, None);
    dispatch_routed(
        &target,
        "on_pointer_entered",
        &args,
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &args,
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_exited",
        &args,
        &RoutedEventArgs::default(),
    );

    assert_eq!(
        &*kinds.borrow(),
        &[InvalidationKind::Render, InvalidationKind::Render]
    );
}

#[test]
fn drag_started_callback_removing_item_only_cancels() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let events = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let events_for_completed = events.clone();
    view.set_on_tab_drag_completed(Box::new(move |payload| {
        if payload.canceled {
            events_for_completed
                .borrow_mut()
                .push("completed(canceled=true)");
        } else {
            events_for_completed
                .borrow_mut()
                .push("completed(canceled=false)");
        }
    }));
    let weak_view = Rc::downgrade(&view);
    let events_for_started = events.clone();
    view.set_on_tab_drag_started(Box::new(move |_| {
        events_for_started.borrow_mut().push("started");
        weak_view
            .upgrade()
            .expect("view alive during callback")
            .set_children(Vec::new());
    }));

    let target: Rc<dyn UIElementExt> = item;
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 1.0, y: 1.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 8.0, y: 1.0 }, None),
        &RoutedEventArgs::default(),
    );

    assert_eq!(&*events.borrow(), &["started", "completed(canceled=true)"]);
    dispatch_routed(
        &target,
        "on_pointer_released",
        &pointer(Point { x: 8.0, y: 1.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    assert_eq!(&*events.borrow(), &["started", "completed(canceled=true)"]);
}

#[test]
fn drag_started_callback_reorder_refreshes_moved_index() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone(), second.clone()]);
    let size = Size {
        width: 240.0,
        height: 120.0,
    };
    view.measure_override(size);
    view.arrange_override(size);
    let started = Rc::new(RefCell::new(Vec::new()));
    let moved = Rc::new(RefCell::new(Vec::new()));
    let started_for_callback = started.clone();
    let weak_view = Rc::downgrade(&view);
    let first_for_callback = first.clone();
    let second_for_callback = second.clone();
    view.set_on_tab_drag_started(Box::new(move |payload| {
        started_for_callback.borrow_mut().push(payload.index);
        weak_view
            .upgrade()
            .expect("view alive during callback")
            .set_children(vec![
                second_for_callback.clone(),
                first_for_callback.clone(),
            ]);
    }));
    let moved_for_callback = moved.clone();
    view.set_on_tab_drag_moved(Box::new(move |payload| {
        moved_for_callback.borrow_mut().push(payload.index);
    }));

    let target: Rc<dyn UIElementExt> = first;
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 1.0, y: 1.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 8.0, y: 1.0 }, None),
        &RoutedEventArgs::default(),
    );

    assert_eq!(&*started.borrow(), &[0]);
    assert_eq!(&*moved.borrow(), &[1]);
}

#[test]
fn canceled_completion_reconciliation_restarts_from_reentrant_children() {
    let first = CustomTabViewItem::new_item();
    let replacement = CustomTabViewItem::new_item();
    let final_item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone()]);
    let completion_count = Rc::new(RefCell::new(0));
    let completion_count_for_callback = completion_count.clone();
    let weak_view = Rc::downgrade(&view);
    let final_for_callback = final_item.clone();
    view.set_on_tab_drag_completed(Box::new(move |payload| {
        if payload.canceled {
            *completion_count_for_callback.borrow_mut() += 1;
            weak_view
                .upgrade()
                .expect("view alive during callback")
                .set_children(vec![final_for_callback.clone()]);
        }
    }));

    let target: Rc<dyn UIElementExt> = first.clone();
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 1.0, y: 1.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 8.0, y: 1.0 }, None),
        &RoutedEventArgs::default(),
    );
    view.set_children(vec![replacement.clone()]);

    assert_eq!(*completion_count.borrow(), 1);
    assert_eq!(view.children().len(), 1);
    assert!(Rc::ptr_eq(
        &view.children().to_vec()[0],
        &(final_item.clone() as Rc<dyn CustomTabViewItemExt>)
    ));
    assert!(first.visual_parent().is_none());
    assert!(replacement.visual_parent().is_none());
    let view_node: Rc<dyn UIElementExt> = view;
    assert!(
        final_item
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &view_node))
    );
}

#[test]
fn removing_a_tab_during_drag_cancels_the_gesture() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let completed = Rc::new(RefCell::new(Vec::<TabDragCompleted>::new()));
    let completed_for_callback = completed.clone();
    view.set_on_tab_drag_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
    }));

    let target: Rc<dyn UIElementExt> = item;
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 1.0, y: 1.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 8.0, y: 1.0 }, None),
        &RoutedEventArgs::default(),
    );
    view.set_children(Vec::new());
    assert_eq!(completed.borrow().len(), 1);
    assert!(completed.borrow()[0].canceled);
}

#[test]
fn tab_icon_realization_uses_core_icon_source_element() {
    let item = CustomTabViewItem::new_item();
    item.set_icon(Some(IconSource::System(SystemIcon::Add)));
    let icon = item.realize_icon().expect("icon source should realize");
    assert!(icon.type_name().contains("IconSourceElement"));
}

#[test]
fn content_control_item_owns_one_logical_content_element() {
    let item = CustomTabViewItem::new_item();
    let content = elwindui_custom_controls::core::ui::TextBlock::new();
    item.set_content(content.clone());
    let item_node: Rc<dyn UIElementExt> = item;
    assert!(
        content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &item_node))
    );
}

#[test]
fn splitter_reports_logical_axis_delta_and_canceled_completion() {
    let splitter = CustomSplitter::new_splitter();
    splitter.set_orientation(Orientation::Horizontal);
    let completed = Rc::new(RefCell::new(Vec::<SplitterDragCompleted>::new()));
    let completed_for_callback = completed.clone();
    splitter.set_on_drag_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
    }));

    let target: Rc<dyn UIElementExt> = splitter;
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 2.0, y: 4.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 12.0, y: 4.0 }, None),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_released",
        &pointer(Point { x: 12.0, y: 4.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    assert_eq!(completed.borrow().len(), 1);
    assert_eq!(completed.borrow()[0].cumulative_delta, 10.0);
    assert!(!completed.borrow()[0].canceled);

    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 12.0, y: 4.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_canceled",
        &pointer(Point { x: 12.0, y: 4.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(completed.borrow().len(), 2);
    assert!(completed.borrow()[1].canceled);
}

#[test]
fn source_selection_is_not_echoed_and_invalid_selection_has_no_content() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone(), second.clone()]);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let changes_for_callback = changes.clone();
    view.set_on_selected_index_change(Box::new(move |index| {
        changes_for_callback.borrow_mut().push(index);
    }));

    view.set_selected_index(99);
    assert_eq!(view.selected_index(), 99);
    assert!(changes.borrow().is_empty());
    let size = Size {
        width: 240.0,
        height: 120.0,
    };
    // `#[component]` currently emits these hooks as inherent helpers rather than wiring them
    // into the inherited UIElement vtable. Exercise the approved hook logic directly here; the
    // host-level dispatch remains a C-class prerequisite tracked in the status document.
    view.measure_override(size);
    view.arrange_override(size);
    assert_eq!(first.arranged_width(), Some(0.0));
    assert_eq!(second.arranged_width(), Some(0.0));

    assert!(view.select_index(1));
    assert_eq!(&*changes.borrow(), &[1]);
}

#[test]
fn selected_content_is_arranged_below_top_strip_and_unselected_is_zero_clipped() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    first.set_content(elwindui_custom_controls::core::ui::TextBlock::new());
    second.set_content(elwindui_custom_controls::core::ui::TextBlock::new());
    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone(), second.clone()]);
    let size = Size {
        width: 240.0,
        height: 120.0,
    };
    view.measure_override(size);
    view.arrange_override(size);

    assert_eq!(first.arranged_offset(), Some(Point { x: 0.0, y: 32.0 }));
    assert_eq!(first.arranged_width(), Some(240.0));
    assert_eq!(second.arranged_width(), Some(0.0));
    assert_eq!(second.arranged_height(), Some(0.0));
    assert!(second.clip_to_bounds());

    view.set_tab_strip_position(TabStripPosition::Bottom);
    view.measure_override(size);
    view.arrange_override(size);
    assert_eq!(first.arranged_offset(), Some(Point { x: 0.0, y: 0.0 }));
}

#[test]
#[ignore = "C prerequisite: composed #[component] render overrides are not in the UIElement vtable"]
fn close_affordance_is_custom_geometry_and_respects_presentation() {
    let item = CustomTabViewItem::new_item();
    item.set_header("document".to_string());
    let view = CustomTabView::new_view();
    view.set_children(vec![item]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    let always = RenderTree::new::<()>(&root);
    let always_lines = always
        .root
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::DrawLine { .. }))
        .count();
    assert_eq!(always_lines, 2);

    view.set_close_button_presentation(CloseButtonPresentation::Never);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let never = RenderTree::new::<()>(&root);
    let never_lines = never
        .root
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::DrawLine { .. }))
        .count();
    assert_eq!(never_lines, 0);
}

#[test]
fn splitter_freezes_axis_and_does_not_emit_zero_deltas() {
    let splitter = CustomSplitter::new_splitter();
    splitter.set_orientation(Orientation::Horizontal);
    let deltas = Rc::new(RefCell::new(Vec::new()));
    let deltas_for_callback = deltas.clone();
    splitter.set_on_drag_delta(Box::new(move |payload| {
        deltas_for_callback.borrow_mut().push(payload);
    }));
    let completed = Rc::new(RefCell::new(Vec::new()));
    let completed_for_callback = completed.clone();
    splitter.set_on_drag_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
    }));
    let target: Rc<dyn UIElementExt> = splitter.clone();
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 5.0, y: 5.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    splitter.set_orientation(Orientation::Vertical);
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 5.0, y: 8.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert!(deltas.borrow().is_empty());
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 8.0, y: 8.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(deltas.borrow().len(), 1);
    assert_eq!(deltas.borrow()[0].delta, 3.0);
    dispatch_routed(
        &target,
        "on_pointer_released",
        &pointer(Point { x: 8.0, y: 8.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    assert_eq!(completed.borrow().len(), 1);
    assert_eq!(completed.borrow()[0].cumulative_delta, 3.0);
}
