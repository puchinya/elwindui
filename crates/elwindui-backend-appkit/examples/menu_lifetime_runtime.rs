//! Main-thread AppKit regression for the native menu-item ownership boundary.
//!
//! This is intentionally a runnable example rather than a normal `#[test]`: AppKit menu
//! objects require the process main thread, while Rust's test harness executes tests on worker
//! threads. The native `NSMenu::performActionForItemAtIndex` path is used so this fails under the
//! old implementation that added only the raw `NSMenuItem` and immediately dropped the Rust
//! wrapper/`MenuItemTarget`.

#[cfg(target_os = "macos")]
fn main() {
    use elwindui_backend_appkit::{Menu, MenuItem};
    use elwindui_core::ui::{MenuExt, MenuItemExt};
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use std::cell::Cell;
    use std::rc::Rc;

    let mtm =
        MainThreadMarker::new().expect("menu lifetime runtime must run on AppKit main thread");
    let _application = NSApplication::sharedApplication(mtm);
    let menu = Menu::new();
    let callback_count = Rc::new(Cell::new(0_u32));

    let item = MenuItem::new();
    item.set_text("Lifetime regression");
    item.set_enabled(true);
    let callback_count_for_item = Rc::clone(&callback_count);
    item.set_on_select(Box::new(move || {
        callback_count_for_item.set(callback_count_for_item.get() + 1);
    }));

    menu.items()
        .add(Rc::clone(&item) as Rc<dyn elwindui_core::ui::MenuItemExt>);
    drop(item);

    let native_menu = menu.inner_ns();
    let native_item = native_menu
        .itemAtIndex(0)
        .expect("menu must retain its native item");
    assert!(
        native_item.target().is_some(),
        "native target must remain alive"
    );
    assert!(
        native_item.isEnabled(),
        "explicit enabled state must remain authoritative"
    );

    native_menu.performActionForItemAtIndex(0);
    assert_eq!(
        callback_count.get(),
        1,
        "native menu selection must fire exactly once"
    );

    println!(
        "menu_lifetime_runtime: PASS (native_item_retained=true, callback_count={})",
        callback_count.get()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("menu_lifetime_runtime: NOT RUN (AppKit is macOS-only)");
}
