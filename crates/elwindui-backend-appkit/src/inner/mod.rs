//! Raw per-control AppKit plumbing — every type here is `Inner`-prefixed and private to
//! this crate. `native_ui` composes these as plain fields and calls into them.
//!
//! One file per control family; see `lib.rs` for the crate's layering.


mod button;
mod menu;
mod scroll_view;
mod tab_view;
mod text;
mod window;

pub(crate) use button::InnerButton;
pub(crate) use menu::{InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuItem};
pub(crate) use scroll_view::InnerScrollView;
pub(crate) use tab_view::{InnerTabView, TabChipImpl};
pub(crate) use text::{InnerPasswordBox, InnerTextArea, InnerTextBox};
pub(crate) use window::InnerWindow;
