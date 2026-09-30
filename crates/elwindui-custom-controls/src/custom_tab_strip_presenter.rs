use super::core::base::{Point, Rect, Size};
use super::core::ui::{LayoutExt, UIElementExt};
use super::{CloseButtonPresentation, CustomTabViewItem, TabStripPosition};
#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const TAB_STRIP_HEIGHT: f32 = 32.0;
const TAB_STRIP_FRAME_INSET: f32 = 0.0;
const TAB_HEADER_MAX_WIDTH: f32 = 200.0;

#[derive(Clone)]
struct MeasuredTabStripPass {
    item_identities: Vec<Weak<CustomTabViewItem>>,
    max_item_width: f32,
    height: f32,
    compact: bool,
    tab_strip_position: TabStripPosition,
    close_button_presentation: CloseButtonPresentation,
    widths: Vec<f32>,
}

impl MeasuredTabStripPass {
    fn new(
        items: &[Rc<CustomTabViewItem>],
        max_item_width: f32,
        height: f32,
        compact: bool,
        tab_strip_position: TabStripPosition,
        close_button_presentation: CloseButtonPresentation,
        widths: Vec<f32>,
    ) -> Self {
        Self {
            item_identities: items.iter().map(Rc::downgrade).collect(),
            max_item_width,
            height,
            compact,
            tab_strip_position,
            close_button_presentation,
            widths,
        }
    }

    fn matches(
        &self,
        items: &[Rc<CustomTabViewItem>],
        max_item_width: f32,
        height: f32,
        compact: bool,
        tab_strip_position: TabStripPosition,
        close_button_presentation: CloseButtonPresentation,
    ) -> bool {
        self.item_identities.len() == items.len()
            && self
                .item_identities
                .iter()
                .zip(items)
                .all(|(identity, item)| {
                    identity
                        .upgrade()
                        .is_some_and(|cached| Rc::ptr_eq(&cached, item))
                })
            && self.max_item_width == max_item_width
            && self.height == height
            && self.compact == compact
            && self.tab_strip_position == tab_strip_position
            && self.close_button_presentation == close_button_presentation
            && self.widths.len() == items.len()
            && items
                .iter()
                .all(|item| Self::measurement_tree_is_valid(item.as_ui_element()))
    }

    fn measurement_tree_is_valid(element: &dyn UIElementExt) -> bool {
        element.measured_size().is_some()
            && element
                .visual_children()
                .iter()
                .all(|child| Self::measurement_tree_is_valid(child.as_ui_element()))
    }
}

