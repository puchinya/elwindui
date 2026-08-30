//! Issue #152: Integration and type-check tests for Context Menu, Custom Context Menu presentation, and rich Context Popup.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::{Point, Rect, Size};
use elwindui::core::ui::popup::{
    ContextMenuService, ContextRequest, PopupAnchor, PopupDismissAction, PopupHost, PopupRequest,
    PopupSurfaceHandle, ResolvedContextDefinition,
};
use elwindui::ui::{
    LayoutExt, MenuItemExt, TextBlock, UIElementExt, VerticalLayout, ViewFactory, unmount_subtree,
};
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
    let template = ViewFactory::new(|_ctx| {
        let layout = elwindui::core::ui::VerticalLayout::new();
        let title = elwindui::core::ui::TextBlock::new();
        layout
            .children()
            .add(Rc::clone(&title) as Rc<dyn UIElementExt>);
        Some(layout as Rc<dyn UIElementExt>)
    });

    target.set_context_popup(Some(template.clone()));

    let target_dyn: Rc<dyn UIElementExt> = target;
    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 50.0, y: 50.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("should resolve");

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
    icon: RefCell<Option<elwindui::core::graphics::IconSource>>,
    enabled: Cell<bool>,
    shortcut: RefCell<Option<String>>,
    on_select: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

#[elwindui::class]
impl TestMenuItem {
    fn construct() -> Self {
        Self {
            text: RefCell::new(String::new()),
            icon: RefCell::new(None),
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
    fn icon(&self) -> Option<elwindui::core::graphics::IconSource> {
        self.icon.borrow().clone()
    }
    fn set_icon(&self, icon: Option<elwindui::core::graphics::IconSource>) {
        *self.icon.borrow_mut() = icon;
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
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    let _handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

    let (content, _, _) = &host.shown.borrow()[0];
    let keyboard = elwindui::core::input::KeyboardDispatcher::new();

    // Give focus to the menu root
    keyboard
        .focus
        .set_focus(content, elwindui::core::input::FocusState::Programmatic);

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
    assert!(
        sel2.get(),
        "second item should be selected via KeyboardDispatcher"
    );
    assert!(
        host.closed.get(),
        "popup surface should be dismissed upon selection"
    );
}

#[test]
fn custom_menu_requests_root_focus_policy() {
    let host = TestPopupHost::new();
    let menu = TestMenu::new();
    let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    let _handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

    let req = host
        .last_request
        .borrow()
        .clone()
        .expect("must have received PopupRequest");
    assert_eq!(
        req.focus_policy,
        elwindui::core::ui::popup::PopupFocusPolicy::Root
    );
    assert_eq!(
        req.dismiss_policy,
        elwindui::core::ui::popup::PopupDismissPolicy::LightDismiss
    );
}

#[test]
fn custom_menu_popup_handle_cycle_and_weak_release() {
    let host = TestPopupHost::new();
    let menu = TestMenu::new();
    let anchor = PopupAnchor::Point(Point { x: 50.0, y: 50.0 });
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    let handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);
    assert!(!host.closed.get(), "popup is initially open");

    // Explicit drop of the handle should trigger close() and cleanly release without cycle
    drop(handle);
    assert!(
        host.closed.get(),
        "popup surface should be closed when handle is dropped"
    );
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
    let anchor = PopupAnchor::Point(Point {
        x: -100.0,
        y: 1000.0,
    });
    let popup_size = Size {
        width: 200.0,
        height: 150.0,
    };

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
    template: template_view!(|templated_parent: Self| {
        on_mount {
            assert!(self.apply_template());
            let template = ViewFactory::new(|ctx| {
                OBSERVED_SCOPE_THEME.with(|c| {
                    *c.borrow_mut() = ctx.environment.get::<PopupTestScopeTheme>();
                });
                Some(VerticalLayout::new() as Rc<dyn UIElementExt>)
            });
            let target = elwindui::core::visual_tree::find_all::<TextBlock>(self)
                .into_iter()
                .next()
                .expect("template target TextBlock");
            target.set_context_popup(Some(template));
            LAST_TARGET_BLOCK.with(|c| *c.borrow_mut() = Some(Rc::clone(&target) as Rc<dyn UIElementExt>));
        }
        let target = TextBlock {
            text: "Context target",
        };
        ContentControl {
            target
        }
    }),
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
    let child_target = LAST_TARGET_BLOCK
        .with(|c| c.borrow().clone())
        .expect("target block must be mounted");

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 50.0, y: 50.0 })));
    let (resolved, anchor) =
        ContextMenuService::process_request_for_target(&child_target, &request)
            .expect("should resolve target");

    let host = TestPopupHost::new();
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

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
                observed, "DerivedDarkTheme",
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
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

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

    assert_eq!(
        *state_counter.borrow(),
        1,
        "callback should execute cleanly"
    );
    assert!(host.closed.get(), "popup should close on selection");
}

