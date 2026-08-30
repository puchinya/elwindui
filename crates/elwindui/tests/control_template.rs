#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::Size;
use elwindui::core::graphics::{RenderCommand, RenderGroup, RenderTree};
use elwindui::core::layout::Visibility;
use elwindui::core::ui::{
    ContentControlExt as _, ContentPresenter, ControlExt as _, ControlTemplate, TextBlock,
    TextBlockExt as _, UIElementExt as _, layout_root,
};
use elwindui::template_view;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

#[path = "control_template/local_template_base.rs"]
mod local_template_base;
pub use local_template_base::*;

#[path = "control_template/local_template_descendant.rs"]
mod local_template_descendant;

use local_template_descendant::{
    LocalTemplateDescendantProbe, LocalTemplateDescendantProbeExt as _,
};

thread_local! {
    static DEFAULT_TEMPLATE_MOUNTS: Cell<u32> = const { Cell::new(0) };
    static OVERRIDE_TEMPLATE_MOUNTS: Cell<u32> = const { Cell::new(0) };
    static TARGET_MOUNTS: Cell<u32> = const { Cell::new(0) };
    static VIRTUAL_MEASURE_EVENTS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static ATTACHMENT_PANIC_FACTORY_CALLS: Cell<u32> = const { Cell::new(0) };
    static ATTACHMENT_PANIC_MOUNT_HOOKS: Cell<u32> = const { Cell::new(0) };
    static ATTACHMENT_PANIC_ON_APPLY: Cell<u32> = const { Cell::new(0) };
    static ATTACHMENT_PANIC_ROOT: RefCell<Option<Rc<TextBlock>>> = const { RefCell::new(None) };
    static REENTRANT_FACTORY_CALLS: Cell<u32> = const { Cell::new(0) };
    static REENTRANT_INNER_RESULTS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    static REENTRANT_ON_APPLY: Cell<u32> = const { Cell::new(0) };
    static FACTORY_PANIC_FACTORY_CALLS: Cell<u32> = const { Cell::new(0) };
    static FACTORY_PANIC_ON_APPLY: Cell<u32> = const { Cell::new(0) };
    static POST_COMMIT_FACTORY_CALLS: Cell<u32> = const { Cell::new(0) };
    static POST_COMMIT_ON_APPLY: Cell<u32> = const { Cell::new(0) };
}

fn record_default_template_mount() {
    DEFAULT_TEMPLATE_MOUNTS.with(|count| count.set(count.get() + 1));
}

#[elwindui::component(inherits ContentControl)]
struct DefaultBodyContentProbe {
    #[prop(default = "header".to_string())]
    label: String,

    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            TextBlock { text: templated_parent.label }
        }
    }),
}

#[elwindui::component]
impl DefaultBodyContentProbe {}

#[elwindui::component(inherits VerticalLayout)]
struct DefaultBodyContentUseProbe {
    body: view! {
        #[id("probe")]
        let probe = DefaultBodyContentProbe {
            TextBlock { text: "logical page" }
        };

        probe
    },
}

#[elwindui::component]
impl DefaultBodyContentUseProbe {}

#[elwindui::component(inherits VerticalLayout)]
struct LocalTemplateUseProbe {
    body: view! {
        #[id("probe")]
        let probe = LocalTemplateDescendantProbe {
            TextBlock { text: "logical page" }
        };

        probe
    },
}

#[elwindui::component]
impl LocalTemplateUseProbe {}

#[elwindui::component(inherits VerticalLayout)]
struct LocalDynamicTemplateUseProbe {
    body: view! {
        #[id("probe")]
        let probe = LocalTemplateDescendantProbe {
            TextBlock { text: "logical page" }
        };

        probe
    },
}

#[elwindui::component]
impl LocalDynamicTemplateUseProbe {}

#[elwindui::component(inherits ContentControl)]
struct DefaultBodyPresenterProbe {
    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            TextBlock { text: "header" }
            ContentPresenter {}
        }
    }),
}

