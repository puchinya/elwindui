use super::core::base::{Rect, Size};
use super::core::reactive::Subscription;
use super::core::ui::{ContentControlExt, ControlExt, UIElementExt};
use super::{CustomTabViewItem, weak_self_from_visual_owner};
use std::rc::Rc;

// This module is private; `pub` only allows the component macro to name the
// state type in generated methods and does not expose it through the crate.
pub struct ContentEntry {
    item: std::rc::Weak<CustomTabViewItem>,
    content: Option<Rc<dyn UIElementExt>>,
    // Retained solely to keep the item-content subscription alive for this entry.
    #[allow(dead_code)]
    subscription: Subscription,
}

/// Private presenter that keeps every tab page content visually attached while arranging only the
/// selected page into the available content rectangle.
#[elwindui::component(inherits Control)]
pub(crate) struct CustomTabContentPresenter {
    #[prop(default = Vec::new())]
    items: Vec<Rc<CustomTabViewItem>>,
    #[prop(default = 0)]
    selected_index: usize,
    #[state(default = Vec::new())]
    bound_items: Vec<std::rc::Weak<CustomTabViewItem>>,
    #[state(default = None)]
    presentation_state: Option<Rc<std::cell::RefCell<Vec<ContentEntry>>>>,
    template: template_view!(|this: Self| {
        on_mount {
            this.reconcile_contents();
        }
        on_update(items, selected_index) {
            this.reconcile_contents();
            this.invalidate_measure();
        }
        Grid {}
    }),
}

impl CustomTabContentPresenter {
    fn state(&self) -> Rc<std::cell::RefCell<Vec<ContentEntry>>> {
        if let Some(state) = self.presentation_state() {
            return state;
        }
        let state = Rc::new(std::cell::RefCell::new(Vec::new()));
        self.set_presentation_state(Some(state.clone()));
        state
    }

    pub(crate) fn reconcile_contents(&self) {
        let items = self.items();
        let unchanged = self.bound_items().len() == items.len()
            && self
                .bound_items()
                .iter()
                .zip(items.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)));
        if unchanged {
            return;
        }

        let state = self.state();
        let old_entries = std::mem::take(&mut *state.borrow_mut());
        for entry in old_entries {
            if let Some(old) = entry.content {
                self.as_ui_element().visual_collection.remove(&old);
            }
        }
        let mut entries = Vec::with_capacity(items.len());
        for item in &items {
            // A tab item is also a templated ContentControl. Prepare its inherited content
            // surface before the stable presenter adopts the page visually; this preserves the
            // logical item parent while switching the content from direct to presenter display.
            item.__prepare_template_presentation();
            let content = item.__content_opt();
            if let Some(content) = content.as_ref() {
                if let Some(parent) = content.visual_parent() {
                    let owner = self.as_ui_element().visual_collection.owner_rc();
                    assert!(
                        owner
                            .as_ref()
                            .is_some_and(|owner| Rc::ptr_eq(&parent, owner)),
                        "CustomTabContentPresenter cannot steal content owned by another visual parent"
                    );
                }
                self.as_ui_element().visual_collection.add(content.clone());
            }
            let weak_presenter: std::rc::Weak<CustomTabContentPresenter> = self.weak_self();
            let weak_item: std::rc::Weak<CustomTabViewItem> = Rc::downgrade(item);
            let subscription = item.__subscribe_content_changed(Rc::new(move |replacement| {
                let presenter: Option<Rc<CustomTabContentPresenter>> = weak_presenter.upgrade();
                let item: Option<Rc<CustomTabViewItem>> = weak_item.upgrade();
                if let (Some(presenter), Some(item)) = (presenter, item) {
                    presenter.replace_item_content(&item, replacement);
                }
            }));
            entries.push(ContentEntry {
                item: Rc::downgrade(item),
                content,
                subscription,
            });
        }
        *state.borrow_mut() = entries;
        self.set_bound_items(items.iter().map(Rc::downgrade).collect());
    }

    fn replace_item_content(
        &self,
        item: &CustomTabViewItem,
        replacement: Option<Rc<dyn UIElementExt>>,
    ) {
        let state = self.state();
        let mut entries = state.borrow_mut();
        let Some(entry) = entries.iter_mut().find(|entry| {
            entry
                .item
                .upgrade()
                .is_some_and(|candidate: Rc<CustomTabViewItem>| {
                    std::ptr::eq(candidate.as_ref(), item)
                })
        }) else {
            return;
        };
        if let Some(old) = entry.content.take() {
            self.as_ui_element().visual_collection.remove(&old);
        }
        if let Some(content) = replacement {
            if let Some(parent) = content.visual_parent() {
                let owner = self.as_ui_element().visual_collection.owner_rc();
                assert!(
                    owner
                        .as_ref()
                        .is_some_and(|owner| Rc::ptr_eq(&parent, owner)),
                    "CustomTabContentPresenter cannot steal replacement content"
                );
            }
            self.as_ui_element().visual_collection.add(content.clone());
            entry.content = Some(content);
        }
        drop(entries);
        self.invalidate_measure();
    }

    fn entries(&self) -> Vec<(usize, Option<Rc<dyn UIElementExt>>)> {
        self.state()
            .borrow()
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, entry.content.clone()))
            .collect()
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        weak_self_from_visual_owner(self)
    }
}

#[elwindui::component]
impl CustomTabContentPresenter {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        self.reconcile_contents();
        if let Some(root) = self.__template_root() {
            root.measure(available);
        }
        let entries = self.entries();
        for (_, content) in &entries {
            if let Some(content) = content {
                content.measure(available);
            }
        }
        entries
            .iter()
            .find(|(index, _)| *index == self.selected_index())
            .and_then(|(_, content)| content.as_ref()?.measured_size())
            .unwrap_or_default()
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        self.reconcile_contents();
        if let Some(root) = self.__template_root() {
            root.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width: final_size.width.max(0.0),
                height: final_size.height.max(0.0),
            });
        }
        for (index, content) in self.entries() {
            let Some(content) = content else {
                continue;
            };
            let rect = if index == self.selected_index() {
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: final_size.width.max(0.0),
                    height: final_size.height.max(0.0),
                }
            } else {
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                }
            };
            content.set_clip_to_bounds(Some(true));
            content.arrange(rect);
        }
        final_size
    }
}
