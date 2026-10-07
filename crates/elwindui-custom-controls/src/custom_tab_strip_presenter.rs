use super::core::base::{Point, Rect, Size};
use super::core::layout::Visibility;
use super::core::ui::{LayoutExt, UIElementExt};
use super::{CloseButtonPresentation, CustomTabViewItem, TabStripPosition};
#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const TAB_STRIP_HEIGHT: f32 = 32.0;
const TAB_STRIP_FRAME_INSET: f32 = 0.0;
const TAB_HEADER_MAX_WIDTH: f32 = 200.0;
/// Natural-width ceiling of a generic compact header.
const GENERIC_COMPACT_MAX_WIDTH: f32 = 240.0;

/// Inputs that decide a tab header's natural width. The available strip width is deliberately not
/// one of them: compact headers are measured against the fixed canonical ceiling, so a strip-width
/// change only reallocates widths.
#[derive(Clone, Copy, PartialEq)]
struct TabMeasureInputs {
    measure_width: f32,
    height: f32,
    compact: bool,
    connected: bool,
    tab_strip_position: TabStripPosition,
    close_button_presentation: CloseButtonPresentation,
}

/// Natural header widths captured by the last Measure pass. Arrange reads them without measuring;
/// Measure reuses them while every retained header subtree still holds the measurement it had when
/// they were captured.
struct MeasuredTabStripPass {
    item_identities: Vec<Weak<CustomTabViewItem>>,
    layouts: Vec<Vec<Option<Size>>>,
    inputs: TabMeasureInputs,
    natural_widths: Vec<f32>,
}

impl MeasuredTabStripPass {
    fn same_items(&self, items: &[Rc<CustomTabViewItem>]) -> bool {
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
    }

    fn reusable(&self, items: &[Rc<CustomTabViewItem>], inputs: TabMeasureInputs) -> bool {
        self.inputs == inputs
            && self.same_items(items)
            && items.iter().zip(&self.layouts).all(|(item, layout)| {
                measured_layout(item.as_ui_element()).as_ref() == Some(layout)
            })
    }
}