#[elwindui::component]
impl DefaultBodyPresenterProbe {}

#[elwindui::component(inherits VerticalLayout)]
struct DefaultBodyPresenterUseProbe {
    body: view! {
        #[id("probe")]
        let probe = DefaultBodyPresenterProbe {
            TextBlock { text: "logical page" }
        };

        probe
    },
}

#[elwindui::component]
impl DefaultBodyPresenterUseProbe {}

#[elwindui::component(inherits ContentControl)]
struct DynamicDefaultBodyProbe {
    #[prop(default = false)]
    alternate: bool,

    template: template_view!(|templated_parent: Self| {
        if alternate {
            TextBlock { text: "alternate" }
        } else {
            TextBlock { text: "initial" }
        }
    }),
}

#[elwindui::component]
impl DynamicDefaultBodyProbe {}

#[elwindui::component(inherits Control)]
struct DefaultTemplateProbe {
    template: template_view!(|templated_parent: Self| {
        on_mount {
            record_default_template_mount();
        }
        VerticalLayout { }
    }),
}

#[elwindui::component]
impl DefaultTemplateProbe {}

#[elwindui::component(inherits Control)]
struct VirtualApplyMeasureProbe {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "template root",
        }
    }),
}

#[elwindui::component]
impl VirtualApplyMeasureProbe {
    #[overrides]
    fn on_apply_template(&self) {
        VIRTUAL_MEASURE_EVENTS.with(|events| events.borrow_mut().push("on_apply_template"));
    }

    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let event = if self.visual_children().len() == 1 {
            "measure_override_saw_template_root"
        } else {
            "measure_override_ran_before_template"
        };
        VIRTUAL_MEASURE_EVENTS.with(|events| events.borrow_mut().push(event));
        available
    }
}

#[elwindui::component(inherits Control)]
struct AttachmentPanicProbe {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "default attachment panic root",
        }
    }),
}

#[elwindui::component]
impl AttachmentPanicProbe {
    #[overrides]
    fn on_apply_template(&self) {
        ATTACHMENT_PANIC_ON_APPLY.with(|count| count.set(count.get() + 1));
    }
}

fn attachment_panic_template() -> ControlTemplate<AttachmentPanicProbe> {
    ControlTemplate::new(|_context| {
        ATTACHMENT_PANIC_FACTORY_CALLS.with(|count| count.set(count.get() + 1));
        let root = TextBlock::new();
        root.add_mount_hook(Box::new(|| {
            ATTACHMENT_PANIC_MOUNT_HOOKS.with(|count| count.set(count.get() + 1));
            panic!("attachment-mount-panic-sentinel");
        }));
        ATTACHMENT_PANIC_ROOT.with(|slot| *slot.borrow_mut() = Some(root.clone()));
        root
    })
}

#[elwindui::component(inherits Control)]
struct ReentrantApplyProbe {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "default reentrant root",
        }
    }),
}

#[elwindui::component]
impl ReentrantApplyProbe {
    #[overrides]
    fn on_apply_template(&self) {
        REENTRANT_ON_APPLY.with(|count| count.set(count.get() + 1));
    }
}

fn reentrant_template() -> ControlTemplate<ReentrantApplyProbe> {
    ControlTemplate::<ReentrantApplyProbe>::new(|context| {
        REENTRANT_FACTORY_CALLS.with(|count| count.set(count.get() + 1));
        let inner_result = context.control.apply_template();
        REENTRANT_INNER_RESULTS.with(|results| results.borrow_mut().push(inner_result));
        TextBlock::new()
    })
}

#[elwindui::component(inherits Control)]
struct FactoryPanicProbe {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "default factory panic root",
        }
    }),
}

#[elwindui::component]
impl FactoryPanicProbe {
    #[overrides]
    fn on_apply_template(&self) {
        FACTORY_PANIC_ON_APPLY.with(|count| count.set(count.get() + 1));
    }
}

