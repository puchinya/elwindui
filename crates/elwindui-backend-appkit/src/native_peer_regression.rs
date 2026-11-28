//! Executable main-thread regression for the AppKit native peers of Issue #313 (PRs #319/#320),
//! run on a real macOS host for Issue #316.
//!
//! AppKit objects require the process main thread, while Rust's test harness runs tests on worker
//! threads, so this runs from the `native_peer_runtime` example through `application::run`.
//! Each check prints one `PASS` line; any failure panics.

use crate::native_ui::__IntoAnyView;
use crate::native_ui::tab_view::AppKitTabViewPeer;
use elwindui_core::ui::*;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{Message, msg_send};
use objc2_app_kit::{NSControl, NSMenu, NSPopUpButton, NSScrollView, NSTextView};
use objc2_foundation::{NSNotification, NSString};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The native view behind a Core control, viewed as `T`.
fn native<C: NativeControlExt + ?Sized, T: Message>(control: &Rc<C>) -> Retained<T> {
    let view = control.into_any_view().as_nsview();
    // SAFETY: each caller names the class the AppKit peer of that control creates.
    unsafe { Retained::cast_unchecked::<T>(view) }
}

/// Sends `perform:` to the control's current target without holding our own reference to the
/// target during the call, as AppKit's action dispatch does.
fn fire_target(control: &NSControl) {
    let Some(target) = control.target() else {
        return;
    };
    let raw: *const AnyObject = Retained::as_ptr(&target);
    drop(target);
    unsafe {
        let _: () = msg_send![&*raw, perform: control];
    }
}

/// Calls `selector` on `delegate` with a notification, without holding our own reference to the
/// delegate during the call.
fn fire_delegate(
    delegate: Option<Retained<AnyObject>>,
    name: &str,
    object: &AnyObject,
    text: bool,
) {
    let Some(delegate) = delegate else {
        return;
    };
    let raw: *const AnyObject = Retained::as_ptr(&delegate);
    drop(delegate);
    let note = unsafe {
        NSNotification::notificationWithName_object(&NSString::from_str(name), Some(object))
    };
    unsafe {
        if text {
            let _: () = msg_send![&*raw, textDidChange: &*note];
        } else {
            let _: () = msg_send![&*raw, controlTextDidChange: &*note];
        }
    }
}

