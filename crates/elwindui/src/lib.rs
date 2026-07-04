//! End-user facing facade crate. See docs/elwindui_gui_framework_design.md §1.

pub use elwindui_core::{find_all, find_by_id, Element};
pub use elwindui_macros::{component, viewmodel};

/// `platform::clipboard`/`platform::file_dialog` etc. See docs/elwindui_spec.md 付録T.
#[cfg(feature = "backend-appkit")]
pub mod platform {
    pub use elwindui_backend_appkit::platform::file_dialog;
}
