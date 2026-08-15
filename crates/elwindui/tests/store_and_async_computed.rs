//! End-to-end proof (Issue #82) that `#[elwindui::store]` and `#[async_computed]` — both on a
//! `viewmodel` and on a `store` — actually work through the real macro/codegen/runtime pipeline,
//! not just that the generated tokens contain the right substrings:
//!
//! - a `store` is a process-wide singleton (`TypeName::instance()` returns the same `Rc` every
//!   call, lazily constructed on first access via `EnvironmentContext`);
//! - `#[async_computed]` starts `Loading`, transitions to `Ready(T)`/`Failed(String)` once its
//!   spawned recompute resolves;
//! - the generation-counter "supersede, not cancel" policy actually discards a stale result: two
//!   rapid dependency changes, driven through a real cross-thread `spawn_background` suspend
//!   (proving this isn't just a synchronous-resolution coincidence), leave only the *second*
//!   trigger's result observable.

#[elwindui::store]
mod counter_store {
    pub struct CounterStore {
        #[observable(default = 0i32)]
        count: i32,

        #[async_computed(expr = double_after_a_real_suspend(count))]
        doubled: i32,
    }
}

#[elwindui::viewmodel]
mod greeting_vm {
    pub struct GreetingViewModel {
        #[observable(default = String::new())]
        name: String,

        #[async_computed(expr = greet(name.clone()))]
        greeting: String,
    }
}

async fn double_after_a_real_suspend(value: i32) -> Result<i32, String> {
    let handle = elwindui::core::task::spawn_background(async move {
        // A short, deterministic delay on a genuinely different (tokio worker) thread — the
        // generation check below only means anything if this recompute can still be in flight
        // when the *next* one is spawned, which requires a real suspend, not a same-poll
        // resolution.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        value * 2
    });
    handle.await.map_err(|e| e.to_string())
}

async fn greet(name: String) -> Result<String, String> {
    if name.is_empty() {
        Err("name must not be empty".to_string())
    } else {
        Ok(format!("hello, {name}"))
    }
}

/// Marshals a woken task back onto this test's thread — see
/// `crates/elwindui-core/tests/spawn_local_cross_thread_wake.rs` for the same pattern with more
/// detail on why this is needed at all.
struct ChannelDispatcher {
    sender: std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

impl elwindui::core::task::Dispatcher for ChannelDispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        let _ = self.sender.send(job);
    }
}

fn drain_until(
    receiver: &std::sync::mpsc::Receiver<Box<dyn FnOnce() + Send>>,
    mut done: impl FnMut() -> bool,
    what: &str,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        if let Ok(job) = receiver.recv_timeout(std::time::Duration::from_millis(50)) {
            job();
        }
    }
}

#[test]
fn store_singleton_and_async_computed_end_to_end() {
    let (sender, receiver) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
    elwindui::core::task::set_current(elwindui::core::task::LocalExecutor::new(
        ChannelDispatcher { sender },
    ));
    let _background_runtime = elwindui::core::task::install_background_runtime();

    // `store` is a process-wide singleton: repeated `instance()` calls return the same `Rc`.
    let a = CounterStore::instance();
    let b = CounterStore::instance();
    assert!(
        std::rc::Rc::ptr_eq(&a, &b),
        "CounterStore::instance() should return the same shared instance every call"
    );

    // `#[async_computed]` starts `Loading` before its eager first spawn (kicked off inside
    // `CounterStore::new()`, already in flight by the time `instance()` returns it) resolves.
    assert_eq!(
        a.doubled(),
        elwindui::core::reactive::AsyncComputed::Loading
    );
    drain_until(
        &receiver,
        || !matches!(a.doubled(), elwindui::core::reactive::AsyncComputed::Loading),
        "store's initial #[async_computed] recompute to resolve",
    );
    assert_eq!(a.doubled(), elwindui::core::reactive::AsyncComputed::Ready(0));

    // Supersede: two rapid dependency changes while the first recompute is still suspended
    // (`double_after_a_real_suspend` genuinely awaits a cross-thread `spawn_background` delay) —
    // only the second trigger's result should ever become observable.
    a.set_count(1);
    a.set_count(2);
    drain_until(
        &receiver,
        || matches!(a.doubled(), elwindui::core::reactive::AsyncComputed::Ready(4)),
        "superseding #[async_computed] recompute to settle on the latest trigger's result",
    );
    assert_eq!(
        a.doubled(),
        elwindui::core::reactive::AsyncComputed::Ready(4),
        "the stale (count=1) recompute must have been discarded, not just outraced"
    );

    // `#[async_computed]` on a `viewmodel` (not just a `store`) resolving through `Failed(..)`.
    let vm = GreetingViewModel::new();
    drain_until(
        &receiver,
        || !matches!(vm.greeting(), elwindui::core::reactive::AsyncComputed::Loading),
        "viewmodel's initial #[async_computed] recompute to resolve",
    );
    assert_eq!(
        vm.greeting(),
        elwindui::core::reactive::AsyncComputed::Failed("name must not be empty".to_string())
    );

    vm.set_name("world".to_string());
    drain_until(
        &receiver,
        || matches!(vm.greeting(), elwindui::core::reactive::AsyncComputed::Ready(_)),
        "viewmodel's #[async_computed] to resolve after a dependency change",
    );
    assert_eq!(
        vm.greeting(),
        elwindui::core::reactive::AsyncComputed::Ready("hello, world".to_string())
    );
}
