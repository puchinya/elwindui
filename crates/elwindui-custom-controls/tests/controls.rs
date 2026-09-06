use elwindui_custom_controls::core::base::{Point, Size};
use elwindui_custom_controls::core::graphics::{
    IconSource, RenderCommand, RenderGroup, RenderTree, SystemIcon,
};
use elwindui_custom_controls::core::input::{
    Key, KeyEventArgs, KeyModifiers, MouseButton, PointerEventArgs, RawPointerEvent,
    RawPointerEventKind, RoutedEventArgs,
};
use elwindui_custom_controls::core::layout::{
    GridLength, GridTrackConstraint, HorizontalAlignment, VerticalAlignment,
};
use elwindui_custom_controls::core::ui::{
    ContentControlExt, Control, ControlExt, Grid, GridExt, LayoutExt, ListExt, Rectangle,
    UIElementExt, dispatch_routed, hit_test, layout_root, unmount_subtree,
};
use elwindui_custom_controls::{
    CloseButtonPresentation, CustomGridSplitter, CustomGridSplitterExt, CustomTabView,
    CustomTabViewExt, CustomTabViewItem, CustomTabViewItemExt, GridResizeBehavior,
    GridResizeDirection, GridSplitterInputKind, GridSplitterResizeCompletedEventArgs,
    TabDragCompleted, TabStripPosition,
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

fn raw_pointer(kind: RawPointerEventKind, position: Point, timestamp_ms: f64) -> RawPointerEvent {
    RawPointerEvent {
        kind,
        position,
        screen_position: Some(position),
        modifiers: KeyModifiers::default(),
        timestamp_ms,
    }
}

fn render_commands<'a>(group: &'a RenderGroup, out: &mut Vec<&'a RenderCommand>) {
    out.extend(group.commands.iter());
    for child in &group.children {
        render_commands(child, out);
    }
}

fn rendered_texts(tree: &RenderTree) -> Vec<String> {
    let mut commands = Vec::new();
    render_commands(&tree.root, &mut commands);
    commands
        .into_iter()
        .filter_map(|command| match command {
            RenderCommand::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

fn find_visual(node: &Rc<dyn UIElementExt>, name: &str) -> Option<Rc<dyn UIElementExt>> {
    if node.type_name().contains(name) {
        return Some(node.clone());
    }
    for child in node.visual_children() {
        if let Some(found) = find_visual(&child, name) {
            return Some(found);
        }
    }
    None
}

fn close_icon_is_vector(item: &Rc<CustomTabViewItem>) -> bool {
    let close = item.close_button();
    let close: Rc<dyn UIElementExt> = close;
    find_visual(&close, "IconSourceElement").is_some()
}

fn item_indicator(item: &Rc<CustomTabViewItem>) -> Rc<dyn UIElementExt> {
    item.__template_root()
        .expect("tab item template root")
        .visual_children()
        .into_iter()
        .nth(1)
        .expect("tab item indicator")
}

fn absolute_offset(node: &Rc<dyn UIElementExt>) -> Point {
    let mut point = node.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
    let mut current = node.visual_parent();
    while let Some(parent) = current {
        let parent_offset = parent.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        point.x += parent_offset.x;
        point.y += parent_offset.y;
        current = parent.visual_parent();
    }
    point
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
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    assert_eq!(view.children().len(), 2);
    assert_eq!(view.children().to_vec()[0].header(), "first");
    assert_eq!(view.tab_position(), TabStripPosition::Bottom);
    assert_eq!(
        view.close_button_presentation(),
        CloseButtonPresentation::Always
    );
    assert!(second.visual_parent().is_some());
    assert!(
        second
            .visual_parent()
            .is_some_and(|parent| parent.type_name().contains("CustomTabStripPresenter"))
    );

    let replacement = CustomTabViewItem::new_item();
    view.set_children(vec![replacement.clone()]);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert!(first.visual_parent().is_none());
    assert!(second.visual_parent().is_none());
    assert!(replacement.visual_parent().is_some());
}

#[test]
fn compact_tab_metrics_update_the_retained_tab_view_grid() {
    let item = CustomTabViewItem::new_item();
    item.set_header("compact".to_string());
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

    let template = view.__template_root().expect("tab view template root");
    let grid = template
        .as_any()
        .downcast_ref::<Grid>()
        .expect("tab view template root is a Grid");
    assert!(matches!(
        grid.rows.borrow().first(),
        Some(GridLength::Fixed(height)) if (*height - 32.0).abs() < f32::EPSILON
    ));

    view.set_compact(true);
    assert!(matches!(
        grid.rows.borrow().first(),
        Some(GridLength::Fixed(height)) if (*height - 28.0).abs() < f32::EPSILON
    ));
}

#[test]
fn mounted_public_controls_release_after_external_owners_drop() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let splitter = CustomGridSplitter::new_splitter();
    let root: Rc<dyn UIElementExt> = view.clone();
    let splitter_root: Rc<dyn UIElementExt> = splitter.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    layout_root(
        &splitter_root,
        Size {
            width: 240.0,
            height: 6.0,
        },
    );

    let item_weak = Rc::downgrade(&item);
    let view_weak = Rc::downgrade(&view);
    let splitter_weak = Rc::downgrade(&splitter);
    unmount_subtree(&root);
    unmount_subtree(&splitter_root);
    drop(root);
    drop(splitter_root);
    drop(item);
    drop(view);
    drop(splitter);

    assert!(item_weak.upgrade().is_none());
    assert!(view_weak.upgrade().is_none());
    assert!(splitter_weak.upgrade().is_none());
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
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    let completed = Rc::new(RefCell::new(Vec::<TabDragCompleted>::new()));
    let completed_for_callback = completed.clone();
    view.set_on_tab_drag_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
    }));

    let target: Rc<dyn UIElementExt> = item.clone();
    assert!(
        target
            .visual_parent()
            .is_some_and(|parent| parent.type_name().contains("CustomTabStripPresenter"))
    );
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

    let size = Size {
        width: 240.0,
        height: 120.0,
    };
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(&root, size);
    let close: Rc<dyn UIElementExt> = second.close_button();
    let close_origin = absolute_offset(&close);
    let close_point = Point {
        x: close_origin.x + close.arranged_width().unwrap_or(20.0) / 2.0,
        y: close_origin.y + close.arranged_height().unwrap_or(32.0) / 2.0,
    };
    assert!(hit_test(&root, close_point).is_some());
    let dispatcher = elwindui_custom_controls::core::input::PointerDispatcher::new();
    let focus = elwindui_custom_controls::core::focus::FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Pressed(MouseButton::Left),
            close_point,
            0.0,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Released(MouseButton::Left),
            close_point,
            1.0,
        ),
    );

    assert_eq!(&*close_requests.borrow(), &[1]);
    assert_eq!(view.selected_index(), 0);
    assert!(selection.borrow().is_empty());
    assert_eq!(view.children().len(), 2);
    let _ = first;
}

