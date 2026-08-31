//! CustomSplitter realization for weighted split nodes.

use crate::DockLayoutModel;

/// Stores transient splitter previews without writing them back on every pointer delta.
#[allow(dead_code)]
pub(crate) struct SplitterSession {
    original: DockLayoutModel,
    transient: DockLayoutModel,
    captured: bool,
}

#[allow(dead_code)]
impl SplitterSession {
    pub(crate) fn begin(model: &DockLayoutModel) -> Self {
        Self {
            original: model.clone(),
            transient: model.clone(),
            captured: true,
        }
    }

    pub(crate) fn preview(&mut self, model: DockLayoutModel) {
        if self.captured {
            self.transient = model;
        }
    }

    pub(crate) fn cancel(&mut self) -> DockLayoutModel {
        self.captured = false;
        self.original.clone()
    }

    pub(crate) fn commit(&mut self) -> Option<DockLayoutModel> {
        if !self.captured {
            return None;
        }
        self.captured = false;
        Some(self.transient.clone())
    }
}