#[cfg(test)]
std::thread_local! {
    static ITEM_MEASURE_CALLS: Cell<usize> = const { Cell::new(0) };
    static INTRINSIC_WIDTH_CALLS: Cell<usize> = const { Cell::new(0) };
}

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
    #[state(default = Rc::new(RefCell::new(None)))]
    last_measurement_pass: Rc<RefCell<Option<MeasuredTabStripPass>>>,
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
    fn max_item_width(available_width: f32, count: usize) -> f32 {
        if count == 0 {
            return 0.0;
        }
        let inner_width = if available_width.is_finite() {
            (available_width - TAB_STRIP_FRAME_INSET * 2.0).max(0.0)
        } else {
            TAB_HEADER_MAX_WIDTH * count as f32
        };
        (inner_width / count as f32).min(TAB_HEADER_MAX_WIDTH)
    }

    fn measured_item_width(item: &CustomTabViewItem, maximum: f32, height: f32) -> f32 {
        #[cfg(test)]
        INTRINSIC_WIDTH_CALLS.with(|calls| calls.set(calls.get() + 1));
        item.intrinsic_header_width(maximum, height)
    }

    fn measure_item_widths(
        &self,
        items: &[Rc<CustomTabViewItem>],
        max_width: f32,
        height: f32,
    ) -> Vec<f32> {
        let compact = self.compact();
        items
            .iter()
            .map(|item| {
                #[cfg(test)]
                ITEM_MEASURE_CALLS.with(|calls| calls.set(calls.get() + 1));
                item.measure(Size {
                    width: max_width,
                    height,
                });
                if compact {
                    Self::measured_item_width(item, max_width, height)
                } else {
                    max_width
                }
            })
            .collect()
    }

    /// Resolves a point against the retained arranged tab headers. This intentionally reads only
    /// the last layout result: a drag preview must not reconcile the presenter or measure pages.
    pub(crate) fn tab_insertion_index_at(&self, point: Point) -> Option<usize> {
        let width = self.arranged_width()?;
        let height = self.arranged_height()?;
        if !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
            || !point.x.is_finite()
            || !point.y.is_finite()
            || point.x < 0.0
            || point.x > width
            || point.y < 0.0
            || point.y > height
        {
            return None;
        }
        let items = self.items();
        if items.is_empty() {
            return Some(0);
        }
        for (index, item) in items.iter().enumerate() {
            let offset = item.arranged_offset()?;
            let item_width = item.arranged_width()?;
            if !offset.x.is_finite() || !item_width.is_finite() || item_width < 0.0 {
                return None;
            }
            if point.x <= offset.x + item_width * 0.5 {
                return Some(index);
            }
        }
        Some(items.len())
    }

    /// Returns the retained header boundary for an insertion index. `width` is zero so the
    /// docking runtime can apply its single shared marker width without changing tab geometry.
    pub(crate) fn tab_insertion_boundary(&self, index: usize) -> Option<Rect> {
        let height = self.arranged_height()?;
        let items = self.items();
        if index > items.len() || !height.is_finite() || height < 0.0 {
            return None;
        }
        if items.is_empty() {
            return (index == 0).then_some(Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height,
            });
        }
        let (x, y, item_height) = if index == items.len() {
            let item = items.last()?;
            let offset = item.arranged_offset()?;
            (
                offset.x + item.arranged_width()?,
                offset.y,
                item.arranged_height()?,
            )
        } else {
            let item = items.get(index)?;
            let offset = item.arranged_offset()?;
            (offset.x, offset.y, item.arranged_height()?)
        };
        (x.is_finite() && y.is_finite() && item_height.is_finite() && item_height >= 0.0).then_some(
            Rect {
                x,
                y,
                width: 0.0,
                height: item_height,
            },
        )
    }

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
            let selection_changed = self.last_presented_selected_index() != Some(selected);
            let presentation_changed = self.last_presented_tab_strip_position() != Some(position)
                || self.last_presented_compact_tabs() != Some(compact)
                || self.last_presented_close_button_presentation() != Some(close);
            if selection_changed && !presentation_changed {
                self.sync_selection_only(&items, self.last_presented_selected_index(), selected);
            } else if presentation_changed {
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
                );
            }
        }
        self.set_last_presented_selected_index(Some(selected));
        self.set_last_presented_compact_tabs(Some(self.compact()));
    }
}

#[elwindui::component]
impl CustomTabStripPresenter {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let items = self.items();
        if items.is_empty() {
            self.last_measurement_pass().borrow_mut().take();
            return Size {
                width: TAB_STRIP_FRAME_INSET * 2.0,
                height: TAB_STRIP_HEIGHT,
            };
        }
        let max_width = Self::max_item_width(available.width, items.len());
        let widths = self.measure_item_widths(&items, max_width, available.height);
        let content_width = widths.iter().sum::<f32>();
        *self.last_measurement_pass().borrow_mut() = Some(MeasuredTabStripPass::new(
            &items,
            max_width,
            available.height,
            self.compact(),
            self.tab_strip_position(),
            self.close_button_presentation(),
            widths,
        ));
        Size {
            width: content_width + TAB_STRIP_FRAME_INSET * 2.0,
            height: TAB_STRIP_HEIGHT,
        }
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let items = self.items();
        if items.is_empty() {
            self.last_measurement_pass().borrow_mut().take();
            return final_size;
        }
        let max_width = Self::max_item_width(final_size.width, items.len());
        let measured_pass = self
            .last_measurement_pass()
            .borrow()
            .clone()
            .filter(|pass| {
                pass.matches(
                    &items,
                    max_width,
                    final_size.height,
                    self.compact(),
                    self.tab_strip_position(),
                    self.close_button_presentation(),
                )
            });
        let widths = measured_pass
            .map(|pass| pass.widths)
            .unwrap_or_else(|| self.measure_item_widths(&items, max_width, final_size.height));
        *self.last_measurement_pass().borrow_mut() = Some(MeasuredTabStripPass::new(
            &items,
            max_width,
            final_size.height,
            self.compact(),
            self.tab_strip_position(),
            self.close_button_presentation(),
            widths.clone(),
        ));
        let mut x = TAB_STRIP_FRAME_INSET;
        for (item, width) in items.iter().zip(widths) {
            item.arrange(Rect {
                x,
                y: 0.0,
                width,
                height: final_size.height,
            });
            x += width;
        }
        final_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ui::UIElementExt;