/// A handler that releases the last owner of its own control mid-callback must neither crash nor
/// panic, and a later native event is a no-op (PR #319 review fix: `_in_flight` retains).
fn input_controls_release_themselves_mid_callback() {
    // Slider (NSSlider target).
    {
        let holder: Rc<RefCell<Option<Rc<Slider>>>> = Rc::new(RefCell::new(None));
        let fired = Rc::new(Cell::new(0_u32));
        let slider = Slider::new();
        slider.set_max(1.0);
        let ns: Retained<NSControl> = native(&slider);
        let (holder_cb, fired_cb) = (Rc::clone(&holder), Rc::clone(&fired));
        slider.set_on_value_change(Box::new(move |_| {
            fired_cb.set(fired_cb.get() + 1);
            holder_cb.borrow_mut().take();
        }));
        let weak = Rc::downgrade(&slider);
        *holder.borrow_mut() = Some(slider);
        ns.setDoubleValue(0.5);
        fire_target(&ns);
        assert_eq!(fired.get(), 1, "Slider handler fires once");
        assert!(
            weak.upgrade().is_none(),
            "Slider released by its own handler"
        );
        ns.setDoubleValue(0.25);
        fire_target(&ns);
        assert_eq!(fired.get(), 1, "a later Slider event is a no-op");
    }
    // ToggleSwitch (NSSwitch target).
    {
        let holder: Rc<RefCell<Option<Rc<ToggleSwitch>>>> = Rc::new(RefCell::new(None));
        let fired = Rc::new(Cell::new(0_u32));
        let toggle = ToggleSwitch::new();
        let ns: Retained<objc2_app_kit::NSSwitch> = native(&toggle);
        let (holder_cb, fired_cb) = (Rc::clone(&holder), Rc::clone(&fired));
        toggle.set_on_is_on_change(Box::new(move |_| {
            fired_cb.set(fired_cb.get() + 1);
            holder_cb.borrow_mut().take();
        }));
        let weak = Rc::downgrade(&toggle);
        *holder.borrow_mut() = Some(toggle);
        ns.setState(objc2_app_kit::NSControlStateValueOn);
        fire_target(&ns);
        assert_eq!(fired.get(), 1, "ToggleSwitch handler fires once");
        assert!(
            weak.upgrade().is_none(),
            "ToggleSwitch released by its own handler"
        );
        ns.setState(objc2_app_kit::NSControlStateValueOff);
        fire_target(&ns);
        assert_eq!(fired.get(), 1, "a later ToggleSwitch event is a no-op");
    }
    // TextBox and PasswordBox (NSTextField delegate).
    fn text_field<C: NativeControlExt + 'static>(
        control: Rc<C>,
        set_handler: impl FnOnce(&C, Box<dyn Fn(String)>),
        label: &str,
    ) {
        let holder: Rc<RefCell<Option<Rc<C>>>> = Rc::new(RefCell::new(None));
        let fired = Rc::new(Cell::new(0_u32));
        let field: Retained<objc2_app_kit::NSTextField> = native(&control);
        let (holder_cb, fired_cb) = (Rc::clone(&holder), Rc::clone(&fired));
        set_handler(
            &control,
            Box::new(move |_| {
                fired_cb.set(fired_cb.get() + 1);
                holder_cb.borrow_mut().take();
            }),
        );
        let weak = Rc::downgrade(&control);
        *holder.borrow_mut() = Some(control);
        let delegate = |field: &objc2_app_kit::NSTextField| unsafe {
            field
                .delegate()
                .map(|d| Retained::cast_unchecked::<AnyObject>(d))
        };
        field.setStringValue(&NSString::from_str("abc"));
        fire_delegate(
            delegate(&field),
            "NSControlTextDidChangeNotification",
            &field,
            false,
        );
        assert_eq!(fired.get(), 1, "{label} handler fires once");
        assert!(
            weak.upgrade().is_none(),
            "{label} released by its own handler"
        );
        field.setStringValue(&NSString::from_str("abcd"));
        fire_delegate(
            delegate(&field),
            "NSControlTextDidChangeNotification",
            &field,
            false,
        );
        assert_eq!(fired.get(), 1, "a later {label} event is a no-op");
    }
    text_field(TextBox::new(), |c, h| c.set_on_text_change(h), "TextBox");
    text_field(
        PasswordBox::new(),
        |c, h| c.set_on_password_change(h),
        "PasswordBox",
    );
    // TextArea (NSTextView delegate inside an NSScrollView).
    {
        let holder: Rc<RefCell<Option<Rc<TextArea>>>> = Rc::new(RefCell::new(None));
        let fired = Rc::new(Cell::new(0_u32));
        let area = TextArea::new();
        let scroll: Retained<NSScrollView> = native(&area);
        let text_view: Retained<NSTextView> = unsafe {
            Retained::cast_unchecked(scroll.documentView().expect("TextArea document view"))
        };
        let (holder_cb, fired_cb) = (Rc::clone(&holder), Rc::clone(&fired));
        area.set_on_text_change(Box::new(move |_| {
            fired_cb.set(fired_cb.get() + 1);
            holder_cb.borrow_mut().take();
        }));
        let weak = Rc::downgrade(&area);
        *holder.borrow_mut() = Some(area);
        let delegate = |tv: &NSTextView| unsafe {
            tv.delegate()
                .map(|d| Retained::cast_unchecked::<AnyObject>(d))
        };
        text_view.setString(&NSString::from_str("abc"));
        fire_delegate(
            delegate(&text_view),
            "NSTextDidChangeNotification",
            &text_view,
            true,
        );
        assert_eq!(fired.get(), 1, "TextArea handler fires once");
        assert!(
            weak.upgrade().is_none(),
            "TextArea released by its own handler"
        );
        text_view.setString(&NSString::from_str("abcd"));
        fire_delegate(
            delegate(&text_view),
            "NSTextDidChangeNotification",
            &text_view,
            true,
        );
        assert_eq!(fired.get(), 1, "a later TextArea event is a no-op");
    }
    eprintln!(
        "native_peer_runtime: PASS in_flight_self_release (Slider, ToggleSwitch, TextBox, PasswordBox, TextArea)"
    );
}

fn titles(menu: &NSMenu) -> Vec<String> {
    (0..menu.numberOfItems())
        .map(|index| {
            menu.itemAtIndex(index)
                .expect("native entry")
                .title()
                .to_string()
        })
        .collect()
}

