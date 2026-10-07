use super::core::ui::UIElementExt;
use std::rc::{Rc, Weak};

/// Accent for self-drawn active chrome: the Theme's `Primary` when it resolves to a value,
/// otherwise the platform accent, so the chrome stays visible under a platform-default Primary.
pub(crate) fn accent_style() -> super::core::theme::BrushStyle {
    use super::core::{
        environment::application_environment,
        graphics::Color,
        theme::{BrushStyle, ResolvedValue, platform_accent_color},
    };
    match BrushStyle::Primary.resolve(&application_environment()) {
        ResolvedValue::Value(_) => BrushStyle::Primary,
        ResolvedValue::PlatformDefault => BrushStyle::Value(
            platform_accent_color()
                .unwrap_or(Color::rgb(0, 120, 215))
                .into(),
        ),
    }
}

/// Native TabView palette fallback, selected from the live platform/application foreground.
/// Semantic application foreground also identifies explicitly authored light/dark themes.
pub(crate) fn native_tab_dark(foreground_style: super::core::theme::BrushStyle) -> bool {
    use super::core::{
        environment::application_environment, graphics::Brush, theme::ResolvedValue,
    };
    let foreground = match foreground_style.resolve(&application_environment()) {
        ResolvedValue::Value(value) => value,
        ResolvedValue::PlatformDefault => {
            super::core::graphics::text_backend()
                .default_text_style()
                .foreground
        }
    };
    matches!(foreground, Brush::Solid(color) if u16::from(color.r) + u16::from(color.g) + u16::from(color.b) > 384)
}

pub(crate) fn native_tab_background(selected: bool, dark: bool) -> super::core::theme::BrushStyle {
    use super::core::{graphics::Color, theme::BrushStyle};
    let color = if selected {
        if dark {
            Color::rgb(40, 40, 40)
        } else {
            Color::rgb(249, 249, 249)
        }
    } else if dark {
        Color::rgba(255, 255, 255, 15)
    } else {
        Color::rgba(0, 0, 0, 10)
    };
    BrushStyle::Value(color.into())
}

pub(crate) fn native_tab_action(
    pressed: bool,
    hovered: bool,
    dark: bool,
) -> super::core::theme::BrushStyle {
    use super::core::{graphics::Color, theme::BrushStyle};
    let color = if !pressed && !hovered {
        Color::TRANSPARENT
    } else if dark {
        Color::rgba(255, 255, 255, if pressed { 10 } else { 15 })
    } else {
        Color::rgba(0, 0, 0, if pressed { 6 } else { 9 })
    };
    BrushStyle::Value(color.into())
}

pub(crate) fn native_tab_stroke(dark: bool) -> super::core::theme::BrushStyle {
    super::core::theme::BrushStyle::Value(
        super::core::graphics::Color::rgba(0, 0, 0, if dark { 25 } else { 15 }).into(),
    )
}

pub(crate) fn selected_tab_stroke(
    selected: bool,
    connected: bool,
    active: bool,
    dark: bool,
) -> super::core::theme::BrushStyle {
    use super::core::{graphics::Color, theme::BrushStyle};
    if !selected {
        BrushStyle::Value(Color::TRANSPARENT.into())
    } else if !connected {
        native_tab_stroke(dark)
    } else if active {
        accent_style()
    } else {
        BrushStyle::Separator
    }
}

/// Fixed-width edge pieces keep the corner radii constant as the middle tab track stretches.
pub(crate) fn selected_tab_edge(
    right: bool,
    bottom: bool,
    connected: bool,
    dark: bool,
    stroke: super::core::theme::BrushStyle,
) -> Option<super::core::graphics::ImageSource> {
    native_tab_edge(
        right,
        bottom,
        connected,
        if connected {
            super::core::theme::BrushStyle::Background
        } else {
            native_tab_background(true, dark)
        },
        stroke,
    )
}

pub(crate) fn native_tab_edge(
    right: bool,
    bottom: bool,
    connected: bool,
    fill: super::core::theme::BrushStyle,
    stroke: super::core::theme::BrushStyle,
) -> Option<super::core::graphics::ImageSource> {
    use super::core::{
        base::{Point, Rect, Size},
        environment::application_environment,
        graphics::*,
        theme::ResolvedValue,
    };
    let width = if connected { 8.0 } else { 12.0 };
    // Connected edges span the whole 32 px item so their feet end on the strip baseline.
    let height = 32.0;
    let radius = if connected { 4.0 } else { 8.0 };
    let map = |x: f32, y: f32| Point {
        x: if right { width - x } else { x },
        y: if bottom { height - y } else { y },
    };
    let make_edge = || {
        let mut path = PathBuilder::new();
        path.move_to(map(width, 0.5))
            .cubic_to(
                map(4.5 + radius * 0.45, 0.5),
                map(4.5, radius * 0.45),
                map(4.5, radius),
            )
            .line_to(map(4.5, height - 4.5))
            .cubic_to(
                map(4.5, height - 2.3),
                map(2.7, height - 0.5),
                map(0.0, height - 0.5),
            );
        path
    };
    let outline = make_edge().build().ok()?;
    let mut edge = make_edge();
    edge.line_to(map(width, height - 0.5)).close();
    let filled = edge.build().ok()?;
    let resolve =
        |style: super::core::theme::BrushStyle| match style.resolve(&application_environment()) {
            ResolvedValue::Value(brush) => brush,
            ResolvedValue::PlatformDefault => Color::TRANSPARENT.into(),
        };
    let nodes = [
        VectorNode::Path(VectorPathNode {
            path: filled,
            fill: Some(VectorFill {
                paint: VectorPaint::Brush(resolve(fill)),
                opacity: 1.0,
                rule: FillRule::NonZero,
            }),
            stroke: None,
            transform: super::core::base::AffineTransform::IDENTITY,
            paint_order: VectorPaintOrder::default(),
            rendering: VectorShapeRendering::GeometricPrecision,
            visibility: true,
        }),
        VectorNode::Path(VectorPathNode {
            path: outline,
            fill: None,
            stroke: Some(VectorStroke {
                paint: VectorPaint::Brush(resolve(stroke)),
                opacity: 1.0,
                style: StrokeStyle {
                    width: 1.0,
                    ..StrokeStyle::default()
                },
            }),
            transform: super::core::base::AffineTransform::IDENTITY,
            paint_order: VectorPaintOrder::default(),
            rendering: VectorShapeRendering::GeometricPrecision,
            visibility: true,
        }),
    ];
    Some(ImageSource::Vector(
        VectorImageBuilder::new(
            Size { width, height },
            Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
        )
        .ok()?
        .root(VectorGroup {
            children: std::sync::Arc::from(nodes),
            ..VectorGroup::default()
        })
        .finish()
        .ok()?,
    ))
}