#[test]
fn close_pointer_release_outside_does_not_request_close_or_select() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_close_button_presentation(CloseButtonPresentation::Always);
    view.set_children(vec![first, second.clone()]);

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

    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let close: Rc<dyn UIElementExt> = second.close_button();
    let close_origin = absolute_offset(&close);
    let close_point = Point {
        x: close_origin.x + close.arranged_width().unwrap_or(20.0) / 2.0,
        y: close_origin.y + close.arranged_height().unwrap_or(32.0) / 2.0,
    };
    let outside = Point { x: 400.0, y: 160.0 };
    assert!(hit_test(&root, close_point).is_some());

    let dispatcher = elwindui_custom_controls::core::input::PointerDispatcher::new();
    let focus = elwindui_custom_controls::core::focus::FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Pressed(MouseButton::Left),
            close_point,
            0.0,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(RawPointerEventKind::Moved, outside, 1.0),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Released(MouseButton::Left),
            outside,
            2.0,
        ),
    );

    assert!(close_requests.borrow().is_empty());
    assert_eq!(view.selected_index(), 0);
    assert!(selection.borrow().is_empty());
    assert_eq!(view.children().len(), 2);

    // A completion event after the captured release must be harmless.
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Released(MouseButton::Left),
            outside,
            3.0,
        ),
    );
    assert!(close_requests.borrow().is_empty());
    assert!(selection.borrow().is_empty());
}

#[test]
fn close_pointer_canceled_does_not_request_close_or_select() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_close_button_presentation(CloseButtonPresentation::Always);
    view.set_children(vec![first, second.clone()]);

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

    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let close: Rc<dyn UIElementExt> = second.close_button();
    let close_origin = absolute_offset(&close);
    let close_point = Point {
        x: close_origin.x + close.arranged_width().unwrap_or(20.0) / 2.0,
        y: close_origin.y + close.arranged_height().unwrap_or(32.0) / 2.0,
    };
    assert!(hit_test(&root, close_point).is_some());

    let dispatcher = elwindui_custom_controls::core::input::PointerDispatcher::new();
    let focus = elwindui_custom_controls::core::focus::FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Pressed(MouseButton::Left),
            close_point,
            0.0,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Canceled,
            Point { x: 400.0, y: 160.0 },
            1.0,
        ),
    );

    assert!(close_requests.borrow().is_empty());
    assert_eq!(view.selected_index(), 0);
    assert!(selection.borrow().is_empty());

    // Repeated cancellation after the gesture has been cleared is a no-op.
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Canceled,
            Point { x: 400.0, y: 160.0 },
            2.0,
        ),
    );
    assert!(close_requests.borrow().is_empty());
    assert!(selection.borrow().is_empty());
}

#[test]
fn pointer_over_hover_changes_close_template_without_width_jitter() {
    let item = CustomTabViewItem::new_item();
    item.set_header("hover".to_string());
    let view = CustomTabView::new_view();
    view.set_close_button_presentation(CloseButtonPresentation::OnPointerOver);
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let item_measure_before = item.measured_size();
    let item_arranged_before = (
        item.arranged_width(),
        item.arranged_height(),
        item.arranged_offset(),
    );
    let close: Rc<dyn UIElementExt> = item.close_button();
    let close_measure_before = close.measured_size();
    let close_arranged_before = (
        close.arranged_width(),
        close.arranged_height(),
        close.arranged_offset(),
    );
    assert!(!close_icon_is_vector(&item));

    let target: Rc<dyn UIElementExt> = item.clone();
    let args = pointer(Point { x: 1.0, y: 1.0 }, None);
    dispatch_routed(
        &target,
        "on_pointer_entered",
        &args,
        &RoutedEventArgs::default(),
    );
    assert_eq!(item.measured_size(), item_measure_before);
    assert_eq!(
        (
            item.arranged_width(),
            item.arranged_height(),
            item.arranged_offset()
        ),
        item_arranged_before
    );
    assert_eq!(close.measured_size(), close_measure_before);
    assert_eq!(
        (
            close.arranged_width(),
            close.arranged_height(),
            close.arranged_offset()
        ),
        close_arranged_before
    );
    assert!(close_icon_is_vector(&item));
    dispatch_routed(
        &target,
        "on_pointer_exited",
        &args,
        &RoutedEventArgs::default(),
    );
    assert_eq!(item.measured_size(), item_measure_before);
    assert_eq!(
        (
            item.arranged_width(),
            item.arranged_height(),
            item.arranged_offset()
        ),
        item_arranged_before
    );
    assert_eq!(close.measured_size(), close_measure_before);
    assert_eq!(
        (
            close.arranged_width(),
            close.arranged_height(),
            close.arranged_offset()
        ),
        close_arranged_before
    );
    assert!(!close_icon_is_vector(&item));
}

