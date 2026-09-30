//! `elwindui::ui::TextBlock` — self-drawn text display, and its local text-style storage.

use super::*;

#[derive(Clone)]
struct TextMeasureCache {
    text: String,
    style: crate::graphics::ComputedTextStyle,
    available: Size,
    alignment: TextAlignment,
    backend_generation: u64,
    measured_size: Size,
}

impl TextMeasureCache {
    fn matches(
        &self,
        text: &str,
        style: &crate::graphics::ComputedTextStyle,
        available: Size,
        alignment: TextAlignment,
        backend_generation: u64,
    ) -> bool {
        self.text == text
            && self.style == *style
            && self.available == available
            && self.alignment == alignment
            && self.backend_generation == backend_generation
    }
}

/// Self-drawn primitive text (WinUI3's `TextBlock`) — no native widget. A leaf, like `NativeControlImpl`. Field named `text` (not `content`) to match `elwindui::ui::TextBlock`'s own `#[param]
/// text` name — `elwindui-codegen`'s setter-based construction calls `.set_{param name}(..)`
/// generically, so the Rust field/setter name must agree with the DSL's own field name.
/// `TextBlock`'s own class trait (docs/design/runtime/ui_tree_design.md); `TextBlock` has no
/// further DSL-level subclass today.
///
/// `text_style` replaces the old `color: RefCell<Option<Color>>` field — foreground is now one of
/// the seven properties [`TextStyleOwner`] manages (`foreground: Option<Brush>`, not a bare
/// `Color`), inherited the same way `font_size`/`font_family`/etc. are (指示書 §2/§8). There is no
/// DSL `color:` property anymore; use `foreground:` instead.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
#[text_style]
#[prop(text: String)]
#[prop(text_alignment: Option<crate::ui::TextAlignment>)]
pub struct TextBlock {
    pub text: RefCell<String>,
    pub text_style: crate::graphics::TextStyleStorage,
    pub alignment: Cell<TextAlignment>,
    measure_cache: RefCell<Option<TextMeasureCache>>,
}

#[elwindui_macros::class]
impl TextBlock {
    #[overrides]
    fn accessibility_intrinsic_semantics(
        &self,
    ) -> Option<crate::accessibility::AccessibilitySemantics> {
        let mut semantics = crate::accessibility::AccessibilitySemantics::new(
            crate::accessibility::AccessibilityRole::StaticText,
        );
        let text = self.text.borrow().clone();
        semantics.label = Some(text.clone());
        semantics.value = Some(text);
        Some(semantics)
    }

    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let style = self.resolved_text_style();
        let text = self.text.borrow().clone();
        let alignment = self.alignment.get();
        let backend_generation = crate::graphics::text_backend_generation();
        if let Some(cache) = self.measure_cache.borrow().as_ref() {
            if cache.matches(&text, &style, available, alignment, backend_generation) {
                return cache.measured_size;
            }
        }
        let measured_size = crate::graphics::text_backend()
            .measure_text(&crate::graphics::TextMeasureRequest {
                text: &text,
                style: &style,
                available,
                // `TextBlock` has no `text_wrapping` DSL property yet (未対応, outside the seven
                // properties this pass covers — see `docs/design/runtime/text_design.md`); the request
                // shape already has the field so adding it later needs no signature change here.
                wrapping: crate::graphics::TextWrapping::NoWrap,
                alignment,
                max_lines: None,
                // No DPI/text-scale concept exists anywhere in `elwindui-core` yet (未対応).
                scale: 1.0,
            })
            .size;
        *self.measure_cache.borrow_mut() = Some(TextMeasureCache {
            text,
            style,
            available,
            alignment,
            backend_generation,
            measured_size,
        });
        measured_size
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        final_size
    }
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        // Re-resolved rather than cached from `measure_override` — nothing mutates between measure
        // and render within one pass, so the two resolutions are identical, and re-resolving avoids
        // a second, potentially-stale source of truth if a render pass ever runs without a
        // preceding full layout pass (see `docs/design/runtime/text_design.md`).
        let cascaded_style = self.cascaded_text_style();
        let style =
            cascaded_style.materialize(&crate::graphics::text_backend().default_text_style());
        context.draw_text_with_foreground(
            &self.text.borrow(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.arranged_width().unwrap_or(0.0),
                height: self.arranged_height().unwrap_or(0.0),
            },
            &style,
            cascaded_style.foreground.as_ref(),
            self.alignment.get(),
        );
    }
    #[overrides]
    fn as_text_style_owner(&self) -> Option<&dyn TextStyleOwner> {
        Some(self)
    }
    fn set_text(&self, text: &str) {
        if self.text.borrow().as_str() == text {
            return;
        }
        *self.text.borrow_mut() = text.to_string();
        self.invalidate_measure();
        self.request_accessibility_update();
    }
    fn set_text_alignment(&self, alignment: TextAlignment) {
        self.alignment.set(alignment);
        self.invalidate();
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            text: RefCell::new(String::new()),
            text_style: crate::graphics::TextStyleStorage::new(),
            alignment: Cell::new(TextAlignment::Left),
            measure_cache: RefCell::new(None),
        }
    }
}