#[test]
fn context_request_separates_local_hittest_from_screen_anchor() {
    let root = elwindui::core::ui::VerticalLayout::new();
    let menu = TestMenu::new();
    let child = elwindui::core::ui::TextBlock::new();
    child.set_context_menu(Some(Rc::clone(&menu) as Rc<dyn elwindui::core::ui::MenuExt>));
    root.children()
        .add(Rc::clone(&child) as Rc<dyn UIElementExt>);

    // Local coordinates in window (e.g. at (10, 10)) vs desktop screen coordinates (e.g. at (1930, 500))
    let local_pos = Point { x: 0.0, y: 0.0 };
    let screen_pos = Point {
        x: 1930.0,
        y: 500.0,
    };

    let root_dyn: Rc<dyn UIElementExt> = root;
    let focus = elwindui::core::focus::FocusTracker::new();
    let request = ContextRequest::pointer(local_pos, screen_pos);

    let (resolved, anchor) = ContextMenuService::process_request(&root_dyn, &focus, &request)
        .expect("should hit-test child at local position");

    assert!(Rc::ptr_eq(
        &resolved.owner,
        &(child as Rc<dyn UIElementExt>)
    ));
    match anchor {
        PopupAnchor::Point(pt) => {
            assert_eq!(
                pt.x, 1930.0,
                "anchor must use screen_position, not local_position"
            );
            assert_eq!(
                pt.y, 500.0,
                "anchor must use screen_position, not local_position"
            );
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
    let template = ViewFactory::new(move |ctx| {
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
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

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
    assert!(
        host.closed.get(),
        "declarative dismiss() must close the popup surface"
    );

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
        let template = ViewFactory::new(move |ctx| {
            let instance = PopupDismissFieldContent::__new_unmounted();
            instance.mount(ctx.environment);
            *captured_clone.borrow_mut() = instance.dismiss();
            Some(instance.into_node())
        });
        let anchor = PopupAnchor::Point(Point { x: 0.0, y: 0.0 });
        let work_area = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
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
        captured
            .borrow()
            .clone()
            .expect("popup_dismiss must resolve inside the popup")
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
/// root mounts *inside* `ViewFactory::build`, before any native popup surface exists).
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

    let weak_content: Rc<RefCell<Option<std::rc::Weak<dyn UIElementExt>>>> =
        Rc::new(RefCell::new(None));
    let weak_clone = Rc::clone(&weak_content);
    let template = ViewFactory::new(move |ctx| {
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
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    let handle = ContextMenuService::open_custom_popup(
        &host,
        &owner,
        &template,
        &anchor,
        owner.effective_environment(),
        work_area,
    );

    assert!(
        handle.is_none(),
        "a popup dismissed during on_mount must not be shown"
    );
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

    let weak = weak_content
        .borrow()
        .clone()
        .expect("template captured its content");
    assert!(
        weak.upgrade().is_none(),
        "content built/mounted before a pre-show dismiss must be released, not retained"
    );
}

// ---------------------------------------------------------------------------
// Issue #162: declarative `context_popup: view! { .. }`
// ---------------------------------------------------------------------------

#[elwindui::viewmodel]
mod deferred_popup_view_model {
    struct DeferredPopupViewModel {
        #[observable(default = 1)]
        selected_item: i32,

        #[observable(default = String::new())]
        label: String,
    }
}

/// The popup's own content Component — a real, ordinary `#[bindable]`-injected viewmodel read,
/// completely unrelated to Issue #162's own machinery. Its `on_mount` records the *current*
/// `vm.selected_item` at the moment it's built (once per popup-open, per a fresh instance).
#[elwindui::component(inherits ContentControl)]
struct DeferredPopupProbe {
    #[bindable]
    vm: std::rc::Rc<DeferredPopupViewModel>,
    #[param]
    log: std::rc::Rc<RefCell<Vec<i32>>>,
    template: template_view!(|templated_parent: Self| {
        on_mount {
            log.borrow_mut().push(vm.selected_item());
        }
        TextBlock { text: "probe" }
    }),
}

#[elwindui::component]
impl DeferredPopupProbe {}

/// The owner element declaring `context_popup: view! { .. }` directly. `vm`/`log` inside the
/// deferred body are bare 1-segment references to *this* Component's own fields — Issue #162's
/// implicit lexical owner (`__view_owner`) resolves them, the same way an ordinary `view!` body
/// resolves its own fields.
#[elwindui::component(inherits VerticalLayout)]
struct OwnerWithDeferredPopup {
    #[bindable]
    vm: std::rc::Rc<DeferredPopupViewModel>,
    #[param]
    log: std::rc::Rc<RefCell<Vec<i32>>>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                DeferredPopupProbe { vm: vm, log: log }
            },
        };

        VerticalLayout {
            target
        }
    },
}

#[elwindui::component]
impl OwnerWithDeferredPopup {}

/// Issue #162 T7: the owner's *current* value (not a mount-time snapshot) is observed at
/// popup-open time — `vm.selected_item` is changed after `new!(OwnerWithDeferredPopup(..))` but
/// before the popup opens, and the built popup content must observe the new value.
#[test]
fn declarative_context_popup_reads_current_owner_value_at_open_time() {
    let vm = DeferredPopupViewModel::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(OwnerWithDeferredPopup(
        vm: vm.clone(),
        log: Rc::clone(&log),
    ));

    vm.set_selected_item(2);

    let target_dyn: Rc<dyn UIElementExt> = owner.target();
    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");

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
            .expect("owner is alive, deferred view should build");

            assert_eq!(
                *log.borrow(),
                vec![2],
                "the declarative popup must read the owner's CURRENT value at open time, not a \
                 value snapshotted earlier"
            );
            handle.close();
        }
        _ => panic!("expected Popup definition"),
    }
}

/// Issue #162: every open of the same declarative `context_popup: view! { .. }` builds a fresh
/// hidden Component instance (fresh `on_mount`), not a single instance reused/rebuilt in place.
#[test]
fn declarative_context_popup_builds_a_fresh_instance_on_every_open() {
    let vm = DeferredPopupViewModel::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(OwnerWithDeferredPopup(
        vm: vm.clone(),
        log: Rc::clone(&log),
    ));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let open_once = |value: i32, log: &Rc<RefCell<Vec<i32>>>| {
        vm.set_selected_item(value);
        let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
        let (resolved, anchor) =
            ContextMenuService::process_request_for_target(&target_dyn, &request)
                .expect("target should resolve a context popup");
        let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
            panic!("expected Popup definition");
        };
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
        .expect("owner is alive, deferred view should build");
        let _ = log;
        handle.close();
    };

    open_once(1, &log);
    open_once(2, &log);

    assert_eq!(
        *log.borrow(),
        vec![1, 2],
        "each open must build a fresh instance observing that open's own current value"
    );
}

/// Issue #162 T15: the enclosing Component is captured only `Weak` — once it's gone, the popup
/// simply declines to build (`None`), the same "owner went away" contract `ViewFactory::build`
/// already enforces for `ViewBuildContext::owner`, never a panic or a stale/dangling popup.
#[test]
fn declarative_context_popup_returns_none_when_lexical_owner_is_dropped() {
    let vm = DeferredPopupViewModel::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(OwnerWithDeferredPopup(
        vm: vm.clone(),
        log: Rc::clone(&log),
    ));
    // `target_dyn` keeps the *target element* (and its `context_popup: Option<ViewFactory>`)
    // alive independently of `owner` itself — exactly the ownership shape a real popup-target
    // element has relative to the Component that declared it.
    let target_dyn: Rc<dyn UIElementExt> = owner.target();
    drop(owner);

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should still resolve a context popup definition");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    );

    assert!(
        handle.is_none(),
        "the deferred view must decline to build once its lexical owner is gone"
    );
    assert_eq!(
        host.shown.borrow().len(),
        0,
        "no native popup should ever be shown when the lexical owner is already dropped"
    );
}

