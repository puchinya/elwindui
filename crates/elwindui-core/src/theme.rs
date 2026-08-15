//! Theme-as-Preset-over-Environment (`docs/specs/theme_environment_spec.md` §3–§6, Issue #96).
//!
//! A Theme is not a second resolution mechanism alongside Environment ([`crate::environment`]) — it
//! is a batch of [`crate::environment::EnvironmentContext::set`] calls. See
//! `docs/design/runtime/theme_environment_design.md` (`## Theme`) for the full rationale, including
//! why there is no separate `EnvironmentOverrides` type and why applying a Theme is scoped to
//! [`crate::environment::EnvironmentContext::application_environment`] only (no per-Window override
//! in this iteration).

use crate::environment::{EnvironmentContext, EnvironmentKey};
use crate::graphics::{Brush, Color};
use crate::reactive::Subscription;
use std::rc::Rc;

/// The result of resolving a semantic value without materializing a toolkit-owned default.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedValue<T> {
    /// A concrete value that may be assigned to the target property.
    Value(T),
    /// The target property must use its backend, inherited, transparent, or no-paint default.
    PlatformDefault,
}

/// A concrete brush or a semantic brush role resolved through [`EnvironmentContext`].
#[derive(Debug, Clone, PartialEq)]
pub enum BrushStyle {
    /// Uses the contained concrete brush without consulting Environment.
    Value(Brush),
    /// Resolves the framework's primary semantic brush.
    Primary,
    /// Resolves the framework's secondary semantic brush.
    Secondary,
    /// Resolves the framework's tertiary semantic brush.
    Tertiary,
    /// Resolves the ordinary foreground semantic brush.
    Foreground,
    /// Resolves the ordinary background semantic brush.
    Background,
    /// Resolves the top-level window background semantic brush.
    WindowBackground,
    /// Resolves the application accent or tint semantic brush.
    Tint,
    /// Resolves the selection semantic brush.
    Selection,
    /// Resolves the separator semantic brush.
    Separator,
    /// Resolves the placeholder-content semantic brush.
    Placeholder,
    /// Resolves the link semantic brush.
    Link,
    /// Leaves the target property's concrete value to its backend or property default.
    PlatformDefault,
}

macro_rules! semantic_brush_environment_key {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        pub struct $name;

        impl EnvironmentKey for $name {
            type Value = BrushStyle;

            fn default_value() -> Self::Value {
                BrushStyle::PlatformDefault
            }
        }
    };
}

semantic_brush_environment_key!(
    PrimaryBrushEnvironment,
    "Environment Key for [`BrushStyle::Primary`]."
);
semantic_brush_environment_key!(
    SecondaryBrushEnvironment,
    "Environment Key for [`BrushStyle::Secondary`]."
);
semantic_brush_environment_key!(
    TertiaryBrushEnvironment,
    "Environment Key for [`BrushStyle::Tertiary`]."
);
semantic_brush_environment_key!(
    ForegroundBrushEnvironment,
    "Environment Key for [`BrushStyle::Foreground`]."
);
semantic_brush_environment_key!(
    BackgroundBrushEnvironment,
    "Environment Key for [`BrushStyle::Background`]."
);
semantic_brush_environment_key!(
    WindowBackgroundBrushEnvironment,
    "Environment Key for [`BrushStyle::WindowBackground`]."
);
semantic_brush_environment_key!(
    TintBrushEnvironment,
    "Environment Key for [`BrushStyle::Tint`]."
);
semantic_brush_environment_key!(
    SelectionBrushEnvironment,
    "Environment Key for [`BrushStyle::Selection`]."
);
semantic_brush_environment_key!(
    SeparatorBrushEnvironment,
    "Environment Key for [`BrushStyle::Separator`]."
);
semantic_brush_environment_key!(
    PlaceholderBrushEnvironment,
    "Environment Key for [`BrushStyle::Placeholder`]."
);
semantic_brush_environment_key!(
    LinkBrushEnvironment,
    "Environment Key for [`BrushStyle::Link`]."
);

impl BrushStyle {
    /// Resolves this style against the explicitly supplied effective Environment.
    ///
    /// Semantic aliases may reference other semantic roles. A cycle safely resolves to
    /// [`ResolvedValue::PlatformDefault`] instead of panicking or recursing indefinitely.
    pub fn resolve(&self, environment: &EnvironmentContext) -> ResolvedValue<Brush> {
        self.resolve_with_seen(environment, 0)
    }

