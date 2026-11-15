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