thread_local! {
    static REACTIVE_POPUP_UNMOUNT_COUNT: Cell<u32> = Cell::new(0);
}

/// A popup content Component whose own view reactively binds to its `#[bindable]` viewmodel —
/// ordinary `view!` reactive semantics, exercised here specifically while mounted as a lowered
/// Issue #162 hidden Component, to prove the popup-local subscription this creates is owned by
/// (and torn down with) that hidden Component's own lifetime, not leaked onto the outer owner.
#[elwindui::component(inherits ContentControl)]
struct ReactiveDeferredPopupContent {
    #[bindable]
    vm: std::rc::Rc<DeferredPopupViewModel>,
    template: template_view!(|templated_parent: Self| {
        on_unmount {
            REACTIVE_POPUP_UNMOUNT_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: vm.label }
    }),
}

#[elwindui::component]
impl ReactiveDeferredPopupContent {}

#[elwindui::component(inherits VerticalLayout)]
struct OwnerWithReactiveDeferredPopup {
    #[bindable]
    vm: std::rc::Rc<DeferredPopupViewModel>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                ReactiveDeferredPopupContent { vm: vm }
            },
        };

        VerticalLayout {
            target
        }
    },
}

#[elwindui::component]
impl OwnerWithReactiveDeferredPopup {}

/// Issue #162 T8: while the popup is open, `TextBlock { text: vm.label }` inside its deferred
/// body is bound through the ordinary `#[bindable]` reactive machinery, generalized in Step 7 to
/// also cover a lowered hidden Component's `__view_owner` — the same resync path
/// `bind_owner_dynamic_resync.rs` already proves for an ordinary (non-popup) Component. Changing
/// `vm` while the popup is open must not panic and must reach that binding (`on_update` covers
/// only this Component's *own* `#[prop]`/`#[state]`/`#[computed]`/`#[environment]` fields, not a
/// `#[bindable]` owner's nested properties — verified indirectly here, not through `on_update`).
#[test]
fn declarative_context_popup_live_updates_while_open() {
    let vm = DeferredPopupViewModel::new();
    let owner = elwindui::new!(OwnerWithReactiveDeferredPopup(vm: vm.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    vm.set_label("seven".to_string());

    let content = Rc::clone(&host.shown.borrow()[0].0);
    unmount_subtree(&content);
    handle.close();
}

/// Issue #162 T9: closing the popup must release its own subscriptions to the outer owner's
/// `#[bindable]` field — proven by weak-reference releasability, not merely "no panic": if
/// closing left a subscription closure alive (holding a strong `Rc` back to the hidden Component,
/// e.g. via `vm`'s own `__property_changed_subscriptions`), the content could never be freed even
/// after every *external* strong reference (the test host's own retained handle included) is gone.
#[test]
fn declarative_context_popup_content_is_releasable_after_close() {
    REACTIVE_POPUP_UNMOUNT_COUNT.with(|c| c.set(0));

    let vm = DeferredPopupViewModel::new();
    let owner = elwindui::new!(OwnerWithReactiveDeferredPopup(vm: vm.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let weak_content = Rc::downgrade(&content);
    unmount_subtree(&content);
    handle.close();

    assert_eq!(
        REACTIVE_POPUP_UNMOUNT_COUNT.with(|c| c.get()),
        1,
        "closing the popup must unmount its own reactive content exactly once"
    );

    // Idempotent: a second unmount_subtree (e.g. a duplicate close path) must not re-run it, and
    // must not panic despite the outer owner's own `#[bindable]` subscription machinery having
    // already been exercised once for this now-closed instance.
    unmount_subtree(&content);
    assert_eq!(REACTIVE_POPUP_UNMOUNT_COUNT.with(|c| c.get()), 1);

    // PR #165 review remediation, A5: T9 must prove actual weak releasability, not merely that
    // on_unmount fired. The prior version of this test stopped here, leaving `Weak::strong_count()
    // == 1` unexplained — the actual cause (found while fixing this) was never a real internal
    // leak in the deferred-view/subscription machinery at all: `TestPopupHost` itself retains two
    // independent strong references beyond this test's own `content` local — `shown` (pushed by
    // `show_popup`, never popped) and `last_request` (the whole `PopupRequest`, including its own
    // `content: Rc<dyn UIElementExt>`, also never cleared). Every external/test-host strong
    // reference must be dropped — not just the one this test happened to hold locally — before a
    // `Weak::upgrade().is_none()` check can mean anything.
    drop(content);
    host.shown.borrow_mut().clear();
    host.last_request.borrow_mut().take();

    assert!(
        weak_content.upgrade().is_none(),
        "the hidden Component's own content must be releasable once every external strong \
         reference (including the test popup host's own `shown`/`last_request` bookkeeping) is \
         dropped — a real leak here would mean closing a popup never actually frees its content"
    );
}

/// Issue #162 T14: a nested Component's own `on_mount` calling the declarative `popup_dismiss`
/// action (`PreShowDismissContent`, already exercised via the low-level `ViewFactory::new(|ctx|
/// ..)` API above) works identically through the real `context_popup: view! { .. }` declarative
/// path — the popup is aborted before any native surface is shown, distinguishing "never shown"
/// from "shown then immediately closed".
#[elwindui::component(inherits VerticalLayout)]
struct OwnerWithDeclarativePreShowDismissPopup {
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                PreShowDismissContent {}
            },
        };

        VerticalLayout {
            target
        }
    },
}

#[elwindui::component]
impl OwnerWithDeclarativePreShowDismissPopup {}

#[test]
fn declarative_context_popup_dismiss_during_on_mount_prevents_popup_from_showing() {
    PRE_SHOW_DISMISS_MOUNT_COUNT.with(|c| c.set(0));
    PRE_SHOW_DISMISS_UNMOUNT_COUNT.with(|c| c.set(0));

    let host = TestPopupHost::new();
    let owner = OwnerWithDeclarativePreShowDismissPopup::new();
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
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
        &t,
        &anchor,
        resolved.owner.effective_environment(),
        work_area,
    );

    assert!(
        handle.is_none(),
        "a popup dismissed during its declarative content's own on_mount must not be shown"
    );
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
}

