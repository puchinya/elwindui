//! Interactive visual verification for the reusable controls in `elwindui-custom-controls`.
//!
//! The window intentionally uses the public programmatic API: the custom controls live in a
//! separate crate, while their page content is ordinary Core UI. Clicking tab headers exercises
//! selection and the authored header template; dragging the divider exercises the routed pointer
//! gesture and completion callback.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::graphics::{Brush, Color, FontWeight};
use elwindui::core::layout::{GridLength, Orientation};
use elwindui::core::ui::{
    ContentControlExt as _, Grid, GridExt as _, LayoutExt as _, ShapeExt as _, TextBlock,
    TextBlockExt as _, TextStyleOwner as _, UIElementExt as _, VerticalLayout,
    VerticalLayoutExt as _, WindowExt,
};
use elwindui_custom_controls::{
    CloseButtonPresentation, CustomSplitter, CustomSplitterExt as _, CustomTabView,
    CustomTabViewExt as _, CustomTabViewItem,
};
use std::rc::Rc;

const BACKGROUND: Color = Color::rgb(30, 34, 40);
const PANEL_BACKGROUND: Color = Color::rgb(42, 48, 56);
const PAGE_BACKGROUND: Color = Color::rgb(48, 55, 64);
const FOREGROUND: Color = Color::rgb(238, 242, 247);
const MUTED_FOREGROUND: Color = Color::rgb(171, 183, 196);
const ACCENT: Color = Color::rgb(70, 156, 232);

fn brush(color: Color) -> Brush {
    Brush::Solid(color)
}

fn text(value: &str, size: f32, color: Color) -> Rc<TextBlock> {
    let node = TextBlock::new();
    node.set_text(value);
    node.set_font_size(size);
    node.set_foreground(Some(brush(color)));
    node
}

fn heading(value: &str) -> Rc<TextBlock> {
    let node = text(value, 20.0, FOREGROUND);
    node.set_font_weight(FontWeight::BOLD);
    node
}

fn page(title: &str, description: &str, details: &[&str]) -> Rc<VerticalLayout> {
    let layout = VerticalLayout::new();
    layout.set_margin(18.0);
    layout.set_spacing(10.0);
    layout.set_background(Some(brush(PAGE_BACKGROUND)));

    layout
        .children()
        .add(heading(title) as Rc<dyn elwindui::core::ui::UIElementExt>);
    layout
        .children()
        .add(text(description, 14.0, MUTED_FOREGROUND) as Rc<dyn elwindui::core::ui::UIElementExt>);

    let accent = elwindui::core::ui::Rectangle::new();
    accent.set_width(72.0);
    accent.set_height(3.0);
    accent.set_fill(Some(brush(ACCENT)));
    layout
        .children()
        .add(accent as Rc<dyn elwindui::core::ui::UIElementExt>);

    for detail in details {
        layout
            .children()
            .add(text(detail, 14.0, FOREGROUND) as Rc<dyn elwindui::core::ui::UIElementExt>);
    }

    layout
}

#[elwindui::component(inherits Window)]
struct CustomControlsDemoWindow {
    root: Rc<VerticalLayout>,

    body: view! {
        title: "elwindui Custom Controls Demo"
        width: 980.0
        height: 620.0
        content: root
    },
}

#[elwindui::component]
impl CustomControlsDemoWindow {}

