//! Issue #152: Integration and type-check tests for Context Menu, Custom Context Menu presentation, and rich Context Popup.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::{Point, Rect, Size};
use elwindui::core::ui::popup::{
    ContextMenuService, ContextRequest, PopupAnchor, PopupDismissAction,
    PopupHost, PopupRequest, PopupSurfaceHandle, ResolvedContextDefinition,
};
use elwindui::core::ui::{LayoutExt, MenuItemExt, UIElementExt, ViewTemplate, unmount_subtree};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct TestPopupHandle {
    closed: Rc<Cell<bool>>,
}

impl PopupSurfaceHandle for TestPopupHandle {
    fn close(&self) {
        self.closed.set(true);
    }
}

struct TestPopupHost {
    shown: RefCell<Vec<(Rc<dyn UIElementExt>, Point, Size)>>,
    last_request: RefCell<Option<PopupRequest>>,
    closed: Rc<Cell<bool>>,
}

impl TestPopupHost {
    fn new() -> Self {
        Self {
            shown: RefCell::new(Vec::new()),
            last_request: RefCell::new(None),
            closed: Rc::new(Cell::new(false)),
        }
    }
}

impl PopupHost for TestPopupHost {
    fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>> {
        self.shown
            .borrow_mut()
            .push((Rc::clone(&request.content), request.position, request.size));
        *self.last_request.borrow_mut() = Some(request);
        Some(Rc::new(TestPopupHandle {
            closed: Rc::clone(&self.closed),
        }))
    }
}

#[elwindui::component(inherits VerticalLayout)]
struct ViewWithNativeContextMenu {
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Right click me",
            context_menu: Menu {
                MenuItem {
                    text: "Cut",
                }
                MenuItem {
                    text: "Copy",
                }
                MenuItem {
                    text: "Paste",
                }
            },
        };

        VerticalLayout {
            target
        }
    },
}

#[elwindui::component]
impl ViewWithNativeContextMenu {}

#[elwindui::component(inherits VerticalLayout)]
struct ViewWithCustomContextMenu {
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Custom context menu target",
            context_menu: Menu {
                MenuItem {
                    text: "Custom Option 1",
                }
                MenuItem {
                    text: "Custom Option 2",
                }
            },
            context_menu_presentation: ContextMenuPresentation::Custom,
        };

        VerticalLayout {
            target
        }
    },
}

#[elwindui::component]
impl ViewWithCustomContextMenu {}

#[allow(dead_code)]
fn type_checked_context_menu_and_custom_views() {
    let native_view = ViewWithNativeContextMenu::new();
    let custom_view = ViewWithCustomContextMenu::new();
    let _ = (native_view, custom_view);
}

#[test]
fn context_menu_dsl_and_components_type_check() {
    let _ = type_checked_context_menu_and_custom_views as fn();
}

#[elwindui::class(struct_only = elwindui::core::ui::MenuExt)]
struct TestMenu {
    items: elwindui::core::ui::ChildList<dyn elwindui::core::ui::MenuItemExt>,
}

#[elwindui::class]
impl TestMenu {
    fn construct() -> Self {
        Self {
            items: elwindui::core::ui::ChildList::new(),
        }
    }
    fn add_item(&self, _item: &dyn elwindui::core::ui::MenuItemExt) {}
    fn remove_item(&self, _item: &dyn elwindui::core::ui::MenuItemExt) {}
    fn items(&self) -> &dyn elwindui::core::ui::ListExt<dyn elwindui::core::ui::MenuItemExt> {
        self
    }
}

