#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::environment::EnvironmentContext;
use elwindui::core::ui::ControlTemplate;
use elwindui::{component, template_view};
use std::rc::Rc;

#[elwindui::environment_key(
    name = standalone_template_environment_text,
    value = String,
    default = String::from("application")
)]
pub struct StandaloneTemplateEnvironmentText;

#[component(inherits Control)]
struct TemplateProbe {
    #[prop]
    label: String,
    template: template_view! {
        TextBlock { text: templated_parent.label }
    },
}

#[component]
impl TemplateProbe {}

#[component(inherits ContentControl)]
struct DynamicTemplateProbe {
    #[prop(default = false)]
    alternate: bool,
    #[prop(default = Vec::new())]
    items: Vec<String>,
    template: template_view! { TextBlock { text: "default" } },
}

#[component]
impl DynamicTemplateProbe {}

#[component(inherits ContentControl)]
struct TemplateEnvironmentChild {
    template: template_view! {
        TextBlock { text: environment_text }
    },

    #[environment(standalone_template_environment_text)]
    environment_text: String,
}

#[component]
impl TemplateEnvironmentChild {}

#[test]
fn typed_template_view_can_be_passed_to_environment() {
    let environment = EnvironmentContext::root();
    environment.set_control_template::<TemplateProbe>(Some(template_view! {
        TextBlock { text: templated_parent.label }
    }));
    let _ = ControlTemplate::<TemplateProbe>::new(|context| {
        let _ = context.control.label();
        elwindui::core::ui::TextBlock::new()
    });
}

#[test]
fn standalone_template_view_uses_typed_templated_parent() {
    let template: ControlTemplate<TemplateProbe> = template_view! {
        TextBlock { text: templated_parent.label }
    };
    let probe = TemplateProbe::new("initial".to_string());
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let text = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("template root is TextBlock");
    assert_eq!(text.text.borrow().as_str(), "initial");
    probe.set_label("updated".to_string());
    assert_eq!(text.text.borrow().as_str(), "updated");
}

#[test]
fn standalone_template_view_without_parent_expression_is_typed_by_context() {
    let _: ControlTemplate<DynamicTemplateProbe> = template_view! {
        TextBlock { text: "plain" }
    };
    let environment = EnvironmentContext::root();
    environment.set_control_template::<DynamicTemplateProbe>(Some(template_view! {
        TextBlock { text: "environment plain" }
    }));
}

#[test]
fn standalone_template_view_can_capture_external_values() {
    let captured = String::from("captured");
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        TextBlock { text: format!("{}", captured) }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe,
        environment: EnvironmentContext::root(),
    });
    let text = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("captured template root is TextBlock");
    assert_eq!(text.text.borrow().as_str(), "captured");
}

#[test]
fn standalone_template_view_replaces_dynamic_root() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        if templated_parent.alternate {
            TextBlock { text: "alternate" }
        } else {
            TextBlock { text: "initial" }
        }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
    probe.__prepare_template_presentation();
    let initial = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    probe.__set_template_root(initial);
    assert_eq!(probe.visual_children().len(), 1);
    probe.set_alternate(true);
    assert_eq!(probe.visual_children().len(), 1);
    let visual_children = probe.visual_children();
    let text = visual_children[0]
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("dynamic template root is TextBlock");
    assert_eq!(text.text.borrow().as_str(), "alternate");
}

