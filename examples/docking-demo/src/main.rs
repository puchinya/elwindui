//! Small declarative consumer for the separate `elwindui-docking` crate.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::environment::application_environment;
use elwindui::core::graphics::{Brush, Color};
use elwindui::core::theme::{BrushStyle, Theme};
use elwindui::core::ui::WindowExt;

#[elwindui::theme]
struct VisualStudioTheme {
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(0, 120, 215))))]
    primary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(243, 243, 243))))]
    secondary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(250, 250, 250))))]
    tertiary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(31, 31, 31))))]
    foreground: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::WHITE)))]
    background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(247, 247, 247))))]
    window_background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(0, 120, 215))))]
    tint: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgba(0, 120, 215, 54))))]
    selection: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(214, 214, 214))))]
    separator: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(107, 107, 107))))]
    placeholder: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(0, 120, 215))))]
    link: BrushStyle,
}

#[elwindui::theme]
struct DarkDockingTheme {
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(0, 120, 215))))]
    primary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(45, 45, 48))))]
    secondary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(37, 37, 38))))]
    tertiary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(241, 241, 241))))]
    foreground: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(30, 30, 30))))]
    background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(30, 30, 30))))]
    window_background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(0, 120, 215))))]
    tint: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgba(0, 120, 215, 90))))]
    selection: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(80, 80, 80))))]
    separator: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(180, 180, 180))))]
    placeholder: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(75, 170, 255))))]
    link: BrushStyle,
}

#[elwindui::viewmodel]
mod docking_demo_view_model {
    struct DockingDemoViewModel {
        #[observable(default = elwindui_docking::DockLayoutModel::empty())]
        layout: elwindui_docking::DockLayoutModel,
        #[observable(default = elwindui_docking::DockLayoutModel::empty())]
        authored_layout: elwindui_docking::DockLayoutModel,
        #[observable(default = None)]
        saved_snapshot: Option<elwindui_docking::DockLayoutSnapshot>,
        #[observable(default = String::from("Ready — release a drag to publish one layout change"))]
        latest_status: String,
        #[observable(default = String::from("Active item: none"))]
        active_status: String,
        #[observable(default = String::from("Floating windows: 0"))]
        floating_status: String,
    }

    impl DockingDemoViewModel {
        fn publish_layout_status(&self, model: elwindui_docking::DockLayoutModel) {
            if self.authored_layout.borrow().is_empty() && !model.is_empty() {
                self.set_authored_layout(model.clone());
            }
            self.set_layout(model.clone());
            self.set_active_status(format!(
                "Active item: {}",
                model
                    .active_item()
                    .map(|item| format!("{:?}", item))
                    .unwrap_or_else(|| "none".to_owned())
            ));
            self.set_floating_status(format!("Floating windows: {}", model.floating_root_count()));
            self.set_latest_status("Committed a live layout change".to_owned());
        }

        fn clear_layout(&self) {
            let current = self.layout.borrow().clone();
            if let Ok(next) = current.with_cleared_layout() {
                layout = next.clone();
                self.publish_layout_status(next);
                latest_status = "Cleared live items; authored groups remain available".to_owned();
            }
        }

        fn reset_layout(&self) {
            let authored = self.authored_layout.borrow().clone();
            if let Ok(next) = authored.with_reset() {
                layout = next.clone();
                self.publish_layout_status(next);
                latest_status = "Reset to the authored docking declaration".to_owned();
            }
        }

        fn save_layout(&self) {
            saved_snapshot = Some(self.layout.borrow().snapshot());
            latest_status = "Saved the current V2 snapshot".to_owned();
        }

        fn restore_layout(&self) {
            let Some(snapshot) = self.saved_snapshot.borrow().clone() else {
                return;
            };
            if let Ok(next) = elwindui_docking::DockLayoutModel::from_snapshot(snapshot) {
                layout = next.clone();
                self.publish_layout_status(next);
                latest_status = "Restored the saved V2 snapshot".to_owned();
            } else {
                latest_status = "Snapshot restore rejected".to_owned();
            }
        }
    }
}

