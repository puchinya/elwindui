//! Weak surface registry used for cross-window target discovery.

use crate::core::base::{Point, Rect};
use crate::core::ui::UIElementExt;
use std::rc::{Rc, Weak};

#[derive(Default)]
pub(crate) struct SurfaceRegistry {
    surfaces: Vec<Weak<dyn UIElementExt>>,
}

impl SurfaceRegistry {
    pub(crate) fn register(&mut self, surface: &Rc<dyn UIElementExt>) {
        self.compact();
        if self
            .surfaces
            .iter()
            .filter_map(Weak::upgrade)
            .any(|existing| Rc::ptr_eq(&existing, surface))
        {
            return;
        }
        self.surfaces.push(Rc::downgrade(surface));
    }

    pub(crate) fn unregister(&mut self, surface: &Rc<dyn UIElementExt>) {
        self.surfaces.retain(|candidate| {
            candidate
                .upgrade()
                .is_some_and(|existing| !Rc::ptr_eq(&existing, surface))
        });
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

    pub(crate) fn all_surfaces(&self) -> Vec<Rc<dyn UIElementExt>> {
        self.surfaces.iter().filter_map(Weak::upgrade).collect()
    }

    pub(crate) fn bounds_in_surface_root(
        element: &Rc<dyn UIElementExt>,
        surface: &Rc<dyn UIElementExt>,
    ) -> Option<Rect> {
        let width = element.arranged_width()?;
        let height = element.arranged_height()?;
        if !width.is_finite() || !height.is_finite() || width < 0.0 || height < 0.0 {
            return None;
        }
        let mut x = 0.0;
        let mut y = 0.0;
        let mut current = element.clone();
        loop {
            if Rc::ptr_eq(&current, surface) {
                return Some(Rect {
                    x,
                    y,
                    width,
                    height,
                });
            }
            let offset = current.arranged_offset()?;
            if !offset.x.is_finite() || !offset.y.is_finite() {
                return None;
            }
            x += offset.x;
            y += offset.y;
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            current = current.visual_parent()?;
        }
    }

    pub(crate) fn surface_bounds(surface: &Rc<dyn UIElementExt>) -> Option<Rect> {
        let width = surface.arranged_width()?;
        let height = surface.arranged_height()?;
        (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0).then_some(Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        })
    }
}
