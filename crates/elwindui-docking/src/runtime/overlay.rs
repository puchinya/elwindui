//! Dock target overlays and transient previews.

use crate::DockTarget;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DropPreview {
    target: Option<DockTarget>,
}

impl DropPreview {
    pub(crate) fn new() -> Self {
        Self { target: None }
    }

    pub(crate) fn set_target(&mut self, target: DockTarget) {
        self.target = Some(target);
    }

    #[cfg(test)]
    pub(crate) fn target(&self) -> Option<DockTarget> {
        self.target
    }

    pub(crate) fn clear(&mut self) {
        self.target = None;
    }
}
