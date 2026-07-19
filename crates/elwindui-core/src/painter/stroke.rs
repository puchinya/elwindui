use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrokeError;

impl fmt::Display for StrokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stroke width, miter limit, and dash values must be finite and non-negative"
        )
    }
}
impl std::error::Error for StrokeError {}

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub start_cap: LineCap,
    pub end_cap: LineCap,
    pub dash_cap: LineCap,
    pub line_join: LineJoin,
    pub miter_limit: f32,
    /// Dash lengths in logical coordinate units — a backend-independent absolute length, not a
    /// multiple of the stroke width (painter design doc §7).
    pub dash_pattern: Arc<[f32]>,
    pub dash_offset: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 1.0,
            start_cap: LineCap::Butt,
            end_cap: LineCap::Butt,
            dash_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 10.0,
            dash_pattern: Arc::from([]),
            dash_offset: 0.0,
        }
    }
}

impl StrokeStyle {
    /// Validates `width`/`miter_limit`/`dash_offset`/every `dash_pattern` entry are finite and
    /// non-negative. Called explicitly rather than at construction time, since `StrokeStyle` is a
    /// plain data struct built with struct-update syntax (`StrokeStyle { width: 2.0, ..Default
    /// ::default() }`) throughout the painter API (see design doc §17's usage example).
    pub fn validate(&self) -> Result<(), StrokeError> {
        let finite_non_negative = |v: f32| v.is_finite() && v >= 0.0;
        if !finite_non_negative(self.width)
            || !finite_non_negative(self.miter_limit)
            || !finite_non_negative(self.dash_offset)
        {
            return Err(StrokeError);
        }
        if self.dash_pattern.iter().any(|&d| !finite_non_negative(d)) {
            return Err(StrokeError);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_documented_values() {
        let s = StrokeStyle::default();
        assert_eq!(s.width, 1.0);
        assert_eq!(s.start_cap, LineCap::Butt);
        assert_eq!(s.line_join, LineJoin::Miter);
        assert_eq!(s.miter_limit, 10.0);
        assert!(s.dash_pattern.is_empty());
        assert!(s.validate().is_ok());
    }

    #[test]
    fn negative_width_is_rejected() {
        let s = StrokeStyle {
            width: -1.0,
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn nan_dash_entry_is_rejected() {
        let s = StrokeStyle {
            dash_pattern: Arc::from([1.0, f32::NAN]),
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn infinite_dash_entry_is_rejected() {
        let s = StrokeStyle {
            dash_pattern: Arc::from([f32::INFINITY]),
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }
}
