//! Manual verification harness for Issue #246 animation and transition behavior.
//!
//! The controls cover explicit transactions, an implicit `#[animation]` scope, self-drawn layout
//! presentation, dynamic insertion/removal, a NativeControl removal while focused, and the
//! `ReduceMotionEnvironment` snap path. AppKit is the host used for local runtime evidence;
//! WinUI 3 uses the same Core declarations when run on Windows.
//!
//! Issue #290 adds stable-ID position markers below the dynamic self-drawn panel and the dynamic
//! NativeControl. Inserting or removing either region moves its marker by layout reflow, so a
//! native UI driver can read intermediate, exit, reinsertion, reversal, and Reduce Motion
//! positions from the marker's semantic bounds. A separate verification-only region at the end
//! inserts a 120 DIP spacer before a tall probe TextBox over a fixed 12 s linear transaction, so a
//! driver can click a point inside the probe's shown bounds but outside its target bounds.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::environment::{ReduceMotionEnvironment, application_environment};
#[allow(unused_imports)]
use elwindui::core::ui::{Animation, Transition, WindowExt};
use std::time::Duration;

#[elwindui::viewmodel]
mod animation_demo_view_model {
    use super::{Animation, ReduceMotionEnvironment, application_environment};
    use std::time::Duration;

    struct AnimationDemoViewModel {
        #[observable(default = true)]
        show_panel: bool,
        #[observable(default = true)]
        show_native: bool,
        #[observable(default = false)]
        show_reflow_spacer: bool,
        #[observable(default = false)]
        reduce_motion: bool,
        #[observable(default = "Unfocused".to_string())]
        probe_focus: String,
        #[computed(expr = reduce_motion.to_string())]
        reduce_motion_label: String,
        #[observable(default = "Unfocused".to_string())]
        native_focus: String,
        #[observable(default = "Ready".to_string())]
        status: String,
    }

    impl AnimationDemoViewModel {
        fn toggle_panel(&self) {
            elwindui::core::ui::with_animation(
                // Long enough for a native UI driver to sample the reflowing marker below the panel.
                Animation::ease_in_out(Duration::from_millis(4000)),
                || {
                    show_panel = !show_panel;
                },
            );
            status = "Dynamic self-drawn panel toggled".to_string();
        }

        fn toggle_native(&self) {
            elwindui::core::ui::with_animation(
                // Keep the native exit long enough to capture, but short enough that the GUI E2E
                // can exercise both the Exiting and completed states without a 20-second wait.
                Animation::ease_out(Duration::from_millis(5000)),
                || {
                    show_native = !show_native;
                },
            );
            status = "Dynamic NativeControl toggled".to_string();
        }

        fn toggle_reflow_spacer(&self) {
            elwindui::core::ui::with_animation(
                // Verification-only: slow and linear so the moving TextBox position between two
                // driver samples is monotonic and bracketed.
                Animation::linear(Duration::from_millis(12000)),
                || {
                    show_reflow_spacer = !show_reflow_spacer;
                },
            );
            status = "Reflow spacer toggled".to_string();
        }

        fn probe_got_focus(&self) {
            probe_focus = "Focused".to_string();
        }

        fn probe_lost_focus(&self) {
            probe_focus = "Unfocused".to_string();
        }

        fn native_got_focus(&self) {
            native_focus = "Focused".to_string();
            status = "Focus entered the native TextBox".to_string();
        }

        fn native_lost_focus(&self) {
            native_focus = "Unfocused".to_string();
        }

        fn toggle_reduce_motion(&self) {
            let enabled = !self.reduce_motion.get();
            reduce_motion = enabled;
            application_environment().set::<ReduceMotionEnvironment>(enabled);
            status = if enabled {
                "Reduce motion: transitions snap".to_string()
            } else {
                "Reduce motion: animations enabled".to_string()
            };
        }

        fn reset(&self) {
            elwindui::core::ui::with_animation(
                Animation::spring(Duration::from_millis(360), 0.82),
                || {
                    show_panel = true;
                    show_native = true;
                },
            );
            reduce_motion = false;
            application_environment().set::<ReduceMotionEnvironment>(false);
            status = "Reset with a spring transaction".to_string();
        }
    }
}

