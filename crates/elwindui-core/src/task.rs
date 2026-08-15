use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

/// Modeled on WinUI3's `DispatcherQueue.TryEnqueue`: marshals a closure onto the host's UI
/// thread. Each backend implements this once; see docs/design/runtime/state_management_design.md (WinUI3 →
/// `DispatcherQueue`, AppKit → `DispatchQueue.main`, GTK4 → `glib::MainContext`, egui/iced →
/// the host's own `tokio`/等 runtime). `enqueue`'s job must be `Send`: a `Waker` built on top of
/// this (`LocalExecutor` below) may be woken from any thread — a background `tokio` task
/// finishing, say — so the closure that hops back to the UI thread has to be safely shippable
/// across that boundary, even though once there it only ever touches `!Send` UI state.
pub trait Dispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>);
}

type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;

/// A single-threaded executor for `!Send` futures — a `viewmodel`'s async action methods (any
/// `async fn` in an `#[elwindui::viewmodel]` `impl` block), which own `Rc`/`RefCell`
/// component/viewmodel state and so can never be handed to a `Send`-bound executor. Mirrors C#'s
/// `async`/`await` + `SynchronizationContext.Post`: a task starts on the
/// UI thread, may genuinely suspend (e.g. awaiting a background `tokio` task's `JoinHandle`), and
/// resumes back on the UI thread — wherever the real work actually happened doesn't matter, since
/// only the `Waker` (never the future itself, never any `Rc`/`RefCell` state) needs to cross
/// threads.
pub struct LocalExecutor<D> {
    dispatcher: Arc<D>,
    tasks: RefCell<HashMap<u64, LocalFuture>>,
    next_id: RefCell<u64>,
}

impl<D: Dispatcher + Send + Sync + 'static> LocalExecutor<D> {
    pub fn new(dispatcher: D) -> Rc<Self> {
        Rc::new(Self {
            dispatcher: Arc::new(dispatcher),
            tasks: RefCell::new(HashMap::new()),
            next_id: RefCell::new(0),
        })
    }

    /// Spawns `fut`, polling it once immediately — most async action bodies today still
    /// resolve synchronously (a modal dialog's `.await` that never really suspends), so this path
    /// costs nothing extra for them. A future that returns `Pending` is kept alive in `tasks` and
    /// resumed later through its `Waker`.
    pub fn spawn_local(&self, fut: impl Future<Output = ()> + 'static) {
        self.spawn_local_boxed(Box::pin(fut));
    }

    fn spawn_local_boxed(&self, fut: LocalFuture) {
        let id = {
            let mut next_id = self.next_id.borrow_mut();
            let id = *next_id;
            *next_id += 1;
            id
        };
        self.tasks.borrow_mut().insert(id, fut);
        self.poll_task(id);
    }

    fn poll_task(&self, id: u64) {
        let Some(mut fut) = self.tasks.borrow_mut().remove(&id) else {
            return; // already completed, or a stale/duplicate wake
        };
        let waker = Waker::from(Arc::new(TaskWaker {
            id,
            dispatcher: self.dispatcher.clone(),
        }));
        let mut cx = Context::from_waker(&waker);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(()) => {}
            Poll::Pending => {
                self.tasks.borrow_mut().insert(id, fut);
            }
        }
    }
}

/// Only ever holds `id` (`Copy`) and `Arc<D>` (`Send + Sync` by construction below) — never the
/// executor itself (an `Rc`), which must stay confined to the UI thread. `wake()`'s closure
/// captures just `id`, so it stays `Send` regardless of how `wake()` is called.
struct TaskWaker<D> {
    id: u64,
    dispatcher: Arc<D>,
}

impl<D: Dispatcher + Send + Sync + 'static> Wake for TaskWaker<D> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let id = self.id;
        self.dispatcher.enqueue(Box::new(move || {
            with_current(|executor| executor.poll_task_erased(id));
        }));
    }
}

/// Object-safe facade over `LocalExecutor<D>` so `CURRENT` (below) can hold one regardless of
/// which concrete `Dispatcher` the active backend uses.
trait ErasedExecutor {
    fn poll_task_erased(&self, id: u64);
    fn spawn_local_erased(&self, fut: LocalFuture);
}

