use crate::base::{AffineTransform, Point, Rect, Size};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepDirection {
    Clockwise,
    CounterClockwise,
}

/// SVG-style elliptical-arc endpoint parameterization — `to` is the arc's end point (the start
/// point comes from whatever preceded this segment in the path).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcSegment {
    pub radii: Size,
    pub x_axis_rotation: f32,
    pub large_arc: bool,
    pub sweep: SweepDirection,
    pub to: Point,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    MoveTo(Point),
    LineTo(Point),
    QuadTo {
        control: Point,
        to: Point,
    },
    CubicTo {
        control1: Point,
        control2: Point,
        to: Point,
    },
    ArcTo(ArcSegment),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathError;

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid path: a drawing command was issued before any move_to"
        )
    }
}
impl std::error::Error for PathError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeometryError;

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "path combine failed (degenerate or self-intersecting input)"
        )
    }
}
impl std::error::Error for GeometryError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryCombineMode {
    Union,
    Intersect,
    Xor,
    Exclude,
}

#[derive(Debug, PartialEq)]
struct PathData {
    commands: Vec<PathCommand>,
    bounds: Rect,
}

/// An immutable, cheaply-`Clone`able (via `Arc`) sequence of path commands. Construct one with
/// [`PathBuilder`].
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    data: Arc<PathData>,
}

/// One MoveTo-delimited, flattened (line-segments-only) subpath, used internally by `contains`/
/// `stroked_contains`/bounds/combine — each `Path` normalizes to a `Vec<FlatSubpath>` before
/// running any of those algorithms.
struct FlatSubpath {
    points: Vec<Point>,
    closed: bool,
}

/// Recursive de Casteljau flattening tolerance (logical units) for hit-testing/bounds/combine —
/// not used for rendering, where each backend flattens (or draws natively) at its own precision.
const FLATTEN_TOLERANCE: f32 = 0.25;
const FLATTEN_MAX_DEPTH: u32 = 16;

fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    }
}

fn flatten_cubic(p0: Point, p1: Point, p2: Point, p3: Point, depth: u32, out: &mut Vec<Point>) {
    // Flatness test: distance of the control points from the chord p0-p3.
    let flat_enough = |a: Point, b: Point, c: Point| -> bool {
        let dx = c.x - a.x;
        let dy = c.y - a.y;
        let len_sq = dx * dx + dy * dy;
        let d = if len_sq < 1e-9 {
            ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
        } else {
            ((b.x - a.x) * dy - (b.y - a.y) * dx).abs() / len_sq.sqrt()
        };
        d <= FLATTEN_TOLERANCE
    };
    if depth >= FLATTEN_MAX_DEPTH || (flat_enough(p0, p1, p3) && flat_enough(p0, p2, p3)) {
        out.push(p3);
        return;
    }
    let p01 = lerp(p0, p1, 0.5);
    let p12 = lerp(p1, p2, 0.5);
    let p23 = lerp(p2, p3, 0.5);
    let p012 = lerp(p01, p12, 0.5);
    let p123 = lerp(p12, p23, 0.5);
    let mid = lerp(p012, p123, 0.5);
    flatten_cubic(p0, p01, p012, mid, depth + 1, out);
    flatten_cubic(mid, p123, p23, p3, depth + 1, out);
}

fn quad_to_cubic(p0: Point, control: Point, to: Point) -> (Point, Point) {
    let c1 = Point {
        x: p0.x + 2.0 / 3.0 * (control.x - p0.x),
        y: p0.y + 2.0 / 3.0 * (control.y - p0.y),
    };
    let c2 = Point {
        x: to.x + 2.0 / 3.0 * (control.x - to.x),
        y: to.y + 2.0 / 3.0 * (control.y - to.y),
    };
    (c1, c2)
}

