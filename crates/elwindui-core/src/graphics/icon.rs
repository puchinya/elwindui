//! `IconSource`/`SystemIcon` — the backend-neutral menu icon value types (規範仕様:
//! `docs/specs/ui_spec.md` §9 Menu, `docs/specs/graphics_spec.md` §9 Icons; durable design:
//! `docs/design/runtime/icon_source_design.md`). `SystemIcon` never exposes a backend-native
//! identifier (SF Symbol name, WinUI `Symbol` name, GTK icon name); only the fixed 12-variant
//! common subset documented there may be added to, and only once every supported backend can
//! express the same semantic meaning.

use super::brush::Brush;
use super::path::{FillRule, Path, PathBuilder};
use super::stroke::{LineCap, LineJoin, StrokeStyle};
use super::vector_image::{ImageSource, VectorImageBuilder};
use super::vector_scene::{
    VectorFill, VectorGroup, VectorNode, VectorPaint, VectorPaintOrder, VectorPathNode,
    VectorShapeRendering, VectorStroke,
};
use crate::base::{AffineTransform, Point, Rect, Size};
use std::sync::{Arc, OnceLock};

/// A shareable icon value: either a user-supplied `ImageSource` or a backend-neutral `SystemIcon`.
///
/// `MenuItem.icon` consumes this value directly; [`crate::ui::IconSourceElement`] wraps it when an
/// icon must participate in the Visual tree. The value itself never owns a `UIElement` or backend
/// native handle.
#[derive(Debug, Clone)]
pub enum IconSource {
    System(SystemIcon),
    Image(ImageSource),
}

/// A semantic, backend-neutral system icon. `#[non_exhaustive]`: a new variant may only be added
/// once its meaning is confirmed identical across every backend ElwindUI targets (see the mapping
/// table in `docs/design/runtime/icon_source_design.md` §2).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemIcon {
    Add,
    Remove,
    Delete,
    Edit,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    Search,
    Settings,
    Refresh,
}

impl SystemIcon {
    /// Every currently-defined variant, in declaration order. `pub(crate)` only — exists for
    /// exhaustive internal/test coverage (e.g. mapping-completeness tests), not as public API.
    pub(crate) const ALL: [SystemIcon; 12] = [
        SystemIcon::Add,
        SystemIcon::Remove,
        SystemIcon::Delete,
        SystemIcon::Edit,
        SystemIcon::Copy,
        SystemIcon::Cut,
        SystemIcon::Paste,
        SystemIcon::Undo,
        SystemIcon::Redo,
        SystemIcon::Search,
        SystemIcon::Settings,
        SystemIcon::Refresh,
    ];

    fn index(self) -> usize {
        match self {
            SystemIcon::Add => 0,
            SystemIcon::Remove => 1,
            SystemIcon::Delete => 2,
            SystemIcon::Edit => 3,
            SystemIcon::Copy => 4,
            SystemIcon::Cut => 5,
            SystemIcon::Paste => 6,
            SystemIcon::Undo => 7,
            SystemIcon::Redo => 8,
            SystemIcon::Search => 9,
            SystemIcon::Settings => 10,
            SystemIcon::Refresh => 11,
        }
    }
}

fn pt(x: f32, y: f32) -> Point {
    Point { x, y }
}

/// Whether a `SystemIcon`'s canonical geometry is painted by filling it or by stroking its
/// outline — a per-icon authoring choice, not something callers pick.
#[derive(Clone, Copy)]
enum IconPaint {
    Fill(FillRule),
    Stroke,
}

fn build_add_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_rect(Rect {
        x: 7.0,
        y: 3.0,
        width: 2.0,
        height: 10.0,
    });
    b.add_rect(Rect {
        x: 3.0,
        y: 7.0,
        width: 10.0,
        height: 2.0,
    });
    b.build().expect("static add icon geometry is well-formed")
}

fn build_remove_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_rect(Rect {
        x: 3.0,
        y: 7.0,
        width: 10.0,
        height: 2.0,
    });
    b.build()
        .expect("static remove icon geometry is well-formed")
}

fn build_delete_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_line(pt(3.0, 4.0), pt(13.0, 4.0));
    b.add_rect(Rect {
        x: 6.0,
        y: 2.0,
        width: 4.0,
        height: 2.0,
    });
    b.add_rect(Rect {
        x: 4.0,
        y: 4.0,
        width: 8.0,
        height: 10.0,
    });
    b.add_line(pt(6.5, 6.0), pt(6.5, 12.0));
    b.add_line(pt(9.5, 6.0), pt(9.5, 12.0));
    b.build()
        .expect("static delete icon geometry is well-formed")
}

fn build_edit_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_line(pt(3.0, 13.0), pt(10.0, 6.0));
    b.add_line(pt(10.0, 6.0), pt(13.0, 3.0));
    b.build().expect("static edit icon geometry is well-formed")
}