impl<D: Dispatcher + Send + Sync + 'static> ErasedExecutor for LocalExecutor<D> {
    fn poll_task_erased(&self, id: u64) {
        self.poll_task(id);
    }

    fn spawn_local_erased(&self, fut: LocalFuture) {
        self.spawn_local_boxed(fut);
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<dyn ErasedExecutor>>> = const { RefCell::new(None) };
}

/// Installs `executor` as this thread's task executor — called once by a backend's
/// `application::run()` before entering the platform event loop. Generated async action bodies
/// never see `D`/`LocalExecutor` directly; they only ever call the backend-agnostic
/// `spawn_local` below.
pub fn set_current<D: Dispatcher + Send + Sync + 'static>(executor: Rc<LocalExecutor<D>>) {
    CURRENT.with(|current| *current.borrow_mut() = Some(executor));
}

fn with_current(f: impl FnOnce(&Rc<dyn ErasedExecutor>)) {
    CURRENT.with(|current| match current.borrow().as_ref() {
        Some(executor) => f(executor),
        None => panic!(
            "elwindui: spawn_local called with no executor installed \
             (application::run() must install one before any async action can run)"
        ),
    });
}

/// Spawns `fut` on the current thread's executor (installed via `set_current`). This is what
/// generated async action bodies call — backend-agnostic, since by the time any component
/// code runs, `application::run()` has already installed the concrete one.
#[allow(unused_variables)] // rust-analyzer can analyze this with the executor call cfg-disabled.
pub fn spawn_local(fut: impl Future<Output = ()> + 'static) {
    let boxed: LocalFuture = Box::pin(fut);
    with_current(move |executor| executor.spawn_local_erased(boxed));
}

/// Process-wide (not thread-local, unlike `CURRENT`/`LocalExecutor`): a `tokio::runtime::Handle`
/// is `Send + Sync + Clone`, so it can be shared freely with whatever thread a `spawn_background`
/// call happens to run on, unlike `LocalExecutor` which must stay confined to the UI thread.
static BACKGROUND_RUNTIME_HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Installs a process-wide background `tokio` runtime for `spawn_background`. `#[elwindui::main]`
/// (`elwindui_macros::main`) calls this exactly once, immediately after `elwindui::init()` and
/// before `elwindui::application::run(...)` — the same "before any generated/component code runs"
/// point `set_current` is installed at, per each backend's `application::run`. The caller must
/// keep the returned `Runtime` alive for as long as background work should keep running; binding
/// it in a local in `main()`'s own generated body is sufficient, since `main` does not return
/// until `application::run` does, at process exit — worker threads shut down once the `Runtime`
/// value is dropped.
///
/// See docs/design/runtime/state_management_design.md "Async work" for why ElwindUI installs this
/// unconditionally rather than requiring each application to set up its own runtime: every
/// application pays the worker-thread startup cost, even one with no `#[async_computed]` fields,
/// in exchange for `#[async_computed]` working with no manual runtime wiring.
///
/// # Panics
///
/// Panics if called more than once per process.
pub fn install_background_runtime() -> tokio::runtime::Runtime {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("elwindui: failed to start background async runtime");
    BACKGROUND_RUNTIME_HANDLE
        .set(rt.handle().clone())
        .expect("elwindui: install_background_runtime called more than once");
    rt
}

/// Spawns `fut` onto the background runtime installed by `install_background_runtime` (installed
/// automatically by `#[elwindui::main]`). An `#[async_computed]` expression that needs genuine
/// off-thread I/O wraps that work in `spawn_background(..)` and `.await`s the returned
/// `JoinHandle` — `spawn_local`'s executor only ever drives the UI-affine resumption around that
/// `.await`, never the I/O itself. See docs/design/runtime/state_management_design.md "Async
/// work".
///
/// # Panics
///
/// Panics with a descriptive message if no background runtime has been installed yet — mirrors
/// `spawn_local`'s own "no executor installed" panic.
pub fn spawn_background<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    BACKGROUND_RUNTIME_HANDLE
        .get()
        .expect(
            "elwindui: spawn_background called with no background runtime installed \
             (an app using #[elwindui::main] gets one automatically; otherwise call \
             elwindui_core::task::install_background_runtime() once at startup)",
        )
        .spawn(fut)
}
