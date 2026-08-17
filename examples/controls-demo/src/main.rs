//! Manual verification harness for the NativeControl expansion Phase 1 controls (TextBox/
//! PasswordBox/ScrollView) added in `docs/status/control_status.md`, following
//! `examples/graphics-demo`'s own structure (single `main.rs`, `#[elwindui::viewmodel]`, one
//! `TabView` with one tab per area — see that file's own doc comment for why this shape was
//! chosen). Unlike `graphics-demo` (which exercises custom-drawn `Canvas` content), every tab here
//! is real DSL usage of the new native controls, each showing: the current property value
//! (round-tripped through two-way binding), an event log (`on_change`/`on_got_focus`/
//! `on_lost_focus`/submit), and — for TextBox — live focus state, the most direct manual check for
//! the native-focus-in wiring this Phase's common infrastructure work added.
//!
//! `PasswordBox`'s own event log deliberately never shows the password value itself, only its
//! length (`"changed (len=N)"`) — this demo's own source doubles as documentation of the no-leak
//! policy recorded by `docs/status/control_status.md`.
//!
//! The "Regression" tab re-exercises the *existing* `TextArea`/`Button` controls (unchanged by this
//! Phase, but affected by its common-infrastructure focus-wiring changes) as the demo counterpart
//! to the `docs/status/control_status.md` regression-check procedure.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::input::Key;
#[allow(unused_imports)]
use elwindui::core::ui::{
    CheckState, ContextMenuPresentation, LayoutExt, TextBlockExt,
    UIElementExt, ViewTemplate,
};

#[elwindui::viewmodel]
mod controls_demo_view_model {
    use super::{
        CheckState, ContextMenuPresentation, Key, LayoutExt, TextBlockExt,
        TextStyleOwner, UIElementExt, ViewTemplate,
    };

    struct ControlsDemoViewModel {
        // `<=>` installs `TabView`'s typed selected-index write-back callback; a chip click updates
        // this observable and the resulting PropertyChanged notification resynchronizes the view.
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
        button_log: String,