impl elwindui::core::ui::ListExt<dyn elwindui::core::ui::MenuItemExt> for TestMenu {
    fn add(&self, item: Rc<dyn elwindui::core::ui::MenuItemExt>) {
        self.items.add(item);
    }
    fn insert(&self, index: usize, item: Rc<dyn elwindui::core::ui::MenuItemExt>) {
        self.items.insert(index, item);
    }
    fn remove(&self, item: &Rc<dyn elwindui::core::ui::MenuItemExt>) -> bool {
        self.items.remove(item)
    }
    fn remove_at(&self, index: usize) -> Rc<dyn elwindui::core::ui::MenuItemExt> {
        self.items.remove_at(index)
    }
    fn clear(&self) {
        self.items.clear();
    }
    fn len(&self) -> usize {
        self.items.len()
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui::core::ui::MenuItemExt>> {
        self.items.to_vec()
    }
}

#[test]
fn custom_context_menu_service_opens_and_closes_popup() {
    let host = TestPopupHost::new();
    let anchor = PopupAnchor::Point(Point { x: 120.0, y: 80.0 });
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    let menu = TestMenu::new();
    let handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area)
        .expect("host should show successfully");

    assert_eq!(host.shown.borrow().len(), 1);
    let (_content, pos, size) = &host.shown.borrow()[0];
    assert_eq!(*pos, Point { x: 120.0, y: 80.0 });
    assert!(size.width > 0.0);
    assert!(size.height > 0.0);

    // Verify handle closes popup
    assert!(!host.closed.get());
    handle.close();
    assert!(host.closed.get());
}

#[test]
fn rich_context_popup_displays_arbitrary_layout_and_controls() {
    let target = elwindui::core::ui::TextBlock::new();
    let template = ViewTemplate::new(|_ctx| {
        let layout = elwindui::core::ui::VerticalLayout::new();
        let title = elwindui::core::ui::TextBlock::new();
        layout.children().add(Rc::clone(&title) as Rc<dyn UIElementExt>);
        Some(layout as Rc<dyn UIElementExt>)
    });

    target.set_context_popup(Some(template.clone()));

    let target_dyn: Rc<dyn UIElementExt> = target;
    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 50.0, y: 50.0 })));
    let (resolved, anchor) =
        ContextMenuService::process_request_for_target(&target_dyn, &request).expect("should resolve");

    match resolved.definition {
        ResolvedContextDefinition::Popup { template: t } => {
            let host = TestPopupHost::new();
            let work_area = Rect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            };
            let handle = ContextMenuService::open_custom_popup(
                &host,
                &resolved.owner,
                &t,
                &anchor,
                resolved.owner.effective_environment(),
                work_area,
            )
            .expect("owner is alive, template should build");

            assert_eq!(host.shown.borrow().len(), 1);
            let (_content, _pos, size) = &host.shown.borrow()[0];
            assert!(size.width > 0.0);
            assert!(size.height > 0.0);
            assert!(!host.closed.get());
            handle.close();
            assert!(host.closed.get());
        }
        _ => panic!("expected Popup definition"),
    }
}

#[elwindui::class(struct_only = elwindui::core::ui::MenuItemExt)]
struct TestMenuItem {
    text: RefCell<String>,
    enabled: Cell<bool>,
    shortcut: RefCell<Option<String>>,
    on_select: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

#[elwindui::class]
impl TestMenuItem {
    fn construct() -> Self {
        Self {
            text: RefCell::new(String::new()),
            enabled: Cell::new(true),
            shortcut: RefCell::new(None),
            on_select: Rc::new(RefCell::new(None)),
        }
    }
    fn text(&self) -> String {
        self.text.borrow().clone()
    }
    fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
    }
    fn enabled(&self) -> bool {
        self.enabled.get()
    }
    fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }
    fn shortcut(&self) -> Option<String> {
        self.shortcut.borrow().clone()
    }
    fn set_shortcut(&self, key_equivalent: &str) {
        *self.shortcut.borrow_mut() = if key_equivalent.is_empty() {
            None
        } else {
            Some(key_equivalent.to_string())
        };
    }
    fn set_on_select(&self, callback: Box<dyn Fn()>) {
        *self.on_select.borrow_mut() = Some(Rc::from(callback));
    }
    fn select(&self) {
        let cb = self.on_select.borrow().clone();
        if let Some(cb) = cb {
            cb();
        }
    }
}

