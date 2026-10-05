/// Authored pixel size of a [`crate::DockGroup`] or [`crate::DockSplitPanel`] inside its parent
/// split, mirroring WinUI.Dock's module `Width`/`Height`/min/max (`docs/specs/docking_spec.md`).
///
/// A set `width`/`height` makes that node a fixed-size track along the parent split's axis;
/// unset extents stay proportional (`weight`). Min/max bound the track either way. A split
/// panel's size applies to its descendants across the panel's own axis. Sizes are authored
/// layout input only: they never enter `DockLayoutSnapshot`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DockSize {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
}

impl DockSize {
    /// A fixed width with no other constraint.
    pub fn width(width: f32) -> Self {
        Self {
            width: Some(width),
            ..Self::default()
        }
    }

    /// A fixed height with no other constraint.
    pub fn height(height: f32) -> Self {
        Self {
            height: Some(height),
            ..Self::default()
        }
    }

    /// Drops non-finite or negative values so they behave as unset.
    pub(crate) fn sanitized(self) -> Self {
        let finite = |value: Option<f32>| value.filter(|value| value.is_finite() && *value >= 0.0);
        Self {
            width: finite(self.width),
            height: finite(self.height),
            min_width: finite(self.min_width),
            max_width: finite(self.max_width),
            min_height: finite(self.min_height),
            max_height: finite(self.max_height),
        }
    }
}