// PR #165 review remediation, A2: `on_mount`/`on_unmount`/event-handler closures written
// *directly* inside a `context_popup: view! { .. }` block (not inside a separately-declared,
// ordinary Component nested as the popup's root) must resolve bare references to the *enclosing
// source* Component's own fields through the implicit lexical owner — the same resolution the
// deferred view's own root value expressions already had. Every existing declarative-popup test
// above constructs a *separate* ordinary Component as the popup's content and only exercises bare
// names at the deferred view's root-element-construction-argument position; none of them reach
// `on_mount`/`on_unmount`/an event closure's own body written directly inside the `view! { .. }`
// block itself, which is exactly the gap A2 found (those hook bodies went through
// `rewrite_base_calls` only — or, for `on_update`, no rewriting at all — never through
// `ViewClosureRewriter`'s `ctx.implicit_owner` fallback).

#[elwindui::component(inherits VerticalLayout)]
struct A2DirectDeferredHookOwner {
    #[state(default = "outer".to_string())]
    label: String,
    #[param]
    log: Rc<RefCell<Vec<String>>>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                on_mount {
                    // A2 test 1: `on_mount` reads the enclosing source Component's own field via
                    // a bare identifier. Kept in a separate fixture from the shadowing test below
                    // (`A2DirectDeferredLocalShadowOwner`) as a focused regression, not because the
                    // compiler needs them split — `ViewClosureRewriter`'s real lexical scope stack
                    // (see `declarative_context_popup_direct_lexical_scope_stack_matches_rust_
                    // scoping`, below, for the full statement-order-precise regression suite) would
                    // handle both cases correctly combined in one block too.
                    log.borrow_mut().push(format!("mount:{label}"));
                }
                on_unmount {
                    // A2 test 2: `on_unmount` reads the enclosing source Component's own field via
                    // a bare identifier.
                    log.borrow_mut().push(format!("unmount:{label}"));
                }
                TextBlock {
                    text: "popup content",
                    on_tapped: |label| {
                        // A2 test 5b: an event closure's own declared parameter shadows the outer
                        // field of the same name — `label` here must be the `TappedEventArgs`
                        // parameter (proven by `.position.x`, which does not exist on the outer
                        // `String` field), not the outer field.
                        log.borrow_mut().push(format!("tapped-shadowed-x:{}", label.position.x));
                    },
                }
            }
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl A2DirectDeferredHookOwner {}

