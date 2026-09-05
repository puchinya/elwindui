use super::core;
use super::core::base::Point;
use super::core::environment::application_environment;
use super::core::input::{MouseButton, PointerEventArgs};
use super::core::layout::Visibility;
use super::core::theme::{BrushStyle, ResolvedValue};
use super::core::ui::{ControlExt, Grid, LayoutExt, UIElementExt};
use super::weak_self_from_visual_owner;
use super::{ChromeIcon, chrome_icon};
use std::rc::Rc;

/// Private close-slot control used by [`CustomTabViewItem`]'s authored header template.
#[elwindui::component(inherits Control)]
pub(crate) struct CustomTabCloseButton {
    #[prop(default = true)]
    slot_visible: bool,
    #[prop(default = false)]
    glyph_visible: bool,
    #[state(default = None)]
    close_callback: Option<Rc<dyn Fn()>>,
    #[state(default = false)]
    pressed: bool,
    #[state(default = false)]
    handlers_bound: bool,
    #[computed(expr = if slot_visible { Visibility::Visible } else { Visibility::Collapsed })]
    slot_visibility: Visibility,
    template: template_view!(|this: Self| {
        on_mount {
            this.bind_pointer_handlers();
            this.sync_glyph_paint();
        }
        on_update(glyph_visible) {
            this.sync_glyph_paint();
        }
        Grid {
            width: 20.0
            height: 32.0
            visibility: slot_visibility
        }
    }),
}

#[elwindui::component]
impl CustomTabCloseButton {
    #[overrides]
    fn hit_test_content(&self) -> bool {
        self.slot_visible()
    }

    #[overrides]
    fn on_apply_template(&self) {
        self.sync_glyph_paint();
    }
}

impl CustomTabCloseButton {
    pub(crate) fn sync_glyph_paint(&self) {
        let foreground = if self.glyph_visible() {
            match BrushStyle::Foreground.resolve(&application_environment()) {
                ResolvedValue::Value(brush) => Some(brush),
                ResolvedValue::PlatformDefault => None,
            }
        } else {
            None
        };
        let Some(slot_node) = core::visual_tree::find_all::<Grid>(self).into_iter().next() else {
            return;
        };
        let Some(slot) = slot_node.as_any().downcast_ref::<Grid>() else {
            return;
        };
        slot.children().clear();
        if self.glyph_visible() {
            let glyph = chrome_icon(ChromeIcon::Close, foreground);
            glyph.set_hit_test_visible(false);
            slot.children().add(glyph);
        }
    }

    pub(crate) fn set_on_close(&self, callback: Option<Rc<dyn Fn()>>) {
        self.set_close_callback(callback);
    }

    fn bind_pointer_handlers(&self) {
        if self.handlers_bound() {
            return;
        }
        let weak_self: std::rc::Weak<CustomTabCloseButton> = self.weak_self();
        let self_handle: Option<Rc<CustomTabCloseButton>> = weak_self.upgrade();
        if self_handle.is_none() {
            return;
        }
        self.set_handlers_bound(true);

        let weak_self = weak_self.clone();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, args| {
                let button: Option<Rc<CustomTabCloseButton>> = weak_self.upgrade();
                if args.handled.get() || event.button != Some(MouseButton::Left) || button.is_none()
                {
                    return;
                }
                let button = button.expect("close button alive");
                if !button.slot_visible() {
                    return;
                }
                button.set_pressed(true);
                args.handled.set(true);
            }),
        );

        let weak_self: std::rc::Weak<CustomTabCloseButton> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                let button: Option<Rc<CustomTabCloseButton>> = weak_self.upgrade();
                let Some(button) = button else {
                    return;
                };
                if !button.pressed() {
                    return;
                }
                button.set_pressed(false);
                args.handled.set(true);
                if button.slot_visible()
                    && button.contains_root_point(event.position)
                    && let Some(callback) = button.close_callback()
                {
                    callback();
                }
            }),
        );

        let weak_self: std::rc::Weak<CustomTabCloseButton> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |_, args| {
                let button: Option<Rc<CustomTabCloseButton>> = weak_self.upgrade();
                if let Some(button) = button {
                    if button.pressed() {
                        args.handled.set(true);
                    }
                }
            }),
        );

        let weak_self: std::rc::Weak<CustomTabCloseButton> = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |_, args| {
                let button: Option<Rc<CustomTabCloseButton>> = weak_self.upgrade();
                if let Some(button) = button {
                    button.set_pressed(false);
                    args.handled.set(true);
                }
            }),
        );
    }

    fn contains_root_point(&self, point: Point) -> bool {
        let mut offset = self.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let mut parent = self.visual_parent();
        while let Some(element) = parent {
            let child_offset = element
                .arranged_offset()
                .unwrap_or(Point { x: 0.0, y: 0.0 });
            offset.x += child_offset.x;
            offset.y += child_offset.y;
            parent = element.visual_parent();
        }
        let width = self.arranged_width().unwrap_or(20.0);
        let height = self.arranged_height().unwrap_or(32.0);
        point.x >= offset.x
            && point.y >= offset.y
            && point.x < offset.x + width
            && point.y < offset.y + height
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        weak_self_from_visual_owner(self)
    }
}
