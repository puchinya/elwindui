//! [`RenderCommand`]'s own cheap identity for backend-side leaf diffing (the AppKit render
//! optimization work's Phase 3: reusing an existing `CALayer` in place when only a color/opacity/
//! path changed, instead of rebuilding a whole `RenderGroup`'s sublayers from scratch on any
//! change at all).
//!
//! Lives in core rather than a backend because a backend-local hash would have to touch every
//! field of every variant anyway, and would silently stop covering a field the moment someone
//! adds one without updating every backend's own copy. [`RenderCommand::kind`]'s `match` has no
//! `_` arm specifically so that adding a variant is a compile error at exactly the place that
//! needs updating, in every crate that matches on [`CommandKind`].

use super::brush::Brush;
use super::command::{Clip, RenderCommand};
use super::stroke::StrokeStyle;
use super::text::ComputedTextStyle;
use crate::base::{AffineTransform, CornerRadius, Point, Rect};
use std::hash::{Hash, Hasher};

/// Which [`RenderCommand`] variant this is, with no payload — the primary key a leaf-diffing
/// matcher probes on before ever looking at [`CommandFingerprint`]'s own hash fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    FillRect,
    StrokeRect,
    FillRoundedRect,
    StrokeRoundedRect,
    FillEllipse,
    StrokeEllipse,
    DrawLine,
    FillPath,
    StrokePath,
    DrawImage,
    DrawVectorImage,
    Text,
    PushClip,
    PopClip,
    PushTransform,
    PopTransform,
    PushOpacity,
    PopOpacity,
    NativeControl,
}

/// A cheap, deterministic *fast-reject* digest of one [`RenderCommand`]'s replay-relevant inputs
/// — not a correctness gate by itself. A backend should treat `fingerprint_a == fingerprint_b` as
/// "probably the same, worth confirming" and always follow up with
/// [`RenderCommand::visually_eq`] before actually reusing a cached representation; a 64-bit FNV-1a
/// collision that went unconfirmed would silently skip re-recording a genuinely changed command,
/// so `visually_eq` — not this type — is the actual safety net.
///
/// Split into `geometry` (shape/position/text-content — whatever would force a `CGPath`/frame
/// rebuild) and `paint` (brush/color/opacity — whatever a plain property setter can update in
/// place) so a backend can tell *which* half of a command changed without a second full pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandFingerprint {
    pub kind: CommandKind,
    pub geometry: u64,
    pub paint: u64,
}

/// A minimal FNV-1a accumulator. This crate has no hashing-crate dependency, and
/// `CommandFingerprint` only ever needs to be cheap and deterministic (see that type's own doc
/// comment on why it is not itself a correctness gate) — not cryptographically strong.
struct Fnv1a(u64);

impl Fnv1a {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write_u8(&mut self, byte: u8) {
        self.0 = (self.0 ^ byte as u64).wrapping_mul(Self::PRIME);
    }

    fn write_u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.write_u8(byte);
        }
    }

    fn write_f32(&mut self, value: f32) {
        // Exact-bits, not `to_bits()`'s IEEE-754 nuances (`-0.0` vs `0.0`, `NaN` payloads) — the
        // same convention `host::replay::GroupCacheKey`'s exact float `PartialEq` already relies
        // on in the AppKit backend. A spurious "changed" fingerprint from a bit-identical-but-
        // differently-signed zero costs one extra `visually_eq` call at worst, never a
        // correctness bug.
        self.write_u64(value.to_bits() as u64);
    }

    fn write_bool(&mut self, value: bool) {
        self.write_u8(value as u8);
    }
}

impl std::hash::Hasher for Fnv1a {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u8(*byte);
        }
    }
}

fn hash_point(h: &mut Fnv1a, p: Point) {
    h.write_f32(p.x);
    h.write_f32(p.y);
}

fn hash_rect(h: &mut Fnv1a, r: Rect) {
    h.write_f32(r.x);
    h.write_f32(r.y);
    h.write_f32(r.width);
    h.write_f32(r.height);
}

fn hash_corner_radius(h: &mut Fnv1a, r: CornerRadius) {
    h.write_f32(r.top_left);
    h.write_f32(r.top_right);
    h.write_f32(r.bottom_right);
    h.write_f32(r.bottom_left);
}

fn hash_transform(h: &mut Fnv1a, t: &AffineTransform) {
    h.write_f32(t.m11);
    h.write_f32(t.m12);
    h.write_f32(t.m21);
    h.write_f32(t.m22);
    h.write_f32(t.dx);
    h.write_f32(t.dy);
}

