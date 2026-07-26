//! Typed, backend-neutral theme values.
//!
//! A theme deliberately does not turn an operating-system default into an eager color/font.
//! [`ThemeValue::PlatformDefault`] survives until the property adapter reaches the backend, where
//! WinUI can call `ClearValue` and AppKit can restore a dynamic system color/font.

use crate::graphics::{Brush, FontFamily, FontStretch, FontStyle, FontWeight};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;

/// Whether the application follows the operating system or requests a particular appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePreference {
    /// Follow the operating-system appearance.
    #[default]
    System,
    /// Request a light appearance.
    Light,
    /// Request a dark appearance.
    Dark,
}

/// The appearance currently reported by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeAppearance {
    /// The backend currently uses a light appearance.
    #[default]
    Light,
    /// The backend currently uses a dark appearance.
    Dark,
    /// The backend currently uses a high-contrast appearance.
    HighContrast,
}

/// The most expensive work required after a token change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ThemeChangeImpact {
    /// Only retained drawing needs to be repainted.
    #[default]
    Paint,
    /// Layout measurement may have changed.
    Measure,
    /// Native toolkit styling must be reapplied.
    NativeStyle,
}

/// The resolved value of one typed token.
#[derive(Debug, Clone, PartialEq)]
pub enum ThemeValue<T> {
    /// Apply the contained explicit theme value.
    Value(T),
    /// Clear any explicit value and let the active backend choose its default.
    PlatformDefault,
}

impl<T> ThemeValue<T> {
    /// Maps an explicit value while preserving [`ThemeValue::PlatformDefault`].
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> ThemeValue<U> {
        match self {
            Self::Value(value) => ThemeValue::Value(map(value)),
            Self::PlatformDefault => ThemeValue::PlatformDefault,
        }
    }

    /// Borrows an explicit value while preserving [`ThemeValue::PlatformDefault`].
    pub fn as_ref(&self) -> ThemeValue<&T> {
        match self {
            Self::Value(value) => ThemeValue::Value(value),
            Self::PlatformDefault => ThemeValue::PlatformDefault,
        }
    }
}

/// A typed token descriptor. Theme definitions store values by `name`; the generic parameter
/// makes an accidental `Brush`/`f32` mix-up a Rust type error at the call site.
#[derive(Debug, PartialEq, Eq)]
pub struct ThemeToken<T> {
    name: &'static str,
    impact: ThemeChangeImpact,
    standard: bool,
    marker: PhantomData<fn() -> T>,
}

impl<T> Copy for ThemeToken<T> {}

impl<T> Clone for ThemeToken<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> ThemeToken<T> {
    /// Creates a typed token descriptor.
    pub const fn new(name: &'static str, impact: ThemeChangeImpact, standard: bool) -> Self {
        Self {
            name,
            impact,
            standard,
            marker: PhantomData,
        }
    }

    /// Returns the stable token name used by generated theme definitions.
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns the invalidation required when the token changes.
    pub const fn impact(self) -> ThemeChangeImpact {
        self.impact
    }

    /// Returns whether the token belongs to the built-in [`SystemTheme`] manifest.
    pub const fn is_standard(self) -> bool {
        self.standard
    }
}

/// Type-erased storage used only at the boundary between generated theme definitions and typed
/// [`ThemeHandle::resolve`] calls.
#[doc(hidden)]
pub enum ErasedThemeValue {
    /// A type-erased explicit value.
    Value(Box<dyn Any>),
    /// A backend-owned default.
    PlatformDefault,
}

#[doc(hidden)]
pub fn erase_theme_value<T: 'static>(value: ThemeValue<T>) -> ErasedThemeValue {
    match value {
        ThemeValue::Value(value) => ErasedThemeValue::Value(Box::new(value)),
        ThemeValue::PlatformDefault => ErasedThemeValue::PlatformDefault,
    }
}

/// Implemented by code generated from `#[elwindui::theme_definition]`.
#[doc(hidden)]
pub trait ThemeDefinition {
    /// Resolves one token into type-erased storage.
    fn resolve_erased(&self, token: &str) -> Option<ErasedThemeValue>;
    /// Returns the active variant label.
    fn variant_name(&self) -> &'static str;
}