        #[observable(default = CheckState::Unchecked)]
        check_box_checked: CheckState,
        #[computed(expr = match check_box_checked {
            CheckState::Unchecked => "Unchecked".to_string(),
            CheckState::Checked => "Checked".to_string(),
            CheckState::Indeterminate => "Indeterminate".to_string(),
        })]
        check_box_checked_label: String,
        #[observable(default = String::new())]
        selection_log: String,

        // Two-way bound directly (`checked <=> vm.radio_small_checked` in the view below), the same
        // way `TextBox { text <=> vm.text_box_value }` round-trips without an explicit `on_change:` —
        // `CheckBox`/`RadioButton`/`ToggleSwitch` have no user-visible `on_change` property either
        // (see `elwindui_core::ui::TextBox`'s own `set_on_change`: a plain, non-`#[prop]` trait
        // method the `#[two_way]` binding machinery calls internally). The native backend's own
        // group-exclusivity bookkeeping (`group: "size"` below) keeps only one of these three ever
        // `true`, and that change flows back up through the same two-way path.
        #[observable(default = true)]
        radio_small_checked: bool,
        #[observable(default = false)]
        radio_medium_checked: bool,
        #[observable(default = false)]
        radio_large_checked: bool,
        #[computed(expr = {
            if radio_small_checked { "Small".to_string() }
            else if radio_medium_checked { "Medium".to_string() }
            else if radio_large_checked { "Large".to_string() }
            else { "(none)".to_string() }
        })]
        radio_selected_label: String,

        #[computed(expr = Some(ViewTemplate::new(|_ctx| {
            let layout = elwindui::core::ui::VerticalLayout::new();
            layout.set_margin(12.0);
            let text = elwindui::core::ui::TextBlock::new();
            text.set_text("✨ Rich Context Popup (Custom UIElement)");
            text.text_style.set_foreground(Some(elwindui::core::graphics::Color::rgb(20, 20, 25).into()));
            LayoutExt::children(&*layout).add(text as std::rc::Rc<dyn UIElementExt>);
            Some(layout as std::rc::Rc<dyn UIElementExt>)
        })))]
        custom_popup_template: Option<ViewTemplate>,

        // A second logical group under the same native parent catches WinUI's implicit
        // parent-based grouping: changing this pair must never clear the `size` group above.
        #[observable(default = true)]
        radio_light_checked: bool,
        #[observable(default = false)]
        radio_dark_checked: bool,
        #[computed(expr = {
            if radio_light_checked { "Light".to_string() }
            else if radio_dark_checked { "Dark".to_string() }
            else { "(none)".to_string() }
        })]
        radio_theme_label: String,

        #[observable(default = false)]
        toggle_is_on: bool,
        #[computed(expr = toggle_is_on.to_string())]
        toggle_is_on_label: String,

        // `Dropdown` has no user-visible `on_change` either — same two-way-only shape as
        // `checked`/`is_on` above (see `Dropdown`'s own doc comment, elwindui-core).
        #[observable(default = 0usize)]
        dropdown_selected_index: usize,
        #[computed(expr = match dropdown_selected_index {
            0 => "Small".to_string(),
            1 => "Medium".to_string(),
            2 => "Large".to_string(),
            3 => "Extra Large".to_string(),
            _ => "(none)".to_string(),
        })]
        dropdown_selected_label: String,
        // Toggled by a button below to demonstrate `items` changing at runtime (a 4th
        // `DropdownItem` appearing/disappearing) — the native item list must follow along.
        #[observable(default = false)]
        dropdown_extra_item: bool,

        // `min`/`max` are plain (one-way) `#[prop]`s, not `#[two_way]` — only `value` round-trips.
        // Bound to `vm` fields anyway to demonstrate the range can still change reactively at
        // runtime (toggled by a button below), just without a change-back path of its own.
        #[observable(default = 0.5f32)]
        slider_value: f32,
        #[computed(expr = slider_value.to_string())]
        slider_value_label: String,
        #[observable(default = 0.0f32)]
        slider_min: f32,
        #[observable(default = 1.0f32)]
        slider_max: f32,
        #[observable(default = false)]
        slider_wide_range: bool,

        #[observable(default = String::new())]
        regression_text: String,
        #[observable(default = String::new())]
        regression_log: String,

        #[observable(default = String::new())]
        context_menu_log: String,
    }

    impl ControlsDemoViewModel {
        fn context_menu_item_selected(&self, which: String) {
            context_menu_log = format!("{}{which} selected\n", self.context_menu_log.borrow());
        }

        // `text`/`password` themselves are already two-way bound directly (`text <=>
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
            password_box_log =
                format!("{}lost_focus (len={len})\n", self.password_box_log.borrow());
        }

        fn button_clicked(&self, which: String) {
            button_log = format!("{}{which} clicked\n", self.button_log.borrow());
        }

        // The only real event in this tab: `on_click` is a genuine `#[routed]` event, unlike
        // `checked`/`is_on` above, which round-trip through two-way binding with no user-visible
        // `on_change` — see those fields' own comments.
        fn force_indeterminate(&self) {
            check_box_checked = CheckState::Indeterminate;
            selection_log = format!(
                "{}Force Indeterminate clicked (programmatic — not reachable by a user click)\n",
                self.selection_log.borrow()
            );
        }

        fn toggle_dropdown_extra_item(&self) {
            dropdown_extra_item = !self.dropdown_extra_item.get();
        }

        fn toggle_slider_range(&self) {
            let wide = !self.slider_wide_range.get();
            slider_wide_range = wide;
            slider_min = if wide { -100.0 } else { 0.0 };
            slider_max = if wide { 100.0 } else { 1.0 };
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

    #[state(default = "")]
    search_query: String,

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
                        TextBlock { text: "Component-owned search state" }
                        TextBox {
                            text <=> search_query
                            placeholder: "search locally"
                        }
                        TextBlock { text: format!("Live query: {}", search_query) }
                        TextBlock { text: once!(format!("Initial snapshot: {}", search_query)) }
                        TextBox {
                            text <=> vm.text_box_value
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
                            password <=> vm.password_box_value
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
                        height: 150.0
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
                            TextBox { text <=> vm.nested_text_box_value, placeholder: "focus me while scrolled" }
                            TextBlock { text: "Row 12" }
                            TextBlock { text: "Row 13" }
                            TextBlock { text: "Row 14" }
                            TextBlock { text: "Row 15 — bottom" }
                        }
                    }
                }
            }
            TabViewItem {
                header: "Button"
                closable: false
                on_close: || {}
                content: Grid {
                    rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
                    columns: [elwindui::core::layout::GridLength::Star(1.0)]

                    VerticalLayout {
                        Grid::row: 0
                        margin: 12.0
                        spacing: 6.0
                        TextBlock { text: "role — mapped to each platform's own emphasis affordance, not to elwindui styling:" }
                        HorizontalLayout {
                            spacing: 8.0
                            Button {
                                text: "Normal"
                                tooltip: "An ordinary action — the plain platform push button"
                                on_click: || { vm.button_clicked("Normal".to_string()) }
                            }
                            Button {
                                text: "Primary"
                                role: elwindui::core::ui::ButtonRole::Primary
                                tooltip: "Accent-filled: the action this view is primarily for"
                                on_click: || { vm.button_clicked("Primary".to_string()) }
                            }
                            Button {
                                text: "Destructive"
                                role: elwindui::core::ui::ButtonRole::Destructive
                                tooltip: "Red-filled bezel; also flagged hasDestructiveAction on macOS 11+"
                                on_click: || { vm.button_clicked("Destructive".to_string()) }
                            }
                        }
                        TextBlock { text: "is_default — Return activates it. Orthogonal to role, so both can be set:" }
                        HorizontalLayout {
                            spacing: 8.0
                            Button {
                                text: "Default (press Return)"
                                is_default: true
                                tooltip: "keyEquivalent = \\r on AppKit"
                                on_click: || { vm.button_clicked("Default".to_string()) }
                            }
                            Button {
                                text: "Disabled"
                                enabled: false
                                tooltip: "Tooltips still show on a disabled control"
                                on_click: || { vm.button_clicked("Disabled".to_string()) }
                            }
                        }
                        TextBlock { text: "tooltip is declared on NativeControl, so every native leaf has it — hover the TextBox below:" }
                        TextBox {
                            text <=> vm.nested_text_box_value
                            placeholder: "hover me"
                            tooltip: "A TextBox tooltip, inherited from NativeControl"
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
                        content: TextBlock { text: vm.button_log }
                    }
                }
            }
            TabViewItem {
                header: "Selection"
                closable: false
                on_close: || {}
                content: Grid {
                    rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
                    columns: [elwindui::core::layout::GridLength::Star(1.0)]

                    VerticalLayout {
                        Grid::row: 0
                        margin: 12.0
                        spacing: 6.0
                        TextBlock { text: "CheckBox — user clicks only ever reach Unchecked/Checked:" }
                        HorizontalLayout {
                            spacing: 8.0
                            CheckBox {
                                text: "Subscribe to updates"
                                checked <=> vm.check_box_checked
                            }
                            TextBlock { text: vm.check_box_checked_label }
                        }
                        Button {
                            text: "Force Indeterminate (programmatic only)"
                            on_click: vm.force_indeterminate
                        }
                        TextBlock { text: "RadioButton — same group, elwindui's own exclusivity bookkeeping:" }
                        HorizontalLayout {
                            spacing: 8.0
                            RadioButton {
                                text: "Small"
                                group: "size"
                                checked <=> vm.radio_small_checked
                            }
                            RadioButton {
                                text: "Medium"
                                group: "size"
                                checked <=> vm.radio_medium_checked
                            }
                            RadioButton {
                                text: "Large"
                                group: "size"
                                checked <=> vm.radio_large_checked
                            }
                            TextBlock { text: vm.radio_selected_label }
                        }
                        TextBlock { text: "RadioButton — separate group under the same native parent:" }
                        HorizontalLayout {
                            spacing: 8.0
                            RadioButton {
                                text: "Light"
                                group: "theme"
                                checked <=> vm.radio_light_checked
                            }
                            RadioButton {
                                text: "Dark"
                                group: "theme"
                                checked <=> vm.radio_dark_checked
                            }
                            TextBlock { text: vm.radio_theme_label }
                        }
                        TextBlock { text: "ToggleSwitch — no text property of its own, paired with a TextBlock:" }
                        HorizontalLayout {
                            spacing: 8.0
                            ToggleSwitch {
                                is_on <=> vm.toggle_is_on
                            }
                            TextBlock { text: "Airplane mode" }
                            TextBlock { text: vm.toggle_is_on_label }
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
                        content: TextBlock { text: vm.selection_log }
                    }
                }
            }
            TabViewItem {
                header: "Dropdown"
                closable: false
                on_close: || {}
                content: VerticalLayout {
                    margin: 12.0
                    spacing: 6.0
                    TextBlock { text: "Dropdown — non-editable native selection, selected_index is the single source of truth:" }
                    HorizontalLayout {
                        spacing: 8.0
                        Dropdown {
                            selected_index <=> vm.dropdown_selected_index
                            DropdownItem { text: "Small" }
                            DropdownItem { text: "Medium" }
                            DropdownItem { text: "Large" }
                            if vm.dropdown_extra_item {
                                DropdownItem { text: "Extra Large" }
                            }
                        }
                        TextBlock { text: vm.dropdown_selected_label }
                    }
                    Button {
                        text: "Toggle 4th item (Extra Large)"
                        on_click: vm.toggle_dropdown_extra_item
                    }
                }
            }
            TabViewItem {
                header: "Slider"
                closable: false
                on_close: || {}
                content: VerticalLayout {
                    margin: 12.0
                    spacing: 6.0
                    TextBlock { text: "Slider — value is #[two_way], min/max are one-way #[prop]s:" }
                    HorizontalLayout {
                        spacing: 8.0
                        Slider {
                            // `NSSlider`'s own `fittingSize()` has no natural width (a slider's
                            // length isn't content-derived the way a button's title is) — an
                            // explicit `width` is required, the same way any `UIElement` overrides
                            // its own measured size (docs/design/runtime/ui_tree_design.md).
                            width: 200.0
                            value <=> vm.slider_value
                            min: vm.slider_min
                            max: vm.slider_max
                        }
                        TextBlock { text: vm.slider_value_label }
                    }
                    Button {
                        text: "Toggle range (0..1 / -100..100)"
                        on_click: vm.toggle_slider_range
                    }
                }
            }
            TabViewItem {
                header: "Context Menu"
                closable: false
                on_close: || {}
                content: Grid {
                    rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
                    columns: [elwindui::core::layout::GridLength::Star(1.0)]

                    VerticalLayout {
                        Grid::row: 0
                        margin: 12.0
                        spacing: 8.0

                        TextBlock { text: "1. Native Context Menu (Right-click or Shift+F10 / Menu key):" }
                        VerticalLayout {
                            margin: 4.0
                            TextBlock {
                                margin: 8.0
                                text: "【Native Context Menu Target — Right-click here】"
                                context_menu: Menu {
                                    MenuItem {
                                        text: "Cut"
                                        on_select: || vm.context_menu_item_selected("Native > Cut".to_string())
                                    }
                                    MenuItem {
                                        text: "Copy"
                                        on_select: || vm.context_menu_item_selected("Native > Copy".to_string())
                                    }
                                    MenuItem {
                                        text: "Paste"
                                        on_select: || vm.context_menu_item_selected("Native > Paste".to_string())
                                    }
                                }
                            }
                        }

                        TextBlock { text: "2. Custom-rendered Context Menu (ElwindUI UIElement presentation with shortcuts & disabled item):" }
                        VerticalLayout {
                            margin: 4.0
                            TextBlock {
                                margin: 8.0
                                text: "【Custom-rendered Context Menu Target — Right-click here】"
                                context_menu: Menu {
                                    MenuItem {
                                        text: "Custom Action 1 (Save)"
                                        shortcut: "S"
                                        on_select: || vm.context_menu_item_selected("Custom > Action 1 (Save)".to_string())
                                    }
                                    MenuItem {
                                        text: "Disabled Action"
                                        enabled: false
                                        on_select: || vm.context_menu_item_selected("Custom > Disabled Action".to_string())
                                    }
                                    MenuItem {
                                        text: "Custom Action 2"
                                        on_select: || vm.context_menu_item_selected("Custom > Action 2".to_string())
                                    }
                                }
                                context_menu_presentation: ContextMenuPresentation::Custom
                            }
                        }

                        TextBlock { text: "3. Custom Context Popup (arbitrary UIElement subtree):" }
                        VerticalLayout {
                            margin: 4.0
                            TextBlock {
                                margin: 8.0
                                text: "【Custom Context Popup Target — Right-click here】"
                                context_popup: vm.custom_popup_template
                            }
                        }
                    }

                    TextBlock {
                        Grid::row: 1
                        margin: 12.0
                        text: "Context Menu Event Log:"
                    }

                    ScrollView {
                        Grid::row: 2
                        margin: 12.0
                        content: TextBlock { text: vm.context_menu_log }
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
                        TextArea { text <=> vm.regression_text }
                    }
                    ScrollView {
                        Grid::row: 1
                        margin: 12.0
                        content: TextBlock { text: vm.regression_log }
                    }
                }
            }
            selected_index <=> vm.selected_tab
            on_new_tab: || {}
        }
    },
}

#[elwindui::component]
impl ControlsDemoWindow {}

#[elwindui::main]
fn main() {
    let vm = ControlsDemoViewModel::new();
    let window = ControlsDemoWindow::new(vm);
    window.show();
}
