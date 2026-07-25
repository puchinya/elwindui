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

// `control` is declared before its subclasses' modules on purpose: each of those is
// `#[class(inherits = crate::NativeControl)]`, which reaches its parent through a
// `#[macro_export]`ed `__elwindui_inherit_NativeControl!`. A `#[macro_export]` macro only
// becomes reachable *after* the item defining it has been expanded, so a sibling module
// declared above `control` expands first and fails to find it.
// `#[macro_use]` is what carries that macro into this module's textual scope so the sibling
// modules below can see it: for a *same-crate* parent, `#[class]` emits the invocation
// unqualified (see `inherit_macro_prefix` in crates/elwindui-macros/src/class.rs), and an
// unqualified `macro_rules!` is not otherwise visible across sibling modules.
// See docs/elwindui_macro_class_spec.md.
#[macro_use]
mod control;

mod button;
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
