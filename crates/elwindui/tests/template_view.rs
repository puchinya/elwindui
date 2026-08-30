#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::Point;
use elwindui::core::environment::EnvironmentContext;
use elwindui::core::ui::{
    ControlTemplate, TemplateProperty as _, UIElementExt as _, WritableTemplateProperty as _,
};
use elwindui::ui::{ContentControl, Control, ListExt as _, Rectangle, TextArea, TextBlock};
use elwindui::{component, template_view};
use std::cell::Cell;
use std::marker::PhantomData;
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

const fn template_property_key(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        index += 1;
    }
    hash
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
    template: template_view!(|control: Self| {
        TextBlock {
            text: control.label,
        }
    }),
}

#[component]
impl TemplateProbe {}

#[component(inherits Control)]
struct InheritedWritableTemplateBase {
    #[prop(default = String::from("base"))]
    value: String,
    template: template_view!(|templated_parent: Self| { TextBlock { text: value } }),
}

#[component]
impl InheritedWritableTemplateBase {}

#[component(inherits crate::InheritedWritableTemplateBase)]
struct InheritedWritableTemplateChild {
    template: template_view!(|templated_parent: Self| {
        TextArea { text <=> templated_parent.value }
    }),
}

#[component]
impl InheritedWritableTemplateChild {}

fn named_template_probe() -> ControlTemplate<TemplateProbe> {
    template_view!(|button: TemplateProbe| {
        TextBlock {
            text: button.label,
            on_tapped: |_event| {
                button.set_label("named-clicked".to_string());
            },
        }
    })
}

fn prefixed_template(prefix: String) -> ControlTemplate<TemplateProbe> {
    template_view!(|owner: TemplateProbe| {
        TextBlock {
            text: format!("{}{}", prefix.as_str(), owner.label),
        }
    })
}

#[component(inherits Control)]
struct DefaultEventTemplateProbe {
    #[prop]
    label: String,
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: templated_parent.label,
            on_tapped: |_event| {
                templated_parent.set_label("default-clicked".to_string());
            },
        }
    }),
}

#[component]
impl DefaultEventTemplateProbe {}

#[component(inherits ContentControl)]
struct DynamicTemplateProbe {
    #[prop(default = false)]
    alternate: bool,
    #[prop(default = Vec::new())]
    items: Vec<String>,
    template: template_view!(|templated_parent: Self| { TextBlock { text: "default" } }),
}

#[component]
impl DynamicTemplateProbe {}

#[component(inherits ContentControl)]
struct TemplateEnvironmentChild {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: environment_text,
        }
    }),

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
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: templated_parent.label,
        }
    }),
}

#[component]
impl RequiredLabelChild {}

#[component(inherits ContentControl)]
struct UserScalarContentHost {
    template: template_view!(|templated_parent: Self| { TextBlock { text: "host" } }),
}

#[component]
impl UserScalarContentHost {}

pub struct TestListExt<T: ?Sized> {
    items: std::cell::RefCell<Vec<Rc<dyn elwindui::core::ui::UIElementExt>>>,
    marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Default for TestListExt<T> {
    fn default() -> Self {
        Self {
            items: std::cell::RefCell::new(Vec::new()),
            marker: PhantomData,
        }
    }
}

impl elwindui::core::ui::ListExt<dyn elwindui::core::ui::UIElementExt>
    for TestListExt<dyn elwindui::core::ui::UIElementExt>
{
    fn add(&self, item: Rc<dyn elwindui::core::ui::UIElementExt>) {
        self.items.borrow_mut().push(item);
    }

    fn insert(&self, index: usize, item: Rc<dyn elwindui::core::ui::UIElementExt>) {
        let mut items = self.items.borrow_mut();
        let index = index.min(items.len());
        items.insert(index, item);
    }

    fn remove(&self, item: &Rc<dyn elwindui::core::ui::UIElementExt>) -> bool {
        let mut items = self.items.borrow_mut();
        let Some(index) = items
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, item))
        else {
            return false;
        };
        items.remove(index);
        true
    }

    fn remove_at(&self, index: usize) -> Rc<dyn elwindui::core::ui::UIElementExt> {
        self.items.borrow_mut().remove(index)
    }

    fn clear(&self) {
        self.items.borrow_mut().clear();
    }

    fn len(&self) -> usize {
        self.items.borrow().len()
    }

    fn is_empty(&self) -> bool {
        self.items.borrow().is_empty()
    }

    fn to_vec(&self) -> Vec<Rc<dyn elwindui::core::ui::UIElementExt>> {
        self.items.borrow().clone()
    }
}

#[component(inherits Control)]
#[content(children)]
struct UserCollectionContentHost {
    #[prop(default = Rc::new(TestListExt::<dyn elwindui::core::ui::UIElementExt>::default()))]
    children: Rc<TestListExt<dyn elwindui::core::ui::UIElementExt>>,
    template: template_view!(|templated_parent: Self| { Rectangle {} }),
}

