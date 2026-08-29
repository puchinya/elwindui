// `#[elwindui::viewmodel] mod document_view_model { .. }` (in `main.rs`) doesn't keep its `mod`
// wrapper past expansion — `DocumentViewModel` ends up declared directly at the crate root.
use crate::DocumentViewModel;
use crate::ui::rounded_panel::{RoundedPanel, RoundedPanelExt};

// `inherits ContentControl` (docs/design/runtime/ui_tree_design.md, same pattern as
// `RoundedPanel` above) — own field renamed to `document_text` (not `content`) since
// `ContentControl` already declares an inherited `content: Rc<dyn UIElement>` field of its own.
#[elwindui::component(inherits ContentControl)]
struct DocumentView {
    #[bindable]
    doc: std::rc::Rc<DocumentViewModel>,

    // `Grid` (not `VerticalLayout`) for the outer split: `VerticalLayout`'s main axis is always
    // "Auto" (each child's own natural size) — a `TextArea` inside a `VerticalLayout` would only
    // ever get its own small natural height, never the remaining window space. `Grid`'s
    // `GridLength::Star` row does actually fill whatever's left after the status bar's `Auto` row
    // takes its own height.
    template: template_view!(|templated_parent: Self| {
        Grid {
            rows: [elwindui::core::layout::GridLength::Star(1.0), elwindui::core::layout::GridLength::Auto]
            columns: [elwindui::core::layout::GridLength::Star(1.0)]
            TextArea { text <=> doc.content, Grid::row: 0 }
            HorizontalLayout {
                Grid::row: 1
                TextBlock {
                    text: doc.file_name
                    margin: 4.0
                }
                RoundedPanel { label: t!("notepad-status-chars", count: doc.char_count) }
            }
        }
    }),
}

#[elwindui::component]
impl DocumentView {}