#[test]
fn custom_menu_items_support_enabled_and_shortcut_semantics() {
    let item = TestMenuItem::new();
    item.set_text("Save As");
    item.set_shortcut("S");
    item.set_enabled(false);

    assert_eq!(item.text(), "Save As");
    assert_eq!(item.shortcut(), Some("S".to_string()));
    assert!(!item.enabled());

    item.set_enabled(true);
    assert!(item.enabled());
}

#[test]
fn custom_menu_keyboard_dispatcher_navigates_and_selects() {
    let host = TestPopupHost::new();
    let menu = TestMenu::new();

    let item1 = TestMenuItem::new();
    item1.set_text("First");
    let sel1 = Rc::new(Cell::new(false));
    let sel1_clone = Rc::clone(&sel1);
    item1.set_on_select(Box::new(move || sel1_clone.set(true)));
    menu.items.add(Rc::clone(&item1) as Rc<dyn MenuItemExt>);

    let item2 = TestMenuItem::new();
    item2.set_text("Second");
    let sel2 = Rc::new(Cell::new(false));
    let sel2_clone = Rc::clone(&sel2);
    item2.set_on_select(Box::new(move || sel2_clone.set(true)));
    menu.items.add(Rc::clone(&item2) as Rc<dyn MenuItemExt>);

    let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
    let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

    let _handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

    let (content, _, _) = &host.shown.borrow()[0];
    let keyboard = elwindui::core::input::KeyboardDispatcher::new();

    // Give focus to the menu root
    keyboard.focus.set_focus(content, elwindui::core::input::FocusState::Programmatic);

    // Send Key::Down -> highlights first item
    keyboard.handle_key(
        content,
        elwindui::core::input::RawKeyEvent {
            kind: elwindui::core::input::RawKeyEventKind::Down { is_repeat: false },
            key: elwindui::core::input::Key::Down,
            modifiers: elwindui::core::input::KeyModifiers::default(),
            timestamp_ms: 0.0,
        },
    );

    // Send Key::Down -> highlights second item
    keyboard.handle_key(
        content,
        elwindui::core::input::RawKeyEvent {
            kind: elwindui::core::input::RawKeyEventKind::Down { is_repeat: false },
            key: elwindui::core::input::Key::Down,
            modifiers: elwindui::core::input::KeyModifiers::default(),
            timestamp_ms: 0.0,
        },
    );

    // Send Key::Enter -> selects second item and closes popup
    keyboard.handle_key(
        content,
        elwindui::core::input::RawKeyEvent {
            kind: elwindui::core::input::RawKeyEventKind::Down { is_repeat: false },
            key: elwindui::core::input::Key::Enter,
            modifiers: elwindui::core::input::KeyModifiers::default(),
            timestamp_ms: 0.0,
        },
    );

    assert!(!sel1.get(), "first item should not be selected");
    assert!(sel2.get(), "second item should be selected via KeyboardDispatcher");
    assert!(host.closed.get(), "popup surface should be dismissed upon selection");
}

#[test]
fn custom_menu_requests_root_focus_policy() {
    let host = TestPopupHost::new();
    let menu = TestMenu::new();
    let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
    let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

    let _handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

    let req = host.last_request.borrow().clone().expect("must have received PopupRequest");
    assert_eq!(req.focus_policy, elwindui::core::ui::popup::PopupFocusPolicy::Root);
    assert_eq!(req.dismiss_policy, elwindui::core::ui::popup::PopupDismissPolicy::LightDismiss);
}

#[test]
fn custom_menu_popup_handle_cycle_and_weak_release() {
    let host = TestPopupHost::new();
    let menu = TestMenu::new();
    let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
    let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

    let handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);
    assert!(!host.closed.get(), "popup is initially open");

    // Explicit drop of the handle should trigger close() and cleanly release without cycle
    drop(handle);
    assert!(host.closed.get(), "popup surface should be closed when handle is dropped");
}