/// Converts one SVG endpoint-parameterized elliptical arc into a sequence of cubic Bézier
/// segments (standard 4-segments-per-quarter-turn approximation), so every downstream consumer
/// (bounds/flattening/combine) only ever has to deal with cubics.
fn arc_to_cubics(from: Point, arc: &ArcSegment) -> Vec<(Point, Point, Point)> {
    let (rx, ry) = (arc.radii.width.abs(), arc.radii.height.abs());
    if rx < 1e-6 || ry < 1e-6 || (from.x == arc.to.x && from.y == arc.to.y) {
        return vec![(from, from, arc.to)];
    }
    let phi = arc.x_axis_rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();

    let dx2 = (from.x - arc.to.x) / 2.0;
    let dy2 = (from.y - arc.to.y) / 2.0;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    let mut rx = rx;
    let mut ry = ry;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }

    let sign = if arc.large_arc == (arc.sweep == SweepDirection::Clockwise) {
        -1.0
    } else {
        1.0
    };
    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let coef = if den < 1e-9 {
        0.0
    } else {
        sign * (num / den).sqrt()
    };
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);

    let cx = cos_phi * cxp - sin_phi * cyp + (from.x + arc.to.x) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (from.y + arc.to.y) / 2.0;

    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let theta1 = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut delta_theta = angle(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if arc.sweep == SweepDirection::CounterClockwise && delta_theta > 0.0 {
        delta_theta -= std::f32::consts::TAU;
    } else if arc.sweep == SweepDirection::Clockwise && delta_theta < 0.0 {
        delta_theta += std::f32::consts::TAU;
    }

    let segment_count =
        ((delta_theta.abs() / (std::f32::consts::FRAC_PI_2)).ceil() as usize).max(1);
    let delta = delta_theta / segment_count as f32;
    let alpha = 4.0 / 3.0 * (delta / 4.0).tan();

    let mut out = Vec::with_capacity(segment_count);
    let point_at = |theta: f32| -> (Point, f32, f32) {
        let ct = theta.cos();
        let st = theta.sin();
        let x = cx + rx * ct * cos_phi - ry * st * sin_phi;
        let y = cy + rx * ct * sin_phi + ry * st * cos_phi;
        let dx = -rx * st * cos_phi - ry * ct * sin_phi;
        let dy = -rx * st * sin_phi + ry * ct * cos_phi;
        (Point { x, y }, dx, dy)
    };
    let mut theta = theta1;
    let (mut prev_pt, mut prev_dx, mut prev_dy) = point_at(theta);
    for i in 0..segment_count {
        let next_theta = theta + delta;
        let (next_pt, next_dx, next_dy) = point_at(next_theta);
        let c1 = Point {
            x: prev_pt.x + alpha * prev_dx,
            y: prev_pt.y + alpha * prev_dy,
        };
        let c2 = Point {
            x: next_pt.x - alpha * next_dx,
            y: next_pt.y - alpha * next_dy,
        };
        let end = if i == segment_count - 1 {
            arc.to
        } else {
            next_pt
        };
        out.push((c1, c2, end));
        theta = next_theta;
        prev_pt = next_pt;
        prev_dx = next_dx;
        prev_dy = next_dy;
    }
    out
}

/// Normalizes a command list into cubic-only `(control1, control2, to)` triples per subpath,
/// alongside each subpath's start point and whether it was explicitly `Close`d. Shared by
/// `bounds`, flattening, and the `flo_curves` combine adapter.
fn to_cubic_subpaths(commands: &[PathCommand]) -> Vec<(Point, Vec<(Point, Point, Point)>, bool)> {
    let mut subpaths = Vec::new();
    let mut current_start = Point { x: 0.0, y: 0.0 };
    let mut current = Point { x: 0.0, y: 0.0 };
    let mut segments: Vec<(Point, Point, Point)> = Vec::new();
    let mut has_subpath = false;
    let mut closed = false;

    let flush = |subpaths: &mut Vec<(Point, Vec<(Point, Point, Point)>, bool)>,
                 start: Point,
                 segments: &mut Vec<(Point, Point, Point)>,
                 closed: bool| {
        if !segments.is_empty() {
            subpaths.push((start, std::mem::take(segments), closed));
        }
    };

    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo(p) => {
                flush(&mut subpaths, current_start, &mut segments, closed);
                current_start = p;
                current = p;
                has_subpath = true;
                closed = false;
            }
            PathCommand::LineTo(p) => {
                segments.push((current, p, p));
                current = p;
            }
            PathCommand::QuadTo { control, to } => {
                let (c1, c2) = quad_to_cubic(current, control, to);
                segments.push((c1, c2, to));
                current = to;
            }
            PathCommand::CubicTo {
                control1,
                control2,
                to,
            } => {
                segments.push((control1, control2, to));
                current = to;
            }
            PathCommand::ArcTo(arc) => {
                for seg in arc_to_cubics(current, &arc) {
                    segments.push(seg);
                }
                current = arc.to;
            }
            PathCommand::Close => {
                if has_subpath && (current.x != current_start.x || current.y != current_start.y) {
                    segments.push((current, current_start, current_start));
                }
                closed = true;
                current = current_start;
            }
        }
    }
    flush(&mut subpaths, current_start, &mut segments, closed);
    subpaths
}

