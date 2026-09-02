use super::core::base::{Rect, Size};
use super::core::reactive::Subscription;
use super::core::ui::{ContentControlExt, ControlExt, UIElementExt};
use super::{CustomTabViewItem, weak_self_from_visual_owner};
#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::collections::HashMap;
use std::rc::Rc;

#[cfg(test)]
thread_local! {
    static CONTENT_STRUCTURAL_RECONCILIATION_COUNTS: RefCell<HashMap<usize, usize>> =
        RefCell::new(HashMap::new());
}

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
    #[state(default = None)]
    last_arranged_selected_index: Option<usize>,
    #[state(default = true)]
    structure_dirty: bool,
    template: template_view!(|this: Self| {
        on_mount {
            this.reconcile_contents();
        }
        on_update(items, selected_index) {
            this.sync_property_update();
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

        #[cfg(test)]
        self.note_structural_reconciliation();

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
            let weak_presenter = self.weak_self();
            let weak_item = Rc::downgrade(item);
            let subscription = item.__subscribe_content_changed(Rc::new(move |replacement| {
                if let (Some(presenter), Some(item)) =
                    (weak_presenter.upgrade(), weak_item.upgrade())
                {
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
        self.set_structure_dirty(true);
        self.set_last_arranged_selected_index(None);
    }

    fn items_match_bound(&self) -> bool {
        let items = self.items();
        self.bound_items().len() == items.len()
            && self
                .bound_items()
                .iter()
                .zip(items.iter())
                .all(|(old, new)| old.upgrade().is_some_and(|old| Rc::ptr_eq(&old, new)))
    }

    fn sync_property_update(&self) {
        if !self.items_match_bound() {
            self.reconcile_contents();
        }
        // A selected page may have a different desired size, but selecting it never changes the
        // retained content list or its subscriptions. The next measure touches only that page.
        self.invalidate_measure();
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
                .is_some_and(|candidate| std::ptr::eq(candidate.as_ref(), item))
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
        self.set_structure_dirty(true);
        self.set_last_arranged_selected_index(None);
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

    #[cfg(test)]
    fn note_structural_reconciliation(&self) {
        let key = self as *const Self as usize;
        CONTENT_STRUCTURAL_RECONCILIATION_COUNTS.with(|counts| {
            let mut counts = counts.borrow_mut();
            *counts.entry(key).or_default() += 1;
        });
    }

    #[cfg(test)]
    pub(crate) fn structural_reconciliation_count_for_test(&self) -> usize {
        let key = self as *const Self as usize;
        CONTENT_STRUCTURAL_RECONCILIATION_COUNTS
            .with(|counts| counts.borrow().get(&key).copied().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CustomTabView;
    use std::cell::{Cell, RefCell};

    #[elwindui_macros::class(inherits = elwindui_core::ui::UIElement)]
    struct LayoutProbe {
        measure_count: Cell<usize>,
        arrange_count: Cell<usize>,
        last_arranged_size: RefCell<Option<Size>>,
        reported_size: Size,
    }

    #[elwindui_macros::class]
    impl LayoutProbe {
        #[overrides]
        fn measure_override(&self, _available: Size) -> Size {
            self.measure_count.set(self.measure_count.get() + 1);
            self.reported_size
        }

        #[overrides]
        fn arrange_override(&self, final_size: Size) -> Size {
            self.arrange_count.set(self.arrange_count.get() + 1);
            *self.last_arranged_size.borrow_mut() = Some(final_size);
            final_size
        }

        fn construct(reported_size: Size) -> Self {
            Self {
                base: elwindui_core::ui::UIElement::construct(),
                measure_count: Cell::new(0),
                arrange_count: Cell::new(0),
                last_arranged_size: RefCell::new(None),
                reported_size,
            }
        }
    }

    impl LayoutProbe {
        fn reset_counts(&self) {
            self.measure_count.set(0);
            self.arrange_count.set(0);
        }
    }

    fn probe_view(count: usize) -> (Rc<CustomTabView>, Vec<Rc<LayoutProbe>>) {
        let view = CustomTabView::new_view();
        let mut probes = Vec::with_capacity(count);
        let mut items = Vec::with_capacity(count);
        for index in 0..count {
            let probe = LayoutProbe::new(Size {
                width: 40.0 + index as f32,
                height: 20.0,
            });
            let item = CustomTabViewItem::new_item();
            item.set_content(probe.clone());
            probes.push(probe);
            items.push(item);
        }
        view.replace_children(items);
        (view, probes)
    }

    #[test]
    fn selection_layout_measures_only_the_selected_page() {
        let (view, probes) = probe_view(8);
        let root: Rc<dyn UIElementExt> = view.clone();
        let size = Size {
            width: 320.0,
            height: 180.0,
        };
        crate::core::ui::layout_root(&root, size);
        for probe in &probes {
            probe.reset_counts();
        }

        assert!(view.select_index(7));
        crate::core::ui::layout_root(&root, size);

        assert!(probes[7].measure_count.get() > 0);
        assert!(
            probes[..7]
                .iter()
                .chain(probes[8..].iter())
                .all(|probe| probe.measure_count.get() == 0)
        );
    }

    #[test]
    fn selection_layout_arranges_only_previous_and_current_pages() {
        let (view, probes) = probe_view(4);
        let root: Rc<dyn UIElementExt> = view.clone();
        let size = Size {
            width: 320.0,
            height: 180.0,
        };
        crate::core::ui::layout_root(&root, size);
        for probe in &probes {
            probe.reset_counts();
        }

        assert!(view.select_index(2));
        crate::core::ui::layout_root(&root, size);

        assert_eq!(probes[0].last_arranged_size.borrow().unwrap().width, 0.0);
        assert_eq!(probes[0].last_arranged_size.borrow().unwrap().height, 0.0);
        assert_eq!(probes[2].last_arranged_size.borrow().unwrap().width, 320.0);
        assert_eq!(probes[2].last_arranged_size.borrow().unwrap().height, 148.0);
        assert_eq!(probes[1].arrange_count.get(), 0);
        assert_eq!(probes[3].arrange_count.get(), 0);
    }

    #[test]
    fn selection_updates_do_not_rebuild_content_entries() {
        let presenter = CustomTabContentPresenter::new();
        let items = (0..4)
            .map(|_| CustomTabViewItem::new_item())
            .collect::<Vec<_>>();
        presenter.set_items(items);
        presenter.reconcile_contents();
        let structural_count = presenter.structural_reconciliation_count_for_test();

        presenter.set_selected_index(3);
        presenter.set_selected_index(1);

        assert_eq!(
            presenter.structural_reconciliation_count_for_test(),
            structural_count
        );
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
        if let Some((_, Some(content))) = entries
            .iter()
            .find(|(index, _)| *index == self.selected_index())
        {
            content.measure(available);
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
        let full_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: final_size.width.max(0.0),
            height: final_size.height.max(0.0),
        };
        let empty_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        let entries = self.entries();
        let selected = self.selected_index();
        let previous = self.last_arranged_selected_index();
        if self.structure_dirty() || previous.is_none() {
            for (index, content) in entries {
                if let Some(content) = content {
                    content.set_clip_to_bounds(Some(true));
                    content.arrange(if index == selected {
                        full_rect
                    } else {
                        empty_rect
                    });
                }
            }
        } else if previous != Some(selected) {
            for index in [previous, Some(selected)].into_iter().flatten() {
                if let Some((_, Some(content))) =
                    entries.iter().find(|(candidate, _)| *candidate == index)
                {
                    content.set_clip_to_bounds(Some(true));
                    content.arrange(if index == selected {
                        full_rect
                    } else {
                        empty_rect
                    });
                }
            }
        }
        self.set_last_arranged_selected_index(Some(selected));
        self.set_structure_dirty(false);
        final_size
    }
}
