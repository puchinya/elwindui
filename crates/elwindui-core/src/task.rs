/// Modeled on WinUI3's `DispatcherQueue.TryEnqueue`: marshals a completion callback onto the
/// host's UI thread. Each backend implements this once; see docs/elwindui_spec.md 付録P.5
/// (WinUI3 → `DispatcherQueue`, AppKit → `DispatchQueue.main`, GTK4 → `glib::MainContext`,
/// egui/iced → the host's own `tokio`/等 runtime).
pub trait Dispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>);
}

/// Runs `fut` in the background and marshals its completion back onto `dispatcher`.
///
/// Which `Dispatcher` implementation is linked is resolved statically through the generic `D`
/// (not `dyn Dispatcher`): since `target::backend()` is a compile-time constant (付録D), a given
/// build only ever has one concrete `Dispatcher` in scope, so this call monomorphizes to it.
/// See docs/elwindui_spec.md 付録P.5.
pub fn spawn<D: Dispatcher>(
    _dispatcher: &D,
    _fut: impl std::future::Future<Output = ()> + Send + 'static,
) {
    todo!("elwindui-core: spawn not yet implemented")
}