#[test]
fn standalone_template_view_supports_match_root() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        match templated_parent.alternate {
            true => TextBlock { text: "match-true" },
            false => TextBlock { text: "match-false" },
        }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
    probe.__prepare_template_presentation();
    let initial = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    probe.__set_template_root(initial);
    let initial_child = probe.visual_children()[0].clone();
    assert_eq!(
        initial_child
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("match root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "match-false"
    );
    probe.set_alternate(true);
    let next_child = probe.visual_children()[0].clone();
    assert!(!Rc::ptr_eq(&initial_child, &next_child));
    assert_eq!(
        next_child
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("match root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "match-true"
    );
}

#[test]
fn standalone_template_view_supports_nested_dynamic_child() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        VerticalLayout {
            if templated_parent.alternate {
                TextBlock { text: "nested-true" }
            } else {
                TextBlock { text: "nested-false" }
            }
        }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    use elwindui::core::ui::{ControlExt as _, LayoutExt as _};
    probe.__prepare_template_presentation();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    probe.__set_template_root(root.clone());
    let layout = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::VerticalLayout>()
        .expect("nested template root is VerticalLayout");
    let first = layout.children().to_vec()[0].clone();
    assert_eq!(
        first
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("nested child is TextBlock")
            .text
            .borrow()
            .as_str(),
        "nested-false"
    );
    probe.set_alternate(true);
    let next = layout.children().to_vec()[0].clone();
    assert_eq!(
        next.as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("nested child is TextBlock")
            .text
            .borrow()
            .as_str(),
        "nested-true"
    );
}

#[test]
fn standalone_template_view_supports_nested_match_child() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        VerticalLayout {
            match templated_parent.alternate {
                true => TextBlock { text: "nested-match-true" },
                false => TextBlock { text: "nested-match-false" },
            }
        }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    use elwindui::core::ui::{ControlExt as _, LayoutExt as _};
    probe.__prepare_template_presentation();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    probe.__set_template_root(root.clone());
    let layout = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::VerticalLayout>()
        .expect("nested match root is VerticalLayout");
    let initial = layout.children().to_vec()[0]
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("nested match child is TextBlock")
        .text
        .borrow()
        .clone();
    assert_eq!(initial, "nested-match-false");
    probe.set_alternate(true);
    let next = layout.children().to_vec()[0]
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("nested match child is TextBlock")
        .text
        .borrow()
        .clone();
    assert_eq!(next, "nested-match-true");
}

#[test]
fn standalone_template_mounts_nested_components_with_context_environment() {
    let application = EnvironmentContext::root();
    let environment = application.derive();
    environment.set::<StandaloneTemplateEnvironmentText>("derived".to_string());
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        VerticalLayout {
            TemplateEnvironmentChild {}
        }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    use elwindui::core::ui::{ControlExt as _, LayoutExt as _, UIElementExt as _};
    probe.__prepare_template_presentation();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment,
    });
    probe.__set_template_root(root.clone());
    let layout = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::VerticalLayout>()
        .expect("template root is VerticalLayout");
    let children = layout.children().to_vec();
    let child = children[0]
        .as_any()
        .downcast_ref::<TemplateEnvironmentChild>()
        .expect("nested template child keeps its concrete type");
    let text = child
        .visual_children()
        .into_iter()
        .next()
        .expect("nested template has one TextBlock")
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("nested template root is TextBlock")
        .text
        .borrow()
        .clone();
    assert_eq!(text, "derived");
}

#[test]
fn standalone_template_view_supports_nested_for_children() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        VerticalLayout {
            for item in templated_parent.items {
                TextBlock { text: format!("{}", item) }
            }
        }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    probe.set_items(vec!["one".to_string(), "two".to_string()]);
    use elwindui::core::ui::{ControlExt as _, LayoutExt as _};
    probe.__prepare_template_presentation();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    probe.__set_template_root(root.clone());
    let layout = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::VerticalLayout>()
        .expect("nested for root is VerticalLayout");
    let values = || {
        layout
            .children()
            .to_vec()
            .into_iter()
            .map(|child| {
                child
                    .as_any()
                    .downcast_ref::<elwindui::core::ui::TextBlock>()
                    .expect("nested for child is TextBlock")
                    .text
                    .borrow()
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(values(), vec!["one", "two"]);
    probe.set_items(vec!["three".to_string()]);
    assert_eq!(values(), vec!["three"]);
}
