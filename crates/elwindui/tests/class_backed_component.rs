//! PR #312 Implementation Contract v3: builtins written with the plain `#[component]` attribute
//! keep their hand-written storage and get the same constructor ABI as generated Components.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::UIElementExt as _;

/// T1/T12: the facade-root constructor ABI of `VerticalLayout` (a class-backed `#[component]` in
/// Core) builds the real type, and sets its own `spacing` and the inherited `margin` through the
/// class chain, exactly as `new!` drives an external constructor ABI.
#[test]
fn vertical_layout_constructor_abi_sets_own_and_inherited_properties() {
    let layout = elwindui::__elwindui_ctor_VerticalLayout!(
        @new elwindui::ui::VerticalLayout, props = [(spacing => 8.0), (margin => 4.0)]
    );
    #[allow(unused_variables)]
    let environment = elwindui::core::environment::application_environment();
    elwindui::__elwindui_ctor_VerticalLayout!(
        @new_initial_props layout, props = [(spacing => 8.0), (margin => 4.0)],
        environment = environment
    );
    assert_eq!(layout.margin(), 4.0);

    let plain = elwindui::ui::VerticalLayout::new();
    for (child_layout, height) in [(&layout, 10.0_f32), (&plain, 10.0_f32)] {
        for _ in 0..2 {
            let child = elwindui::ui::Rectangle::new();
            elwindui::core::ui::UIElementExt::set_height(&*child, height);
            elwindui::core::ui::LayoutExt::children(&**child_layout).add(child);
        }
    }
    let available = elwindui::core::base::Size {
        width: 100.0,
        height: f32::INFINITY,
    };
    layout.measure(available);
    plain.measure(available);
    let spaced = layout.measured_size().unwrap().height;
    let unspaced = plain.measured_size().unwrap().height;
    // One gap between two children, plus the margin on both sides.
    assert_eq!(spaced - unspaced, 8.0 + 2.0 * 4.0);
}

/// T4/T5: the backend's `Button`/`Window` (class-backed `struct_only` Components whose public
/// properties are declared by the Core interfaces, `abi_props`) build through their facade-root
/// constructor ABI; `text`/`title` reach the backend implementation through the Core property
/// shape. The bare-name `new!` path keeps working alongside.
#[cfg(all(feature = "backend-winui3", target_os = "windows"))]
#[test]
fn native_builtins_build_through_their_constructor_abi() {
    elwindui::init().expect("initialize WinUI3");
    elwindui::application::run(|| {
        // Only the bare-name `new!` and the method call below need the traits in scope; the
        // constructor ABI does not.
        #[allow(unused_imports)]
        use elwindui::ui::*;
        let label = |node: &dyn elwindui::core::ui::UIElementExt| {
            node.intrinsic_accessibility_semantics()
                .and_then(|semantics| semantics.label)
        };
        #[allow(unused_variables)]
        let environment = elwindui::core::environment::application_environment();
        let button = elwindui::__elwindui_ctor_Button!(
            @new elwindui::ui::Button, props = [(text => "abi")]
        );
        elwindui::__elwindui_ctor_Button!(
            @new_initial_props button, props = [(text => "abi")], environment = environment
        );
        assert_eq!(label(&*button).as_deref(), Some("abi"));

        let window = elwindui::__elwindui_ctor_Window!(
            @new elwindui::ui::Window, props = [(title => "abi window")]
        );
        elwindui::__elwindui_ctor_Window!(
            @new_initial_props window, props = [(title => "abi window")], environment = environment
        );
        window.close();

        let via_new = elwindui::new!(Button(text: "builtin"));
        assert_eq!(label(&*via_new).as_deref(), Some("builtin"));
    });
}

thread_local! {
    static CUSTOM_CONSTRUCTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// T3 (D2): a declarative Component with a hand-written `construct` keeps its generated storage:
/// `construct` runs once per instance and builds `Self` through the generated initializer, with
/// the Component's constructor inputs in order.
#[elwindui::component(inherits VerticalLayout)]
struct CustomConstructed {
    #[param]
    title: String,

    body: view! {
        TextBlock { text: title }
    },
}

#[elwindui::component]
impl CustomConstructed {
    fn construct(title: String) -> Self {
        CUSTOM_CONSTRUCTS.with(|count| count.set(count.get() + 1));
        Self::__elwindui_generated_construct(__self_weak, format!("{title}!"))
    }
}

fn texts(root: &dyn elwindui::core::ui::UIElementExt) -> Vec<String> {
    root.visual_children()
        .iter()
        .filter_map(|child| {
            child
                .as_any()
                .downcast_ref::<elwindui::ui::TextBlock>()
                .map(|text| text.text.borrow().clone())
        })
        .collect()
}

#[test]
fn a_declarative_component_with_a_hand_written_construct() {
    CUSTOM_CONSTRUCTS.with(|count| count.set(0));
    let first = CustomConstructed::new("a".to_string());
    let second = CustomConstructed::new("b".to_string());
    assert_eq!(CUSTOM_CONSTRUCTS.with(std::cell::Cell::get), 2);
    assert_eq!(texts(&*first), ["a!"]);
    assert_eq!(texts(&*second), ["b!"]);
    assert_eq!(first.title(), "a!");
}

/// D1 case (iii): a struct with no view, no field-level DSL attribute and no struct-level class
/// shape attribute, whose `impl` has a `construct`, keeps its hand-written storage (here a
/// `Cell`) and its untagged methods, like any class-backed Component.
#[elwindui::component(inherits elwindui::core::ui::VerticalLayout)]
pub struct RawCounterLayout {
    hits: std::cell::Cell<u32>,
}

#[elwindui::component]
impl RawCounterLayout {
    fn hit(&self) -> u32 {
        self.hits.set(self.hits.get() + 1);
        self.hits.get()
    }

    fn construct() -> Self {
        Self {
            base: elwindui::core::ui::VerticalLayout::construct(),
            hits: std::cell::Cell::new(10),
        }
    }
}

#[test]
fn a_construct_only_struct_keeps_its_hand_written_storage() {
    let layout = RawCounterLayout::new();
    assert_eq!(layout.hit(), 11);
    assert_eq!(layout.hit(), 12);
    layout.set_margin(3.0);
    assert_eq!(layout.margin(), 3.0);
}
