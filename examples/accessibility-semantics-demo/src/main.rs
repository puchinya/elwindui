//! Durable fixture for `tests/e2e/accessibility-semantics/scenario.md`.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::accessibility::AccessibilityRole;
use elwindui::core::base::{CornerRadius, Rect};
use elwindui::core::graphics::{Brush, Color, RenderContext};
use elwindui::core::ui::{Animation, CheckState, Transition, UIElementExt, WindowExt};
use std::time::Duration;

#[elwindui::viewmodel]
mod accessibility_semantics_demo_view_model {
    use super::{Animation, CheckState, Duration};

    struct AccessibilitySemanticsDemoViewModel {
        #[observable(default = 0usize)]
        button_count: usize,
        #[observable(default = CheckState::Unchecked)]
        check_box_checked: CheckState,
        #[observable(default = 0.0)]
        slider_value: f32,
        #[observable(default = "hello".to_string())]
        text_area_value: String,
        #[observable(default = true)]
        show_exiting_text_area: bool,
        #[computed(expr = {
            let button = button_count;
            let checkbox = check_box_checked;
            let slider = slider_value;
            let text = text_area_value;
            format!("button={} checkbox={:?} slider={} text={}", button, checkbox, slider, text)
        })]
        result: String,
    }

    impl AccessibilitySemanticsDemoViewModel {
        fn activate_button(&self) {
            button_count = button_count + 1;
        }

        fn remove_exiting_text_area(&self) {
            elwindui::core::ui::with_animation(
                Animation::ease_out(Duration::from_millis(1200)),
                || {
                    show_exiting_text_area = false;
                },
            );
        }
    }
}

#[elwindui::class(inherits = elwindui::core::ui::UIElement)]
struct AccessibilityCanvas {}

#[elwindui::class]
impl AccessibilityCanvas {
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        context.fill_rounded_rect(
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.arranged_width().unwrap_or(0.0),
                height: self.arranged_height().unwrap_or(0.0),
            },
            CornerRadius::uniform(8.0),
            &Brush::Solid(Color::rgb(220, 235, 250)),
        );
    }

    fn construct() -> Self {
        Self {
            base: elwindui::core::ui::UIElement::construct(),
        }
    }
}

#[elwindui::component(inherits Window)]
struct AccessibilitySemanticsDemoWindow {
    #[bindable]
    vm: std::rc::Rc<AccessibilitySemanticsDemoViewModel>,
    #[param]
    a11y_canvas: std::rc::Rc<dyn UIElementExt>,

    body: view! {
        title: "ElwindUI Accessibility Semantics Demo"
        width: 620.0
        height: 560.0
        content: VerticalLayout {
            margin: 16.0
            spacing: 8.0
            TextBlock {
                text: "Core-owned accessibility semantics"
                accessibility_identifier: "a11y-text"
            }
            TextBlock {
                text: vm.result
                accessibility_identifier: "a11y-result"
            }
            Button {
                text: "Activate"
                accessibility_identifier: "a11y-button"
                on_click: vm.activate_button
            }
            TextArea {
                text <=> vm.text_area_value
                accessibility_identifier: "a11y-text-area"
            }
            CheckBox {
                text: "Subscribe to updates"
                checked <=> vm.check_box_checked
                accessibility_identifier: "a11y-check-box"
            }
            Slider {
                width: 260.0
                value <=> vm.slider_value
                min: 0.0
                max: 10.0
                accessibility_identifier: "a11y-slider"
            }
            a11y_canvas
            VerticalLayout {
                accessibility_hidden: true
                accessibility_identifier: "a11y-hidden"
                TextBlock { text: "This subtree is hidden from accessibility" }
            }
            Button {
                text: "Remove exiting TextArea"
                on_click: vm.remove_exiting_text_area
            }
            if vm.show_exiting_text_area {
                #[transition(Transition::opacity())]
                TextArea {
                    text: "This TextArea exits visually"
                    accessibility_identifier: "a11y-exiting-text-area"
                }
            }
        }
    },
}

#[elwindui::component]
impl AccessibilitySemanticsDemoWindow {}

#[elwindui::main]
fn main() {
    let vm = AccessibilitySemanticsDemoViewModel::new();
    let canvas = AccessibilityCanvas::new();
    canvas.set_width(260.0);
    canvas.set_height(42.0);
    canvas.set_accessibility_role(AccessibilityRole::Group);
    canvas.set_accessibility_label("Canvas semantic");
    canvas.set_accessibility_identifier("a11y-canvas");
    let canvas: std::rc::Rc<dyn UIElementExt> = canvas;
    let window = elwindui::new!(AccessibilitySemanticsDemoWindow(vm: vm, a11y_canvas: canvas));
    window.show();
}
