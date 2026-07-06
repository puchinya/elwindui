//! End-user facing facade crate. See docs/elwindui_gui_framework_design.md §1.

pub use elwindui_core::{find_all, find_by_id, Element};
pub use elwindui_macros::{component, viewmodel};

/// `platform::clipboard`/`platform::file_dialog` etc. See docs/elwindui_spec.md 付録T.
#[cfg(feature = "backend-appkit")]
pub mod platform {
    pub use elwindui_backend_appkit::platform::file_dialog;
}

/// See the `backend-appkit` `platform` module above. `elwindui-backend-winui3` is best-effort/
/// unverified (see its crate-level doc comment) — not built or run on a real Windows machine.
#[cfg(feature = "backend-winui3")]
pub mod platform {
    pub use elwindui_backend_winui3::platform::file_dialog;
}

/// `application::run()` enters the platform's event loop — call once, after showing every
/// top-level window. See docs/elwindui_spec.md 付録P.5, `elwindui-backend-appkit`'s `application`
/// module.
#[cfg(feature = "backend-appkit")]
pub mod application {
    pub use elwindui_backend_appkit::application::run;
}

/// See the `backend-appkit` `application` module above. `elwindui-backend-winui3` is best-effort/
/// unverified (see its crate-level doc comment) — not built or run on a real Windows machine.
#[cfg(feature = "backend-winui3")]
pub mod application {
    pub use elwindui_backend_winui3::application::run;
}
