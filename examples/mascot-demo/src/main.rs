//! Transparent, always-on-top mascot Window using the repository's real alpha PNG.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::{Point, Rect};
use elwindui::core::graphics::{BitmapImage, ImageDrawOptions, ImageFit, RenderContext};
use elwindui::core::input::{MouseButton, PointerEventArgs};
use elwindui::core::ui::{UIElementExt, WindowExt};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

const MASCOT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../images/elwind_chan_real_anime.png"
);

fn mascot_image() -> &'static BitmapImage {
    static IMAGE: OnceLock<BitmapImage> = OnceLock::new();
    IMAGE.get_or_init(|| {
        BitmapImage::from_file(MASCOT_PATH)
            .expect("images/elwind_chan_real_anime.png must be readable")
    })
}

fn dragged_window_position(
    current_left: f32,
    current_top: f32,
    anchor: Point,
    pointer: Point,
) -> (f32, f32) {
    (
        current_left + pointer.x - anchor.x,
        current_top + pointer.y - anchor.y,
    )
}

#[elwindui::class(inherits = elwindui::core::ui::UIElement)]
pub struct MascotCanvas {}

#[elwindui::class]
impl MascotCanvas {
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        context.draw_image(
            mascot_image(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.arranged_width().unwrap_or(0.0),
                height: self.arranged_height().unwrap_or(0.0),
            },
            None,
            ImageDrawOptions {
                fit: ImageFit::Contain,
                ..Default::default()
            },
        );
    }

    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }

    fn construct() -> Self {
        Self {
            base: elwindui::core::ui::UIElement::construct(),
        }
    }
}

#[elwindui::component(inherits Window)]
struct MascotWindow {
    mascot: Rc<MascotCanvas>,

    body: view! {
        title: "Elwind-chan"
        width: 512.0
        height: 512.0
        transparent: true
        always_on_top: true
        content: mascot
    },
}

#[elwindui::component]
impl MascotWindow {}

#[elwindui::main]
fn main() {
    let mascot = MascotCanvas::new();
    let window = MascotWindow::new(mascot.clone());
    let window_ext: Rc<dyn WindowExt> = window.clone();
    let drag_anchor = Rc::new(RefCell::new(None::<Point>));

    {
        let drag_anchor = drag_anchor.clone();
        mascot.register_routed_handler(
            "on_pointer_pressed",
            Box::new(move |args: &PointerEventArgs, _| {
                if args.button == Some(MouseButton::Left) {
                    *drag_anchor.borrow_mut() = Some(args.position);
                }
            }),
        );
    }
    {
        let drag_anchor = drag_anchor.clone();
        // The application retains the native window after this startup function returns, but not
        // the generated Rust host component. Keep its Window façade alive for the demo's lifetime
        // so drag events can still call the position setters after `main` has returned.
        let window = window_ext.clone();
        mascot.register_routed_handler(
            "on_pointer_moved",
            Box::new(move |args: &PointerEventArgs, _| {
                let Some(anchor) = *drag_anchor.borrow() else {
                    return;
                };
                let (left, top) =
                    dragged_window_position(window.left(), window.top(), anchor, args.position);
                window.set_left(left);
                window.set_top(top);
            }),
        );
    }
    mascot.register_routed_handler(
        "on_pointer_released",
        Box::new(move |args: &PointerEventArgs, _| {
            if args.button == Some(MouseButton::Left) {
                *drag_anchor.borrow_mut() = None;
            }
        }),
    );

    window.show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_keeps_the_pressed_point_under_the_pointer() {
        assert_eq!(
            dragged_window_position(
                120.0,
                80.0,
                Point { x: 40.0, y: 25.0 },
                Point { x: 65.0, y: 70.0 },
            ),
            (145.0, 125.0)
        );
    }
}
