//! Auto-hide strip and single-open overlay state.

use crate::DockItemId;
use crate::DockingControl;
use crate::core::graphics::{Color, IconSource};
use crate::core::input::PointerEventArgs;
use crate::core::layout::{GridLength, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    Grid, GridExt, IconSourceElement, IconSourceElementExt, LayoutExt, TextBlock, TextBlockExt,
    UIElementExt,
};
use crate::model::RootKind;
use crate::runtime::metrics::{
    AUTO_HIDE_ENTRY_HEIGHT, AUTO_HIDE_ENTRY_WIDTH, AUTO_HIDE_ICON_SIZE, AUTO_HIDE_PIN_SIZE,
    AUTO_HIDE_STRIP_SIZE,
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
        pane.set_visibility(Visibility::Collapsed);
        pane.set_attached("Grid", "row", 1i32);
        pane.set_attached("Grid", "column", 1i32);
        visual.children().add(pane.clone());
        let pin_button = Grid::new();
        // Keep the full button hit-testable without painting a surface over the strip.
        pin_button.set_background(Some(Color::TRANSPARENT.into()));
        pin_button.set_width(AUTO_HIDE_PIN_SIZE);
        pin_button.set_height(AUTO_HIDE_PIN_SIZE);
        pin_button.children().add(chrome_icon(
            ChromeIcon::Pin,
            themed_brush(BrushStyle::Foreground),
        ));
        pin_button.set_visibility(Visibility::Collapsed);
        pin_button.set_attached("Grid", "row", 1i32);
        pin_button.set_attached("Grid", "column", 1i32);
        visual.children().add(pin_button.clone());
        let root_context = Rc::new(RefCell::new(RootKind::Main));
        Self {
            open: None,
            visual,
            strips,
            pane,
            pin_button,
            root_context,
        }
    }

    pub(crate) fn open(&mut self, item: DockItemId) -> Option<DockItemId> {
        let previous = self.open.replace(item);
        self.show_pane();
        previous
    }

    pub(crate) fn close(&mut self) -> Option<DockItemId> {
        let previous = self.open.take();
        self.pane.set_visibility(Visibility::Collapsed);
        self.pin_button.set_visibility(Visibility::Collapsed);
        self.pane.children().clear();
        previous
    }

    pub(crate) fn current(&self) -> Option<&DockItemId> {
        self.open.as_ref()
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.visual.clone()
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
        self.pane.children().clear();
        if let Some(wrapper) = wrapper {
            self.pane.children().add(wrapper);
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
