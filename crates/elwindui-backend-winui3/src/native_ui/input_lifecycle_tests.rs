//! Hosted lifecycle regressions for the input-control peers (Issue #313, PR #319 review).
//!
//! Run by `inner::button::hosted_xaml_regression_tests` inside the single hosted XAML
//! `Application` session: every widget here is a real XAML control.

use super::check_box::WinUi3CheckBoxPeer;
use super::radio_button::WinUi3RadioButtonPeer;
use super::slider::WinUi3SliderPeer;
use super::text::{WinUi3PasswordBoxPeer, WinUi3TextAreaPeer, WinUi3TextBoxPeer};
use super::toggle_switch::WinUi3ToggleSwitchPeer;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    CheckBox as XamlCheckBox, RadioButton as XamlRadioButton, Slider as XamlSlider,
    ToggleSwitch as XamlToggleSwitch,
};
use crate::ffi::{AnyView, input_control_callback_counts};
use elwindui_core::ui::native::{
    CheckBoxPeer, NativePeer, PasswordBoxPeer, RadioButtonPeer, SliderPeer, TextAreaPeer,
    TextBoxPeer, ToggleSwitchPeer,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::Foundation::{IReference, PropertyValue};
use windows::core::Interface;

thread_local! {
    /// The only strong reference to the peer under test, released by its own callback.
    static HELD: RefCell<Option<Rc<dyn NativePeer>>> = const { RefCell::new(None) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

/// What a user handler does when it removes its control from the tree and drops the last
/// owner: Core's `Drop` disposes the peer (which replaces this very callback) and releases it.
fn release_held_peer() {
    CALLS.with(|calls| calls.set(calls.get() + 1));
    let peer = HELD.with(|held| held.borrow_mut().take());
    if let Some(peer) = peer {
        peer.dispose();
        drop(peer);
    }
}

fn element<T: Interface>(peer: &dyn NativePeer) -> T {
    peer.handle()
        .downcast_ref::<AnyView>()
        .expect("WinUI3 peer handle")
        .as_element()
        .cast::<T>()
        .expect("native control type")
}

fn boxed_bool(value: bool) -> Option<IReference<bool>> {
    PropertyValue::CreateBoolean(value)
        .ok()
        .and_then(|value| value.cast::<IReference<bool>>().ok())
}

/// Holds `peer` as the only owner, fires one native event through `fire`, and checks the
/// handler ran once, released every registry entry, and that a later (stale) event on the
/// still-alive XAML control is a no-op.
fn assert_self_release(
    name: &str,
    baseline: (usize, usize, usize, usize),
    peer: Rc<dyn NativePeer>,
    fire: &dyn Fn(),
) {
    CALLS.with(|calls| calls.set(0));
    HELD.with(|held| *held.borrow_mut() = Some(peer));
    fire();
    assert_eq!(CALLS.with(Cell::get), 1, "{name}: the handler runs once");
    assert!(
        HELD.with(|held| held.borrow().is_none()),
        "{name}: the handler released the peer"
    );
    assert_eq!(
        input_control_callback_counts(),
        baseline,
        "{name}: releasing the peer inside its own event removes every registration"
    );
    fire();
    assert_eq!(
        CALLS.with(Cell::get),
        1,
        "{name}: a stale native event after release is a no-op"
    );
}

/// Creating and dropping every input peer (with callbacks installed) returns the callback
/// registries to their baseline: no registration outlives its widget.
pub(crate) fn input_peers_release_every_registration() {
    let baseline = input_control_callback_counts();
    for _ in 0..3 {
        let check_box = WinUi3CheckBoxPeer::new();
        check_box.set_on_change(Some(Box::new(|_| {})));
        let radio = WinUi3RadioButtonPeer::new();
        radio.set_on_click(Some(Box::new(|| {})));
        let slider = WinUi3SliderPeer::new();
        slider.set_on_change(Some(Box::new(|_| {})));
        let toggle = WinUi3ToggleSwitchPeer::new();
        toggle.set_on_change(Some(Box::new(|_| {})));
        let text_box = WinUi3TextBoxPeer::new();
        text_box.set_on_change(Some(Box::new(|_| {})));
        text_box.set_on_submit(Some(Box::new(|| {})));
        let text_area = WinUi3TextAreaPeer::new();
        text_area.set_on_change(Some(Box::new(|_| {})));
        let password = WinUi3PasswordBoxPeer::new();
        password.set_on_change(Some(Box::new(|_| {})));
        assert_ne!(input_control_callback_counts(), baseline);
        for peer in [
            &check_box as &dyn NativePeer,
            &radio,
            &slider,
            &toggle,
            &text_box,
            &text_area,
            &password,
        ] {
            peer.dispose();
        }
    }
    assert_eq!(
        input_control_callback_counts(),
        baseline,
        "dropping input peers must return the callback registries to their baseline"
    );
}

/// The stage ④ peers (Dropdown, ScrollView, TabView, the menus) own every registration they make
/// as well: creating, wiring and dropping them three times returns the registries to baseline.
pub(crate) fn container_peers_release_every_registration() {
    use super::dropdown::WinUi3DropdownPeer;
    use super::menu::{
        WinUi3MenuBarItemPeer, WinUi3MenuBarPeer, WinUi3MenuItemPeer, WinUi3MenuPeer,
    };
    use super::scroll_view::WinUi3ScrollViewPeer;
    use super::tab_view::WinUi3TabViewPeer;
    use elwindui_core::ui::native::{
        DropdownPeer, MenuBarItemPeer, MenuBarPeer, MenuItemPeer, MenuPeer, TabViewPeer,
    };

    let baseline = input_control_callback_counts();
    for _ in 0..3 {
        let dropdown = WinUi3DropdownPeer::new();
        dropdown.set_on_change(Some(Box::new(|_| {})));
        let scroll_view = WinUi3ScrollViewPeer::new();
        let tab_view = WinUi3TabViewPeer::new();
        tab_view.set_on_select(Some(Box::new(|_| {})));
        tab_view.set_on_close(Some(Box::new(|_| {})));
        tab_view.set_on_new_tab(Some(Box::new(|| {})));
        let item: Rc<dyn MenuItemPeer> = Rc::new(WinUi3MenuItemPeer::new());
        item.set_on_select(Some(Box::new(|| {})));
        let menu: Rc<dyn MenuPeer> = Rc::new(WinUi3MenuPeer::new());
        menu.set_items(std::slice::from_ref(&item));
        let bar_item: Rc<dyn MenuBarItemPeer> = Rc::new(WinUi3MenuBarItemPeer::new());
        bar_item.set_submenu(menu.clone());
        let bar = WinUi3MenuBarPeer::new();
        bar.set_items(std::slice::from_ref(&bar_item));
        assert_ne!(input_control_callback_counts(), baseline);
        dropdown.dispose();
        tab_view.dispose();
        item.dispose();
        drop((dropdown, scroll_view, tab_view, item, menu, bar_item, bar));
    }
    assert_eq!(
        input_control_callback_counts(),
        baseline,
        "dropping container and menu peers must return the callback registries to their baseline"
    );
}

/// A handler that releases the last owner of its own control mid-event (dispose replaces the
/// callback slot it is running from) neither panics with a `RefCell` double borrow nor leaks.
pub(crate) fn input_peer_released_by_its_own_event_is_safe() {
    let baseline = input_control_callback_counts();

    let peer = Rc::new(WinUi3CheckBoxPeer::new());
    peer.set_on_change(Some(Box::new(|_| release_held_peer())));
    let xaml: XamlCheckBox = element(peer.as_ref());
    let next = Cell::new(true);
    assert_self_release("CheckBox", baseline, peer, &|| {
        let _ = xaml.SetIsChecked(boxed_bool(next.get()).as_ref());
        next.set(!next.get());
    });

    let peer = Rc::new(WinUi3RadioButtonPeer::new());
    peer.set_on_click(Some(Box::new(release_held_peer)));
    let xaml: XamlRadioButton = element(peer.as_ref());
    let next = Cell::new(true);
    assert_self_release("RadioButton", baseline, peer, &|| {
        let _ = xaml.SetIsChecked(boxed_bool(next.get()).as_ref());
        next.set(!next.get());
    });

    let peer = Rc::new(WinUi3ToggleSwitchPeer::new());
    peer.set_on_change(Some(Box::new(|_| release_held_peer())));
    let xaml: XamlToggleSwitch = element(peer.as_ref());
    let next = Cell::new(true);
    assert_self_release("ToggleSwitch", baseline, peer, &|| {
        let _ = xaml.SetIsOn(next.get());
        next.set(!next.get());
    });

    let peer = Rc::new(WinUi3SliderPeer::new());
    peer.set_on_change(Some(Box::new(|_| release_held_peer())));
    let xaml: XamlSlider = element(peer.as_ref());
    let next = Cell::new(1.0);
    assert_self_release("Slider", baseline, peer, &|| {
        let _ = xaml.SetValue2(next.get());
        next.set(1.0 - next.get());
    });

    // `TextChanged`/`PasswordChanged` are raised asynchronously, so these deliver the change
    // through the exact registry entry the native handler resolves.
    let peer = Rc::new(WinUi3TextBoxPeer::new());
    peer.set_on_change(Some(Box::new(|_| release_held_peer())));
    let fire_id = peer.change_callback_id_for_test();
    assert_self_release("TextBox", baseline, peer, &|| {
        crate::ffi::invoke_ui_event_callback(fire_id);
    });

    let peer = Rc::new(WinUi3TextAreaPeer::new());
    peer.set_on_change(Some(Box::new(|_| release_held_peer())));
    let fire_id = peer.change_callback_id_for_test();
    assert_self_release("TextArea", baseline, peer, &|| {
        crate::ffi::invoke_ui_event_callback(fire_id);
    });

    let peer = Rc::new(WinUi3PasswordBoxPeer::new());
    peer.set_on_change(Some(Box::new(|_| release_held_peer())));
    let fire_id = peer.change_callback_id_for_test();
    assert_self_release("PasswordBox", baseline, peer, &|| {
        crate::ffi::invoke_ui_event_callback(fire_id);
    });
}