fn hash_stroke_geometry(h: &mut Fnv1a, s: &StrokeStyle) {
    h.write_f32(s.width);
    h.write_u8(s.start_cap as u8);
    h.write_u8(s.end_cap as u8);
    h.write_u8(s.dash_cap as u8);
    h.write_u8(s.line_join as u8);
    h.write_f32(s.miter_limit);
    h.write_u64(s.dash_pattern.len() as u64);
    for length in s.dash_pattern.iter() {
        h.write_f32(*length);
    }
    h.write_f32(s.dash_offset);
}

fn hash_clip(h: &mut Fnv1a, clip: &Clip) {
    match clip {
        Clip::Rect(rect) => {
            h.write_u8(0);
            hash_rect(h, *rect);
        }
        Clip::RoundedRect { rect, radii } => {
            h.write_u8(1);
            hash_rect(h, *rect);
            hash_corner_radius(h, *radii);
        }
        Clip::Path { path, rule } => {
            h.write_u8(2);
            hash_rect(h, path.bounds());
            h.write_u64(path.commands().len() as u64);
            h.write_u8(*rule as u8);
        }
    }
}

/// Hashes only the discriminant-independent identity of a brush's *paint* — its color(s) and
/// opacity — never an `Image` brush's actual pixel data (see [`brushes_visually_eq`]'s own doc
/// comment on why the corresponding equality check must not use `Brush`'s derived `PartialEq`
/// either).
fn hash_brush(h: &mut Fnv1a, brush: &Brush) {
    match brush {
        Brush::Solid(color) => {
            h.write_u8(0);
            h.write_u8(color.r);
            h.write_u8(color.g);
            h.write_u8(color.b);
            h.write_u8(color.a);
        }
        Brush::LinearGradient(g) => {
            h.write_u8(1);
            hash_point(h, g.start);
            hash_point(h, g.end);
            h.write_u64(g.stops.len() as u64);
            for stop in g.stops.iter() {
                h.write_f32(stop.offset);
                h.write_u8(stop.color.r);
                h.write_u8(stop.color.g);
                h.write_u8(stop.color.b);
                h.write_u8(stop.color.a);
            }
            h.write_u8(g.spread as u8);
            h.write_u8(g.mapping as u8);
            hash_transform(h, &g.transform);
            h.write_f32(g.opacity);
        }
        Brush::RadialGradient(g) => {
            h.write_u8(2);
            hash_point(h, g.center);
            hash_point(h, g.gradient_origin);
            h.write_f32(g.radius_x);
            h.write_f32(g.radius_y);
            h.write_u64(g.stops.len() as u64);
            for stop in g.stops.iter() {
                h.write_f32(stop.offset);
                h.write_u8(stop.color.r);
                h.write_u8(stop.color.g);
                h.write_u8(stop.color.b);
                h.write_u8(stop.color.a);
            }
            h.write_u8(g.spread as u8);
            h.write_u8(g.mapping as u8);
            hash_transform(h, &g.transform);
            h.write_f32(g.opacity);
        }
        Brush::Image(image_brush) => {
            h.write_u8(3);
            image_brush.image.id().hash(h);
            h.write_u8(image_brush.stretch as u8);
            h.write_u8(image_brush.alignment_x as u8);
            h.write_u8(image_brush.alignment_y as u8);
            h.write_u8(image_brush.tile_mode as u8);
            h.write_f32(image_brush.opacity);
            hash_transform(h, &image_brush.transform);
        }
    }
}

/// The `Brush`-comparison every `visually_eq` arm below must go through instead of `Brush`'s own
/// derived `PartialEq` — comparing two `Brush::Image` values through the derived impl would reach
/// `Image`'s own hand-written `PartialEq` (`graphics::image::Image`), a **deep pixel-buffer
/// comparison** that must never run on a per-frame leaf-diffing path. Every other `Brush` variant
/// is safe to compare structurally (`LinearGradientBrush`/`RadialGradientBrush`/their `Arc<[
/// GradientStop]>` stops all derive cheap `PartialEq`), so only the `Image` arm needs special
/// handling — swapping in `ImageId` (a `Copy + Eq` handle) for the image itself.
fn brushes_visually_eq(a: &Brush, b: &Brush) -> bool {
    match (a, b) {
        (Brush::Solid(x), Brush::Solid(y)) => x == y,
        (Brush::LinearGradient(x), Brush::LinearGradient(y)) => x == y,
        (Brush::RadialGradient(x), Brush::RadialGradient(y)) => x == y,
        (Brush::Image(x), Brush::Image(y)) => {
            x.image.id() == y.image.id()
                && x.source_rect == y.source_rect
                && x.stretch == y.stretch
                && x.alignment_x == y.alignment_x
                && x.alignment_y == y.alignment_y
                && x.tile_mode == y.tile_mode
                && x.opacity == y.opacity
                && x.transform == y.transform
        }
        _ => false,
    }
}

