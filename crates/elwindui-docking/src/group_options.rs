use crate::{DockItemId, TabStripPosition};

/// The Document a runtime-created group was created for, passed to
/// [`crate::DockingControl::set_on_group_created`].
#[derive(Clone, Debug, PartialEq)]
pub struct DockGroupCreatedArgs {
    /// The Document whose drop, float or unpin created the group (its first item).
    pub item: DockItemId,
}

/// Presentation of a group the runtime creates, chosen by the application like WinUI.Dock's
/// `IDockAdapter.OnCreated(DocumentGroup, Document)`. Authored groups use their own
/// `DockGroup` properties instead. The choice is runtime state only: it never enters
/// `DockLayoutSnapshot`, and a restored generated group asks the hook again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockGroupOptions {
    pub tab_strip_position: TabStripPosition,
    pub compact_tabs: bool,
}

impl Default for DockGroupOptions {
    /// Top tabs with equal widths: the presentation of a generated group when no hook is set.
    fn default() -> Self {
        Self {
            tab_strip_position: TabStripPosition::Top,
            compact_tabs: false,
        }
    }
}
