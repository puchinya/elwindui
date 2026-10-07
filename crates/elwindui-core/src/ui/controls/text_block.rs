//! `elwindui::ui::TextBlock` — self-drawn text display, and its local text-style storage.

use super::*;

const TEXT_MEASURE_CACHE_CAPACITY: usize = 8;

#[derive(Clone)]
struct TextMeasureCache {
    text: String,
    style: crate::graphics::ComputedTextStyle,
    alignment: TextAlignment,
    wrapping: crate::graphics::TextWrapping,
    backend_generation: u64,
    measurements: Vec<(Size, Size)>,
}

impl TextMeasureCache {
    fn matches(
        &self,
        text: &str,
        style: &crate::graphics::ComputedTextStyle,
        alignment: TextAlignment,
        wrapping: crate::graphics::TextWrapping,
        backend_generation: u64,
    ) -> bool {
        self.text == text
            && self.style == *style
            && self.alignment == alignment
            && self.wrapping == wrapping
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
///
/// `text_wrapping` defaults to [`crate::graphics::TextWrapping::NoWrap`]. Setting it to
/// `Wrap` or `WrapWholeWords` wraps text within the available width during both measurement
/// and painting, and invalidates previously measured sizes. An unconstrained width retains
/// the text's natural width.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
#[text_style]
#[prop(text: String)]
#[prop(text_alignment: Option<crate::ui::TextAlignment>)]
#[prop(text_wrapping: crate::graphics::TextWrapping)]
pub struct TextBlock {
    pub text: RefCell<String>,
    pub text_style: crate::graphics::TextStyleStorage,
    pub alignment: Cell<TextAlignment>,
    wrapping: Cell<crate::graphics::TextWrapping>,
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
        let wrapping = self.wrapping.get();
        let backend_generation = crate::graphics::text_backend_generation();
        {
            let mut cache_slot = self.measure_cache.borrow_mut();
            if cache_slot.as_ref().is_some_and(|cache| {
                !cache.matches(&text, &style, alignment, wrapping, backend_generation)
            }) {
                cache_slot.take();
            }
            if let Some(cache) = cache_slot.as_mut() {
                if let Some(index) = cache
                    .measurements
                    .iter()
                    .position(|(constraint, _)| *constraint == available)
                {
                    let measurement = cache.measurements.remove(index);
                    cache.measurements.push(measurement);
                    return measurement.1;
                }
            }
        }
        let measured_size = crate::graphics::text_backend()
            .measure_text(&crate::graphics::TextMeasureRequest {
                text: &text,
                style: &style,
                available,
                wrapping,
                alignment,
                max_lines: None,
                // No DPI/text-scale concept exists anywhere in `elwindui-core` yet (未対応).
                scale: 1.0,
            })
            .size;
        let mut cache_slot = self.measure_cache.borrow_mut();
        let cache = cache_slot.get_or_insert_with(|| TextMeasureCache {
            text,
            style,
            alignment,
            wrapping,
            backend_generation,
            measurements: Vec::new(),
        });
        if cache.measurements.len() == TEXT_MEASURE_CACHE_CAPACITY {
            cache.measurements.remove(0);
        }
        cache.measurements.push((available, measured_size));
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
        context.draw_text_with_foreground_and_wrapping(
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
            self.wrapping.get(),
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
    fn text_wrapping(&self) -> crate::graphics::TextWrapping {
        self.wrapping.get()
    }
    fn set_text_wrapping(&self, wrapping: crate::graphics::TextWrapping) {
        if self.wrapping.replace(wrapping) != wrapping {
            self.invalidate_measure();
        }
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            text: RefCell::new(String::new()),
            text_style: crate::graphics::TextStyleStorage::new(),
            alignment: Cell::new(TextAlignment::Left),
            wrapping: Cell::new(crate::graphics::TextWrapping::NoWrap),
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
                    width: (request.text.chars().count() as f32 * self.width_per_character)
                        .min(request.available.width.max(0.0)),
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
    fn wrapping_invalidates_cached_measurement_and_matches_rendering() {
        use crate::graphics::{DummyTextBackend, TextWrapping};
        struct WrappedBackend(Rc<Cell<usize>>);
        impl TextBackend for WrappedBackend {
            fn default_text_style(&self) -> ComputedTextStyle {
                ComputedTextStyle::fallback()
            }
            fn measure_text(&self, request: &TextMeasureRequest<'_>) -> TextMeasureResult {
                self.0.set(self.0.get() + 1);
                DummyTextBackend.measure_text(request)
            }
        }
        let calls = Rc::new(Cell::new(0));
        crate::graphics::set_text_backend(Rc::new(WrappedBackend(calls.clone())));
        let text = TextBlock::new();
        text.set_text("abcdefghij");
        assert_eq!(text.text_wrapping(), TextWrapping::NoWrap);
        let constraint = Size {
            width: 32.0,
            height: f32::INFINITY,
        };
        assert_eq!(text.measure_override(constraint).height, 16.0);
        text.measure_override(constraint);
        assert_eq!(calls.get(), 1);
        text.set_text_wrapping(TextWrapping::Wrap);
        assert_eq!(
            text.measure_override(constraint),
            Size {
                width: 32.0,
                height: 48.0
            }
        );
        text.measure_override(constraint);
        assert_eq!(calls.get(), 2);
        assert_eq!(
            text.measure_override(Size {
                width: f32::INFINITY,
                height: f32::INFINITY
            }),
            Size {
                width: 80.0,
                height: 16.0
            }
        );
        let mut commands = Vec::new();
        text.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(matches!(
            commands[0],
            RenderCommand::Text {
                wrapping: TextWrapping::Wrap,
                ..
            }
        ));
        let wrapped = commands[0].clone();
        text.set_text_wrapping(TextWrapping::NoWrap);
        commands.clear();
        text.render(&mut RenderContext::begin_group(
            &mut commands,
            Point { x: 0.0, y: 0.0 },
            None,
        ));
        assert!(!wrapped.visually_eq(&commands[0]));
        assert_ne!(
            wrapped.fingerprint().geometry,
            commands[0].fingerprint().geometry
        );
        crate::graphics::clear_text_backend();
    }

    #[test]
    fn alternating_constraints_reuse_metrics_and_evict_only_the_oldest_constraint() {
        let calls = Rc::new(Cell::new(0));
        crate::graphics::set_text_backend(Rc::new(CountingTextBackend {
            calls: calls.clone(),
            width_per_character: 5.0,
        }));
        let text_block = TextBlock::new();
        text_block.set_text("alternating");
        let natural = Size {
            width: f32::INFINITY,
            height: f32::INFINITY,
        };
        let constrained = Size {
            width: 20.0,
            height: 24.0,
        };
        for _ in 0..32 {
            assert_eq!(text_block.measure_override(natural).width, 55.0);
            assert_eq!(text_block.measure_override(constrained).width, 20.0);
        }
        assert_eq!(calls.get(), 2);
        for index in 0..TEXT_MEASURE_CACHE_CAPACITY - 2 {
            text_block.measure_override(Size {
                width: 100.0 + index as f32,
                height: 24.0,
            });
        }
        assert_eq!(calls.get(), TEXT_MEASURE_CACHE_CAPACITY);
        text_block.measure_override(natural); // Keep natural metrics most recently used.
        text_block.measure_override(Size {
            width: 999.0,
            height: 24.0,
        });
        assert_eq!(calls.get(), TEXT_MEASURE_CACHE_CAPACITY + 1);
        text_block.measure_override(natural);
        assert_eq!(calls.get(), TEXT_MEASURE_CACHE_CAPACITY + 1);
        text_block.measure_override(constrained);
        assert_eq!(calls.get(), TEXT_MEASURE_CACHE_CAPACITY + 2);
        assert_eq!(
            text_block
                .measure_cache
                .borrow()
                .as_ref()
                .unwrap()
                .measurements
                .len(),
            TEXT_MEASURE_CACHE_CAPACITY
        );
        crate::graphics::clear_text_backend();
    }

    #[test]
    fn local_inherited_style_and_alignment_changes_clear_all_constraint_entries() {
        let calls = Rc::new(Cell::new(0));
        crate::graphics::set_text_backend(Rc::new(CountingTextBackend {
            calls: calls.clone(),
            width_per_character: 5.0,
        }));
        let parent = ContentControl::new();
        let text_block = TextBlock::new();
        text_block.set_text("styled");
        parent.set_content(text_block.clone());
        let constraints = [
            Size {
                width: 100.0,
                height: 24.0,
            },
            Size {
                width: 80.0,
                height: 24.0,
            },
        ];
        let measure_both = || {
            for available in constraints {
                text_block.measure_override(available);
            }
        };
        measure_both();
        measure_both();
        assert_eq!(calls.get(), 2);
        parent.as_control().set_font_size(24.0);
        measure_both();
        assert_eq!(calls.get(), 4);
        text_block.set_font_size(18.0);
        measure_both();
        assert_eq!(calls.get(), 6);
        text_block.set_text_alignment(TextAlignment::Right);
        measure_both();
        assert_eq!(calls.get(), 8);
        text_block.set_text("new text");
        measure_both();
        assert_eq!(calls.get(), 10);
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
