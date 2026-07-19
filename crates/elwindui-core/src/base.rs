use std::any::Any;

/// Lets the generic RenderTree builder downcast a `&dyn UIElement` to a concrete
/// `H` to pull out its handle — the *only* place `native_handle`-style access
/// exists (deliberately not a method on `UIElement` itself: every other implementor would have to
/// carry a meaningless default for a concept that doesn't apply to it). Blanket-implemented for
/// every `'static` type, so no concrete `UIElement` impl needs its own boilerplate.
pub trait AsAny: Any {
    fn as_any(&self) -> &dyn Any;
}
impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// See docs/elwindui_spec.md 付録H.2. Common geometry primitives shared across layout
/// (`elwindui_core::layout`), painting (`elwindui_core::painter`), input
/// (`elwindui_core::input`), and every backend crate — kept in their own module rather than
/// under `layout`/`painter` since neither of those is the "owner" of a concept this widely used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A direction/magnitude pair, as distinct from `Point` (a location) — WinUI3's
/// `Windows.Foundation.Numerics.Vector2` role for offsets and deltas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vector {
    pub x: f32,
    pub y: f32,
}

/// Per-corner rounding radii, one value per corner rather than a single uniform radius —
/// WinUI3's `CornerRadius`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CornerRadius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl CornerRadius {
    pub const fn uniform(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }
}

/// A general 2D affine transform (row-vector convention: `x' = m11*x + m21*y + dx`,
/// `y' = m12*x + m22*y + dy`), matching `Windows.Foundation.Numerics.Matrix3x2`/`CGAffineTransform`.
/// Every backend maps this directly onto its own native affine matrix type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineTransform {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}

impl AffineTransform {
    pub const IDENTITY: Self = Self {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    pub const fn identity() -> Self {
        Self::IDENTITY
    }

    pub const fn translation(dx: f32, dy: f32) -> Self {
        Self {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx,
            dy,
        }
    }

    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self {
            m11: sx,
            m12: 0.0,
            m21: 0.0,
            m22: sy,
            dx: 0.0,
            dy: 0.0,
        }
    }

    pub fn rotation(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self {
            m11: cos,
            m12: sin,
            m21: -sin,
            m22: cos,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// `x_radians`/`y_radians` skew the x and y axes independently, mirroring
    /// `CGAffineTransform(a: 1, b: tan(y), c: tan(x), d: 1, tx: 0, ty: 0)`-style construction.
    pub fn skew(x_radians: f32, y_radians: f32) -> Self {
        Self {
            m11: 1.0,
            m12: y_radians.tan(),
            m21: x_radians.tan(),
            m22: 1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }

    /// Returns the transform that applies `other` first, then `self` — i.e.
    /// `self.concat(&other).transform_point(p) == self.transform_point(other.transform_point(p))`.
    #[must_use]
    pub fn concat(&self, other: &Self) -> Self {
        Self {
            m11: self.m11 * other.m11 + self.m21 * other.m12,
            m12: self.m12 * other.m11 + self.m22 * other.m12,
            m21: self.m11 * other.m21 + self.m21 * other.m22,
            m22: self.m12 * other.m21 + self.m22 * other.m22,
            dx: self.m11 * other.dx + self.m21 * other.dy + self.dx,
            dy: self.m12 * other.dx + self.m22 * other.dy + self.dy,
        }
    }

    pub fn transform_point(&self, point: Point) -> Point {
        Point {
            x: self.m11 * point.x + self.m21 * point.y + self.dx,
            y: self.m12 * point.x + self.m22 * point.y + self.dy,
        }
    }

    pub fn transform_vector(&self, vector: Vector) -> Vector {
        Vector {
            x: self.m11 * vector.x + self.m21 * vector.y,
            y: self.m12 * vector.x + self.m22 * vector.y,
        }
    }
}

impl Default for AffineTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod affine_transform_tests {
    use super::*;

    #[test]
    fn identity_is_a_no_op() {
        let p = Point { x: 3.0, y: 4.0 };
        assert_eq!(AffineTransform::identity().transform_point(p), p);
    }

    #[test]
    fn translation_offsets_points() {
        let t = AffineTransform::translation(10.0, -5.0);
        assert_eq!(
            t.transform_point(Point { x: 1.0, y: 1.0 }),
            Point { x: 11.0, y: -4.0 }
        );
    }

    #[test]
    fn scale_scales_from_origin() {
        let t = AffineTransform::scale(2.0, 3.0);
        assert_eq!(
            t.transform_point(Point { x: 2.0, y: 2.0 }),
            Point { x: 4.0, y: 6.0 }
        );
    }

    #[test]
    fn concat_applies_other_then_self() {
        let translate = AffineTransform::translation(10.0, 0.0);
        let scale = AffineTransform::scale(2.0, 2.0);
        // scale.concat(&translate): translate first, then scale => (1+10)*2 = 22
        let combined = scale.concat(&translate);
        assert_eq!(
            combined.transform_point(Point { x: 1.0, y: 0.0 }),
            Point { x: 22.0, y: 0.0 }
        );
    }

    #[test]
    fn concat_matches_sequential_application_with_rotation() {
        let rotate = AffineTransform::rotation(std::f32::consts::FRAC_PI_2);
        let translate = AffineTransform::translation(5.0, 0.0);
        let combined = rotate.concat(&translate);
        let p = Point { x: 1.0, y: 0.0 };
        let sequential = rotate.transform_point(translate.transform_point(p));
        let direct = combined.transform_point(p);
        assert!((sequential.x - direct.x).abs() < 1e-4);
        assert!((sequential.y - direct.y).abs() < 1e-4);
    }
}