    #[test]
    fn layout_does_not_mutate_tab_width_constraints() {
        let presenter = CustomTabStripPresenter::new();
        let item = CustomTabViewItem::new_item();
        presenter.set_items(vec![item.clone()]);
        presenter.reconcile_items();
        ITEM_MEASURE_CALLS.with(|calls| calls.set(0));

        presenter.measure(Size {
            width: 640.0,
            height: TAB_STRIP_HEIGHT,
        });
        assert_eq!(item.min_width(), None);
        assert_eq!(item.max_width(), None);

        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: TAB_STRIP_HEIGHT,
        });

        assert_eq!(item.min_width(), None);
        assert_eq!(item.max_width(), None);
        assert_eq!(item.arranged_width(), Some(80.0));
        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: TAB_STRIP_HEIGHT,
        });
        assert_eq!(ITEM_MEASURE_CALLS.with(Cell::get), 2);
    }

    #[test]
    fn arrange_reuses_tab_measurements_when_constraints_match() {
        let presenter = CustomTabStripPresenter::new();
        let item = CustomTabViewItem::new_item();
        presenter.set_items(vec![item]);
        presenter.set_compact(true);
        presenter.reconcile_items();
        ITEM_MEASURE_CALLS.with(|calls| calls.set(0));
        INTRINSIC_WIDTH_CALLS.with(|calls| calls.set(0));

        presenter.measure(Size {
            width: 200.0,
            height: TAB_STRIP_HEIGHT,
        });
        assert_eq!(ITEM_MEASURE_CALLS.with(Cell::get), 1);
        assert_eq!(INTRINSIC_WIDTH_CALLS.with(Cell::get), 1);

        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: TAB_STRIP_HEIGHT,
        });

        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: TAB_STRIP_HEIGHT,
        });

        assert_eq!(ITEM_MEASURE_CALLS.with(Cell::get), 1);
        assert_eq!(INTRINSIC_WIDTH_CALLS.with(Cell::get), 1);
    }

    #[test]
    fn arrange_remeasures_when_a_header_measurement_is_invalidated() {
        let presenter = CustomTabStripPresenter::new();
        let item = CustomTabViewItem::new_item();
        item.set_header("First".to_owned());
        presenter.set_items(vec![item.clone()]);
        presenter.set_compact(true);
        presenter.reconcile_items();
        ITEM_MEASURE_CALLS.with(|calls| calls.set(0));
        INTRINSIC_WIDTH_CALLS.with(|calls| calls.set(0));

        presenter.measure(Size {
            width: 200.0,
            height: TAB_STRIP_HEIGHT,
        });
        item.set_header("A substantially longer header".to_owned());

        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: TAB_STRIP_HEIGHT,
        });

        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: TAB_STRIP_HEIGHT,
        });

        assert_eq!(ITEM_MEASURE_CALLS.with(Cell::get), 2);
        assert_eq!(INTRINSIC_WIDTH_CALLS.with(Cell::get), 2);
    }
}
