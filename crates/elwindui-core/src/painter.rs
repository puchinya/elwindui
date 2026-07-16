use crate::base::{Point, Rect, Size};
use crate::ui::UIElementExt;
use std::any::Any;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// Modeled on WinUI3's `Windows.UI.Color { A, R, G, B }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// Parses `"#rrggbb"` or `"#rrggbbaa"` (alpha defaults to opaque).
    pub fn hex(s: &str) -> Self {
        let s = s.trim_start_matches('#');
        let r = u8::from_str_radix(&s[0..2], 16).expect("invalid hex color");
        let g = u8::from_str_radix(&s[2..4], 16).expect("invalid hex color");
        let b = u8::from_str_radix(&s[4..6], 16).expect("invalid hex color");
        let a = if s.len() >= 8 {
            u8::from_str_radix(&s[6..8], 16).expect("invalid hex color")
        } else {
            0xff
        };
        Self { r, g, b, a }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Path;
#[derive(Debug, Clone, Copy, Default)]
pub struct PaintStyle;
#[derive(Debug, Clone, Copy, Default)]
pub struct Font;
#[derive(Debug, Clone, Copy, Default)]
pub struct Image;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}
impl Default for TextAlignment {
    fn default() -> Self {
        Self::Left
    }
}

/// A backend-independent command owned by one Visual. Coordinates are local to its RenderGroup.
#[derive(Clone)]
pub enum RenderCommand {
    Rectangle {
        rect: Rect,
        corner_radius: f32,
        fill: Option<String>,
        stroke: Option<String>,
        stroke_width: f32,
    },
    Ellipse {
        rect: Rect,
        fill: Option<String>,
        stroke: Option<String>,
        stroke_width: f32,
    },
    Line {
        from: Point,
        to: Point,
        color: String,
        width: f32,
    },
    Path {
        path: Path,
        style: PaintStyle,
    },
    Text {
        content: String,
        rect: Rect,
        font: Font,
        color: Option<String>,
        alignment: TextAlignment,
    },
    Image {
        image: Image,
        rect: Rect,
    },
    /// The handle stays type-erased until a backend replays this command. `owner_id` resolves to
    /// the owning UIElement through RenderTree's weak index.
    NativeControl {
        owner_id: u64,
        handle: Rc<dyn Any>,
        rect: Rect,
    },
}

/// One retained Visual node. `commands` are this Visual's own content; `children` is the visual tree.
pub struct RenderGroup {
    pub id: u64,
    pub is_dirty: bool,
    pub offset: Point,
    /// The arranged local extent. It is retained separately from `clip`: an unclipped Visual can
    /// still need to re-record its local commands when only its size changes.
    pub(crate) size: Size,
    pub clip: Option<Rect>,
    pub commands: Vec<RenderCommand>,
    pub children: Vec<RenderGroup>,
}

impl RenderGroup {
    pub fn new(id: u64, offset: Point, clip: Option<Rect>) -> Self {
        Self {
            id,
            is_dirty: true,
            offset,
            size: Size::default(),
            clip,
            commands: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// Retained render tree plus lookup tables used by a host's deferred layout/render pass.
pub struct RenderTree {
    pub root: RenderGroup,
    pub group_paths: HashMap<u64, Vec<usize>>,
    pub visual_index: HashMap<u64, Weak<dyn UIElementExt>>,
}

impl RenderTree {
    pub(crate) fn with_root(root: RenderGroup) -> Self {
        Self {
            root,
            group_paths: HashMap::new(),
            visual_index: HashMap::new(),
        }
    }

    pub fn mark_dirty(&mut self, id: u64) -> bool {
        let Some(path) = self.group_paths.get(&id).cloned() else {
            return false;
        };
        let mut group = &mut self.root;
        for index in path {
            group = &mut group.children[index];
        }
        group.is_dirty = true;
        true
    }
}

/// The recording context supplied to `UIElement::render`. It records only the current Visual's
/// local commands; Visual-tree traversal owns begin/end group nesting.
pub struct RenderContext<'a> {
    commands: &'a mut Vec<RenderCommand>,
    pub offset: Point,
    pub clip: Option<Rect>,
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
        }
    }
    pub(crate) fn end_group(self) {}

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.commands.push(RenderCommand::Rectangle {
            rect,
            corner_radius: 0.0,
            fill: Some(color.to_hex()),
            stroke: None,
            stroke_width: 0.0,
        });
    }
    pub fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.commands.push(RenderCommand::Rectangle {
            rect,
            corner_radius: 0.0,
            fill: None,
            stroke: Some(color.to_hex()),
            stroke_width: width,
        });
    }
    pub fn stroke_circle(&mut self, center: Point, radius: f32, color: Color, width: f32) {
        self.commands.push(RenderCommand::Ellipse {
            rect: Rect {
                x: center.x - radius,
                y: center.y - radius,
                width: radius * 2.0,
                height: radius * 2.0,
            },
            fill: None,
            stroke: Some(color.to_hex()),
            stroke_width: width,
        });
    }
    pub fn draw_line(&mut self, from: Point, to: Point, color: Color, width: f32) {
        self.commands.push(RenderCommand::Line {
            from,
            to,
            color: color.to_hex(),
            width,
        });
    }
    pub fn draw_path(&mut self, path: &Path, style: PaintStyle) {
        self.commands
            .push(RenderCommand::Path { path: *path, style });
    }
    pub fn draw_text(&mut self, text: &str, pos: Point, font: Font, color: Color) {
        self.commands.push(RenderCommand::Text {
            content: text.into(),
            rect: Rect {
                x: pos.x,
                y: pos.y,
                width: 0.0,
                height: 0.0,
            },
            font,
            color: Some(color.to_hex()),
            alignment: TextAlignment::Left,
        });
    }
    pub fn draw_image(&mut self, image: &Image, rect: Rect) {
        self.commands.push(RenderCommand::Image {
            image: *image,
            rect,
        });
    }
    pub(crate) fn native_control(&mut self, owner_id: u64, handle: Rc<dyn Any>, rect: Rect) {
        self.commands.push(RenderCommand::NativeControl {
            owner_id,
            handle,
            rect,
        });
    }
    pub(crate) fn rectangle(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        fill: Option<String>,
        stroke: Option<String>,
        stroke_width: f32,
    ) {
        self.commands.push(RenderCommand::Rectangle {
            rect,
            corner_radius,
            fill,
            stroke,
            stroke_width,
        });
    }
    pub(crate) fn ellipse(
        &mut self,
        rect: Rect,
        fill: Option<String>,
        stroke: Option<String>,
        stroke_width: f32,
    ) {
        self.commands.push(RenderCommand::Ellipse {
            rect,
            fill,
            stroke,
            stroke_width,
        });
    }
    pub(crate) fn text(
        &mut self,
        content: String,
        rect: Rect,
        color: Option<String>,
        alignment: TextAlignment,
    ) {
        self.commands.push(RenderCommand::Text {
            content,
            rect,
            font: Font,
            color,
            alignment,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_without_alpha_defaults_to_opaque() {
        assert_eq!(Color::hex("#eeeeee").a, 0xff);
    }
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
            Color::hex("#112233"),
        );
        assert!(matches!(
            commands[0],
            RenderCommand::Rectangle {
                rect: Rect { x: 1.0, .. },
                ..
            }
        ));
    }
}
