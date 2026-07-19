//! Visual smoke test for `elwindui_core::painter`'s post-migration feature set (gradients,
//! dashed/capped/joined strokes, `Path`/`PathBuilder`, `Clip`/`AffineTransform`/opacity stacks,
//! `Image`) — none of these are reachable from `.elwind`/`view!` DSL syntax yet (`Rectangle`/
//! `Ellipse`'s `fill`/`stroke` only accept a solid-color hex literal), so this demo bypasses the
//! DSL for its actual drawing: `PainterDemoCanvas` is a hand-written self-drawing leaf element
//! (the same `#[elwindui::class]` pattern `elwindui_core::ui::Shape`/`TextBlock` themselves use),
//! whose `render()` calls the new `RenderContext` primitives directly. It's then embedded into an
//! ordinary `#[elwindui::component]` window by passing it in as a plain constructor argument and
//! bare-forwarding it to `Window`'s own `content` field — no `.elwind` file needed, per the same
//! inline style `examples/notepad-inline` uses.

// See `examples/notepad-inline/src/main.rs`'s own copy of this line for the full explanation
// (`crates/elwindui-macros/src/class.rs`'s `inherit_macro_self_ref_path` doc comment) — needed by
// any crate using `#[class]` (directly, as `PainterDemoCanvas` does here, or via
// `#[elwindui::component]`, as `DemoWindow` does) with a cross-crate `inherits` target.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::{AffineTransform, CornerRadius, Point, Rect};
use elwindui::core::painter::{
    AlphaMode, Brush, Clip, Color, FillRule, GradientStop, Image, ImageDrawOptions, ImageFit,
    ImageSampling, LineCap, LineJoin, LinearGradientBrush, PathBuilder, RadialGradientBrush,
    RenderContext, StrokeStyle,
};
use elwindui::core::ui::UIElementExt;
use elwindui::ui::WindowExt;
use std::rc::Rc;
use std::sync::Arc;

const CELL_W: f32 = 150.0;
const CELL_H: f32 = 140.0;
const GAP: f32 = 15.0;

fn cell_rect(col: i32, row: i32) -> Rect {
    Rect {
        x: GAP + col as f32 * (CELL_W + GAP),
        y: GAP + row as f32 * (CELL_H + GAP),
        width: CELL_W,
        height: CELL_H,
    }
}

/// Hand-written self-drawing leaf — same shape as `elwindui_core::ui::Shape`/`TextBlock`
/// (`#[elwindui_macros::class]`, `#[overrides] fn render`, `#[inherent] into_node`, a bare
/// `construct()`). Deliberately doesn't override `measure_override`/`arrange_override`: the base
/// `UIElement::measure`'s `constrain` helper already replaces the (otherwise zero) measured size
/// with an explicitly-set `width()`/`height()` (`ui.rs`'s `constrain`, `elem.width().unwrap_or
/// (size.width)`), and the base `arrange_override` (no children) already just echoes `final_size`
/// back — both defaults are exactly right for a fixed-size leaf, so `main()` just calls
/// `set_width`/`set_height` on the constructed instance instead of overriding either method.
#[elwindui::class(inherits = elwindui::core::ui::UIElement)]
pub struct PainterDemoCanvas {}

#[elwindui::class]
impl PainterDemoCanvas {
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        draw_linear_gradient(context, cell_rect(0, 0));
        draw_radial_gradient(context, cell_rect(1, 0));
        draw_dashed_stroke(context, cell_rect(2, 0));
        draw_star_path(context, cell_rect(3, 0));
        draw_clip_demo(context, cell_rect(0, 1));
        draw_transform_demo(context, cell_rect(1, 1));
        draw_opacity_demo(context, cell_rect(2, 1));
        draw_image_demo(context, cell_rect(3, 1));
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    fn construct() -> Self {
        Self {
            base: elwindui::core::ui::UIElement::default(),
        }
    }
}

fn draw_linear_gradient(context: &mut RenderContext<'_>, rect: Rect) {
    let brush = Brush::LinearGradient(
        LinearGradientBrush::new(
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 1.0 },
            vec![
                GradientStop::new(0.0, Color::rgb(59, 130, 246)).unwrap(),
                GradientStop::new(1.0, Color::rgb(168, 85, 247)).unwrap(),
            ],
        )
        .unwrap(),
    );
    context.fill_rounded_rect(rect, CornerRadius::uniform(14.0), &brush);
}

fn draw_radial_gradient(context: &mut RenderContext<'_>, rect: Rect) {
    let brush = Brush::RadialGradient(
        RadialGradientBrush::new(
            Point { x: 0.5, y: 0.5 },
            0.5,
            0.5,
            vec![
                GradientStop::new(0.0, Color::rgb(250, 204, 21)).unwrap(),
                GradientStop::new(1.0, Color::rgb(220, 38, 38)).unwrap(),
            ],
        )
        .unwrap(),
    );
    context.fill_ellipse(rect, &brush);
}

fn draw_dashed_stroke(context: &mut RenderContext<'_>, rect: Rect) {
    let stroke = StrokeStyle {
        width: 4.0,
        start_cap: LineCap::Round,
        end_cap: LineCap::Round,
        dash_cap: LineCap::Round,
        line_join: LineJoin::Round,
        dash_pattern: Arc::from([10.0, 6.0]),
        ..Default::default()
    };
    context.stroke_rounded_rect(
        rect,
        CornerRadius::uniform(14.0),
        &Brush::Solid(Color::rgb(16, 185, 129)),
        &stroke,
    );
}