#[test]
fn calculate_placement_secondary_monitor_negative_coordinates() {
    // Secondary monitor positioned to the left: X [-1920, 0], Y [0, 1080]
    let work_area = Rect {
        x: -1920.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    let anchor = PopupAnchor::Point(Point { x: -100.0, y: 1000.0 });
    let popup_size = Size { width: 200.0, height: 150.0 };

    let pos = elwindui::core::ui::popup::calculate_popup_placement(
        &anchor,
        popup_size,
        work_area,
        elwindui::core::ui::popup::PopupPlacement::AutoFlip,
    );

    // Y should flip upward because 1000 + 150 = 1150 > 1080
    assert_eq!(pos.y, 850.0);
    // X should flip leftward because -100 + 200 = 100 > 0.0 (secondary monitor max X)
    assert_eq!(pos.x, -300.0);
}

#[elwindui::environment_key(
    name = popup_test_scope_theme,
    value = String,
    default = String::from("DefaultTheme")
)]
pub struct PopupTestScopeTheme;

thread_local! {
    static OBSERVED_SCOPE_THEME: RefCell<String> = RefCell::new(String::new());
    static LAST_TARGET_BLOCK: RefCell<Option<Rc<dyn UIElementExt>>> = RefCell::new(None);
}

#[elwindui::component(inherits ContentControl)]
struct PopupScopeChild {
    body: view! {
        on_mount {
            let template = ViewTemplate::new(|ctx| {
                OBSERVED_SCOPE_THEME.with(|c| {
                    *c.borrow_mut() = ctx.environment.get::<PopupTestScopeTheme>();
                });
                Some(VerticalLayout::new() as Rc<dyn UIElementExt>)
            });
            let target = self.target();
            target.set_context_popup(Some(template));
            LAST_TARGET_BLOCK.with(|c| *c.borrow_mut() = Some(Rc::clone(&target) as Rc<dyn UIElementExt>));
        }
        #[id("target")]
        let target = TextBlock {
            text: "Context target",
        };
        ContentControl {
            target
        }
    },
}

#[elwindui::component]
impl PopupScopeChild {}

#[elwindui::component(inherits VerticalLayout)]
struct PopupScopeParent {
    body: view! {
        EnvironmentScope {
            popup_test_scope_theme: "DerivedDarkTheme",
            PopupScopeChild {}
        }
    },
}

#[elwindui::component]
impl PopupScopeParent {}

#[test]
fn environment_scope_dsl_context_popup_integration() {
    OBSERVED_SCOPE_THEME.with(|c| *c.borrow_mut() = String::new());
    LAST_TARGET_BLOCK.with(|c| *c.borrow_mut() = None);

    let _parent = PopupScopeParent::new();
    let child_target = LAST_TARGET_BLOCK.with(|c| c.borrow().clone()).expect("target block must be mounted");

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 50.0, y: 50.0 })));
    let (resolved, anchor) =
        ContextMenuService::process_request_for_target(&child_target, &request).expect("should resolve target");

    let host = TestPopupHost::new();
    let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

    match resolved.definition {
        ResolvedContextDefinition::Popup { template: t } => {
            let _handle = ContextMenuService::open_custom_popup(
                &host,
                &resolved.owner,
                &t,
                &anchor,
                resolved.owner.effective_environment(),
                work_area,
            );

            let observed = OBSERVED_SCOPE_THEME.with(|c| c.borrow().clone());
            assert_eq!(
                observed,
                "DerivedDarkTheme",
                "popup template should inherit derived environment via actual EnvironmentScope DSL"
            );
        }
        _ => panic!("expected Popup definition"),
    }
}

