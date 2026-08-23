//! `IconSourceElement` — a self-drawn Visual wrapper for a shareable `IconSource` value.

use super::*;
use crate::graphics::IconSource;

/// Displays an [`IconSource`] as a backend-neutral, self-drawn leaf element.
///
/// System icons use Core's canonical monochrome vector geometry and the effective `foreground`;
/// user raster/vector images keep their own paints. The element owns no backend-native resource.
#[elwindui_macros::class(inherits = crate::ui::IconElement)]
#[prop(icon_source: Option<crate::graphics::IconSource>)]
pub struct IconSourceElement {
    icon_source: RefCell<Option<IconSource>>,
}

#[elwindui_macros::class]
impl IconSourceElement {
    /// Returns the shareable icon value currently displayed by this element.
    fn icon_source(&self) -> Option<IconSource> {
        self.icon_source.borrow().clone()
    }

    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        match &*self.icon_source.borrow() {
            Some(IconSource::System(_)) => Size {
                width: 16.0,
                height: 16.0,
            },
            Some(IconSource::Image(source)) => image_source_intrinsic_size(source),
            None => Size::default(),
        }
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        final_size
    }

    #[overrides]
    fn hit_test_content(&self) -> bool {
        self.icon_source.borrow().is_some()
    }

    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: self.arranged_width().unwrap_or(0.0),
            height: self.arranged_height().unwrap_or(0.0),
        };
        match &*self.icon_source.borrow() {
            Some(IconSource::Image(source)) => render_image_source(
                context,
                source,
                rect,
                ImageFit::Contain,
                VectorRasterizeMode::default(),
            ),
            Some(IconSource::System(icon)) => {
                let foreground = self
                    .base
                    .foreground
                    .borrow()
                    .clone()
                    .or_else(|| inherited_cascaded_text_style(self.as_ui_element()).foreground);
                if let Some(foreground) = foreground {
                    let source = crate::graphics::system_icon_vector(*icon, foreground);
                    render_image_source(
                        context,
                        &source,
                        rect,
                        ImageFit::Contain,
                        VectorRasterizeMode::default(),
                    );
                }
            }
            None => {}
        }
    }

    /// Replaces the displayed value. Passing `None` clears the icon.
    fn set_icon_source(&self, icon_source: Option<IconSource>) {
        *self.icon_source.borrow_mut() = icon_source;
        self.invalidate_measure();
    }

    #[inherent]
    /// Erases the concrete icon element type for insertion into a Visual collection.
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }

    fn construct() -> Self {
        Self {
            base: IconElement::construct(),
            icon_source: RefCell::new(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{
        AlphaMode, BitmapImage, Color, RenderCommand, SystemIcon, VectorImageBuilder, VectorNode,
        VectorPaint,
    };

    fn test_vector(width: f32, height: f32) -> crate::graphics::VectorImage {
        VectorImageBuilder::new(
            Size { width, height },
            Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
        )
        .expect("positive vector bounds")
        .finish()
        .expect("empty vector scene is valid")
    }

    fn render_commands(icon: &IconSourceElement) -> Vec<RenderCommand> {
        let mut commands = Vec::new();
        icon.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        commands
    }

    fn canonical_color(command: &RenderCommand) -> Color {
        let RenderCommand::DrawVectorImage { image, .. } = command else {
            panic!("expected a vector render command");
        };
        let node = image
            .root()
            .children
            .first()
            .expect("canonical icon has geometry");
        let VectorNode::Path(path) = node else {
            panic!("canonical icon root child is a path");
        };
        let paint = match (&path.fill, &path.stroke) {
            (Some(fill), None) => &fill.paint,
            (None, Some(stroke)) => &stroke.paint,
            _ => panic!("canonical icon has exactly one paint operation"),
        };
        let VectorPaint::Brush(Brush::Solid(color)) = paint else {
            panic!("test foreground remains a solid brush");
        };
        *color
    }

    #[test]
    fn intrinsic_measurement_covers_empty_system_raster_and_vector_sources() {
        let icon = IconSourceElement::new();
        icon.measure(Size {
            width: 100.0,
            height: 100.0,
        });
        assert_eq!(icon.measured_size(), Some(Size::default()));

        icon.set_icon_source(Some(IconSource::System(SystemIcon::Copy)));
        icon.measure(Size {
            width: 100.0,
            height: 100.0,
        });
        assert_eq!(
            icon.measured_size(),
            Some(Size {
                width: 16.0,
                height: 16.0
            })
        );

        let bitmap = BitmapImage::from_rgba8(3, 5, 12, vec![255; 3 * 5 * 4], AlphaMode::Straight)
            .expect("valid test bitmap");
        icon.set_icon_source(Some(IconSource::Image(ImageSource::Raster(bitmap))));
        icon.measure(Size {
            width: 100.0,
            height: 100.0,
        });
        assert_eq!(
            icon.measured_size(),
            Some(Size {
                width: 3.0,
                height: 5.0
            })
        );

        icon.set_icon_source(Some(IconSource::Image(ImageSource::Vector(test_vector(
            7.0, 9.0,
        )))));
        icon.measure(Size {
            width: 100.0,
            height: 100.0,
        });
        assert_eq!(
            icon.measured_size(),
            Some(Size {
                width: 7.0,
                height: 9.0
            })
        );
    }

    #[test]
    fn system_icon_uses_local_then_inherited_foreground_and_none_emits_no_paint() {
        let icon = IconSourceElement::new();
        icon.set_icon_source(Some(IconSource::System(SystemIcon::Settings)));
        assert!(render_commands(&icon).is_empty());

        icon.set_foreground(Some(Color::rgb(1, 2, 3).into()));
        let commands = render_commands(&icon);
        assert_eq!(canonical_color(&commands[0]), Color::rgb(1, 2, 3));

        icon.clear_foreground();
        let parent = Control::new();
        parent.set_foreground(Some(Color::rgb(4, 5, 6).into()));
        parent
            .as_ui_element()
            .visual_collection
            .add(Rc::clone(&icon) as Rc<dyn UIElementExt>);
        let commands = render_commands(&icon);
        assert_eq!(canonical_color(&commands[0]), Color::rgb(4, 5, 6));
    }

    #[test]
    fn user_images_keep_the_existing_render_command_path_and_clear_without_stale_paint() {
        let icon = IconSourceElement::new();
        let bitmap = BitmapImage::from_rgba8(1, 1, 4, vec![10, 20, 30, 255], AlphaMode::Straight)
            .expect("valid test bitmap");
        icon.set_icon_source(Some(IconSource::Image(ImageSource::Raster(bitmap))));
        assert!(matches!(
            render_commands(&icon).as_slice(),
            [RenderCommand::DrawImage { .. }]
        ));

        icon.set_icon_source(Some(IconSource::Image(ImageSource::Vector(test_vector(
            2.0, 2.0,
        )))));
        assert!(matches!(
            render_commands(&icon).as_slice(),
            [RenderCommand::DrawVectorImage { .. }]
        ));

        icon.set_icon_source(None);
        assert!(render_commands(&icon).is_empty());
    }
}