fn draw_star_path(context: &mut RenderContext<'_>, rect: Rect) {
    let center = Point {
        x: rect.x + rect.width / 2.0,
        y: rect.y + rect.height / 2.0,
    };
    let (outer_r, inner_r) = (58.0, 24.0);
    let mut builder = PathBuilder::new();
    for i in 0..10 {
        let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let r = if i % 2 == 0 { outer_r } else { inner_r };
        let p = Point {
            x: center.x + r * angle.cos(),
            y: center.y + r * angle.sin(),
        };
        if i == 0 {
            builder.move_to(p);
        } else {
            builder.line_to(p);
        }
    }
    builder.close();
    let star = builder.build().expect("star path is never empty");
    context.fill_path(&star, &Brush::Solid(Color::rgb(251, 146, 60)), FillRule::NonZero);
    let stroke = StrokeStyle {
        width: 3.0,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    context.stroke_path(&star, &Brush::Solid(Color::rgb(124, 45, 18)), &stroke);
}

fn draw_clip_demo(context: &mut RenderContext<'_>, rect: Rect) {
    let clip_rect = Rect {
        x: rect.x + 20.0,
        y: rect.y + 15.0,
        width: rect.width - 40.0,
        height: rect.height - 30.0,
    };
    context.with_clip(
        Clip::RoundedRect {
            rect: clip_rect,
            radii: CornerRadius::uniform(10.0),
        },
        |ctx| {
            // A checkerboard deliberately larger than `clip_rect` — only the clipped region
            // should end up visible, verifying `PushClip`/`PopClip` actually bound the paint.
            let tile = 18.0;
            let mut row = 0;
            let mut y = rect.y;
            while y < rect.y + rect.height {
                let mut col = 0;
                let mut x = rect.x;
                while x < rect.x + rect.width {
                    let color = if (row + col) % 2 == 0 {
                        Color::rgb(99, 102, 241)
                    } else {
                        Color::rgb(224, 231, 255)
                    };
                    ctx.fill_rect(
                        Rect {
                            x,
                            y,
                            width: tile,
                            height: tile,
                        },
                        &Brush::Solid(color),
                    );
                    x += tile;
                    col += 1;
                }
                y += tile;
                row += 1;
            }
        },
    );
}

fn draw_transform_demo(context: &mut RenderContext<'_>, rect: Rect) {
    let center = Point {
        x: rect.x + rect.width / 2.0,
        y: rect.y + rect.height / 2.0,
    };
    let local_rect = Rect {
        x: -40.0,
        y: -25.0,
        width: 80.0,
        height: 50.0,
    };
    let rotate =
        AffineTransform::translation(center.x, center.y).concat(&AffineTransform::rotation(30f32.to_radians()));
    context.with_transform(rotate, |ctx| {
        ctx.fill_rounded_rect(local_rect, CornerRadius::uniform(8.0), &Brush::Solid(Color::rgb(236, 72, 153)));
    });
}

fn draw_opacity_demo(context: &mut RenderContext<'_>, rect: Rect) {
    let c1 = Point {
        x: rect.x + rect.width * 0.38,
        y: rect.y + rect.height * 0.55,
    };
    let c2 = Point {
        x: rect.x + rect.width * 0.62,
        y: rect.y + rect.height * 0.55,
    };
    context.with_opacity(0.55, |ctx| {
        ctx.fill_circle(c1, 42.0, &Brush::Solid(Color::rgb(239, 68, 68)));
    });
    context.with_opacity(0.55, |ctx| {
        ctx.fill_circle(c2, 42.0, &Brush::Solid(Color::rgb(37, 99, 235)));
    });
}

fn draw_image_demo(context: &mut RenderContext<'_>, rect: Rect) {
    let size: u32 = 8;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (r, g, b) = if (x + y) % 2 == 0 {
                (250u8, 204u8, 21u8)
            } else {
                (30u8, 41u8, 59u8)
            };
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let image = Image::from_rgba8(size, size, size * 4, pixels, AlphaMode::Opaque).expect("valid RGBA8 buffer");
    context.draw_image(
        &image,
        rect,
        None,
        ImageDrawOptions {
            sampling: ImageSampling::Nearest,
            fit: ImageFit::Fill,
            ..Default::default()
        },
    );
}

#[elwindui::component(inherits Window)]
struct DemoWindow {
    // Concrete `Rc<PainterDemoCanvas>`, not `Rc<dyn UIElementExt>` — codegen's `into_node_if_needed`
    // already handles "concrete element value forwarded into a `dyn UIElement`-typed target field"
    // uniformly (the same conversion any literal DSL child element goes through), so `content:
    // canvas` below still resolves correctly. Two independent reasons to prefer the concrete type
    // over a trait object here regardless: (1) `component_frontend.rs` re-serializes this struct's
    // fields as `.elwind`-DSL text — a bare `dyn UIElementExt` round-trips through that as the
    // single malformed identifier `dynUIElementExt` (the space between `dyn` and the path is lost);
    // (2) `codegen.rs`'s `is_copy_type` heuristic (no real type resolution, just the field's type
    // text) treats any bare, uppercase, generic-free identifier as one of this file's own `Copy`
    // enums — a `type AnyElement = Rc<dyn UIElementExt>` alias would silently mis-classify as
    // `Copy`-storable (`Cell`) even though `Rc<dyn Trait>` isn't `Copy` at all.
    canvas: std::rc::Rc<PainterDemoCanvas>,

    body: view! {
        title: "Painter Demo"
        width: 730.0
        height: 420.0
        content: canvas
    },
}

fn main() {
    let canvas = PainterDemoCanvas::new();
    canvas.set_width(GAP * 5.0 + CELL_W * 4.0);
    canvas.set_height(GAP * 3.0 + CELL_H * 2.0);
    let window = DemoWindow::new(canvas);
    window.show();
    elwindui::application::run();
}