#[component]
impl UserCollectionContentHost {}

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
    template: template_view!(|templated_parent: Self| {
        on_mount {
            record_standalone_mount();
        }
        on_unmount {
            record_standalone_unmount();
        }
        TextBlock { text: "lifecycle" }
    }),
}

#[component]
impl LifecycleTemplateProbe {}

#[component(inherits Control)]
struct UpdateLifecycleTemplateProbe {
    #[prop]
    label: String,
    template: template_view!(|templated_parent: Self| {
        on_update(label) {
            record_standalone_update();
        }
        TextBlock { text: templated_parent.label }
    }),
}

#[component]
impl UpdateLifecycleTemplateProbe {}

#[component(inherits Control)]
struct ReadOnlyComputedTemplateProbe {
    #[prop(default = String::from("source"))]
    source: String,
    #[computed(expr = source.clone())]
    read_only_value: String,
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: templated_parent.read_only_value,
        }
    }),
}

#[component]
impl ReadOnlyComputedTemplateProbe {}

#[test]
fn typed_template_view_can_be_passed_to_environment() {
    let environment = EnvironmentContext::root();
    environment.set_control_template(Some(template_view!(|templated_parent: TemplateProbe| {
        TextBlock {
            text: templated_parent.label,
        }
    })));
    let _ = ControlTemplate::<TemplateProbe>::new(|context| {
        let _ = context.control.label();
        elwindui::core::ui::TextBlock::new()
    });
}

#[test]
fn explicit_target_template_infers_result_without_expected_type_annotation() {
    let template = template_view!(|owner: TemplateProbe| { TextBlock { text: owner.label } });
    let probe = elwindui::new!(TemplateProbe(label: "initial".to_string()));
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let text = root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("explicit-target template root is TextBlock");
    assert_eq!(text.text.borrow().as_str(), "initial");
    probe.set_label("updated".to_string());
    assert_eq!(text.text.borrow().as_str(), "updated");
}

#[test]
fn standalone_template_view_uses_typed_templated_parent() {
    let template: ControlTemplate<TemplateProbe> =
        template_view!(|templated_parent: TemplateProbe| {
            TextBlock {
                text: templated_parent.label,
            }
        });
    let probe = elwindui::new!(TemplateProbe(label: "initial".to_string()));
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
    let _: ControlTemplate<DynamicTemplateProbe> =
        template_view!(|templated_parent: DynamicTemplateProbe| { TextBlock { text: "plain" } });
    let environment = EnvironmentContext::root();
    environment.set_control_template::<DynamicTemplateProbe>(Some(template_view!(
        |templated_parent: DynamicTemplateProbe| {
            TextBlock {
                text: "environment plain",
            }
        }
    )));
}

#[test]
fn property_free_template_view_accepts_raw_control_target() {
    let _: ControlTemplate<Control> = template_view!(|templated_parent: Control| {
        TextBlock {
            text: "framework target",
        }
    });
}

#[test]
fn standalone_template_view_can_capture_external_values() {
    let captured = String::from("captured");
    let template: ControlTemplate<DynamicTemplateProbe> =
        template_view!(|templated_parent: DynamicTemplateProbe| {
            TextBlock {
                text: format!("{}", captured),
            }
        });
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
    let template: ControlTemplate<TemplateProbe> =
        template_view!(|templated_parent: TemplateProbe| {
            TextBlock {
                context_popup: view! {
                    TextBlock { text: "deferred" }
                },
            }
        });
    let probe = elwindui::new!(TemplateProbe(label: "parent".to_string()));
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
    let template: ControlTemplate<DynamicTemplateProbe> =
        template_view!(|templated_parent: DynamicTemplateProbe| {
            if templated_parent.alternate {
                TextBlock { text: "alternate" }
            } else {
                TextBlock { text: "initial" }
            }
        });
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
fn standalone_template_view_replaces_scalar_content_without_layout_host() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        ContentControl {
            if templated_parent.alternate {
                TextBlock { text: "alternate" }
            } else {
                TextBlock { text: "initial" }
            }
        }
    });
    let probe = DynamicTemplateProbe::__new_unmounted();
    use elwindui::core::ui::{ContentControlExt as _, ControlExt as _};
    probe.__prepare_template_presentation();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    probe.__set_template_root(root.clone());
    let host = root
        .as_any()
        .downcast_ref::<ContentControl>()
        .expect("scalar template host is ContentControl");
    let old = host.content();
    let old_text = old
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("initial scalar content is TextBlock");
    assert_eq!(old_text.text.borrow().as_str(), "initial");
    probe.set_alternate(true);
    let new = host.content();
    let new_text = new
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("replacement scalar content is TextBlock");
    assert_eq!(new_text.text.borrow().as_str(), "alternate");
    assert!(!Rc::ptr_eq(&old, &new));
    assert!(old.visual_parent().is_none());
    assert!(new.visual_parent().is_some());
}

