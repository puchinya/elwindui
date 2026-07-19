//! Adapter onto the `flo_curves` crate's bezier-path boolean arithmetic (`path_add`/`path_sub`/
//! `path_intersect`), used exclusively by [`super::path::Path::combine`]. Kept in its own module
//! since it exists only to bridge our f32 `Point`/`PathCommand` representation onto
//! `flo_curves`'s f64 `Coordinate`/`BezierPath` traits — see painter design doc §2.4/§9.4 for why
//! this is an adopted external library rather than a self-written boolean-geometry algorithm.

use super::path::{GeometryCombineMode, GeometryError, Path, PathCommand};
use crate::base::Point;
use flo_curves::bezier::path::{
    BezierPath, BezierPathFactory, SimpleBezierPath, path_add, path_intersect, path_sub,
};
use flo_curves::geo::Coord2;

fn to_flo_paths(path: &Path) -> Vec<SimpleBezierPath> {
    super::path::to_cubic_subpaths_pub(path)
        .into_iter()
        .map(|(start, segments)| {
            let points = segments
                .into_iter()
                .map(|(c1, c2, to)| (to_coord(c1), to_coord(c2), to_coord(to)));
            SimpleBezierPath::from_points(to_coord(start), points)
        })
        .collect()
}

fn to_coord(p: Point) -> Coord2 {
    Coord2(p.x as f64, p.y as f64)
}

fn from_coord(c: Coord2) -> Point {
    Point {
        x: c.0 as f32,
        y: c.1 as f32,
    }
}

fn flo_path_to_commands(path: &SimpleBezierPath, out: &mut Vec<PathCommand>) {
    out.push(PathCommand::MoveTo(from_coord(path.start_point())));
    for (c1, c2, to) in path.points() {
        out.push(PathCommand::CubicTo {
            control1: from_coord(c1),
            control2: from_coord(c2),
            to: from_coord(to),
        });
    }
    out.push(PathCommand::Close);
}

pub(super) fn combine(
    a: &Path,
    b: &Path,
    mode: GeometryCombineMode,
    tolerance: f32,
) -> Result<Path, GeometryError> {
    let a = to_flo_paths(a);
    let b = to_flo_paths(b);
    let accuracy = (tolerance as f64).max(1e-6);

    let result: Vec<SimpleBezierPath> = match mode {
        GeometryCombineMode::Union => path_add(&a, &b, accuracy),
        GeometryCombineMode::Intersect => path_intersect(&a, &b, accuracy),
        GeometryCombineMode::Exclude => path_sub(&a, &b, accuracy),
        GeometryCombineMode::Xor => {
            let a_minus_b: Vec<SimpleBezierPath> = path_sub(&a, &b, accuracy);
            let b_minus_a: Vec<SimpleBezierPath> = path_sub(&b, &a, accuracy);
            path_add(&a_minus_b, &b_minus_a, accuracy)
        }
    };

    if result.is_empty() {
        return Err(GeometryError);
    }

    let mut commands = Vec::new();
    for subpath in &result {
        flo_path_to_commands(subpath, &mut commands);
    }
    super::path::path_from_commands(commands).ok_or(GeometryError)
}

#[cfg(test)]
mod tests {
    use super::super::path::PathBuilder;
    use super::*;
    use crate::base::Rect;

    #[test]
    fn union_of_disjoint_rects_is_non_empty() {
        let mut a = PathBuilder::new();
        a.add_rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        let a = a.build().unwrap();
        let mut b = PathBuilder::new();
        b.add_rect(Rect {
            x: 20.0,
            y: 20.0,
            width: 10.0,
            height: 10.0,
        });
        let b = b.build().unwrap();
        let combined = Path::combine(&a, &b, GeometryCombineMode::Union, 0.01).unwrap();
        assert!(!combined.is_empty());
    }

    #[test]
    fn intersect_of_disjoint_rects_is_an_error() {
        let mut a = PathBuilder::new();
        a.add_rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        let a = a.build().unwrap();
        let mut b = PathBuilder::new();
        b.add_rect(Rect {
            x: 20.0,
            y: 20.0,
            width: 10.0,
            height: 10.0,
        });
        let b = b.build().unwrap();
        assert!(Path::combine(&a, &b, GeometryCombineMode::Intersect, 0.01).is_err());
    }

    #[test]
    fn intersect_of_overlapping_rects_contains_overlap_center() {
        let mut a = PathBuilder::new();
        a.add_rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        let a = a.build().unwrap();
        let mut b = PathBuilder::new();
        b.add_rect(Rect {
            x: 5.0,
            y: 5.0,
            width: 10.0,
            height: 10.0,
        });
        let b = b.build().unwrap();
        let combined = Path::combine(&a, &b, GeometryCombineMode::Intersect, 0.01).unwrap();
        assert!(combined.contains(
            Point { x: 7.0, y: 7.0 },
            super::super::path::FillRule::NonZero,
            None
        ));
    }
}