fn factory_panic_template() -> ControlTemplate<FactoryPanicProbe> {
    ControlTemplate::new(|_context| {
        FACTORY_PANIC_FACTORY_CALLS.with(|count| count.set(count.get() + 1));
        panic!("factory-panic-sentinel");
    })
}

#[elwindui::component(inherits Control)]
struct PostCommitPanicProbe {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "default post-commit root",
        }
    }),
}

#[elwindui::component]
impl PostCommitPanicProbe {
    #[overrides]
    fn on_apply_template(&self) {
        POST_COMMIT_ON_APPLY.with(|count| count.set(count.get() + 1));
        panic!("post-commit-panic-sentinel");
    }
}

fn post_commit_template() -> ControlTemplate<PostCommitPanicProbe> {
    ControlTemplate::new(|_context| {
        POST_COMMIT_FACTORY_CALLS.with(|count| count.set(count.get() + 1));
        TextBlock::new()
    })
}

#[elwindui::component(inherits ContentControl)]
struct ControlTemplateTestPanel {
    #[prop]
    label: String,

    template: template_view!(|templated_parent: Self| {
        on_mount {
            TARGET_MOUNTS.with(|count| count.set(count.get() + 1));
        }
        VerticalLayout {
            DefaultTemplateProbe { }
            TextBlock { text: label }
            ContentPresenter { }
        }
    }),
}

#[elwindui::component]
impl ControlTemplateTestPanel {}

#[elwindui::component(inherits VerticalLayout)]
struct NamedContentUseProbe {
    #[param]
    logical_content: Rc<TextBlock>,

    body: view! {
        #[id("panel")]
        let panel = ControlTemplateTestPanel {
            label: "nested"
            content: logical_content
        };

        panel
    },
}

#[elwindui::component]
impl NamedContentUseProbe {}

fn compact_control_template_test_panel(
    prefix: String,
) -> ControlTemplate<ControlTemplateTestPanel> {
    template_view!(|panel: ControlTemplateTestPanel| {
        on_mount {
            OVERRIDE_TEMPLATE_MOUNTS.with(|count| count.set(count.get() + 1));
        }
        VerticalLayout {
            TextBlock {
                text: format!("{}{}", prefix, panel.label)
            }
            ContentPresenter { }
        }
    })
}

fn text_values(root: &dyn elwindui::core::ui::UIElementExt) -> Vec<String> {
    elwindui::core::visual_tree::find_all::<TextBlock>(root)
        .into_iter()
        .map(|node| {
            node.as_any()
                .downcast_ref::<TextBlock>()
                .expect("find_all returned the requested concrete type")
                .text
                .borrow()
                .clone()
        })
        .collect()
}

#[test]
fn participating_measure_dispatches_virtual_template_application_before_override() {
    VIRTUAL_MEASURE_EVENTS.with(|events| events.borrow_mut().clear());
    let probe = VirtualApplyMeasureProbe::new();
    assert!(probe.visual_children().is_empty());

    let root: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    layout_root(
        &root,
        Size {
            width: 120.0,
            height: 80.0,
        },
    );

    assert_eq!(
        VIRTUAL_MEASURE_EVENTS.with(|events| events.borrow().clone()),
        vec!["on_apply_template", "measure_override_saw_template_root"],
        "UIElement::measure must dispatch the virtual apply_template override before the derived measure_override"
    );
}

#[test]
fn non_control_ui_element_uses_default_false_template_application() {
    let text = TextBlock::new();
    text.set_text("plain element");

    assert!(!text.apply_template());
    text.measure(Size {
        width: 120.0,
        height: 80.0,
    });

    assert!(text.measured_size().is_some());
    assert!(text.visual_children().is_empty());
}