#[test]
fn standalone_template_view_uses_user_scalar_content_metadata() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        UserScalarContentHost {
            if templated_parent.alternate {
                TextBlock { text: "user-alternate" }
            } else {
                TextBlock { text: "user-initial" }
            }
        }
    });
    let probe = DynamicTemplateProbe::__new_unmounted();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let host = root
        .as_any()
        .downcast_ref::<UserScalarContentHost>()
        .expect("user scalar host keeps its concrete type");
    use elwindui::core::ui::ContentControlExt as _;
    let initial = host.content();
    assert_eq!(
        initial
            .as_any()
            .downcast_ref::<TextBlock>()
            .expect("initial user scalar content is TextBlock")
            .text
            .borrow()
            .as_str(),
        "user-initial"
    );
    probe.set_alternate(true);
    let replacement = host.content();
    assert_eq!(
        replacement
            .as_any()
            .downcast_ref::<TextBlock>()
            .expect("replacement user scalar content is TextBlock")
            .text
            .borrow()
            .as_str(),
        "user-alternate"
    );
    assert!(!Rc::ptr_eq(&initial, &replacement));
}

#[test]
fn standalone_template_view_uses_non_layout_collection_content_metadata() {
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        UserCollectionContentHost {
            TextBlock { text: "static" }
            if templated_parent.alternate {
                TextBlock { text: "A" }
            } else {
                TextBlock { text: "B" }
            }
        }
    });
    let probe = DynamicTemplateProbe::__new_unmounted();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let host = root
        .as_any()
        .downcast_ref::<UserCollectionContentHost>()
        .expect("user collection host keeps its concrete type");
    let labels = || {
        host.children()
            .to_vec()
            .into_iter()
            .map(|child| {
                child
                    .as_any()
                    .downcast_ref::<TextBlock>()
                    .expect("collection item is TextBlock")
                    .text
                    .borrow()
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(), vec!["static", "B"]);
    probe.set_alternate(true);
    assert_eq!(labels(), vec!["static", "A"]);
}

#[test]
fn standalone_template_view_supports_match_root() {
    let template: ControlTemplate<DynamicTemplateProbe> =
        template_view!(|templated_parent: DynamicTemplateProbe| {
            match templated_parent.alternate {
                true => TextBlock { text: "match-true" },
                false => TextBlock {
                    text: "match-false",
                },
            }
        });
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
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        VerticalLayout {
            if templated_parent.alternate {
                TextBlock { text: "nested-true" }
            } else {
                TextBlock { text: "nested-false" }
            }
        }
    });
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
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        VerticalLayout {
            match templated_parent.alternate {
                true => TextBlock { text: "nested-match-true" },
                false => TextBlock { text: "nested-match-false" },
            }
        }
    });
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
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        VerticalLayout {
            TemplateEnvironmentChild {}
        }
    });
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
    assert!(child.apply_template());
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
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        VerticalLayout {
            for item in templated_parent.items {
                TextBlock { text: format!("{}", item) }
            }
        }
    });
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
    let template: ControlTemplate<TemplateProbe> =
        template_view!(|templated_parent: TemplateProbe| {
            let heading = TextBlock {
                text: templated_parent.label,
            };
            VerticalLayout { heading }
        });
    let probe = elwindui::new!(TemplateProbe(label: "let-value".to_string()));
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
    let template: ControlTemplate<TemplateProbe> =
        template_view!(|templated_parent: TemplateProbe| {
            RequiredLabelChild {
                label: templated_parent.label,
            }
        });
    let probe = elwindui::new!(TemplateProbe(label: "child".to_string()));
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
    let template: ControlTemplate<DynamicTemplateProbe> = template_view!(|templated_parent: DynamicTemplateProbe| {
        UserLayoutHost {
            if templated_parent.alternate {
                TextBlock { text: "true" }
            } else {
                TextBlock { text: "false" }
            }
        }
    });
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
    let template: ControlTemplate<TemplateProbe> =
        template_view!(|templated_parent: TemplateProbe| {
            TextBlock {
                text: templated_parent.label,
                on_tapped: |_event| {
                    templated_parent.set_label("clicked".to_string());
                },
            }
        });
    let probe = elwindui::new!(TemplateProbe(label: "initial".to_string()));
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
fn reusable_template_function_returns_control_template_and_uses_shared_event_backend() {
    let probe = elwindui::new!(TemplateProbe(label: "initial".to_string()));
    let root = named_template_probe().__build(elwindui::core::ui::ControlTemplateContext {
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
fn reusable_template_function_captures_prefix_and_resyncs_typed_parent() {
    let template = prefixed_template("P:".to_string());
    let probe = elwindui::new!(TemplateProbe(label: "A".to_string()));
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    let text = root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("prefixed template root is TextBlock");
    assert_eq!(text.text.borrow().as_str(), "P:A");
    probe.set_label("B".to_string());
    assert_eq!(text.text.borrow().as_str(), "P:B");
}

#[test]
fn reusable_template_accepts_a_non_reserved_parent_alias() {
    let template: ControlTemplate<TemplateProbe> =
        template_view!(|owner: TemplateProbe| { TextBlock { text: owner.label } });
    let probe = elwindui::new!(TemplateProbe(label: "alias".to_string()));
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe,
        environment: EnvironmentContext::root(),
    });
    let text = root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("aliased template root is TextBlock");
    assert_eq!(text.text.borrow().as_str(), "alias");
}

#[test]
fn standalone_template_view_two_way_binding_uses_shared_property_wiring() {
    // `TextArea::text` is a real two-way property.  Keeping this as a typed construction probe
    // exercises both halves of the common template backend: the initial `@set` and the target
    // props-macro `@set_on_change` callback that writes through `TemplateProperty`.
    let _: ControlTemplate<TemplateProbe> = template_view!(|templated_parent: TemplateProbe| {
        TextArea { text <=> templated_parent.label }
    });
}

#[test]
fn inherited_writable_template_property_delegates_to_base() {
    const VALUE_KEY: u64 = template_property_key("value");

    let child = InheritedWritableTemplateChild::__new_unmounted();
    assert_eq!(
        <InheritedWritableTemplateChild as elwindui::core::ui::TemplateProperty<VALUE_KEY>>::__template_get(
            &*child,
        ),
        "base"
    );
    <InheritedWritableTemplateChild as elwindui::core::ui::WritableTemplateProperty<VALUE_KEY>>::__template_set(
        &*child,
        "updated".to_string(),
    );
    assert_eq!(child.base.value(), "updated");
}

#[test]
fn component_default_template_event_closure_uses_shared_backend() {
    use elwindui::core::ui::UIElementExt as _;
    let probe = elwindui::new!(DefaultEventTemplateProbe(label: "initial".to_string()));
    assert!(probe.apply_template());
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
    let template: ControlTemplate<LifecycleTemplateProbe> = template_view!(|templated_parent: LifecycleTemplateProbe| {
        on_mount {
            record_standalone_mount();
        }
        on_unmount {
            record_standalone_unmount();
        }
        TextBlock { text: "lifecycle" }
    });
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
fn component_default_template_view_on_update_uses_shared_lifecycle_subscription() {
    STANDALONE_UPDATE_COUNT.with(|count| count.set(0));
    let probe = elwindui::new!(UpdateLifecycleTemplateProbe(label: "initial".to_string()));
    assert!(probe.apply_template());
    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 0);
    probe.set_label("updated".to_string());
    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 1);
}