impl TextStyleOwner for TextBlock {
    fn text_style_storage(&self) -> &crate::graphics::TextStyleStorage {
        &self.text_style
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{ComputedTextStyle, TextBackend, TextMeasureRequest, TextMeasureResult};
    use std::rc::Rc;

    struct CountingTextBackend {
        calls: Rc<Cell<usize>>,
        width_per_character: f32,
    }

    impl TextBackend for CountingTextBackend {
        fn default_text_style(&self) -> ComputedTextStyle {
            ComputedTextStyle::fallback()
        }

        fn measure_text(&self, request: &TextMeasureRequest<'_>) -> TextMeasureResult {
            self.calls.set(self.calls.get() + 1);
            TextMeasureResult {
                size: Size {
                    width: request.text.chars().count() as f32 * self.width_per_character,
                    height: 16.0,
                },
                baseline: 12.0,
                line_count: 1,
            }
        }
    }

    #[test]
    fn repeated_text_measurements_reuse_only_matching_backend_inputs() {
        crate::graphics::clear_text_backend();
        let first_calls = Rc::new(Cell::new(0));
        crate::graphics::set_text_backend(Rc::new(CountingTextBackend {
            calls: first_calls.clone(),
            width_per_character: 5.0,
        }));
        let text_block = TextBlock::new();
        text_block.set_text("cache");
        let initial = Size {
            width: 100.0,
            height: 24.0,
        };

        assert_eq!(text_block.measure_override(initial).width, 25.0);
        assert_eq!(text_block.measure_override(initial).width, 25.0);
        assert_eq!(first_calls.get(), 1);

        text_block.measure_override(Size {
            width: 80.0,
            height: 24.0,
        });
        assert_eq!(first_calls.get(), 2);
        text_block.set_text("changed");
        text_block.measure_override(Size {
            width: 80.0,
            height: 24.0,
        });
        assert_eq!(first_calls.get(), 3);

        let second_calls = Rc::new(Cell::new(0));
        crate::graphics::set_text_backend(Rc::new(CountingTextBackend {
            calls: second_calls.clone(),
            width_per_character: 7.0,
        }));
        assert_eq!(
            text_block
                .measure_override(Size {
                    width: 80.0,
                    height: 24.0,
                })
                .width,
            49.0
        );
        assert_eq!(second_calls.get(), 1);
        crate::graphics::clear_text_backend();
    }

    #[test]
    fn text_block_defaults_to_left_alignment_and_set_text_alignment_updates_paint() {
        let text_block = TextBlock::new();
        assert_eq!(text_block.alignment.get(), TextAlignment::Left);
        let mut commands = Vec::new();
        text_block.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                alignment: TextAlignment::Left,
                ..
            }
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                foreground: None,
                ..
            }
        ));

        text_block.set_text_alignment(TextAlignment::Center);
        commands.clear();
        text_block.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                alignment: TextAlignment::Center,
                ..
            }
        ));

        text_block.set_foreground(Some(Brush::Solid(Color::rgb(1, 2, 3))));
        commands.clear();
        text_block.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                foreground: Some(Brush::Solid(Color {
                    r: 1,
                    g: 2,
                    b: 3,
                    ..
                })),
                ..
            }
        ));
    }

    #[test]
    fn equal_text_writes_do_not_request_relayout() {
        struct CountingHost {
            requests: Cell<usize>,
        }
        impl RelayoutHost for CountingHost {
            fn request_relayout(&self, _dirty_group_id: u64, _kind: InvalidationKind) {
                self.requests.set(self.requests.get() + 1);
            }
        }

        let text_block = TextBlock::new();
        let host = Rc::new(CountingHost {
            requests: Cell::new(0),
        });
        text_block.set_invalidate_host(Some(host.clone()));
        text_block.set_text("a");
        assert_eq!(host.requests.get(), 1);
        text_block.set_text("a");
        assert_eq!(host.requests.get(), 1);
        text_block.set_text("b");
        assert_eq!(host.requests.get(), 2);
    }
}
