//! Native-side AppKit plumbing — every type here is `Inner`-prefixed and, except for `AnyView`
//! itself (re-exported at the crate root; see `lib.rs`'s own doc comment), private to this crate.
//! `native_ui.rs` composes these as plain fields and calls into them; this module owns every bit
//! of genuinely AppKit-specific complexity (NSTextView delegates, tab strip bookkeeping, ...) so
//! `native_ui.rs` stays a thin, uniform "implement the core-side trait by delegating" layer.


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
