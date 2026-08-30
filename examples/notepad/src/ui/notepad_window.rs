// `#[elwindui::viewmodel] mod notepad_view_model { .. }` (in `main.rs`) doesn't keep its `mod`
// wrapper past expansion — `NotepadViewModel` ends up declared directly at the crate root.
use crate::NotepadViewModel;
use crate::ui::document_view::DocumentView;

// Demonstrates a same-crate sibling `#[elwindui::component]` reference resolved entirely through
// `component_frontend::same_crate_components` (no text-form counterpart existed —
// carried over from the now-removed `examples/notepad-inline`, which used it to show the mechanism
// works within a single file; here it instead demonstrates the same registry resolving across two
// files in the same crate, `notepad_window.rs`'s own declaration and use immediately below).
#[elwindui::component(inherits ContentControl)]
struct CustomCheckBox {
    #[prop(default = false)]
    is_checked: bool,
    #[prop(default = String::new())]
    label: String,

    template: template_view!(|templated_parent: Self| {
        tab_stop: true
        #[shortcut("Ctrl+D")]
        on_tapped: |e| { is_checked = !is_checked }
        HorizontalLayout {
            if is_checked {
                Rectangle {
                    width: 16.0
                    height: 16.0
                    fill: "#0078D7"
                    stroke: "#000000"
                    stroke_width: 1.0
                }
            } else {
                Rectangle {
                    width: 16.0
                    height: 16.0
                    fill: "#FFFFFF"
                    stroke: "#000000"
                    stroke_width: 1.0
                }
            }
            TextBlock { text: label, foreground: "#ffffff" }
        }
    }),
}

#[elwindui::component]
impl CustomCheckBox {}

#[elwindui::component(inherits Window)]
struct NotepadWindow {
    #[bindable]
    vm: std::rc::Rc<NotepadViewModel>,

    body: view! {
        title: t!("notepad-app-title")
        left: 200.0
        top: 200.0
        width: 640.0
        height: 480.0

        menu_bar: MenuBar {
            MenuBarItem {
                text: t!("notepad-menu-file")
                Menu {
                    MenuItem { text: t!("notepad-menu-new"), shortcut: "n", on_select: vm.new_tab }
                    MenuItem { text: t!("notepad-menu-open"), shortcut: "o", on_select: vm.open }
                    MenuItem { text: t!("notepad-menu-save"), shortcut: "s", on_select: vm.save, enabled: vm.save_can_execute }
                }
            }
        }

        content: Grid {
            rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
            columns: [elwindui::core::layout::GridLength::Star(1.0)]
            HorizontalLayout {
                Grid::row: 0
                CustomCheckBox { label: t!("notepad-demo-checkbox") }
            }
            TabView {
                Grid::row: 1
                for doc in vm.documents {
                    TabViewItem {
                        header: doc.file_name
                        closable: true
                        on_close: vm.close_active_tab
                        DocumentView { doc: doc }
                    }
                }
                selected_index <=> vm.active_tab
                on_new_tab: vm.new_tab
            }
        }
    },
}

#[elwindui::component]
impl NotepadWindow {}