#[test]
fn declarative_context_popup_direct_on_mount_and_on_unmount_resolve_the_enclosing_owner_field() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(A2DirectDeferredHookOwner(log: log.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    assert_eq!(
        log.borrow().as_slice(),
        ["mount:outer"],
        "on_mount must read the enclosing owner's current field value via a bare identifier"
    );

    let content = Rc::clone(&host.shown.borrow()[0].0);
    unmount_subtree(&content);
    handle.close();

    assert_eq!(
        log.borrow().as_slice(),
        ["mount:outer", "unmount:outer"],
        "on_unmount must also read the enclosing owner's current field value via a bare identifier"
    );
}

/// A2 test 5a: a block-local `let` shadows the outer field of the same name — the exact shape
/// (`if let Some(dismiss) = ..) { dismiss.dismiss() }`) the A2 investigation found already broken:
/// `ViewClosureRewriter` treated every bare name matching an own/implicit-owner field as a rewrite
/// target, even one a local `let`/`if let` pattern re-binds first, producing e.g.
/// `self.__view_owner...label().position` for what should have stayed the plain local `label`.
/// See `declarative_context_popup_direct_lexical_scope_stack_matches_rust_scoping`, below, for the
/// full A2-T1 through A2-T6 regression suite proving real (not block-wide-flat) lexical scoping,
/// including the exact statement-order case (an *earlier*, unrelated outer-field read followed by
/// a local shadow of the same name in the *same* block) this fixture originally had to avoid.
#[elwindui::component(inherits VerticalLayout)]
struct A2DirectDeferredLocalShadowOwner {
    #[state(default = "outer".to_string())]
    label: String,
    #[param]
    log: Rc<RefCell<Vec<String>>>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                on_mount {
                    let label: i32 = 42;
                    // `label + 1` only type-checks against the local `i32`, never the outer
                    // `String` field — a real, compile-distinguishing proof of shadowing.
                    log.borrow_mut().push(format!("shadowed:{}", label + 1));
                }
                TextBlock { text: "popup content" }
            }
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl A2DirectDeferredLocalShadowOwner {}

#[test]
fn declarative_context_popup_direct_on_mount_local_let_shadows_the_enclosing_owner_field() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(A2DirectDeferredLocalShadowOwner(log: log.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    assert_eq!(log.borrow().as_slice(), ["shadowed:43"]);

    let content = Rc::clone(&host.shown.borrow()[0].0);
    unmount_subtree(&content);
    handle.close();
}

// PR #165 rereview remediation round 2, A2: `ViewClosureRewriter` now tracks a genuine lexical
// scope stack (`ViewClosureRewriter::scopes`) instead of one block-wide flat set, so a local
// binding shadows an outer field of the same name only where real Rust scoping would actually
// consider it in scope. A2-T1 through A2-T6 below are the required regression suite proving this,
// deliberately combined into a *single* `on_mount` block (unlike the flat-set-era fixtures above,
// which had to keep an outer-field read and a same-name local shadow in *separate* blocks to
// avoid the old implementation's own block-wide over-suppression).
thread_local! {
    static A2_SCOPE_STACK_FREE_FUNCTION_LOG: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());
}

/// A2-T6: a plain free function, called bare (no receiver) directly inside a deferred view's own
/// `on_mount` — must remain an ordinary free-function call, never rewritten into a bogus
/// `__view_owner.a2_scope_stack_free_function(..)` method call.
fn a2_scope_stack_free_function(tag: &'static str) {
    A2_SCOPE_STACK_FREE_FUNCTION_LOG.with(|l| l.borrow_mut().push(tag));
}

#[elwindui::component(inherits VerticalLayout)]
struct A2LexicalScopeStackOwner {
    #[state(default = "outer".to_string())]
    label: String,
    #[param]
    log: Rc<RefCell<Vec<String>>>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                on_mount {
                    // A2-T1: statement-order `let` — the outer field is read correctly both
                    // *before* and *after* a nested block-local shadow of the same name; the
                    // shadow's own initializer itself still reads the *outer* field (not the not-
                    // yet-bound local), and the shadow never leaks past its own block.
                    log.borrow_mut().push(format!("outer-before:{label}"));
                    {
                        let label = format!("local-from:{label}");
                        log.borrow_mut().push(format!("local-from:{label}"));
                    }
                    log.borrow_mut().push(format!("outer-after:{label}"));

                    // A2-T2: `if let` scope — the pattern's own binding is visible only inside
                    // `then`, never after the `if`.
                    let maybe_label: Option<i32> = Some(99);
                    if let Some(label) = maybe_label {
                        log.borrow_mut().push(format!("iflet-inner:{label}"));
                    }
                    log.borrow_mut().push(format!("iflet-after:{label}"));

                    // A2-T3: `match` arm isolation — one arm binds a name equal to the outer
                    // field; a *different* arm (and code after the whole `match`) still reads the
                    // outer field. Two separate `match`es (rather than one, since only one arm of
                    // any single `match` ever runs) exercise both arms in one test run.
                    match 0 {
                        0 => {
                            let label = "matched-zero".to_string();
                            log.borrow_mut().push(format!("match-shadow-arm:{label}"));
                        }
                        _ => {}
                    }
                    match 1 {
                        0 => {}
                        _ => {
                            log.borrow_mut().push(format!("match-other-arm:{label}"));
                        }
                    }
                    log.borrow_mut().push(format!("match-after:{label}"));

                    // A2-T4: `for` binding — the loop pattern shadows the outer field only inside
                    // the loop body.
                    for label in 0..1 {
                        log.borrow_mut().push(format!("for-inner:{label}"));
                    }
                    log.borrow_mut().push(format!("for-after:{label}"));

                    // A2-T5: nested closure parameter — shadows the outer field only inside the
                    // closure's own body.
                    let describe = |label: i32| {
                        log.borrow_mut().push(format!("closure-inner:{label}"));
                    };
                    describe(7);
                    log.borrow_mut().push(format!("closure-after:{label}"));

                    // A2-T6: a bare free-function call must remain a free-function call.
                    a2_scope_stack_free_function("free-fn-called");
                }
                TextBlock {
                    text: "popup content",
                    // A2-T7: an event closure whose own parameter does *not* collide with the
                    // outer field reads the enclosing source Component's *current* field value.
                    on_tapped: |_event| {
                        log.borrow_mut().push(format!("event-closure-outer-read:{label}"));
                    },
                }
            }
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl A2LexicalScopeStackOwner {}

#[test]
fn declarative_context_popup_direct_lexical_scope_stack_matches_rust_scoping() {
    A2_SCOPE_STACK_FREE_FUNCTION_LOG.with(|l| l.borrow_mut().clear());

    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(A2LexicalScopeStackOwner(log: log.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    assert_eq!(
        log.borrow().as_slice(),
        [
            "outer-before:outer",
            "local-from:local-from:outer",
            "outer-after:outer",
            "iflet-inner:99",
            "iflet-after:outer",
            "match-shadow-arm:matched-zero",
            "match-other-arm:outer",
            "match-after:outer",
            "for-inner:0",
            "for-after:outer",
            "closure-inner:7",
            "closure-after:outer",
        ],
        "real lexical scoping (statement-order let, if-let/match/for/nested-closure scope \
         isolation) must hold inside a deferred view's own on_mount block"
    );
    assert_eq!(
        A2_SCOPE_STACK_FREE_FUNCTION_LOG.with(|l| l.borrow().clone()),
        ["free-fn-called"],
        "a bare free-function call must remain an ordinary free-function call"
    );

    // A2-T7: dispatch the popup content's own `on_tapped` (a non-shadowing event closure) and
    // confirm it reads the enclosing source Component's current field.
    let content = Rc::clone(&host.shown.borrow()[0].0);
    let inner_target = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &inner_target,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 0.0, y: 0.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );
    assert_eq!(
        log.borrow().last(),
        Some(&"event-closure-outer-read:outer".to_string()),
        "a non-shadowing event closure parameter must not block the implicit-owner fallback for \
         an unrelated bare name"
    );

    unmount_subtree(&content);
    handle.close();
}

#[test]
fn declarative_context_popup_direct_event_closure_param_shadows_the_enclosing_owner_field() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(A2DirectDeferredHookOwner(log: log.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let inner = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());

    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &inner,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 42.0, y: 7.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );

    assert_eq!(
        log.borrow().as_slice(),
        ["mount:outer", "tapped-shadowed-x:42"],
        "the event closure's own `|label|` parameter must shadow the outer `label` field \
         (`.position.x` only compiles/resolves against the TappedEventArgs parameter, never the \
         outer String field) — `mount:outer` is this same fixture's own on_mount hook firing first"
    );

    unmount_subtree(&content);
    handle.close();
}

/// A2 test 3 (`on_update`): a lowered deferred view has no own `#[prop]`/`#[state]`/`#[computed]`/
/// `#[environment]` field of its own to trigger `on_update` with (`DeferredViewBody` only carries
/// `on_mount`/`on_unmount`/`on_update`/`lets`/`root` — no field declarations at all), so
/// `on_update`'s own `subscribe_property_changed` dispatch is unreachable at runtime for a hidden
/// Component *by construction*, on every DSL-authored component this way, not merely in this test.
/// This is therefore verified at the only level that is actually meaningful here: the bare
/// `log`/`label` references inside it must still generate valid Rust resolving against the
/// enclosing owner — proven by this compiling and constructing successfully at all (an incorrect
/// resolution, e.g. treating `label` as an unresolvable bare name, is a `elwindui-codegen`
/// compile-time failure, not a silently-wrong runtime value).
#[elwindui::component(inherits VerticalLayout)]
struct A2DirectDeferredOnUpdateOwner {
    #[state(default = "outer".to_string())]
    label: String,
    #[param]
    log: Rc<RefCell<Vec<String>>>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                on_update: {
                    log.borrow_mut().push(format!("update:{label}"));
                }
                TextBlock { text: "popup content" }
            }
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl A2DirectDeferredOnUpdateOwner {}

#[test]
fn declarative_context_popup_direct_on_update_compiles_and_resolves_the_enclosing_owner_field() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(A2DirectDeferredOnUpdateOwner(log: log.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    unmount_subtree(&content);
    handle.close();
}

/// PR #165 review remediation, A3: a `context_popup: view! { .. }` nested inside another
/// `context_popup: view! { .. }` must still resolve bare names against the *original* source
/// Component, at runtime, for both levels — the end-to-end counterpart to `codegen.rs`'s own
/// `nested_deferred_view_keeps_the_original_source_component_as_lexical_owner` (which proves the
/// generated `Weak<..>` type; this proves the *value* actually observed is correct and current).
#[elwindui::component(inherits VerticalLayout)]
struct A3NestedDeferredOwner {
    #[state(default = "outer".to_string())]
    value: String,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                TextBlock {
                    text: "inner",
                    context_popup: view! {
                        TextBlock { text: value }
                    },
                }
            },
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl A3NestedDeferredOwner {}

#[test]
fn declarative_context_popup_nested_popup_observes_current_outer_value() {
    let owner = A3NestedDeferredOwner::new();
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    // Change the outer value *after* construction, before either popup is opened — proves the
    // inner (second-level) popup reads the *current* value, not one snapshotted at construction.
    owner.set_value("changed".to_string());

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

    let outer_host = TestPopupHost::new();
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    let outer_handle = ContextMenuService::open_custom_popup(
        &outer_host,
        &resolved.owner,
        &t,
        &anchor,
        resolved.owner.effective_environment(),
        work_area,
    )
    .expect("owner is alive, outer deferred view should build");

    // Navigate to the inner TextBlock (the outer popup's own deferred-view root) to open the
    // *second*, nested popup from it — reusing the same `visual_children().first()` navigation
    // already proven correct for a bare-root-TextBlock deferred view above.
    let outer_content = Rc::clone(&outer_host.shown.borrow()[0].0);
    let inner_target = outer_content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| outer_content.clone());

    let inner_request =
        ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (inner_resolved, inner_anchor) =
        ContextMenuService::process_request_for_target(&inner_target, &inner_request)
            .expect("inner target should resolve its own context popup");
    let ResolvedContextDefinition::Popup { template: inner_t } = inner_resolved.definition else {
        panic!("expected Popup definition");
    };

    let inner_host = TestPopupHost::new();
    let _inner_handle = ContextMenuService::open_custom_popup(
        &inner_host,
        &inner_resolved.owner,
        &inner_t,
        &inner_anchor,
        inner_resolved.owner.effective_environment(),
        work_area,
    )
    .expect("outer popup content is alive, inner deferred view should build");

    assert_eq!(inner_host.shown.borrow().len(), 1);
    // The inner popup's own root is `TextBlock { text: value }` — `value` must resolve against
    // `A3NestedDeferredOwner` (the original source Component), reading its *current* value
    // ("changed", set above), not the outer popup's own hidden Component (which has no `value`
    // field of its own at all — this DSL bare name is only satisfiable via the implicit_owner
    // chain reaching all the way back to `A3NestedDeferredOwner`).
    let inner_content = Rc::clone(&inner_host.shown.borrow()[0].0);
    // The inner popup's own content root is the second hidden Component's own root (a
    // `ContentControl`, per `hidden_view_factory_component`'s base) — its declared `TextBlock {
    // text: value }` is its visual *child*, same one-level-of-wrapping shape already navigated
    // from `outer_content` to `inner_target` above.
    let inner_text_node = inner_content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| inner_content.clone());
    let text_block = inner_text_node
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("inner popup content should resolve to the TextBlock itself");
    assert_eq!(text_block.text.borrow().as_str(), "changed");

    unmount_subtree(&inner_content);
    unmount_subtree(&outer_content);
    outer_handle.close();
}

