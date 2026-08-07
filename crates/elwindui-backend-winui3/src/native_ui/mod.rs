//! The public façade: one class per builtin this backend provides, each implementing the
//! matching `elwindui_core::ui` `*Ext` trait by delegating to its `crate::inner` twin.
//!
//! Deliberately free of XAML calls — every bit of genuinely toolkit-specific complexity
//! lives one layer down in `inner`. That boundary is why this layer is ~70% identical between
//! the two backends and is the natural candidate if it is ever shared outright.
//!
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no type here at
//! all: they are plain `elwindui_core::ui::UIElement` values that `elwindui-codegen` builds
//! directly, reflected into native views by `crate::host`.

// Deliberately *not* `use elwindui_core::base::AsAny;` here — see
// `elwindui_backend_appkit::native_ui::MenuBarItem::set_submenu`'s doc comment (the one place that
// pattern is explained in full) for why importing `AsAny` directly, rather than relying on it as
// `MenuBarItemExt`/`MenuExt`/etc.'s own supertrait, silently breaks every
// `.as_any().downcast_ref::<T>()` call in this file.

// `#[class(inherits = crate::NativeControl)]` in the submodules below expands its supertrait
// bound to `crate::NativeControlExt`, so that trait has to be nameable at *this crate's* root.
pub use elwindui_core::ui::NativeControlExt;

mod button;
mod check_box;
mod control;
mod menu;
mod radio_button;
mod scroll_view;
mod tab_view;
mod text;
mod toggle_switch;
mod window;

// Glob re-exports, not a named list: `#[class]` generates a companion `__elwindui_macros_of_*`
// path alias next to each class, which downstream `#[component(inherits ..)]` resolves as
// `elwindui::ui::__elwindui_macros_of_Window`. Naming only the types here would leave those
// aliases behind in the submodule and break every inheriting user component.
pub use button::*;
pub use check_box::*;
pub use control::*;
pub use menu::*;
pub use radio_button::*;
pub use scroll_view::*;
pub use tab_view::*;
pub use text::*;
pub use toggle_switch::*;
pub use window::*;