#[test]
fn attachment_mount_hook_panic_rolls_back_failed_template_root() {
    ATTACHMENT_PANIC_FACTORY_CALLS.with(|count| count.set(0));
    ATTACHMENT_PANIC_MOUNT_HOOKS.with(|count| count.set(0));
    ATTACHMENT_PANIC_ON_APPLY.with(|count| count.set(0));
    ATTACHMENT_PANIC_ROOT.with(|slot| *slot.borrow_mut() = None);

    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template::<AttachmentPanicProbe>(Some(attachment_panic_template()));
    let probe = AttachmentPanicProbe::new();

    let panic = catch_unwind(AssertUnwindSafe(|| probe.apply_template()))
        .expect_err("a root attachment mount hook must panic");
    assert_eq!(
        *panic
            .downcast_ref::<&'static str>()
            .expect("sentinel panic payload is a static string"),
        "attachment-mount-panic-sentinel"
    );
    assert_eq!(ATTACHMENT_PANIC_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(ATTACHMENT_PANIC_MOUNT_HOOKS.with(Cell::get), 1);
    assert_eq!(ATTACHMENT_PANIC_ON_APPLY.with(Cell::get), 0);

    let failed_root = ATTACHMENT_PANIC_ROOT
        .with(|slot| slot.borrow().clone())
        .expect("the factory retained the failed root for structural assertions");
    assert!(
        probe.visual_children().is_empty(),
        "a failed root must not remain in Control.visual_children"
    );
    assert!(
        probe.__template_root().is_none(),
        "a failed root must not remain in Control.template_root"
    );
    assert!(
        failed_root.visual_parent().is_none(),
        "a failed root must not retain the Control as visual parent"
    );

    assert!(!probe.apply_template());
    assert_eq!(ATTACHMENT_PANIC_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(ATTACHMENT_PANIC_MOUNT_HOOKS.with(Cell::get), 1);
    assert_eq!(ATTACHMENT_PANIC_ON_APPLY.with(Cell::get), 0);
    environment.set_control_template::<AttachmentPanicProbe>(None);
}

#[test]
fn reentrant_apply_returns_false_without_double_building() {
    REENTRANT_FACTORY_CALLS.with(|count| count.set(0));
    REENTRANT_INNER_RESULTS.with(|results| results.borrow_mut().clear());
    REENTRANT_ON_APPLY.with(|count| count.set(0));

    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template::<ReentrantApplyProbe>(Some(reentrant_template()));
    let probe = ReentrantApplyProbe::new();

    assert!(probe.apply_template());
    assert_eq!(REENTRANT_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(
        REENTRANT_INNER_RESULTS.with(|results| results.borrow().clone()),
        vec![false]
    );
    assert_eq!(probe.visual_children().len(), 1);
    assert!(probe.__template_root().is_some());
    assert_eq!(REENTRANT_ON_APPLY.with(Cell::get), 1);

    assert!(!probe.apply_template());
    assert_eq!(REENTRANT_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(REENTRANT_ON_APPLY.with(Cell::get), 1);
    environment.set_control_template::<ReentrantApplyProbe>(None);
}

#[test]
fn factory_panic_is_terminal_before_root_commit() {
    FACTORY_PANIC_FACTORY_CALLS.with(|count| count.set(0));
    FACTORY_PANIC_ON_APPLY.with(|count| count.set(0));

    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template::<FactoryPanicProbe>(Some(factory_panic_template()));
    let probe = FactoryPanicProbe::new();

    let panic = catch_unwind(AssertUnwindSafe(|| probe.apply_template()))
        .expect_err("the selected template factory must panic");
    assert_eq!(
        *panic
            .downcast_ref::<&'static str>()
            .expect("sentinel panic payload is a static string"),
        "factory-panic-sentinel"
    );
    assert_eq!(FACTORY_PANIC_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(FACTORY_PANIC_ON_APPLY.with(Cell::get), 0);
    assert!(probe.visual_children().is_empty());
    assert!(probe.__template_root().is_none());

    assert!(!probe.apply_template());
    assert_eq!(FACTORY_PANIC_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(FACTORY_PANIC_ON_APPLY.with(Cell::get), 0);
    environment.set_control_template::<FactoryPanicProbe>(None);
}

#[test]
fn on_apply_template_panic_keeps_committed_root() {
    POST_COMMIT_FACTORY_CALLS.with(|count| count.set(0));
    POST_COMMIT_ON_APPLY.with(|count| count.set(0));

    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template::<PostCommitPanicProbe>(Some(post_commit_template()));
    let probe = PostCommitPanicProbe::new();

    let panic = catch_unwind(AssertUnwindSafe(|| probe.apply_template()))
        .expect_err("on_apply_template must panic after root commit");
    assert_eq!(
        *panic
            .downcast_ref::<&'static str>()
            .expect("sentinel panic payload is a static string"),
        "post-commit-panic-sentinel"
    );
    assert_eq!(POST_COMMIT_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(POST_COMMIT_ON_APPLY.with(Cell::get), 1);
    assert_eq!(probe.visual_children().len(), 1);
    let root = probe
        .visual_children()
        .into_iter()
        .next()
        .expect("committed root");
    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    assert!(
        probe.__template_root().is_some(),
        "post-commit hook panic must retain template_root"
    );
    assert!(
        root.visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node)),
        "post-commit hook panic must retain the Visual parent"
    );

    assert!(!probe.apply_template());
    assert_eq!(POST_COMMIT_FACTORY_CALLS.with(Cell::get), 1);
    assert_eq!(POST_COMMIT_ON_APPLY.with(Cell::get), 1);
    environment.set_control_template::<PostCommitPanicProbe>(None);
}

#[test]
fn collapsed_control_does_not_apply_template_until_participating_measure() {
    VIRTUAL_MEASURE_EVENTS.with(|events| events.borrow_mut().clear());
    let probe = VirtualApplyMeasureProbe::new();
    probe.set_visibility(Visibility::Collapsed);
    assert_eq!(probe.visibility(), Visibility::Collapsed);
    let root: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();

    layout_root(
        &root,
        Size {
            width: 120.0,
            height: 80.0,
        },
    );

    assert!(VIRTUAL_MEASURE_EVENTS.with(|events| events.borrow().is_empty()));
    assert!(probe.visual_children().is_empty());

    probe.set_visibility(Visibility::Visible);
    layout_root(
        &root,
        Size {
            width: 120.0,
            height: 80.0,
        },
    );

    assert_eq!(
        VIRTUAL_MEASURE_EVENTS.with(|events| events.borrow().clone()),
        vec!["on_apply_template", "measure_override_saw_template_root"]
    );
    assert_eq!(probe.visual_children().len(), 1);
}

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

#[test]
fn environment_template_resyncs_and_presents_logical_content() {
    DEFAULT_TEMPLATE_MOUNTS.with(|count| count.set(0));
    OVERRIDE_TEMPLATE_MOUNTS.with(|count| count.set(0));
    TARGET_MOUNTS.with(|count| count.set(0));
    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template(Some(compact_control_template_test_panel(
        "Override: ".to_string(),
    )));

    let panel = elwindui::new!(ControlTemplateTestPanel(label: "custom".to_string()));
    assert_eq!(DEFAULT_TEMPLATE_MOUNTS.with(Cell::get), 0);
    assert_eq!(OVERRIDE_TEMPLATE_MOUNTS.with(Cell::get), 0);
    assert_eq!(TARGET_MOUNTS.with(Cell::get), 1);
    assert!(text_values(panel.as_ref()).is_empty());
    assert!(panel.apply_template());
    assert_eq!(OVERRIDE_TEMPLATE_MOUNTS.with(Cell::get), 1);
    assert_eq!(text_values(panel.as_ref()), vec!["Override: custom"]);

    panel.set_label("updated".to_string());
    assert_eq!(OVERRIDE_TEMPLATE_MOUNTS.with(Cell::get), 1);
    assert_eq!(TARGET_MOUNTS.with(Cell::get), 1);
    assert_eq!(text_values(panel.as_ref()), vec!["Override: updated"]);

    let content = TextBlock::new();
    content.set_text("logical content");
    panel.set_content(content.clone());

    let panel_node: Rc<dyn elwindui::core::ui::UIElementExt> = panel.clone();
    assert_eq!(panel.visual_children().len(), 1);
    let presenter = elwindui::core::visual_tree::find_all::<ContentPresenter>(panel.as_ref())
        .into_iter()
        .next()
        .expect("override template ContentPresenter");
    let presenter_node: Rc<dyn elwindui::core::ui::UIElementExt> = presenter.clone();
    let logical_parent = content
        .as_ui_element()
        .parent
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .expect("content retains its logical ContentControl parent");
    assert!(Rc::ptr_eq(&logical_parent, &panel_node));
    assert!(
        content
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node))
    );

    let replacement = TextBlock::new();
    replacement.set_text("replacement");
    panel.set_content(replacement.clone());
    assert_eq!(OVERRIDE_TEMPLATE_MOUNTS.with(Cell::get), 1);
    assert!(content.visual_parent().is_none());
    assert!(
        replacement
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node))
    );

    let mounted_values_before_environment_change = text_values(panel.as_ref());
    environment.set_control_template::<ControlTemplateTestPanel>(None);
    assert_eq!(
        text_values(panel.as_ref()),
        mounted_values_before_environment_change,
        "changing the Environment slot must not re-template an already-mounted panel"
    );
    assert_eq!(OVERRIDE_TEMPLATE_MOUNTS.with(Cell::get), 1);
    let default_panel = elwindui::new!(ControlTemplateTestPanel(label: "default".to_string()));
    assert_eq!(DEFAULT_TEMPLATE_MOUNTS.with(Cell::get), 0);
    assert_eq!(OVERRIDE_TEMPLATE_MOUNTS.with(Cell::get), 1);
    assert_eq!(TARGET_MOUNTS.with(Cell::get), 2);
    assert!(text_values(default_panel.as_ref()).is_empty());
    assert!(default_panel.apply_template());
    assert_eq!(DEFAULT_TEMPLATE_MOUNTS.with(Cell::get), 1);
    assert_eq!(text_values(default_panel.as_ref()), vec!["default"]);
}

#[test]
fn default_template_root_lays_out_and_reaches_render_tree() {
    DEFAULT_TEMPLATE_MOUNTS.with(|count| count.set(0));
    TARGET_MOUNTS.with(|count| count.set(0));
    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template::<ControlTemplateTestPanel>(None);

    let panel = elwindui::new!(ControlTemplateTestPanel(label: "default".to_string()));
    let root: Rc<dyn elwindui::core::ui::UIElementExt> = panel.clone();
    assert!(root.visual_children().is_empty());
    assert_eq!(DEFAULT_TEMPLATE_MOUNTS.with(Cell::get), 0);
    assert_eq!(TARGET_MOUNTS.with(Cell::get), 1);
    assert!(text_values(panel.as_ref()).is_empty());

    layout_root(
        &root,
        Size {
            width: 520.0,
            height: 260.0,
        },
    );

    let template_root = root
        .visual_children()
        .into_iter()
        .next()
        .expect("default template root is attached exactly once");
    assert_eq!(root.visual_children().len(), 1);
    assert!(
        template_root
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &root))
    );
    assert_eq!(DEFAULT_TEMPLATE_MOUNTS.with(Cell::get), 1);
    assert_eq!(TARGET_MOUNTS.with(Cell::get), 1);
    assert!(text_values(panel.as_ref()).contains(&"default".to_string()));

    assert!(
        root.measured_size()
            .is_some_and(|size| size.width > 0.0 && size.height > 0.0)
    );
    assert!(root.arranged_width().is_some_and(|width| width > 0.0));
    assert!(root.arranged_height().is_some_and(|height| height > 0.0));
    assert!(
        template_root
            .measured_size()
            .is_some_and(|size| size.width > 0.0 && size.height > 0.0)
    );
    assert!(
        template_root
            .arranged_width()
            .is_some_and(|width| width > 0.0)
    );
    assert!(
        template_root
            .arranged_height()
            .is_some_and(|height| height > 0.0)
    );

    let render_tree = RenderTree::new::<()>(&root);
    assert_eq!(render_tree.root_id(), root.render_group_id());
    assert_eq!(render_tree.root.children.len(), 1);
    let mut rendered_texts = Vec::new();
    render_tree_texts(&render_tree.root, &mut rendered_texts);
    assert!(rendered_texts.iter().any(|text| text == "default"));
    assert!(
        render_tree
            .group_paths
            .contains_key(&template_root.render_group_id())
    );
    assert!(
        render_tree
            .visual_index
            .contains_key(&template_root.render_group_id())
    );
}

