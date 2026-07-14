use elwindui_core::ui::UIElement;
use std::fmt::Write;

/// Indented text dump of a `UIElement` tree, for use with `assert_snapshot!` (e.g. `insta`).
/// See docs/elwindui_spec.md 付録V.1.
pub fn render_tree(root: &dyn UIElement) -> String {
    let mut out = String::new();
    write_node(root, 0, &mut out);
    out
}

fn write_node(node: &dyn UIElement, depth: usize, out: &mut String) {
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
    use elwindui_core::ui::{new_element, TextBlockImpl, VerticalLayout as _, VerticalLayoutImpl};

    #[test]
    fn dumps_nested_type_names_with_indentation() {
        let layout = VerticalLayoutImpl::construct();
        layout.children().add(new_element(TextBlockImpl::construct()));
        layout.children().add(new_element(TextBlockImpl::construct()));
        let tree = new_element(layout);

        let dump = render_tree(tree.as_ref());
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("VerticalLayoutImpl"));
        assert!(lines[1].starts_with("  ") && lines[1].contains("TextBlockImpl"));
        assert!(lines[2].starts_with("  ") && lines[2].contains("TextBlockImpl"));
    }
}
