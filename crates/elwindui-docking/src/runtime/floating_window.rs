//! Target-specific native floating-window adapters.

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

fn create_platform_host(
    surface: Rc<DockSurfaceView>,
    bounds: Rect,
    handler: Rc<dyn Fn() -> bool>,
) -> Result<Rc<dyn FloatingWindowHost>, DockLayoutError> {
    #[cfg(target_os = "macos")]
    {
        let window = elwindui_backend_appkit::Window::new();
        let host = Rc::new(PlatformFloatingHost { window });
        let content: Rc<dyn UIElementExt> = surface.clone();
        host.set_bounds(bounds);
        host.set_content(content);
        host.set_close_request_handler(Some(handler));
        Ok(host)
    }
    #[cfg(target_os = "windows")]
    {
        let window = elwindui_backend_winui3::Window::new();
        let host = Rc::new(PlatformFloatingHost { window });
        let content: Rc<dyn UIElementExt> = surface.clone();
        host.set_bounds(bounds);
        host.set_content(content);
        host.set_close_request_handler(Some(handler));
        Ok(host)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (surface, bounds, handler);
        Err(DockLayoutError::FloatingHostUnavailable {
            reason: "the current platform has no Docking Window implementation".to_owned(),
        })
    }
}

fn close_handler(
    owner: &std::rc::Weak<crate::DockingControl>,
    floating_index: usize,
) -> Rc<dyn Fn() -> bool> {
    let weak_owner = owner.clone();
    Rc::new(move || {
        weak_owner
            .upgrade()
            .is_some_and(|owner| owner.handle_floating_close(floating_index))
    })
}

pub(crate) struct FloatingHostState {
    pub(crate) id: FloatingHostId,
    pub(crate) bounds: Rect,
    pub(crate) surface: Rc<DockSurfaceView>,
    pub(crate) host: Rc<dyn FloatingWindowHost>,
}

#[derive(Default)]
pub(crate) struct FloatingHostRegistry {
    next_id: u64,
    hosts: Vec<FloatingHostState>,
}

impl FloatingHostRegistry {
    pub(crate) fn sync(
        &mut self,
        specs: &[(Rect, Rc<DockSurfaceView>)],
        owner: &std::rc::Weak<crate::DockingControl>,
    ) -> Result<(), DockLayoutError> {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (specs, owner);
            return Ok(());
        }
        if specs.len() < self.hosts.len() {
            for host in self.hosts.drain(specs.len()..) {
                host.host.set_close_request_handler(None);
                host.host.close();
            }
        }
        for (index, (bounds, surface)) in specs.iter().enumerate() {
            if let Some(host) = self.hosts.get_mut(index) {
                host.bounds = *bounds;
                host.surface = surface.clone();
                let content: Rc<dyn UIElementExt> = surface.clone();
                host.host.set_bounds(*bounds);
                host.host.set_content(content);
                host.host
                    .set_close_request_handler(Some(close_handler(owner, index)));
            } else {
                self.next_id = self.next_id.max(1);
                let host_id = FloatingHostId(self.next_id);
                self.next_id = self.next_id.saturating_add(1);
                let handler = close_handler(owner, index);
                let host = create_platform_host(surface.clone(), *bounds, handler)?;
                self.hosts.push(FloatingHostState {
                    id: host_id,
                    bounds: *bounds,
                    surface: surface.clone(),
                    host,
                });
            }
        }
        for host in &self.hosts {
            host.host.show();
        }
        Ok(())
    }

    pub(crate) fn close_empty(&mut self) {
        for host in self.hosts.drain(..) {
            let _host_id = host.id;
            host.host.set_close_request_handler(None);
            host.host.close();
        }
    }
}

pub(crate) fn floating_host_available() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}
