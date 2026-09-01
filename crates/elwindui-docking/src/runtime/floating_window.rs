//! Target-specific native floating-window adapters and their staged registry.

use crate::DockLayoutError;
use crate::Rect;
use crate::core::ui::{UIElementExt, WindowExt, WindowLifecycleHost};
use crate::runtime::DockSurfaceView;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FloatingHostId(u64);

pub(crate) trait FloatingWindowHost {
    fn set_content(&self, content: Rc<dyn UIElementExt>);
    fn set_bounds(&self, bounds: Rect);
    fn show(&self);
    fn close(&self);
    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>);
}

#[cfg(target_os = "macos")]
struct PlatformFloatingHost {
    window: Rc<elwindui_backend_appkit::Window>,
}

#[cfg(target_os = "windows")]
struct PlatformFloatingHost {
    window: Rc<elwindui_backend_winui3::Window>,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl FloatingWindowHost for PlatformFloatingHost {
    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.window.set_content(content);
    }

    fn set_bounds(&self, bounds: Rect) {
        self.window.set_left(bounds.x);
        self.window.set_top(bounds.y);
        self.window.set_width(bounds.width);
        self.window.set_height(bounds.height);
    }

    fn show(&self) {
        self.window.show();
    }

    fn close(&self) {
        self.window.close();
    }

    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        self.window.set_close_request_handler(handler);
    }
}

fn create_platform_host() -> Result<Rc<dyn FloatingWindowHost>, DockLayoutError> {
    #[cfg(target_os = "macos")]
    {
        Ok(Rc::new(PlatformFloatingHost {
            window: elwindui_backend_appkit::Window::new(),
        }))
    }
    #[cfg(target_os = "windows")]
    {
        Ok(Rc::new(PlatformFloatingHost {
            window: elwindui_backend_winui3::Window::new(),
        }))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(DockLayoutError::FloatingHostUnavailable {
            reason: "the current platform has no Docking Window implementation".to_owned(),
        })
    }
}

pub(crate) type FloatingHostFactory =
    Rc<dyn Fn() -> Result<Rc<dyn FloatingWindowHost>, DockLayoutError>>;

fn close_handler(
    owner: &std::rc::Weak<crate::DockingControl>,
    host_id: FloatingHostId,
) -> Rc<dyn Fn() -> bool> {
    let weak_owner = owner.clone();
    Rc::new(move || {
        weak_owner
            .upgrade()
            .is_some_and(|owner| owner.handle_floating_close_host(host_id))
    })
}

pub(crate) struct PreparedFloatingHost {
    pub(crate) id: FloatingHostId,
    pub(crate) bounds: Rect,
    pub(crate) surface: Rc<DockSurfaceView>,
    pub(crate) host: Rc<dyn FloatingWindowHost>,
}

impl PreparedFloatingHost {
    pub(crate) fn abort(self) {
        self.host.set_close_request_handler(None);
        self.host.close();
    }
}

pub(crate) struct FloatingHostState {
    pub(crate) id: FloatingHostId,
    pub(crate) root_index: usize,
    pub(crate) bounds: Rect,
    pub(crate) surface: Rc<DockSurfaceView>,
    pub(crate) host: Rc<dyn FloatingWindowHost>,
}

pub(crate) struct FloatingHostRegistry {
    next_id: u64,
    hosts: Vec<FloatingHostState>,
    factory: FloatingHostFactory,
    hosting_enabled: bool,
}

