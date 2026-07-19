//! Visual verification harness for `elwindui_core::graphics` (gradients, dashed/capped/joined
//! strokes, `Path`/`PathBuilder`, boolean path combine, `Clip`/`AffineTransform`/opacity stacks,
//! `Image`) — none of these are reachable from `.elwind`/`view!` DSL syntax yet (`Rectangle`/
//! `Ellipse`'s `fill`/`stroke` only accept a solid-color hex literal), so this demo bypasses the
//! DSL for its actual drawing: `GraphicsDemoCanvas` is a hand-written self-drawing leaf element
//! (the same `#[elwindui::class]` pattern `elwindui_core::ui::Shape`/`TextBlock` themselves use),
//! whose `render()` calls the new `RenderContext` primitives directly.
//!
//! Grouped into a `TabView` — one tab per `graphics` submodule area (fills, strokes, paths,
//! path-boolean-combine, compositing/images) — so each area gets its own screen's worth of room.
//! Each tab lays its demos out in a single labeled-card row (the same idea the pre-TabView version
//! of this file used for its whole 4x2 grid, just scoped to one tab's cells at a time). This is
//! the standing tool to re-run and screenshot (see CLAUDE.md's screenshot recipe) whenever
//! `elwindui_core::graphics` changes — extend the relevant tab's `const` table, or add a new tab
//! if a whole new submodule area shows up, keeping the tab count comfortably under ten.
//!
//! Two `graphics` features are deliberately **not** demoed here: `Brush::Image` as a fill (AppKit's
//! `apply_fill` treats it as a no-op — see that function's own doc comment) and
//! `PathBuilder::arc_to`/`arc_center` (AppKit's `path_to_cgpath` skips raw `PathCommand::ArcTo`
//! entirely — see that function's own doc comment). Both are real, already-documented gaps in the
//! AppKit backend, not oversights here; a demo cell for either would just render blank.

// See `examples/notepad-inline/src/main.rs`'s own copy of this line for the full explanation
// (`crates/elwindui-macros/src/class.rs`'s `inherit_macro_self_ref_path` doc comment) — needed by
// any crate using `#[class]` (directly, as `GraphicsDemoCanvas` does here, or via
// `#[elwindui::component]`, as `GraphicsDemoWindow` does) with a cross-crate `inherits` target.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::base::{AffineTransform, CornerRadius, Point, Rect};
use elwindui::core::graphics::{
    AlphaMode, Brush, Clip, Color, FillRule, GeometryCombineMode, GradientStop, Image,
    ImageDrawOptions, ImageFit, ImageSampling, LineCap, LineJoin, LinearGradientBrush, Path,
    PathBuilder, RadialGradientBrush, RenderContext, StrokeStyle, TextAlignment,
};
use elwindui::core::ui::UIElementExt;
use elwindui::ui::WindowExt;
use std::rc::Rc;
use std::sync::Arc;

const GAP: f32 = 16.0;
const LABEL_HEIGHT: f32 = 20.0;
const CARD_BACKGROUND: Color = Color::rgb(246, 246, 248);
const LABEL_COLOR: Color = Color::rgb(51, 51, 51);