#[elwindui::component(inherits VerticalLayout)]
struct DockingDemoSurface {
    #[bindable]
    vm: std::rc::Rc<DockingDemoViewModel>,
    #[environment(foreground)]
    theme_foreground: BrushStyle,
    #[environment(placeholder)]
    theme_placeholder: BrushStyle,
    #[computed(expr = elwindui_docking::DockItemId::from("document-a"))]
    document_a: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("document-b"))]
    document_b: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("solution-explorer"))]
    solution_explorer: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("git-changes"))]
    git_changes: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("error-list"))]
    error_list: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("output"))]
    output: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("terminal"))]
    terminal: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockGroupId::from("documents"))]
    documents: elwindui_docking::DockGroupId,
    #[computed(expr = elwindui_docking::DockGroupId::from("solution-tools"))]
    solution_tools: elwindui_docking::DockGroupId,
    #[computed(expr = elwindui_docking::DockGroupId::from("error-tools"))]
    error_tools: elwindui_docking::DockGroupId,
    #[computed(expr = elwindui_docking::DockGroupId::from("output-tools"))]
    output_tools: elwindui_docking::DockGroupId,
    #[computed(expr = elwindui::core::layout::Orientation::Horizontal)]
    horizontal: elwindui::core::layout::Orientation,
    #[computed(expr = elwindui::core::layout::Orientation::Vertical)]
    vertical: elwindui::core::layout::Orientation,
    body: view! {
        on_mount {
            for node in elwindui::core::visual_tree::find_all::<
                elwindui_docking::DockingControl,
            >(this.as_ref()) {
                if let Some(docking) = node
                    .as_any()
                    .downcast_ref::<elwindui_docking::DockingControl>()
                {
                    let status_vm = std::rc::Rc::clone(&vm);
                    docking.set_on_layout_change(Box::new(move |layout| {
                        status_vm.publish_layout_status(layout);
                    }));
                    docking.synchronize_layout_source();
                    vm.set_latest_status("Ready — release a drag to publish one layout change".to_owned());
                }
            }
        }
        let documents = elwindui_docking::DockGroup {
            id: documents
            weight: 2.1
            elwindui_docking::DockItem {
                id: document_a
                title: "Document A"
                can_pin: false
                TextBlock { text: "Document A editor" foreground: theme_foreground }
            }
            elwindui_docking::DockItem {
                id: document_b
                title: "Document B"
                can_pin: false
                TextBlock { text: "Document B editor" foreground: theme_foreground }
            }
        };

        let solution = elwindui_docking::DockGroup {
            id: solution_tools
            tab_strip_position: elwindui_docking::TabStripPosition::Bottom
            compact_tabs: true
            show_when_empty: true
            weight: 1.0
            elwindui_docking::DockItem {
                id: solution_explorer
                title: "Solution Explorer"
                TextBlock { text: "Solution Explorer" foreground: theme_foreground }
            }
            elwindui_docking::DockItem {
                id: git_changes
                title: "Git Changes"
                can_dock: false
                TextBlock { text: "Git Changes" foreground: theme_foreground }
            }
        };

        let error = elwindui_docking::DockGroup {
            id: error_tools
            tab_strip_position: elwindui_docking::TabStripPosition::Bottom
            show_when_empty: true
            weight: 1.0
            elwindui_docking::DockItem {
                id: error_list
                title: "Error List"
                can_close: false
                TextBlock { text: "No errors" foreground: theme_placeholder }
            }
        };

        let output = elwindui_docking::DockGroup {
            id: output_tools
            tab_strip_position: elwindui_docking::TabStripPosition::Bottom
            weight: 1.0
            elwindui_docking::DockItem {
                id: output
                title: "Output"
                can_float: false
                TextBlock { text: "Build output" foreground: theme_foreground }
            }
            elwindui_docking::DockItem {
                id: terminal
                title: "Terminal"
                TextBlock { text: "Terminal" foreground: theme_foreground }
            }
        };

        let top = elwindui_docking::DockSplitPanel {
            orientation: horizontal
            weight: 2.1
            documents
            solution
        };

        let bottom = elwindui_docking::DockSplitPanel {
            orientation: horizontal
            weight: 1.0
            error
            output
        };

        let root = elwindui_docking::DockSplitPanel {
            orientation: vertical
            top
            bottom
        };

        let docking = elwindui_docking::DockingControl {
            layout <=> vm.layout
            root
        };
        let menu = HorizontalLayout {
            height: 32.0
            spacing: 18.0
            background: BrushStyle::Secondary
            TextBlock { text: "File" foreground: theme_foreground }
            TextBlock { text: "Edit" foreground: theme_foreground }
            TextBlock { text: "View" foreground: theme_foreground }
            TextBlock { text: "Help" foreground: theme_foreground }
            Button {
                text: "Clear layout"
                foreground: theme_foreground
                tooltip: "Close every live item while keeping the authored layout for Reset"
                on_click: vm.clear_layout
            }
            Button {
                text: "Reset layout"
                foreground: theme_foreground
                tooltip: "Restore the authored DockGroup/DockSplitPanel declaration"
                on_click: vm.reset_layout
            }
            Button {
                text: "Save snapshot"
                foreground: theme_foreground
                tooltip: "Capture the version-2 layout snapshot"
                on_click: || { vm.save_layout(); }
            }
            Button {
                text: "Restore snapshot"
                foreground: theme_foreground
                tooltip: "Restore the last version-2 snapshot"
                on_click: vm.restore_layout
            }
            Button {
                text: "Light theme"
                foreground: theme_foreground
                on_click: || { VisualStudioTheme.apply(&application_environment()); }
            }
            Button {
                text: "Dark theme"
                foreground: theme_foreground
                on_click: || { DarkDockingTheme.apply(&application_environment()); }
            }
        };
        let docking_host = Grid {
            height: 574.0
            rows: [elwindui::core::layout::GridLength::Star(1.0)]
            columns: [elwindui::core::layout::GridLength::Star(1.0)]
            docking
        };
        let status = HorizontalLayout {
            height: 26.0
            spacing: 18.0
            background: BrushStyle::Tertiary
            TextBlock {
                text: vm.active_status
                foreground: theme_foreground
            }
            TextBlock {
                text: vm.floating_status
                foreground: theme_foreground
            }
            TextBlock { text: vm.latest_status foreground: theme_foreground }
        };

        spacing: 0.0
        background: BrushStyle::WindowBackground
        VerticalLayout {
            spacing: 0.0
            menu
            status
            docking_host
        }
    },
}

#[elwindui::component]
impl DockingDemoSurface {}

#[elwindui::component(inherits Window)]
struct DockingDemoWindow {
    #[bindable]
    vm: std::rc::Rc<DockingDemoViewModel>,
    body: view! {
        title: "ElwindUI Docking Demo"
        width: 960.0
        height: 640.0
        content: DockingDemoSurface { vm: vm }
    },
}

#[elwindui::component]
impl DockingDemoWindow {}

#[elwindui::main]
fn main() {
    VisualStudioTheme.apply(&application_environment());
    let vm = DockingDemoViewModel::new();
    let window = elwindui::new!(DockingDemoWindow(vm: vm));
    window.show();
}
