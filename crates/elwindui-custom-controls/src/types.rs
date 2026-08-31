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

/// Payload emitted when splitter dragging starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterDragStartedEventArgs {
    /// The root-relative pointer position.
    pub position: Point,
    /// The logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted for an incremental splitter movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterDragDeltaEventArgs {
    /// Movement along the splitter's logical axis since the previous event.
    pub delta: f32,
    /// Total movement along the logical axis since the gesture began.
    pub cumulative_delta: f32,
    /// The root-relative pointer position.
    pub position: Point,
    /// The logical desktop position, when the host supplies it.
    pub screen_position: Option<Point>,
}

/// Payload emitted when splitter dragging completes or is canceled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterDragCompletedEventArgs {
    /// Total movement along the logical axis.
    pub cumulative_delta: f32,
    /// The final root-relative pointer position.
    pub position: Point,
    /// The final normalized logical desktop position, when supplied by the host.
    pub screen_position: Option<Point>,
    /// Whether the gesture was canceled rather than committed.
    pub canceled: bool,
}

/// Backwards-compatible short name for [`SplitterDragStartedEventArgs`].
pub type SplitterDragStarted = SplitterDragStartedEventArgs;
/// Backwards-compatible short name for [`SplitterDragDeltaEventArgs`].
pub type SplitterDragDelta = SplitterDragDeltaEventArgs;
/// Backwards-compatible short name for [`SplitterDragCompletedEventArgs`].
pub type SplitterDragCompleted = SplitterDragCompletedEventArgs;