/// The native `NSMenu` order follows the Core list after `insert(0, X)` and a reorder
/// (PR #320 review fix).
fn menu_order_follows_core_list() {
    let item = |text: &str| {
        let item = MenuItem::new();
        item.set_text(text);
        item
    };
    let menu = Menu::new();
    menu.add(item("A"));
    menu.add(item("B"));
    let ns = __menu_peer_of(&*menu)
        .as_any()
        .downcast_ref::<crate::native_ui::menu::AppKitMenuPeer>()
        .expect("AppKit menu peer")
        .ns();
    assert_eq!(titles(&ns), ["A", "B"]);
    ListExt::insert(&*menu, 0, item("X"));
    assert_eq!(titles(&ns), ["X", "A", "B"], "Menu insert(0, X)");
    let first = ListExt::remove_at(&*menu, 0);
    menu.add(first);
    assert_eq!(titles(&ns), ["A", "B", "X"], "Menu reorder");

    let bar_item = |text: &str| {
        let item = MenuBarItem::new();
        item.set_text(text);
        item.set_submenu(Menu::new());
        item
    };
    let bar = MenuBar::new();
    bar.add(bar_item("File"));
    bar.add(bar_item("Edit"));
    let bar_peer = __menu_bar_peer_of(&*bar);
    let bar_ns = bar_peer
        .as_any()
        .downcast_ref::<crate::native_ui::menu::AppKitMenuBarPeer>()
        .expect("AppKit menu bar peer")
        .inner
        .ns
        .clone();
    // Index 0 is the platform app menu the AppKit menu bar always provides.
    let user_titles = |ns: &NSMenu| titles(ns)[1..].to_vec();
    assert_eq!(user_titles(&bar_ns), ["File", "Edit"]);
    ListExt::insert(&*bar, 0, bar_item("View"));
    assert_eq!(
        user_titles(&bar_ns),
        ["View", "File", "Edit"],
        "MenuBar insert(0, X)"
    );
    let first = ListExt::remove_at(&*bar, 0);
    bar.add(first);
    assert_eq!(
        user_titles(&bar_ns),
        ["File", "Edit", "View"],
        "MenuBar reorder"
    );
    eprintln!("native_peer_runtime: PASS menu_order (Menu, MenuBar: insert(0) and reorder)");
}

