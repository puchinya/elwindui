use crate::DockItemId;
use crate::core::graphics::IconSource;
use crate::core::ui::{ContentControlExt, UIElementExt};
use elwindui_custom_controls::{CustomTabViewItem, CustomTabViewItemExt};
use std::rc::Rc;

/// An authored document/tool entry in a [`DockGroup`].
#[elwindui::component(inherits ContentControl)]
pub struct DockItem {
    #[prop(default = crate::DockItemId::new(String::new()))]
    id: crate::DockItemId,
    #[prop(default = String::new())]
    title: String,
    #[prop(default = None)]
    icon: Option<IconSource>,
    #[prop(default = true)]
    can_close: bool,
    #[prop(default = true)]
    can_pin: bool,
    #[prop(default = true)]
    can_float: bool,
    #[prop(default = true)]
    can_dock: bool,
    template: template_view!(|_this: Self| { ContentPresenter {} }),
}

#[elwindui::component]
impl DockItem {}

impl DockItem {
    /// Creates an authored dock item in the Created lifecycle state.
    pub fn new_item() -> Rc<Self> {
        Self::new()
    }

    /// Creates the stable tab wrapper used by runtime realization.
    pub fn to_tab_item(&self) -> Rc<CustomTabViewItem> {
        let item = CustomTabViewItem::new_item();
        item.set_header(self.title());
        item.set_icon(self.icon());
        item.set_closable(self.can_close());
        let content = self
            .as_content()
            .expect("DockItem must have authored content before realization");
        item.set_content(content);
        item
    }

    /// Returns this item's authored identity.
    pub fn id_value(&self) -> DockItemId {
        self.id()
    }

    /// Returns the current tab title.
    pub fn title_value(&self) -> String {
        self.title()
    }

    /// Returns the optional authored icon.
    pub fn icon_value(&self) -> Option<IconSource> {
        self.icon()
    }

    /// Returns whether a user may close this item.
    pub fn can_close_value(&self) -> bool {
        self.can_close()
    }

    /// Returns whether a user may pin this item into auto-hide.
    pub fn can_pin_value(&self) -> bool {
        self.can_pin()
    }

    /// Returns whether a user may float this item.
    pub fn can_float_value(&self) -> bool {
        self.can_float()
    }

    /// Returns whether a user may dock this item through a drag.
    pub fn can_dock_value(&self) -> bool {
        self.can_dock()
    }

    pub(crate) fn as_content(&self) -> Option<Rc<dyn UIElementExt>> {
        self.__content_opt()
    }
}
