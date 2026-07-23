//! Manual verification harness for the NativeControl expansion Phase 1 controls (TextBox/
//! PasswordBox/ScrollView) added in `docs/elwindui_nativecontrol_expansion_status.md`, following
//! `examples/graphics-demo`'s own structure (single `main.rs`, `#[elwindui::viewmodel]`, one
//! `TabView` with one tab per area — see that file's own doc comment for why this shape was
//! chosen). Unlike `graphics-demo` (which exercises custom-drawn `Canvas` content), every tab here
//! is real DSL usage of the new native controls, each showing: the current property value
//! (round-tripped through two-way binding), an event log (`on_change`/`on_got_focus`/
//! `on_lost_focus`/submit), and — for TextBox — live focus state, the most direct manual check for
//! the native-focus-in wiring (§1a) this Phase's common infrastructure work added.
//!
//! `PasswordBox`'s own event log deliberately never shows the password value itself, only its
//! length (`"changed (len=N)"`) — this demo's own source doubles as documentation of the no-leak
//! policy `docs/elwindui_nativecontrol_expansion_status.md` requires (§1.6).
//!
//! The "Regression" tab re-exercises the *existing* `TextArea`/`Button` controls (unchanged by this
//! Phase, but affected by its common-infrastructure focus-wiring changes) as the demo counterpart
//! to the `docs/elwindui_nativecontrol_expansion_status.md` §2 regression-check procedure.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::input::Key;
use elwindui::ui::WindowExt;

#[elwindui::viewmodel]
mod controls_demo_view_model {
    use super::Key;

    struct ControlsDemoViewModel {
        // `TabView`'s chip click only ever fires `on_select` — it never updates `selected_index`
        // on its own, so this round-trips the click back down through `bind!` the same way
        // `examples/graphics-demo`'s own `GraphicsDemoViewModel` does (see that file's own doc
        // comment on `selected_tab`).
        #[observable(default = 0usize)]
        selected_tab: usize,

        #[observable(default = String::new())]
        text_box_value: String,
        #[observable(default = "Unfocused".to_string())]
        text_box_focus_state: String,
        #[observable(default = String::new())]
        text_box_log: String,

        #[observable(default = String::new())]
        password_box_value: String,
        #[computed(expr = format!("{}", self.password_box_value.borrow().chars().count()))]
        password_box_length: String,
        #[observable(default = String::new())]
        password_box_log: String,

        #[observable(default = String::new())]
        nested_text_box_value: String,

        #[observable(default = String::new())]
        regression_text: String,
        #[observable(default = String::new())]
        regression_log: String,
    }

    impl ControlsDemoViewModel {
        fn select_tab(&self, index: usize) {
            selected_tab = index;
        }

        // `text`/`password` themselves are already two-way bound directly (`text:
        // vm.text_box_value` in the view below) — model sync doesn't need a manual hook. Only the
        // events that *aren't* otherwise observable (focus, submit) get logged here.
        fn text_box_got_focus(&self) {
            text_box_focus_state = "Focused (Pointer)".to_string();
            text_box_log = format!("{}got_focus\n", self.text_box_log.borrow());
        }
        fn text_box_lost_focus(&self) {
            text_box_focus_state = "Unfocused".to_string();
            text_box_log = format!("{}lost_focus\n", self.text_box_log.borrow());
        }
        fn text_box_key_down(&self, key: Key) {
            if key == Key::Enter {
                text_box_log = format!("{}submit (Enter)\n", self.text_box_log.borrow());
            }
        }

        // Length only — never the password value itself. See this module's own doc comment.
        fn password_box_got_focus(&self) {
            password_box_log = format!("{}got_focus\n", self.password_box_log.borrow());
        }
        fn password_box_lost_focus(&self) {
            let len = self.password_box_value.borrow().chars().count();
            password_box_log = format!("{}lost_focus (len={len})\n", self.password_box_log.borrow());
        }

        fn regression_button_clicked(&self) {
            regression_log = format!("{}Button clicked\n", self.regression_log.borrow());
        }
    }
}

#[elwindui::component(inherits Window)]
struct ControlsDemoWindow {
    #[bindable]
    vm: std::rc::Rc<ControlsDemoViewModel>,