#[test]
fn named_inherited_content_is_bound_before_template_application() {
    DEFAULT_TEMPLATE_MOUNTS.with(|count| count.set(0));
    TARGET_MOUNTS.with(|count| count.set(0));
    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template(Some(compact_control_template_test_panel(
        "Override: ".to_string(),
    )));

    let logical = TextBlock::new();
    logical.set_text("pre-mount logical content");
    let host = NamedContentUseProbe::new(logical.clone());
    let panel = host.panel();
    assert!(panel.apply_template());
    let panel_node: Rc<dyn elwindui::core::ui::UIElementExt> = panel.clone();
    let presenter = elwindui::core::visual_tree::find_all::<ContentPresenter>(panel.as_ref())
        .into_iter()
        .next()
        .expect("named content reaches the selected template presenter");
    let presenter_node: Rc<dyn elwindui::core::ui::UIElementExt> = presenter.clone();

    let logical_parent = logical
        .as_ui_element()
        .parent
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .expect("named content retains its logical ContentControl parent");
    assert!(Rc::ptr_eq(&logical_parent, &panel_node));
    assert!(
        logical
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node))
    );
    assert_eq!(
        text_values(panel.as_ref()),
        vec!["Override: nested", "pre-mount logical content"]
    );
}

#[test]
fn default_template_is_separate_from_bare_logical_content() {
    let parent = DefaultBodyContentUseProbe::new();
    let probe = parent.probe();
    assert!(probe.apply_template());
    let logical = probe.content();
    let logical_text = logical
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("bare content is the authored TextBlock")
        .text
        .borrow()
        .clone();

    assert_eq!(logical_text, "logical page");
    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    let logical_node: Rc<dyn elwindui::core::ui::UIElementExt> = logical.clone();
    let logical_parent = logical
        .as_ui_element()
        .parent
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .expect("logical content retains its ContentControl parent");
    assert!(Rc::ptr_eq(&logical_parent, &probe_node));
    assert!(logical.visual_parent().is_none());
    assert_eq!(text_values(probe.as_ref()), vec!["header"]);
    assert!(!text_values(probe.as_ref()).contains(&logical_text));
    assert!(logical_node.visual_parent().is_none());
}

