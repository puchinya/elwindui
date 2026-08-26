#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::Point;
use elwindui::core::environment::EnvironmentContext;
use elwindui::core::ui::{ControlTemplate, UIElementExt as _};
use elwindui::ui::TextBlock;
use elwindui::{component, control_template, template_view};
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static STANDALONE_MOUNT_COUNT: Cell<u32> = const { Cell::new(0) };
    static STANDALONE_UNMOUNT_COUNT: Cell<u32> = const { Cell::new(0) };
    static STANDALONE_UPDATE_COUNT: Cell<u32> = const { Cell::new(0) };
}

fn record_standalone_mount() {
    STANDALONE_MOUNT_COUNT.with(|count| count.set(count.get() + 1));
}

fn record_standalone_unmount() {
    STANDALONE_UNMOUNT_COUNT.with(|count| count.set(count.get() + 1));
}

fn record_standalone_update() {
    STANDALONE_UPDATE_COUNT.with(|count| count.set(count.get() + 1));
}

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

#[control_template(target = TemplateProbe)]
struct NamedTemplateProbe {
    template: template_view! {
        TextBlock {
            text: templated_parent.label,
            on_tapped: |_event| {
                templated_parent.set_label("named-clicked".to_string());
            },
        }
    },
}

#[component(inherits Control)]
struct DefaultEventTemplateProbe {
    #[prop]
    label: String,
    template: template_view! {
        TextBlock {
            text: templated_parent.label,
            on_tapped: |_event| {
                templated_parent.set_label("default-clicked".to_string());
            },
        }
    },
}

#[component]
impl DefaultEventTemplateProbe {}

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

// This probe exercises the same user-component constructor/property metadata as an ordinary
// generated view.  In particular, the required `label` argument must not be lost when the child
// is created by a standalone `template_view!` factory.
#[component(inherits Control)]
struct RequiredLabelChild {
    #[prop]
    label: String,
    template: template_view! {
        TextBlock { text: templated_parent.label }
    },
}

#[component]
impl RequiredLabelChild {}

// A user-defined Layout-derived host must take the generic dynamic-child path.  This deliberately
// avoids naming any builtin layout type in the template itself.
#[component(inherits VerticalLayout)]
struct UserLayoutHost {
    body: view! { TextBlock { text: "host" } },
}

#[component]
impl UserLayoutHost {}

#[component(inherits Control)]
struct LifecycleTemplateProbe {
    template: template_view! {
        on_mount {
            record_standalone_mount();
        }
        on_unmount {
            record_standalone_unmount();
        }
        TextBlock { text: "lifecycle" }
    },
}

#[component]
impl LifecycleTemplateProbe {}

#[component(inherits Control)]
struct UpdateLifecycleTemplateProbe {
    #[prop]
    label: String,
    template: template_view! {
        on_update(label) {
            record_standalone_update();
        }
        TextBlock { text: templated_parent.label }
    },
}

#[component]
impl UpdateLifecycleTemplateProbe {}

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
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let text = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("captured template root is TextBlock");
    assert_eq!(text.text.borrow().as_str(), "captured");
}

#[test]
fn standalone_template_view_supports_deferred_view_values_through_shared_backend() {
    let template: ControlTemplate<TemplateProbe> = template_view! {
        TextBlock {
            context_popup: view! {
                TextBlock { text: "deferred" }
            }
        }
    };
    let probe = TemplateProbe::new("parent".to_string());
    let environment = EnvironmentContext::root();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: environment.clone(),
    });
    let text = root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("template root is TextBlock");
    let popup = text.context_popup().expect("deferred popup template");
    let popup_root = popup
        .build(elwindui::core::ui::ViewBuildContext {
            owner: std::rc::Rc::downgrade(&root),
            environment,
        })
        .expect("deferred popup owner is alive");
    let popup_text = popup_root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("deferred root is TextBlock");
    assert_eq!(popup_text.text.borrow().as_str(), "deferred");
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

#[test]
fn standalone_template_view_supports_template_local_let_references() {
    let template: ControlTemplate<TemplateProbe> = template_view! {
        let heading = TextBlock { text: templated_parent.label };
        VerticalLayout { heading }
    };
    let probe = TemplateProbe::new("let-value".to_string());
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let layout = root
        .as_any()
        .downcast_ref::<elwindui::core::ui::VerticalLayout>()
        .expect("let reference template root is VerticalLayout");
    use elwindui::core::ui::LayoutExt as _;
    let heading = layout
        .children()
        .to_vec()
        .into_iter()
        .next()
        .expect("let reference child is attached");
    let heading = heading
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("let reference child is TextBlock");
    assert_eq!(heading.text.borrow().as_str(), "let-value");
    probe.set_label("updated-let".to_string());
    assert_eq!(heading.text.borrow().as_str(), "updated-let");
}

