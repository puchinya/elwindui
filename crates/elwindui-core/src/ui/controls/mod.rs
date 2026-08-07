//! One file per builtin class, mirroring how each backend crate already lays out its own
//! `native_ui/`/`inner/` twins.
//!
//! The property-setter traits here (`TextArea`/`Button`/`MenuItem`/`Menu`/`MenuBar`/`MenuBarItem`/
//! `Window`) are declared once in this crate rather than duplicated per backend crate.
//! Each backend crate provides `impl Xxx for BackendXImpl { .. }` — the property
//! *shape* (what setters exist, what they take) is common to every backend, only the method
//! *bodies* (the actual platform API calls) differ, exactly the same split
//! `NativeControl`/`Layout`/`Shape`/`Control`/etc. model for the virtual builtins.
//!
//! `Menu`/`MenuBar`/`MenuBarItem`/`Window` are *not* generic over the backend's own concrete
//! menu-entry/menu-bar-entry/menu/menu-bar type the way a backend's own `NativeControlImpl`'s
//! `handle` is — instead each such argument is `&dyn` (or `Rc<dyn>`) the matching leaf trait itself
//! (`MenuItem`/`Menu`/
//! `MenuBarItem`/`MenuBar`), and each backend's own `impl Xxx for BackendXImpl` downcasts it back to
//! its own concrete type via `AsAny::as_any` (see that trait's own doc comment; already the
//! established pattern for `UIElement::try_as_native_control`/`visual_tree::find_all`) before
//! delegating to its real native handle.
//!
//! `TabView`/`TabViewItem` are deliberately **not** included in that shared set: their own methods
//! (`insert_tab`/`remove_tab`/`set_tab_content_visible`, an owned content host handle per platform)
//! are genuinely different in shape per backend (AppKit's `Retained<TreeHostView>`/`TabChipImpl` vs
//! WinUI3's own equivalents have no common signature to share without associated types this crate
//! doesn't need yet) — each backend keeps declaring its own local `TabView` trait.

use super::*;

// Bases first — see the ordering note in `super`'s own `mod.rs`. `UIElement` itself is declared
// there, before `mod controls`, so everything below can `inherits = crate::ui::UIElement`.
mod control;
mod layout;
mod native_control;
mod shape;

// Then everything that inherits one of the above (or inherits nothing at all).
mod button;
mod check_box;
mod content_control;
mod ellipse;
mod grid;
mod horizontal_layout;
mod image;
mod menu;
mod menu_bar;
mod menu_bar_item;
mod menu_item;
mod password_box;
mod radio_button;
mod rectangle;
mod scroll_view;
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
pub use control::*;
pub use ellipse::*;
pub use grid::*;
pub use horizontal_layout::*;
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
pub use tab_view::*;
pub use tab_view_item::*;
pub use text_area::*;
pub use text_block::*;
pub use text_box::*;
pub use toggle_switch::*;
pub use vertical_layout::*;
pub use window::*;
