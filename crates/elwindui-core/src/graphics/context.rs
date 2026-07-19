use super::brush::Brush;
use super::color::Color;
use super::command::{Clip, Font, RenderCommand, TextAlignment};
use super::image::{Image, ImageDrawOptions};
use super::path::{FillRule, Path};
use super::stroke::StrokeStyle;
use super::vector_image::{VectorImage, VectorImageDrawOptions};
use crate::base::{AffineTransform, CornerRadius, Point, Rect};

pub struct Fill<'a> {
    pub brush: &'a Brush,
    pub rule: FillRule,
}

pub struct Stroke<'a> {
    pub brush: &'a Brush,
    pub style: &'a StrokeStyle,
}

/// The recording context supplied to `UIElement::render`. It records only the current Visual's
/// local commands; Visual-tree traversal owns begin/end group nesting.
pub struct RenderContext<'a> {
    commands: &'a mut Vec<RenderCommand>,
    pub offset: Point,
    pub clip: Option<Rect>,
    stack_depth: u32,
}

/// Returned by [`RenderContext::save`]. Doesn't itself push any state — it exists to let a
/// caller wrap a sequence of manual `push_*`/`pop_*` calls and get a debug-build assertion that
/// they balanced (painter design doc §12) — prefer `with_transform`/`with_clip`/`with_opacity`
/// for anything that fits the closure-scoped shape, since those can't be unbalanced by construction.
pub struct SaveGuard<'ctx, 'a> {
    context: &'ctx mut RenderContext<'a>,
    depth_at_save: u32,
}

impl Drop for SaveGuard<'_, '_> {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.context.stack_depth, self.depth_at_save,
            "RenderContext state stack is unbalanced: a push_transform/push_clip/push_opacity \
             inside this SaveGuard's scope was never matched by a pop"
        );
    }
}

impl<'a> RenderContext<'a> {
    pub(crate) fn begin_group(
        commands: &'a mut Vec<RenderCommand>,
        offset: Point,
        clip: Option<Rect>,
    ) -> Self {
        Self {
            commands,
            offset,
            clip,
            stack_depth: 0,
        }
    }
    pub(crate) fn end_group(self) {
        debug_assert_eq!(
            self.stack_depth, 0,
            "RenderContext recording ended with an unbalanced push_transform/push_clip/push_opacity"
        );
    }

    pub fn save(&mut self) -> SaveGuard<'_, 'a> {
        SaveGuard {
            depth_at_save: self.stack_depth,
            context: self,
        }
    }

    pub fn push_transform(&mut self, transform: AffineTransform) {
        self.commands
            .push(RenderCommand::PushTransform { transform });
        self.stack_depth += 1;
    }
    pub fn pop_transform(&mut self) {
        self.commands.push(RenderCommand::PopTransform);
        self.stack_depth = self.stack_depth.saturating_sub(1);
    }
    pub fn with_transform(&mut self, transform: AffineTransform, f: impl FnOnce(&mut Self)) {
        self.push_transform(transform);
        f(self);
        self.pop_transform();
    }

    pub fn push_clip(&mut self, clip: Clip) {
        self.commands.push(RenderCommand::PushClip { clip });
        self.stack_depth += 1;
    }
    pub fn pop_clip(&mut self) {
        self.commands.push(RenderCommand::PopClip);
        self.stack_depth = self.stack_depth.saturating_sub(1);
    }
    pub fn with_clip(&mut self, clip: Clip, f: impl FnOnce(&mut Self)) {
        self.push_clip(clip);
        f(self);
        self.pop_clip();
    }

    pub fn push_opacity(&mut self, opacity: f32) {
        self.commands.push(RenderCommand::PushOpacity { opacity });
        self.stack_depth += 1;
    }
    pub fn pop_opacity(&mut self) {
        self.commands.push(RenderCommand::PopOpacity);
        self.stack_depth = self.stack_depth.saturating_sub(1);
    }
    pub fn with_opacity(&mut self, opacity: f32, f: impl FnOnce(&mut Self)) {
        self.push_opacity(opacity);
        f(self);
        self.pop_opacity();
    }