#[test]
fn component_default_template_reads_and_resyncs_computed_property() {
    use elwindui::core::ui::UIElementExt as _;

    let probe = ReadOnlyComputedTemplateProbe::new();
    assert!(probe.apply_template());
    let root = probe.visual_children()[0].clone();
    let text = root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("computed-property template root is TextBlock");
    assert_eq!(probe.read_only_value(), "source");
    assert_eq!(text.text.borrow().as_str(), "source");

    probe.set_source("updated".to_string());

    assert_eq!(probe.read_only_value(), "updated");
    assert_eq!(text.text.borrow().as_str(), "updated");
}

#[test]
fn standalone_template_view_on_update_uses_shared_lifecycle_subscription() {
    STANDALONE_UPDATE_COUNT.with(|count| count.set(0));
    let template: ControlTemplate<UpdateLifecycleTemplateProbe> = template_view!(|templated_parent: UpdateLifecycleTemplateProbe| {
        on_update(label) {
            record_standalone_update();
        }
        TextBlock {
            text: templated_parent.label
        }
    });

    let probe = UpdateLifecycleTemplateProbe::__new_unmounted();
    probe.__set_initial_prop_label("initial".to_string());
    use elwindui::core::ui::ControlExt as _;
    probe.__prepare_template_presentation();
    let root = template.__build(elwindui::core::ui::ControlTemplateContext {
        control: probe.clone(),
        environment: EnvironmentContext::root(),
    });
    probe.__set_template_root(root);

    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 0);
    probe.set_label("updated".to_string());
    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 1);
    probe.set_label("updated-again".to_string());
    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 2);

    let probe_node: Rc<dyn elwindui::core::ui::UIElementExt> = probe.clone();
    elwindui::core::ui::unmount_subtree(&probe_node);
    probe.set_label("after-unmount".to_string());
    assert_eq!(STANDALONE_UPDATE_COUNT.with(Cell::get), 2);
}
