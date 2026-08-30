use super::core;
use super::core::graphics::IconSource;
use super::core::input::PointerEventArgs;
use super::core::layout::Visibility;
use super::core::ui::{ControlExt, IconSourceElementExt, UIElementExt};
use super::{
    CloseButtonPresentation, CustomTabCloseButton, CustomTabCloseButtonExt, TabItemPointerEvent,
    TabStripPosition, weak_self_from_visual_owner,
};
use std::rc::Rc;

/// One item displayed by [`CustomTabView`]. Its visual template is the tab header; its inherited
/// `ContentControl` content remains the logical page presented by the private content presenter.
#[elwindui::component(inherits ContentControl)]
pub struct CustomTabViewItem {
    #[prop(default = String::new())]
    header: String,
    #[prop(default = None)]
    icon: Option<IconSource>,
    #[prop(default = true)]
    closable: bool,
    #[state(default = None)]
    owner_pointer_callback: Option<Rc<dyn Fn(TabItemPointerEvent)>>,
    #[state(default = None)]
    owner_close_callback: Option<Rc<dyn Fn()>>,
    #[state(default = false)]
    header_handlers_bound: bool,
    #[state(default = false)]
    is_selected: bool,
    #[state(default = false)]
    is_pointer_over: bool,
    #[state(default = TabStripPosition::Top)]
    tab_strip_position: TabStripPosition,
    #[state(default = CloseButtonPresentation::Always)]
    close_button_presentation: CloseButtonPresentation,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 0 } else { 1 })]
    header_row: i32,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top { 1 } else { 0 })]
    indicator_row: i32,
    #[computed(expr = if tab_strip_position == TabStripPosition::Top {
        vec![
            elwindui::core::layout::GridLength::Fixed(30.0),
            elwindui::core::layout::GridLength::Fixed(2.0),
        ]
    } else {
        vec![
            elwindui::core::layout::GridLength::Fixed(2.0),
            elwindui::core::layout::GridLength::Fixed(30.0),
        ]
    })]
    header_grid_rows: Vec<elwindui::core::layout::GridLength>,
    #[computed(expr = if icon.is_some() { Visibility::Visible } else { Visibility::Collapsed })]
    icon_visibility: Visibility,
    #[computed(expr = closable && close_button_presentation != CloseButtonPresentation::Never)]
    close_slot_visible: bool,
    #[computed(expr = closable && match close_button_presentation {
        CloseButtonPresentation::Always => true,
        CloseButtonPresentation::OnPointerOver => is_pointer_over,
        CloseButtonPresentation::Never => false,
    })]
    close_glyph_visible: bool,
    #[computed(expr = closable && close_button_presentation == CloseButtonPresentation::Always)]
    initial_close_glyph_visible: bool,
    #[computed(expr = if is_selected { Visibility::Visible } else { Visibility::Collapsed })]
    indicator_visibility: Visibility,
    template: template_view!(|this: Self| {
        on_mount {
            this.bind_header_handlers();
            this.sync_close_button();
        }
        on_update(header, icon, closable, is_selected, tab_strip_position, close_button_presentation) {
            this.sync_close_button();
        }
        let close_button = CustomTabCloseButton {
            slot_visible: close_slot_visible
            glyph_visible: initial_close_glyph_visible
        };
        Grid {
            rows: header_grid_rows
            columns: [
                elwindui::core::layout::GridLength::Fixed(10.0),
                elwindui::core::layout::GridLength::Auto,
                elwindui::core::layout::GridLength::Fixed(10.0),
            ]
            HorizontalLayout {
                Grid::row: header_row
                Grid::column: 1
                height: 30.0
                spacing: 6.0
                IconSourceElement {
                    width: 16.0
                    height: 16.0
                    icon_source: icon
                    visibility: icon_visibility
                }
                TextBlock {
                    text: header
                    text_alignment: elwindui::core::ui::TextAlignment::Center
                }
                close_button
            }
            Rectangle {
                Grid::row: indicator_row
                Grid::column: 1
                fill: "#0078d4"
                visibility: indicator_visibility
            }
        }
    }),
}

#[elwindui::component]
impl CustomTabViewItem {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        true
    }
}

impl CustomTabViewItem {
    /// Creates a tab item with its default presentation properties.
    pub fn new_item() -> Rc<Self> {
        Self::new()
    }

    /// Returns whether this item may be closed by a user gesture.
    pub fn is_closable(&self) -> bool {
        self.closable()
    }

    /// Updates the tab label only when its value changes.
    #[cfg(not(rust_analyzer))]
    pub fn set_header(&self, header: String) {
        if self.header() == header {
            return;
        }
        <Self as CustomTabViewItemExt>::set_header(self, header);
    }