#[test]
fn same_crate_multi_hop_dynamic_template_replaces_only_the_template_root() {
    let parent = LocalDynamicTemplateUseProbe::new();
    let probe = parent.probe();
    assert!(probe.apply_template());
    let logical = probe.content();
    let logical_node: Rc<dyn elwindui::core::ui::UIElementExt> = logical.clone();
    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    let logical_parent = logical
        .as_ui_element()
        .parent
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .expect("logical content keeps its derived ContentControl parent");
    assert!(Rc::ptr_eq(&logical_parent, &probe_node));
    assert!(logical_node.visual_parent().is_none());

    let initial_root = probe.visual_children()[0].clone();
    assert_eq!(
        initial_root
            .as_any()
            .downcast_ref::<TextBlock>()
            .expect("initial derived template root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "derived initial"
    );

    probe.set_show_alternate(true);

    let visual_children = probe.visual_children();
    assert_eq!(visual_children.len(), 1);
    let replacement_root = visual_children[0].clone();
    assert!(!Rc::ptr_eq(&initial_root, &replacement_root));
    assert!(initial_root.visual_parent().is_none());
    assert!(
        replacement_root
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node))
    );
    assert_eq!(
        replacement_root
            .as_any()
            .downcast_ref::<TextBlock>()
            .expect("replacement derived template root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "derived alternate"
    );
    assert!(logical_node.visual_parent().is_none());
    assert!(
        logical
            .as_ui_element()
            .parent
            .borrow()
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
            .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node))
    );
}