#[test]
fn custom_menu_callback_mutates_state_and_resyncs_without_panic() {
    let host = TestPopupHost::new();
    let menu = TestMenu::new();

    let state_counter = Rc::new(RefCell::new(0));
    let counter_clone = Rc::clone(&state_counter);

    let item = TestMenuItem::new();
    item.set_text("Mutate State");
    let item_clone = item.clone();

    // Callback updates state, triggers setter, and checks item text (reentrant inspection)
    item.set_on_select(Box::new(move || {
        *counter_clone.borrow_mut() += 1;
        item_clone.set_text("Updated");
        assert_eq!(item_clone.text(), "Updated");
    }));
    menu.items.add(Rc::clone(&item) as Rc<dyn MenuItemExt>);

    let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
    let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

    let _handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

    let (content, _, _) = &host.shown.borrow()[0];
    let row = &content.visual_children()[0];

    // Trigger on_tapped
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        row,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 10.0, y: 10.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );

    assert_eq!(*state_counter.borrow(), 1, "callback should execute cleanly");
    assert!(host.closed.get(), "popup should close on selection");
}

#[test]
fn context_request_separates_local_hittest_from_screen_anchor() {
    let root = elwindui::core::ui::VerticalLayout::new();
    let menu = TestMenu::new();
    let child = elwindui::core::ui::TextBlock::new();
    child.set_context_menu(Some(Rc::clone(&menu) as Rc<dyn elwindui::core::ui::MenuExt>));
    root.children().add(Rc::clone(&child) as Rc<dyn UIElementExt>);

    // Local coordinates in window (e.g. at (10, 10)) vs desktop screen coordinates (e.g. at (1930, 500))
    let local_pos = Point { x: 0.0, y: 0.0 };
    let screen_pos = Point { x: 1930.0, y: 500.0 };

    let root_dyn: Rc<dyn UIElementExt> = root;
    let focus = elwindui::core::focus::FocusTracker::new();
    let request = ContextRequest::pointer(local_pos, screen_pos);

    let (resolved, anchor) = ContextMenuService::process_request(&root_dyn, &focus, &request)
        .expect("should hit-test child at local position");

    assert!(Rc::ptr_eq(&resolved.owner, &(child as Rc<dyn UIElementExt>)));
    match anchor {
        PopupAnchor::Point(pt) => {
            assert_eq!(pt.x, 1930.0, "anchor must use screen_position, not local_position");
            assert_eq!(pt.y, 500.0, "anchor must use screen_position, not local_position");
        }
        _ => panic!("expected Point anchor"),
    }
}

#[test]
fn context_request_without_screen_anchor_returns_none_and_never_falls_back_to_local_offset() {
    let menu = TestMenu::new();
    let target = elwindui::core::ui::TextBlock::new();
    target.set_context_menu(Some(Rc::clone(&menu) as Rc<dyn elwindui::core::ui::MenuExt>));
    let target_dyn: Rc<dyn UIElementExt> = target;

    let request_no_anchor = ContextRequest::keyboard(None);
    let resolved = ContextMenuService::process_request_for_target(&target_dyn, &request_no_anchor);
    assert!(
        resolved.is_none(),
        "process_request_for_target must return None when screen_anchor is missing and never fall back to local arranged offset"
    );
}

thread_local! {
    static POPUP_DISMISS_FIELD_UNMOUNT_COUNT: Cell<u32> = Cell::new(0);
}

/// Declares `popup_dismiss` through the ordinary `#[environment(name)]` field syntax (no
/// `#[elwindui::environment_key]` declaration needed — `popup_dismiss` is a framework built-in key,
/// same resolution path as the Semantic Style Brush keys, `component_frontend::
/// lookup_builtin_popup_dismiss_key`).
#[elwindui::component(inherits VerticalLayout)]
struct PopupDismissFieldContent {
    #[environment(popup_dismiss)]
    dismiss: Option<PopupDismissAction>,

    body: view! {
        on_unmount {
            POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock {
            text: "popup dismiss field content",
        }
    },
}

#[elwindui::component]
impl PopupDismissFieldContent {}

#[test]
fn popup_dismiss_environment_field_is_none_outside_a_popup() {
    let outside = PopupDismissFieldContent::new();
    assert!(
        outside.dismiss().is_none(),
        "popup_dismiss must resolve to None outside any popup-scoped Environment"
    );
}

#[test]
fn popup_dismiss_environment_field_resolves_and_dismisses_declaratively() {
    POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.set(0));

