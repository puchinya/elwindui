//! Target-specific native floating-window adapters and their staged registry.

use crate::DockLayoutError;
use crate::Rect;
use crate::core::ui::{UIElementExt, WindowExt, WindowLifecycleHost};
use crate::runtime::DockSurfaceView;
use std::rc::{Rc, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FloatingHostId(u64);

pub(crate) trait FloatingWindowHost {
    fn set_content(&self, content: Rc<dyn UIElementExt>);
    fn set_bounds(&self, bounds: Rect);
    fn set_title(&self, title: &str);
    fn show(&self);
    fn activate(&self);
    fn close(&self);
    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>);
    fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>);
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

    fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    fn show(&self) {
        self.window.show();
    }

    fn activate(&self) {
        WindowLifecycleHost::activate(self.window.as_ref());
    }

    fn close(&self) {
        self.window.close();
    }

    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        self.window.set_close_request_handler(handler);
    }

    fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
        WindowLifecycleHost::set_bounds_changed_handler(self.window.as_ref(), handler);
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
    let weak_owner: Weak<crate::DockingControl> = owner.clone();
    Rc::new(move || {
        let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
        owner.is_some_and(|owner| owner.handle_floating_close_host(host_id))
    })
}

fn bounds_changed_handler(
    owner: &std::rc::Weak<crate::DockingControl>,
    host_id: FloatingHostId,
) -> Rc<dyn Fn(Rect)> {
    let weak_owner = owner.clone();
    Rc::new(move |bounds| {
        if let Some(owner) = weak_owner.upgrade() {
            owner.handle_floating_bounds_changed(host_id, bounds);
        }
    })
}

pub(crate) struct PreparedFloatingHost {
    pub(crate) id: FloatingHostId,
    pub(crate) root_index: usize,
    pub(crate) bounds: Rect,
    pub(crate) surface: Rc<DockSurfaceView>,
    pub(crate) host: Rc<dyn FloatingWindowHost>,
}

impl PreparedFloatingHost {
    pub(crate) fn abort(self) {
        self.host.set_bounds_changed_handler(None);
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
    /// Set while this host is being closed by the native window manager. The native callback is
    /// still on the stack, so the committed model removes the host from our registry and lets the
    /// original native close continue instead of calling `close()` reentrantly.
    pub(crate) native_close_in_flight: bool,
}

struct PreparedExistingHostUpdate {
    id: FloatingHostId,
    root_index: usize,
    bounds: Rect,
    surface: Rc<DockSurfaceView>,
    close_handler: Rc<dyn Fn() -> bool>,
    bounds_changed_handler: Rc<dyn Fn(Rect)>,
}

/// Native floating-host changes prepared without touching the committed registry.
pub(crate) struct PreparedFloatingHostSync {
    next_id: u64,
    updates: Vec<PreparedExistingHostUpdate>,
    new_hosts: Vec<PreparedFloatingHost>,
    stale_ids: Vec<FloatingHostId>,
}

impl PreparedFloatingHostSync {
    pub(crate) fn empty(next_id: u64) -> Self {
        Self {
            next_id,
            updates: Vec::new(),
            new_hosts: Vec::new(),
            stale_ids: Vec::new(),
        }
    }

    pub(crate) fn abort(self) {
        for host in self.new_hosts {
            host.abort();
        }
    }
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

