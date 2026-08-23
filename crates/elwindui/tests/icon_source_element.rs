//! Issue #176: cross-crate DSL and public-class coverage for `IconSourceElement`.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::graphics::{Brush, Color, IconSource, SystemIcon};
use elwindui::core::ui::{IconElementExt as _, IconSourceElementExt as _};

#[elwindui::component(inherits VerticalLayout)]
struct IconSourceElementDslView {
    #[prop(default = Some(IconSource::System(SystemIcon::Copy)))]
    icon_value: Option<IconSource>,

    #[prop(default = Brush::Solid(Color::rgb(1, 2, 3)))]
    icon_foreground: Brush,

    body: view! {
        #[id("icon")]
        let icon = IconSourceElement {
            icon_source: icon_value
            foreground: icon_foreground
        };

        VerticalLayout {
            icon
        }
    },
}

#[elwindui::component]
impl IconSourceElementDslView {}

#[test]
fn icon_source_element_is_constructible_and_configurable_through_view_dsl() {
    let view = IconSourceElementDslView::new();
    assert!(matches!(
        view.icon().icon_source(),
        Some(IconSource::System(SystemIcon::Copy))
    ));
    assert_eq!(
        view.icon().foreground(),
        Some(Brush::Solid(Color::rgb(1, 2, 3)))
    );
}