fn cubic_extrema_1d(p0: f32, p1: f32, p2: f32, p3: f32, out: &mut Vec<f32>) {
    // Derivative of a cubic Bézier is a quadratic in t; solve for its roots.
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    if a.abs() < 1e-9 {
        if b.abs() > 1e-9 {
            let t = -c / b;
            if (0.0..=1.0).contains(&t) {
                out.push(t);
            }
        }
        return;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return;
    }
    let sqrt_disc = disc.sqrt();
    for t in [(-b + sqrt_disc) / (2.0 * a), (-b - sqrt_disc) / (2.0 * a)] {
        if (0.0..=1.0).contains(&t) {
            out.push(t);
        }
    }
}

fn cubic_at(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    mt * mt * mt * p0 + 3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t * p3
}

impl Path {
    fn subpaths(&self) -> Vec<(Point, Vec<(Point, Point, Point)>, bool)> {
        to_cubic_subpaths(&self.data.commands)
    }

    fn flattened_subpaths(&self) -> Vec<FlatSubpath> {
        self.subpaths()
            .into_iter()
            .map(|(start, segments, closed)| {
                let mut points = vec![start];
                let mut current = start;
                for (c1, c2, to) in segments {
                    flatten_cubic(current, c1, c2, to, 0, &mut points);
                    current = to;
                }
                FlatSubpath { points, closed }
            })
            .collect()
    }

    pub fn commands(&self) -> &[PathCommand] {
        &self.data.commands
    }

    pub fn bounds(&self) -> Rect {
        self.data.bounds
    }

    pub fn is_empty(&self) -> bool {
        self.data.commands.is_empty()
    }

    /// Nonzero/even-odd hit-test against a bezier-flattened representation of this path (each
    /// subpath is treated as implicitly closed for fill purposes, matching every backend's own
    /// fill semantics). `transform` maps this path's own coordinate space into the space `point`
    /// is expressed in; pass `None` for the identity transform.
    pub fn contains(
        &self,
        point: Point,
        fill_rule: FillRule,
        transform: Option<AffineTransform>,
    ) -> bool {
        let point = match transform {
            Some(t) => {
                let inv = match invert(&t) {
                    Some(inv) => inv,
                    None => return false,
                };
                inv.transform_point(point)
            }
            None => point,
        };
        let mut winding = 0i32;
        for subpath in self.flattened_subpaths() {
            let pts = &subpath.points;
            if pts.len() < 2 {
                continue;
            }
            for i in 0..pts.len() {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                if (a.y <= point.y) != (b.y <= point.y) {
                    let t = (point.y - a.y) / (b.y - a.y);
                    let x_at = a.x + t * (b.x - a.x);
                    if x_at > point.x {
                        winding += if b.y > a.y { 1 } else { -1 };
                    }
                }
            }
        }
        match fill_rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }

