//! Docking primitives for ElwindUI.
//!
//! The crate deliberately keeps the authored declaration objects, the value-semantic layout model,
//! and the private runtime realization separate. Applications import this crate explicitly as
//! `elwindui_docking`; nothing is re-exported by the `elwindui` facade.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

extern crate self as elwindui;

pub use elwindui_macros::{class, component, template_view};

pub mod core {
    pub use elwindui_core::*;
}

// The component macro resolves builtin inheritance and generated extension traits through this
// private facade. It is intentionally not a public re-export from `elwindui`.
pub mod ui {
    #[doc(hidden)]
    pub use crate::{DockRuntimeHost, DockRuntimeHostExt};
    pub use elwindui_core::ui::*;
}

mod dock_group;
mod dock_item;
mod dock_split_panel;
mod docking_control;
mod id;
mod model;
mod placement;
mod runtime;
mod snapshot;

pub use dock_group::{DockGroup, DockGroupExt};
pub use dock_item::{DockItem, DockItemExt};
pub use dock_split_panel::{DockSplitPanel, DockSplitPanelExt};
pub use docking_control::{DockRuntimeHost, DockRuntimeHostExt, DockingControl, DockingControlExt};
pub use id::{DockGroupId, DockItemId};
pub use model::DockLayoutModel;

// The generated component property storage classifies bare capitalized types as Copy. This alias
// keeps the non-Copy model in RefCell storage while preserving the public semantic type exactly.
#[allow(non_camel_case_types)]
pub type dock_layout_model = DockLayoutModel;
pub use placement::{DockLayoutError, DockPlacement, DockSide, DockTarget};
pub use snapshot::DockLayoutSnapshot;

pub use elwindui_core::base::Rect;
pub use elwindui_core::graphics::IconSource;
pub use elwindui_core::layout::Orientation;
pub use elwindui_custom_controls::TabStripPosition;

#[cfg(test)]
mod tests;
