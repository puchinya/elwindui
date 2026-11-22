//! The public façade: one class per builtin this backend provides, each implementing the
//! matching `elwindui_core::ui` `*Ext` trait by delegating to its `crate::inner` twin.
//!
//! Deliberately free of AppKit calls — every bit of genuinely toolkit-specific complexity
//! lives one layer down in `inner`. That boundary is why this layer is ~70% identical between
//! the two backends and is the natural candidate if it is ever shared outright.
//!
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no type here at
//! all: they are plain `elwindui_core::ui::UIElement` values that `elwindui-codegen` builds
//! directly, reflected into native views by `crate::host`.

// Deliberately *not* `use elwindui_core::base::AsAny;` here — see the doc comment on
// `MenuBarItem::set_submenu` (the one place that pattern is explained in full) for why importing
// `AsAny` directly, rather than relying on it as `MenuBarItemExt`/`MenuExt`/etc.'s own supertrait,
// silently breaks every `.as_any().downcast_ref::<T>()` call in this file.

pub use elwindui_core::ui::NativeControlExt;
// The submodules' native leaves inherit Core's `NativeControl` (Issue #313).

use elwindui_core::ui::native::{
    ButtonPeer, CheckBoxPeer, DropdownPeer, MenuBarItemPeer, MenuBarPeer, MenuItemPeer, MenuPeer,
    NativePeerFactory, PasswordBoxPeer, RadioButtonPeer, ScrollViewPeer, SliderPeer, TabViewPeer,
    TextAreaPeer, TextBoxPeer, ToggleSwitchPeer, WindowPeer,
};
use std::rc::{Rc, Weak};

/// This backend's [`NativePeerFactory`], registered for the main thread by [`crate::init`].
pub(crate) struct AppKitPeerFactory;

impl NativePeerFactory for AppKitPeerFactory {
    fn backend_name(&self) -> &'static str {
        "appkit"
    }
    fn create_button(&self) -> Rc<dyn ButtonPeer> {
        Rc::new(button::AppKitButtonPeer::new())
    }
    fn create_check_box(&self) -> Rc<dyn CheckBoxPeer> {
        Rc::new(check_box::AppKitCheckBoxPeer::new())
    }
    fn create_radio_button(&self) -> Rc<dyn RadioButtonPeer> {
        Rc::new(radio_button::AppKitRadioButtonPeer::new())
    }
    fn create_slider(&self) -> Rc<dyn SliderPeer> {
        Rc::new(slider::AppKitSliderPeer::new())
    }
    fn create_toggle_switch(&self) -> Rc<dyn ToggleSwitchPeer> {
        Rc::new(toggle_switch::AppKitToggleSwitchPeer::new())
    }
    fn create_text_box(&self) -> Rc<dyn TextBoxPeer> {
        Rc::new(text::AppKitTextBoxPeer::new())
    }
    fn create_text_area(&self) -> Rc<dyn TextAreaPeer> {
        Rc::new(text::AppKitTextAreaPeer::new())
    }
    fn create_password_box(&self) -> Rc<dyn PasswordBoxPeer> {
        Rc::new(text::AppKitPasswordBoxPeer::new())
    }
    fn create_dropdown(&self) -> Rc<dyn DropdownPeer> {
        Rc::new(dropdown::AppKitDropdownPeer::new())
    }
    fn create_scroll_view(&self) -> Rc<dyn ScrollViewPeer> {
        Rc::new(scroll_view::AppKitScrollViewPeer::new())
    }
    fn create_tab_view(&self) -> Rc<dyn TabViewPeer> {
        Rc::new(tab_view::AppKitTabViewPeer::new())
    }
    fn create_menu_item(&self) -> Rc<dyn MenuItemPeer> {
        Rc::new(menu::AppKitMenuItemPeer::new())
    }
    fn create_menu(&self) -> Rc<dyn MenuPeer> {
        Rc::new(menu::AppKitMenuPeer::new())
    }
    fn create_menu_bar_item(&self) -> Rc<dyn MenuBarItemPeer> {
        Rc::new(menu::AppKitMenuBarItemPeer::new())
    }
    fn create_menu_bar(&self) -> Rc<dyn MenuBarPeer> {
        Rc::new(menu::AppKitMenuBarPeer::new())
    }
    fn create_window(&self, owner: Weak<dyn elwindui_core::ui::WindowExt>) -> Rc<dyn WindowPeer> {
        Rc::new(window::AppKitWindowPeer::new(owner))
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
            .expect("an AppKit native peer's handle is an AnyView")
            .clone()
    }
}

mod button;
mod check_box;
mod control;
mod dropdown;
pub(crate) mod menu;
mod radio_button;
mod scroll_view;
mod slider;
mod tab_view;
mod text;
mod toggle_switch;
mod window;
