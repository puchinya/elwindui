//! GTK4 (gtk-rs) implementation of `elwindui-core`'s backend traits.
//! See docs/design/README.md and docs/status/backend_status.md.
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
