#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::{
    ContentControlExt as _, ContentPresenter, ControlTemplate, ControlTemplateContext, TextBlock,
    TextBlockExt as _, UIElementExt as _,
};
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static DEFAULT_TEMPLATE_MOUNTS: Cell<u32> = const { Cell::new(0) };
    static TARGET_MOUNTS: Cell<u32> = const { Cell::new(0) };
}

fn record_default_template_mount() {
    DEFAULT_TEMPLATE_MOUNTS.with(|count| count.set(count.get() + 1));
}

#[elwindui::component(inherits ContentControl)]
struct DefaultBodyContentProbe {
    #[prop(default = "header".to_string())]
    label: String,

    body: view! {
        VerticalLayout {
            TextBlock { text: label }
        }
    },
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

#[elwindui::component(inherits ContentControl)]
struct DefaultBodyPresenterProbe {
    body: view! {
        VerticalLayout {
            TextBlock { text: "header" }
            ContentPresenter {}
        }
    },
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

    body: view! {
        if alternate {
            TextBlock { text: "alternate" }
        } else {
            TextBlock { text: "initial" }
        }
    },
}

#[elwindui::component]
impl DynamicDefaultBodyProbe {}

#[elwindui::component(inherits Control)]
struct DefaultTemplateProbe {
    body: view! {
        on_mount {
            record_default_template_mount();
        }
        VerticalLayout { }
    },
}

#[elwindui::component]
impl DefaultTemplateProbe {}

#[elwindui::environment_key(
    name = control_template_test_template,
    value = Option<ControlTemplate<ControlTemplateTestPanel>>,
    default = None
)]
pub struct ControlTemplateTestTemplate;

#[elwindui::component(
    inherits ContentControl,
    template = control_template_test_template
)]
struct ControlTemplateTestPanel {
    #[prop]
    label: String,

    body: view! {
        on_mount {
            TARGET_MOUNTS.with(|count| count.set(count.get() + 1));
        }
        VerticalLayout {
            DefaultTemplateProbe { }
            TextBlock { text: label }
            ContentPresenter { }
        }
    },
}

#[elwindui::component]
impl ControlTemplateTestPanel {}

#[elwindui::control_template(target = ControlTemplateTestPanel)]
struct CompactControlTemplateTestPanel {
    body: view! {
        VerticalLayout {
            TextBlock { text: templated_parent.label }
            ContentPresenter { }
        }
    },
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
fn environment_template_is_built_once_resyncs_and_presents_logical_content() {
    DEFAULT_TEMPLATE_MOUNTS.with(|count| count.set(0));
    TARGET_MOUNTS.with(|count| count.set(0));
    let environment = elwindui::core::environment::application_environment();
    let executions = Rc::new(Cell::new(0));
    let authored = CompactControlTemplateTestPanel::template();
    let executions_for_factory = executions.clone();
    let capturing = ControlTemplate::new(
        move |context: ControlTemplateContext<ControlTemplateTestPanel>| {
            executions_for_factory.set(executions_for_factory.get() + 1);
            authored.__build(context)
        },
    );
    environment.set::<ControlTemplateTestTemplate>(Some(capturing));

    let panel = ControlTemplateTestPanel::new("custom".to_string());
    assert_eq!(executions.get(), 1);
    assert_eq!(DEFAULT_TEMPLATE_MOUNTS.with(Cell::get), 0);
    assert_eq!(TARGET_MOUNTS.with(Cell::get), 1);
    assert_eq!(text_values(panel.as_ref()), vec!["custom"]);

    panel.set_label("updated".to_string());
    assert_eq!(
        executions.get(),
        1,
        "property changes must not rebuild the template"
    );
    assert_eq!(text_values(panel.as_ref()), vec!["updated"]);

    let content = TextBlock::new();
    content.set_text("logical content");
    panel.set_content(content.clone());

    let panel_node: Rc<dyn elwindui::core::ui::UIElementExt> = panel.clone();
    let logical_parent = content
        .as_ui_element()
        .parent
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .expect("content retains its logical ContentControl parent");
    assert!(Rc::ptr_eq(&logical_parent, &panel_node));
    assert!(content
        .visual_parent()
        .is_some_and(|parent| !Rc::ptr_eq(&parent, &panel_node)));

    let replacement = TextBlock::new();
    replacement.set_text("replacement");
    panel.set_content(replacement.clone());
    assert!(content.visual_parent().is_none());
    assert!(replacement.visual_parent().is_some());

    environment.set::<ControlTemplateTestTemplate>(None);
    let default_panel = ControlTemplateTestPanel::new("default".to_string());
    assert_eq!(DEFAULT_TEMPLATE_MOUNTS.with(Cell::get), 1);
    assert_eq!(TARGET_MOUNTS.with(Cell::get), 2);
    assert_eq!(text_values(default_panel.as_ref()), vec!["default"]);
}

#[test]
fn implicit_content_control_body_is_template_and_bare_child_is_logical_content() {
    let parent = DefaultBodyContentUseProbe::new();
    let probe = parent.probe();
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
fn implicit_content_control_body_presenter_owns_logical_content_visual() {
    let parent = DefaultBodyPresenterUseProbe::new();
    let probe = parent.probe();
    let logical = probe.content();

    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    let presenter = elwindui::core::visual_tree::find_all::<ContentPresenter>(probe.as_ref())
        .into_iter()
        .next()
        .expect("default body ContentPresenter");
    let presenter_node: Rc<dyn elwindui::core::ui::UIElementExt> = presenter.clone();
    assert!(logical
        .as_ui_element()
        .parent
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node)));
    assert!(logical
        .visual_parent()
        .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node)));

    let replacement = TextBlock::new();
    replacement.set_text("replacement");
    probe.set_content(replacement.clone());
    assert!(logical.visual_parent().is_none());
    assert!(replacement
        .visual_parent()
        .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node)));
}

#[test]
fn implicit_content_control_moves_pre_mount_content_to_logical_only_storage() {
    let probe = DefaultBodyContentProbe::__new_unmounted();
    let logical = TextBlock::new();
    logical.set_text("logical page");
    probe.set_content(logical.clone());
    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    assert!(logical
        .visual_parent()
        .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node)));

    probe.mount(elwindui::core::environment::application_environment());
    assert!(logical.visual_parent().is_none());
    assert_eq!(text_values(probe.as_ref()), vec!["header"]);
    assert!(logical
        .as_ui_element()
        .parent
        .borrow()
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .is_some_and(|parent| Rc::ptr_eq(&parent, &probe_node)));
}

#[test]
fn implicit_content_control_dynamic_body_replaces_template_root() {
    let probe = DynamicDefaultBodyProbe::__new_unmounted();
    let logical = TextBlock::new();
    logical.set_text("logical page");
    probe.set_content(logical.clone());
    probe.mount(elwindui::core::environment::application_environment());
    assert_eq!(text_values(probe.as_ref()), vec!["initial"]);
    assert_eq!(probe.visual_children().len(), 1);

    probe.set_alternate(true);
    assert_eq!(text_values(probe.as_ref()), vec!["alternate"]);
    assert_eq!(probe.visual_children().len(), 1);
    assert!(logical.visual_parent().is_none());
}
