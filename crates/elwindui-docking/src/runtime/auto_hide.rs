//! Auto-hide strip and single-open overlay state.

use crate::DockItemId;
use crate::DockingControl;
use crate::core::graphics::{Color, IconSource};
use crate::core::input::PointerEventArgs;
use crate::core::layout::{GridLength, HorizontalAlignment, VerticalAlignment, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    ContentControlExt, ControlExt, Grid, GridExt, IconSourceElement, IconSourceElementExt,
    LayoutExt, TextBlock, TextBlockExt, UIElementExt,
};
use crate::model::RootKind;
use crate::placement::DockSide;
use crate::runtime::metrics::{
    AUTO_HIDE_ENTRY_HEIGHT, AUTO_HIDE_ENTRY_WIDTH, AUTO_HIDE_ICON_SIZE, AUTO_HIDE_PANEL_HEIGHT,
    AUTO_HIDE_PANEL_WIDTH, AUTO_HIDE_PIN_SIZE, AUTO_HIDE_STRIP_SIZE,
};
use crate::runtime::themed_brush;
use elwindui_custom_controls::{ChromeIcon, chrome_icon};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// Only one auto-hide item owns the open overlay at a time.
pub(crate) struct AutoHideOverlay {
    open: Option<DockItemId>,
    visual: Rc<Grid>,
    strips: [Rc<Grid>; 4],
    pane: Rc<Grid>,
    page_host: Rc<Grid>,
    pin_button: Rc<Grid>,
    root_context: Rc<RefCell<RootKind>>,
}

impl AutoHideOverlay {
    pub(crate) fn new() -> Self {
        let visual = Grid::new();
        visual.set_rows(vec![
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
        ]);
        visual.set_columns(vec![
            GridLength::Auto,
            GridLength::Star(1.0),
            GridLength::Auto,
        ]);
        let strips = std::array::from_fn(|index| {
            let strip = Grid::new();
            strip.set_width(AUTO_HIDE_STRIP_SIZE);
            strip.set_height(AUTO_HIDE_STRIP_SIZE);
            strip.set_attached("DockSurface", "side", index as i32);
            match index {
                0 => {
                    strip.set_attached("Grid", "row", 1i32);
                    strip.set_attached("Grid", "column", 0i32);
                }
                1 => {
                    strip.set_attached("Grid", "row", 0i32);
                    strip.set_attached("Grid", "column", 1i32);
                }
                2 => {
                    strip.set_attached("Grid", "row", 1i32);
                    strip.set_attached("Grid", "column", 2i32);
                }
                _ => {
                    strip.set_attached("Grid", "row", 2i32);
                    strip.set_attached("Grid", "column", 1i32);
                }
            }
            visual.children().add(strip.clone());
            strip
        });
        let pane = Grid::new();
        pane.set_rows(vec![
            GridLength::Fixed(AUTO_HIDE_PIN_SIZE + 8.0),
            GridLength::Star(1.0),
        ]);
        pane.set_columns(vec![GridLength::Star(1.0)]);
        // Keep the panel at a deterministic size while leaving the parent Grid's slot intact;
        // alignment then positions that fixed-size panel against the selected side.
        pane.set_min_width(AUTO_HIDE_PANEL_WIDTH);
        pane.set_max_width(AUTO_HIDE_PANEL_WIDTH);
        pane.set_min_height(AUTO_HIDE_PANEL_HEIGHT);
        pane.set_max_height(AUTO_HIDE_PANEL_HEIGHT);
        pane.set_background(themed_brush(BrushStyle::Background));
        pane.set_visibility(Visibility::Collapsed);
        pane.set_attached("Grid", "row", 1i32);
        pane.set_attached("Grid", "column", 1i32);
        visual.children().add(pane.clone());
        let page_host = Grid::new();
        page_host.set_attached("Grid", "row", 1i32);
        page_host.set_attached("Grid", "column", 0i32);
        pane.children().add(page_host.clone());
        let pin_button = Grid::new();
        // Keep the full button hit-testable without painting a surface over the strip.
        pin_button.set_background(Some(Color::TRANSPARENT.into()));
        // Use min/max constraints instead of explicit Width/Height so the parent cell remains
        // the alignment slot and the icon button can stay anchored at the panel's top-right.
        pin_button.set_min_width(AUTO_HIDE_PIN_SIZE);
        pin_button.set_max_width(AUTO_HIDE_PIN_SIZE);
        pin_button.set_min_height(AUTO_HIDE_PIN_SIZE);
        pin_button.set_max_height(AUTO_HIDE_PIN_SIZE);
        pin_button.set_horizontal_alignment(HorizontalAlignment::Right);
        pin_button.set_vertical_alignment(VerticalAlignment::Top);
        pin_button.set_attached("Grid", "row", 0i32);
        pin_button.set_attached("Grid", "column", 0i32);
        pin_button.children().add(chrome_icon(
            ChromeIcon::Pin,
            themed_brush(BrushStyle::Foreground),
        ));
        pin_button.set_visibility(Visibility::Collapsed);
        pane.children().add(pin_button.clone());
        let root_context = Rc::new(RefCell::new(RootKind::Main));
        Self {
            open: None,
            visual,
            strips,
            pane,
            page_host,
            pin_button,
            root_context,
        }
    }

