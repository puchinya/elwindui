use super::core::base::Point;

/// The edge on which a tab strip is authored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabStripPosition {
    #[default]
    /// Place tabs above the content.
    Top,
    /// Place tabs below the content.
    Bottom,
}

/// Controls when an item's close affordance is presented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloseButtonPresentation {
    #[default]
    /// Always show a close affordance for closeable items.
    Always,
    /// Show a close affordance while the pointer is over the item.
    OnPointerOver,
    /// Do not show a close affordance.
    Never,
}

/// Payload emitted when a tab drag starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragStartedEventArgs {
    /// The child index at the start of the gesture.
    pub index: usize,
    /// The root-relative pointer position.
    pub position: Point,
    /// The normalized logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted while a tab drag is active.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragMovedEventArgs {
    /// The child index being dragged.
    pub index: usize,
    /// The root-relative pointer position.
    pub position: Point,
    /// The normalized logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted when a tab drag completes or is canceled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragCompletedEventArgs {
    /// The child index being dragged.
    pub index: usize,
    /// The final root-relative pointer position.
    pub position: Point,
    /// The normalized logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
    /// Whether the gesture was canceled rather than committed.
    pub canceled: bool,
}

/// Payload emitted when a closeable tab requests closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabCloseRequestedEventArgs {
    /// The child index requesting closure.
    pub index: usize,
}

/// Backwards-compatible short name for [`TabDragStartedEventArgs`].
pub type TabDragStarted = TabDragStartedEventArgs;
/// Backwards-compatible short name for [`TabDragMovedEventArgs`].
pub type TabDragMoved = TabDragMovedEventArgs;
/// Backwards-compatible short name for [`TabDragCompletedEventArgs`].
pub type TabDragCompleted = TabDragCompletedEventArgs;
/// Backwards-compatible short name for [`TabCloseRequestedEventArgs`].
pub type TabCloseRequested = TabCloseRequestedEventArgs;

/// Direction of the Grid axis resized by a `CustomGridSplitter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridResizeDirection {
    #[default]
    Auto,
    Columns,
    Rows,
}

/// Pair of Grid tracks affected by a `CustomGridSplitter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridResizeBehavior {
    #[default]
    BasedOnAlignment,
    CurrentAndNext,
    PreviousAndCurrent,
    PreviousAndNext,
}

/// Input source that created one Grid splitter transaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridSplitterInputKind {
    #[default]
    Pointer,
    Keyboard,
}

/// Payload emitted after a valid Grid splitter transaction starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSplitterResizeStartedEventArgs {
    pub direction: GridResizeDirection,
    pub target_index: usize,
    pub sibling_index: usize,
    pub input_kind: GridSplitterInputKind,
    pub position: Option<Point>,
    pub screen_position: Option<Point>,
}

/// Payload emitted after a Grid splitter preview changes the definitions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSplitterResizeDeltaEventArgs {
    pub delta: f32,
    pub cumulative_delta: f32,
    pub direction: GridResizeDirection,
    pub target_index: usize,
    pub sibling_index: usize,
    pub input_kind: GridSplitterInputKind,
    pub position: Option<Point>,
    pub screen_position: Option<Point>,
}

/// Payload emitted when a Grid splitter transaction completes or is canceled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSplitterResizeCompletedEventArgs {
    pub cumulative_delta: f32,
    pub direction: GridResizeDirection,
    pub target_index: usize,
    pub sibling_index: usize,
    pub input_kind: GridSplitterInputKind,
    pub position: Option<Point>,
    pub screen_position: Option<Point>,
    pub canceled: bool,
}