/// Marker implemented for a generated theme type.
pub trait ThemeFactory: 'static {
    /// The generated variant enum for this theme.
    type Variant: Clone + PartialEq + 'static;

    /// Builds an immutable definition for one variant.
    fn create_definition(variant: &Self::Variant) -> Rc<dyn ThemeDefinition>;

    /// Returns the invalidation required when switching between two variants.
    fn change_impact(_previous: &Self::Variant, _next: &Self::Variant) -> ThemeChangeImpact {
        ThemeChangeImpact::NativeStyle
    }
}

struct ThemeState {
    definition: RefCell<Rc<dyn ThemeDefinition>>,
    preference: Cell<ThemePreference>,
    appearance: Cell<ThemeAppearance>,
    last_change_impact: Cell<ThemeChangeImpact>,
    revision: Cell<u64>,
}

/// Type-erased theme handle suitable for application and Window storage.
#[derive(Clone)]
pub struct ThemeHandle {
    state: Rc<ThemeState>,
}

impl std::fmt::Debug for ThemeHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ThemeHandle")
            .field("variant", &self.variant_name())
            .field("preference", &self.preference())
            .field("appearance", &self.appearance())
            .field("revision", &self.revision())
            .finish()
    }
}

impl ThemeHandle {
    fn new(
        definition: Rc<dyn ThemeDefinition>,
        preference: ThemePreference,
        appearance: ThemeAppearance,
    ) -> Self {
        Self {
            state: Rc::new(ThemeState {
                definition: RefCell::new(definition),
                preference: Cell::new(preference),
                appearance: Cell::new(appearance),
                last_change_impact: Cell::new(ThemeChangeImpact::NativeStyle),
                revision: Cell::new(0),
            }),
        }
    }

    /// Resolves a typed token without materializing backend defaults.
    pub fn resolve<T: Clone + 'static>(&self, token: ThemeToken<T>) -> ThemeValue<T> {
        let definition = self.state.definition.borrow();
        let mut name = token.name();
        loop {
            match definition.resolve_erased(name) {
                Some(ErasedThemeValue::Value(value)) => {
                    let value = value.downcast::<T>().unwrap_or_else(|_| {
                        panic!(
                            "theme token `{}` was generated with an incompatible value type",
                            name
                        )
                    });
                    return ThemeValue::Value((*value).clone());
                }
                // An explicitly declared platform default is terminal. It must not fall through
                // to a base token, otherwise an app could not restore one concrete native widget
                // while retaining a themed base-control value.
                Some(ErasedThemeValue::PlatformDefault) => {
                    return ThemeValue::PlatformDefault;
                }
                None if token.is_standard() => {
                    let Some(fallback) = standard_token_fallback(name) else {
                        return ThemeValue::PlatformDefault;
                    };
                    name = fallback;
                }
                None => return ThemeValue::PlatformDefault,
            }
        }
    }

    /// Returns the requested light/dark preference.
    pub fn preference(&self) -> ThemePreference {
        self.state.preference.get()
    }

    /// Returns the effective appearance most recently reported by the backend.
    pub fn appearance(&self) -> ThemeAppearance {
        self.state.appearance.get()
    }

    /// Returns the monotonically wrapping theme revision.
    pub fn revision(&self) -> u64 {
        self.state.revision.get()
    }

    pub(crate) fn last_change_impact(&self) -> ThemeChangeImpact {
        self.state.last_change_impact.get()
    }

    /// Returns the generated name of the active application variant.
    pub fn variant_name(&self) -> &'static str {
        self.state.definition.borrow().variant_name()
    }

    /// Updates the effective appearance reported by a backend.
    pub fn set_appearance(&self, appearance: ThemeAppearance) {
        if self.state.appearance.replace(appearance) != appearance {
            self.changed(ThemeChangeImpact::NativeStyle);
        }
    }

    fn replace_definition(&self, definition: Rc<dyn ThemeDefinition>, impact: ThemeChangeImpact) {
        *self.state.definition.borrow_mut() = definition;
        self.changed(impact);
    }

    fn set_preference(&self, preference: ThemePreference) {
        if self.state.preference.replace(preference) != preference {
            self.changed(ThemeChangeImpact::NativeStyle);
        }
    }

    fn changed(&self, impact: ThemeChangeImpact) {
        self.state.last_change_impact.set(impact);
        self.state
            .revision
            .set(self.state.revision.get().wrapping_add(1));
        notify_application_theme_changed(impact);
    }
}

