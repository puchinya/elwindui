use crate::{DockGroupId, DockItemId};
use elwindui_core::base::Rect;
use std::fmt;

/// A side of a dock surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockSide {
    /// The leading horizontal side.
    Left,
    /// The leading vertical side.
    Top,
    /// The trailing horizontal side.
    Right,
    /// The trailing vertical side.
    Bottom,
}

impl DockSide {
    pub(crate) const ALL: [Self; 4] = [Self::Left, Self::Top, Self::Right, Self::Bottom];

    pub(crate) fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Top => 1,
            Self::Right => 2,
            Self::Bottom => 3,
        }
    }
}

/// Target kind reported by interactive docking hit testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockTarget {
    /// Insert into the target group.
    Center,
    /// Split the target group on its left side.
    SplitLeft,
    /// Split the target group on its top side.
    SplitTop,
    /// Split the target group on its right side.
    SplitRight,
    /// Split the target group on its bottom side.
    SplitBottom,
    /// Dock at the root's left edge.
    DockLeft,
    /// Dock at the root's top edge.
    DockTop,
    /// Dock at the root's right edge.
    DockRight,
    /// Dock at the root's bottom edge.
    DockBottom,
}

/// Programmatic transformation of a dock layout.
#[derive(Clone, Debug, PartialEq)]
pub enum DockPlacement {
    /// Inserts into an authored group at an optional tab index.
    Group {
        group: DockGroupId,
        index: Option<usize>,
    },
    /// Creates a generated group beside an authored group.
    SplitGroup {
        group: DockGroupId,
        side: DockSide,
        weight: f32,
    },
    /// Creates a generated group beside the main root.
    RootEdge { side: DockSide, weight: f32 },
    /// Creates a floating root with the supplied logical desktop bounds.
    Floating { bounds: Rect },
    /// Places the item in an auto-hide strip.
    AutoHide { side: DockSide },
}

/// Typed failures from model transformations and snapshot restore.
#[derive(Clone, Debug, PartialEq)]
pub enum DockLayoutError {
    /// The requested item is not registered or present.
    UnknownItem(DockItemId),
    /// The requested authored group is not present.
    UnknownGroup(DockGroupId),
    /// A split or placement weight is invalid.
    InvalidWeight,
    /// Floating bounds are invalid.
    InvalidBounds,
    /// The model has no authored default to reset to.
    DefaultLayoutUnavailable,
    /// The snapshot version is newer than this crate understands.
    UnknownSnapshotVersion { version: u32 },
    /// The snapshot or authored runtime state violates a structural invariant.
    InvalidSnapshot { reason: String },
    /// A platform floating host could not be created or hosted.
    FloatingHostUnavailable { reason: String },
    /// An internal interactive placement addressed a floating root that no longer exists.
    InvalidFloatingRoot { index: usize },
}

impl fmt::Display for DockLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownItem(id) => write!(f, "unknown dock item {id}"),
            Self::UnknownGroup(id) => write!(f, "unknown dock group {id}"),
            Self::InvalidWeight => f.write_str("dock weight must be finite and positive"),
            Self::InvalidBounds => f.write_str("floating bounds must be finite and positive"),
            Self::DefaultLayoutUnavailable => {
                f.write_str("no authored default dock layout is attached")
            }
            Self::UnknownSnapshotVersion { version } => {
                write!(f, "unknown dock snapshot version {version}")
            }
            Self::InvalidSnapshot { reason } => write!(f, "invalid dock snapshot: {reason}"),
            Self::FloatingHostUnavailable { reason } => {
                write!(f, "floating dock host unavailable: {reason}")
            }
            Self::InvalidFloatingRoot { index } => {
                write!(f, "invalid floating dock root index {index}")
            }
        }
    }
}

impl std::error::Error for DockLayoutError {}

pub(crate) fn valid_weight(weight: f32) -> bool {
    weight.is_finite() && weight > 0.0
}

pub(crate) fn valid_bounds(bounds: Rect) -> bool {
    bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width > 0.0
        && bounds.height > 0.0
}
