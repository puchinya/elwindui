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
