//! GTK4 (gtk-rs) implementation of `elwindui-core`'s backend traits.
//! See docs/elwindui_spec.md 付録C, docs/elwindui_gui_framework_design.md §3.
/// Performs process-wide GTK setup required before creating views.
///
/// The GTK backend is presently a placeholder, but it still participates in the uniform facade
/// initialization contract.
pub fn init() -> Result<(), std::convert::Infallible> {
    Ok(())
}

/// Placeholder for GTK4's future application activation callback.
pub mod application {
    pub fn run<F>(startup: F)
    where
        F: FnOnce() + 'static,
    {
        startup();
    }
}