fn build_copy_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_rect(Rect {
        x: 3.0,
        y: 3.0,
        width: 7.0,
        height: 7.0,
    });
    b.add_rect(Rect {
        x: 6.0,
        y: 6.0,
        width: 7.0,
        height: 7.0,
    });
    b.build().expect("static copy icon geometry is well-formed")
}

fn build_cut_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_line(pt(3.0, 3.0), pt(13.0, 13.0));
    b.add_line(pt(13.0, 3.0), pt(3.0, 13.0));
    b.build().expect("static cut icon geometry is well-formed")
}

fn build_paste_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_rect(Rect {
        x: 4.0,
        y: 3.0,
        width: 8.0,
        height: 11.0,
    });
    b.add_rect(Rect {
        x: 6.0,
        y: 2.0,
        width: 4.0,
        height: 2.0,
    });
    b.add_line(pt(6.0, 7.0), pt(10.0, 7.0));
    b.add_line(pt(6.0, 9.5), pt(10.0, 9.5));
    b.add_line(pt(6.0, 12.0), pt(9.0, 12.0));
    b.build()
        .expect("static paste icon geometry is well-formed")
}

fn build_undo_path() -> Path {
    let mut b = PathBuilder::new();
    b.arc_center(
        pt(8.0, 9.0),
        Size {
            width: 5.0,
            height: 5.0,
        },
        0.0,
        -std::f32::consts::PI,
    );
    b.add_line(pt(3.0, 9.0), pt(6.0, 6.5));
    b.add_line(pt(3.0, 9.0), pt(5.0, 11.5));
    b.build().expect("static undo icon geometry is well-formed")
}

fn build_redo_path() -> Path {
    let mut b = PathBuilder::new();
    b.arc_center(
        pt(8.0, 9.0),
        Size {
            width: 5.0,
            height: 5.0,
        },
        std::f32::consts::PI,
        std::f32::consts::PI,
    );
    b.add_line(pt(13.0, 9.0), pt(10.0, 6.5));
    b.add_line(pt(13.0, 9.0), pt(11.0, 11.5));
    b.build().expect("static redo icon geometry is well-formed")
}

fn build_search_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_circle(pt(6.5, 6.5), 4.0);
    b.add_line(pt(9.5, 9.5), pt(13.0, 13.0));
    b.build()
        .expect("static search icon geometry is well-formed")
}

fn build_settings_path() -> Path {
    let mut b = PathBuilder::new();
    b.add_circle(pt(8.0, 8.0), 5.0);
    b.add_circle(pt(8.0, 8.0), 2.7);
    b.add_rect(Rect {
        x: 7.0,
        y: 0.5,
        width: 2.0,
        height: 2.0,
    });
    b.add_rect(Rect {
        x: 7.0,
        y: 13.5,
        width: 2.0,
        height: 2.0,
    });
    b.add_rect(Rect {
        x: 0.5,
        y: 7.0,
        width: 2.0,
        height: 2.0,
    });
    b.add_rect(Rect {
        x: 13.5,
        y: 7.0,
        width: 2.0,
        height: 2.0,
    });
    b.build()
        .expect("static settings icon geometry is well-formed")
}

fn build_refresh_path() -> Path {
    let mut b = PathBuilder::new();
    b.arc_center(
        pt(8.0, 8.0),
        Size {
            width: 6.0,
            height: 6.0,
        },
        (-30.0f32).to_radians(),
        300.0f32.to_radians(),
    );
    b.add_line(pt(8.0, 2.0), pt(5.2, 3.2));
    b.add_line(pt(8.0, 2.0), pt(9.8, 4.4));
    b.build()
        .expect("static refresh icon geometry is well-formed")
}

/// The 12 canonical vectors' geometry, built once and cached for the process lifetime — the key
/// space is fixed (`SystemIcon::ALL`), so this bounded cache doesn't grow (`icon_source_design.md`
/// §4). Only the `Path` (the expensive, static part) is cached; foreground is applied per call in
/// [`system_icon_vector`], since it varies with the caller's enabled/disabled foreground.
fn icon_geometry() -> &'static [(Path, IconPaint); 12] {
    static CACHE: OnceLock<[(Path, IconPaint); 12]> = OnceLock::new();
    CACHE.get_or_init(|| {
        [
            (build_add_path(), IconPaint::Fill(FillRule::NonZero)),
            (build_remove_path(), IconPaint::Fill(FillRule::NonZero)),
            (build_delete_path(), IconPaint::Stroke),
            (build_edit_path(), IconPaint::Stroke),
            (build_copy_path(), IconPaint::Stroke),
            (build_cut_path(), IconPaint::Stroke),
            (build_paste_path(), IconPaint::Stroke),
            (build_undo_path(), IconPaint::Stroke),
            (build_redo_path(), IconPaint::Stroke),
            (build_search_path(), IconPaint::Stroke),
            (build_settings_path(), IconPaint::Fill(FillRule::EvenOdd)),
            (build_refresh_path(), IconPaint::Stroke),
        ]
    })
}