#[elwindui::main]
fn main() {
    let status = text(
        "Selected tab: Overview · click a header, close affordance, or divider to exercise callbacks",
        13.0,
        MUTED_FOREGROUND,
    );

    let overview = CustomTabViewItem::new_item();
    overview.set_header("Overview".to_string());
    overview.set_closable(false);
    overview.set_content(page(
        "CustomTabView",
        "A reusable tab host composed from ordinary template visuals.",
        &[
            "• Headers are authored by the component template.",
            "• Page content keeps CustomTabViewItem as its logical owner.",
            "• Selection changes only the visual arrangement.",
        ],
    ));

    let inspector = CustomTabViewItem::new_item();
    inspector.set_header("Inspector".to_string());
    inspector.set_content(page(
        "CustomTabViewItem",
        "ContentControl content is presented without reparenting the logical page.",
        &[
            "• This tab keeps the default close affordance.",
            "• Click × to emit an advisory close request.",
            "• The host decides whether an item is removed.",
        ],
    ));

    let activity = CustomTabViewItem::new_item();
    activity.set_header("Activity".to_string());
    activity.set_content(page(
        "CustomSplitter",
        "The divider beside this tab view reports logical-axis drag deltas.",
        &[
            "• Press and drag the 6-pixel divider.",
            "• Pointer capture keeps the gesture coherent.",
            "• Completion updates the status line below.",
        ],
    ));

    let tabs = CustomTabView::new_view();
    tabs.set_close_button_presentation(CloseButtonPresentation::Always);
    tabs.set_children(vec![overview, inspector, activity]);

    let splitter = CustomSplitter::new_splitter();
    // Horizontal means movement along X and a vertical six-pixel divider.
    splitter.set_orientation(Orientation::Horizontal);

    let details = VerticalLayout::new();
    details.set_margin(18.0);
    details.set_spacing(10.0);
    details.set_background(Some(brush(PANEL_BACKGROUND)));
    details
        .children()
        .add(heading("Interaction surface") as Rc<dyn elwindui::core::ui::UIElementExt>);
    details.children().add(text(
        "Ordinary Core layout content beside the reusable controls.",
        14.0,
        MUTED_FOREGROUND,
    ) as Rc<dyn elwindui::core::ui::UIElementExt>);
    details.children().add(text(
        "Click a tab header to update selected_index.",
        14.0,
        FOREGROUND,
    ) as Rc<dyn elwindui::core::ui::UIElementExt>);
    details.children().add(text(
        "Drag a tab header or divider to test routed input.",
        14.0,
        FOREGROUND,
    ) as Rc<dyn elwindui::core::ui::UIElementExt>);
    details.children().add(text(
        "Close requests are advisory; the host controls removal.",
        14.0,
        FOREGROUND,
    ) as Rc<dyn elwindui::core::ui::UIElementExt>);

    let main_grid = Grid::new();
    main_grid.set_height(450.0);
    main_grid.set_rows(vec![GridLength::Star(1.0)]);
    main_grid.set_columns(vec![
        GridLength::Star(1.0),
        GridLength::Fixed(6.0),
        GridLength::Star(1.0),
    ]);
    tabs.set_attached("Grid", "column", 0i32);
    splitter.set_attached("Grid", "column", 1i32);
    details.set_attached("Grid", "column", 2i32);
    main_grid
        .children()
        .add(tabs.clone() as Rc<dyn elwindui::core::ui::UIElementExt>);
    main_grid
        .children()
        .add(splitter.clone() as Rc<dyn elwindui::core::ui::UIElementExt>);
    main_grid
        .children()
        .add(details as Rc<dyn elwindui::core::ui::UIElementExt>);

    let header = VerticalLayout::new();
    header.set_spacing(4.0);
    header
        .children()
        .add(text("elwindui Custom Controls", 26.0, FOREGROUND)
            as Rc<dyn elwindui::core::ui::UIElementExt>);
    header.children().add(text(
        "Template-backed CustomTabView, ContentControl page ownership, and CustomSplitter input.",
        14.0,
        MUTED_FOREGROUND,
    ) as Rc<dyn elwindui::core::ui::UIElementExt>);

    let root = VerticalLayout::new();
    root.set_margin(18.0);
    root.set_spacing(12.0);
    root.set_background(Some(brush(BACKGROUND)));
    root.children()
        .add(header as Rc<dyn elwindui::core::ui::UIElementExt>);
    root.children()
        .add(main_grid as Rc<dyn elwindui::core::ui::UIElementExt>);
    root.children()
        .add(status.clone() as Rc<dyn elwindui::core::ui::UIElementExt>);

    let status_for_selection = status.clone();
    tabs.set_on_selected_index_changed(move |index| {
        let name = match index {
            0 => "Overview",
            1 => "Inspector",
            2 => "Activity",
            _ => "Unknown",
        };
        status_for_selection.set_text(&format!(
            "Selected tab: {name} · selected_index callback received {index}"
        ));
    });

    let status_for_close = status.clone();
    tabs.set_on_close_request(Box::new(move |index| {
        status_for_close.set_text(&format!(
            "Close requested for tab {index} · item remains until the host removes it"
        ));
    }));

    let status_for_tab_drag = status.clone();
    tabs.set_on_tab_drag_completed(Box::new(move |event| {
        status_for_tab_drag.set_text(&format!(
            "Tab drag completed: index={} cumulative movement=({:.1}, {:.1}) canceled={}",
            event.index, event.position.x, event.position.y, event.canceled
        ));
    }));

    let status_for_splitter = status.clone();
    splitter.set_on_drag_completed(Box::new(move |event| {
        status_for_splitter.set_text(&format!(
            "Splitter drag completed: cumulative delta={:.1}px canceled={}",
            event.cumulative_delta, event.canceled
        ));
    }));

    let window = CustomControlsDemoWindow::new(root);
    window.show();
}
