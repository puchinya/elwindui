//! End-user facing facade crate. See docs/elwindui_gui_framework_design.md §1.
//!
//! A consumer crate needs only `elwindui` itself in `[dependencies]` — `elwindui-codegen`
//! (`compile_dir`/`compile_dir_with_extra_viewmodels`, called from build.rs) and `elwindui-macros`'
//! `#[component]`/`#[viewmodel]` proc-macros both emit generated code that refers exclusively to
//! `elwindui::core::..`/`elwindui::backend::..`/`elwindui::i18n::..` (never `elwindui_core::..`/
//! `elwindui_backend_*::..`/`elwindui_i18n::..` directly), which resolve through the re-exports
//! below regardless of how many crates deep `elwindui` itself pulls them in from.

pub use elwindui_core as core;
pub use elwindui_core::visual_tree;
pub use elwindui_i18n as i18n;
/// SVG loading (`load_svg_file`/`load_svg_bytes`/`load_svg_str`, `SvgLoader`) — backends never
/// depend on this crate directly, only on the `elwindui_core::graphics::VectorImage` it produces
/// (SVG読み込み・ベクター描画対応 実装指示書§1.5/§4.3).
#[cfg(feature = "svg")]
pub use elwindui_svg as svg;
/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` — writes
/// a `component`+`view` pair (docs/elwindui_spec.md §3/§13) as a single ordinary Rust `struct`,
/// alongside `#[elwindui::viewmodel] mod foo { struct Foo { .. } impl Foo { .. } }` for the
/// viewmodel half (付録O.2). `view!` is not a real macro (never invoked/expanded — see
/// `elwindui_macros::component`'s own doc comment); its tokens are read as `.elwind`-DSL text.
pub use elwindui_macros::{class, component, main, viewmodel};

/// Initializes the native UI runtime selected for the current operating system.
///
/// Call this once at the start of `main`, before creating a window. On Windows it activates the
/// Windows App SDK dynamic dependency required by WinUI 3 and Win2D; the other native backends
/// currently need no process-wide bootstrap.
#[derive(Debug, Clone)]
pub struct InitError(String);

impl std::fmt::Display for InitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for InitError {}

#[cfg(all(target_os = "windows", feature = "backend-winui3"))]
pub fn init() -> Result<(), InitError> {
    elwindui_backend_winui3::init().map_err(|error| InitError(error.to_string()))
}

#[cfg(any(
    all(target_os = "macos", feature = "backend-appkit"),
    all(target_os = "linux", feature = "backend-gtk4"),
))]
pub fn init() -> Result<(), InitError> {
    #[cfg(all(target_os = "macos", feature = "backend-appkit"))]
    return elwindui_backend_appkit::init().map_err(|error| InitError(error.to_string()));
    #[cfg(all(target_os = "linux", feature = "backend-gtk4"))]
    return elwindui_backend_gtk4::init().map_err(|error| InitError(error.to_string()));
}

#[cfg(all(target_os = "macos", feature = "backend-appkit"))]
pub use elwindui_backend_appkit as backend;
#[cfg(all(target_os = "windows", feature = "backend-winui3"))]
pub use elwindui_backend_winui3 as backend;
#[cfg(all(target_os = "linux", feature = "backend-gtk4"))]
pub use elwindui_backend_gtk4 as backend;

#[cfg(all(target_os = "macos", not(feature = "backend-appkit")))]
compile_error!("elwindui on macOS requires the `backend-appkit` feature");
#[cfg(all(target_os = "windows", not(feature = "backend-winui3")))]
compile_error!("elwindui on Windows requires the `backend-winui3` feature");
#[cfg(all(target_os = "linux", not(feature = "backend-gtk4")))]
compile_error!("elwindui on Linux requires the `backend-gtk4` feature");
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("elwindui supports only macOS, Windows, and Linux");

/// `elwindui_core::ui`(共通トレイト/仮想 builtin)と、有効化中バックエンドの
/// ネイティブ builtin 実装(`Window`/`Button`等)を1つの名前空間にまとめたもの。
/// `elwindui-backend-appkit`/`elwindui-backend-winui3`はいずれも`native_ui`モジュール(非公開)の
/// 内容をクレートルート直下に再エクスポートしている(各クレートの`src/lib.rs`参照)ため、
/// ここではそのクレートルートを丸ごとglobする。
pub mod ui {
    #[cfg(all(target_os = "macos", feature = "backend-appkit"))]
    pub use elwindui_backend_appkit::*;
    #[cfg(all(target_os = "windows", feature = "backend-winui3"))]
    pub use elwindui_backend_winui3::*;
    #[cfg(all(target_os = "linux", feature = "backend-gtk4"))]
    pub use elwindui_backend_gtk4::*;
    pub use elwindui_core::ui::*;
}

/// `platform::clipboard`/`platform::file_dialog` etc. See docs/elwindui_spec.md 付録T.
#[cfg(all(target_os = "macos", feature = "backend-appkit"))]
pub mod platform {
    pub use elwindui_backend_appkit::platform::file_dialog;
}

/// See the `backend-appkit` `platform` module above. `elwindui-backend-winui3` is best-effort/
/// unverified (see its crate-level doc comment) — not built or run on a real Windows machine.
#[cfg(all(target_os = "windows", feature = "backend-winui3"))]
pub mod platform {
    pub use elwindui_backend_winui3::platform::file_dialog;
}

/// `application::run(startup)` initializes the native GUI loop and invokes `startup` on its UI
/// thread. `init()` and `run()` must be called from that same OS thread.
#[cfg(all(target_os = "macos", feature = "backend-appkit"))]
pub mod application {
    pub use elwindui_backend_appkit::application::run;
}

/// See the `backend-appkit` `application` module above. `elwindui-backend-winui3` is best-effort/
/// unverified (see its crate-level doc comment) — not built or run on a real Windows machine.
#[cfg(all(target_os = "windows", feature = "backend-winui3"))]
pub mod application {
    pub use elwindui_backend_winui3::application::run;
}

#[cfg(all(target_os = "linux", feature = "backend-gtk4"))]
pub mod application {
    pub use elwindui_backend_gtk4::application::run;
}
