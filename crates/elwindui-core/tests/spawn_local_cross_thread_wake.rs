//! Verification spike required by Issue #82's approved design before any `#[async_computed]`
//! codegen is built on top of it: proves `LocalExecutor`'s `Poll::Pending` -> `Waker` ->
//! re-poll path is correct for a future that genuinely suspends across a real cross-thread I/O
//! boundary (a `spawn_background`/`tokio` `JoinHandle`), not merely one that resolves on its
//! first poll.
//!
//! Every existing async-action example in this repo only exercises the trivially-resolving case:
//! `examples/notepad`'s `async fn save`/`async fn open` await `platform::file_dialog::{save,open}`,
//! whose own doc comments say a native modal dialog blocks synchronously underneath and so these
//! futures "never actually suspend — they resolve on the first poll." Nothing before this test
//! proved the genuinely-suspending path (a `Waker` invoked from a different OS thread than the one
//! that installed the executor) actually works.

use elwindui_core::task::{self, Dispatcher, LocalExecutor};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Marshals a woken task back onto this test's "UI thread" via a channel — the same shape a real
/// backend's `Dispatcher` (AppKit's `DispatchQueue.main`, WinUI3's `DispatcherQueue`) has, just
/// implemented with `std::sync::mpsc` instead of a native queue.
struct ChannelDispatcher {
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

impl Dispatcher for ChannelDispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        // If the receiver has already been dropped, the "UI thread" has exited — nothing to wake
        // into, matching how a real backend's dispatcher would silently no-op post-shutdown.
        let _ = self.sender.send(job);
    }
}

#[test]
fn spawn_local_wakes_across_a_real_background_thread_suspend() {
    let (sender, receiver) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
    task::set_current(LocalExecutor::new(ChannelDispatcher { sender }));
    let _background_runtime = task::install_background_runtime();

    let done = Arc::new(AtomicBool::new(false));
    let done_for_task = done.clone();

    task::spawn_local(async move {
        let handle = task::spawn_background(async {
            // A deterministic delay on the background runtime's own worker thread — guarantees
            // the outer `JoinHandle::poll` below returns `Pending` on its first poll (run
            // synchronously, inline, by `spawn_local`) rather than racing to resolve before this
            // test can observe the suspend/resume path at all.
            tokio::time::sleep(Duration::from_millis(20)).await;
            42
        });
        let result = handle.await.expect("background task should not panic");
        assert_eq!(result, 42);
        done_for_task.store(true, Ordering::SeqCst);
    });

    // Drive the "UI thread" event loop: run every job the background runtime's `Waker` enqueues
    // (from a tokio worker thread, crossing into this thread via `ChannelDispatcher`), until the
    // task reports completion or this test gives up and fails.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the cross-thread wake to reach the UI-thread dispatcher — \
             LocalExecutor's suspend/resume path appears broken"
        );
        if let Ok(job) = receiver.recv_timeout(Duration::from_millis(50)) {
            job();
        }
    }
}
