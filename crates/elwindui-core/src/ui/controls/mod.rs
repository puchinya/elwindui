//! One file per builtin class, mirroring how each backend crate already lays out its own
//! `native_ui/`/`inner/` twins.
//!
//! Every builtin is declared once here. The native builtins (`Button`, `Window`, the input
//! controls, `Dropdown`, `ScrollView`, `TabView`, the menus, ...) are Core-owned Components whose
//! OS widgets are backend peers (`crate::ui::native`, Issue #313,
//! `docs/design/runtime/native_peer_design.md`): Core owns their properties, logical state,
//! item lists and events, and each backend only implements the peer traits.

use super::*;

// Bases first — see the ordering note in `super`'s own `mod.rs`. `UIElement` itself is declared
// there, before `mod controls`, so everything below can `inherits = crate::ui::UIElement`.
mod control;
mod icon_element;
mod layout;
mod native_control;
mod shape;

// Then everything that inherits one of the above (or inherits nothing at all).
mod button;
mod check_box;
mod content_control;
mod content_presenter;
mod control_template;
mod dropdown;
mod dropdown_item;
mod ellipse;
mod grid;
mod horizontal_layout;
mod icon_source_element;
mod image;
mod menu;
mod menu_bar;
mod menu_bar_item;
mod menu_item;
#[cfg(test)]
mod native_container_tests;
#[cfg(test)]
mod native_input_tests;
mod password_box;
mod radio_button;
mod rectangle;
mod scroll_view;
mod slider;
mod tab_view;
mod tab_view_item;
mod text_area;
mod text_block;
mod text_box;
mod toggle_switch;
mod vertical_layout;
mod window;

// Globs, never named lists — each class's companion `__elwindui_macros_of_*` alias has to travel
// up to `elwindui::ui::` for downstream `#[component(inherits ..)]` to resolve it.
pub use button::*;
pub use check_box::*;
pub use content_control::*;
pub use content_presenter::*;
pub use control::*;
pub use control_template::*;
pub use dropdown::*;
pub use dropdown_item::*;
pub use ellipse::*;
pub use grid::*;
pub use horizontal_layout::*;
pub use icon_element::*;
pub use icon_source_element::*;
pub use image::*;
pub use layout::*;
pub use menu::*;
pub use menu_bar::*;
pub use menu_bar_item::*;
pub use menu_item::*;
pub use native_control::*;
pub use password_box::*;
pub use radio_button::*;
pub use rectangle::*;
pub use scroll_view::*;
pub use shape::*;
pub use slider::*;
pub use tab_view::*;
pub use tab_view_item::*;
pub use text_area::*;
pub use text_block::*;
pub use text_box::*;
pub use toggle_switch::*;
pub use vertical_layout::*;
pub use window::*;