fn brushes_opt_visually_eq(a: Option<&Brush>, b: Option<&Brush>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => brushes_visually_eq(x, y),
        (None, None) => true,
        _ => false,
    }
}

/// The `ComputedTextStyle`-comparison `visually_eq`'s `Text` arm must go through instead of that
/// type's own derived `PartialEq` — it embeds a `foreground: Brush`, the same `Image`-brush
/// hazard `brushes_visually_eq` exists for.
fn text_styles_visually_eq(a: &ComputedTextStyle, b: &ComputedTextStyle) -> bool {
    a.font_family == b.font_family
        && a.font_size == b.font_size
        && a.font_weight == b.font_weight
        && a.font_style == b.font_style
        && a.font_stretch == b.font_stretch
        && a.character_spacing == b.character_spacing
        && brushes_visually_eq(&a.foreground, &b.foreground)
}

impl RenderCommand {
    /// Which variant this command is — see [`CommandKind`]'s own doc comment on why this `match`
    /// must never gain a `_` arm.
    pub fn kind(&self) -> CommandKind {
        match self {
            RenderCommand::FillRect { .. } => CommandKind::FillRect,
            RenderCommand::StrokeRect { .. } => CommandKind::StrokeRect,
            RenderCommand::FillRoundedRect { .. } => CommandKind::FillRoundedRect,
            RenderCommand::StrokeRoundedRect { .. } => CommandKind::StrokeRoundedRect,
            RenderCommand::FillEllipse { .. } => CommandKind::FillEllipse,
            RenderCommand::StrokeEllipse { .. } => CommandKind::StrokeEllipse,
            RenderCommand::DrawLine { .. } => CommandKind::DrawLine,
            RenderCommand::FillPath { .. } => CommandKind::FillPath,
            RenderCommand::StrokePath { .. } => CommandKind::StrokePath,
            RenderCommand::DrawImage { .. } => CommandKind::DrawImage,
            RenderCommand::DrawVectorImage { .. } => CommandKind::DrawVectorImage,
            RenderCommand::Text { .. } => CommandKind::Text,
            RenderCommand::PushClip { .. } => CommandKind::PushClip,
            RenderCommand::PopClip => CommandKind::PopClip,
            RenderCommand::PushTransform { .. } => CommandKind::PushTransform,
            RenderCommand::PopTransform => CommandKind::PopTransform,
            RenderCommand::PushOpacity { .. } => CommandKind::PushOpacity,
            RenderCommand::PopOpacity => CommandKind::PopOpacity,
            RenderCommand::NativeControl { .. } => CommandKind::NativeControl,
        }
    }