    /// Updates the close capability only when its value changes.
    #[cfg(not(rust_analyzer))]
    pub fn set_closable(&self, closable: bool) {
        if self.closable() == closable {
            return;
        }
        <Self as CustomTabViewItemExt>::set_closable(self, closable);
    }

    pub(crate) fn set_owner_pointer_handler(
        &self,
        callback: Option<Box<dyn Fn(TabItemPointerEvent)>>,
    ) {
        self.set_owner_pointer_callback(callback.map(Rc::from));
    }

    pub(crate) fn set_owner_close_handler(&self, callback: Option<Box<dyn Fn()>>) {
        self.set_owner_close_callback(callback.map(Rc::from));
        self.sync_close_button();
    }

    pub(crate) fn update_pointer_over(&self, value: bool) {
        if self.is_pointer_over() == value {
            return;
        }
        self.set_is_pointer_over(value);
        self.sync_close_button();
    }

    pub(crate) fn set_presentation(
        &self,
        is_selected: bool,
        is_pointer_over: bool,
        tab_strip_position: TabStripPosition,
        close_button_presentation: CloseButtonPresentation,
    ) {
        if self.is_selected() != is_selected {
            self.set_is_selected(is_selected);
        }
        if self.is_pointer_over() != is_pointer_over {
            self.set_is_pointer_over(is_pointer_over);
        }
        let position_changed = self.tab_strip_position() != tab_strip_position;
        if position_changed {
            self.set_tab_strip_position(tab_strip_position);
            self.sync_header_rows();
        }
        if self.close_button_presentation() != close_button_presentation {
            self.set_close_button_presentation(close_button_presentation);
        }
        self.sync_close_button();
    }

    pub(crate) fn pointer_over(&self) -> bool {
        self.is_pointer_over()
    }

    fn sync_header_rows(&self) {
        let Some(root) = self.__template_root() else {
            return;
        };
        let children = root.visual_children();
        if let Some(header) = children.first() {
            header
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.header_row());
        }
        if let Some(indicator) = children.get(1) {
            indicator
                .as_ui_element()
                .set_attached::<i32>("Grid", "row", self.indicator_row());
        }
    }

    fn bind_header_handlers(&self) {
        if self.header_handlers_bound() {
            return;
        }
        let weak_self = self.weak_self();
        if weak_self.upgrade().is_none() {
            return;
        }
        self.set_header_handlers_bound(true);

        let weak_self = weak_self.clone();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Pressed(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Moved(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Released(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |event, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Canceled(*event));
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_entered",
            Box::new(move |_, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        item.update_pointer_over(true);
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Entered);
                        }
                    }
                }
            }),
        );
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_exited",
            Box::new(move |_, args| {
                if !args.handled.get() {
                    if let Some(item) = weak_self.upgrade() {
                        item.update_pointer_over(false);
                        if let Some(callback) = item.owner_pointer_callback() {
                            callback(TabItemPointerEvent::Exited);
                        }
                    }
                }
            }),
        );
    }

    fn sync_close_button(&self) {
        for node in core::visual_tree::find_all::<CustomTabCloseButton>(self) {
            let Some(button) = node.as_any().downcast_ref::<CustomTabCloseButton>() else {
                continue;
            };
            let slot_visible = self.closable()
                && self.close_button_presentation() != CloseButtonPresentation::Never;
            if button.slot_visible() != slot_visible {
                button.set_slot_visible(slot_visible);
            }
            let glyph_visible = self.closable()
                && match self.close_button_presentation() {
                    CloseButtonPresentation::Always => true,
                    CloseButtonPresentation::OnPointerOver => self.is_pointer_over(),
                    CloseButtonPresentation::Never => false,
                };
            if button.glyph_visible() != glyph_visible {
                button.set_glyph_visible(glyph_visible);
            }
            button.set_on_close(self.owner_close_callback());
            button.sync_glyph_paint();
            break;
        }
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        weak_self_from_visual_owner(self)
    }

    /// Resolves the icon into the Core `IconSourceElement` realization used by callers that need a
    /// standalone icon element. The authored header template itself owns its icon element.
    pub fn realize_icon(&self) -> Option<Rc<dyn UIElementExt>> {
        self.icon().map(|icon_source| {
            let icon = core::ui::IconSourceElement::new();
            icon.set_icon_source(Some(icon_source));
            icon as Rc<dyn UIElementExt>
        })
    }

    /// Returns the close affordance from the mounted header template.
    pub fn close_button(&self) -> Rc<dyn UIElementExt> {
        let button = core::visual_tree::find_all::<CustomTabCloseButton>(self)
            .into_iter()
            .next()
            .expect("CustomTabViewItem close button is not mounted");
        button
    }
}