/// `TabView::set_children` reorder: the native chip order follows the list, every tab keeps its
/// content host, and the visible host is the one of `selected_index` (PR #320 review fix). A
/// realized TabView is released when its last owner drops (chip callbacks hold it weakly).
fn tab_view_reorder_keeps_hosts_and_selection() {
    let tab = |header: &str| {
        let item = TabViewItem::new();
        item.set_header(header);
        item.set_content(VerticalLayout::new() as Rc<dyn UIElementExt>);
        item
    };
    let (t1, t2, t3) = (tab("T1"), tab("T2"), tab("T3"));
    let tab_view = TabView::new();
    tab_view.set_children(vec![t1.clone(), t2.clone(), t3.clone()]);
    tab_view.set_selected_index(1);
    let peer = tab_view.__tab_view_peer();
    let snapshot = || {
        peer.as_any()
            .downcast_ref::<AppKitTabViewPeer>()
            .expect("AppKit TabView peer")
            .regression_snapshot()
    };
    let before = snapshot();
    let titles = |s: &[(String, usize, bool)]| s.iter().map(|c| c.0.clone()).collect::<Vec<_>>();
    let host_of =
        |s: &[(String, usize, bool)], title: &str| s.iter().find(|c| c.0 == title).expect("chip").1;
    let visible = |s: &[(String, usize, bool)]| {
        s.iter()
            .filter(|c| c.2)
            .map(|c| c.0.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(&before), ["T1", "T2", "T3"]);
    assert_eq!(visible(&before), ["T2"]);

    tab_view.set_children(vec![t3.clone(), t1.clone(), t2.clone()]);
    let after = snapshot();
    assert_eq!(
        titles(&after),
        ["T3", "T1", "T2"],
        "chip order follows the list"
    );
    for title in ["T1", "T2", "T3"] {
        assert_eq!(
            host_of(&before, title),
            host_of(&after, title),
            "{title} keeps its content host"
        );
    }
    let selected = tab_view.selected_index();
    let selected_header = [&t3, &t1, &t2][selected].header();
    assert_eq!(
        visible(&after),
        [selected_header],
        "the only visible host is the tab at selected_index"
    );

    drop(peer);
    let weak = Rc::downgrade(&tab_view);
    drop(tab_view);
    assert!(weak.upgrade().is_none(), "a realized TabView is released");
    eprintln!(
        "native_peer_runtime: PASS tab_view_reorder (chips {:?}, hosts kept, visible {:?}, released)",
        titles(&after),
        visible(&after)
    );
}

/// Dropdown: the native selection matches Core's `selected_index` after items are added and
/// removed, and a native pick reaches the two-way value once.
fn dropdown_selection_survives_item_changes() {
    let item = |text: &str| {
        let item = DropdownItem::new();
        item.set_text(text);
        item
    };
    let dropdown = Dropdown::new();
    dropdown.set_items(vec![item("S"), item("M"), item("L")]);
    dropdown.set_selected_index(1);
    let ns: Retained<NSPopUpButton> = native(&dropdown);
    // Core keeps the selected index across item changes; the rebuilt native list must show it.
    let check = |step: &str| {
        assert_eq!(
            ns.indexOfSelectedItem(),
            1,
            "native selection stays on Core's index 1 after {step}"
        );
    };
    check("initial");
    dropdown.add(item("XL"));
    check("add");
    ListExt::remove_at(&*dropdown, 3);
    check("remove");

    let picks = Rc::new(RefCell::new(Vec::new()));
    let picks_cb = Rc::clone(&picks);
    dropdown.set_on_selected_index_change(Box::new(move |i| picks_cb.borrow_mut().push(i)));
    ns.selectItemAtIndex(2);
    fire_target(&ns);
    assert_eq!(*picks.borrow(), [2], "a native pick reports once");
    dropdown.add(item("XL"));
    assert_eq!(
        ns.indexOfSelectedItem(),
        2,
        "the picked index survives a later add"
    );
    eprintln!("native_peer_runtime: PASS dropdown_selection (add/remove keep index, pick=2 once)");
}

/// Issue #329: an unmodified Return reaching a first-responder `TreeHost` (outer or nested) fires
/// the window's `is_default` button exactly once; a modified Return or another key does not, and a
/// window without a default button ignores Return.
fn default_button_answers_return_from_tree_host() {
    use crate::host::TreeHost;
    use objc2::MainThreadOnly;
    use objc2_app_kit::{
        NSBackingStoreType, NSButton, NSEvent, NSEventModifierFlags, NSEventType, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let mtm = crate::ffi::mtm();
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(240.0, 120.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let outer = TreeHost::new();
    let nested = TreeHost::new();
    window.setContentView(Some(&outer));
    outer.addSubview(&nested);

    let key = |code: u16, chars: &str, flags: NSEventModifierFlags| -> Retained<NSEvent> {
        let chars = NSString::from_str(chars);
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            window.windowNumber(),
            None,
            &chars,
            &chars,
            false,
            code,
        )
        .expect("synthetic key event")
    };
    let press = |host: &TreeHost, event: &NSEvent| {
        assert!(
            window.makeFirstResponder(Some(host)),
            "host becomes first responder"
        );
        unsafe {
            let _: () = msg_send![host, keyDown: event];
        }
    };
    let none = NSEventModifierFlags::empty();

    // No default button yet: Return is a no-op.
    press(&outer, &key(36, "\r", none));

    let clicks = Rc::new(Cell::new(0_u32));
    let button = Button::new();
    button.set_text("OK");
    button.set_is_default(true);
    let clicks_cb = Rc::clone(&clicks);
    button.set_on_click(Box::new(move || clicks_cb.set(clicks_cb.get() + 1)));
    let ns: Retained<NSButton> = native(&button);
    outer.addSubview(&ns);

    press(&outer, &key(36, "\r", none));
    assert_eq!(
        clicks.get(),
        1,
        "Return on the outer host fires the default button once"
    );
    press(&nested, &key(36, "\r", none));
    assert_eq!(
        clicks.get(),
        2,
        "Return on a nested host fires the default button once"
    );
    press(&outer, &key(36, "\r", NSEventModifierFlags::Shift));
    press(&outer, &key(36, "\r", NSEventModifierFlags::Command));
    press(&outer, &key(0, "a", none));
    press(&outer, &key(53, "\u{1b}", none));
    assert_eq!(
        clicks.get(),
        2,
        "modified Return and other keys do not fire it"
    );

    button.set_is_default(false);
    press(&outer, &key(36, "\r", none));
    assert_eq!(
        clicks.get(),
        2,
        "a button that is no longer default ignores Return"
    );

    window.close();
    eprintln!(
        "native_peer_runtime: PASS default_button_return (outer=1, nested=1, filtered=4, unset=0)"
    );
}

pub(crate) fn run_all() {
    input_controls_release_themselves_mid_callback();
    menu_order_follows_core_list();
    tab_view_reorder_keeps_hosts_and_selection();
    dropdown_selection_survives_item_changes();
    default_button_answers_return_from_tree_host();
}