fn standard_token_fallback(name: &str) -> Option<&'static str> {
    Some(match name {
        "button_hover_background" | "button_pressed_background" | "button_disabled_background" => {
            "button_background"
        }
        "button_hover_foreground" | "button_pressed_foreground" | "button_disabled_foreground" => {
            "button_foreground"
        }
        "button_background" => "native_control_background",
        "button_foreground" => "native_control_foreground",
        "button_border" => "native_control_border",

        "text_box_placeholder_foreground" => "text_box_foreground",
        "text_box_selection_background" => "text_box_background",
        "text_box_focus_border" => "text_box_border",
        "text_box_background" => "native_control_background",
        "text_box_foreground" | "text_box_caret" => "native_control_foreground",
        "text_box_border" => "native_control_border",

        "password_box_placeholder_foreground" => "password_box_foreground",
        "password_box_selection_background" => "password_box_background",
        "password_box_focus_border" => "password_box_border",
        "password_box_background" => "native_control_background",
        "password_box_foreground" | "password_box_caret" => "native_control_foreground",
        "password_box_border" => "native_control_border",

        "text_area_placeholder_foreground" => "text_area_foreground",
        "text_area_selection_background" => "text_area_background",
        "text_area_focus_border" => "text_area_border",
        "text_area_background" => "native_control_background",
        "text_area_foreground" | "text_area_caret" => "native_control_foreground",
        "text_area_border" => "native_control_border",

        "scroll_view_scrollbar_hover_thumb" => "scroll_view_scrollbar_thumb",
        "scroll_view_scrollbar_thumb" => "native_control_foreground",
        "scroll_view_scrollbar_background" | "scroll_view_background" => {
            "native_control_background"
        }

        "tab_view_item_selected_background"
        | "tab_view_item_hover_background"
        | "tab_view_item_close_button_background" => "tab_view_item_background",
        "tab_view_item_selected_foreground"
        | "tab_view_item_disabled_foreground"
        | "tab_view_item_close_button_foreground" => "tab_view_item_foreground",
        "tab_view_item_background" => "tab_view_background",
        "tab_view_item_foreground" => "tab_view_foreground",
        "tab_view_background" => "native_control_background",
        "tab_view_foreground" => "native_control_foreground",

        "menu_item_selected_background" => "menu_item_background",
        "menu_item_selected_foreground" | "menu_item_disabled_foreground" => "menu_item_foreground",
        "menu_item_background" => "menu_background",
        "menu_item_foreground" => "menu_foreground",
        "menu_background" => "menu_bar_background",
        "menu_foreground" => "menu_bar_foreground",

        "native_control_background" => "control_background",
        "native_control_foreground" => "control_foreground",
        "native_control_border" => "control_border",
        "native_control_font_family" => "control_font_family",
        "native_control_font_size" => "control_font_size",
        "native_control_font_weight" => "control_font_weight",
        "native_control_font_style" => "control_font_style",
        "native_control_font_stretch" => "control_font_stretch",
        "native_control_character_spacing" => "control_character_spacing",
        _ => return None,
    })
}

/// A live typed controller. All clones of its [`ThemeHandle`] observe variant/preference changes.
pub struct ThemeController<T: ThemeFactory> {
    variant: RefCell<T::Variant>,
    handle: ThemeHandle,
    marker: PhantomData<T>,
}

impl<T: ThemeFactory> ThemeController<T> {
    /// Creates a live controller using `initial_variant`.
    pub fn new(initial_variant: T::Variant) -> Self {
        Self {
            handle: ThemeHandle::new(
                T::create_definition(&initial_variant),
                ThemePreference::System,
                ThemeAppearance::Light,
            ),
            variant: RefCell::new(initial_variant),
            marker: PhantomData,
        }
    }

    /// Returns a type-erased handle sharing this controller's live state.
    pub fn handle(&self) -> ThemeHandle {
        self.handle.clone()
    }

    /// Returns the active application variant.
    pub fn variant(&self) -> T::Variant {
        self.variant.borrow().clone()
    }

    /// Selects an application variant and publishes a theme revision.
    pub fn set_variant(&self, variant: T::Variant) {
        let previous = self.variant.borrow().clone();
        if previous == variant {
            return;
        }
        let impact = T::change_impact(&previous, &variant);
        *self.variant.borrow_mut() = variant.clone();
        self.handle
            .replace_definition(T::create_definition(&variant), impact);
    }

