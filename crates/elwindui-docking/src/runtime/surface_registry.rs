//! Weak surface registry used for cross-window target discovery.

use crate::core::base::Point;
use crate::core::ui::UIElementExt;
use std::rc::{Rc, Weak};

#[allow(dead_code)]
#[derive(Default)]
pub(crate) struct SurfaceRegistry {
    surfaces: Vec<Weak<dyn UIElementExt>>,
}

#[allow(dead_code)]
impl SurfaceRegistry {
    pub(crate) fn register(&mut self, surface: &Rc<dyn UIElementExt>) {
        self.surfaces.push(Rc::downgrade(surface));
        self.compact();
    }

    pub(crate) fn compact(&mut self) {
        self.surfaces.retain(|surface| surface.strong_count() > 0);
    }

    /// Coordinate conversion is the only cross-window discovery capability used by docking.
    pub(crate) fn roots_for_screen_point(&self, point: Point) -> Vec<Rc<dyn UIElementExt>> {
        self.surfaces
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|surface| surface.screen_to_root(point).is_some())
            .collect()
    }
}
