use super::core::ui::{LayoutExt, UIElementExt};
use super::{CloseButtonPresentation, CustomTabViewItem, TabStripPosition};
use std::rc::Rc;

/// Private presenter that owns the ordered tab-header controls and delegates layout to
/// `HorizontalLayout`.
#[elwindui::component(inherits HorizontalLayout)]
pub(crate) struct CustomTabStripPresenter {
    #[prop(default = Vec::new())]
    items: Vec<Rc<CustomTabViewItem>>,
    #[prop(default = 0)]
    selected_index: usize,
    #[prop(default = TabStripPosition::Top)]
    tab_strip_position: TabStripPosition,
    #[prop(default = false)]
    compact: bool,
    #[prop(default = CloseButtonPresentation::Always)]
    close_button_presentation: CloseButtonPresentation,
    #[state(default = Vec::new())]
    bound_items: Vec<std::rc::Weak<CustomTabViewItem>>,
    #[state(default = None)]
    last_presented_selected_index: Option<usize>,
    #[state(default = None)]
    last_presented_tab_strip_position: Option<TabStripPosition>,
    #[state(default = None)]
    last_presented_compact_tabs: Option<bool>,
    #[state(default = None)]
    last_presented_close_button_presentation: Option<CloseButtonPresentation>,
    body: view! {
        on_mount {
            this.reconcile_items();
        }
        on_update(items, selected_index, tab_strip_position, compact, close_button_presentation) {
            this.sync_property_update();
        }
    },
}

impl CustomTabStripPresenter {
    fn sync_property_update(&self) {
        let items = self.items();
        let unchanged = self.bound_items().len() == items.len()
            && self
                .bound_items()
                .iter()
                .zip(items.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)));
        if unchanged {
            let selected = self.selected_index();
            let position = self.tab_strip_position();
            let compact = self.compact();
            let close = self.close_button_presentation();
            if self.last_presented_tab_strip_position() == Some(position)
                && self.last_presented_compact_tabs() == Some(compact)
                && self.last_presented_close_button_presentation() == Some(close)
                && self.last_presented_selected_index() != Some(selected)
            {
                self.sync_selection_only(&items, self.last_presented_selected_index(), selected);
            } else {
                self.sync_items(&items);
            }
        } else {
            self.reconcile_items();
        }
    }

    pub(crate) fn reconcile_items(&self) {
        let items = self.items();
        let unchanged = self.bound_items().len() == items.len()
            && self
                .bound_items()
                .iter()
                .zip(items.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)));
        if !unchanged {
            LayoutExt::children(self).clear();
            for item in &items {
                let visual: Rc<dyn UIElementExt> = item.clone();
                LayoutExt::children(self).add(visual);
            }
            self.set_bound_items(items.iter().map(Rc::downgrade).collect());
        }
        self.sync_items(&items);
    }

    fn sync_items(&self, items: &[Rc<CustomTabViewItem>]) {
        let selected = self.selected_index();
        let position = self.tab_strip_position();
        let compact = self.compact();
        let presentation = self.close_button_presentation();
        for (index, item) in items.iter().enumerate() {
            item.set_presentation(
                index == selected,
                item.pointer_over(),
                position,
                presentation,
                compact,
            );
        }
        self.set_last_presented_selected_index(Some(selected));
        self.set_last_presented_tab_strip_position(Some(position));
        self.set_last_presented_compact_tabs(Some(compact));
        self.set_last_presented_close_button_presentation(Some(presentation));
    }

    fn sync_selection_only(
        &self,
        items: &[Rc<CustomTabViewItem>],
        previous: Option<usize>,
        selected: usize,
    ) {
        for index in [previous, Some(selected)].into_iter().flatten() {
            if let Some(item) = items.get(index) {
                item.set_presentation(
                    index == selected,
                    item.pointer_over(),
                    self.tab_strip_position(),
                    self.close_button_presentation(),
                    self.compact(),
                );
            }
        }
        self.set_last_presented_selected_index(Some(selected));
        self.set_last_presented_compact_tabs(Some(self.compact()));
    }
}

#[elwindui::component]
impl CustomTabStripPresenter {}