/// Retained measured sizes of `element`'s subtree in visual order (`None` for an element that does
/// not participate in layout), or `None` when a participating element has no measurement (it was
/// invalidated and not measured again). A natural width is a sum of these sizes, so an unchanged
/// snapshot keeps it valid even when a descendant was re-measured in between, and a header that
/// collapsed or changed size since is caught although it leaves no invalid measurement behind.
fn measured_layout(element: &dyn UIElementExt) -> Option<Vec<Option<Size>>> {
    fn visit(element: &dyn UIElementExt, out: &mut Vec<Option<Size>>) -> bool {
        if !element.participates_in_layout() {
            out.push(None);
            return true;
        }
        let Some(size) = element.measured_size() else {
            return false;
        };
        out.push(Some(size));
        element
            .visual_children()
            .iter()
            .all(|child| visit(child.as_ui_element(), out))
    }
    let mut layout = Vec::new();
    visit(element, &mut layout).then_some(layout)
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
    #[state(default = false)]
    connected_chrome: bool,
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
    pub(crate) fn apply_connected_chrome(&self, connected: bool) {
        if self.connected_chrome() != connected {
            self.set_connected_chrome(connected);
            self.last_measurement_pass().borrow_mut().take();
            for item in self.items() {
                item.apply_connected_chrome(connected);
            }
        }
    }

    fn effective_max_width(&self, available: f32, count: usize) -> f32 {
        if self.connected_chrome() {
            return Self::max_item_width(available, count, self.compact());
        }
        if count == 0 {
            return 0.0;
        }
        if self.compact() {
            available.min(GENERIC_COMPACT_MAX_WIDTH).max(0.0)
        } else if available.is_finite() {
            // Keep the current bounded strip rather than allowing offscreen headers. Whole-pixel
            // widths (native TabView rounds down too) keep header edges off fractional pixels,
            // where the selected outline's fixed edge pieces would show an antialiased seam.
            (available / count as f32).floor().clamp(0.0, 240.0)
        } else {
            240.0
        }
    }
    fn max_item_width(available_width: f32, count: usize, compact: bool) -> f32 {
        if count == 0 {
            return 0.0;
        }
        let inner_width = if available_width.is_finite() {
            (available_width - TAB_STRIP_FRAME_INSET * 2.0).max(0.0)
        } else {
            TAB_HEADER_MAX_WIDTH * count as f32
        };
        if compact {
            inner_width.min(TAB_HEADER_MAX_WIDTH)
        } else {
            (inner_width / count as f32).min(TAB_HEADER_MAX_WIDTH)
        }
    }

    /// Width a header is measured against: the canonical natural-width ceiling for compact tabs,
    /// which no strip width can raise, or the equal slot for the other tabs.
    fn measure_width(&self, max_width: f32) -> f32 {
        match (self.compact(), self.connected_chrome()) {
            (true, true) => TAB_HEADER_MAX_WIDTH,
            (true, false) => GENERIC_COMPACT_MAX_WIDTH,
            (false, _) => max_width,
        }
    }

    fn measure_inputs(&self, max_width: f32, height: f32) -> TabMeasureInputs {
        TabMeasureInputs {
            measure_width: self.measure_width(max_width),
            height,
            compact: self.compact(),
            connected: self.connected_chrome(),
            tab_strip_position: self.tab_strip_position(),
            close_button_presentation: self.close_button_presentation(),
        }
    }

    fn measured_item_width(item: &CustomTabViewItem, maximum: f32) -> f32 {
        #[cfg(test)]
        INTRINSIC_WIDTH_CALLS.with(|calls| calls.set(calls.get() + 1));
        item.measured_intrinsic_header_width(maximum).unwrap_or(0.0)
    }

    /// Strip widths for one layout: natural widths capped by the current slot, or the equal slot
    /// itself. A collapsed header (Docking hides a dragged tab) takes no strip width, so the
    /// remaining headers close up like the reference.
    fn allocated_widths(
        items: &[Rc<CustomTabViewItem>],
        natural_widths: &[f32],
        compact: bool,
        max_width: f32,
    ) -> Vec<f32> {
        items
            .iter()
            .zip(natural_widths)
            .map(|(item, natural)| {
                if item.visibility() != Visibility::Visible {
                    0.0
                } else if compact {
                    natural.min(max_width)
                } else {
                    max_width
                }
            })
            .collect()
    }

    fn fit_compact_widths(widths: &mut [f32], available_content_width: f32) {
        if !available_content_width.is_finite() {
            return;
        }
        let content_width = widths.iter().sum::<f32>();
        if content_width > available_content_width && content_width > 0.0 {
            let scale = available_content_width.max(0.0) / content_width;
            for width in widths {
                *width *= scale;
            }
        }
    }

    /// Measure phase only: measures each visible header once against `inputs` and reads its
    /// natural width from that retained measurement.
    fn measure_natural_widths(
        items: &[Rc<CustomTabViewItem>],
        inputs: TabMeasureInputs,
    ) -> Vec<f32> {
        items
            .iter()
            .map(|item| {
                if item.visibility() != Visibility::Visible {
                    return 0.0;
                }
                #[cfg(test)]
                ITEM_MEASURE_CALLS.with(|calls| calls.set(calls.get() + 1));
                item.measure(Size {
                    width: inputs.measure_width + 2.0 * item.header_overhang(),
                    height: inputs.height,
                });
                if inputs.compact {
                    Self::measured_item_width(item, inputs.measure_width)
                } else {
                    inputs.measure_width
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
                offset.x + item.arranged_width()? - item.header_overhang(),
                offset.y,
                item.arranged_height()?,
            )
        } else {
            let item = items.get(index)?;
            let offset = item.arranged_offset()?;
            (
                offset.x + item.header_overhang(),
                offset.y,
                item.arranged_height()?,
            )
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
            item.apply_connected_chrome(self.connected_chrome());
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
        let max_width = self.effective_max_width(available.width, visible_count(&items));
        let inputs = self.measure_inputs(max_width, available.height);
        let cache = self.last_measurement_pass();
        let reusable = cache
            .borrow()
            .as_ref()
            .is_some_and(|pass| pass.reusable(&items, inputs));
        if !reusable {
            let natural_widths = Self::measure_natural_widths(&items, inputs);
            *cache.borrow_mut() = Some(MeasuredTabStripPass {
                item_identities: items.iter().map(Rc::downgrade).collect(),
                layouts: items
                    .iter()
                    .map(|item| measured_layout(item.as_ui_element()).unwrap_or_default())
                    .collect(),
                inputs,
                natural_widths,
            });
        }
        let content_width = cache.borrow().as_ref().map_or(0.0, |pass| {
            Self::allocated_widths(&items, &pass.natural_widths, inputs.compact, max_width)
                .iter()
                .sum::<f32>()
        });
        Size {
            width: content_width + TAB_STRIP_FRAME_INSET * 2.0,
            height: TAB_STRIP_HEIGHT,
        }
    }

    /// Positions the headers from the last Measure's natural widths. Arrange never measures: a
    /// header changed since then has invalidated Measure, and the next Measure refreshes it.
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let items = self.items();
        if items.is_empty() {
            return final_size;
        }
        let compact = self.compact();
        let max_width = self.effective_max_width(final_size.width, visible_count(&items));
        let natural_widths = self
            .last_measurement_pass()
            .borrow()
            .as_ref()
            .filter(|pass| pass.same_items(&items))
            .map(|pass| pass.natural_widths.clone())
            .unwrap_or_else(|| {
                let ceiling = self.measure_width(max_width);
                items
                    .iter()
                    .map(|item| {
                        if compact {
                            item.measured_intrinsic_header_width(ceiling).unwrap_or(0.0)
                        } else {
                            max_width
                        }
                    })
                    .collect()
            });
        let mut widths = Self::allocated_widths(&items, &natural_widths, compact, max_width);
        let frame_inset = TAB_STRIP_FRAME_INSET;
        if compact {
            Self::fit_compact_widths(&mut widths, (final_size.width - frame_inset * 2.0).max(0.0));
        }
        let mut x = frame_inset;
        for (item, width) in items.iter().zip(widths) {
            let overhang = item.header_overhang();
            item.arrange(Rect {
                x: x - overhang,
                y: 0.0,
                width: width + 2.0 * overhang,
                height: final_size.height,
            });
            x += width;
        }
        final_size
    }
}

fn visible_count(items: &[Rc<CustomTabViewItem>]) -> usize {
    items
        .iter()
        .filter(|item| item.visibility() == Visibility::Visible)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ui::UIElementExt;

    #[test]
    fn compact_headers_get_a_content_width_cap_instead_of_a_shared_slot() {
        assert_eq!(
            CustomTabStripPresenter::max_item_width(200.0, 2, true),
            200.0
        );
        assert_eq!(
            CustomTabStripPresenter::max_item_width(200.0, 2, false),
            100.0
        );
        assert_eq!(
            CustomTabStripPresenter::max_item_width(640.0, 4, true),
            200.0
        );
    }

    #[test]
    fn compact_header_widths_fit_the_strip_only_when_their_sum_overflows() {
        let mut overflowing = vec![140.0, 100.0];
        CustomTabStripPresenter::fit_compact_widths(&mut overflowing, 188.0);
        assert!((overflowing[0] - 188.0 * 140.0 / 240.0).abs() < 0.001);
        assert!((overflowing[1] - 188.0 * 100.0 / 240.0).abs() < 0.001);
        assert!((overflowing.iter().sum::<f32>() - 188.0).abs() < 0.001);

        let mut fitting = vec![70.0, 50.0];
        CustomTabStripPresenter::fit_compact_widths(&mut fitting, 188.0);
        assert_eq!(fitting, vec![70.0, 50.0]);
    }

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
        // Generic headers extend by the outline overhang on both sides of the logical width.
        assert_eq!(
            item.arranged_width(),
            Some(80.0 + 2.0 * crate::custom_tab_view_item::GENERIC_HEADER_OVERHANG)
        );
        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: TAB_STRIP_HEIGHT,
        });
        // Arrange at a width other than the measured one allocates without measuring again.
        assert_eq!(ITEM_MEASURE_CALLS.with(Cell::get), 1);
    }

    #[test]
    fn generic_equal_headers_use_whole_pixel_widths_and_overhang_both_edges() {
        let presenter = CustomTabStripPresenter::new();
        let items: Vec<_> = (0..3).map(|_| CustomTabViewItem::new_item()).collect();
        presenter.set_items(items.clone());
        presenter.reconcile_items();
        let size = Size {
            width: 452.0,
            height: TAB_STRIP_HEIGHT,
        };
        presenter.measure(size);
        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: size.width,
            height: size.height,
        });
        let overhang = crate::custom_tab_view_item::GENERIC_HEADER_OVERHANG;
        for (index, item) in items.iter().enumerate() {
            // 452 / 3 rounds down to the native 150 px; edges stay on whole pixels.
            assert_eq!(
                item.arranged_offset().unwrap().x,
                index as f32 * 150.0 - overhang
            );
            assert_eq!(item.arranged_width(), Some(150.0 + 2.0 * overhang));
        }
        assert_eq!(presenter.tab_insertion_boundary(1).unwrap().x, 150.0);
        assert_eq!(presenter.tab_insertion_boundary(3).unwrap().x, 450.0);
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

    struct CountingTextBackend(Rc<Cell<usize>>);

    impl crate::core::graphics::TextBackend for CountingTextBackend {
        fn default_text_style(&self) -> crate::core::graphics::ComputedTextStyle {
            crate::core::graphics::ComputedTextStyle::fallback()
        }

        fn measure_text(
            &self,
            request: &crate::core::graphics::TextMeasureRequest<'_>,
        ) -> crate::core::graphics::TextMeasureResult {
            self.0.set(self.0.get() + 1);
            crate::core::graphics::TextMeasureResult {
                size: Size {
                    width: (request.text.chars().count() as f32 * 7.0)
                        .min(request.available.width.max(0.0)),
                    height: 16.0,
                },
                baseline: 12.0,
                line_count: 1,
            }
        }
    }

    /// Installs a counting text backend for the test's thread and removes it on drop.
    struct TextMeasureProbe(Rc<Cell<usize>>);

    impl TextMeasureProbe {
        fn install() -> Self {
            let calls = Rc::new(Cell::new(0));
            crate::core::graphics::set_text_backend(Rc::new(CountingTextBackend(calls.clone())));
            Self(calls)
        }

        fn calls(&self) -> usize {
            self.0.get()
        }
    }

    impl Drop for TextMeasureProbe {
        fn drop(&mut self) {
            crate::core::graphics::clear_text_backend();
        }
    }

    fn reset_counters() {
        ITEM_MEASURE_CALLS.with(|calls| calls.set(0));
        INTRINSIC_WIDTH_CALLS.with(|calls| calls.set(0));
    }

    fn measure_calls() -> (usize, usize) {
        (
            ITEM_MEASURE_CALLS.with(Cell::get),
            INTRINSIC_WIDTH_CALLS.with(Cell::get),
        )
    }

    fn strip(width: f32) -> Size {
        Size {
            width,
            height: TAB_STRIP_HEIGHT,
        }
    }

    fn arrange_strip(presenter: &CustomTabStripPresenter, width: f32) {
        presenter.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width,
            height: TAB_STRIP_HEIGHT,
        });
    }

    /// A connected compact strip like a Docking group with `compact_tabs`.
    fn connected_compact_strip(
        headers: &[&str],
    ) -> (Rc<CustomTabStripPresenter>, Vec<Rc<CustomTabViewItem>>) {
        let presenter = CustomTabStripPresenter::new();
        let items: Vec<_> = headers
            .iter()
            .map(|header| {
                let item = CustomTabViewItem::new_item();
                item.set_header((*header).to_owned());
                item
            })
            .collect();
        presenter.set_items(items.clone());
        presenter.set_compact(true);
        presenter.reconcile_items();
        presenter.apply_connected_chrome(true);
        (presenter, items)
    }

    #[test]
    fn intrinsic_width_reader_uses_the_retained_measurement_only() {
        let probe = TextMeasureProbe::install();
        let (presenter, items) = connected_compact_strip(&["Document"]);
        let item = &items[0];
        assert_eq!(item.measured_intrinsic_header_width(200.0), None);
        presenter.measure(strip(640.0));
        let text_calls = probe.calls();
        reset_counters();

        let width = item
            .measured_intrinsic_header_width(200.0)
            .expect("a measured header has a retained natural width");
        assert_eq!(item.measured_intrinsic_header_width(200.0), Some(width));
        assert!(width > "Document".len() as f32 * 7.0);
        assert_eq!(probe.calls(), text_calls);
        assert_eq!(measure_calls(), (0, 0));
        assert_eq!(item.measured_intrinsic_header_width(40.0), Some(40.0));
    }

    #[test]
    fn presenter_arrange_never_measures() {
        let probe = TextMeasureProbe::install();
        let (presenter, items) = connected_compact_strip(&["One", "A longer title", "Three"]);
        presenter.measure(strip(640.0));
        let text_calls = probe.calls();
        reset_counters();

        for width in [640.0, 120.0, 900.0, 640.0] {
            arrange_strip(&presenter, width);
        }

        assert_eq!(measure_calls(), (0, 0));
        assert_eq!(probe.calls(), text_calls);
        // The narrow pass still fit the strip; the last pass restored natural widths.
        let total: f32 = items
            .iter()
            .map(|item| item.arranged_width().unwrap())
            .sum();
        let natural: f32 = items
            .iter()
            .map(|item| item.measured_intrinsic_header_width(200.0).unwrap())
            .sum();
        assert!((total - natural).abs() < 0.001);
    }

    #[test]
    fn strip_width_changes_reuse_natural_widths() {
        let probe = TextMeasureProbe::install();
        let (presenter, items) = connected_compact_strip(&["One", "A longer title"]);
        presenter.measure(strip(640.0));
        let text_calls = probe.calls();
        let natural: Vec<f32> = items
            .iter()
            .map(|item| item.measured_intrinsic_header_width(200.0).unwrap())
            .collect();
        reset_counters();

        for width in [300.0, 120.0, 900.0] {
            presenter.measure(strip(width));
            arrange_strip(&presenter, width);
        }

        assert_eq!(measure_calls(), (0, 0));
        assert_eq!(probe.calls(), text_calls);
        // The last narrow-to-wide sequence ended at 900, where natural widths fit unchanged.
        for (item, natural) in items.iter().zip(&natural) {
            assert_eq!(item.arranged_width(), Some(*natural));
        }
        presenter.measure(strip(60.0));
        arrange_strip(&presenter, 60.0);
        // Each natural width is capped by the 60 px strip, then the sum is fit proportionally.
        let capped: Vec<f32> = natural.iter().map(|width| width.min(60.0)).collect();
        let sum = capped.iter().sum::<f32>();
        for (item, capped) in items.iter().zip(&capped) {
            let fitted = item.arranged_width().unwrap();
            assert!((fitted - 60.0 * capped / sum).abs() < 0.001);
        }
        assert_eq!(measure_calls(), (0, 0));
    }

    #[test]
    fn header_mutation_is_refreshed_by_the_next_measure_not_by_arrange() {
        let (presenter, items) = connected_compact_strip(&["First"]);
        let item = &items[0];
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        let short = item.arranged_width().unwrap();
        reset_counters();

        item.set_header("A substantially longer header".to_owned());
        arrange_strip(&presenter, 640.0);
        assert_eq!(measure_calls(), (0, 0));

        presenter.measure(strip(640.0));
        assert_eq!(measure_calls(), (1, 1));
        arrange_strip(&presenter, 640.0);
        arrange_strip(&presenter, 640.0);
        assert_eq!(measure_calls(), (1, 1));
        let long = item.arranged_width().unwrap();
        assert!(long > short);
        assert_eq!(item.measured_intrinsic_header_width(200.0), Some(long));
    }

    #[test]
    fn activation_reserves_the_reference_marker_slot_through_measure() {
        // WinUI.Dock collapses its ActiveIndicator while inactive, so the 4 px marker and 6 px gap
        // join the header only while active. The width change goes through Measure invalidation.
        let (presenter, items) = connected_compact_strip(&["Document"]);
        let item = &items[0];
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        let inactive = item.arranged_width().unwrap();
        reset_counters();

        item.set_active_document_marker_visible(true);
        arrange_strip(&presenter, 640.0);
        assert_eq!(measure_calls(), (0, 0));
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        assert_eq!(item.arranged_width(), Some(inactive + 10.0));

        item.set_active_document_marker_visible(false);
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        assert_eq!(item.arranged_width(), Some(inactive));
        assert_eq!(measure_calls(), (2, 2));
    }

    #[test]
    fn hover_actions_keep_their_reserved_slot_width() {
        let (presenter, items) = connected_compact_strip(&["Document"]);
        presenter.set_close_button_presentation(CloseButtonPresentation::OnPointerOver);
        let item = &items[0];
        item.set_document_pin_action(true, Rc::new(|| {}));
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        let width = item.arranged_width().unwrap();

        for hovered in [true, false, true] {
            item.update_pointer_over(hovered);
            assert_eq!(item.pointer_over(), hovered);
            presenter.measure(strip(640.0));
            arrange_strip(&presenter, 640.0);
            assert_eq!(item.arranged_width(), Some(width));
        }
    }

    #[test]
    fn collapsed_headers_leave_the_strip_and_return_through_measure() {
        let (presenter, items) = connected_compact_strip(&["One", "Two"]);
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        let first = items[0].arranged_width().unwrap();

        items[0].set_visibility(Visibility::Collapsed);
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        assert_eq!(items[1].arranged_offset().unwrap().x, 0.0);

        items[0].set_visibility(Visibility::Visible);
        presenter.measure(strip(640.0));
        arrange_strip(&presenter, 640.0);
        assert_eq!(items[0].arranged_width(), Some(first));
        assert_eq!(items[1].arranged_offset().unwrap().x, first);
    }
}