impl Default for FloatingHostRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FloatingHostRegistry {
    pub(crate) fn new() -> Self {
        Self {
            next_id: 1,
            hosts: Vec::new(),
            factory: Rc::new(create_platform_host),
            hosting_enabled: cfg!(any(target_os = "macos", target_os = "windows")),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_factory(factory: FloatingHostFactory) -> Self {
        Self {
            next_id: 1,
            hosts: Vec::new(),
            factory,
            hosting_enabled: true,
        }
    }

    fn allocate_id(&mut self) -> FloatingHostId {
        let id = FloatingHostId(self.next_id.max(1));
        self.next_id = self.next_id.saturating_add(1).max(1);
        id
    }

    fn configure_host(
        host: &Rc<dyn FloatingWindowHost>,
        surface: &Rc<DockSurfaceView>,
        bounds: Rect,
        handler: Rc<dyn Fn() -> bool>,
    ) {
        host.set_bounds(bounds);
        let content: Rc<dyn UIElementExt> = surface.clone();
        host.set_content(content);
        host.set_close_request_handler(Some(handler));
    }

    /// Creates and fully configures a host without inserting it into the committed registry.
    pub(crate) fn prepare_new(
        &mut self,
        surface: Rc<DockSurfaceView>,
        bounds: Rect,
        owner: &std::rc::Weak<crate::DockingControl>,
    ) -> Result<PreparedFloatingHost, DockLayoutError> {
        let id = self.allocate_id();
        let host = (self.factory)()?;
        Self::configure_host(&host, &surface, bounds, close_handler(owner, id));
        Ok(PreparedFloatingHost {
            id,
            bounds,
            surface,
            host,
        })
    }

    pub(crate) fn commit_prepared(
        &mut self,
        prepared: PreparedFloatingHost,
        root_index: usize,
    ) -> FloatingHostId {
        let id = prepared.id;
        self.hosts.push(FloatingHostState {
            id,
            root_index,
            bounds: prepared.bounds,
            surface: prepared.surface,
            host: prepared.host,
        });
        id
    }

    pub(crate) fn show(&self, id: FloatingHostId) {
        if let Some(host) = self.hosts.iter().find(|host| host.id == id) {
            host.host.show();
        }
    }

    pub(crate) fn sync(
        &mut self,
        specs: &[(Rect, Rc<DockSurfaceView>)],
        owner: &std::rc::Weak<crate::DockingControl>,
    ) -> Result<(), DockLayoutError> {
        if !self.hosting_enabled {
            return Ok(());
        }
        let mut used = Vec::with_capacity(specs.len());
        for (index, (bounds, surface)) in specs.iter().enumerate() {
            if let Some(host_index) = self
                .hosts
                .iter()
                .position(|host| Rc::ptr_eq(&host.surface, surface))
            {
                let host = &mut self.hosts[host_index];
                host.root_index = index;
                host.bounds = *bounds;
                host.surface = surface.clone();
                host.host.set_bounds(*bounds);
                let content: Rc<dyn UIElementExt> = surface.clone();
                host.host.set_content(content);
                host.host
                    .set_close_request_handler(Some(close_handler(owner, host.id)));
                used.push(host.id);
            } else {
                let host_id = self.allocate_id();
                let host = (self.factory)()?;
                Self::configure_host(&host, surface, *bounds, close_handler(owner, host_id));
                self.hosts.push(FloatingHostState {
                    id: host_id,
                    root_index: index,
                    bounds: *bounds,
                    surface: surface.clone(),
                    host,
                });
                used.push(host_id);
            }
        }
        let mut retained = Vec::with_capacity(self.hosts.len());
        let mut stale = Vec::new();
        for host in self.hosts.drain(..) {
            if used.contains(&host.id) {
                retained.push(host);
            } else {
                stale.push(host);
            }
        }
        self.hosts = retained;
        for host in stale {
            host.host.set_close_request_handler(None);
            host.host.close();
        }
        for id in used {
            self.show(id);
        }
        Ok(())
    }

    pub(crate) fn root_index_for_host(&self, id: FloatingHostId) -> Option<usize> {
        self.hosts
            .iter()
            .find(|host| host.id == id)
            .map(|host| host.root_index)
    }

    pub(crate) fn close_empty(&mut self) {
        for host in self.hosts.drain(..) {
            host.host.set_close_request_handler(None);
            host.host.close();
        }
    }

    #[cfg(test)]
    pub(crate) fn host_ids(&self) -> Vec<FloatingHostId> {
        self.hosts.iter().map(|host| host.id).collect()
    }

    #[cfg(test)]
    pub(crate) fn host_count(&self) -> usize {
        self.hosts.len()
    }
}
