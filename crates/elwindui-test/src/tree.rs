use elwindui_core::ui::UIElementExt;
use std::fmt::Write;

/// Indented text dump of a `UIElement` tree, for use with `assert_snapshot!` (e.g. `insta`).
/// See docs/elwindui_spec.md 付録V.1.
pub fn render_tree(root: &dyn UIElementExt) -> String {
    let mut out = String::new();
    write_node(root, 0, &mut out);
    out
}

fn write_node(node: &dyn UIElementExt, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    writeln!(out, "{}", node.type_name()).unwrap();
    for child in node.visual_children() {
        write_node(child.as_ref(), depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use elwindui_core::ui::{new_element, TextBlock, VerticalLayout, VerticalLayoutExt as _};

    #[test]
    fn dumps_nested_type_names_with_indentation() {
        let layout = VerticalLayout::construct();
        layout.children().add(new_element(TextBlock::construct()));
        layout.children().add(new_element(TextBlock::construct()));
        let tree = new_element(layout);

        let dump = render_tree(tree.as_ref());
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("VerticalLayout"));
        assert!(lines[1].starts_with("  ") && lines[1].contains("TextBlock"));
        assert!(lines[2].starts_with("  ") && lines[2].contains("TextBlock"));
    }
}