    let host = TestPopupHost::new();
    let owner: Rc<dyn UIElementExt> = elwindui::core::ui::TextBlock::new();

    let captured_dismiss: Rc<RefCell<Option<PopupDismissAction>>> = Rc::new(RefCell::new(None));
    let captured_clone = Rc::clone(&captured_dismiss);
    let template = ViewTemplate::new(move |ctx| {
        // Mirrors what #162's `context_popup: view! { PopupDismissFieldContent {} }` codegen will
        // generate: construct without auto-mounting, then mount explicitly against the popup-scoped
        // Environment `ctx` carries (the same `__new_unmounted`/`mount` split `EnvironmentScope`'s
        // own generated children already use).
        let instance = PopupDismissFieldContent::__new_unmounted();
        instance.mount(ctx.environment);
        *captured_clone.borrow_mut() = instance.dismiss();
        Some(instance.into_node())
    });

    let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
    let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

    let _handle = ContextMenuService::open_custom_popup(
        &host,
        &owner,
        &template,
        &anchor,
        owner.effective_environment(),
        work_area,
    )
    .expect("owner is alive, template should build");

    let dismiss = captured_dismiss
        .borrow()
        .clone()
        .expect("popup_dismiss must resolve to Some(..) inside the popup-scoped Environment");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    assert_eq!(POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.get()), 0);

    assert!(!host.closed.get());
    dismiss.dismiss();
    assert!(host.closed.get(), "declarative dismiss() must close the popup surface");

    // The test host doesn't itself run unmount_subtree on close (that's a backend responsibility,
    // exercised by elwindui-core's own teardown-ordering tests) — simulate it here, exactly as
    // AppKit's/WinUI3's close() now do, to prove on_unmount fires exactly once from the declarative
    // dismiss path end to end.
    unmount_subtree(&content);
    assert_eq!(
        POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.get()),
        1,
        "on_unmount must run exactly once after declarative dismiss"
    );

    // Idempotent: a second unmount_subtree (e.g. a duplicate dismissal path) must not re-run it.
    unmount_subtree(&content);
    assert_eq!(POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.get()), 1);
}

#[test]
fn popup_dismiss_field_content_repeated_open_close_has_independent_lifetimes() {
    // Regression for repeated-open/close leak-freedom (directive §33) and popup-replacement-style
    // sequencing (a new popup opened only after the previous one's close() completed, the same
    // ordering both backends' `dispatch_context_request` already enforce before calling
    // `open_custom_popup` again) — each open produces a fresh on_unmount call and a fresh
    // `PopupDismissAction`, and dismissing an already-closed popup a second time stays a safe no-op.
    POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.set(0));

    let host = TestPopupHost::new();
    let owner: Rc<dyn UIElementExt> = elwindui::core::ui::TextBlock::new();

    let open_and_close = || -> PopupDismissAction {
        let captured: Rc<RefCell<Option<PopupDismissAction>>> = Rc::new(RefCell::new(None));
        let captured_clone = Rc::clone(&captured);
        let template = ViewTemplate::new(move |ctx| {
            let instance = PopupDismissFieldContent::__new_unmounted();
            instance.mount(ctx.environment);
            *captured_clone.borrow_mut() = instance.dismiss();
            Some(instance.into_node())
        });
        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };
        let _handle = ContextMenuService::open_custom_popup(
            &host,
            &owner,
            &template,
            &anchor,
            owner.effective_environment(),
            work_area,
        )
        .expect("owner is alive, template should build");
        let content = Rc::clone(&host.shown.borrow().last().unwrap().0);
        // Mirrors the backend close() sequence this PR fixed: unmount before detach.
        unmount_subtree(&content);
        captured.borrow().clone().expect("popup_dismiss must resolve inside the popup")
    };

    let dismiss_a = open_and_close();
    assert_eq!(
        POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.get()),
        1,
        "popup A's on_unmount must fire exactly once"
    );

    let dismiss_b = open_and_close();
    assert_eq!(
        POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.get()),
        2,
        "popup B's on_unmount must fire independently of A's (no leaked/duplicated teardown)"
    );

    // Both are already-unmounted-content dismiss actions; calling either now must stay a safe
    // no-op — no cross-talk between the two popups' independent lifetimes, and no double-teardown.
    dismiss_a.dismiss();
    dismiss_b.dismiss();
    assert_eq!(POPUP_DISMISS_FIELD_UNMOUNT_COUNT.with(|c| c.get()), 2);
}