#[test]
fn default_template_content_presenter_owns_logical_content_visual() {
    let parent = DefaultBodyPresenterUseProbe::new();
    let probe = parent.probe();
    assert!(probe.apply_template());
    let logical = probe.content();

    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    let presenter = elwindui::core::visual_tree::find_all::<ContentPresenter>(probe.as_ref())
        .into_iter()
        .next()
        .expect("default body ContentPresenter");
    let presenter_node: Rc<dyn elwindui::core::ui::UIElementExt> = presenter.clone();
    assert!(
        logical
            .as_ui_element()
            .parent
            .borrow()
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
            .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node))
    );
    assert!(
        logical
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node))
    );

    let replacement = TextBlock::new();
    replacement.set_text("replacement");
    probe.set_content(replacement.clone());
    assert!(logical.visual_parent().is_none());
    assert!(
        replacement
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node))
    );
}

#[test]
fn local_multi_hop_content_control_uses_explicit_template_root() {
    let parent = LocalTemplateUseProbe::new();
    let probe = parent.probe();
    let logical = probe.content();

    assert!(probe.apply_template());
    assert_eq!(text_values(probe.as_ref()), vec!["derived initial"]);
    assert_eq!(
        logical
            .as_any()
            .downcast_ref::<TextBlock>()
            .expect("bare content is the authored TextBlock")
            .text
            .borrow()
            .as_str(),
        "logical page"
    );
    assert!(logical.visual_parent().is_none());
}

