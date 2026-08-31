#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[cfg(all(target_os = "macos", feature = "backend-appkit"))]
mod macos_rt4 {
    use elwindui::core::graphics::{RenderCommand, RenderGroup, RenderTree};
    use elwindui::core::ui::{
        ContentPresenter, ControlTemplate, TextBlock, TextBlockExt as _, UIElementExt as _,
        WindowExt,
    };
    use elwindui::template_view;
    use std::cell::Cell;
    use std::future::poll_fn;
    use std::rc::Rc;
    use std::task::Poll;
    use std::time::Duration;

    #[elwindui::component(inherits ContentControl)]
    struct Rt4Panel {
        #[prop]
        label: String,

        template: template_view!(|panel: Self| {
            VerticalLayout {
                TextBlock { text: "Default RT4 template" }
                TextBlock { text: panel.label }
                ContentPresenter { }
            }
        }),
    }

    #[elwindui::component]
    impl Rt4Panel {}

    fn rt4_override_template(prefix: String) -> ControlTemplate<Rt4Panel> {
        template_view!(|panel: Rt4Panel| {
            VerticalLayout {
                TextBlock {
                    text: format!("{} Environment override", prefix)
                }
                TextBlock { text: panel.label }
                ContentPresenter { }
            }
        })
    }

    #[elwindui::component(inherits Window)]
    struct Rt4Window {
        #[param]
        logical_content: Rc<TextBlock>,

        body: view! {
            title: "elwindui ControlTemplate RT4"
            width: 520.0
            height: 260.0
            content: Rt4Panel {
                label: "RT4 reactive alias"
                content: logical_content
            }
        },
    }

    #[elwindui::component]
    impl Rt4Window {}

    fn render_tree_texts(group: &RenderGroup, texts: &mut Vec<String>) {
        for command in &group.commands {
            if let RenderCommand::Text { content, .. } = command {
                texts.push(content.clone());
            }
        }
        for child in &group.children {
            render_tree_texts(child, texts);
        }
    }

    fn defer_to_next_appkit_turn(callback: impl FnOnce() + 'static) {
        let started = Rc::new(Cell::new(false));
        let started_on_poll = started.clone();
        let mut callback = Some(callback);
        elwindui::core::task::spawn_local(poll_fn(move |context| {
            if !started_on_poll.replace(true) {
                let waker = context.waker().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(50));
                    waker.wake();
                });
                return Poll::Pending;
            }

            callback
                .take()
                .expect("deferred callback is polled only once after it is scheduled")(
            );
            Poll::Ready(())
        }));
    }

    pub fn run() {
        let environment = elwindui::core::environment::application_environment();
        environment.set_control_template(Some(rt4_override_template("RT4".to_string())));

        let logical_content = TextBlock::new();
        logical_content.set_text("RT4 logical ContentPresenter content");
        let window = Rt4Window::new(logical_content.clone());

        window.show();

        defer_to_next_appkit_turn(move || {
            let window_content = WindowExt::content_element(&*window)
                .expect("shown Window exposes its mounted content through the public API");
            let panel = window_content
                .as_any()
                .downcast_ref::<Rt4Panel>()
                .expect("Window content is the templated RT4 panel");
            let panel_node: Rc<dyn elwindui::core::ui::UIElementExt> = window_content.clone();

            let template_children = panel.visual_children();
            assert_eq!(
                template_children.len(),
                1,
                "the target has exactly one active template root"
            );
            let template_root = template_children
                .first()
                .expect("template root exists")
                .clone();
            assert!(
                template_root
                    .visual_parent()
                    .is_some_and(|parent| Rc::ptr_eq(&parent, &panel_node)),
                "the template root is visually owned by the target"
            );

            assert!(
                panel
                    .measured_size()
                    .is_some_and(|size| size.width > 0.0 && size.height > 0.0),
                "the Window-hosted target has non-zero measured layout"
            );
            let target_arranged_width = panel.arranged_width();
            let target_arranged_height = panel.arranged_height();
            assert!(
                target_arranged_width.is_some_and(|width| width > 0.0)
                    && target_arranged_height.is_some_and(|height| height > 0.0),
                "the Window-hosted target has non-zero arranged layout: measured={:?}, arranged=({:?}, {:?})",
                panel.measured_size(),
                target_arranged_width,
                target_arranged_height
            );
            assert!(
                template_root
                    .measured_size()
                    .is_some_and(|size| size.width > 0.0 && size.height > 0.0),
                "the Window-hosted template root has non-zero measured layout"
            );
            assert!(
                template_root
                    .arranged_width()
                    .is_some_and(|width| width > 0.0)
                    && template_root
                        .arranged_height()
                        .is_some_and(|height| height > 0.0),
                "the Window-hosted template root has non-zero arranged layout"
            );

            let presenter = elwindui::core::visual_tree::find_all::<ContentPresenter>(panel)
                .into_iter()
                .next()
                .expect("the selected template contains a ContentPresenter");
            let presenter_node: Rc<dyn elwindui::core::ui::UIElementExt> = presenter.clone();
            let logical_parent = logical_content
                .as_ui_element()
                .parent
                .borrow()
                .as_ref()
                .and_then(std::rc::Weak::upgrade)
                .expect("logical content retains its target ContentControl parent");
            assert!(
                Rc::ptr_eq(&logical_parent, &panel_node),
                "logical content remains owned by the target"
            );
            assert!(
                logical_content
                    .visual_parent()
                    .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node)),
                "logical content is visually presented by ContentPresenter"
            );

            let render_tree = RenderTree::new::<()>(&window_content);
            let mut rendered_texts = Vec::new();
            render_tree_texts(&render_tree.root, &mut rendered_texts);
            assert!(
                rendered_texts
                    .iter()
                    .any(|text| text == "RT4 Environment override")
            );
            assert!(
                rendered_texts
                    .iter()
                    .any(|text| text == "RT4 reactive alias")
            );
            assert!(
                rendered_texts
                    .iter()
                    .any(|text| text == "RT4 logical ContentPresenter content")
            );
            assert!(
                !rendered_texts
                    .iter()
                    .any(|text| text == "Default RT4 template")
            );

            assert!(
                render_tree
                    .group_paths
                    .contains_key(&template_root.render_group_id()),
                "RenderTree contains the Window-hosted template root"
            );
            assert!(
                render_tree
                    .visual_index
                    .contains_key(&template_root.render_group_id()),
                "RenderTree indexes the Window-hosted template root"
            );

            println!(
                "RT4 diagnostics: window_size=({}, {}), target_measured={:?}, target_arranged=({:?}, {:?}), template_root_measured={:?}, template_root_arranged=({:?}, {:?}), render_tree_texts={rendered_texts:?}",
                WindowExt::width(&*window),
                WindowExt::height(&*window),
                panel.measured_size(),
                panel.arranged_width(),
                panel.arranged_height(),
                template_root.measured_size(),
                template_root.arranged_width(),
                template_root.arranged_height(),
            );
            window.close();
        });
    }
}

#[cfg(all(target_os = "macos", feature = "backend-appkit"))]
#[elwindui::main]
fn main() {
    macos_rt4::run();
}

#[cfg(not(all(target_os = "macos", feature = "backend-appkit")))]
fn main() {}