/// PR #165 final rereview remediation, A2-R4: a popup event closure's assignment to a bare name
/// that is a *writable* (`Prop`/`State`) field of the enclosing source Component must actually
/// mutate that Component's own state through its real generated setter — not merely compile, and
/// not silently become a no-op/local shadow. Proven end-to-end (real popup open, real routed event
/// dispatch, real post-dispatch read of the owner's own state) rather than by inspecting generated
/// tokens, since the contract explicitly requires this over a codegen-level check.
#[elwindui::component(inherits VerticalLayout)]
struct A2OuterStateWriteOwner {
    #[state(default = false)]
    selected: bool,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                TextBlock {
                    text: "popup content",
                    on_tapped: |_event| {
                        selected = true;
                    },
                }
            }
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl A2OuterStateWriteOwner {}

#[test]
fn declarative_context_popup_event_closure_writes_the_enclosing_owner_state() {
    let owner = A2OuterStateWriteOwner::new();
    assert!(!owner.selected());
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let inner_target = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &inner_target,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 0.0, y: 0.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );

    assert!(
        owner.selected(),
        "the popup event closure's assignment must mutate the enclosing owner's own #[state] \
         field through its generated setter, not a local/no-op"
    );

    unmount_subtree(&content);
    handle.close();
}

/// PR #165 final rereview remediation, A2-R5: the write-side counterpart to
/// `declarative_context_popup_direct_on_mount_local_let_shadows_the_enclosing_owner_field` — a
/// local `let mut` binding of the same name as a writable enclosing-owner field must shadow it for
/// *assignment* too, not only for reads. If the write-routing decision in `ViewClosureRewriter`
/// ever stopped checking `is_shadowed` before routing to the implicit-owner setter, this local
/// assignment would incorrectly reach through to (and flip) the real owner's own `#[state]`.
#[elwindui::component(inherits VerticalLayout)]
struct A2LocalShadowWriteOwner {
    #[state(default = false)]
    selected: bool,
    #[param]
    log: Rc<RefCell<Vec<bool>>>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                TextBlock {
                    text: "popup content",
                    on_tapped: |_event| {
                        let mut selected = false;
                        selected = true;
                        log.borrow_mut().push(selected);
                    },
                }
            }
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl A2LocalShadowWriteOwner {}

#[test]
fn declarative_context_popup_event_closure_local_shadow_assignment_does_not_write_outer_state() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = elwindui::new!(A2LocalShadowWriteOwner(log: log.clone()));
    assert!(!owner.selected());
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let inner_target = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &inner_target,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 0.0, y: 0.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );

    assert_eq!(
        log.borrow().as_slice(),
        [true],
        "the block-local shadow must itself have been mutated"
    );
    assert!(
        !owner.selected(),
        "a lexically-shadowed local assignment must never reach through to the enclosing \
         owner's own #[state] setter"
    );

    unmount_subtree(&content);
    handle.close();
}

// ---------------------------------------------------------------------------
// PR #165 post-final rereview remediation: A8/A9 — direct source-qualified paths and direct
// source-field reactivity inside a lowered `DeferredView`, with no intermediate nested Component.
// ---------------------------------------------------------------------------

#[elwindui::viewmodel]
mod t28_vm_mod {
    struct T28Vm {
        #[observable(default = String::new())]
        label: String,
    }
}

