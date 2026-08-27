//! Interactive visual verification for the reusable controls in `elwindui-custom-controls`.
//!
//! The complete demo surface—including the reusable custom controls and their content—is authored
//! with `view!` so the example exercises the same declarative composition path as application code.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::{cell::Cell, rc::Rc};

extern crate elwindui as elwindui_facade;

// `view!` currently emits the facade paths for external element names. Extend that facade only
// inside this example so the custom controls can participate in the declarative tree without
// changing the dependency direction of the reusable-controls crate.
mod elwindui {
    pub use crate::elwindui_facade::{class, component, main};

    pub mod core {
        pub use crate::elwindui_facade::core::*;
    }

    pub mod ui {
        pub use crate::elwindui_facade::ui::*;
        pub use ::elwindui_custom_controls::{CustomSplitter, CustomTabView, CustomTabViewItem};
    }
}

use elwindui::core::graphics::FontWeight;
use elwindui::core::layout::{GridLength, Orientation};
use elwindui::core::ui::WindowExt;
use elwindui_custom_controls::{
    CloseButtonPresentation, CustomSplitterExt as _, CustomTabViewExt as _,
};

#[elwindui::component(inherits VerticalLayout)]
struct OverviewPage {
    body: view! {
        margin: 18.0
        spacing: 10.0
        background: "#303740"
        TextBlock {
            text: "CustomTabView"
            font_size: 20.0
            font_weight: FontWeight::BOLD
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "A reusable tab host composed from ordinary template visuals."
            font_size: 14.0
            foreground: "#abb7c4"
        }
        Rectangle {
            width: 72.0
            height: 3.0
            fill: "#469ce8"
        }
        TextBlock {
            text: "• Headers are authored by the component template."
            font_size: 14.0
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "• Page content keeps CustomTabViewItem as its logical owner."
            font_size: 14.0
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "• Selection changes only the visual arrangement."
            font_size: 14.0
            foreground: "#eef2f7"
        }
    },
}

#[elwindui::component]
impl OverviewPage {}

#[elwindui::component(inherits VerticalLayout)]
struct InspectorPage {
    body: view! {
        margin: 18.0
        spacing: 10.0
        background: "#303740"
        TextBlock {
            text: "CustomTabViewItem"
            font_size: 20.0
            font_weight: FontWeight::BOLD
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "ContentControl content is presented without reparenting the logical page."
            font_size: 14.0
            foreground: "#abb7c4"
        }
        TextBlock {
            text: "• This tab keeps the default close affordance."
            font_size: 14.0
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "• Click × to emit an advisory close request."
            font_size: 14.0
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "• The host decides whether an item is removed."
            font_size: 14.0
            foreground: "#eef2f7"
        }
    },
}

#[elwindui::component]
impl InspectorPage {}

#[elwindui::component(inherits VerticalLayout)]
struct ActivityPage {
    body: view! {
        margin: 18.0
        spacing: 10.0
        background: "#303740"
        TextBlock {
            text: "CustomSplitter"
            font_size: 20.0
            font_weight: FontWeight::BOLD
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "The divider beside this tab view reports logical-axis drag deltas consumed by the demo."
            font_size: 14.0
            foreground: "#abb7c4"
        }
        TextBlock {
            text: "• Press and drag the 6-pixel divider to resize the panes."
            font_size: 14.0
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "• Pointer capture keeps the gesture coherent."
            font_size: 14.0
            foreground: "#eef2f7"
        }
        TextBlock {
            text: "• Completion updates the status line below."
            font_size: 14.0
            foreground: "#eef2f7"
        }
    },
}

#[elwindui::component]
impl ActivityPage {}

