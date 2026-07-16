use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

/// Modeled on WinUI3's `DispatcherQueue.TryEnqueue`: marshals a closure onto the host's UI
/// thread. Each backend implements this once; see docs/elwindui_spec.md 付録P.5 (WinUI3 →
/// `DispatcherQueue`, AppKit → `DispatchQueue.main`, GTK4 → `glib::MainContext`, egui/iced →
/// the host's own `tokio`/等 runtime). `enqueue`'s job must be `Send`: a `Waker` built on top of
/// this (`LocalExecutor` below) may be woken from any thread — a background `tokio` task
/// finishing, say — so the closure that hops back to the UI thread has to be safely shippable
/// across that boundary, even though once there it only ever touches `!Send` UI state.
pub trait Dispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>);
}

type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;

/// A single-threaded executor for `!Send` futures — `#[command(async)]` bodies, which own
/// `Rc`/`RefCell` component/viewmodel state and so can never be handed to a `Send`-bound
/// executor. Mirrors C#'s `async`/`await` + `SynchronizationContext.Post`: a task starts on the
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

    /// Spawns `fut`, polling it once immediately — most `#[command(async)]` bodies today still
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
/// `application::run()` before entering the platform event loop. Generated `#[command(async)]`
/// bodies never see `D`/`LocalExecutor` directly; they only ever call the backend-agnostic
/// `spawn_local` below.
pub fn set_current<D: Dispatcher + Send + Sync + 'static>(executor: Rc<LocalExecutor<D>>) {
    CURRENT.with(|current| *current.borrow_mut() = Some(executor));
}

fn with_current(f: impl FnOnce(&Rc<dyn ErasedExecutor>)) {
    CURRENT.with(|current| match current.borrow().as_ref() {
        Some(executor) => f(executor),
        None => panic!(
            "elwindui: spawn_local called with no executor installed \
             (application::run() must install one before any #[command(async)] can run)"
        ),
    });
}

/// Spawns `fut` on the current thread's executor (installed via `set_current`). This is what
/// generated `#[command(async)]` bodies call — backend-agnostic, since by the time any component
/// code runs, `application::run()` has already installed the concrete one.
pub fn spawn_local(fut: impl Future<Output = ()> + 'static) {
    let boxed: LocalFuture = Box::pin(fut);
    with_current(move |executor| executor.spawn_local_erased(boxed));
}