    /// Returns the requested light/dark preference.
    pub fn preference(&self) -> ThemePreference {
        self.handle.preference()
    }

    /// Sets the requested light/dark preference.
    pub fn set_preference(&self, preference: ThemePreference) {
        self.handle.set_preference(preference);
    }

    /// Returns the backend-reported effective appearance.
    pub fn appearance(&self) -> ThemeAppearance {
        self.handle.appearance()
    }

    /// Returns the current theme revision.
    pub fn revision(&self) -> u64 {
        self.handle.revision()
    }
}

type ThemeListener = Rc<dyn Fn(ThemeChangeImpact)>;

thread_local! {
    static APPLICATION_THEME: RefCell<ThemeHandle> =
        RefCell::new(SystemTheme::controller(SystemThemeVariant::Default).handle());
    static APPLICATION_THEME_LISTENERS: RefCell<Vec<ThemeListener>> = const { RefCell::new(Vec::new()) };
}

/// Makes a handle the default for windows that do not provide their own theme.
pub fn set_application_theme(theme: ThemeHandle) {
    APPLICATION_THEME.with(|slot| *slot.borrow_mut() = theme);
    notify_application_theme_changed(ThemeChangeImpact::NativeStyle);
}

/// Returns the current application default theme.
pub fn application_theme() -> ThemeHandle {
    APPLICATION_THEME.with(|slot| slot.borrow().clone())
}

/// Resolves a token from the current application default theme.
pub fn resolve_application_theme<T: Clone + 'static>(token: ThemeToken<T>) -> ThemeValue<T> {
    application_theme().resolve(token)
}

/// Registers a process-UI-thread listener. Generated components capture themselves weakly, so the
/// application theme does not keep a window alive.
pub fn subscribe_application_theme(listener: impl Fn(ThemeChangeImpact) + 'static) {
    APPLICATION_THEME_LISTENERS.with(|listeners| listeners.borrow_mut().push(Rc::new(listener)));
}

fn notify_application_theme_changed(impact: ThemeChangeImpact) {
    APPLICATION_THEME_LISTENERS.with(|listeners| {
        for listener in listeners.borrow().iter() {
            listener(impact);
        }
    });
}

/// Theme selection inherited by a hosted visual tree.
#[derive(Debug, Clone)]
pub struct ThemeContext {
    theme: ThemeHandle,
}

impl ThemeContext {
    /// Captures the current application theme for a visual tree.
    pub fn application_default() -> Self {
        Self {
            theme: application_theme(),
        }
    }

    /// Creates a context backed by an explicit Window theme.
    pub fn new(theme: ThemeHandle) -> Self {
        Self { theme }
    }

    /// Returns the inherited theme handle.
    pub fn theme(&self) -> ThemeHandle {
        self.theme.clone()
    }

    /// Returns the inherited theme revision.
    pub fn revision(&self) -> u64 {
        self.theme.revision()
    }
}

/// The platform-backed base theme. Every standard token resolves to `PlatformDefault`.
pub struct SystemTheme;

/// The only variant of [`SystemTheme`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemThemeVariant {
    /// Use all active backend defaults.
    Default,
}

struct SystemThemeDefinition;

impl ThemeDefinition for SystemThemeDefinition {
    fn resolve_erased(&self, _token: &str) -> Option<ErasedThemeValue> {
        Some(ErasedThemeValue::PlatformDefault)
    }

    fn variant_name(&self) -> &'static str {
        "Default"
    }
}

impl ThemeFactory for SystemTheme {
    type Variant = SystemThemeVariant;

    fn create_definition(_variant: &Self::Variant) -> Rc<dyn ThemeDefinition> {
        Rc::new(SystemThemeDefinition)
    }
}

impl SystemTheme {
    /// Creates a controller whose tokens all resolve to platform defaults.
    pub fn controller(initial_variant: SystemThemeVariant) -> ThemeController<Self> {
        ThemeController::new(initial_variant)
    }
}

macro_rules! standard_token {
    ($name:ident, $ty:ty, $impact:ident) => {
        #[doc = concat!("The standard `", stringify!($name), "` theme token.")]
        #[allow(non_upper_case_globals)]
        pub const $name: ThemeToken<$ty> =
            ThemeToken::new(stringify!($name), ThemeChangeImpact::$impact, true);
    };
}