/// Recovers the concrete self handle from the Visual collection's owner weak reference.
///
/// Generated component storage is intentionally private to the #[component] expansion. The
/// collection owner is the same most-derived Rc used to initialize that storage, so this keeps
/// callback wiring weak without depending on generated implementation fields that are not part
/// of the rust-analyzer shadow surface.
pub(crate) fn weak_self_from_visual_owner<T: UIElementExt + 'static>(value: &T) -> Weak<T> {
    let owner: Option<Rc<dyn UIElementExt>> = value.as_ui_element().visual_collection.owner_rc();
    let Some(owner) = owner else {
        return Weak::<T>::new();
    };
    assert!(
        owner.as_any().is::<T>(),
        "component Visual collection owner has an unexpected concrete type"
    );
    let raw = Rc::into_raw(owner) as *const () as *const T;
    // SAFETY: the owner was checked against the exact T type above. Reconstructing this
    // temporary Rc<T> preserves the original strong count; it is dropped normally after the
    // weak callback handle is derived.
    let owner = unsafe { Rc::from_raw(raw) };
    let weak = Rc::downgrade(&owner);
    drop(owner);
    weak
}

#[cfg(test)]
mod tests {
    use super::super::CustomTabView;
    use super::super::core::ui::Control;
    use super::*;

    #[test]
    fn selected_native_tab_edges_have_opaque_theme_fill_in_every_orientation() {
        use super::super::core::graphics::{Brush, Color, ImageSource, VectorNode, VectorPaint};
        for dark in [false, true] {
            for right in [false, true] {
                for bottom in [false, true] {
                    let Some(ImageSource::Vector(image)) =
                        selected_tab_edge(right, bottom, false, dark, native_tab_stroke(dark))
                    else {
                        panic!("selected tab edge must contain vector chrome");
                    };
                    let VectorNode::Path(edge) = &image.root().children[0] else {
                        panic!("selected edge must start with the filled silhouette");
                    };
                    let fill = edge.fill.as_ref().expect("selected silhouette fill");
                    assert_eq!(fill.opacity, 1.0);
                    let VectorPaint::Brush(Brush::Solid(color)) = &fill.paint else {
                        panic!("selected edge fill must be a solid brush");
                    };
                    assert_eq!(
                        *color,
                        if dark {
                            Color::rgb(40, 40, 40)
                        } else {
                            Color::rgb(249, 249, 249)
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn native_palette_tracks_light_and_dark_foregrounds_and_action_states() {
        use super::super::core::{graphics::Color, theme::BrushStyle};
        assert!(!native_tab_dark(BrushStyle::Value(Color::BLACK.into())));
        assert!(native_tab_dark(BrushStyle::Value(Color::WHITE.into())));
        assert_eq!(
            native_tab_background(true, false),
            BrushStyle::Value(Color::rgb(249, 249, 249).into())
        );
        assert_eq!(
            native_tab_background(true, true),
            BrushStyle::Value(Color::rgb(40, 40, 40).into())
        );
        assert_eq!(
            native_tab_action(false, false, true),
            BrushStyle::Value(Color::TRANSPARENT.into())
        );
        assert_ne!(
            native_tab_action(true, true, true),
            native_tab_action(false, true, true)
        );
        assert!(
            native_tab_edge(
                false,
                false,
                false,
                native_tab_background(true, false),
                native_tab_stroke(false)
            )
            .is_some()
        );
        assert!(
            native_tab_edge(
                true,
                true,
                true,
                native_tab_background(true, true),
                native_tab_stroke(true)
            )
            .is_some()
        );
    }

    #[test]
    fn weak_self_helper_does_not_leak_owner_strong_reference() {
        let owner = Control::new();
        let original = Rc::downgrade(&owner);
        let strong_before = Rc::strong_count(&owner);

        let returned = weak_self_from_visual_owner(owner.as_ref());

        assert!(returned.upgrade().is_some());
        assert_eq!(Rc::strong_count(&owner), strong_before);

        drop(returned.upgrade());
        drop(owner);

        assert!(original.upgrade().is_none());
        assert!(returned.upgrade().is_none());
    }

    #[test]
    fn weak_self_helper_does_not_leak_component_owner_strong_reference() {
        let owner = CustomTabView::new_view();
        let original = Rc::downgrade(&owner);
        let strong_before = Rc::strong_count(&owner);
        let returned = weak_self_from_visual_owner(owner.as_ref());

        assert_eq!(Rc::strong_count(&owner), strong_before);
        drop(owner);

        assert!(original.upgrade().is_none());
        assert!(returned.upgrade().is_none());
    }
}
