use super::core::base::{Point, Rect, Size};
use super::core::graphics::{
    Brush, ImageSource, LineCap, LineJoin, Path, PathBuilder, StrokeStyle, VectorGroup,
    VectorImageBuilder, VectorNode, VectorPaint, VectorPaintOrder, VectorPathNode,
    VectorShapeRendering, VectorStroke,
};
use super::core::layout::{HorizontalAlignment, VerticalAlignment};
use super::core::ui::{IconSourceElement, IconSourceElementExt, UIElementExt};
use std::rc::Rc;
use std::sync::OnceLock;

/// Small monochrome symbols used by the docking chrome. These are intentionally separate from
/// `SystemIcon`: they are private interaction affordances, not public semantic menu icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeIcon {
    Close,
    Pin,
    Float,
}

impl ChromeIcon {
    fn index(self) -> usize {
        match self {
            Self::Close => 0,
            Self::Pin => 1,
            Self::Float => 2,
        }
    }
}

fn point(x: f32, y: f32) -> Point {
    Point { x, y }
}

fn close_path() -> Path {
    let mut path = PathBuilder::new();
    path.add_line(point(4.0, 4.0), point(12.0, 12.0));
    path.add_line(point(12.0, 4.0), point(4.0, 12.0));
    path.build().expect("static close icon geometry is valid")
}

fn pin_path() -> Path {
    let mut path = PathBuilder::new();
    path.move_to(point(5.0, 3.0))
        .line_to(point(11.0, 3.0))
        .line_to(point(10.2, 5.0))
        .line_to(point(10.7, 8.5))
        .line_to(point(5.3, 8.5))
        .line_to(point(5.8, 5.0))
        .close();
    path.add_line(point(8.0, 8.5), point(8.0, 14.0));
    path.build().expect("static pin icon geometry is valid")
}

fn float_path() -> Path {
    let mut path = PathBuilder::new();
    path.add_rect(Rect {
        x: 4.5,
        y: 2.5,
        width: 9.0,
        height: 9.0,
    });
    path.add_rect(Rect {
        x: 2.5,
        y: 5.5,
        width: 9.0,
        height: 8.0,
    });
    path.build().expect("static float icon geometry is valid")
}

fn chrome_geometry() -> &'static [Path; 3] {
    static CACHE: OnceLock<[Path; 3]> = OnceLock::new();
    CACHE.get_or_init(|| [close_path(), pin_path(), float_path()])
}

/// Creates a crisp, backend-neutral vector glyph with a centered 16x16 drawing box.
pub fn chrome_icon(kind: ChromeIcon, foreground: Option<Brush>) -> Rc<dyn UIElementExt> {
    let brush = foreground
        .unwrap_or_else(|| Brush::Solid(super::core::graphics::Color::rgb(232, 232, 232)));
    let node = VectorNode::Path(VectorPathNode {
        path: chrome_geometry()[kind.index()].clone(),
        transform: super::core::base::AffineTransform::IDENTITY,
        fill: None,
        stroke: Some(VectorStroke {
            paint: VectorPaint::Brush(brush),
            opacity: 1.0,
            style: StrokeStyle {
                width: 1.5,
                start_cap: LineCap::Round,
                end_cap: LineCap::Round,
                dash_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..StrokeStyle::default()
            },
        }),
        paint_order: VectorPaintOrder::default(),
        rendering: VectorShapeRendering::GeometricPrecision,
        visibility: true,
    });
    let image = VectorImageBuilder::new(
        Size {
            width: 16.0,
            height: 16.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 16.0,
            height: 16.0,
        },
    )
    .expect("16x16 chrome icon canvas is valid")
    .root(VectorGroup {
        children: std::sync::Arc::from([node]),
        ..VectorGroup::default()
    })
    .finish()
    .expect("static chrome icon scene is valid");

    let icon = IconSourceElement::new();
    icon.set_icon_source(Some(super::core::graphics::IconSource::Image(
        ImageSource::Vector(image),
    )));
    icon.set_width(16.0);
    icon.set_height(16.0);
    icon.set_horizontal_alignment(HorizontalAlignment::Center);
    icon.set_vertical_alignment(VerticalAlignment::Center);
    icon.into_node()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::graphics::PathCommand;

    #[test]
    fn chrome_symbols_use_cached_vector_paths() {
        let first = chrome_geometry();
        let second = chrome_geometry();
        assert!(std::ptr::eq(first, second));
        assert_eq!(first[ChromeIcon::Close.index()].commands().len(), 4);
        assert!(
            first[ChromeIcon::Pin.index()]
                .commands()
                .iter()
                .any(|command| matches!(command, PathCommand::Close))
        );
        assert_eq!(first[ChromeIcon::Float.index()].commands().len(), 10);
    }
}