    /// A cheap fast-reject digest of this command's replay-relevant inputs — see
    /// [`CommandFingerprint`]'s own doc comment on why a match here must still be confirmed with
    /// [`Self::visually_eq`] before a backend actually reuses a cached representation.
    pub fn fingerprint(&self) -> CommandFingerprint {
        let kind = self.kind();
        let mut geometry = Fnv1a::new();
        let mut paint = Fnv1a::new();
        match self {
            RenderCommand::FillRect { rect, brush } => {
                hash_rect(&mut geometry, *rect);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::StrokeRect { rect, brush, stroke } => {
                hash_rect(&mut geometry, *rect);
                hash_stroke_geometry(&mut geometry, stroke);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::FillRoundedRect { rect, radii, brush } => {
                hash_rect(&mut geometry, *rect);
                hash_corner_radius(&mut geometry, *radii);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::StrokeRoundedRect {
                rect,
                radii,
                brush,
                stroke,
            } => {
                hash_rect(&mut geometry, *rect);
                hash_corner_radius(&mut geometry, *radii);
                hash_stroke_geometry(&mut geometry, stroke);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::FillEllipse { rect, brush } => {
                hash_rect(&mut geometry, *rect);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::StrokeEllipse { rect, brush, stroke } => {
                hash_rect(&mut geometry, *rect);
                hash_stroke_geometry(&mut geometry, stroke);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::DrawLine { from, to, brush, stroke } => {
                hash_point(&mut geometry, *from);
                hash_point(&mut geometry, *to);
                hash_stroke_geometry(&mut geometry, stroke);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::FillPath { path, brush, rule } => {
                hash_rect(&mut geometry, path.bounds());
                geometry.write_u64(path.commands().len() as u64);
                geometry.write_u8(*rule as u8);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::StrokePath { path, brush, stroke } => {
                hash_rect(&mut geometry, path.bounds());
                geometry.write_u64(path.commands().len() as u64);
                hash_stroke_geometry(&mut geometry, stroke);
                hash_brush(&mut paint, brush);
            }
            RenderCommand::DrawImage {
                image,
                dest,
                source,
                options,
            } => {
                image.id().hash(&mut geometry);
                hash_rect(&mut geometry, *dest);
                if let Some(source) = source {
                    geometry.write_bool(true);
                    hash_rect(&mut geometry, *source);
                } else {
                    geometry.write_bool(false);
                }
                paint.write_u8(options.fit as u8);
                paint.write_u8(options.alignment_x as u8);
                paint.write_u8(options.alignment_y as u8);
                paint.write_u8(options.repeat as u8);
                paint.write_f32(options.opacity);
            }
            RenderCommand::DrawVectorImage {
                image,
                dest,
                source,
                options,
            } => {
                image.id().hash(&mut geometry);
                hash_rect(&mut geometry, *dest);
                if let Some(source) = source {
                    geometry.write_bool(true);
                    hash_rect(&mut geometry, *source);
                } else {
                    geometry.write_bool(false);
                }
                paint.write_f32(options.opacity);
            }
            RenderCommand::Text {
                content,
                rect,
                style,
                foreground,
                alignment,
            } => {
                geometry.write_u64(content.len() as u64);
                for byte in content.as_bytes() {
                    geometry.write_u8(*byte);
                }
                hash_rect(&mut geometry, *rect);
                geometry.write_f32(style.font_size);
                geometry.write_u8(*alignment as u8);
                if let Some(foreground) = foreground {
                    paint.write_bool(true);
                    hash_brush(&mut paint, foreground);
                } else {
                    paint.write_bool(false);
                }
                hash_brush(&mut paint, &style.foreground);
            }
            RenderCommand::PushClip { clip } => hash_clip(&mut geometry, clip),
            RenderCommand::PopClip => {}
            RenderCommand::PushTransform { transform } => hash_transform(&mut geometry, transform),
            RenderCommand::PopTransform => {}
            RenderCommand::PushOpacity { opacity } => paint.write_f32(*opacity),
            RenderCommand::PopOpacity => {}
            RenderCommand::NativeControl {
                owner_id,
                handle,
                rect,
            } => {
                geometry.write_u64(*owner_id);
                hash_rect(&mut geometry, *rect);
                // Pointer identity is one *input* to this fingerprint, not the fingerprint's sole
                // basis and never `visually_eq`'s sole basis either (see that method's own
                // `NativeControl` arm, which compares `Rc::ptr_eq` directly) — consistent with the
                // AppKit render optimization guide's warning against relying on unstable pointer
                // identity alone.
                geometry.write_u64(std::rc::Rc::as_ptr(handle) as *const () as u64);
            }
        }
        CommandFingerprint {
            kind,
            geometry: geometry.finish(),
            paint: paint.finish(),
        }
    }

    /// Exact, cheap equality — the confirmation step [`Self::fingerprint`]'s own doc comment
    /// describes. Deliberately never routes a `Brush`/`ComputedTextStyle` comparison through
    /// their own derived `PartialEq` (see [`brushes_visually_eq`]'s own doc comment): both embed
    /// an `Image`, whose hand-written `PartialEq` is a deep pixel-buffer comparison that must
    /// never reach a per-frame path. `Image`/`VectorImage` are otherwise compared by their stable
    /// `ImageId`/`VectorImageId`, and `NativeControl`'s type-erased `handle` by `Rc::ptr_eq`
    /// (identity is exactly what matters there — the same handle rendered again).
    pub fn visually_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                RenderCommand::FillRect { rect: r1, brush: b1 },
                RenderCommand::FillRect { rect: r2, brush: b2 },
            ) => r1 == r2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::StrokeRect {
                    rect: r1,
                    brush: b1,
                    stroke: s1,
                },
                RenderCommand::StrokeRect {
                    rect: r2,
                    brush: b2,
                    stroke: s2,
                },
            ) => r1 == r2 && s1 == s2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::FillRoundedRect {
                    rect: r1,
                    radii: rad1,
                    brush: b1,
                },
                RenderCommand::FillRoundedRect {
                    rect: r2,
                    radii: rad2,
                    brush: b2,
                },
            ) => r1 == r2 && rad1 == rad2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::StrokeRoundedRect {
                    rect: r1,
                    radii: rad1,
                    brush: b1,
                    stroke: s1,
                },
                RenderCommand::StrokeRoundedRect {
                    rect: r2,
                    radii: rad2,
                    brush: b2,
                    stroke: s2,
                },
            ) => r1 == r2 && rad1 == rad2 && s1 == s2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::FillEllipse { rect: r1, brush: b1 },
                RenderCommand::FillEllipse { rect: r2, brush: b2 },
            ) => r1 == r2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::StrokeEllipse {
                    rect: r1,
                    brush: b1,
                    stroke: s1,
                },
                RenderCommand::StrokeEllipse {
                    rect: r2,
                    brush: b2,
                    stroke: s2,
                },
            ) => r1 == r2 && s1 == s2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::DrawLine {
                    from: f1,
                    to: t1,
                    brush: b1,
                    stroke: s1,
                },
                RenderCommand::DrawLine {
                    from: f2,
                    to: t2,
                    brush: b2,
                    stroke: s2,
                },
            ) => f1 == f2 && t1 == t2 && s1 == s2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::FillPath {
                    path: p1,
                    brush: b1,
                    rule: ru1,
                },
                RenderCommand::FillPath {
                    path: p2,
                    brush: b2,
                    rule: ru2,
                },
            ) => p1 == p2 && ru1 == ru2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::StrokePath {
                    path: p1,
                    brush: b1,
                    stroke: s1,
                },
                RenderCommand::StrokePath {
                    path: p2,
                    brush: b2,
                    stroke: s2,
                },
            ) => p1 == p2 && s1 == s2 && brushes_visually_eq(b1, b2),
            (
                RenderCommand::DrawImage {
                    image: i1,
                    dest: d1,
                    source: so1,
                    options: op1,
                },
                RenderCommand::DrawImage {
                    image: i2,
                    dest: d2,
                    source: so2,
                    options: op2,
                },
            ) => i1.id() == i2.id() && d1 == d2 && so1 == so2 && op1 == op2,
            (
                RenderCommand::DrawVectorImage {
                    image: i1,
                    dest: d1,
                    source: so1,
                    options: op1,
                },
                RenderCommand::DrawVectorImage {
                    image: i2,
                    dest: d2,
                    source: so2,
                    options: op2,
                },
            ) => i1 == i2 && d1 == d2 && so1 == so2 && op1 == op2,
            (
                RenderCommand::Text {
                    content: c1,
                    rect: r1,
                    style: st1,
                    foreground: fg1,
                    alignment: a1,
                },
                RenderCommand::Text {
                    content: c2,
                    rect: r2,
                    style: st2,
                    foreground: fg2,
                    alignment: a2,
                },
            ) => {
                c1 == c2
                    && r1 == r2
                    && a1 == a2
                    && text_styles_visually_eq(st1, st2)
                    && brushes_opt_visually_eq(fg1.as_ref(), fg2.as_ref())
            }
            (RenderCommand::PushClip { clip: c1 }, RenderCommand::PushClip { clip: c2 }) => c1 == c2,
            (RenderCommand::PopClip, RenderCommand::PopClip) => true,
            (
                RenderCommand::PushTransform { transform: t1 },
                RenderCommand::PushTransform { transform: t2 },
            ) => t1 == t2,
            (RenderCommand::PopTransform, RenderCommand::PopTransform) => true,
            (
                RenderCommand::PushOpacity { opacity: o1 },
                RenderCommand::PushOpacity { opacity: o2 },
            ) => o1 == o2,
            (RenderCommand::PopOpacity, RenderCommand::PopOpacity) => true,
            (
                RenderCommand::NativeControl {
                    owner_id: id1,
                    handle: h1,
                    rect: r1,
                },
                RenderCommand::NativeControl {
                    owner_id: id2,
                    handle: h2,
                    rect: r2,
                },
            ) => id1 == id2 && r1 == r2 && std::rc::Rc::ptr_eq(h1, h2),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{AlphaMode, Color, Image};

    fn solid_fill_rect(x: f32, color: Color) -> RenderCommand {
        RenderCommand::FillRect {
            rect: Rect {
                x,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            brush: Brush::Solid(color),
        }
    }

    #[test]
    fn kind_maps_every_variant_distinctly() {
        assert_eq!(
            solid_fill_rect(0.0, Color::black()).kind(),
            CommandKind::FillRect
        );
        assert_eq!(RenderCommand::PopClip.kind(), CommandKind::PopClip);
        assert_eq!(RenderCommand::PopTransform.kind(), CommandKind::PopTransform);
        assert_eq!(RenderCommand::PopOpacity.kind(), CommandKind::PopOpacity);
    }

    #[test]
    fn identical_commands_fingerprint_equal_and_are_visually_eq() {
        let a = solid_fill_rect(5.0, Color { r: 10, g: 20, b: 30, a: 255 });
        let b = solid_fill_rect(5.0, Color { r: 10, g: 20, b: 30, a: 255 });
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(a.visually_eq(&b));
    }

    #[test]
    fn a_changed_rect_changes_the_geometry_fingerprint_and_fails_visually_eq() {
        let a = solid_fill_rect(5.0, Color::black());
        let b = solid_fill_rect(6.0, Color::black());
        assert_ne!(a.fingerprint().geometry, b.fingerprint().geometry);
        assert_eq!(
            a.fingerprint().paint,
            b.fingerprint().paint,
            "only the rect changed — the paint half of the fingerprint should be unaffected"
        );
        assert!(!a.visually_eq(&b));
    }

    #[test]
    fn a_changed_color_changes_only_the_paint_fingerprint() {
        let a = solid_fill_rect(5.0, Color { r: 0, g: 0, b: 0, a: 255 });
        let b = solid_fill_rect(5.0, Color { r: 255, g: 0, b: 0, a: 255 });
        assert_eq!(a.fingerprint().geometry, b.fingerprint().geometry);
        assert_ne!(a.fingerprint().paint, b.fingerprint().paint);
        assert!(!a.visually_eq(&b));
    }

    #[test]
    fn different_kinds_are_never_visually_eq_even_with_matching_geometry() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let fill = RenderCommand::FillRect {
            rect,
            brush: Brush::Solid(Color::black()),
        };
        let stroke = RenderCommand::StrokeRect {
            rect,
            brush: Brush::Solid(Color::black()),
            stroke: StrokeStyle::default(),
        };
        assert_ne!(fill.kind(), stroke.kind());
        assert!(!fill.visually_eq(&stroke));
    }

    #[test]
    fn cloned_image_shares_its_id_so_a_moved_but_otherwise_identical_image_brush_is_visually_eq() {
        // `Image` is `Arc`-backed — cloning it (as `RenderCommand::visually_eq`'s `DrawImage` arm
        // is specifically designed to compare via `ImageId` rather than `Image`'s own deep-pixel
        // `PartialEq`) must keep the same id. This is the property the AppKit render optimization
        // work's leaf diffing actually depends on: two `RenderCommand`s built from the same
        // decoded `Image` resource, even if not the literal same `Image` value, must compare
        // cheaply equal.
        let image = Image::from_rgba8(1, 1, 4, vec![0u8, 0, 0, 255], AlphaMode::Straight).unwrap();
        let cloned = image.clone();
        assert_eq!(image.id(), cloned.id());

        let a = RenderCommand::DrawImage {
            image: image.clone(),
            dest: Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            source: None,
            options: Default::default(),
        };
        let b = RenderCommand::DrawImage {
            image: cloned,
            dest: Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            source: None,
            options: Default::default(),
        };
        assert!(
            a.visually_eq(&b),
            "comparing by ImageId must not fall through to Image's own deep pixel PartialEq"
        );
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn native_control_visually_eq_is_pointer_identity_not_content_equality() {
        use std::any::Any;
        use std::rc::Rc;

        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let handle_a: Rc<dyn Any> = Rc::new(42u32);
        let handle_b: Rc<dyn Any> = Rc::new(42u32);
        let same_a = Rc::clone(&handle_a);

        let a = RenderCommand::NativeControl {
            owner_id: 1,
            handle: handle_a,
            rect,
        };
        let a_again = RenderCommand::NativeControl {
            owner_id: 1,
            handle: same_a,
            rect,
        };
        let b = RenderCommand::NativeControl {
            owner_id: 1,
            handle: handle_b,
            rect,
        };
        assert!(a.visually_eq(&a_again), "the same Rc must compare equal");
        assert!(
            !a.visually_eq(&b),
            "two distinct Rcs with identical inner content must not compare equal — identity, \
             not content, is what a native control's own replay depends on"
        );
    }
}