/// Core's canonical monochrome vector realization for a `SystemIcon` — never the `SystemIcon`'s
/// public semantic identity, only a backend-neutral visual used by `IconSourceElement` (including
/// the Custom Context Menu; native presentation uses the OS's own system icon instead — see
/// `icon_source_design.md` §3). Intrinsic size `16x16`, viewBox `(0,0,16,16)`.
pub(crate) fn system_icon_vector(icon: SystemIcon, foreground: Brush) -> ImageSource {
    let (path, paint) = &icon_geometry()[icon.index()];
    let node = VectorNode::Path(VectorPathNode {
        path: path.clone(),
        transform: AffineTransform::IDENTITY,
        fill: match paint {
            IconPaint::Fill(rule) => Some(VectorFill {
                paint: VectorPaint::Brush(foreground.clone()),
                opacity: 1.0,
                rule: *rule,
            }),
            IconPaint::Stroke => None,
        },
        stroke: match paint {
            IconPaint::Fill(_) => None,
            IconPaint::Stroke => Some(VectorStroke {
                paint: VectorPaint::Brush(foreground),
                opacity: 1.0,
                style: StrokeStyle {
                    width: 1.4,
                    start_cap: LineCap::Round,
                    end_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..StrokeStyle::default()
                },
            }),
        },
        paint_order: VectorPaintOrder::default(),
        rendering: VectorShapeRendering::default(),
        visibility: true,
    });
    let group = VectorGroup {
        children: Arc::from([node]),
        ..VectorGroup::default()
    };
    let vector_image = VectorImageBuilder::new(
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
    .expect("16x16 icon canvas is always a valid finite positive size")
    .root(group)
    .finish()
    .expect("static icon geometry always builds a valid VectorImage");
    ImageSource::Vector(vector_image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::Color;

    #[test]
    fn every_system_icon_variant_produces_a_16x16_vector() {
        for icon in SystemIcon::ALL {
            let source = system_icon_vector(icon, Color::rgb(240, 240, 240).into());
            match source {
                ImageSource::Vector(vector) => {
                    assert_eq!(
                        vector.intrinsic_size(),
                        Size {
                            width: 16.0,
                            height: 16.0
                        },
                        "{icon:?} canonical vector must be 16x16"
                    );
                    assert_eq!(
                        vector.view_box(),
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 16.0,
                            height: 16.0
                        },
                        "{icon:?} canonical vector viewBox must be (0,0,16,16)"
                    );
                    assert!(
                        !vector.root().children.is_empty(),
                        "{icon:?} canonical vector must not be empty geometry"
                    );
                }
                ImageSource::Raster(_) => {
                    panic!("{icon:?} canonical fallback must be a vector, not a raster")
                }
            }
        }
    }

    #[test]
    fn disabled_color_reaches_the_canonical_vector_paint() {
        let enabled = Color::rgb(240, 240, 240);
        let disabled = Color::rgb(128, 128, 128);
        for icon in SystemIcon::ALL {
            let enabled_source = system_icon_vector(icon, enabled.into());
            let disabled_source = system_icon_vector(icon, disabled.into());
            let color_of = |source: ImageSource| -> Color {
                let ImageSource::Vector(vector) = source else {
                    panic!("canonical fallback must be a vector");
                };
                let node = vector
                    .root()
                    .children
                    .first()
                    .expect("canonical icon geometry has at least one node");
                match node {
                    VectorNode::Path(path_node) => match (&path_node.fill, &path_node.stroke) {
                        (Some(fill), None) => match &fill.paint {
                            VectorPaint::Brush(Brush::Solid(color)) => *color,
                            other => panic!("unexpected fill paint: {other:?}"),
                        },
                        (None, Some(stroke)) => match &stroke.paint {
                            VectorPaint::Brush(Brush::Solid(color)) => *color,
                            other => panic!("unexpected stroke paint: {other:?}"),
                        },
                        other => panic!("expected exactly one of fill/stroke, got {other:?}"),
                    },
                    other => panic!("expected a path node, got {other:?}"),
                }
            };
            assert_eq!(color_of(enabled_source), enabled);
            assert_eq!(color_of(disabled_source), disabled);
        }
    }

    #[test]
    fn repeated_calls_reuse_the_same_cached_geometry() {
        let a = icon_geometry();
        let b = icon_geometry();
        assert!(std::ptr::eq(a, b), "geometry cache must be built once");
    }
}