impl SystemTheme {
    standard_token!(window_background, Brush, Paint);
    standard_token!(layout_background, Brush, Paint);
    standard_token!(layout_spacing, f32, Measure);

    standard_token!(control_background, Brush, NativeStyle);
    standard_token!(control_foreground, Brush, NativeStyle);
    standard_token!(control_border, Brush, NativeStyle);
    standard_token!(control_padding, f32, Measure);
    standard_token!(control_corner_radius, f32, Measure);
    standard_token!(control_font_family, FontFamily, Measure);
    standard_token!(control_font_size, f32, Measure);
    standard_token!(control_font_weight, FontWeight, Measure);
    standard_token!(control_font_style, FontStyle, Measure);
    standard_token!(control_font_stretch, FontStretch, Measure);
    standard_token!(control_character_spacing, i32, Measure);

    standard_token!(native_control_background, Brush, NativeStyle);
    standard_token!(native_control_foreground, Brush, NativeStyle);
    standard_token!(native_control_border, Brush, NativeStyle);
    standard_token!(native_control_focus_width, f32, NativeStyle);
    standard_token!(native_control_font_family, FontFamily, Measure);
    standard_token!(native_control_font_size, f32, Measure);
    standard_token!(native_control_font_weight, FontWeight, Measure);
    standard_token!(native_control_font_style, FontStyle, Measure);
    standard_token!(native_control_font_stretch, FontStretch, Measure);
    standard_token!(native_control_character_spacing, i32, Measure);

    standard_token!(text_block_foreground, Brush, Paint);
    standard_token!(text_block_font_family, FontFamily, Measure);
    standard_token!(text_block_font_size, f32, Measure);
    standard_token!(text_block_font_weight, FontWeight, Measure);
    standard_token!(text_block_font_style, FontStyle, Measure);
    standard_token!(text_block_font_stretch, FontStretch, Measure);
    standard_token!(text_block_character_spacing, i32, Measure);

    standard_token!(shape_fill, Brush, Paint);
    standard_token!(shape_stroke, Brush, Paint);
    standard_token!(shape_stroke_width, f32, Paint);
    standard_token!(rectangle_corner_radius, f32, Paint);

    standard_token!(button_background, Brush, NativeStyle);
    standard_token!(button_foreground, Brush, NativeStyle);
    standard_token!(button_border, Brush, NativeStyle);
    standard_token!(button_hover_background, Brush, NativeStyle);
    standard_token!(button_hover_foreground, Brush, NativeStyle);
    standard_token!(button_pressed_background, Brush, NativeStyle);
    standard_token!(button_pressed_foreground, Brush, NativeStyle);
    standard_token!(button_disabled_background, Brush, NativeStyle);
    standard_token!(button_disabled_foreground, Brush, NativeStyle);

    standard_token!(text_box_background, Brush, NativeStyle);
    standard_token!(text_box_foreground, Brush, NativeStyle);
    standard_token!(text_box_border, Brush, NativeStyle);
    standard_token!(text_box_placeholder_foreground, Brush, NativeStyle);
    standard_token!(text_box_selection_background, Brush, NativeStyle);
    standard_token!(text_box_caret, Brush, NativeStyle);
    standard_token!(text_box_focus_border, Brush, NativeStyle);

    standard_token!(password_box_background, Brush, NativeStyle);
    standard_token!(password_box_foreground, Brush, NativeStyle);
    standard_token!(password_box_border, Brush, NativeStyle);
    standard_token!(password_box_placeholder_foreground, Brush, NativeStyle);
    standard_token!(password_box_selection_background, Brush, NativeStyle);
    standard_token!(password_box_caret, Brush, NativeStyle);
    standard_token!(password_box_focus_border, Brush, NativeStyle);

    standard_token!(text_area_background, Brush, NativeStyle);
    standard_token!(text_area_foreground, Brush, NativeStyle);
    standard_token!(text_area_border, Brush, NativeStyle);
    standard_token!(text_area_placeholder_foreground, Brush, NativeStyle);
    standard_token!(text_area_selection_background, Brush, NativeStyle);
    standard_token!(text_area_caret, Brush, NativeStyle);
    standard_token!(text_area_focus_border, Brush, NativeStyle);