#[test]
fn collapsing_close_slot_from_hover_presentation_invalidates_measure() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_close_button_presentation(CloseButtonPresentation::OnPointerOver);
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert!(item.measured_size().is_some());
    let close: Rc<dyn UIElementExt> = item.close_button();
    let slot = close
        .visual_children()
        .into_iter()
        .next()
        .expect("close button slot is not mounted");
    assert!(slot.measured_size().is_some());

    view.set_close_button_presentation(CloseButtonPresentation::Never);

    assert!(
        slot.measured_size().is_none(),
        "collapsing the close slot must retain the slot's measure invalidation"
    );
    assert_eq!(
        slot.visibility(),
        elwindui_custom_controls::core::layout::Visibility::Collapsed
    );
}

#[test]
fn drag_started_callback_removing_item_only_cancels() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
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
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(&root, size);
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
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
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
    assert!(
        final_item
            .visual_parent()
            .is_some_and(|parent| parent.type_name().contains("CustomTabStripPresenter"))
    );
}

#[test]
fn removing_a_tab_during_drag_cancels_the_gesture() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
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
fn tab_header_render_tree_is_composed_from_standard_visuals() {
    let item = CustomTabViewItem::new_item();
    item.set_header("document".to_string());
    item.set_icon(Some(IconSource::System(SystemIcon::Add)));
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    assert!(find_visual(&root, "CustomTabStripPresenter").is_some());
    assert!(find_visual(&root, "IconSourceElement").is_some());
    assert!(find_visual(&root, "TextBlock").is_some());
    assert!(find_visual(&root, "Rectangle").is_some());
    let texts = rendered_texts(&RenderTree::new::<()>(&root));
    assert!(texts.iter().any(|text| text == "document"));
    assert!(close_icon_is_vector(&item));
}

#[test]
fn tab_insertion_uses_retained_unequal_header_midpoints_and_boundaries() {
    let first = CustomTabViewItem::new_item();
    first.set_header("A".to_string());
    first.set_width(80.0);
    let second = CustomTabViewItem::new_item();
    second.set_header("B".to_string());
    second.set_width(140.0);
    let third = CustomTabViewItem::new_item();
    third.set_header("C".to_string());
    third.set_width(80.0);
    let view = CustomTabView::new_view();
    view.set_children(vec![first, second, third]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 300.0,
            height: 120.0,
        },
    );

    assert_eq!(
        view.tab_insertion_index_at(Point { x: 40.0, y: 16.0 }),
        Some(0)
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 41.0, y: 16.0 }),
        Some(1)
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 150.0, y: 16.0 }),
        Some(1)
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 151.0, y: 16.0 }),
        Some(2)
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 260.0, y: 16.0 }),
        Some(2)
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 261.0, y: 16.0 }),
        Some(3)
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 150.0, y: 60.0 }),
        None
    );
    assert_eq!(view.tab_insertion_boundary(0).map(|rect| rect.x), Some(0.0));
    assert_eq!(
        view.tab_insertion_boundary(1).map(|rect| rect.x),
        Some(80.0)
    );
    assert_eq!(
        view.tab_insertion_boundary(2).map(|rect| rect.x),
        Some(220.0)
    );
    assert_eq!(
        view.tab_insertion_boundary(3).map(|rect| rect.x),
        Some(300.0)
    );
}

#[test]
fn compact_and_empty_tab_strips_share_the_retained_geometry_path() {
    let first = CustomTabViewItem::new_item();
    first.set_header("compact".to_string());
    first.set_width(120.0);
    let view = CustomTabView::new_view();
    view.set_compact(true);
    view.set_children(vec![first]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 60.0, y: 14.0 }),
        Some(0)
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 121.0, y: 14.0 }),
        Some(1)
    );

    view.set_children(Vec::new());
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(
        view.tab_insertion_index_at(Point { x: 20.0, y: 14.0 }),
        Some(0)
    );
    assert_eq!(view.tab_insertion_boundary(0).map(|rect| rect.x), Some(0.0));
}

#[test]
fn header_and_icon_property_changes_resync_the_template_subtree() {
    let item = CustomTabViewItem::new_item();
    item.set_header("a".to_string());
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let width_a = item.arranged_width().expect("initial tab width");

    item.set_header("a much longer header".to_string());
    item.set_icon(Some(IconSource::System(SystemIcon::Add)));
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let width_b = item.arranged_width().expect("updated tab width");
    assert!(width_b > width_a);
    assert!(
        rendered_texts(&RenderTree::new::<()>(&root))
            .iter()
            .any(|text| text == "a much longer header")
    );
    let icon = find_visual(&root, "IconSourceElement").expect("template icon");
    assert_eq!(
        icon.visibility(),
        elwindui_custom_controls::core::layout::Visibility::Visible
    );

    item.set_icon(None);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(
        icon.visibility(),
        elwindui_custom_controls::core::layout::Visibility::Collapsed
    );
}

#[test]
fn selected_indicator_moves_without_recreating_header_items() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone(), second.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let first_indicator = item_indicator(&first);
    let second_indicator = item_indicator(&second);
    assert_eq!(
        first_indicator.visibility(),
        elwindui_custom_controls::core::layout::Visibility::Visible
    );
    assert_eq!(
        second_indicator.visibility(),
        elwindui_custom_controls::core::layout::Visibility::Collapsed
    );

    view.select_index(1);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(
        first_indicator.visibility(),
        elwindui_custom_controls::core::layout::Visibility::Collapsed
    );
    assert_eq!(
        second_indicator.visibility(),
        elwindui_custom_controls::core::layout::Visibility::Visible
    );
    assert!(first.visual_parent().is_some());
    assert!(second.visual_parent().is_some());
}

