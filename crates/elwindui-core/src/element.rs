use std::any::Any;

/// See docs/elwindui_spec.md §13 ("要素ツリーの探索").
pub trait Element: Any {
    fn children(&self) -> Vec<&dyn Element>;

    fn id(&self) -> Option<&str> {
        None
    }

    fn as_any(&self) -> &dyn Any;
}

pub fn find_by_id<'a>(root: &'a dyn Element, id: &str) -> Option<&'a dyn Element> {
    if root.id() == Some(id) {
        return Some(root);
    }
    root.children().into_iter().find_map(|c| find_by_id(c, id))
}

pub fn find_all<'a, T: 'static>(root: &'a dyn Element) -> Vec<&'a T> {
    let mut out = Vec::new();
    collect_all(root, &mut out);
    out
}

fn collect_all<'a, T: 'static>(root: &'a dyn Element, out: &mut Vec<&'a T>) {
    if let Some(t) = root.as_any().downcast_ref::<T>() {
        out.push(t);
    }
    for c in root.children() {
        collect_all(c, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        id: Option<&'static str>,
        children: Vec<Box<dyn Element>>,
    }

    impl Element for Branch {
        fn children(&self) -> Vec<&dyn Element> {
            self.children.iter().map(|c| c.as_ref()).collect()
        }

        fn id(&self) -> Option<&str> {
            self.id
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn find_by_id_recurses_into_children() {
        let tree = Branch {
            id: None,
            children: vec![
                Box::new(Leaf { id: Some("a") }),
                Box::new(Branch {
                    id: None,
                    children: vec![Box::new(Leaf { id: Some("b") })],
                }),
            ],
        };

        assert!(find_by_id(&tree, "a").is_some());
        assert!(find_by_id(&tree, "b").is_some());
        assert!(find_by_id(&tree, "missing").is_none());
    }

    #[test]
    fn find_all_collects_matching_type_across_tree() {
        let tree = Branch {
            id: None,
            children: vec![
                Box::new(Leaf { id: Some("a") }),
                Box::new(Branch {
                    id: None,
                    children: vec![Box::new(Leaf { id: Some("b") })],
                }),
            ],
        };

        let leaves: Vec<&Leaf> = find_all(&tree);
        assert_eq!(leaves.len(), 2);
    }
}