    standard_token!(scroll_view_background, Brush, NativeStyle);
    standard_token!(scroll_view_scrollbar_background, Brush, NativeStyle);
    standard_token!(scroll_view_scrollbar_thumb, Brush, NativeStyle);
    standard_token!(scroll_view_scrollbar_hover_thumb, Brush, NativeStyle);

    standard_token!(menu_bar_background, Brush, NativeStyle);
    standard_token!(menu_bar_foreground, Brush, NativeStyle);
    standard_token!(menu_background, Brush, NativeStyle);
    standard_token!(menu_foreground, Brush, NativeStyle);
    standard_token!(menu_item_background, Brush, NativeStyle);
    standard_token!(menu_item_foreground, Brush, NativeStyle);
    standard_token!(menu_item_selected_background, Brush, NativeStyle);
    standard_token!(menu_item_selected_foreground, Brush, NativeStyle);
    standard_token!(menu_item_disabled_foreground, Brush, NativeStyle);

    standard_token!(tab_view_background, Brush, NativeStyle);
    standard_token!(tab_view_foreground, Brush, NativeStyle);
    standard_token!(tab_view_item_background, Brush, NativeStyle);
    standard_token!(tab_view_item_foreground, Brush, NativeStyle);
    standard_token!(tab_view_item_selected_background, Brush, NativeStyle);
    standard_token!(tab_view_item_selected_foreground, Brush, NativeStyle);
    standard_token!(tab_view_item_hover_background, Brush, NativeStyle);
    standard_token!(tab_view_item_disabled_foreground, Brush, NativeStyle);
    standard_token!(tab_view_item_close_button_background, Brush, NativeStyle);
    standard_token!(tab_view_item_close_button_foreground, Brush, NativeStyle);
}

/// Used by the attribute frontend to decide whether an ident overrides an ElwindUI standard token
/// or declares an application-owned token.
pub const STANDARD_THEME_TOKEN_NAMES: &[&str] = &[
    "window_background",
    "layout_background",
    "layout_spacing",
    "control_background",
    "control_foreground",
    "control_border",
    "control_padding",
    "control_corner_radius",
    "control_font_family",
    "control_font_size",
    "control_font_weight",
    "control_font_style",
    "control_font_stretch",
    "control_character_spacing",
    "native_control_background",
    "native_control_foreground",
    "native_control_border",
    "native_control_focus_width",
    "native_control_font_family",
    "native_control_font_size",
    "native_control_font_weight",
    "native_control_font_style",
    "native_control_font_stretch",
    "native_control_character_spacing",
    "text_block_foreground",
    "text_block_font_family",
    "text_block_font_size",
    "text_block_font_weight",
    "text_block_font_style",
    "text_block_font_stretch",
    "text_block_character_spacing",
    "shape_fill",
    "shape_stroke",
    "shape_stroke_width",
    "rectangle_corner_radius",
    "button_background",
    "button_foreground",
    "button_border",
    "button_hover_background",
    "button_hover_foreground",
    "button_pressed_background",
    "button_pressed_foreground",
    "button_disabled_background",
    "button_disabled_foreground",
    "text_box_background",
    "text_box_foreground",
    "text_box_border",
    "text_box_placeholder_foreground",
    "text_box_selection_background",
    "text_box_caret",
    "text_box_focus_border",
    "password_box_background",
    "password_box_foreground",
    "password_box_border",
    "password_box_placeholder_foreground",
    "password_box_selection_background",
    "password_box_caret",
    "password_box_focus_border",
    "text_area_background",
    "text_area_foreground",
    "text_area_border",
    "text_area_placeholder_foreground",
    "text_area_selection_background",
    "text_area_caret",
    "text_area_focus_border",
    "scroll_view_background",
    "scroll_view_scrollbar_background",
    "scroll_view_scrollbar_thumb",
    "scroll_view_scrollbar_hover_thumb",
    "menu_bar_background",
    "menu_bar_foreground",
    "menu_background",
    "menu_foreground",
    "menu_item_background",
    "menu_item_foreground",
    "menu_item_selected_background",
    "menu_item_selected_foreground",
    "menu_item_disabled_foreground",
    "tab_view_background",
    "tab_view_foreground",
    "tab_view_item_background",
    "tab_view_item_foreground",
    "tab_view_item_selected_background",
    "tab_view_item_selected_foreground",
    "tab_view_item_hover_background",
    "tab_view_item_disabled_foreground",
    "tab_view_item_close_button_background",
    "tab_view_item_close_button_foreground",
];

