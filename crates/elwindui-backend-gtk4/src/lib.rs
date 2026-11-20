//! GTK4 (gtk-rs) implementation of `elwindui-core`'s backend traits.
//! See docs/design/README.md and docs/status/backend_status.md.
/// Performs process-wide GTK setup required before creating views.
///
/// The GTK backend is presently a placeholder, but it still participates in the uniform facade
/// initialization contract.
///
/// Registers a native peer factory that implements no native control yet, so realizing one (for
/// example a `Button`) panics with an explicit "unsupported control" diagnostic instead of silently
/// falling back (`docs/design/runtime/native_peer_design.md`).
pub fn init() -> Result<(), std::convert::Infallible> {
    elwindui_core::ui::native::register_native_peer_factory(std::rc::Rc::new(Gtk4PeerFactory));
    Ok(())
}

/// The GTK4 native peer factory. Every `create_*` keeps the trait's default "unsupported" body.
struct Gtk4PeerFactory;

impl elwindui_core::ui::native::NativePeerFactory for Gtk4PeerFactory {
    fn backend_name(&self) -> &'static str {
        "gtk4"
    }
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
