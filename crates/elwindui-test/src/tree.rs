use elwindui_core::Element;
use std::fmt::Write;

/// Indented text dump of an `Element` tree, for use with `assert_snapshot!` (e.g. `insta`).
/// See docs/elwindui_spec.md 付録V.1.
pub fn render_tree(root: &dyn Element) -> String {
    let mut out = String::new();
    write_node(root, 0, &mut out);
    out
}

fn write_node(node: &dyn Element, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    match node.id() {
        Some(id) => writeln!(out, "#{id}").unwrap(),
        None => writeln!(out, "-").unwrap(),
    }
    for child in node.children() {
        write_node(child, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    struct Leaf {
        id: Option<&'static str>,
    }

    impl Element for Leaf {
        fn children(&self) -> Vec<&dyn Element> {
            Vec::new()
        }

        fn id(&self) -> Option<&str> {
            self.id
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    struct Branch {
        children: Vec<Box<dyn Element>>,
    }

    impl Element for Branch {
        fn children(&self) -> Vec<&dyn Element> {
            self.children.iter().map(|c| c.as_ref()).collect()
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn dumps_nested_ids_with_indentation() {
        let tree = Branch {
            children: vec![
                Box::new(Leaf { id: Some("a") }),
                Box::new(Leaf { id: None }),
            ],
        };

        assert_eq!(render_tree(&tree), "-\n  #a\n  -\n");
    }
}