#[cfg(test)]
mod tests {
    use super::*;

    struct TestTheme;
    #[derive(Clone, PartialEq)]
    enum Variant {
        Default,
        Ocean,
    }
    struct TestDefinition(Variant);

    impl ThemeDefinition for TestDefinition {
        fn resolve_erased(&self, token: &str) -> Option<ErasedThemeValue> {
            match (token, &self.0) {
                ("brand", Variant::Ocean) => Some(erase_theme_value(ThemeValue::Value(
                    Brush::Solid(crate::graphics::Color::rgb(0, 166, 200)),
                ))),
                ("brand", Variant::Default) => Some(ErasedThemeValue::PlatformDefault),
                _ => None,
            }
        }

        fn variant_name(&self) -> &'static str {
            match self.0 {
                Variant::Default => "Default",
                Variant::Ocean => "Ocean",
            }
        }
    }

    impl ThemeFactory for TestTheme {
        type Variant = Variant;

        fn create_definition(variant: &Self::Variant) -> Rc<dyn ThemeDefinition> {
            Rc::new(TestDefinition(variant.clone()))
        }

        fn change_impact(_previous: &Self::Variant, _next: &Self::Variant) -> ThemeChangeImpact {
            ThemeChangeImpact::Paint
        }
    }

    #[test]
    fn controller_keeps_platform_default_distinct_from_a_value() {
        let controller = ThemeController::<TestTheme>::new(Variant::Default);
        let brand = ThemeToken::<Brush>::new("brand", ThemeChangeImpact::Paint, false);
        assert_eq!(
            controller.handle().resolve(brand),
            ThemeValue::PlatformDefault
        );

        controller.set_variant(Variant::Ocean);
        assert!(matches!(
            controller.handle().resolve(brand),
            ThemeValue::Value(Brush::Solid(_))
        ));
        assert_eq!(controller.revision(), 1);
        assert_eq!(
            controller.handle().last_change_impact(),
            ThemeChangeImpact::Paint
        );
    }

    #[test]
    fn system_theme_never_materializes_platform_defaults() {
        let handle = SystemTheme::controller(SystemThemeVariant::Default).handle();
        assert_eq!(
            handle.resolve(SystemTheme::button_background),
            ThemeValue::PlatformDefault
        );
    }

    struct BaseFallbackDefinition {
        button_is_platform_default: bool,
    }

    impl ThemeDefinition for BaseFallbackDefinition {
        fn resolve_erased(&self, token: &str) -> Option<ErasedThemeValue> {
            match token {
                "button_background" if self.button_is_platform_default => {
                    Some(ErasedThemeValue::PlatformDefault)
                }
                "control_background" => Some(erase_theme_value(ThemeValue::Value(Brush::Solid(
                    crate::graphics::Color::rgb(1, 2, 3),
                )))),
                _ => None,
            }
        }

        fn variant_name(&self) -> &'static str {
            "Fallback"
        }
    }

    #[test]
    fn missing_concrete_token_falls_back_but_explicit_platform_default_stops() {
        let inherited = ThemeHandle::new(
            Rc::new(BaseFallbackDefinition {
                button_is_platform_default: false,
            }),
            ThemePreference::System,
            ThemeAppearance::Light,
        );
        assert!(matches!(
            inherited.resolve(SystemTheme::button_background),
            ThemeValue::Value(Brush::Solid(_))
        ));

        let cleared = ThemeHandle::new(
            Rc::new(BaseFallbackDefinition {
                button_is_platform_default: true,
            }),
            ThemePreference::System,
            ThemeAppearance::Light,
        );
        assert_eq!(
            cleared.resolve(SystemTheme::button_background),
            ThemeValue::PlatformDefault
        );
    }

    #[test]
    fn manifest_uses_elwindui_type_names() {
        assert!(STANDARD_THEME_TOKEN_NAMES.contains(&"layout_background"));
        assert!(!STANDARD_THEME_TOKEN_NAMES.contains(&"panel_background"));
        assert!(!STANDARD_THEME_TOKEN_NAMES.contains(&"surface_background"));
        assert!(!STANDARD_THEME_TOKEN_NAMES.contains(&"input_background"));
    }
}
