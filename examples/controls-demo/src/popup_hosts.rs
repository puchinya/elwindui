//! The "Popup Hosts" tab: fixtures for context requests across independent visible hosts and for
//! retiring nested content (Issue #157, `tests/e2e/context-menu-popup-surface.md` CMP-29..CMP-34).
//!
//! - Two side-by-side ScrollViews (each content lives in its own nested host) and a target in the
//!   tab's own host, each with a small traced context popup, so a single secondary click can be
//!   checked to move the popup from one host to another.
//! - A TabView whose closable tabs each carry a popup target, so removing a tab with an open popup
//!   can be checked to close that popup and unmount that tab's content exactly once.
//! - A ScrollView whose content a button replaces A -> B -> C -> ..., so the previous content
//!   (and its open popup) can be checked to retire before the next is attached.
//!
//! Every popup build/unmount and content unmount is reported through [`crate::rich_popup::trace`].

use crate::rich_popup::trace;
use elwindui::core::ui::{
    ButtonExt, HorizontalLayoutExt, LayoutExt, ListExt, ScrollViewExt, TabViewItemExt,
    TextBlockExt, UIElementExt, VerticalLayoutExt, ViewBuildContext, ViewFactory,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

thread_local! {
    static INSTANCE_COUNTERS: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
}

/// Next instance number for `name` (1-based), so each build / content is traced distinctly.
fn next_instance(name: &str) -> u32 {
    INSTANCE_COUNTERS.with(|counters| {
        let mut counters = counters.borrow_mut();
        let counter = counters.entry(name.to_owned()).or_insert(0);
        *counter += 1;
        *counter
    })
}

fn text(value: &str) -> Rc<elwindui::core::ui::TextBlock> {
    let block = elwindui::core::ui::TextBlock::new();
    block.set_text(value);
    block.set_margin(4.0);
    block
}

fn vertical(children: Vec<Rc<dyn UIElementExt>>) -> Rc<elwindui::core::ui::VerticalLayout> {
    let layout = elwindui::core::ui::VerticalLayout::new();
    layout.set_spacing(4.0);
    for child in children {
        LayoutExt::children(&*layout).add(child);
    }
    layout
}

/// A small popup whose builds and unmounts are traced as `host_popup_build` /
/// `host_popup_unmount` with detail `<name>#<n>`.
fn host_popup(name: &'static str) -> ViewFactory {
    ViewFactory::new(move |_ctx: ViewBuildContext| {
        let id = format!("{name}#{}", next_instance(name));
        trace("host_popup_build", &id);
        let label = text(&format!("Popup {id}"));
        let layout = vertical(vec![label as Rc<dyn UIElementExt>]);
        layout.set_margin(10.0);
        layout.set_background(Some(
            elwindui::core::graphics::Color::rgb(255, 244, 214).into(),
        ));
        let id_for_unmount = id.clone();
        layout.add_unmount_hook(Box::new(move || {
            trace("host_popup_unmount", &id_for_unmount);
        }));
        Some(layout as Rc<dyn UIElementExt>)
    })
}

/// A target text with a traced popup.
fn popup_target(label: &str, name: &'static str) -> Rc<dyn UIElementExt> {
    let target = text(label);
    target.set_margin(8.0);
    target.set_context_popup(Some(host_popup(name)));
    target as Rc<dyn UIElementExt>
}

/// Content whose unmount is traced as `content_unmount <id>`.
fn traced_content(id: String, children: Vec<Rc<dyn UIElementExt>>) -> Rc<dyn UIElementExt> {
    let content = vertical(children);
    content.add_unmount_hook(Box::new(move || trace("content_unmount", &id)));
    content as Rc<dyn UIElementExt>
}

fn scroll_host(content: Rc<dyn UIElementExt>, width: f32, height: f32) -> Rc<dyn UIElementExt> {
    let scroll = elwindui::ui::ScrollView::new();
    scroll.set_width(width);
    scroll.set_height(height);
    scroll.set_content(content);
    scroll as Rc<dyn UIElementExt>
}

fn cross_host_section() -> Vec<Rc<dyn UIElementExt>> {
    let host_a = scroll_host(
        vertical(vec![
            text("Host A (ScrollView)") as Rc<dyn UIElementExt>,
            popup_target("【Host A target】", "A"),
        ]) as Rc<dyn UIElementExt>,
        280.0,
        110.0,
    );
    let host_b = scroll_host(
        vertical(vec![
            text("Host B (ScrollView)") as Rc<dyn UIElementExt>,
            popup_target("【Host B target】", "B"),
            text("[Host B - no menu here]") as Rc<dyn UIElementExt>,
        ]) as Rc<dyn UIElementExt>,
        280.0,
        110.0,
    );
    let row = elwindui::core::ui::HorizontalLayout::new();
    row.set_spacing(16.0);
    LayoutExt::children(&*row).add(host_a);
    LayoutExt::children(&*row).add(host_b);
    vec![
        text("5. Cross-host replacement (each ScrollView content is its own host):")
            as Rc<dyn UIElementExt>,
        row as Rc<dyn UIElementExt>,
        popup_target("【Root target (tab host)】", "R"),
    ]
}

fn tab_removal_section() -> Vec<Rc<dyn UIElementExt>> {
    let tab_view = elwindui::ui::TabView::new();
    tab_view.set_height(140.0);
    for name in ["L1", "L2", "L3", "L4"] {
        let item = elwindui::ui::TabViewItem::new();
        item.set_header(&format!("Tab {name}"));
        item.set_closable(true);
        item.set_content(traced_content(
            name.to_owned(),
            vec![popup_target(&format!("【Tab {name} target】"), name)],
        ));
        tab_view.add(item as Rc<dyn TabViewItemExt>);
    }
    let weak: Weak<elwindui::ui::TabView> = Rc::downgrade(&tab_view);
    tab_view.set_on_close(Box::new(move |index| {
        let tab_view: Option<Rc<elwindui::ui::TabView>> = weak.upgrade();
        if let Some(tab_view) = tab_view {
            trace("tab_close_request", &index.to_string());
            tab_view.remove_at(index);
        }
    }));
    let remove_first = elwindui::ui::Button::new();
    remove_first.set_text("Remove first tab");
    let weak: Weak<elwindui::ui::TabView> = Rc::downgrade(&tab_view);
    remove_first.set_on_click(Box::new(move || {
        let tab_view: Option<Rc<elwindui::ui::TabView>> = weak.upgrade();
        if let Some(tab_view) = tab_view {
            if tab_view.len() > 0 {
                trace("tab_remove_first", "");
                tab_view.remove_at(0);
            }
        }
    }));
    vec![
        text("6. Tab removal (close a tab or press the button while its popup is open):")
            as Rc<dyn UIElementExt>,
        remove_first as Rc<dyn UIElementExt>,
        tab_view as Rc<dyn UIElementExt>,
    ]
}

fn replacement_content(cycle: &Cell<usize>) -> Rc<dyn UIElementExt> {
    const NAMES: [&str; 3] = ["SA", "SB", "SC"];
    let name = NAMES[cycle.get() % NAMES.len()];
    cycle.set(cycle.get() + 1);
    let id = format!("{name}#{}", next_instance(name));
    let button = elwindui::ui::Button::new();
    button.set_text(&format!("Content {id} button"));
    let click_id = id.clone();
    button.set_on_click(Box::new(move || trace("content_click", &click_id)));
    traced_content(
        id.clone(),
        vec![
            text(&format!("Content {id}")) as Rc<dyn UIElementExt>,
            popup_target(&format!("【Content {name} target】"), name),
            button as Rc<dyn UIElementExt>,
        ],
    )
}

fn content_replacement_section() -> Vec<Rc<dyn UIElementExt>> {
    let cycle = Rc::new(Cell::new(0usize));
    let scroll = elwindui::ui::ScrollView::new();
    scroll.set_width(420.0);
    scroll.set_height(130.0);
    scroll.set_content(replacement_content(&cycle));
    let replace = elwindui::ui::Button::new();
    replace.set_text("Replace ScrollView content");
    let weak: Weak<elwindui::ui::ScrollView> = Rc::downgrade(&scroll);
    replace.set_on_click(Box::new(move || {
        let scroll: Option<Rc<elwindui::ui::ScrollView>> = weak.upgrade();
        if let Some(scroll) = scroll {
            trace("content_replace", "");
            scroll.set_content(replacement_content(&cycle));
        }
    }));
    vec![
        text("7. Content replacement (A -> B -> C ...):") as Rc<dyn UIElementExt>,
        replace as Rc<dyn UIElementExt>,
        scroll as Rc<dyn UIElementExt>,
    ]
}

/// Fills `layout` (the tab's `PopupHostsPanel`) with the three fixture sections.
pub(crate) fn populate(layout: &dyn LayoutExt) {
    for child in cross_host_section()
        .into_iter()
        .chain(tab_removal_section())
        .chain(content_replacement_section())
    {
        layout.children().add(child);
    }
}