thread_local! {
    static PRE_SHOW_DISMISS_MOUNT_COUNT: Cell<u32> = Cell::new(0);
    static PRE_SHOW_DISMISS_UNMOUNT_COUNT: Cell<u32> = Cell::new(0);
}

/// Calls the declarative `popup_dismiss` action from its own `on_mount` — mirrors what a generated
/// declarative `context_popup: view! { .. }` root will be able to do once #162 lands (its Component
/// root mounts *inside* `ViewTemplate::build`, before any native popup surface exists).
#[elwindui::component(inherits VerticalLayout)]
struct PreShowDismissContent {
    #[environment(popup_dismiss)]
    dismiss: Option<PopupDismissAction>,

    body: view! {
        on_mount {
            PRE_SHOW_DISMISS_MOUNT_COUNT.with(|c| c.set(c.get() + 1));
            if let Some(dismiss) = self.dismiss() {
                dismiss.dismiss();
            }
        }
        on_unmount {
            PRE_SHOW_DISMISS_UNMOUNT_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock {
            text: "pre-show dismiss content",
        }
    },
}

#[elwindui::component]
impl PreShowDismissContent {}

#[test]
fn popup_dismiss_during_on_mount_prevents_popup_from_showing() {
    PRE_SHOW_DISMISS_MOUNT_COUNT.with(|c| c.set(0));
    PRE_SHOW_DISMISS_UNMOUNT_COUNT.with(|c| c.set(0));

    let host = TestPopupHost::new();
    let owner: Rc<dyn UIElementExt> = elwindui::core::ui::TextBlock::new();

    let weak_content: Rc<RefCell<Option<std::rc::Weak<dyn UIElementExt>>>> = Rc::new(RefCell::new(None));
    let weak_clone = Rc::clone(&weak_content);
    let template = ViewTemplate::new(move |ctx| {
        // Mirrors #162's planned codegen shape: construct without auto-mounting, then mount
        // explicitly against the popup-scoped Environment (`EnvironmentScope`'s own existing
        // pattern) — `on_mount` (and therefore the dismiss() call inside it) runs during this
        // `mount()` call, synchronously, before `open_custom_popup` ever calls `host.show_popup`.
        let instance = PreShowDismissContent::__new_unmounted();
        instance.mount(ctx.environment);
        let node = instance.into_node();
        *weak_clone.borrow_mut() = Some(Rc::downgrade(&node));
        Some(node)
    });

    let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
    let work_area = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };

    let handle = ContextMenuService::open_custom_popup(
        &host,
        &owner,
        &template,
        &anchor,
        owner.effective_environment(),
        work_area,
    );

    assert!(handle.is_none(), "a popup dismissed during on_mount must not be shown");
    assert_eq!(
        host.shown.borrow().len(),
        0,
        "the popup host's show_popup must never be called once a pre-show dismiss was requested"
    );
    assert_eq!(PRE_SHOW_DISMISS_MOUNT_COUNT.with(|c| c.get()), 1);
    assert_eq!(
        PRE_SHOW_DISMISS_UNMOUNT_COUNT.with(|c| c.get()),
        1,
        "content mounted before the pre-show dismiss must still be unmounted exactly once"
    );

    let weak = weak_content.borrow().clone().expect("template captured its content");
    assert!(
        weak.upgrade().is_none(),
        "content built/mounted before a pre-show dismiss must be released, not retained"
    );
}