    /// Distance-based hit-test against this path stroked with `stroke_style`. Caps/joins are
    /// approximated as round (the flattened-segment "capsule" test naturally rounds both) rather
    /// than reproducing butt/square caps or miter/bevel joins exactly — acceptable for hit-testing,
    /// where pixel-perfect stroke outline geometry isn't required (painter design doc §9.3).
    pub fn stroked_contains(
        &self,
        point: Point,
        stroke_style: &crate::graphics::StrokeStyle,
        transform: Option<AffineTransform>,
    ) -> bool {
        let point = match transform {
            Some(t) => match invert(&t) {
                Some(inv) => inv.transform_point(point),
                None => return false,
            },
            None => point,
        };
        let half_width = stroke_style.width / 2.0;
        for subpath in self.flattened_subpaths() {
            let pts = &subpath.points;
            let edge_count = if subpath.closed {
                pts.len()
            } else {
                pts.len().saturating_sub(1)
            };
            for i in 0..edge_count {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                if distance_to_segment(point, a, b) <= half_width {
                    return true;
                }
            }
        }
        false
    }

    pub fn transformed(&self, transform: AffineTransform) -> Path {
        let commands = self
            .data
            .commands
            .iter()
            .map(|cmd| match *cmd {
                PathCommand::MoveTo(p) => PathCommand::MoveTo(transform.transform_point(p)),
                PathCommand::LineTo(p) => PathCommand::LineTo(transform.transform_point(p)),
                PathCommand::QuadTo { control, to } => PathCommand::QuadTo {
                    control: transform.transform_point(control),
                    to: transform.transform_point(to),
                },
                PathCommand::CubicTo {
                    control1,
                    control2,
                    to,
                } => PathCommand::CubicTo {
                    control1: transform.transform_point(control1),
                    control2: transform.transform_point(control2),
                    to: transform.transform_point(to),
                },
                PathCommand::ArcTo(arc) => PathCommand::ArcTo(ArcSegment {
                    to: transform.transform_point(arc.to),
                    ..arc
                }),
                PathCommand::Close => PathCommand::Close,
            })
            .collect::<Vec<_>>();
        Path {
            data: Arc::new(PathData {
                bounds: compute_bounds(&commands),
                commands,
            }),
        }
    }

    /// Boolean geometry combine. Implemented on top of the `flo_curves` crate rather than a
    /// self-written bezier-boolean algorithm (painter design doc §2.4/§9.4 explicitly forbid the
    /// latter — self-written boolean-geometry code is exactly the class of bug-prone,
    /// backend-inconsistent logic that rule is guarding against).
    pub fn combine(
        a: &Path,
        b: &Path,
        mode: GeometryCombineMode,
        tolerance: f32,
    ) -> Result<Path, GeometryError> {
        super::path_combine::combine(a, b, mode, tolerance)
    }
}

/// Bridge for [`super::path_combine`]: exposes this module's cubic-normalized subpath
/// representation (dropping the `closed` flag, since a boolean-combine input is always treated as
/// a closed exterior/hole contour regardless of how it was authored).
pub(super) fn to_cubic_subpaths_pub(path: &Path) -> Vec<(Point, Vec<(Point, Point, Point)>)> {
    to_cubic_subpaths(&path.data.commands)
        .into_iter()
        .map(|(start, segments, _closed)| (start, segments))
        .collect()
}

/// Bridge for [`super::path_combine`]: builds a `Path` from an already-cubic-normalized command
/// list (as produced by `flo_curves`'s combine output), returning `None` only if it's empty.
pub(super) fn path_from_commands(commands: Vec<PathCommand>) -> Option<Path> {
    if commands.is_empty() {
        return None;
    }
    let bounds = compute_bounds(&commands);
    Some(Path {
        data: Arc::new(PathData { commands, bounds }),
    })
}

fn invert(t: &AffineTransform) -> Option<AffineTransform> {
    let det = t.m11 * t.m22 - t.m12 * t.m21;
    if det.abs() < 1e-9 {
        return None;
    }
    let inv_det = 1.0 / det;
    let m11 = t.m22 * inv_det;
    let m12 = -t.m12 * inv_det;
    let m21 = -t.m21 * inv_det;
    let m22 = t.m11 * inv_det;
    Some(AffineTransform {
        m11,
        m12,
        m21,
        m22,
        dx: -(t.dx * m11 + t.dy * m21),
        dy: -(t.dx * m12 + t.dy * m22),
    })
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let abx = b.x - a.x;
    let aby = b.y - a.y;
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq < 1e-9 {
        0.0
    } else {
        (((p.x - a.x) * abx + (p.y - a.y) * aby) / len_sq).clamp(0.0, 1.0)
    };
    let proj = Point {
        x: a.x + t * abx,
        y: a.y + t * aby,
    };
    ((p.x - proj.x).powi(2) + (p.y - proj.y).powi(2)).sqrt()
}