#[test]
fn tab_item_header_and_indicator_tracks_follow_strip_position() {
    let item = CustomTabViewItem::new_item();
    item.set_header("document".to_string());
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();

    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let item_root: Rc<dyn UIElementExt> = item.clone();
    let header = find_visual(&item_root, "HorizontalLayout").expect("header row");
    let indicator = item_indicator(&item);
    assert_eq!(header.arranged_offset().expect("header offset").y, 0.0);
    assert_eq!(header.arranged_height(), Some(30.0));
    assert_eq!(
        indicator.arranged_offset().expect("indicator offset").y,
        30.0
    );
    assert_eq!(indicator.arranged_height(), Some(2.0));

    view.set_tab_position(TabStripPosition::Bottom);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(
        indicator.arranged_offset().expect("indicator offset").y,
        0.0
    );
    assert_eq!(indicator.arranged_height(), Some(2.0));
    assert_eq!(header.arranged_offset().expect("header offset").y, 2.0);
    assert_eq!(header.arranged_height(), Some(30.0));
}

#[test]
fn content_control_item_owns_one_logical_content_element() {
    let item = CustomTabViewItem::new_item();
    let content = elwindui_custom_controls::core::ui::TextBlock::new();
    item.set_content(content.clone());
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert!(
        content
            .visual_parent()
            .is_some_and(|parent| parent.type_name().contains("CustomTabContentPresenter"))
    );
}

#[test]
fn content_replacement_detaches_old_visual_before_attaching_new_visual() {
    let item = CustomTabViewItem::new_item();
    let old_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let new_content = elwindui_custom_controls::core::ui::TextBlock::new();
    item.set_content(old_content.clone());
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let presenter = old_content.visual_parent().expect("content presenter");
    assert!(presenter.type_name().contains("CustomTabContentPresenter"));

    item.set_content(new_content.clone());
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    assert!(old_content.visual_parent().is_none());
    assert!(
        new_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter))
    );
}

#[test]
fn hidden_content_replacement_resets_only_the_replaced_page_geometry() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let third = CustomTabViewItem::new_item();
    let first_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let old_hidden_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let third_content = elwindui_custom_controls::core::ui::TextBlock::new();
    first.set_content(first_content.clone());
    second.set_content(old_hidden_content.clone());
    third.set_content(third_content.clone());

    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone(), second.clone(), third.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    let size = Size {
        width: 240.0,
        height: 120.0,
    };
    layout_root(&root, size);
    let presenter = old_hidden_content
        .visual_parent()
        .expect("hidden content presenter");
    let first_parent = first_content.visual_parent().expect("first content parent");
    let third_parent = third_content.visual_parent().expect("third content parent");
    assert_eq!(old_hidden_content.arranged_width(), Some(0.0));

    let replacement = elwindui_custom_controls::core::ui::TextBlock::new();
    second.set_content(replacement.clone());
    layout_root(&root, size);

    assert!(old_hidden_content.visual_parent().is_none());
    assert!(
        replacement
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter))
    );
    assert_eq!(replacement.arranged_width(), Some(0.0));
    assert_eq!(replacement.arranged_height(), Some(0.0));
    assert!(
        first_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &first_parent))
    );
    assert!(
        third_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &third_parent))
    );
}

#[test]
fn selected_content_replacement_is_arranged_full_without_rebuilding_other_pages() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let first_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let old_selected_content = elwindui_custom_controls::core::ui::TextBlock::new();
    first.set_content(first_content.clone());
    second.set_content(old_selected_content.clone());

    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone(), second.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    let size = Size {
        width: 240.0,
        height: 120.0,
    };
    layout_root(&root, size);
    let presenter = old_selected_content
        .visual_parent()
        .expect("selected content presenter");
    let first_parent = first_content.visual_parent().expect("first content parent");

    assert!(view.select_index(1));
    layout_root(&root, size);
    let replacement = elwindui_custom_controls::core::ui::TextBlock::new();
    second.set_content(replacement.clone());
    layout_root(&root, size);

    assert_eq!(view.selected_index(), 1);
    assert!(old_selected_content.visual_parent().is_none());
    assert!(
        replacement
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter))
    );
    assert_eq!(replacement.arranged_width(), Some(240.0));
    assert_eq!(replacement.arranged_height(), Some(88.0));
    assert!(
        first_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &first_parent))
    );
}

#[test]
fn content_presenter_preserves_item_indices_when_a_tab_has_no_content() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let second_content = elwindui_custom_controls::core::ui::TextBlock::new();
    second.set_content(second_content.clone());
    let view = CustomTabView::new_view();
    view.set_children(vec![first, second]);
    view.set_selected_index(1);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    assert_eq!(second_content.arranged_width(), Some(240.0));
    assert_eq!(second_content.arranged_height(), Some(88.0));
}

#[test]
fn removing_item_detaches_header_and_content_without_destroying_external_content() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let first_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let second_content = elwindui_custom_controls::core::ui::TextBlock::new();
    first.set_content(first_content.clone());
    second.set_content(second_content.clone());
    let view = CustomTabView::new_view();
    view.set_children(vec![first.clone(), second.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert!(first.visual_parent().is_some());
    assert!(first_content.visual_parent().is_some());

    let removed = view.remove_child(0).expect("removed item");
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    assert!(Rc::ptr_eq(&removed, &first));
    assert!(first.visual_parent().is_none());
    assert!(first_content.visual_parent().is_none());
    assert!(second.visual_parent().is_some());
    assert!(second_content.visual_parent().is_some());
}

#[test]
fn custom_grid_splitter_resizes_columns_without_a_callback() {
    let grid = Grid::new();
    grid.set_rows(vec![GridLength::Star(1.0)]);
    grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Columns);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_attached("Grid", "column", 1i32);
    grid.children().add(splitter.clone());
    let root: Rc<dyn UIElementExt> = grid.clone();
    let target: Rc<dyn UIElementExt> = splitter.clone();
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 100.0,
        },
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 2.0, y: 4.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 22.0, y: 4.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(120.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(80.0),
        ]
    );
}

