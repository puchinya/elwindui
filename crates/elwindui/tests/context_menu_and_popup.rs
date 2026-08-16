//! Issue #152: Integration and type-check tests for Context Menu, Custom Context Menu presentation, and rich Context Popup.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::{Point, Rect, Size};
use elwindui::core::environment::EnvironmentContext;
use elwindui::core::ui::popup::{
    ContextMenuPresentation, ContextMenuService, ContextRequest, PopupAnchor,
    PopupContentTemplate, PopupHost, PopupSurfaceHandle, ResolvedContextDefinition,
};
use elwindui::core::ui::{LayoutExt, UIElementExt};
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
    closed: Rc<Cell<bool>>,
}

impl TestPopupHost {
    fn new() -> Self {
        Self {
            shown: RefCell::new(Vec::new()),
            closed: Rc::new(Cell::new(false)),
        }
    }
}

impl PopupHost for TestPopupHost {
    fn show_popup(
        &self,
        content: Rc<dyn UIElementExt>,
        position: Point,
        size: Size,
    ) -> Rc<dyn PopupSurfaceHandle> {
        self.shown
            .borrow_mut()
            .push((Rc::clone(&content), position, size));
        Rc::new(TestPopupHandle {
            closed: Rc::clone(&self.closed),
        })
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
    let handle = ContextMenuService::open_custom_menu(&host, &*menu, &anchor, work_area);

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
    let template = PopupContentTemplate::new(|_ctx| {
        let layout = elwindui::core::ui::VerticalLayout::new();
        let title = elwindui::core::ui::TextBlock::new();
        layout.children().add(Rc::clone(&title) as Rc<dyn UIElementExt>);
        layout as Rc<dyn UIElementExt>
    });

    target.set_context_popup(Some(template.clone()));

    let target_dyn: Rc<dyn UIElementExt> = target;
    let request = ContextRequest::keyboard();
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
                &t,
                &anchor,
                EnvironmentContext::default(),
                work_area,
            );

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
