//! `NativeControl` — the base class every native leaf in this backend inherits from.

use crate::AnyView;
use elwindui_core::graphics::{Brush, CascadedTextStyle, TextStyleStorage};
use elwindui_core::theme::{SystemTheme, ThemeHandle, ThemeToken, ThemeValue};
use elwindui_core::ui::{TextStyleOwner, UIElementExt};
use std::any::Any;
use std::cell::RefCell;

/// The backend-owned counterpart to `elwindui_core::ui::NativeControl` (a pure marker trait with no
/// backing struct of its own — measuring/placing a native handle is entirely backend-specific, so
/// `elwindui-core` doesn't define this generically). Holding `handle: AnyView` here once, instead
/// of on each of `TextArea`/`Button`/`TabView` individually, is what lets `inherits = NativeControl`
/// resolve `base`'s field type to this same struct.
///
/// `text_style`/`applied` add font support (指示書 §17): every leaf sharing this one base struct
/// gets `TextStyleOwner` (storage + inherited resolution) and font application for free, without
/// each of `Button`/`TextArea`/`TextBox`/`PasswordBox`/`ScrollView`/`TabView` repeating it.
#[elwindui_macros::class(struct_only = elwindui_core::ui::NativeControlExt, inherits = elwindui_core::ui::UIElement)]
pub struct NativeControl {
    handle: AnyView,
    background: RefCell<Option<Brush>>,
    applied_background: RefCell<Option<ThemeValue<Brush>>>,
    text_style: TextStyleStorage,
    /// The style last actually pushed to `handle` — lets `sync_text_style` skip a redundant
    /// AppKit call when nothing changed since the previous measure pass.
    applied: RefCell<Option<CascadedTextStyle>>,
}

#[elwindui_macros::class]
impl NativeControl {
    #[overrides]
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.sync_background();
        self.sync_text_style();
        self.handle.measure(available)
    }
    #[overrides]
    fn try_as_native_control(&self) -> Option<&dyn Any> {
        Some(&self.handle)
    }
    #[overrides]
    fn as_text_style_owner(&self) -> Option<&dyn TextStyleOwner> {
        Some(self)
    }
    fn set_background(&self, background: Option<Brush>) {
        let Some(background) = background else {
            self.clear_background();
            return;
        };
        self.handle.apply_background(Some(&background));
        *self.background.borrow_mut() = Some(background.clone());
        *self.applied_background.borrow_mut() = Some(ThemeValue::Value(background));
        self.invalidate();
    }
    fn clear_background(&self) {
        self.background.borrow_mut().take();
        self.applied_background.borrow_mut().take();
        self.sync_background();
        self.invalidate();
    }
    fn construct(handle: AnyView) -> Self {
        Self {
            base: elwindui_core::ui::UIElement::construct(),
            handle,
            background: RefCell::new(None),
            applied_background: RefCell::new(None),
            text_style: TextStyleStorage::new(),
            applied: RefCell::new(None),
        }
    }
}

impl NativeControl {
    fn sync_background(&self) {
        let desired = self
            .background
            .borrow()
            .clone()
            .map(ThemeValue::Value)
            .unwrap_or_else(|| {
                self.theme_handle()
                    .resolve(background_token(self.handle.theme_prefix()))
            });
        if self.applied_background.borrow().as_ref() == Some(&desired) {
            return;
        }
        match desired.as_ref() {
            ThemeValue::Value(background) => self.handle.apply_background(Some(background)),
            ThemeValue::PlatformDefault => self.handle.apply_background(None),
        }
        *self.applied_background.borrow_mut() = Some(desired);
    }

    /// Pulls this element's resolved text style and pushes it to `handle`, but only when it
    /// actually differs from what was last applied — pull-based (called from `measure_override`,
    /// which `UIElementExt::measure` runs unconditionally every layout pass) rather than pushed
    /// from `TextStyleOwner::on_text_style_property_changed`, because a base-class method has no
    /// way to reach the most-derived leaf object (`UIElement` keeps no self-`Weak`, and
    /// `#[overridable]` isn't available on a `struct_only` class — see
    /// `docs/elwindui_macro_class_spec.md`). Every text-style setter already calls
    /// `invalidate_measure()`, so this is guaranteed to run again before anything is drawn.
    ///
    /// `pub(crate)` (not `#[inherent]`/private): `native_ui::text::TextArea` overrides
    /// `measure_override` itself (`NSScrollView.fittingSize()` reports `{0,0}`) and must call this
    /// first — see that override's own doc comment.
    pub(crate) fn sync_text_style(&self) {
        // 指示書 §17: a font-incapable leaf (e.g. a bare `NSStackView`) must not be treated as
        // "handled" — but it's also not an error, it simply has nowhere to put a text style, so
        // this skips the (harmless but pointless) resolve-and-apply work rather than warning.
        if !self.handle.supports_text_style() {
            return;
        }
        let mut cascaded = self.cascaded_text_style();
        apply_theme_text_style(&self.theme_handle(), &mut cascaded);
        // AppKit native controls have no supported way to take an arbitrary custom text color
        // without abandoning their own system-drawn appearance — same platform constraint as
        // `AppKitHandle::apply_background`'s own doc comment. They always keep the system's own
        // Light/Dark-following text color; a `foreground` requested via `set_foreground`/`theme!`
        // (explicit or theme-resolved) is intentionally discarded here, never reaching `handle`.
        cascaded.foreground = None;
        if self.applied.borrow().as_ref() != Some(&cascaded) {
            self.handle.apply_text_style(&cascaded);
            *self.applied.borrow_mut() = Some(cascaded);
        }
    }
}

fn background_token(prefix: &str) -> ThemeToken<Brush> {
    match prefix {
        "button" => SystemTheme::button_background,
        "text_box" => SystemTheme::text_box_background,
        "password_box" => SystemTheme::password_box_background,
        "text_area" => SystemTheme::text_area_background,
        "scroll_view" => SystemTheme::scroll_view_background,
        "tab_view" => SystemTheme::tab_view_background,
        _ => SystemTheme::native_control_background,
    }
}

/// Fills only font metrics from the theme — never `foreground` (see `sync_text_style`'s own doc
/// comment on why a native control's text color always stays platform-default).
fn apply_theme_text_style(theme: &ThemeHandle, style: &mut CascadedTextStyle) {
    macro_rules! fill {
        ($field:ident, $token:expr) => {
            if style.$field.is_none() {
                if let ThemeValue::Value(value) = theme.resolve($token) {
                    style.$field = Some(value);
                }
            }
        };
    }
    fill!(font_family, SystemTheme::native_control_font_family);
    fill!(font_size, SystemTheme::native_control_font_size);
    fill!(font_weight, SystemTheme::native_control_font_weight);
    fill!(font_style, SystemTheme::native_control_font_style);
    fill!(font_stretch, SystemTheme::native_control_font_stretch);
    fill!(
        character_spacing,
        SystemTheme::native_control_character_spacing
    );
}

impl TextStyleOwner for NativeControl {
    fn text_style_storage(&self) -> &TextStyleStorage {
        &self.text_style
    }
}