    pub fn fill_rect(&mut self, rect: Rect, brush: &Brush) {
        self.commands.push(RenderCommand::FillRect {
            rect,
            brush: brush.clone(),
        });
    }
    pub fn stroke_rect(&mut self, rect: Rect, brush: &Brush, stroke: &StrokeStyle) {
        self.commands.push(RenderCommand::StrokeRect {
            rect,
            brush: brush.clone(),
            stroke: stroke.clone(),
        });
    }
    pub fn draw_rect(
        &mut self,
        rect: Rect,
        fill: Option<&Brush>,
        stroke: Option<(&Brush, &StrokeStyle)>,
    ) {
        if let Some(brush) = fill {
            self.fill_rect(rect, brush);
        }
        if let Some((brush, style)) = stroke {
            self.stroke_rect(rect, brush, style);
        }
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, radii: CornerRadius, brush: &Brush) {
        self.commands.push(RenderCommand::FillRoundedRect {
            rect,
            radii,
            brush: brush.clone(),
        });
    }
    pub fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: CornerRadius,
        brush: &Brush,
        stroke: &StrokeStyle,
    ) {
        self.commands.push(RenderCommand::StrokeRoundedRect {
            rect,
            radii,
            brush: brush.clone(),
            stroke: stroke.clone(),
        });
    }
    pub fn draw_rounded_rect(
        &mut self,
        rect: Rect,
        radii: CornerRadius,
        fill: Option<&Brush>,
        stroke: Option<(&Brush, &StrokeStyle)>,
    ) {
        if let Some(brush) = fill {
            self.fill_rounded_rect(rect, radii, brush);
        }
        if let Some((brush, style)) = stroke {
            self.stroke_rounded_rect(rect, radii, brush, style);
        }
    }

    pub fn fill_ellipse(&mut self, rect: Rect, brush: &Brush) {
        self.commands.push(RenderCommand::FillEllipse {
            rect,
            brush: brush.clone(),
        });
    }
    pub fn stroke_ellipse(&mut self, rect: Rect, brush: &Brush, stroke: &StrokeStyle) {
        self.commands.push(RenderCommand::StrokeEllipse {
            rect,
            brush: brush.clone(),
            stroke: stroke.clone(),
        });
    }
    pub fn draw_ellipse(
        &mut self,
        rect: Rect,
        fill: Option<&Brush>,
        stroke: Option<(&Brush, &StrokeStyle)>,
    ) {
        if let Some(brush) = fill {
            self.fill_ellipse(rect, brush);
        }
        if let Some((brush, style)) = stroke {
            self.stroke_ellipse(rect, brush, style);
        }
    }

    pub fn fill_circle(&mut self, center: Point, radius: f32, brush: &Brush) {
        self.fill_ellipse(
            Rect {
                x: center.x - radius,
                y: center.y - radius,
                width: radius * 2.0,
                height: radius * 2.0,
            },
            brush,
        );
    }
    pub fn stroke_circle(
        &mut self,
        center: Point,
        radius: f32,
        brush: &Brush,
        stroke: &StrokeStyle,
    ) {
        self.stroke_ellipse(
            Rect {
                x: center.x - radius,
                y: center.y - radius,
                width: radius * 2.0,
                height: radius * 2.0,
            },
            brush,
            stroke,
        );
    }

    pub fn draw_line(&mut self, from: Point, to: Point, brush: &Brush, stroke: &StrokeStyle) {
        self.commands.push(RenderCommand::DrawLine {
            from,
            to,
            brush: brush.clone(),
            stroke: stroke.clone(),
        });
    }

    pub fn fill_path(&mut self, path: &Path, brush: &Brush, rule: FillRule) {
        self.commands.push(RenderCommand::FillPath {
            path: path.clone(),
            brush: brush.clone(),
            rule,
        });
    }
    pub fn stroke_path(&mut self, path: &Path, brush: &Brush, stroke: &StrokeStyle) {
        self.commands.push(RenderCommand::StrokePath {
            path: path.clone(),
            brush: brush.clone(),
            stroke: stroke.clone(),
        });
    }
    pub fn draw_path(&mut self, path: &Path, fill: Option<Fill<'_>>, stroke: Option<Stroke<'_>>) {
        if let Some(fill) = fill {
            self.fill_path(path, fill.brush, fill.rule);
        }
        if let Some(stroke) = stroke {
            self.stroke_path(path, stroke.brush, stroke.style);
        }
    }

    pub fn draw_image(
        &mut self,
        image: &Image,
        dest: Rect,
        source: Option<Rect>,
        options: ImageDrawOptions,
    ) {
        self.commands.push(RenderCommand::DrawImage {
            image: image.clone(),
            dest,
            source,
            options,
        });
    }

    pub fn draw_vector_image(
        &mut self,
        image: &VectorImage,
        dest: Rect,
        source: Option<Rect>,
        options: VectorImageDrawOptions,
    ) {
        self.commands.push(RenderCommand::DrawVectorImage {
            image: image.clone(),
            dest,
            source,
            options,
        });
    }

    pub fn draw_text(
        &mut self,
        text: &str,
        rect: Rect,
        color: Option<Color>,
        alignment: TextAlignment,
    ) {
        self.commands.push(RenderCommand::Text {
            content: text.into(),
            rect,
            font: Font,
            color,
            alignment,
        });
    }

    pub(crate) fn native_control(
        &mut self,
        owner_id: u64,
        handle: std::rc::Rc<dyn std::any::Any>,
        rect: Rect,
    ) {
        self.commands.push(RenderCommand::NativeControl {
            owner_id,
            handle,
            rect,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::Color;

    #[test]
    fn render_context_records_local_commands() {
        let mut commands = Vec::new();
        let mut context =
            RenderContext::begin_group(&mut commands, Point { x: 10.0, y: 20.0 }, None);
        context.fill_rect(
            Rect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
            },
            &Brush::Solid(Color::parse_hex("#112233").unwrap()),
        );
        assert!(matches!(
            commands[0],
            RenderCommand::FillRect {
                rect: Rect { x: 1.0, .. },
                ..
            }
        ));
    }

    #[test]
    fn command_recording_order_is_preserved() {
        let mut commands = Vec::new();
        let mut context = RenderContext::begin_group(&mut commands, Point { x: 0.0, y: 0.0 }, None);
        context.fill_rect(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            &Brush::Solid(Color::black()),
        );
        context.fill_ellipse(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            &Brush::Solid(Color::white()),
        );
        assert!(matches!(commands[0], RenderCommand::FillRect { .. }));
        assert!(matches!(commands[1], RenderCommand::FillEllipse { .. }));
    }

    #[test]
    fn with_transform_pushes_and_pops_in_pairs() {
        let mut commands = Vec::new();
        let mut context = RenderContext::begin_group(&mut commands, Point { x: 0.0, y: 0.0 }, None);
        context.with_transform(AffineTransform::identity(), |_| {});
        context.end_group();
        assert!(matches!(commands[0], RenderCommand::PushTransform { .. }));
        assert!(matches!(commands[1], RenderCommand::PopTransform));
    }

    #[test]
    #[should_panic]
    fn unbalanced_push_is_caught_in_debug_by_end_group() {
        let mut commands = Vec::new();
        let mut context = RenderContext::begin_group(&mut commands, Point { x: 0.0, y: 0.0 }, None);
        context.push_transform(AffineTransform::identity());
        context.end_group();
    }

    fn test_vector_image() -> crate::graphics::VectorImage {
        crate::graphics::VectorImageBuilder::new(
            crate::base::Size {
                width: 10.0,
                height: 10.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
        )
        .unwrap()
        .finish()
        .unwrap()
    }

    #[test]
    fn draw_vector_image_records_one_command_with_dest_source_and_options() {
        let mut commands = Vec::new();
        let mut context = RenderContext::begin_group(&mut commands, Point { x: 0.0, y: 0.0 }, None);
        let image = test_vector_image();
        let dest = Rect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        let source = Some(Rect {
            x: 0.0,
            y: 0.0,
            width: 5.0,
            height: 5.0,
        });
        let options = VectorImageDrawOptions {
            opacity: 0.5,
            ..Default::default()
        };
        context.draw_vector_image(&image, dest, source, options);
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            RenderCommand::DrawVectorImage {
                image: recorded,
                dest: recorded_dest,
                source: recorded_source,
                options: recorded_options,
            } => {
                assert_eq!(recorded.id(), image.id());
                assert_eq!(*recorded_dest, dest);
                assert_eq!(*recorded_source, source);
                assert_eq!(*recorded_options, options);
            }
            _ => panic!("expected DrawVectorImage"),
        }
    }

    #[test]
    fn render_command_draw_vector_image_clone_preserves_image_id() {
        let image = test_vector_image();
        let command = RenderCommand::DrawVectorImage {
            image: image.clone(),
            dest: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            source: None,
            options: VectorImageDrawOptions::default(),
        };
        let cloned = command.clone();
        match (command, cloned) {
            (
                RenderCommand::DrawVectorImage { image: a, .. },
                RenderCommand::DrawVectorImage { image: b, .. },
            ) => {
                assert_eq!(a.id(), b.id());
                assert_eq!(a.id(), image.id());
            }
            _ => panic!("expected DrawVectorImage variants"),
        }
    }
}
