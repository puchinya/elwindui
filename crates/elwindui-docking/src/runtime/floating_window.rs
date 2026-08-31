//! Private target-specific floating-host integration point.

use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FloatingHostId(u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FloatingHostState {
    pub(crate) id: FloatingHostId,
    pub(crate) bounds: Rect,
}

#[derive(Default)]
pub(crate) struct FloatingHostRegistry {
    next_id: u64,
    hosts: Vec<FloatingHostState>,
}

impl FloatingHostRegistry {
    pub(crate) fn create(&mut self, bounds: Rect) -> FloatingHostState {
        self.next_id = self.next_id.max(1);
        let state = FloatingHostState {
            id: FloatingHostId(self.next_id),
            bounds,
        };
        self.next_id = self.next_id.saturating_add(1);
        self.hosts.push(state);
        state
    }

    pub(crate) fn sync(&mut self, bounds: &[Rect]) {
        self.hosts.truncate(bounds.len());
        for (host, bounds) in self.hosts.iter_mut().zip(bounds.iter().copied()) {
            host.bounds = bounds;
        }
        while self.hosts.len() < bounds.len() {
            let index = self.hosts.len();
            self.create(bounds[index]);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn remove(&mut self, id: FloatingHostId) -> bool {
        let before = self.hosts.len();
        self.hosts.retain(|host| host.id != id);
        before != self.hosts.len()
    }

    pub(crate) fn close_empty(&mut self) {
        self.hosts.clear();
    }
}
