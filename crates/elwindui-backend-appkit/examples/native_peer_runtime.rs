//! Executable main-thread regression for the AppKit native input, menu, TabView and Dropdown peers.
//!
//! AppKit objects require the process main thread, while Rust's test harness executes tests on
//! worker threads, so this is a runnable example. See
//! `elwindui_backend_appkit::run_native_peer_regression`.

fn main() {
    elwindui_backend_appkit::run_native_peer_regression();
}