    body: view! {
        title: "elwindui NativeControl Demo"
        width: 640.0
        height: 480.0
        content: TabView {
            TabViewItem {
                header: "TextBox"
                closable: false
                on_close: || {}
                content: Grid {
                    rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
                    columns: [elwindui::core::layout::GridLength::Star(1.0)]

                    VerticalLayout {
                        Grid::row: 0
                        margin: 12.0
                        spacing: 6.0
                        TextBlock { text: "TextBox (single-line, submit on Enter)" }
                        TextBox {
                            text: vm.text_box_value
                            placeholder: "type here, then press Enter"
                            on_key_down: |e| { vm.text_box_key_down(e.key) }
                            on_got_focus: vm.text_box_got_focus
                            on_lost_focus: vm.text_box_lost_focus
                        }
                        HorizontalLayout {
                            spacing: 4.0
                            TextBlock { text: "focus state:" }
                            TextBlock { text: vm.text_box_focus_state }
                        }
                        HorizontalLayout {
                            spacing: 4.0
                            TextBlock { text: "current value:" }
                            TextBlock { text: vm.text_box_value }
                        }
                    }
                    TextBlock {
                        Grid::row: 1
                        margin: 12.0
                        text: "event log:"
                    }
                    ScrollView {
                        Grid::row: 2
                        margin: 12.0
                        content: TextBlock { text: vm.text_box_log }
                    }
                }
            }
            TabViewItem {
                header: "PasswordBox"
                closable: false
                on_close: || {}
                content: Grid {
                    rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
                    columns: [elwindui::core::layout::GridLength::Star(1.0)]

                    VerticalLayout {
                        Grid::row: 0
                        margin: 12.0
                        spacing: 6.0
                        TextBlock { text: "PasswordBox (masked entry, no reveal on AppKit — see log)" }
                        PasswordBox {
                            password: vm.password_box_value
                            placeholder: "type a password"
                            reveal_enabled: true
                            on_got_focus: vm.password_box_got_focus
                            on_lost_focus: vm.password_box_lost_focus
                        }
                        HorizontalLayout {
                            spacing: 4.0
                            TextBlock { text: "current length:" }
                            TextBlock { text: vm.password_box_length }
                        }
                    }
                    TextBlock {
                        Grid::row: 1
                        margin: 12.0
                        text: "event log (length only — password value is never shown):"
                    }
                    ScrollView {
                        Grid::row: 2
                        margin: 12.0
                        content: TextBlock { text: vm.password_box_log }
                    }
                }
            }
            TabViewItem {
                header: "ScrollView"
                closable: false
                on_close: || {}
                content: VerticalLayout {
                    margin: 12.0
                    spacing: 6.0
                    TextBlock { text: "ScrollView wrapping content taller than the viewport:" }
                    ScrollView {
                        height: 320.0
                        content: VerticalLayout {
                            spacing: 8.0
                            TextBlock { text: "Row 1 — scroll down to see more" }
                            TextBlock { text: "Row 2" }
                            TextBlock { text: "Row 3" }
                            TextBlock { text: "Row 4" }
                            TextBlock { text: "Row 5" }
                            TextBlock { text: "Row 6" }
                            TextBlock { text: "Row 7" }
                            TextBlock { text: "Row 8" }
                            TextBlock { text: "Row 9" }
                            TextBlock { text: "Row 10" }
                            TextBlock { text: "Row 11 — a nested TextBox, to confirm native focus still works inside a ScrollView:" }
                            TextBox { text: vm.nested_text_box_value, placeholder: "focus me while scrolled" }
                            TextBlock { text: "Row 12" }
                            TextBlock { text: "Row 13" }
                            TextBlock { text: "Row 14" }
                            TextBlock { text: "Row 15 — bottom" }
                        }
                    }
                }
            }
            TabViewItem {
                header: "Regression (TextArea/Button)"
                closable: false
                on_close: || {}
                content: Grid {
                    rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
                    columns: [elwindui::core::layout::GridLength::Star(1.0)]

                    VerticalLayout {
                        Grid::row: 0
                        margin: 12.0
                        spacing: 6.0
                        TextBlock { text: "Existing TextArea/Button/TabView — unchanged this Phase, but affected by the common focus-wiring infra change (§1a)" }
                        Button {
                            text: "Click me"
                            on_click: vm.regression_button_clicked
                        }
                        TextArea { text: vm.regression_text }
                    }
                    ScrollView {
                        Grid::row: 1
                        margin: 12.0
                        content: TextBlock { text: vm.regression_log }
                    }
                }
            }
            selected_index: vm.selected_tab
            on_select: |index| { vm.select_tab(index) }
            on_new_tab: || {}
        }
    },
}

#[elwindui::main]
fn main() {
    let vm = ControlsDemoViewModel::new();
    let window = ControlsDemoWindow::new(vm);
    window.show();
}