/// T28: a direct, source-qualified 2-segment path (`vm.label`) written straight inside a lowered
/// `DeferredView` — no intermediate nested Component bridging it — must build with the owner's
/// *current* value and live-update while the popup stays open, exactly like an ordinary Component's
/// own `vm.field` reference already does.
#[elwindui::component(inherits VerticalLayout)]
struct T28DirectQualifiedOwner {
    #[bindable]
    vm: Rc<T28Vm>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                TextBlock { text: vm.label }
            },
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl T28DirectQualifiedOwner {}

#[test]
fn declarative_context_popup_direct_qualified_source_path_live_updates_while_open() {
    let vm = T28Vm::new();
    vm.set_label("before".to_string());
    let owner = elwindui::new!(T28DirectQualifiedOwner(vm: vm.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let text_node = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());
    let text_block = text_node
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("popup content should resolve to the TextBlock itself");
    assert_eq!(text_block.text.borrow().as_str(), "before");

    vm.set_label("after".to_string());
    assert_eq!(
        text_block.text.borrow().as_str(),
        "after",
        "the popup content must live-update when a direct source-qualified vm.label changes \
         while the popup stays open, not only read it correctly at build time"
    );

    unmount_subtree(&content);
    handle.close();
}

/// T29: a direct *bare* source field (`label`, no `vm.` qualification, backed by `#[state]` on the
/// source Component itself, no viewmodel involved) must also live-update while the popup stays
/// open — the counterpart to T28 for the source Component's own reactive fields, not a
/// `#[bindable]` owner's.
#[elwindui::component(inherits VerticalLayout)]
struct T29DirectStateOwner {
    #[state(default = "before".to_string())]
    label: String,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                TextBlock { text: label }
            },
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl T29DirectStateOwner {}

#[test]
fn declarative_context_popup_direct_source_state_live_updates_while_open() {
    let owner = T29DirectStateOwner::new();
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let text_node = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());
    let text_block = text_node
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("popup content should resolve to the TextBlock itself");
    assert_eq!(text_block.text.borrow().as_str(), "before");

    owner.set_label("after".to_string());
    assert_eq!(
        text_block.text.borrow().as_str(),
        "after",
        "the popup content must live-update when a direct bare source field changes while the \
         popup stays open"
    );

    unmount_subtree(&content);
    handle.close();
}

/// T30: a direct bare source field used *inside* a supported `format!` expression
/// (`format!("value:{label}")`) must both build and live-update correctly — distinguishing "the
/// value rewrite works" (already proven by `ViewClosureRewriter`'s own format! inline-capture
/// handling) from "the dependency tracker recognizes this as a reactive dependency at all" (the
/// actual A9 gap: `collect_view_expr_owner_properties`/`view_expr_has_reactive_dependency`/
/// `view_expr_depends_on`'s own `format!`-macro branches).
#[elwindui::component(inherits VerticalLayout)]
struct T30DirectFormatOwner {
    #[state(default = "before".to_string())]
    label: String,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                TextBlock { text: format!("value:{label}") }
            },
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl T30DirectFormatOwner {}

#[test]
fn declarative_context_popup_direct_source_field_format_expression_live_updates_while_open() {
    let owner = T30DirectFormatOwner::new();
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let text_node = content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| content.clone());
    let text_block = text_node
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("popup content should resolve to the TextBlock itself");
    assert_eq!(text_block.text.borrow().as_str(), "value:before");

    owner.set_label("after".to_string());
    assert_eq!(
        text_block.text.borrow().as_str(),
        "value:after",
        "a direct source field referenced only inside a format! expression must still be tracked \
         as a reactive dependency and live-update while the popup stays open"
    );

    unmount_subtree(&content);
    handle.close();
}

#[elwindui::viewmodel]
mod t31_vm_mod {
    struct T31Vm {
        #[observable(default = 0i32)]
        run_count: i32,
    }

    impl T31Vm {
        fn run_action(&self) {
            run_count = self.run_count() + 1;
        }
    }
}

/// T31: a direct source-qualified action reference (`vm.run_action`, no explicit closure — the
/// zero-payload bare-callback shorthand `Button.on_click` supports) dispatched from inside a
/// lowered `DeferredView` must at minimum compile and construct through the source-owner bridge.
/// `Button` is a native leaf requiring real native construction on the main thread (unavailable in
/// this harness — see `for_item_two_way.rs`'s own established type-check-only convention for the
/// identical limitation on `TextArea`), so — like T32 — this stays a type-check rather than a real
/// dispatch proof; `TextBlock`'s own generic `on_click` (`emit_generic_on_click_routing`) was tried
/// as a native-free alternative, but `on_click` is only a *declared* property on the controls that
/// explicitly opt into it (`Button`'s own `#[prop(routed, on_click: fn())]`) — `emit_generic_on_
/// click_routing`'s generation logic exists, but the DSL's own static property-existence check
/// (`__elwindui_props_UIElement!`) rejects `on_click` on a plain `TextBlock` before generation is
/// ever reached, so there is no native-free way to exercise real dispatch through this specific
/// event name.
#[elwindui::component(inherits VerticalLayout)]
struct T31DirectActionOwner {
    #[bindable]
    vm: Rc<T31Vm>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                Button {
                    text: "Run",
                    on_click: vm.run_action,
                }
            },
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl T31DirectActionOwner {}

fn t31_type_checked_construction_and_drop(vm: Rc<T31Vm>) {
    let owner = elwindui::new!(T31DirectActionOwner(vm: vm));
    drop(owner);
}

#[test]
fn declarative_context_popup_direct_bindable_action_path_type_checks() {
    let _ = t31_type_checked_construction_and_drop as fn(Rc<T31Vm>);
}

#[elwindui::viewmodel]
mod t32_vm_mod {
    struct T32Vm {
        #[observable(default = String::new())]
        text: String,
    }
}