    fn allocate_id(next_id: &mut u64) -> FloatingHostId {
        let id = FloatingHostId((*next_id).max(1));
        *next_id = (*next_id).saturating_add(1).max(1);
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
    #[cfg(test)]
    pub(crate) fn prepare_new(
        &self,
        surface: Rc<DockSurfaceView>,
        bounds: Rect,
        owner: &std::rc::Weak<crate::DockingControl>,
    ) -> Result<PreparedFloatingHost, DockLayoutError> {
        let mut next_id = self.next_id;
        let id = Self::allocate_id(&mut next_id);
        let host = (self.factory)()?;
        Self::configure_host(&host, &surface, bounds, close_handler(owner, id));
        host.set_bounds_changed_handler(Some(bounds_changed_handler(owner, id)));
        Ok(PreparedFloatingHost {
            id,
            root_index: 0,
            bounds,
            surface,
            host,
        })
    }

    #[cfg(test)]
    pub(crate) fn commit_prepared(
        &mut self,
        prepared: PreparedFloatingHost,
        root_index: usize,
    ) -> FloatingHostId {
        let id = prepared.id;
        self.next_id = self.next_id.max(id.0.saturating_add(1).max(1));
        self.hosts.push(FloatingHostState {
            id,
            root_index,
            bounds: prepared.bounds,
            surface: prepared.surface,
            host: prepared.host,
            native_close_in_flight: false,
        });
        id
    }

    pub(crate) fn show(&self, id: FloatingHostId) {
        if let Some(host) = self.hosts.iter().find(|host| host.id == id) {
            host.host.show();
        }
    }

    pub(crate) fn activate(&self, id: FloatingHostId) {
        if let Some(host) = self.hosts.iter().find(|host| host.id == id) {
            host.host.activate();
        }
    }

    pub(crate) fn set_title(&self, root_index: usize, title: &str) {
        if let Some(host) = self.hosts.iter().find(|host| host.root_index == root_index) {
            host.host.set_title(title);
        }
    }

    #[cfg(test)]
    pub(crate) fn sync(
        &mut self,
        specs: &[(Rect, Rc<DockSurfaceView>)],
        owner: &std::rc::Weak<crate::DockingControl>,
    ) -> Result<(), DockLayoutError> {
        let prepared = self.prepare_sync(specs, owner)?;
        self.commit_sync(prepared);
        Ok(())
    }

    /// Plans synchronization against candidate floating surfaces without changing committed
    /// native hosts or the registry. New hosts are configured but remain hidden and unregistered.
    pub(crate) fn prepare_sync(
        &self,
        specs: &[(Rect, Rc<DockSurfaceView>)],
        owner: &std::rc::Weak<crate::DockingControl>,
    ) -> Result<PreparedFloatingHostSync, DockLayoutError> {
        if !self.hosting_enabled {
            return Ok(PreparedFloatingHostSync::empty(self.next_id));
        }
        let mut next_id = self.next_id;
        let mut used = Vec::with_capacity(specs.len());
        let mut updates = Vec::with_capacity(specs.len());
        let mut new_hosts: Vec<PreparedFloatingHost> = Vec::new();
        for (index, (bounds, surface)) in specs.iter().enumerate() {
            if let Some(host) = self
                .hosts
                .iter()
                .find(|host| Rc::ptr_eq(&host.surface, surface))
            {
                updates.push(PreparedExistingHostUpdate {
                    id: host.id,
                    root_index: index,
                    bounds: *bounds,
                    surface: surface.clone(),
                    close_handler: close_handler(owner, host.id),
                    bounds_changed_handler: bounds_changed_handler(owner, host.id),
                });
                used.push(host.id);
            } else {
                let host_id = Self::allocate_id(&mut next_id);
                let host = match (self.factory)() {
                    Ok(host) => host,
                    Err(error) => {
                        for prepared in new_hosts {
                            prepared.abort();
                        }
                        return Err(error);
                    }
                };
                Self::configure_host(&host, surface, *bounds, close_handler(owner, host_id));
                host.set_bounds_changed_handler(Some(bounds_changed_handler(owner, host_id)));
                new_hosts.push(PreparedFloatingHost {
                    id: host_id,
                    root_index: index,
                    bounds: *bounds,
                    surface: surface.clone(),
                    host,
                });
                used.push(host_id);
            }
        }
        let stale_ids = self
            .hosts
            .iter()
            .filter(|host| !used.contains(&host.id))
            .map(|host| host.id)
            .collect();
        Ok(PreparedFloatingHostSync {
            next_id,
            updates,
            new_hosts,
            stale_ids,
        })
    }

    /// Commits a prepared host synchronization after the retained runtime/model commit has
    /// succeeded. There are no recoverable Docking errors in this phase.
    pub(crate) fn commit_sync(&mut self, prepared: PreparedFloatingHostSync) {
        if !self.hosting_enabled {
            return;
        }
        self.next_id = self.next_id.max(prepared.next_id);

        for update in prepared.updates {
            if let Some(host) = self.hosts.iter_mut().find(|host| host.id == update.id) {
                host.root_index = update.root_index;
                host.bounds = update.bounds;
                host.surface = update.surface.clone();
                host.host.set_bounds(update.bounds);
                let content: Rc<dyn UIElementExt> = update.surface.clone();
                host.host.set_content(content);
                host.host
                    .set_close_request_handler(Some(update.close_handler));
                host.host
                    .set_bounds_changed_handler(Some(update.bounds_changed_handler));
            }
        }

        let mut new_ids = Vec::new();
        for host in prepared.new_hosts {
            new_ids.push(host.id);
            self.hosts.push(FloatingHostState {
                id: host.id,
                root_index: host.root_index,
                bounds: host.bounds,
                surface: host.surface,
                host: host.host,
                native_close_in_flight: false,
            });
        }

        let stale_ids = prepared.stale_ids;
        let mut retained = Vec::with_capacity(self.hosts.len());
        for host in self.hosts.drain(..) {
            if stale_ids.contains(&host.id) {
                let native_close_in_flight = host.native_close_in_flight;
                host.host.set_bounds_changed_handler(None);
                host.host.set_close_request_handler(None);
                if !native_close_in_flight {
                    host.host.close();
                }
            } else {
                retained.push(host);
            }
        }
        self.hosts = retained;

        for id in new_ids {
            self.show(id);
            self.activate(id);
        }
    }

    pub(crate) fn root_index_for_host(&self, id: FloatingHostId) -> Option<usize> {
        self.hosts
            .iter()
            .find(|host| host.id == id)
            .map(|host| host.root_index)
    }

    pub(crate) fn begin_native_close(&mut self, id: FloatingHostId) -> bool {
        let Some(host) = self.hosts.iter_mut().find(|host| host.id == id) else {
            return false;
        };
        if host.native_close_in_flight {
            return false;
        }
        host.native_close_in_flight = true;
        true
    }

    pub(crate) fn cancel_native_close(&mut self, id: FloatingHostId) {
        if let Some(host) = self.hosts.iter_mut().find(|host| host.id == id) {
            host.native_close_in_flight = false;
        }
    }

    pub(crate) fn close_empty(&mut self) {
        for host in self.hosts.drain(..) {
            let native_close_in_flight = host.native_close_in_flight;
            host.host.set_bounds_changed_handler(None);
            host.host.set_close_request_handler(None);
            if !native_close_in_flight {
                host.host.close();
            }
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
