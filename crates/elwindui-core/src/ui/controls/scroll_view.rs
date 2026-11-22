//! `elwindui::ui::ScrollView` — hosts an elwindui subtree inside a native scrolling container.

use super::*;
use crate::ui::native::{NativePeer, ScrollViewPeer, create_native_peer};
use std::cell::OnceCell;

/// Hosts arbitrary elwindui content inside a native scrolling container —
/// `ScrollView -> NativeScrollHost -> ElwinduiContentRoot -> content`
/// (`docs/status/control_status.md`). The content is a full elwindui subtree with its own
/// layout/paint/hit-test/focus, hosted by a second, nested instance of the backend's own tree host
/// whose Measure runs unconstrained on the scrolling axis/axes, so the content gets its natural
/// size and the native container scrolls it.
///
/// Declared once here as a Core-owned Component (Issue #313, `native_peer_design.md` §7c). Core
/// keeps `content` and the two scroll axes (vertical-only by default). The lazily realized
/// [`ScrollViewPeer`] owns the nested host and its content lifecycle: realization pushes the axes,
/// then the stored content; a later `set_content` is pushed straight through, and the peer retires
/// the previous content before attaching the new one. When this ScrollView unmounts, the hosted
/// content is retired for good.
///
/// Scroll-position get/set and a `scroll_changed` event are deliberately not part of this class —
/// the same "ship the minimal, honestly-scoped surface, document the gap" call as `TextBox`/
/// `PasswordBox`'s deferred selection APIs.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
#[prop(horizontal_scroll_enabled: Option<bool>)]
#[prop(vertical_scroll_enabled: Option<bool>)]
pub struct ScrollView {
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    horizontal: Cell<bool>,
    vertical: Cell<bool>,
    peer: OnceCell<Rc<dyn ScrollViewPeer>>,
}

#[elwindui_macros::component]
impl ScrollView {
    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("ScrollView", |factory| factory.create_scroll_view());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a ScrollView's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: the axes, then the stored content.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        peer.set_horizontal_scroll_enabled(self.horizontal.get());
        peer.set_vertical_scroll_enabled(self.vertical.get());
        let content = self.content.borrow().clone();
        if let Some(content) = content {
            peer.set_content(content, self.owner());
        }
    }

    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        *self.content.borrow_mut() = Some(content.clone());
        if let Some(peer) = self.peer.get().cloned() {
            peer.set_content(content, self.owner());
        }
    }

    fn set_horizontal_scroll_enabled(&self, enabled: bool) {
        self.horizontal.set(enabled);
        if let Some(peer) = self.peer.get() {
            peer.set_horizontal_scroll_enabled(enabled);
        }
    }

    fn set_vertical_scroll_enabled(&self, enabled: bool) {
        self.vertical.set(enabled);
        if let Some(peer) = self.peer.get() {
            peer.set_vertical_scroll_enabled(enabled);
        }
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            content: RefCell::new(None),
            horizontal: Cell::new(false),
            vertical: Cell::new(true),
            peer: OnceCell::new(),
        }
    }

    /// The content lives in a nested host outside the Visual tree, so it is retired (popups
    /// first) when this ScrollView unmounts. An unrealized ScrollView hosts nothing.
    fn on_constructed(&self) {
        let Some(owner) = self.as_ui_element().visual_collection.owner_rc() else {
            return;
        };
        let owner = Rc::downgrade(&owner);
        self.add_unmount_hook(Box::new(move || {
            let owner: Option<Rc<dyn UIElementExt>> = owner.upgrade();
            let Some(owner) = owner else { return };
            let Some(this) = owner.as_any().downcast_ref::<ScrollView>() else {
                return;
            };
            let peer = this.peer.get().cloned();
            if let Some(peer) = peer {
                peer.unmount_content();
            }
        }));
    }
}

impl ScrollView {
    fn owner(&self) -> Option<Rc<dyn UIElementExt>> {
        self.as_ui_element().visual_collection.owner_rc()
    }

    /// The realized peer, realizing it first if needed. For backend-internal hosts and tests.
    #[doc(hidden)]
    pub fn __scroll_view_peer(&self) -> Rc<dyn ScrollViewPeer> {
        self.__native_peer();
        self.peer.get().expect("realized above").clone()
    }

    /// The logical content, if any.
    pub fn content(&self) -> Option<Rc<dyn UIElementExt>> {
        self.content.borrow().clone()
    }
}