#[test]
fn standalone_template_view_constructs_user_component_with_required_property() {
    let template: ControlTemplate<TemplateProbe> = template_view! {
        RequiredLabelChild { label: templated_parent.label }
    };
    let probe = TemplateProbe::new("child".to_string());
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let child = root
        .as_any()
        .downcast_ref::<RequiredLabelChild>()
        .expect("standalone template root keeps the user component type");
    assert_eq!(child.label(), "child");
    probe.set_label("updated".to_string());
    assert_eq!(child.label(), "updated");
}

#[test]
fn standalone_template_view_uses_user_layout_as_dynamic_child_host() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view! {
        UserLayoutHost {
            if templated_parent.alternate {
                TextBlock { text: "true" }
            } else {
                TextBlock { text: "false" }
            }
        }
    };
    let probe = DynamicTemplateProbe::__new_unmounted();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    use elwindui::core::ui::LayoutExt as _;
    let host = root
        .as_any()
        .downcast_ref::<UserLayoutHost>()
        .expect("user layout host keeps its concrete type");
    let child_text = || {
        host.children()
            .to_vec()
            .into_iter()
            .filter_map(|child| {
                child
                    .as_any()
                    .downcast_ref::<elwindui::core::ui::TextBlock>()
                    .map(|text| text.text.borrow().clone())
            })
            .find(|text| text == "true" || text == "false")
            .expect("dynamic child is attached through LayoutExt")
    };
    assert_eq!(child_text(), "false");
    probe.set_alternate(true);
    assert_eq!(child_text(), "true");
}

#[test]
fn standalone_template_view_event_closure_can_update_templated_parent() {
    let template: ControlTemplate<TemplateProbe> = template_view! {
        TextBlock {
            text: templated_parent.label,
            on_tapped: |_event| {
                templated_parent.set_label("clicked".to_string());
            },
        }
    };
    let probe = TemplateProbe::new("initial".to_string());
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &root,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 0.0, y: 0.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );
    assert_eq!(probe.label(), "clicked");
}

#[test]
fn named_control_template_uses_the_shared_event_backend() {
    let probe = TemplateProbe::new("initial".to_string());
    let root = NamedTemplateProbe::template().__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &root,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 0.0, y: 0.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );
    assert_eq!(probe.label(), "named-clicked");
}

#[test]
fn standalone_template_view_two_way_binding_uses_shared_property_wiring() {
    // `TextArea::text` is a real two-way property.  Keeping this as a typed construction probe
    // exercises both halves of the common template backend: the initial `@set` and the target
    // props-macro `@set_on_change` callback that writes through `TemplateProperty`.
    let _: ControlTemplate<TemplateProbe> = template_view! {
        TextArea { text <=> templated_parent.label }
    };
}

#[test]
fn component_default_template_event_closure_uses_shared_backend() {
    use elwindui::core::ui::UIElementExt as _;
    let probe = DefaultEventTemplateProbe::new("initial".to_string());
    let root = probe.visual_children()[0].clone();
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &root,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: Point { x: 0.0, y: 0.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );
    assert_eq!(probe.label(), "default-clicked");
}

#[test]
fn standalone_template_view_lifecycle_hooks_run_once() {
    STANDALONE_MOUNT_COUNT.with(|count| count.set(0));
    STANDALONE_UNMOUNT_COUNT.with(|count| count.set(0));
    let template: ControlTemplate<LifecycleTemplateProbe> = template_view! {
        on_mount {
            record_standalone_mount();
        }
        on_unmount {
            record_standalone_unmount();
        }
        TextBlock { text: "lifecycle" }
    };
    let probe = LifecycleTemplateProbe::__new_unmounted();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    use elwindui::core::ui::ControlExt as _;
    probe.__prepare_template_presentation();
    probe.__set_template_root(root.clone());
    STANDALONE_MOUNT_COUNT.with(|count| assert_eq!(count.get(), 1));
    elwindui::core::ui::unmount_subtree(&root);
    STANDALONE_UNMOUNT_COUNT.with(|count| assert_eq!(count.get(), 1));
}

#[test]
fn standalone_template_view_on_update_uses_shared_lifecycle_subscription() {
    STANDALONE_UPDATE_COUNT.with(|count| count.set(0));
    let probe = UpdateLifecycleTemplateProbe::new("initial".to_string());
    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 0);
    probe.set_label("updated".to_string());
    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 1);
}