#[test]
fn custom_grid_splitter_resizes_rows_without_a_callback() {
    let grid = Grid::new();
    grid.set_columns(vec![GridLength::Star(1.0)]);
    grid.set_rows(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Rows);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_attached("Grid", "row", 1i32);
    grid.children().add(splitter.clone());
    let root: Rc<dyn UIElementExt> = grid.clone();
    let target: Rc<dyn UIElementExt> = splitter;
    layout_root(
        &root,
        Size {
            width: 100.0,
            height: 206.0,
        },
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 4.0, y: 2.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 4.0, y: 22.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        grid.rows.borrow().as_slice(),
        &[
            GridLength::Fixed(120.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(80.0),
        ]
    );
}

#[test]
fn custom_grid_splitter_auto_direction_uses_arranged_aspect_ratio() {
    let columns_grid = Grid::new();
    columns_grid.set_rows(vec![GridLength::Star(1.0)]);
    columns_grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let columns_splitter = CustomGridSplitter::new_splitter();
    columns_splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    columns_splitter.set_attached("Grid", "column", 1i32);
    columns_grid.children().add(columns_splitter.clone());
    let columns_root: Rc<dyn UIElementExt> = columns_grid.clone();
    let columns_target: Rc<dyn UIElementExt> = columns_splitter.clone();
    layout_root(
        &columns_root,
        Size {
            width: 206.0,
            height: 100.0,
        },
    );
    dispatch_routed(
        &columns_target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 50.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &columns_target,
        "on_pointer_moved",
        &pointer(Point { x: 123.0, y: 50.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        columns_grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(120.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(80.0),
        ]
    );

    let rows_grid = Grid::new();
    rows_grid.set_columns(vec![GridLength::Star(1.0)]);
    rows_grid.set_rows(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let rows_splitter = CustomGridSplitter::new_splitter();
    rows_splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    rows_splitter.set_attached("Grid", "row", 1i32);
    rows_grid.children().add(rows_splitter.clone());
    let rows_root: Rc<dyn UIElementExt> = rows_grid.clone();
    let rows_target: Rc<dyn UIElementExt> = rows_splitter.clone();
    layout_root(
        &rows_root,
        Size {
            width: 100.0,
            height: 206.0,
        },
    );
    dispatch_routed(
        &rows_target,
        "on_pointer_pressed",
        &pointer(Point { x: 50.0, y: 103.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &rows_target,
        "on_pointer_moved",
        &pointer(Point { x: 50.0, y: 123.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        rows_grid.rows.borrow().as_slice(),
        &[
            GridLength::Fixed(120.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(80.0),
        ]
    );
}

#[test]
fn custom_grid_splitter_auto_direction_prioritizes_alignment() {
    let grid = Grid::new();
    grid.set_rows(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_horizontal_alignment(HorizontalAlignment::Left);
    splitter.set_vertical_alignment(VerticalAlignment::Top);
    splitter.set_attached("Grid", "column", 1i32);
    splitter.set_attached("Grid", "row", 1i32);
    grid.children().add(splitter.clone());
    let root: Rc<dyn UIElementExt> = grid.clone();
    let target: Rc<dyn UIElementExt> = splitter;
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 206.0,
        },
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 103.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 123.0, y: 123.0 }, None),
        &RoutedEventArgs::default(),
    );

    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(120.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(80.0),
        ]
    );
    assert_eq!(
        grid.rows.borrow().as_slice(),
        &[
            GridLength::Fixed(100.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(100.0),
        ]
    );
}

#[test]
fn custom_grid_splitter_invalid_pair_is_a_noop() {
    let grid = Grid::new();
    grid.set_rows(vec![GridLength::Star(1.0)]);
    grid.set_columns(vec![GridLength::Fixed(100.0), GridLength::Fixed(6.0)]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Columns);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndCurrent);
    splitter.set_attached("Grid", "column", 0i32);
    grid.children().add(splitter.clone());
    let started = Rc::new(RefCell::new(0));
    let started_for_callback = started.clone();
    splitter.set_on_resize_started(Box::new(move |_| {
        *started_for_callback.borrow_mut() += 1;
    }));
    let root: Rc<dyn UIElementExt> = grid.clone();
    let target: Rc<dyn UIElementExt> = splitter;
    layout_root(
        &root,
        Size {
            width: 106.0,
            height: 100.0,
        },
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 4.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 123.0, y: 4.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(*started.borrow(), 0);
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[GridLength::Fixed(100.0), GridLength::Fixed(6.0)]
    );
}

#[test]
fn custom_grid_splitter_freezes_properties_until_the_next_pointer_transaction() {
    let grid = Grid::new();
    grid.set_rows(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Columns);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_attached("Grid", "column", 1i32);
    splitter.set_attached("Grid", "row", 1i32);
    grid.children().add(splitter.clone());
    let starts = Rc::new(RefCell::new(0));
    let starts_for_callback = starts.clone();
    splitter.set_on_resize_started(Box::new(move |_| {
        *starts_for_callback.borrow_mut() += 1;
    }));
    let root: Rc<dyn UIElementExt> = grid.clone();
    let target: Rc<dyn UIElementExt> = splitter.clone();
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 206.0,
        },
    );

    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 103.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 103.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    splitter.set_resize_direction(GridResizeDirection::Rows);
    splitter.set_resize_behavior(GridResizeBehavior::CurrentAndNext);
    splitter.set_drag_increment(16.0);
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 123.0, y: 123.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(*starts.borrow(), 1);
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(120.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(80.0),
        ]
    );
    assert_eq!(
        grid.rows.borrow().as_slice(),
        &[
            GridLength::Fixed(100.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(100.0),
        ]
    );
    dispatch_routed(
        &target,
        "on_pointer_canceled",
        &PointerEventArgs {
            position: Point { x: 123.0, y: 123.0 },
            screen_position: None,
            button: None,
            modifiers: KeyModifiers::default(),
        },
        &RoutedEventArgs::default(),
    );
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 206.0,
        },
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 103.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 103.0, y: 123.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(*starts.borrow(), 2);
    assert_eq!(
        grid.rows.borrow().as_slice(),
        &[
            GridLength::Fixed(100.0),
            GridLength::Fixed(22.0),
            GridLength::Fixed(84.0),
        ]
    );
}

#[test]
fn custom_grid_splitter_parent_level_reads_placement_from_the_ancestor_control() {
    let grid = Grid::new();
    grid.set_rows(vec![GridLength::Star(1.0)]);
    grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let wrapper = Control::new();
    wrapper.set_attached("Grid", "column", 1i32);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Columns);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_parent_level(1);
    wrapper
        .as_ui_element()
        .visual_collection
        .add(splitter.clone());
    grid.children().add(wrapper);
    let root: Rc<dyn UIElementExt> = grid.clone();
    let target: Rc<dyn UIElementExt> = splitter.clone();
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 100.0,
        },
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 50.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 123.0, y: 50.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(120.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(80.0),
        ]
    );
}