    fn resolve_with_seen(
        &self,
        environment: &EnvironmentContext,
        seen: u16,
    ) -> ResolvedValue<Brush> {
        match self {
            Self::Value(brush) => ResolvedValue::Value(brush.clone()),
            Self::PlatformDefault => ResolvedValue::PlatformDefault,
            Self::Primary => resolve_role::<PrimaryBrushEnvironment>(environment, seen, 1 << 0),
            Self::Secondary => resolve_role::<SecondaryBrushEnvironment>(environment, seen, 1 << 1),
            Self::Tertiary => resolve_role::<TertiaryBrushEnvironment>(environment, seen, 1 << 2),
            Self::Foreground => {
                resolve_role::<ForegroundBrushEnvironment>(environment, seen, 1 << 3)
            }
            Self::Background => {
                resolve_role::<BackgroundBrushEnvironment>(environment, seen, 1 << 4)
            }
            Self::WindowBackground => {
                resolve_role::<WindowBackgroundBrushEnvironment>(environment, seen, 1 << 5)
            }
            Self::Tint => resolve_role::<TintBrushEnvironment>(environment, seen, 1 << 6),
            Self::Selection => resolve_role::<SelectionBrushEnvironment>(environment, seen, 1 << 7),
            Self::Separator => resolve_role::<SeparatorBrushEnvironment>(environment, seen, 1 << 8),
            Self::Placeholder => {
                resolve_role::<PlaceholderBrushEnvironment>(environment, seen, 1 << 9)
            }
            Self::Link => resolve_role::<LinkBrushEnvironment>(environment, seen, 1 << 10),
        }
    }
}

fn resolve_role<K>(environment: &EnvironmentContext, seen: u16, role: u16) -> ResolvedValue<Brush>
where
    K: EnvironmentKey<Value = BrushStyle>,
{
    if seen & role != 0 {
        return ResolvedValue::PlatformDefault;
    }
    environment
        .get::<K>()
        .resolve_with_seen(environment, seen | role)
}

impl From<Brush> for BrushStyle {
    fn from(value: Brush) -> Self {
        Self::Value(value)
    }
}

impl From<Color> for BrushStyle {
    fn from(value: Color) -> Self {
        Self::Value(value.into())
    }
}

impl From<&str> for BrushStyle {
    fn from(value: &str) -> Self {
        Self::Value(value.into())
    }
}

impl From<String> for BrushStyle {
    fn from(value: String) -> Self {
        Self::Value(value.as_str().into())
    }
}

/// Subscribes one listener to every framework semantic-brush Environment Key.
///
/// This is public only for generated DSL code. Dropping the returned subscriptions unregisters
/// every listener.
#[doc(hidden)]
pub fn subscribe_semantic_brushes(
    environment: &EnvironmentContext,
    listener: Rc<dyn Fn()>,
) -> Vec<Subscription> {
    macro_rules! subscribe {
        ($key:ty) => {{
            let listener = listener.clone();
            environment.subscribe::<$key>(move || listener())
        }};
    }
    vec![
        subscribe!(PrimaryBrushEnvironment),
        subscribe!(SecondaryBrushEnvironment),
        subscribe!(TertiaryBrushEnvironment),
        subscribe!(ForegroundBrushEnvironment),
        subscribe!(BackgroundBrushEnvironment),
        subscribe!(WindowBackgroundBrushEnvironment),
        subscribe!(TintBrushEnvironment),
        subscribe!(SelectionBrushEnvironment),
        subscribe!(SeparatorBrushEnvironment),
        subscribe!(PlaceholderBrushEnvironment),
        subscribe!(LinkBrushEnvironment),
    ]
}

/// Implemented by code generated from `#[elwindui::theme]`. Applying a Theme overrides whichever
/// Environment Keys its `#[theme(value = ..)]` fields target — `docs/specs/theme_environment_spec.md`
/// §3/§4.
pub trait Theme {
    /// Overrides this Theme's Environment values on `env`.
    fn apply(&self, env: &EnvironmentContext);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush(r: u8, g: u8, b: u8) -> Brush {
        Brush::Solid(Color::rgb(r, g, b))
    }

    #[test]
    fn concrete_brush_resolves_without_environment_materialization() {
        let expected = brush(1, 2, 3);
        assert_eq!(
            BrushStyle::Value(expected.clone()).resolve(&EnvironmentContext::root()),
            ResolvedValue::Value(expected)
        );
    }

    #[test]
    fn unset_semantic_role_resolves_to_platform_default() {
        assert_eq!(
            BrushStyle::Primary.resolve(&EnvironmentContext::root()),
            ResolvedValue::PlatformDefault
        );
    }

    #[test]
    fn semantic_alias_chain_resolves_to_concrete_brush() {
        let environment = EnvironmentContext::root();
        let expected = brush(4, 5, 6);
        environment.set::<PrimaryBrushEnvironment>(BrushStyle::Secondary);
        environment.set::<SecondaryBrushEnvironment>(BrushStyle::Value(expected.clone()));
        assert_eq!(
            BrushStyle::Primary.resolve(&environment),
            ResolvedValue::Value(expected)
        );
    }

    #[test]
    fn semantic_alias_cycle_resolves_to_platform_default() {
        let environment = EnvironmentContext::root();
        environment.set::<PrimaryBrushEnvironment>(BrushStyle::Secondary);
        environment.set::<SecondaryBrushEnvironment>(BrushStyle::Primary);
        assert_eq!(
            BrushStyle::Primary.resolve(&environment),
            ResolvedValue::PlatformDefault
        );
    }
}
