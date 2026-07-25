//! Implements every `elwindui_core::ui` builtin trait this backend provides, by composing the
//! matching `crate::inner` type (see that module's own doc comment) — each class here is a thin
//! "call into `self.inner`" layer; all genuinely AppKit-specific complexity lives in `inner.rs`.
//! See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no type here at all:
//! they're `elwindui_core::ui::UIElement` values that `elwindui-codegen` builds directly, reflected
//! into real `NSView`s/`CAShapeLayer`s/`CATextLayer`s by `inner::TreeHostView` (used by both
//! `Window`'s content view and `TabView`'s per-tab content area).

// Deliberately *not* `use elwindui_core::base::AsAny;` here — see the doc comment on
// `MenuBarItem::set_submenu` (the one place that pattern is explained in full) for why importing
// `AsAny` directly, rather than relying on it as `MenuBarItemExt`/`MenuExt`/etc.'s own supertrait,
// silently breaks every `.as_any().downcast_ref::<T>()` call in this file.

// `#[class(inherits = crate::NativeControl)]` in the submodules below expands its supertrait
// bound to `crate::NativeControlExt`, so that trait has to be nameable at *this crate's*
// root. `NativeControl` is a `struct_only` class whose trait lives in elwindui-core, so it
// is re-exported explicitly here and lifted to the root by `lib.rs`'s `pub use native_ui::*`.
pub use elwindui_core::ui::NativeControlExt;

mod button;
mod control;
mod menu;
mod scroll_view;
mod tab_view;
mod text;
mod window;

// Glob re-exports, not a named list: `#[class]` generates a companion `__elwindui_macros_of_*`
// path alias next to each class, which downstream `#[component(inherits ..)]` resolves as
// `elwindui::ui::__elwindui_macros_of_Window`. Naming only the types here would leave those
// aliases behind in the submodule and break every inheriting user component.
pub use button::*;
pub use control::*;
pub use menu::*;
pub use scroll_view::*;
pub use tab_view::*;
pub use text::*;
pub use window::*;
