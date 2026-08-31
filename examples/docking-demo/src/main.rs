//! Small declarative consumer for the separate `elwindui-docking` crate.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::WindowExt;
#[elwindui::component(inherits VerticalLayout)]
struct DockingDemoSurface {
    #[computed(expr = elwindui_docking::DockItemId::from("document-a"))]
    document_a: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("document-b"))]
    document_b: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("outline"))]
    outline: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockItemId::from("terminal"))]
    terminal: elwindui_docking::DockItemId,
    #[computed(expr = elwindui_docking::DockGroupId::from("documents"))]
    documents: elwindui_docking::DockGroupId,
    #[computed(expr = elwindui_docking::DockGroupId::from("outline-tools"))]
    outline_tools: elwindui_docking::DockGroupId,
    #[computed(expr = elwindui_docking::DockGroupId::from("terminal-tools"))]
    terminal_tools: elwindui_docking::DockGroupId,
    #[computed(expr = elwindui::core::layout::Orientation::Horizontal)]
    horizontal: elwindui::core::layout::Orientation,
    #[computed(expr = elwindui::core::layout::Orientation::Vertical)]
    vertical: elwindui::core::layout::Orientation,
    body: view! {
        let documents = elwindui_docking::DockGroup {
            id: documents
            elwindui_docking::DockItem {
                id: document_a
                title: "Document A"
                TextBlock { text: "Document A" }
            }
            elwindui_docking::DockItem {
                id: document_b
                title: "Document B"
                TextBlock { text: "Document B" }
            }
        };

        let outline = elwindui_docking::DockGroup {
            id: outline_tools
            elwindui_docking::DockItem {
                id: outline
                title: "Outline"
                TextBlock { text: "Outline tool window" }
            }
        };

        let terminal = elwindui_docking::DockGroup {
            id: terminal_tools
            elwindui_docking::DockItem {
                id: terminal
                title: "Terminal"
                TextBlock { text: "Terminal tool window" }
            }
        };

        let tools = elwindui_docking::DockSplitPanel {
            orientation: vertical
            outline
            terminal
        };

        let root = elwindui_docking::DockSplitPanel {
            orientation: horizontal
            documents
            tools
        };

        let docking = elwindui_docking::DockingControl {
            root
        };

        docking
    },
}

#[elwindui::component]
impl DockingDemoSurface {}

#[elwindui::component(inherits Window)]
struct DockingDemoWindow {
    body: view! {
        title: "ElwindUI Docking Demo"
        width: 960.0
        height: 640.0
        content: DockingDemoSurface {}
    },
}

#[elwindui::component]
impl DockingDemoWindow {}

#[elwindui::main]
fn main() {
    let model = elwindui_docking::DockLayoutModel::empty();
    let json = serde_json::to_string(&model.snapshot()).expect("snapshot serialization");
    let _: elwindui_docking::DockLayoutSnapshot =
        serde_json::from_str(&json).expect("snapshot parse");
    let window = DockingDemoWindow::new();
    window.show();
}
