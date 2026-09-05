//! Weak surface registry used for cross-window target discovery.

use crate::core::base::{Point, Rect};
use crate::core::ui::UIElementExt;
use crate::model::RootKind;
use std::rc::{Rc, Weak};

pub(crate) struct RegisteredSurface {
    pub(crate) root: RootKind,
    pub(crate) surface: Weak<dyn UIElementExt>,
}

#[derive(Default)]
pub(crate) struct SurfaceRegistry {
    surfaces: Vec<RegisteredSurface>,
}

impl SurfaceRegistry {
    pub(crate) fn register(&mut self, root: RootKind, surface: &Rc<dyn UIElementExt>) {
        self.compact();
        self.surfaces.retain(|existing| existing.root != root);
        self.surfaces.push(RegisteredSurface {
            root,
            surface: Rc::downgrade(surface),
        });
    }

    pub(crate) fn unregister(&mut self, root: RootKind) {
        let surfaces: &mut Vec<RegisteredSurface> = &mut self.surfaces;
        surfaces.retain(|candidate| {
            candidate.root != root && <Weak<dyn UIElementExt>>::strong_count(&candidate.surface) > 0
        });
    }

    pub(crate) fn compact(&mut self) {
        let surfaces: &mut Vec<RegisteredSurface> = &mut self.surfaces;
        surfaces.retain(|surface| <Weak<dyn UIElementExt>>::strong_count(&surface.surface) > 0);
    }

    pub(crate) fn entries(&self) -> Vec<(RootKind, Rc<dyn UIElementExt>)> {
        let mut entries = self
            .surfaces
            .iter()
            .filter_map(|entry| {
                entry
                    .surface
                    .upgrade()
                    .map(|surface| (entry.root.clone(), surface))
            })
            .collect::<Vec<_>>();
        entries.sort_by(|(left, _), (right, _)| root_order(left).cmp(&root_order(right)));
        entries
    }

    pub(crate) fn surface_for_root(&self, root: &RootKind) -> Option<Rc<dyn UIElementExt>> {
        self.surfaces
            .iter()
            .find(|entry| &entry.root == root)
            .and_then(|entry| entry.surface.upgrade())
    }

    /// Returns an element's bounds in the hosted visual tree's root coordinate space.
    pub(crate) fn bounds_in_host_root(element: &Rc<dyn UIElementExt>) -> Option<Rect> {
        let width = element.arranged_width()?;
        let height = element.arranged_height()?;
        if !width.is_finite() || !height.is_finite() || width < 0.0 || height < 0.0 {
            return None;
        }
        let mut x = 0.0;
        let mut y = 0.0;
        let mut current = element.clone();
        loop {
            let offset = current.arranged_offset()?;
            if !offset.x.is_finite() || !offset.y.is_finite() {
                return None;
            }
            x += offset.x;
            y += offset.y;
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            let Some(parent) = current.visual_parent() else {
                return Some(Rect {
                    x,
                    y,
                    width,
                    height,
                });
            };
            current = parent;
        }
    }

    pub(crate) fn host_root_to_surface_local(
        surface: &Rc<dyn UIElementExt>,
        host_root_point: Point,
    ) -> Option<Point> {
        let origin = Self::bounds_in_host_root(surface)?;
        let point = Point {
            x: host_root_point.x - origin.x,
            y: host_root_point.y - origin.y,
        };
        (point.x.is_finite() && point.y.is_finite()).then_some(point)
    }

    pub(crate) fn bounds_in_surface_local(
        element: &Rc<dyn UIElementExt>,
        surface: &Rc<dyn UIElementExt>,
    ) -> Option<Rect> {
        let bounds = Self::bounds_in_host_root(element)?;
        let origin = Self::bounds_in_host_root(surface)?;
        let local = Rect {
            x: bounds.x - origin.x,
            y: bounds.y - origin.y,
            width: bounds.width,
            height: bounds.height,
        };
        valid_rect(local).then_some(local)
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

fn valid_rect(rect: Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0
}

fn root_order(root: &RootKind) -> (u8, usize) {
    match root {
        RootKind::Floating(index) => (0, *index),
        RootKind::Main => (1, 0),
    }
}
