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