#[test]
fn custom_grid_splitter_uses_grid_constraints_without_cumulative_drift() {
    let grid = Grid::new();
    grid.set_rows(vec![GridLength::Star(1.0)]);
    grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    grid.set_column_constraints(vec![
        GridTrackConstraint {
            min: Some(100.0),
            max: Some(115.0),
        },
        GridTrackConstraint::default(),
        GridTrackConstraint {
            min: Some(90.0),
            max: Some(100.0),
        },
    ]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Columns);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_attached("Grid", "column", 1i32);
    grid.children().add(splitter.clone());
    let root: Rc<dyn UIElementExt> = grid.clone();
    let target: Rc<dyn UIElementExt> = splitter;
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 100.0,
        },
    );
    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 4.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 153.0, y: 4.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(110.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(90.0),
        ]
    );
    dispatch_routed(
        &target,
        "on_pointer_moved",
        &pointer(Point { x: 105.0, y: 4.0 }, None),
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(102.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(98.0),
        ]
    );
}

#[test]
fn source_selection_is_not_echoed_and_invalid_selection_has_no_content() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let first_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let second_content = elwindui_custom_controls::core::ui::TextBlock::new();
    first.set_content(first_content.clone());
    second.set_content(second_content.clone());
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
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(&root, size);
    assert_eq!(first_content.arranged_width(), Some(0.0));
    assert_eq!(second_content.arranged_width(), Some(0.0));

    assert!(view.select_index(1));
    assert_eq!(&*changes.borrow(), &[1]);
    layout_root(&root, size);
    assert_eq!(first_content.arranged_width(), Some(0.0));
    assert_eq!(second_content.arranged_width(), Some(240.0));
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
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(&root, size);

    let first_content = first.content();
    let second_content = second.content();
    let content_presenter = first_content
        .visual_parent()
        .expect("selected content presenter");
    assert_eq!(
        content_presenter.arranged_offset(),
        Some(Point { x: 0.0, y: 32.0 })
    );
    assert_eq!(
        first_content.arranged_offset(),
        Some(Point { x: 0.0, y: 0.0 })
    );
    assert_eq!(first_content.arranged_width(), Some(240.0));
    assert_eq!(first_content.arranged_height(), Some(88.0));
    assert_eq!(second_content.arranged_width(), Some(0.0));
    assert_eq!(second_content.arranged_height(), Some(0.0));
    assert!(second_content.clip_to_bounds());

    view.set_tab_strip_position(TabStripPosition::Bottom);
    layout_root(&root, size);
    assert_eq!(
        content_presenter.arranged_offset(),
        Some(Point { x: 0.0, y: 0.0 })
    );
    assert_eq!(first_content.arranged_height(), Some(88.0));
}

#[test]
fn repeated_selection_retains_page_owners_and_only_switches_active_rectangles() {
    let first = CustomTabViewItem::new_item();
    let second = CustomTabViewItem::new_item();
    let third = CustomTabViewItem::new_item();
    let first_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let second_content = elwindui_custom_controls::core::ui::TextBlock::new();
    let third_content = elwindui_custom_controls::core::ui::TextBlock::new();
    first.set_content(first_content.clone());
    second.set_content(second_content.clone());
    third.set_content(third_content.clone());
    let view = CustomTabView::new_view();
    view.set_children(vec![first, second, third]);
    let root: Rc<dyn UIElementExt> = view.clone();
    let size = Size {
        width: 320.0,
        height: 160.0,
    };
    layout_root(&root, size);
    let first_parent = first_content.visual_parent().expect("first content parent");
    let second_parent = second_content
        .visual_parent()
        .expect("second content parent");
    let third_parent = third_content.visual_parent().expect("third content parent");

    assert!(view.select_index(1));
    layout_root(&root, size);
    assert_eq!(first_content.arranged_width(), Some(0.0));
    assert_eq!(second_content.arranged_width(), Some(320.0));
    assert_eq!(third_content.arranged_width(), Some(0.0));
    assert!(
        first_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &first_parent))
    );
    assert!(
        second_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &second_parent))
    );
    assert!(
        third_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &third_parent))
    );

    assert!(view.select_index(2));
    layout_root(&root, size);
    assert_eq!(second_content.arranged_width(), Some(0.0));
    assert_eq!(third_content.arranged_width(), Some(320.0));
    assert!(
        first_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &first_parent))
    );
    assert!(
        second_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &second_parent))
    );
    assert!(
        third_content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &third_parent))
    );
}

