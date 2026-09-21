//! Interactive visual verification for the reusable controls in `elwindui-custom-controls`.
//!
//! The complete demo surface—including the reusable custom controls and their content—is authored
//! with `view!` so the example exercises the same declarative composition path as application code.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::graphics::FontWeight;
use elwindui::core::input::{MouseButton, PointerEventArgs, TappedEventArgs};
use elwindui::core::layout::GridLength;
use elwindui::core::ui::{TextBlock, TextBlockExt, UIElementExt, WindowExt};
use elwindui_custom_controls::{CloseButtonPresentation, GridResizeBehavior, GridResizeDirection};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct PointerProbeState {
    pressed: u32,
    moved: u32,
    released: u32,
    canceled: u32,
    tapped: u32,
    double_tapped: u32,
    right_tapped: u32,
    last_root_position: Option<elwindui::core::base::Point>,
    last_screen_position: Option<elwindui::core::base::Point>,
    last_button: Option<MouseButton>,
    last_canceled_button: Option<Option<MouseButton>>,
}

#[derive(Clone, Copy)]
enum ProbePointerEvent {
    Pressed,
    Moved,
    Released,
    Canceled,
}

impl PointerProbeState {
    fn record_pointer(&mut self, event: ProbePointerEvent, args: &PointerEventArgs) {
        match event {
            ProbePointerEvent::Pressed => self.pressed += 1,
            ProbePointerEvent::Moved => self.moved += 1,
            ProbePointerEvent::Released => self.released += 1,
            ProbePointerEvent::Canceled => {
                self.canceled += 1;
                self.last_canceled_button = Some(args.button);
            }
        }
        self.last_root_position = Some(args.position);
        self.last_screen_position = args.screen_position;
        self.last_button = args.button;
    }

    fn record_tapped(&mut self, event: &'static str) {
        match event {
            "on_tapped" => self.tapped += 1,
            "on_double_tapped" => self.double_tapped += 1,
            "on_right_tapped" => self.right_tapped += 1,
            _ => {}
        }
    }
}

fn format_point(point: Option<elwindui::core::base::Point>) -> String {
    point
        .map(|point| format!("({:.1}, {:.1})", point.x, point.y))
        .unwrap_or_else(|| "None".to_string())
}

fn format_probe_status(label: &str, state: &PointerProbeState) -> String {
    let canceled_button = state
        .last_canceled_button
        .map(|button| format!("{:?}", button))
        .unwrap_or_else(|| "n/a".to_string());
    format!(
        "{label}\npressed={} moved={} released={}\ncanceled={} (last canceled button={canceled_button})\ntapped={} double_tapped={} right_tapped={}\nlast root={} screen={} button={:?}",
        state.pressed,
        state.moved,
        state.released,
        state.canceled,
        state.tapped,
        state.double_tapped,
        state.right_tapped,
        format_point(state.last_root_position),
        format_point(state.last_screen_position),
        state.last_button,
    )
}

fn refresh_probe_status(label: &str, state: &PointerProbeState, status: &TextBlock) {
    status.set_text(&format_probe_status(label, state));
}

fn register_probe_pointer_handler<T: UIElementExt>(
    probe: &Rc<T>,
    state: Rc<RefCell<PointerProbeState>>,
    status: Rc<TextBlock>,
    label: &'static str,
    event_name: &'static str,
    event: ProbePointerEvent,
) {
    probe.register_routed_handler(
        event_name,
        Box::new(move |args: &PointerEventArgs, _| {
            let mut state = state.borrow_mut();
            state.record_pointer(event, args);
            refresh_probe_status(label, &state, &status);
        }),
    );
}

