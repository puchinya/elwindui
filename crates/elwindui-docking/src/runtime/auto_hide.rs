//! Auto-hide strip and single-open overlay state.

use crate::DockItemId;

/// Only one auto-hide item owns the open overlay at a time.
#[derive(Default)]
pub(crate) struct AutoHideOverlay {
    open: Option<DockItemId>,
}

impl AutoHideOverlay {
    pub(crate) fn open(&mut self, item: DockItemId) -> Option<DockItemId> {
        self.open.replace(item)
    }

    pub(crate) fn close(&mut self) -> Option<DockItemId> {
        self.open.take()
    }

    #[cfg(test)]
    pub(crate) fn current(&self) -> Option<&DockItemId> {
        self.open.as_ref()
    }
}
