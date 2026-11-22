//! This backend's native peers (Issue #313, `docs/design/runtime/native_peer_design.md`).
//!
//! Every native builtin (`Button`, `TabView`, `Window`, the menus, ...) is a Core-owned
//! Component in `elwindui_core::ui`. This module provides only the peers that put each one on
//! screen, and the [`WinUi3PeerFactory`] that creates them. Each peer delegates to its
//! `crate::inner` twin, which holds the XAML calls.
//!
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no peer: they are
//! plain `elwindui_core::ui::UIElement` values reflected into native views by `crate::host`.

pub use elwindui_core::ui::NativeControlExt;

use elwindui_core::ui::native::{
    ButtonPeer, CheckBoxPeer, DropdownPeer, MenuBarItemPeer, MenuBarPeer, MenuItemPeer, MenuPeer,
    NativePeerFactory, PasswordBoxPeer, RadioButtonPeer, ScrollViewPeer, SliderPeer, TabViewPeer,
    TextAreaPeer, TextBoxPeer, ToggleSwitchPeer, WindowPeer,
};
use std::rc::{Rc, Weak};

/// This backend's [`NativePeerFactory`], registered for the UI thread by [`crate::init`].
pub(crate) struct WinUi3PeerFactory;

impl NativePeerFactory for WinUi3PeerFactory {
    fn backend_name(&self) -> &'static str {
        "winui3"
    }
    fn create_button(&self) -> Rc<dyn ButtonPeer> {
        Rc::new(button::WinUi3ButtonPeer::new())
    }
    fn create_check_box(&self) -> Rc<dyn CheckBoxPeer> {
        Rc::new(check_box::WinUi3CheckBoxPeer::new())
    }
    fn create_radio_button(&self) -> Rc<dyn RadioButtonPeer> {
        Rc::new(radio_button::WinUi3RadioButtonPeer::new())
    }
    fn create_slider(&self) -> Rc<dyn SliderPeer> {
        Rc::new(slider::WinUi3SliderPeer::new())
    }
    fn create_toggle_switch(&self) -> Rc<dyn ToggleSwitchPeer> {
        Rc::new(toggle_switch::WinUi3ToggleSwitchPeer::new())
    }
    fn create_text_box(&self) -> Rc<dyn TextBoxPeer> {
        Rc::new(text::WinUi3TextBoxPeer::new())
    }
    fn create_text_area(&self) -> Rc<dyn TextAreaPeer> {
        Rc::new(text::WinUi3TextAreaPeer::new())
    }
    fn create_password_box(&self) -> Rc<dyn PasswordBoxPeer> {
        Rc::new(text::WinUi3PasswordBoxPeer::new())
    }
    fn create_dropdown(&self) -> Rc<dyn DropdownPeer> {
        Rc::new(dropdown::WinUi3DropdownPeer::new())
    }
    fn create_scroll_view(&self) -> Rc<dyn ScrollViewPeer> {
        Rc::new(scroll_view::WinUi3ScrollViewPeer::new())
    }
    fn create_tab_view(&self) -> Rc<dyn TabViewPeer> {
        Rc::new(tab_view::WinUi3TabViewPeer::new())
    }
    fn create_menu_item(&self) -> Rc<dyn MenuItemPeer> {
        Rc::new(menu::WinUi3MenuItemPeer::new())
    }
    fn create_menu(&self) -> Rc<dyn MenuPeer> {
        Rc::new(menu::WinUi3MenuPeer::new())
    }
    fn create_menu_bar_item(&self) -> Rc<dyn MenuBarItemPeer> {
        Rc::new(menu::WinUi3MenuBarItemPeer::new())
    }
    fn create_menu_bar(&self) -> Rc<dyn MenuBarPeer> {
        Rc::new(menu::WinUi3MenuBarPeer::new())
    }
    fn create_window(&self, owner: Weak<dyn elwindui_core::ui::WindowExt>) -> Rc<dyn WindowPeer> {
        window::WinUi3WindowPeer::create(owner)
    }
}

/// Converts a native control into this backend's [`crate::AnyView`], realizing its peer if
/// needed.
///
/// Used by generated code (`into_any_view` on a component whose view root is a native builtin)
/// for Core-owned native Components, which cannot name `AnyView` themselves. Controls that still
/// have their own inherent `into_any_view` keep using it, because inherent methods win method
/// resolution.
#[doc(hidden)]
pub trait __IntoAnyView {
    /// Returns the control's native handle.
    fn into_any_view(&self) -> crate::AnyView;
}

impl<T: NativeControlExt + ?Sized> __IntoAnyView for Rc<T> {
    fn into_any_view(&self) -> crate::AnyView {
        self.__native_peer()
            .handle()
            .downcast_ref::<crate::AnyView>()
            .expect("a WinUI 3 native peer's handle is an AnyView")
            .clone()
    }
}

mod button;
mod check_box;
mod control;
mod dropdown;
#[cfg(test)]
pub(crate) mod input_lifecycle_tests;
pub(crate) mod menu;
mod radio_button;
pub(crate) mod scroll_view;
mod slider;
pub(crate) mod tab_view;
mod text;
mod toggle_switch;
pub(crate) mod window;