fn register_probe_tapped_handler<T: UIElementExt>(
    probe: &Rc<T>,
    state: Rc<RefCell<PointerProbeState>>,
    status: Rc<TextBlock>,
    label: &'static str,
    event_name: &'static str,
) {
    probe.register_routed_handler(
        event_name,
        Box::new(move |_: &TappedEventArgs, _| {
            let mut state = state.borrow_mut();
            state.record_tapped(event_name);
            refresh_probe_status(label, &state, &status);
        }),
    );
}

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
            text: "CustomGridSplitter"
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
            let tabs = this.tabs();
            tabs.set_close_button_presentation(CloseButtonPresentation::Always);
            tabs.set_attached("Grid", "column", 0i32);

            let splitter = this.splitter();
            splitter.set_resize_direction(GridResizeDirection::Columns);
            splitter.set_resize_behavior(GridResizeBehavior::PreviousAndNext);
            splitter.set_attached("Grid", "column", 1i32);

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
            splitter.set_on_resize_completed(Box::new(move |event| {
                status_for_splitter.set_text(&format!(
                    "Grid resize completed: cumulative delta={:.1}px canceled={} · panes resized",
                    event.cumulative_delta, event.canceled
                ));
            }));

            let probe_a = this.probe_a();
            let probe_a_state = Rc::new(RefCell::new(PointerProbeState::default()));
            let probe_a_status = this.probe_a_status();
            register_probe_pointer_handler(
                &probe_a,
                probe_a_state.clone(),
                probe_a_status.clone(),
                "Probe A",
                "on_pointer_pressed",
                ProbePointerEvent::Pressed,
            );
            register_probe_pointer_handler(
                &probe_a,
                probe_a_state.clone(),
                probe_a_status.clone(),
                "Probe A",
                "on_pointer_moved",
                ProbePointerEvent::Moved,
            );
            register_probe_pointer_handler(
                &probe_a,
                probe_a_state.clone(),
                probe_a_status.clone(),
                "Probe A",
                "on_pointer_released",
                ProbePointerEvent::Released,
            );
            register_probe_pointer_handler(
                &probe_a,
                probe_a_state.clone(),
                probe_a_status.clone(),
                "Probe A",
                "on_pointer_canceled",
                ProbePointerEvent::Canceled,
            );
            register_probe_tapped_handler(
                &probe_a,
                probe_a_state.clone(),
                probe_a_status.clone(),
                "Probe A",
                "on_tapped",
            );
            register_probe_tapped_handler(
                &probe_a,
                probe_a_state.clone(),
                probe_a_status.clone(),
                "Probe A",
                "on_double_tapped",
            );
            register_probe_tapped_handler(
                &probe_a,
                probe_a_state,
                probe_a_status,
                "Probe A",
                "on_right_tapped",
            );

            let probe_b = this.probe_b();
            let probe_b_state = Rc::new(RefCell::new(PointerProbeState::default()));
            let probe_b_status = this.probe_b_status();
            register_probe_pointer_handler(
                &probe_b,
                probe_b_state.clone(),
                probe_b_status.clone(),
                "Probe B",
                "on_pointer_pressed",
                ProbePointerEvent::Pressed,
            );
            register_probe_pointer_handler(
                &probe_b,
                probe_b_state.clone(),
                probe_b_status.clone(),
                "Probe B",
                "on_pointer_moved",
                ProbePointerEvent::Moved,
            );
            register_probe_pointer_handler(
                &probe_b,
                probe_b_state.clone(),
                probe_b_status.clone(),
                "Probe B",
                "on_pointer_released",
                ProbePointerEvent::Released,
            );
            register_probe_pointer_handler(
                &probe_b,
                probe_b_state.clone(),
                probe_b_status.clone(),
                "Probe B",
                "on_pointer_canceled",
                ProbePointerEvent::Canceled,
            );
            register_probe_tapped_handler(
                &probe_b,
                probe_b_state.clone(),
                probe_b_status.clone(),
                "Probe B",
                "on_tapped",
            );
            register_probe_tapped_handler(
                &probe_b,
                probe_b_state.clone(),
                probe_b_status.clone(),
                "Probe B",
                "on_double_tapped",
            );
            register_probe_tapped_handler(
                &probe_b,
                probe_b_state,
                probe_b_status,
                "Probe B",
                "on_right_tapped",
            );
        }

        #[id("status")]
        let status = TextBlock {
            text: "Selected tab: Overview · click a header, close affordance, or divider to exercise callbacks"
            font_size: 13.0
            foreground: "#abb7c4"
        };

        #[id("tabs")]
        let tabs = elwindui_custom_controls::CustomTabView {
            elwindui_custom_controls::CustomTabViewItem {
                header: "Overview"
                closable: false
                OverviewPage {}
            }
            elwindui_custom_controls::CustomTabViewItem {
                header: "Inspector"
                InspectorPage {}
            }
            elwindui_custom_controls::CustomTabViewItem {
                header: "Activity"
                ActivityPage {}
            }
        };

        #[id("splitter")]
        let splitter = elwindui_custom_controls::CustomGridSplitter {};

        #[id("content_grid")]
        let content_grid = Grid {
            height: 350.0
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

        #[id("probe_a")]
        let probe_a = Rectangle {
            width: 460.0
            height: 64.0
            fill: "#356b9d"
        };
        #[id("probe_a_status")]
        let probe_a_status = TextBlock {
            text: "Probe A\npressed=0 moved=0 released=0\ncanceled=0 (last canceled button=n/a)\ntapped=0 double_tapped=0 right_tapped=0\nlast root=None screen=None button=None"
            font_size: 11.0
            foreground: "#abb7c4"
        };
        #[id("probe_b")]
        let probe_b = Rectangle {
            width: 460.0
            height: 64.0
            fill: "#587a46"
        };
        #[id("probe_b_status")]
        let probe_b_status = TextBlock {
            text: "Probe B\npressed=0 moved=0 released=0\ncanceled=0 (last canceled button=n/a)\ntapped=0 double_tapped=0 right_tapped=0\nlast root=None screen=None button=None"
            font_size: 11.0
            foreground: "#abb7c4"
        };

        HorizontalLayout {
            spacing: 12.0
            VerticalLayout {
                width: 470.0
                spacing: 4.0
                TextBlock {
                    text: "Probe A · native cancellation target"
                    font_size: 13.0
                    foreground: "#eef2f7"
                }
                probe_a
                probe_a_status
            }
            VerticalLayout {
                width: 470.0
                spacing: 4.0
                TextBlock {
                    text: "Probe B · fresh-routing target"
                    font_size: 13.0
                    foreground: "#eef2f7"
                }
                probe_b
                probe_b_status
            }
        }

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
                text: "Template-backed CustomTabView, ContentControl page ownership, and CustomGridSplitter input."
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
