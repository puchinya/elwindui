/// See docs/elwindui_spec.md 付録H.2.
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Measure/Arrange two-pass layout, implemented by every builtin (`Stack`, `Canvas`, ...).
/// See docs/elwindui_spec.md 付録H.2.
pub trait LayoutNode {
    fn measure(&self, available: Size) -> Size;
    fn arrange(&mut self, final_rect: Rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedSize {
        size: Size,
        arranged: Option<Rect>,
    }

    impl LayoutNode for FixedSize {
        fn measure(&self, _available: Size) -> Size {
            self.size
        }

        fn arrange(&mut self, final_rect: Rect) {
            self.arranged = Some(final_rect);
        }
    }

    #[test]
    fn measure_ignores_available_and_arrange_records_final_rect() {
        let mut node = FixedSize {
            size: Size {
                width: 10.0,
                height: 20.0,
            },
            arranged: None,
        };

        let measured = node.measure(Size {
            width: 100.0,
            height: 100.0,
        });
        assert_eq!(measured, node.size);

        let rect = Rect {
            x: 1.0,
            y: 2.0,
            width: 10.0,
            height: 20.0,
        };
        node.arrange(rect);
        assert_eq!(node.arranged, Some(rect));
    }
}