#[elwindui::component(inherits VerticalLayout)]
struct CustomControlsDemoSurface {
    body: view! {
        on_mount {
            let overview = this.overview();
            overview.set_header("Overview".to_string());
            overview.set_closable(false);
            overview.set_content(this.overview_page().into_node());

            let inspector = this.inspector();
            inspector.set_header("Inspector".to_string());
            inspector.set_content(this.inspector_page().into_node());

            let activity = this.activity();
            activity.set_header("Activity".to_string());
            activity.set_content(this.activity_page().into_node());

            let tabs = this.tabs();
            tabs.set_close_button_presentation(CloseButtonPresentation::Always);
            tabs.set_children(vec![overview, inspector, activity]);
            tabs.set_attached("Grid", "column", 0i32);

            let splitter = this.splitter();
            splitter.set_orientation(Orientation::Horizontal);
            splitter.set_attached("Grid", "column", 1i32);

            let content_grid = this.content_grid();
            let left_column_width = Rc::new(Cell::new(460.0_f32));
            let left_column_width_for_drag = left_column_width.clone();
            let content_grid_for_drag = content_grid.clone();
            splitter.set_on_drag_delta(Box::new(move |event| {
                let next_width = (left_column_width_for_drag.get() + event.delta).clamp(180.0, 700.0);
                left_column_width_for_drag.set(next_width);
                content_grid_for_drag.set_columns(vec![
                    GridLength::Fixed(next_width),
                    GridLength::Fixed(6.0),
                    GridLength::Star(1.0),
                ]);
            }));

            let status_for_selection = this.status();
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

            let status_for_close = this.status();
            tabs.set_on_close_request(Box::new(move |index| {
                status_for_close.set_text(&format!(
                    "Close requested for tab {index} · item remains until the host removes it"
                ));
            }));

            let status_for_tab_drag = this.status();
            tabs.set_on_tab_drag_completed(Box::new(move |event| {
                status_for_tab_drag.set_text(&format!(
                    "Tab drag completed: index={} cumulative movement=({:.1}, {:.1}) canceled={}",
                    event.index, event.position.x, event.position.y, event.canceled
                ));
            }));

            let status_for_splitter = this.status();
            splitter.set_on_drag_completed(Box::new(move |event| {
                status_for_splitter.set_text(&format!(
                    "Splitter drag completed: cumulative delta={:.1}px canceled={} · panes resized",
                    event.cumulative_delta, event.canceled
                ));
            }));
        }

        #[id("status")]
        let status = TextBlock {
            text: "Selected tab: Overview · click a header, close affordance, or divider to exercise callbacks"
            font_size: 13.0
            foreground: "#abb7c4"
        };

        #[id("overview_page")]
        let overview_page = OverviewPage {};
        #[id("inspector_page")]
        let inspector_page = InspectorPage {};
        #[id("activity_page")]
        let activity_page = ActivityPage {};

        #[id("overview")]
        let overview = CustomTabViewItem {};
        #[id("inspector")]
        let inspector = CustomTabViewItem {};
        #[id("activity")]
        let activity = CustomTabViewItem {};

        #[id("tabs")]
        let tabs = CustomTabView {};

        #[id("splitter")]
        let splitter = CustomSplitter {};

        #[id("content_grid")]
        let content_grid = Grid {
            height: 450.0
            rows: [GridLength::Star(1.0)]
            columns: [
                GridLength::Fixed(460.0),
                GridLength::Fixed(6.0),
                GridLength::Star(1.0),
            ]
            tabs
            splitter
            VerticalLayout {
                Grid::column: 2
                margin: 18.0
                spacing: 10.0
                background: "#2a3038"
                TextBlock {
                    text: "Interaction surface"
                    font_size: 20.0
                    font_weight: FontWeight::BOLD
                    foreground: "#eef2f7"
                }
                TextBlock {
                    text: "Ordinary Core layout content beside the reusable controls."
                    font_size: 14.0
                    foreground: "#abb7c4"
                }
                TextBlock {
                    text: "Click a tab header to update selected_index."
                    font_size: 14.0
                    foreground: "#eef2f7"
                }
                TextBlock {
                    text: "Drag a tab header or divider to resize the panes and test routed input."
                    font_size: 14.0
                    foreground: "#eef2f7"
                }
                TextBlock {
                    text: "Close requests are advisory; the host controls removal."
                    font_size: 14.0
                    foreground: "#eef2f7"
                }
            }
        };

        margin: 18.0
        spacing: 12.0
        background: "#1e2228"

        VerticalLayout {
            spacing: 4.0
            TextBlock {
                text: "elwindui Custom Controls"
                font_size: 26.0
                font_weight: FontWeight::BOLD
                foreground: "#eef2f7"
            }
            TextBlock {
                text: "Template-backed CustomTabView, ContentControl page ownership, and CustomSplitter input."
                font_size: 14.0
                foreground: "#abb7c4"
            }
        }
        content_grid
        status
    },
}

#[elwindui::component]
impl CustomControlsDemoSurface {}

#[elwindui::component(inherits Window)]
struct CustomControlsDemoWindow {
    body: view! {
        title: "elwindui Custom Controls Demo"
        width: 980.0
        height: 620.0
        content: VerticalLayout {
            CustomControlsDemoSurface {}
        }
    },
}

#[elwindui::component]
impl CustomControlsDemoWindow {}

#[elwindui::main]
fn main() {
    let window = CustomControlsDemoWindow::new();
    window.show();
}