#[test]
fn close_affordance_is_composed_and_respects_presentation() {
    let item = CustomTabViewItem::new_item();
    item.set_header("document".to_string());
    let view = CustomTabView::new_view();
    view.set_children(vec![item.clone()]);
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    let always_width = item.arranged_width().expect("tab width");
    assert!(close_icon_is_vector(&item));

    view.set_close_button_presentation(CloseButtonPresentation::Never);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let never_width = item.arranged_width().expect("tab width");
    let _never = RenderTree::new::<()>(&root);
    assert!(!close_icon_is_vector(&item));
    assert_eq!(always_width, never_width + 20.0);

    view.set_close_button_presentation(CloseButtonPresentation::OnPointerOver);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(item.arranged_width(), Some(never_width + 20.0));
}

#[test]
fn custom_grid_splitter_default_template_is_six_by_six() {
    let splitter = CustomGridSplitter::new_splitter();
    let root: Rc<dyn UIElementExt> = splitter.clone();
    let available = Size {
        width: 240.0,
        height: 120.0,
    };

    layout_root(&root, available);
    assert_eq!(
        splitter.measured_size(),
        Some(Size {
            width: 6.0,
            height: 6.0
        })
    );
}

#[test]
fn custom_grid_splitter_visual_follows_explicit_axis() {
    let splitter = CustomGridSplitter::new_splitter();
    let root: Rc<dyn UIElementExt> = splitter.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );

    let template_root = splitter.__template_root().expect("splitter template root");
    let rectangle = template_root
        .as_any()
        .downcast_ref::<Rectangle>()
        .expect("splitter template rectangle");
    assert_eq!(rectangle.width(), Some(6.0));
    assert_eq!(rectangle.height(), Some(6.0));
    assert_eq!(
        rectangle.horizontal_alignment(),
        HorizontalAlignment::Center
    );
    assert_eq!(rectangle.vertical_alignment(), VerticalAlignment::Center);

    splitter.set_resize_direction(GridResizeDirection::Columns);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(rectangle.width(), Some(6.0));
    assert_eq!(rectangle.height(), None);
    assert_eq!(rectangle.min_height(), Some(6.0));
    assert_eq!(
        rectangle.horizontal_alignment(),
        HorizontalAlignment::Stretch
    );
    assert_eq!(rectangle.vertical_alignment(), VerticalAlignment::Stretch);
    assert_eq!(rectangle.arranged_width(), Some(6.0));
    assert_eq!(rectangle.arranged_height(), Some(120.0));

    splitter.set_resize_direction(GridResizeDirection::Rows);
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    assert_eq!(rectangle.width(), None);
    assert_eq!(rectangle.height(), Some(6.0));
    assert_eq!(rectangle.min_width(), Some(6.0));
    assert_eq!(
        rectangle.horizontal_alignment(),
        HorizontalAlignment::Stretch
    );
    assert_eq!(rectangle.vertical_alignment(), VerticalAlignment::Stretch);
    assert_eq!(rectangle.arranged_width(), Some(240.0));
    assert_eq!(rectangle.arranged_height(), Some(6.0));
}

#[test]
fn pointer_dispatcher_implicit_capture_completes_tab_outside_and_cancels() {
    let item = CustomTabViewItem::new_item();
    let view = CustomTabView::new_view();
    view.set_children(vec![item]);
    let completed = Rc::new(RefCell::new(Vec::<TabDragCompleted>::new()));
    let completed_for_callback = completed.clone();
    view.set_on_tab_drag_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
    }));
    let root: Rc<dyn UIElementExt> = view.clone();
    layout_root(
        &root,
        Size {
            width: 240.0,
            height: 120.0,
        },
    );
    let dispatcher = elwindui_custom_controls::core::input::PointerDispatcher::new();
    let focus = elwindui_custom_controls::core::focus::FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Pressed(MouseButton::Left),
            Point { x: 4.0, y: 16.0 },
            0.0,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(RawPointerEventKind::Moved, Point { x: 9.0, y: 16.0 }, 1.0),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Moved,
            Point { x: 500.0, y: 500.0 },
            2.0,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Released(MouseButton::Left),
            Point { x: 500.0, y: 500.0 },
            3.0,
        ),
    );
    assert_eq!(completed.borrow().len(), 1);
    assert!(!completed.borrow()[0].canceled);

    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Pressed(MouseButton::Left),
            Point { x: 4.0, y: 16.0 },
            4.0,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(RawPointerEventKind::Moved, Point { x: 9.0, y: 16.0 }, 5.0),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Canceled,
            Point { x: 500.0, y: 500.0 },
            6.0,
        ),
    );
    assert_eq!(completed.borrow().len(), 2);
    assert!(completed.borrow()[1].canceled);
}