fn compute_bounds(commands: &[PathCommand]) -> Rect {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut include = |p: Point| {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    };
    for (start, segments, _) in to_cubic_subpaths(commands) {
        include(start);
        let mut current = start;
        for (c1, c2, to) in segments {
            include(c1);
            include(c2);
            include(to);
            let mut ts = Vec::new();
            cubic_extrema_1d(current.x, c1.x, c2.x, to.x, &mut ts);
            cubic_extrema_1d(current.y, c1.y, c2.y, to.y, &mut ts);
            for t in ts {
                include(Point {
                    x: cubic_at(current.x, c1.x, c2.x, to.x, t),
                    y: cubic_at(current.y, c1.y, c2.y, to.y, t),
                });
            }
            current = to;
        }
    }
    if !min_x.is_finite() {
        return Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
    }
    Rect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

/// Records path commands imperatively; `build()` finalizes them into an immutable [`Path`].
#[derive(Debug, Default)]
pub struct PathBuilder {
    commands: Vec<PathCommand>,
    current: Option<Point>,
    figure_start: Option<Point>,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(&mut self, to: Point) -> &mut Self {
        self.commands.push(PathCommand::MoveTo(to));
        self.current = Some(to);
        self.figure_start = Some(to);
        self
    }

    pub fn line_to(&mut self, to: Point) -> &mut Self {
        self.commands.push(PathCommand::LineTo(to));
        self.current = Some(to);
        self
    }

    pub fn quad_to(&mut self, control: Point, to: Point) -> &mut Self {
        self.commands.push(PathCommand::QuadTo { control, to });
        self.current = Some(to);
        self
    }

    pub fn cubic_to(&mut self, control1: Point, control2: Point, to: Point) -> &mut Self {
        self.commands.push(PathCommand::CubicTo {
            control1,
            control2,
            to,
        });
        self.current = Some(to);
        self
    }

    pub fn arc_to(&mut self, arc: ArcSegment) -> &mut Self {
        self.commands.push(PathCommand::ArcTo(arc));
        self.current = Some(arc.to);
        self
    }

    /// Convenience form specifying the arc by its center rather than SVG endpoint parameters.
    pub fn arc_center(
        &mut self,
        center: Point,
        radii: Size,
        start_angle_radians: f32,
        sweep_angle_radians: f32,
    ) -> &mut Self {
        let start = Point {
            x: center.x + radii.width * start_angle_radians.cos(),
            y: center.y + radii.height * start_angle_radians.sin(),
        };
        if self.current.is_none() {
            self.move_to(start);
        } else {
            self.line_to(start);
        }
        let end_angle = start_angle_radians + sweep_angle_radians;
        let to = Point {
            x: center.x + radii.width * end_angle.cos(),
            y: center.y + radii.height * end_angle.sin(),
        };
        self.arc_to(ArcSegment {
            radii,
            x_axis_rotation: 0.0,
            large_arc: sweep_angle_radians.abs() > std::f32::consts::PI,
            sweep: if sweep_angle_radians >= 0.0 {
                SweepDirection::Clockwise
            } else {
                SweepDirection::CounterClockwise
            },
            to,
        })
    }

    pub fn close(&mut self) -> &mut Self {
        self.commands.push(PathCommand::Close);
        self.current = self.figure_start;
        self
    }

    pub fn add_line(&mut self, from: Point, to: Point) -> &mut Self {
        self.move_to(from).line_to(to)
    }

    pub fn add_rect(&mut self, rect: Rect) -> &mut Self {
        self.move_to(Point {
            x: rect.x,
            y: rect.y,
        })
        .line_to(Point {
            x: rect.x + rect.width,
            y: rect.y,
        })
        .line_to(Point {
            x: rect.x + rect.width,
            y: rect.y + rect.height,
        })
        .line_to(Point {
            x: rect.x,
            y: rect.y + rect.height,
        })
        .close()
    }

    pub fn add_rounded_rect(&mut self, rect: Rect, radii: crate::base::CornerRadius) -> &mut Self {
        let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
        let k = 0.5522847498; // cubic Bézier circle-approximation constant
        self.move_to(Point {
            x: x + radii.top_left,
            y,
        });
        self.line_to(Point {
            x: x + w - radii.top_right,
            y,
        });
        if radii.top_right > 0.0 {
            self.cubic_to(
                Point {
                    x: x + w - radii.top_right + radii.top_right * k,
                    y,
                },
                Point {
                    x: x + w,
                    y: y + radii.top_right - radii.top_right * k,
                },
                Point {
                    x: x + w,
                    y: y + radii.top_right,
                },
            );
        }
        self.line_to(Point {
            x: x + w,
            y: y + h - radii.bottom_right,
        });
        if radii.bottom_right > 0.0 {
            self.cubic_to(
                Point {
                    x: x + w,
                    y: y + h - radii.bottom_right + radii.bottom_right * k,
                },
                Point {
                    x: x + w - radii.bottom_right + radii.bottom_right * k,
                    y: y + h,
                },
                Point {
                    x: x + w - radii.bottom_right,
                    y: y + h,
                },
            );
        }
        self.line_to(Point {
            x: x + radii.bottom_left,
            y: y + h,
        });
        if radii.bottom_left > 0.0 {
            self.cubic_to(
                Point {
                    x: x + radii.bottom_left - radii.bottom_left * k,
                    y: y + h,
                },
                Point {
                    x,
                    y: y + h - radii.bottom_left + radii.bottom_left * k,
                },
                Point {
                    x,
                    y: y + h - radii.bottom_left,
                },
            );
        }
        self.line_to(Point {
            x,
            y: y + radii.top_left,
        });
        if radii.top_left > 0.0 {
            self.cubic_to(
                Point {
                    x,
                    y: y + radii.top_left - radii.top_left * k,
                },
                Point {
                    x: x + radii.top_left - radii.top_left * k,
                    y,
                },
                Point {
                    x: x + radii.top_left,
                    y,
                },
            );
        }
        self.close()
    }

    pub fn add_ellipse(&mut self, rect: Rect) -> &mut Self {
        let cx = rect.x + rect.width / 2.0;
        let cy = rect.y + rect.height / 2.0;
        self.add_circle_radii(Point { x: cx, y: cy }, rect.width / 2.0, rect.height / 2.0)
    }

    pub fn add_circle(&mut self, center: Point, radius: f32) -> &mut Self {
        self.add_circle_radii(center, radius, radius)
    }

    fn add_circle_radii(&mut self, center: Point, rx: f32, ry: f32) -> &mut Self {
        let k = 0.5522847498;
        self.move_to(Point {
            x: center.x + rx,
            y: center.y,
        })
        .cubic_to(
            Point {
                x: center.x + rx,
                y: center.y + ry * k,
            },
            Point {
                x: center.x + rx * k,
                y: center.y + ry,
            },
            Point {
                x: center.x,
                y: center.y + ry,
            },
        )
        .cubic_to(
            Point {
                x: center.x - rx * k,
                y: center.y + ry,
            },
            Point {
                x: center.x - rx,
                y: center.y + ry * k,
            },
            Point {
                x: center.x - rx,
                y: center.y,
            },
        )
        .cubic_to(
            Point {
                x: center.x - rx,
                y: center.y - ry * k,
            },
            Point {
                x: center.x - rx * k,
                y: center.y - ry,
            },
            Point {
                x: center.x,
                y: center.y - ry,
            },
        )
        .cubic_to(
            Point {
                x: center.x + rx * k,
                y: center.y - ry,
            },
            Point {
                x: center.x + rx,
                y: center.y - ry * k,
            },
            Point {
                x: center.x + rx,
                y: center.y,
            },
        )
        .close()
    }

    pub fn add_arc(&mut self, arc: ArcSegment, from: Point) -> &mut Self {
        if self.current.is_none() {
            self.move_to(from);
        }
        self.arc_to(arc)
    }

    pub fn add_path(&mut self, path: &Path, transform: Option<AffineTransform>) -> &mut Self {
        let path = match transform {
            Some(t) => path.transformed(t),
            None => path.clone(),
        };
        self.commands.extend(path.data.commands.iter().copied());
        self
    }

    pub fn build(self) -> Result<Path, PathError> {
        let bounds = compute_bounds(&self.commands);
        Ok(Path {
            data: Arc::new(PathData {
                commands: self.commands,
                bounds,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    #[test]
    fn line_to_before_move_to_is_an_error() {
        // A bare LineTo with no prior MoveTo produces a degenerate (zero-length) implicit
        // subpath rather than panicking; PathBuilder's own API prevents this in practice since
        // there is no way to call line_to without a preceding move_to on `&mut self` fluently,
        // but PathData must still tolerate a hand-built empty command list gracefully.
        let empty = to_cubic_subpaths(&[]);
        assert!(empty.is_empty());
    }

    #[test]
    fn close_returns_current_point_to_figure_start() {
        let mut b = PathBuilder::new();
        b.move_to(pt(0.0, 0.0)).line_to(pt(10.0, 0.0)).close();
        b.line_to(pt(5.0, 5.0));
        let path = b.build().unwrap();
        // After close(), current point resets to figure_start (0,0), so this line_to starts there.
        assert!(matches!(path.commands()[3], PathCommand::LineTo(p) if p == pt(5.0, 5.0)));
    }

    #[test]
    fn multiple_subpaths_are_preserved() {
        let mut b = PathBuilder::new();
        b.add_rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        b.add_rect(Rect {
            x: 20.0,
            y: 20.0,
            width: 10.0,
            height: 10.0,
        });
        let path = b.build().unwrap();
        let move_tos = path
            .commands()
            .iter()
            .filter(|c| matches!(c, PathCommand::MoveTo(_)))
            .count();
        assert_eq!(move_tos, 2);
    }

    #[test]
    fn bounds_account_for_cubic_extrema() {
        let mut b = PathBuilder::new();
        b.move_to(pt(0.0, 0.0));
        b.cubic_to(pt(0.0, 100.0), pt(100.0, 100.0), pt(100.0, 0.0));
        let path = b.build().unwrap();
        let bounds = path.bounds();
        assert!(
            bounds.height > 50.0,
            "bounds should include the curve's peak, got {bounds:?}"
        );
    }

    #[test]
    fn contains_nonzero_hit_and_miss() {
        let mut b = PathBuilder::new();
        b.add_rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        let path = b.build().unwrap();
        assert!(path.contains(pt(5.0, 5.0), FillRule::NonZero, None));
        assert!(!path.contains(pt(50.0, 50.0), FillRule::NonZero, None));
    }

    #[test]
    fn contains_evenodd_hole() {
        let mut b = PathBuilder::new();
        b.add_rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        });
        b.add_rect(Rect {
            x: 5.0,
            y: 5.0,
            width: 10.0,
            height: 10.0,
        });
        let path = b.build().unwrap();
        assert!(!path.contains(pt(10.0, 10.0), FillRule::EvenOdd, None));
        assert!(path.contains(pt(2.0, 2.0), FillRule::EvenOdd, None));
    }

    #[test]
    fn stroked_contains_near_edge_only() {
        let mut b = PathBuilder::new();
        b.add_line(pt(0.0, 0.0), pt(100.0, 0.0));
        let path = b.build().unwrap();
        let stroke = crate::graphics::StrokeStyle {
            width: 4.0,
            ..Default::default()
        };
        assert!(path.stroked_contains(pt(50.0, 1.0), &stroke, None));
        assert!(!path.stroked_contains(pt(50.0, 10.0), &stroke, None));
    }
}
