//! End-user facing facade crate. See docs/elwindui_gui_framework_design.md §1.
//!
//! A consumer crate needs only `elwindui` itself in `[dependencies]` — `elwindui-codegen`
//! (`compile_dir`/`compile_dir_with_extra_viewmodels`, called from build.rs) and `elwindui-macros`'
//! `component!`/`#[viewmodel]` proc-macros both emit generated code that refers exclusively to
//! `elwindui::core::..`/`elwindui::backend::..`/`elwindui::i18n::..` (never `elwindui_core::..`/
//! `elwindui_backend_*::..`/`elwindui_i18n::..` directly), which resolve through the re-exports
//! below regardless of how many crates deep `elwindui` itself pulls them in from.

pub use elwindui_core as core;
pub use elwindui_core::visual_tree;
pub use elwindui_i18n as i18n;
pub use elwindui_macros::{class, component, new, viewmodel};

/// See the `backend-appkit`/`backend-winui3` re-export below — `elwindui-backend-gtk4` is still an
/// empty stub (no `builtins`/`platform`/`application` of its own yet), so there is no
/// `backend-gtk4` arm here.
#[cfg(feature = "backend-appkit")]
pub use elwindui_backend_appkit as backend;
#[cfg(feature = "backend-winui3")]
pub use elwindui_backend_winui3 as backend;

/// `elwindui_core::ui`(共通トレイト/仮想 builtin)と、有効化中バックエンドの
/// `builtins`(ネイティブ builtin の DSL 向けラッパー、`WindowImpl`/`ButtonImpl`等)を1つの
/// 名前空間にまとめたもの — `builtins::Window`はどのバックエンドでも`elwindui_core::ui::Window`の
/// 再エクスポートで実体が同一なので、両方の glob import を重ねても衝突しない。
pub mod ui {
    pub use elwindui_core::ui::*;
    #[cfg(feature = "backend-appkit")]
    pub use elwindui_backend_appkit::builtins::*;
    #[cfg(feature = "backend-winui3")]
    pub use elwindui_backend_winui3::builtins::*;
}

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