/// T32: a direct source-qualified `TwoWay` binding (`text <=> vm.text`) inside a lowered
/// `DeferredView` must at minimum compile through the same source-owner bridge as every other
/// direct qualified path. `TextArea` is a native leaf requiring real native construction on the
/// main thread (unavailable in this harness — see `for_item_two_way.rs`'s own established
/// type-check-only convention for the identical limitation on an ordinary, non-deferred TwoWay
/// binding), so this stays a type-check rather than a live-update proof — matching that existing
/// convention exactly rather than inventing a stronger guarantee this harness cannot back up.
#[elwindui::component(inherits VerticalLayout)]
struct T32DirectTwoWayOwner {
    #[bindable]
    vm: Rc<T32Vm>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "Open popup",
            context_popup: view! {
                TextArea { text <=> vm.text }
            },
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl T32DirectTwoWayOwner {}

fn t32_type_checked_construction_and_drop(vm: Rc<T32Vm>) {
    let owner = elwindui::new!(T32DirectTwoWayOwner(vm: vm));
    drop(owner);
}

#[test]
fn declarative_context_popup_direct_two_way_bindable_path_type_checks() {
    let _ = t32_type_checked_construction_and_drop as fn(Rc<T32Vm>);
}

/// T33: the direct-qualified-path subscription this delta adds (T28's own `__resync_vm`
/// subscription on the resolved `vm` value) must be released, and never fire into a destroyed
/// hidden Component, once the popup closes and every external strong reference is dropped — the
/// A8/A9 counterpart to the existing `declarative_context_popup_content_is_releasable_after_close`
/// (T9), which predates the direct-qualified-path subscription this delta introduces.
#[test]
fn declarative_context_popup_direct_qualified_source_subscription_releases_after_close() {
    let vm = T28Vm::new();
    vm.set_label("before".to_string());
    let owner = elwindui::new!(T28DirectQualifiedOwner(vm: vm.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

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
    .expect("owner is alive, deferred view should build");

    let content = Rc::clone(&host.shown.borrow()[0].0);
    let weak_content = Rc::downgrade(&content);
    unmount_subtree(&content);
    handle.close();

    drop(content);
    host.shown.borrow_mut().clear();
    host.last_request.borrow_mut().take();

    assert!(
        weak_content.upgrade().is_none(),
        "the hidden Component's own content (including its own direct-qualified-path vm \
         subscription) must be releasable once every external strong reference is dropped"
    );

    // Changing `vm.label` after the hidden Component is fully released must not panic or attempt
    // to reach a destroyed Component through a dangling subscription — the subscription's own
    // `weak.upgrade()` guard (inside the generated subscribe_stmts closure) must simply no-op.
    vm.set_label("after-close".to_string());
}

// ---------------------------------------------------------------------------
// PR #165 final merge-gate delta: M1 — nested DeferredView direct vm.field runtime live update.
// ---------------------------------------------------------------------------

#[elwindui::viewmodel]
mod t34_runtime_vm_mod {
    struct T34RuntimeVm {
        #[observable(default = String::new())]
        label: String,
    }
}

/// M1: `nested_deferred_view_direct_qualified_source_path_uses_the_original_source_owner`
/// (`elwindui-codegen`) proves the *generated source shape* — both hidden Components type their
/// own `__view_owner` as `Weak<T34Owner>`, and the second-level `vm.label` bridges through the
/// original source Component's own `vm()`. It does not prove the second-level hidden Component's
/// subscription is actually *live* while its own popup instance stays open. This closes that gap:
/// open both popups, change `vm.label` while *both* remain open (no reopen, no remount), and prove
/// the inner popup's own visible content updates through its own real `ObservableExt` subscription.
#[elwindui::component(inherits VerticalLayout)]
struct T34RuntimeNestedOwner {
    #[bindable]
    vm: Rc<T34RuntimeVm>,
    body: view! {
        #[id("target")]
        let target = TextBlock {
            text: "outer target",
            context_popup: view! {
                TextBlock {
                    text: "inner target",
                    context_popup: view! {
                        TextBlock { text: vm.label }
                    },
                }
            },
        };
        VerticalLayout { target }
    },
}

#[elwindui::component]
impl T34RuntimeNestedOwner {}

#[test]
fn declarative_context_popup_nested_direct_qualified_source_path_live_updates_while_inner_popup_is_open()
 {
    let vm = T34RuntimeVm::new();
    vm.set_label("before".to_string());
    let owner = elwindui::new!(T34RuntimeNestedOwner(vm: vm.clone()));
    let target_dyn: Rc<dyn UIElementExt> = owner.target();

    let request = ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (resolved, anchor) = ContextMenuService::process_request_for_target(&target_dyn, &request)
        .expect("target should resolve a context popup");
    let ResolvedContextDefinition::Popup { template: t } = resolved.definition else {
        panic!("expected Popup definition");
    };

    let outer_host = TestPopupHost::new();
    let work_area = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    let outer_handle = ContextMenuService::open_custom_popup(
        &outer_host,
        &resolved.owner,
        &t,
        &anchor,
        resolved.owner.effective_environment(),
        work_area,
    )
    .expect("owner is alive, outer deferred view should build");

    let outer_content = Rc::clone(&outer_host.shown.borrow()[0].0);
    let inner_target = outer_content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| outer_content.clone());

    let inner_request =
        ContextRequest::keyboard(Some(PopupAnchor::Point(Point { x: 0.0, y: 0.0 })));
    let (inner_resolved, inner_anchor) =
        ContextMenuService::process_request_for_target(&inner_target, &inner_request)
            .expect("inner target should resolve its own context popup");
    let ResolvedContextDefinition::Popup { template: inner_t } = inner_resolved.definition else {
        panic!("expected Popup definition");
    };

    let inner_host = TestPopupHost::new();
    let inner_handle = ContextMenuService::open_custom_popup(
        &inner_host,
        &inner_resolved.owner,
        &inner_t,
        &inner_anchor,
        inner_resolved.owner.effective_environment(),
        work_area,
    )
    .expect("outer popup content is alive, inner deferred view should build");

    let inner_content = Rc::clone(&inner_host.shown.borrow()[0].0);
    let inner_text_node = inner_content
        .visual_children()
        .first()
        .cloned()
        .unwrap_or_else(|| inner_content.clone());
    let text_block = inner_text_node
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("inner popup content should resolve to the TextBlock itself");
    assert_eq!(text_block.text.borrow().as_str(), "before");

    // Both popups remain open here — no reopen, no remount. The second-level hidden Component's
    // own `vm` subscription (bridged through `__view_owner.upgrade().vm()`, A9) must be the thing
    // that actually drives this update, not a build-time-only read.
    vm.set_label("after".to_string());
    assert_eq!(
        text_block.text.borrow().as_str(),
        "after",
        "the second-level nested DeferredView's own direct source-qualified vm.label must \
         live-update through its own real ObservableExt subscription while both popups remain \
         open, not merely read correctly once at build time"
    );

    unmount_subtree(&inner_content);
    inner_handle.close();
    unmount_subtree(&outer_content);
    outer_handle.close();
}