    pub(crate) fn open(&mut self, item: DockItemId, side: DockSide) -> Option<DockItemId> {
        self.pane.set_horizontal_alignment(match side {
            DockSide::Left => HorizontalAlignment::Left,
            DockSide::Top | DockSide::Bottom => HorizontalAlignment::Center,
            DockSide::Right => HorizontalAlignment::Right,
        });
        self.pane.set_vertical_alignment(match side {
            DockSide::Left | DockSide::Right => VerticalAlignment::Center,
            DockSide::Top => VerticalAlignment::Top,
            DockSide::Bottom => VerticalAlignment::Bottom,
        });
        let previous = self.open.replace(item);
        self.show_pane();
        previous
    }

    pub(crate) fn close(&mut self) -> Option<DockItemId> {
        let previous = self.open.take();
        self.pane.set_visibility(Visibility::Collapsed);
        self.pin_button.set_visibility(Visibility::Collapsed);
        self.page_host.children().clear();
        previous
    }

    pub(crate) fn current(&self) -> Option<&DockItemId> {
        self.open.as_ref()
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.visual.clone()
    }

    #[cfg(test)]
    pub(crate) fn pane_for_test(&self) -> Rc<Grid> {
        self.pane.clone()
    }

    #[cfg(test)]
    pub(crate) fn page_host_for_test(&self) -> Rc<Grid> {
        self.page_host.clone()
    }

    #[cfg(test)]
    pub(crate) fn pin_button_for_test(&self) -> Rc<Grid> {
        self.pin_button.clone()
    }

    pub(crate) fn render_strips(
        &self,
        titles: impl Iterator<Item = (usize, DockItemId, String, Option<IconSource>)>,
        owner: &std::rc::Weak<DockingControl>,
        root: RootKind,
    ) {
        for strip in &self.strips {
            strip.children().clear();
        }
        for (side, item, title, icon_source) in titles {
            let Some(strip) = self.strips.get(side) else {
                continue;
            };
            let entry = Grid::new();
            entry.set_width(AUTO_HIDE_ENTRY_WIDTH);
            entry.set_height(AUTO_HIDE_ENTRY_HEIGHT);
            entry.set_columns(vec![GridLength::Auto, GridLength::Star(1.0)]);
            if let Some(icon_source) = icon_source {
                let icon = IconSourceElement::new();
                icon.set_icon_source(Some(icon_source));
                icon.set_width(AUTO_HIDE_ICON_SIZE);
                icon.set_height(AUTO_HIDE_ICON_SIZE);
                icon.set_attached("Grid", "column", 0i32);
                entry.children().add(icon);
            }
            let text = TextBlock::new();
            text.set_text(&title);
            text.set_attached("Grid", "column", 1i32);
            entry.children().add(text);
            let weak_owner: Weak<DockingControl> = owner.clone();
            let entry_root = root.clone();
            entry.register_routed_handler::<PointerEventArgs>(
                "on_pointer_released",
                Box::new(move |_, _| {
                    let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
                    if let Some(owner) = owner {
                        owner.handle_auto_hide_open(entry_root.clone(), item.clone());
                    }
                }),
            );
            strip.children().add(entry);
        }
    }

    pub(crate) fn show_pane(&self) {
        self.pane.set_visibility(Visibility::Visible);
        self.pin_button.set_visibility(Visibility::Visible);
    }

    pub(crate) fn present_open_item(
        &self,
        wrapper: Option<Rc<elwindui_custom_controls::CustomTabViewItem>>,
    ) {
        self.page_host.children().clear();
        if let Some(wrapper) = wrapper {
            // Auto-hide items are not represented by a tab-view presenter while hidden. Prepare
            // the inherited ContentControl slot before adopting the logical page into this
            // overlay host so the header template itself can never be rendered in the popup.
            wrapper.__prepare_template_presentation();
            if let Some(content) = wrapper.__content_opt() {
                self.page_host.children().add(content);
            }
        }
        self.show_pane();
    }

    pub(crate) fn bind_pin_handler(&self, owner: &std::rc::Weak<DockingControl>, root: RootKind) {
        *self.root_context.borrow_mut() = root;
        let root_context = self.root_context.clone();
        let weak_owner: Weak<DockingControl> = owner.clone();
        self.pin_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |_, _| {
                let owner: Option<Rc<DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    let root = root_context.borrow().clone();
                    owner.handle_pin_gesture(root);
                }
            }),
        );
    }

    pub(crate) fn set_root(&self, root: RootKind) {
        *self.root_context.borrow_mut() = root;
    }

    pub(crate) fn refresh_theme(&self) {
        self.pane
            .set_background(themed_brush(BrushStyle::Background));
        self.pin_button.children().clear();
        self.pin_button.children().add(chrome_icon(
            ChromeIcon::Pin,
            themed_brush(BrushStyle::Foreground),
        ));
    }
}

impl Default for AutoHideOverlay {
    fn default() -> Self {
        Self::new()
    }
}
