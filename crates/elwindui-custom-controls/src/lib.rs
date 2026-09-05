//! Reusable templated custom controls shared by Docking and application code.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

extern crate self as elwindui;

pub use elwindui_macros::{class, component};

pub mod core {
    pub use elwindui_core::*;
}

pub mod ui {
    pub use crate::{
        CustomSplitter, CustomSplitterExt, CustomTabView, CustomTabViewExt, CustomTabViewItem,
        CustomTabViewItemExt,
    };
    pub use elwindui_core::ui::*;
}

mod chrome_icon;
mod custom_splitter;
mod custom_tab_close_button;
mod custom_tab_content_presenter;
mod custom_tab_strip_presenter;
mod custom_tab_view;
mod custom_tab_view_item;
mod support;
mod types;

pub use chrome_icon::{ChromeIcon, chrome_icon};
pub use custom_splitter::{CustomSplitter, CustomSplitterExt};
pub(crate) use custom_tab_close_button::{CustomTabCloseButton, CustomTabCloseButtonExt};
pub(crate) use custom_tab_content_presenter::{
    CustomTabContentPresenter, CustomTabContentPresenterExt,
};
pub(crate) use custom_tab_strip_presenter::{CustomTabStripPresenter, CustomTabStripPresenterExt};
pub use custom_tab_view::{CustomTabView, CustomTabViewExt};
pub use custom_tab_view_item::{CustomTabViewItem, CustomTabViewItemExt};
pub(crate) use support::weak_self_from_visual_owner;
pub use types::{
    CloseButtonPresentation, SplitterDragCompleted, SplitterDragCompletedEventArgs,
    SplitterDragDelta, SplitterDragDeltaEventArgs, SplitterDragStarted,
    SplitterDragStartedEventArgs, TabCloseRequested, TabCloseRequestedEventArgs, TabDragCompleted,
    TabDragCompletedEventArgs, TabDragMoved, TabDragMovedEventArgs, TabDragStarted,
    TabDragStartedEventArgs, TabStripPosition,
};

pub use core::layout::Orientation;
