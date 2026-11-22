//! Executable main-thread regression for the AppKit native menu-item ownership boundary.
//!
//! AppKit menu objects require the process main thread, while Rust's test harness executes tests
//! on worker threads, so this is a runnable example. See
//! `elwindui_backend_appkit::run_menu_lifetime_regression`.

fn main() {
    elwindui_backend_appkit::run_menu_lifetime_regression();
}
