#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::{Point, Rect, Size};
use elwindui::core::graphics::{Brush, RenderCommand, RenderContext, RenderTree};
use elwindui::core::ui::{UIElementExt, hit_test, layout_root};
use std::rc::Rc;

#[elwindui::component(inherits Control)]
struct OverrideProbe {
    #[prop(default = false)]
    arranged_marker: bool,
    template: template_view!(|templated_parent: Self| { Rectangle { fill: "#00000000" } }),
}

#[elwindui::component]
impl OverrideProbe {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        !base::hit_test_content()
    }

    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let base = base::measure_override(available);
        Size {
            width: base.width + 17.0,
            height: base.height + 19.0,
        }
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        self.set_arranged_marker(true);
        for child in self.visual_children() {
            child.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            });
        }
        final_size
    }

    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let brush: Brush = "#ff00ff".into();
        context.fill_rect(
            Rect {
                x: 7.0,
                y: 8.0,
                width: 9.0,
                height: 10.0,
            },
            &brush,
        );
    }
}

fn new_probe_root() -> (Rc<OverrideProbe>, Rc<dyn UIElementExt>) {
    let probe = OverrideProbe::new();
    let root: Rc<dyn UIElementExt> = probe.clone();
    (probe, root)
}

#[test]
fn component_hit_test_override_reaches_ui_element_trait_path() {
    let (_probe, root) = new_probe_root();
    root.measure(Size {
        width: 100.0,
        height: 100.0,
    });
    assert_eq!(
        root.measured_size(),
        Some(Size {
            width: 17.0,
            height: 19.0
        })
    );

    let child = root
        .visual_children()
        .into_iter()
        .next()
        .expect("probe body child");
    child.set_hit_test_visible(false);
    layout_root(
        &root,
        Size {
            width: 100.0,
            height: 100.0,
        },
    );
    let hit = hit_test(&root, Point { x: 10.0, y: 10.0 }).expect("probe should be hit-testable");
    assert!(Rc::ptr_eq(&hit, &root));
}

#[test]
fn component_measure_and_arrange_overrides_reach_layout_root() {
    let (probe, root) = new_probe_root();
    root.measure(Size {
        width: 100.0,
        height: 100.0,
    });
    assert_eq!(
        root.measured_size(),
        Some(Size {
            width: 17.0,
            height: 19.0
        })
    );
    layout_root(
        &root,
        Size {
            width: 100.0,
            height: 100.0,
        },
    );
    assert!(probe.arranged_marker());
}

#[test]
fn component_render_override_reaches_render_tree() {
    let (_probe, root) = new_probe_root();
    let render_tree = RenderTree::new::<()>(&root);
    assert!(render_tree.root.commands.iter().any(|command| {
        matches!(
            command,
            RenderCommand::FillRect { rect, .. }
                if *rect == Rect { x: 7.0, y: 8.0, width: 9.0, height: 10.0 }
        )
    }));
}