#[test]
fn template_application_moves_pre_mount_content_to_logical_only_storage() {
    let probe = DefaultBodyContentProbe::__new_unmounted();
    let logical = TextBlock::new();
    logical.set_text("logical page");
    probe.set_content(logical.clone());
    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    assert!(
        logical
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node))
    );

    probe.mount(elwindui::core::environment::application_environment());
    assert!(logical.visual_parent().is_some());
    assert_eq!(text_values(probe.as_ref()), vec!["logical page"]);
    assert!(probe.apply_template());
    assert!(logical.visual_parent().is_none());
    assert_eq!(text_values(probe.as_ref()), vec!["header"]);
    assert!(
        logical
            .as_ui_element()
            .parent
            .borrow()
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
            .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node))
    );
}

#[test]
fn dynamic_template_replaces_template_root() {
    let probe = DynamicDefaultBodyProbe::__new_unmounted();
    let logical = TextBlock::new();
    logical.set_text("logical page");
    probe.set_content(logical.clone());
    probe.mount(elwindui::core::environment::application_environment());
    assert_eq!(text_values(probe.as_ref()), vec!["logical page"]);
    assert!(probe.apply_template());
    assert_eq!(text_values(probe.as_ref()), vec!["initial"]);
    assert_eq!(probe.visual_children().len(), 1);

    probe.set_alternate(true);
    assert_eq!(text_values(probe.as_ref()), vec!["alternate"]);
    assert_eq!(probe.visual_children().len(), 1);
    assert!(logical.visual_parent().is_none());
}