#[test]
fn pointer_dispatcher_cancellation_restores_grid_before_notification() {
    let grid = Grid::new();
    grid.set_rows(vec![GridLength::Star(1.0)]);
    grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Columns);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_attached("Grid", "column", 1i32);
    grid.children().add(splitter.clone());
    let root: Rc<dyn UIElementExt> = grid.clone();
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 100.0,
        },
    );
    let original = grid.columns.borrow().clone();
    let completed = Rc::new(RefCell::new(
        Vec::<GridSplitterResizeCompletedEventArgs>::new(),
    ));
    let completed_for_callback = completed.clone();
    let observed = Rc::new(RefCell::new(Vec::<Vec<GridLength>>::new()));
    let observed_for_callback = observed.clone();
    let grid_for_callback = grid.clone();
    let delta_observed = Rc::new(RefCell::new(Vec::<Vec<GridLength>>::new()));
    let delta_observed_for_callback = delta_observed.clone();
    let grid_for_delta = grid.clone();
    splitter.set_on_resize_delta(Box::new(move |_| {
        delta_observed_for_callback
            .borrow_mut()
            .push(grid_for_delta.columns.borrow().clone());
    }));
    splitter.set_on_resize_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
        observed_for_callback
            .borrow_mut()
            .push(grid_for_callback.columns.borrow().clone());
    }));
    let dispatcher = elwindui_custom_controls::core::input::PointerDispatcher::new();
    let focus = elwindui_custom_controls::core::focus::FocusTracker::new();
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Pressed(MouseButton::Left),
            Point { x: 103.0, y: 3.0 },
            0.0,
        ),
    );
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(RawPointerEventKind::Moved, Point { x: 123.0, y: 3.0 }, 1.0),
    );
    assert_ne!(grid.columns.borrow().as_slice(), original.as_slice());
    assert_eq!(delta_observed.borrow().len(), 1);
    assert_ne!(delta_observed.borrow()[0], original);
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Canceled,
            Point { x: 500.0, y: 3.0 },
            2.0,
        ),
    );
    assert_eq!(completed.borrow().len(), 1);
    assert!(completed.borrow()[0].canceled);
    assert_eq!(grid.columns.borrow().as_slice(), original.as_slice());
    assert_eq!(observed.borrow().as_slice(), &[original]);
    dispatcher.handle(
        &root,
        &focus,
        raw_pointer(
            RawPointerEventKind::Released(MouseButton::Left),
            Point { x: 500.0, y: 3.0 },
            3.0,
        ),
    );
    assert_eq!(completed.borrow().len(), 1);
}

#[test]
fn keyboard_resize_uses_the_same_grid_engine_and_reports_input_kind() {
    let grid = Grid::new();
    grid.set_rows(vec![GridLength::Star(1.0)]);
    grid.set_columns(vec![
        GridLength::Fixed(100.0),
        GridLength::Fixed(6.0),
        GridLength::Fixed(100.0),
    ]);
    let splitter = CustomGridSplitter::new_splitter();
    splitter.set_resize_direction(GridResizeDirection::Columns);
    splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
    splitter.set_keyboard_increment(10.0);
    splitter.set_attached("Grid", "column", 1i32);
    grid.children().add(splitter.clone());
    let root: Rc<dyn UIElementExt> = grid.clone();
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 100.0,
        },
    );
    assert!(splitter.is_tab_stop());

    let completed = Rc::new(RefCell::new(
        Vec::<GridSplitterResizeCompletedEventArgs>::new(),
    ));
    let deltas = Rc::new(RefCell::new(Vec::new()));
    let started = Rc::new(RefCell::new(Vec::new()));
    let completed_for_callback = completed.clone();
    let deltas_for_callback = deltas.clone();
    let started_for_callback = started.clone();
    splitter.set_on_resize_started(Box::new(move |payload| {
        started_for_callback.borrow_mut().push(payload);
    }));
    splitter.set_on_resize_delta(Box::new(move |payload| {
        deltas_for_callback.borrow_mut().push(payload);
    }));
    splitter.set_on_resize_completed(Box::new(move |payload| {
        completed_for_callback.borrow_mut().push(payload);
    }));
    let key = KeyEventArgs {
        key: Key::Right,
        modifiers: KeyModifiers::default(),
        is_repeat: false,
    };
    let target: Rc<dyn UIElementExt> = splitter;
    let wrong_axis_key = KeyEventArgs {
        key: Key::Up,
        modifiers: KeyModifiers::default(),
        is_repeat: false,
    };
    dispatch_routed(
        &target,
        "on_key_down",
        &wrong_axis_key,
        &RoutedEventArgs::default(),
    );
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(100.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(100.0),
        ]
    );
    assert!(started.borrow().is_empty());
    assert!(deltas.borrow().is_empty());
    assert!(completed.borrow().is_empty());

    dispatch_routed(
        &target,
        "on_pointer_pressed",
        &pointer(Point { x: 103.0, y: 4.0 }, Some(MouseButton::Left)),
        &RoutedEventArgs::default(),
    );
    assert_eq!(started.borrow().len(), 1);
    dispatch_routed(&target, "on_key_down", &key, &RoutedEventArgs::default());
    assert!(deltas.borrow().is_empty());
    assert!(completed.borrow().is_empty());
    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(100.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(100.0),
        ]
    );
    dispatch_routed(
        &target,
        "on_pointer_canceled",
        &PointerEventArgs {
            position: Point { x: 103.0, y: 4.0 },
            screen_position: None,
            button: None,
            modifiers: KeyModifiers::default(),
        },
        &RoutedEventArgs::default(),
    );
    assert_eq!(completed.borrow().len(), 1);
    assert!(completed.borrow()[0].canceled);
    layout_root(
        &root,
        Size {
            width: 206.0,
            height: 100.0,
        },
    );
    dispatch_routed(&target, "on_key_down", &key, &RoutedEventArgs::default());

    assert_eq!(
        grid.columns.borrow().as_slice(),
        &[
            GridLength::Fixed(110.0),
            GridLength::Fixed(6.0),
            GridLength::Fixed(90.0),
        ]
    );
    assert_eq!(started.borrow().len(), 2);
    assert_eq!(deltas.borrow().len(), 1);
    assert_eq!(deltas.borrow()[0].delta, 10.0);
    assert_eq!(deltas.borrow()[0].cumulative_delta, 10.0);
    assert_eq!(
        deltas.borrow()[0].input_kind,
        GridSplitterInputKind::Keyboard
    );
    assert!(deltas.borrow()[0].position.is_none());
    assert_eq!(completed.borrow().len(), 2);
    assert!(!completed.borrow()[1].canceled);
    assert_eq!(
        completed.borrow()[1].input_kind,
        GridSplitterInputKind::Keyboard
    );
    assert!(completed.borrow()[1].position.is_none());
}