struct DemoEntry {
    label: &'static str,
    draw: fn(&mut RenderContext<'_>, Rect),
}

const FILLS: &[DemoEntry] = &[
    DemoEntry { label: "Linear Gradient", draw: draw_linear_gradient },
    DemoEntry { label: "Radial Gradient", draw: draw_radial_gradient },
];

const STROKES: &[DemoEntry] = &[
    DemoEntry { label: "Dashed Stroke", draw: draw_dashed_stroke },
    DemoEntry { label: "Line Caps", draw: draw_line_caps },
    DemoEntry { label: "Line Joins", draw: draw_line_joins },
];

const PATHS: &[DemoEntry] = &[
    DemoEntry { label: "Star Path", draw: draw_star_path },
    DemoEntry { label: "Fill Rule (Even-Odd)", draw: draw_fill_rule },
    DemoEntry { label: "Bezier Curve", draw: draw_bezier_curve },
];

const PATH_COMBINE: &[DemoEntry] = &[
    DemoEntry { label: "Union", draw: draw_combine_union },
    DemoEntry { label: "Intersect", draw: draw_combine_intersect },
    DemoEntry { label: "Xor", draw: draw_combine_xor },
    DemoEntry { label: "Exclude", draw: draw_combine_exclude },
];

const COMPOSITING_AND_IMAGES: &[DemoEntry] = &[
    DemoEntry { label: "Clip", draw: draw_clip_demo },
    DemoEntry { label: "Transform", draw: draw_transform_demo },
    DemoEntry { label: "Opacity", draw: draw_opacity_demo },
    DemoEntry { label: "Image", draw: draw_image_demo },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphicsDemoCategory {
    Fills,
    Strokes,
    Paths,
    PathCombine,
    CompositingAndImages,
}

impl GraphicsDemoCategory {
    fn entries(self) -> &'static [DemoEntry] {
        match self {
            Self::Fills => FILLS,
            Self::Strokes => STROKES,
            Self::Paths => PATHS,
            Self::PathCombine => PATH_COMBINE,
            Self::CompositingAndImages => COMPOSITING_AND_IMAGES,
        }
    }
}

/// Hand-written self-drawing leaf — same shape as `elwindui_core::ui::Shape`/`TextBlock`
/// (`#[elwindui_macros::class]`, `#[overrides] fn render`, `#[inherent] into_node`, a bare
/// `construct()`). Deliberately doesn't override `measure_override`/`arrange_override`: the base
/// `UIElement`'s default `HorizontalAlignment`/`VerticalAlignment` is `Stretch`, so this element's
/// `arranged_width()`/`arranged_height()` (read at render time below) already fill whatever slot
/// its `TabView` tab hands it — no explicit `set_width`/`set_height` needed.
#[elwindui::class(inherits = elwindui::core::ui::UIElement)]
pub struct GraphicsDemoCanvas {
    category: GraphicsDemoCategory,
}

#[elwindui::class]
impl GraphicsDemoCanvas {
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let entries = self.category.entries();
        let n = entries.len().max(1) as f32;
        let width = self.arranged_width().unwrap_or(0.0);
        let height = self.arranged_height().unwrap_or(0.0);
        let cell_w = ((width - GAP * (n + 1.0)) / n).max(0.0);
        let cell_h = (height - GAP * 2.0).max(0.0);
        for (i, entry) in entries.iter().enumerate() {
            let card = Rect {
                x: GAP + i as f32 * (cell_w + GAP),
                y: GAP,
                width: cell_w,
                height: cell_h,
            };
            context.fill_rounded_rect(card, CornerRadius::uniform(10.0), &Brush::Solid(CARD_BACKGROUND));
            let label_rect = Rect {
                x: card.x,
                y: card.y + 6.0,
                width: card.width,
                height: LABEL_HEIGHT,
            };
            context.draw_text(entry.label, label_rect, Some(LABEL_COLOR), TextAlignment::Center);
            let demo_rect = Rect {
                x: card.x + 8.0,
                y: card.y + LABEL_HEIGHT + 10.0,
                width: (card.width - 16.0).max(0.0),
                height: (card.height - LABEL_HEIGHT - 18.0).max(0.0),
            };
            (entry.draw)(context, demo_rect);
        }
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    fn construct(category: GraphicsDemoCategory) -> Self {
        Self {
            base: elwindui::core::ui::UIElement::default(),
            category,
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

/// `CAShapeLayer.lineCap`(`apply_stroke`が読むのは`style.end_cap`のみ)は3本まとめて1つの値しか
/// 持てないため、`Butt`/`Round`/`Square`を1本ずつ別々の`draw_line`呼び出しで比較する。
fn draw_line_caps(context: &mut RenderContext<'_>, rect: Rect) {
    let caps = [LineCap::Butt, LineCap::Round, LineCap::Square];
    let row_h = rect.height / caps.len() as f32;
    let brush = Brush::Solid(Color::rgb(37, 99, 235));
    for (i, cap) in caps.iter().enumerate() {
        let y = rect.y + row_h * (i as f32 + 0.5);
        let stroke = StrokeStyle {
            width: 14.0,
            start_cap: *cap,
            end_cap: *cap,
            dash_cap: *cap,
            ..Default::default()
        };
        context.draw_line(
            Point { x: rect.x + 14.0, y },
            Point { x: rect.x + rect.width - 14.0, y },
            &brush,
            &stroke,
        );
    }
}

fn draw_line_joins(context: &mut RenderContext<'_>, rect: Rect) {
    let joins = [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel];
    let col_w = rect.width / joins.len() as f32;
    let brush = Brush::Solid(Color::rgb(22, 163, 74));
    for (i, join) in joins.iter().enumerate() {
        let cx = rect.x + col_w * (i as f32 + 0.5);
        let top = rect.y + rect.height * 0.15;
        let bottom = rect.y + rect.height * 0.85;
        let half_w = col_w * 0.3;
        let mut builder = PathBuilder::new();
        builder.move_to(Point { x: cx - half_w, y: bottom });
        builder.line_to(Point { x: cx, y: top });
        builder.line_to(Point { x: cx + half_w, y: bottom });
        let path = builder.build().expect("polyline path is never empty");
        let stroke = StrokeStyle {
            width: 10.0,
            line_join: *join,
            ..Default::default()
        };
        context.stroke_path(&path, &brush, &stroke);
    }
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

/// Two same-winding overlapping rects as *one* path: `NonZero` would fill the overlap solid (both
/// contribute the same winding sign), but `EvenOdd` only cares about crossing parity, so the
/// overlap comes out as a hole — the visible contrast is the point of this demo.
fn draw_fill_rule(context: &mut RenderContext<'_>, rect: Rect) {
    let w = rect.width * 0.55;
    let h = rect.height * 0.55;
    let mut builder = PathBuilder::new();
    builder.add_rect(Rect {
        x: rect.x + rect.width * 0.08,
        y: rect.y + rect.height * 0.2,
        width: w,
        height: h,
    });
    builder.add_rect(Rect {
        x: rect.x + rect.width * 0.37,
        y: rect.y + rect.height * 0.35,
        width: w,
        height: h,
    });
    let path = builder.build().expect("rects path is never empty");
    context.fill_path(&path, &Brush::Solid(Color::rgb(234, 88, 12)), FillRule::EvenOdd);
}

fn draw_bezier_curve(context: &mut RenderContext<'_>, rect: Rect) {
    let mut builder = PathBuilder::new();
    builder.move_to(Point {
        x: rect.x + rect.width * 0.08,
        y: rect.y + rect.height * 0.8,
    });
    builder.quad_to(
        Point {
            x: rect.x + rect.width * 0.3,
            y: rect.y + rect.height * 0.05,
        },
        Point {
            x: rect.x + rect.width * 0.5,
            y: rect.y + rect.height * 0.5,
        },
    );
    builder.cubic_to(
        Point {
            x: rect.x + rect.width * 0.65,
            y: rect.y + rect.height * 0.95,
        },
        Point {
            x: rect.x + rect.width * 0.8,
            y: rect.y + rect.height * 0.05,
        },
        Point {
            x: rect.x + rect.width * 0.92,
            y: rect.y + rect.height * 0.8,
        },
    );
    let path = builder.build().expect("curve path is never empty");
    let stroke = StrokeStyle {
        width: 3.0,
        line_join: LineJoin::Round,
        start_cap: LineCap::Round,
        end_cap: LineCap::Round,
        dash_cap: LineCap::Round,
        ..Default::default()
    };
    context.stroke_path(&path, &Brush::Solid(Color::rgb(219, 39, 119)), &stroke);
}

/// Shared by the four `Path Combine` cells below — builds the same two overlapping circles and
/// combines them with `mode`, the only thing that differs between `Union`/`Intersect`/`Xor`/
/// `Exclude`.
fn draw_path_combine(context: &mut RenderContext<'_>, rect: Rect, mode: GeometryCombineMode) {
    let r = rect.width.min(rect.height) * 0.3;
    // A not-yet-laid-out (or momentarily hidden) `TabView` tab can render with `arranged_width()`/
    // `arranged_height()` still at their `unwrap_or(0.0)` fallback, collapsing `r` to zero — two
    // zero-radius circles are a degenerate `Path::combine` input `flo_curves` rejects outright.
    // Skip the combine entirely rather than let that turn into a `.expect()` panic on real layout
    // timing, not a geometry bug.
    if r < 1.0 {
        return;
    }
    let cx = rect.x + rect.width / 2.0;
    let cy = rect.y + rect.height / 2.0;
    let offset = r * 0.55;
    let mut a_builder = PathBuilder::new();
    a_builder.add_circle(Point { x: cx - offset, y: cy }, r);
    let a = a_builder.build().expect("circle path is never empty");
    let mut b_builder = PathBuilder::new();
    b_builder.add_circle(Point { x: cx + offset, y: cy }, r);
    let b = b_builder.build().expect("circle path is never empty");
    let Ok(combined) = Path::combine(&a, &b, mode, 0.5) else {
        return;
    };
    context.fill_path(&combined, &Brush::Solid(Color::rgb(56, 189, 248)), FillRule::NonZero);
    let stroke = StrokeStyle {
        width: 2.0,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    context.stroke_path(&combined, &Brush::Solid(Color::rgb(15, 118, 110)), &stroke);
}

fn draw_combine_union(context: &mut RenderContext<'_>, rect: Rect) {
    draw_path_combine(context, rect, GeometryCombineMode::Union);
}

fn draw_combine_intersect(context: &mut RenderContext<'_>, rect: Rect) {
    draw_path_combine(context, rect, GeometryCombineMode::Intersect);
}

fn draw_combine_xor(context: &mut RenderContext<'_>, rect: Rect) {
    draw_path_combine(context, rect, GeometryCombineMode::Xor);
}

fn draw_combine_exclude(context: &mut RenderContext<'_>, rect: Rect) {
    draw_path_combine(context, rect, GeometryCombineMode::Exclude);
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

// `TabView`'s chip click handler (`elwindui-backend-appkit`'s `native_ui::TabView::rebuild`) only
// ever fires the DSL `on_select` callback — it never updates `selected_index` on its own (that
// `usize` observable is one-directional, view -> `on_select` -> model -> back down through
// `bind!`). A static `selected_index: 0` plus a no-op `on_select` therefore never actually
// switches tabs; this tiny viewmodel exists solely to round-trip that click back into
// `selected_index`, the same way `examples/notepad`'s real `active_tab`/`select_tab` does.
#[elwindui::viewmodel]
mod graphics_demo_view_model {
    struct GraphicsDemoViewModel {
        #[observable(default = 0usize)]
        selected_tab: usize,
    }

    impl GraphicsDemoViewModel {
        fn select_tab(&self, index: usize) {
            selected_tab = index;
        }
    }
}

#[elwindui::component(inherits Window)]
struct GraphicsDemoWindow {
    #[bindable]
    vm: std::rc::Rc<GraphicsDemoViewModel>,

    // Concrete `Rc<GraphicsDemoCanvas>`, not `Rc<dyn UIElementExt>` — codegen's `into_node_if_needed`
    // already handles "concrete element value forwarded into a `dyn UIElement`-typed target field"
    // uniformly (the same conversion any literal DSL child element goes through), so `content:
    // fills_canvas` etc. below still resolve correctly. Two independent reasons to prefer the
    // concrete type over a trait object here regardless: (1) `component_frontend.rs` re-serializes
    // this struct's fields as `.elwind`-DSL text — a bare `dyn UIElementExt` round-trips through
    // that as the single malformed identifier `dynUIElementExt` (the space between `dyn` and the
    // path is lost); (2) `codegen.rs`'s `is_copy_type` heuristic (no real type resolution, just the
    // field's type text) treats any bare, uppercase, generic-free identifier as one of this file's
    // own `Copy` enums — a `type AnyElement = Rc<dyn UIElementExt>` alias would silently
    // mis-classify as `Copy`-storable (`Cell`) even though `Rc<dyn Trait>` isn't `Copy` at all.
    fills_canvas: std::rc::Rc<GraphicsDemoCanvas>,
    strokes_canvas: std::rc::Rc<GraphicsDemoCanvas>,
    paths_canvas: std::rc::Rc<GraphicsDemoCanvas>,
    path_combine_canvas: std::rc::Rc<GraphicsDemoCanvas>,
    compositing_and_images_canvas: std::rc::Rc<GraphicsDemoCanvas>,

    body: view! {
        title: "Graphics Demo"
        width: 860.0
        height: 460.0
        content: TabView {
            TabViewItem {
                header: "Fills"
                content: fills_canvas
                closable: false
                on_close: || {}
            }
            TabViewItem {
                header: "Strokes"
                content: strokes_canvas
                closable: false
                on_close: || {}
            }
            TabViewItem {
                header: "Paths"
                content: paths_canvas
                closable: false
                on_close: || {}
            }
            TabViewItem {
                header: "Path Combine"
                content: path_combine_canvas
                closable: false
                on_close: || {}
            }
            TabViewItem {
                header: "Compositing & Images"
                content: compositing_and_images_canvas
                closable: false
                on_close: || {}
            }
            selected_index: vm.selected_tab
            on_select: |index| { vm.select_tab(index) }
            on_new_tab: || {}
        }
    },
}

fn main() {
    let vm = GraphicsDemoViewModel::new();
    let window = GraphicsDemoWindow::new(
        vm,
        GraphicsDemoCanvas::new(GraphicsDemoCategory::Fills),
        GraphicsDemoCanvas::new(GraphicsDemoCategory::Strokes),
        GraphicsDemoCanvas::new(GraphicsDemoCategory::Paths),
        GraphicsDemoCanvas::new(GraphicsDemoCategory::PathCombine),
        GraphicsDemoCanvas::new(GraphicsDemoCategory::CompositingAndImages),
    );
    window.show();
    elwindui::application::run();
}
