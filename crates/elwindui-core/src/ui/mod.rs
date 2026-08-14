//! The framework-owned Visual tree, following WinUI3's `UIElement` hierarchy: `Rc<dyn UIElement>`
//! nodes *are* the tree (no separate wrapper/enum type) — a backend's own `NativeControlImpl`
//! (`Button`/`TextArea`/`TabView`, the `NativeControl`-implementing family — see that trait's own
//! doc comment), `TextBlock` (self-drawn primitive text),
//! `Shape` (`Rectangle`/`Ellipse`), `VerticalLayout`/`HorizontalLayout` (each embedding
//! shared `Layout` fields as their own `base`, but doing their own orientation-specific layout
//! math directly rather than delegating it to that base), and `Control` (a composable
//! multi-part component) are all peer implementations of the same `UIElement` trait.
//! `Margin`/`HorizontalAlignment`/`VerticalAlignment` (`UIElement`) are common to every one of
//! them, applied generically by this module's `measure`/`arrange` (WinUI3's
//! `UIElement.Measure`/`Arrange` wrapping each type's own `MeasureOverride`/`ArrangeOverride`) —
//! see docs/design/runtime/layout_design.md.
//!
//! `H` (whatever a backend uses as its native widget handle, e.g. `elwindui-backend-appkit`'s
//! `AnyView`) appears only while RenderTree builds or reconciles a native command,
//! `collect_render_items<H>`, downcasting a leaf's `try_as_native_control()` result straight to `H`)
//! — the `UIElement` trait and every other concrete type
//! (`VerticalLayout`/`HorizontalLayout`/`Shape`/`TextBlock`/`Control`) are
//! handle-agnostic, since they never hold one.
//!
//! `Window` is deliberately *not* a `UIElement` — like WinUI3's `Window`, it's a separate
//! top-level host that owns a `Rc<dyn UIElement>` (its content), drives `layout_root`, and
//! its own client area (see `elwindui-backend-appkit`'s `TreeHostView`).
//!
//! **Ownership: `Rc`, not `Box`.** Every node holds a real parent back-reference
//! (`UIElement::visual_parent`, WinUI3's `_parent`) so `dispatch_routed` can bubble a routed event
//! from any element up to the root by simply following `visual_parent()` — no tree search needed,
//! and critically, no dependence on the tree having been built by a single static DSL
//! traversal. Matches real WinUI3/UWP, where measure/arrange/render/hit-test *and* routed-event
//! bubbling all walk the Visual tree — the separate Logical `parent` back-reference exists purely
//! as a receptacle for a future template/accessibility tree (see `UIElementCollection`'s own doc
//! comment) and plays no part in event routing. A back-reference requires shared (`Rc`) ownership,
//! allowing a child to point back to its parent. Every collection's own owner is already fully
//! established by the time `construct()` returns (via `#[class]`'s `__self_weak`, see
//! `UIElement::construct`) — well before any child is ever added.

use crate::base::{CornerRadius, Point, Rect, Size};
#[cfg(test)]
use crate::graphics::Color;
#[cfg(test)]
use crate::graphics::RenderCommand;
pub use crate::graphics::TextAlignment;
use crate::graphics::{
    Brush, ImageDrawOptions, ImageFit, ImageSource, RenderContext, RenderGroup, RenderTree,
    Stretch, StrokeStyle, VectorImageDrawOptions, VectorRasterizeMode,
};
use crate::input::{FocusState, RoutedEventArgs};
use crate::layout::{
    GridCell, GridLength, HorizontalAlignment, Orientation, VerticalAlignment, Visibility,
    align_within, apply_size_constraints, grid_arrange, grid_measure_pass1_available,
    grid_pass2_available, grid_resolve_track_sizes, grow_by_margin, shrink_by_margin,
    shrink_rect_by_margin, stack_arrange, stack_natural_size,
};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

// Submodule declaration order is load-bearing. `#[elwindui_macros::class]` registers each class in
// a cross-invocation "same crate classes" table as it expands, and a derived class that expands
// *before* its base is registered falls back to emitting
// `crate::ui::__elwindui_macros_of_Base::__elwindui_props_Base` — a path that does not exist, since
// that wrapper module only re-exports the inherit trio and `trait_only` classes never emit it at
// all. So bases must be declared before anything that `inherits` them. (`pub use` order below is
// irrelevant — only `mod` order drives expansion order.)
//
// The submodules deliberately open with `use super::*;` rather than repeating this file's import
// block, which is what let the original single-file `ui.rs` be split as a pure code move.
mod element;

mod controls;

mod collections;
mod engine;
mod text_style;

// Last on purpose: several fixtures in here are `#[class]` declarations that inherit real classes
// (`OverridableBase` inherits `UIElement`, the `Fake*` widgets are `struct_only` implementors of
// the `NativeControl` family), so they must expand after every class above is registered.
#[cfg(test)]
mod testsupport;

// Glob re-exports, never named lists: `#[class]` emits a companion `__elwindui_macros_of_*` module
// next to each class, which downstream `#[component(inherits ..)]` resolves as
// `elwindui::ui::__elwindui_macros_of_Window`. Naming only the types here would strand those
// aliases in the submodule and break every inheriting user component — the same constraint
// `elwindui-backend-appkit`'s `native_ui/mod.rs` documents for its own split.
pub use collections::*;
pub use controls::*;
pub use element::*;
pub use engine::*;
pub use text_style::*;