#[elwindui::component(inherits Window)]
struct AnimationDemoWindow {
    #[bindable]
    vm: std::rc::Rc<AnimationDemoViewModel>,

    #[state(default = false)]
    expanded: bool,

    #[computed(expr = if expanded { 440.0 } else { 140.0 })]
    expanded_width: f32,

    body: view! {
        title: "ElwindUI Animation Demo"
        width: 620.0
        height: 880.0
        content: VerticalLayout {
            margin: 18.0
            spacing: 10.0
            TextBlock { text: "Animation / Transition contract demo" }
            TextBlock { text: "Explicit transactions drive dynamic regions; the width below uses a scoped implicit animation." }
            HorizontalLayout {
                spacing: 6.0
                Button {
                    text: "Animate width"
                    on_click: || { expanded = !expanded; }
                }
                Button {
                    text: "Insert / remove panel"
                    on_click: vm.toggle_panel
                }
                Button {
                    text: "Insert / remove TextBox"
                    on_click: vm.toggle_native
                }
                Button {
                    text: "Reduce motion"
                    on_click: vm.toggle_reduce_motion
                }
                Button {
                    text: "Reset"
                    on_click: vm.reset
                }
            }
            HorizontalLayout {
                spacing: 6.0
                Button {
                    text: "Toggle reflow spacer"
                    on_click: vm.toggle_reflow_spacer
                }
            }
            #[animation(animation = Animation::ease_in_out(Duration::from_millis(5000)), value = expanded)]
            TextBlock {
                text: "Scoped implicit self-drawn presentation"
                width: expanded_width
            }
            // A solid marker makes the animated layout boundary directly observable in the
            // AppKit screenshot E2E; measuring text glyphs would saturate at intrinsic text width
            // before the animated container reaches its target.
            #[animation(animation = Animation::ease_in_out(Duration::from_millis(5000)), value = expanded)]
            Rectangle {
                width: expanded_width
                height: 8.0
                fill: "#469ce8"
            }
            if vm.show_panel {
                #[transition(Transition::opacity().combined(Transition::scale(0.92)))]
                TextBlock { text: "Dynamic self-drawn child: insertion and removal are animated" }
            }
            TextBlock {
                text: "Reflow marker below the self-drawn panel"
                accessibility_identifier: "animation-reflow-panel-marker"
            }
            if vm.show_native {
                #[transition(Transition::asymmetric(
                    Transition::opacity().combined(Transition::offset(elwindui::core::base::Vector { x: 28.0, y: 0.0 })),
                    Transition::opacity().combined(Transition::scale(0.92)),
                ))]
                TextBox {
                    placeholder: "Focus me, then remove this control"
                    accessibility_identifier: "animation-native-textbox"
                    on_got_focus: vm.native_got_focus
                    on_lost_focus: vm.native_lost_focus
                }
            }
            TextBlock {
                text: "Reflow marker below the native TextBox"
                accessibility_identifier: "animation-reflow-native-marker"
            }
            TextBlock { text: "Native focus:" }
            TextBlock {
                text: vm.native_focus
                accessibility_identifier: "animation-native-focus-state"
            }
            TextBlock { text: "Reduce motion:" }
            TextBlock { text: vm.reduce_motion_label }
            TextBlock { text: vm.status }
            // Verification-only (LR-04): the spacer moves the tall probe by 130 DIP, more than
            // enough to leave a band that is inside the probe's shown bounds but outside its
            // target bounds for the whole 12 s transaction.
            if vm.show_reflow_spacer {
                Rectangle {
                    height: 120.0
                    fill: "#e6e6e6"
                }
            }
            TextBox {
                height: 160.0
                placeholder: "Reflow pointer probe"
                accessibility_identifier: "animation-reflow-probe-textbox"
                on_got_focus: vm.probe_got_focus
                on_lost_focus: vm.probe_lost_focus
            }
            TextBlock {
                text: vm.probe_focus
                accessibility_identifier: "animation-reflow-probe-focus-state"
            }
        }
    },
}

#[elwindui::component]
impl AnimationDemoWindow {}

#[elwindui::main]
fn main() {
    application_environment().set::<ReduceMotionEnvironment>(false);
    let vm = AnimationDemoViewModel::new();
    let window = elwindui::new!(AnimationDemoWindow(vm: vm));
    window.show();
}
