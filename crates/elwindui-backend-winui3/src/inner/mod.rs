//! Raw per-control XAML plumbing — every type here is `Inner`-prefixed and private to
//! this crate. `native_ui` composes these as plain fields and calls into them.
//!
//! One file per control family; see `lib.rs` for the crate's layering.

mod button;
mod check_box;
mod dropdown;
mod dropdown_item;
pub(crate) mod menu;
pub(crate) mod popup;
mod radio_button;
mod scroll_view;
mod slider;
mod tab_view;
mod text;
mod toggle_switch;
mod window;

pub(crate) use button::InnerButton;
pub(crate) use check_box::InnerCheckBox;
pub(crate) use dropdown::InnerDropdown;
pub(crate) use dropdown_item::InnerDropdownItem;
pub(crate) use menu::{InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuFlyout, InnerMenuItem};
pub use popup::{WinUI3PopupHandle, WinUI3PopupHost};
pub(crate) use radio_button::InnerRadioButton;
pub(crate) use scroll_view::InnerScrollView;
pub(crate) use slider::InnerSlider;
pub(crate) use tab_view::InnerTabView;
pub(crate) use text::{InnerPasswordBox, InnerTextArea, InnerTextBox};
pub(crate) use toggle_switch::InnerToggleSwitch;
pub(crate) use window::InnerWindow;

use std::cell::RefCell;
use std::rc::Rc;

/// A replaceable callback slot shared between an `Inner*` widget and its registered native-event
/// closure (Issue #313). The slot holds an `Rc`, so [`invoke_slot`] can clone the callback out and
/// release the borrow before calling it: the callback may replace or clear this same slot (for
/// example when it drops the last owner and the peer's `dispose` runs) without a `RefCell`
/// double-borrow panic.
pub(crate) type CallbackSlot<T> = Rc<RefCell<Option<Rc<dyn Fn(T)>>>>;

pub(crate) fn new_callback_slot<T>() -> CallbackSlot<T> {
    Rc::new(RefCell::new(None))
}

/// Calls the slot's current callback, if any, without holding the slot borrowed during the call.
pub(crate) fn invoke_slot<T>(slot: &RefCell<Option<Rc<dyn Fn(T)>>>, value: T) {
    let callback = slot.borrow().clone();
    if let Some(callback) = callback {
        callback(value);
    }
}
