//! Backend peer contract for Core-owned native Components (Issue #313,
//! `docs/design/runtime/native_peer_design.md`).
//!
//! Each native builtin (`Button`, ...) is declared once in `elwindui-core` as an ordinary
//! Component. Its OS widget is a *peer*: an object implementing one of the traits below, created
//! lazily by the [`NativePeerFactory`] the active backend registered for the current UI thread.
//! Core owns the public properties, logical state, routed events, and lifecycle. The peer only
//! translates values to the native widget and reports native notifications back through callbacks
//! Core installs.
//!
//! These are backend-implementer traits, not DSL surface. Core never names a platform type: the
//! native handle crosses the boundary only as `&dyn Any` ([`NativePeer::handle`]), which the
//! backend's own render/host reconciliation downcasts to its handle type.

use crate::base::{Rect, Size};
use crate::graphics::IconSource;
use crate::graphics::{Brush, CascadedTextStyle};
use crate::ui::{ButtonRole, CheckState, TabViewItemExt, TextAlignment, UIElementExt, WindowExt};
use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// The native widget behind a `NativeControl`.
///
/// Every method is called on the UI thread that created the peer. Core never holds a `RefCell`
/// borrow across a peer call, so an implementation may synchronously re-enter Core (for example
/// through a native event raised by a setter).
///
/// When it does, the last logical write wins (`native_peer_design.md` §6): Core derives its own
/// state from the latest stored value, and discards the result of a fallible push that a
/// reentrant push superseded, re-pushing the newest value on the next measure pass instead. A
/// peer whose native call raises a synchronous event must finish its own native write first.
pub trait NativePeer {
    /// The backend's type-erased native handle (for example the WinUI3/AppKit `AnyView`).
    ///
    /// Returned to Core's render pipeline through `UIElementExt::try_as_native_control`, whose
    /// consumer downcasts it to the backend handle type. It must stay the same object for the
    /// peer's whole lifetime.
    fn handle(&self) -> &dyn Any;

    /// The widget's natural size for `available`, used as the control's `measure_override`.
    fn measure(&self, available: Size) -> Size;

    /// Applies an explicit background, or restores the toolkit default for `None`.
    ///
    /// Returns `false` when the native call failed. Core then keeps the value pending and pushes
    /// the current logical background again at the start of every later measure pass until the
    /// peer returns `true`; a newer `set_background`/`clear_background` replaces the pending value.
    fn apply_background(&self, background: Option<&Brush>) -> bool;

    /// Sets the hover tooltip, or removes it for `None`.
    fn set_tooltip(&self, tooltip: Option<&str>);

    /// Whether this widget can display a text style at all. Core skips text-style resolution for
    /// widgets that return `false`.
    fn supports_text_style(&self) -> bool;

    /// Applies a resolved text style. Returns `false` when the native call failed; Core then
    /// retries on the next measure pass instead of caching the style as applied.
    fn apply_text_style(&self, style: &CascadedTextStyle) -> bool;

    /// Whether Core must re-push the resolved text style on every measure pass instead of only
    /// when it changed.
    ///
    /// For widgets that can silently lose their attributed text when a sibling subtree is
    /// reconciled (AppKit `NSButton`). The default is `false`.
    fn refresh_text_style_on_measure(&self) -> bool {
        false
    }

    /// Detaches every native target and callback registration this peer owns.
    ///
    /// Called exactly once by Core when the owning Component is dropped. Must be idempotent and,
    /// once it returns, no callback previously installed by Core may run again. The default does
    /// nothing, for peers that own no registrations.
    fn dispose(&self) {}
}

/// The native push button behind `elwindui::ui::Button`.
pub trait ButtonPeer: NativePeer {
    /// Sets the button caption.
    fn set_text(&self, text: &str);

    /// Enables or disables native interaction.
    fn set_enabled(&self, enabled: bool);

    /// Applies a [`ButtonRole`]'s native emphasis, fully resetting the previous role's effect.
    fn set_role(&self, role: ButtonRole);

    /// Marks the button as the window default (activated by Return).
    fn set_is_default(&self, is_default: bool);

    /// Installs, or with `None` removes, the native activation callback.
    ///
    /// Core installs exactly one callback when the peer is realized. It captures only a `Weak`
    /// reference to the Component, so it is a no-op after the Component is gone.
    fn set_on_click(&self, callback: Option<Box<dyn Fn()>>);
}

/// The native check box behind `elwindui::ui::CheckBox`.
///
/// `set_checked` is programmatic and must not raise the change callback. The callback reports a
/// user change only; Core compares it with its stored value before notifying (§5 two-way rule).
pub trait CheckBoxPeer: NativePeer {
    /// Sets the label.
    fn set_text(&self, text: &str);
    /// Displays `checked`, including the indeterminate state.
    fn set_checked(&self, checked: CheckState);
    /// Enables or disables native interaction.
    fn set_enabled(&self, enabled: bool);
    /// Installs, or with `None` removes, the user-change callback. A user click can only land on
    /// `Unchecked`/`Checked`.
    fn set_on_change(&self, callback: Option<Box<dyn Fn(CheckState)>>);
}

/// The native radio button behind `elwindui::ui::RadioButton`. Grouping is Core's job: the peer
/// only reports activation.
pub trait RadioButtonPeer: NativePeer {
    /// Sets the label.
    fn set_text(&self, text: &str);
    /// Displays the checked state. Programmatic: raises no callback.
    fn set_checked(&self, checked: bool);
    /// Enables or disables native interaction.
    fn set_enabled(&self, enabled: bool);
    /// Installs, or with `None` removes, the user-activation callback.
    fn set_on_click(&self, callback: Option<Box<dyn Fn()>>);
}

/// The native slider behind `elwindui::ui::Slider`.
pub trait SliderPeer: NativePeer {
    /// Displays `value`. Programmatic: raises no callback.
    fn set_value(&self, value: f32);
    /// Sets the minimum.
    fn set_min(&self, min: f32);
    /// Sets the maximum.
    fn set_max(&self, max: f32);
    /// Sets the normalized step (`None` = continuous). Core owns the snapping rule; the peer only
    /// tunes the native feel (snapping, keyboard increment) and must not re-snap the value itself.
    fn set_step(&self, step: Option<f32>);
    /// Enables or disables native interaction.
    fn set_enabled(&self, enabled: bool);
    /// Installs, or with `None` removes, the user-change callback.
    fn set_on_change(&self, callback: Option<Box<dyn Fn(f32)>>);
}

/// The native toggle switch behind `elwindui::ui::ToggleSwitch`.
pub trait ToggleSwitchPeer: NativePeer {
    /// Displays the on/off state. Programmatic: raises no callback.
    fn set_is_on(&self, is_on: bool);
    /// Enables or disables native interaction.
    fn set_enabled(&self, enabled: bool);
    /// Installs, or with `None` removes, the user-change callback.
    fn set_on_change(&self, callback: Option<Box<dyn Fn(bool)>>);
}

/// The native single-line text box behind `elwindui::ui::TextBox`.
pub trait TextBoxPeer: NativePeer {
    /// Displays `text`. Programmatic: must not reach Core as a user change (Core also drops an
    /// echo whose value equals its stored text).
    fn set_text(&self, text: &str);
    /// Sets the placeholder shown while empty.
    fn set_placeholder(&self, text: &str);
    /// Makes the field read-only.
    fn set_read_only(&self, read_only: bool);
    /// Limits the text length; `None` removes the limit.
    fn set_max_length(&self, max_length: Option<u32>);
    /// Aligns the text.
    fn set_text_alignment(&self, alignment: TextAlignment);
    /// Installs, or with `None` removes, the user-edit callback.
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>);
    /// Installs, or with `None` removes, the Enter/submit callback.
    fn set_on_submit(&self, callback: Option<Box<dyn Fn()>>);
}

/// The native multi-line text area behind `elwindui::ui::TextArea`.
pub trait TextAreaPeer: NativePeer {
    /// Displays `text`. Programmatic.
    fn set_text(&self, text: &str);
    /// Installs, or with `None` removes, the user-edit callback.
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>);
}

/// The native secure text field behind `elwindui::ui::PasswordBox`.
///
/// Implementations must never log, `Debug` or otherwise expose the password value.
pub trait PasswordBoxPeer: NativePeer {
    /// Displays `password`. Programmatic.
    fn set_password(&self, password: &str);
    /// Sets the placeholder shown while empty.
    fn set_placeholder(&self, text: &str);
    /// Limits the length; `None` removes the limit.
    fn set_max_length(&self, max_length: Option<u32>);
    /// Shows or hides the reveal affordance.
    fn set_reveal_enabled(&self, enabled: bool);
    /// Installs, or with `None` removes, the user-edit callback.
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>);
}

/// The native drop-down selection control behind `elwindui::ui::Dropdown`.
///
/// Core owns the item list and the selected index; the peer only displays them. `set_items` and
/// `set_selected_index` are programmatic and must not raise the change callback.
pub trait DropdownPeer: NativePeer {
    /// Replaces the displayed item texts, in order.
    fn set_items(&self, texts: &[String]);
    /// Displays `index` as the selection. Programmatic: raises no callback.
    fn set_selected_index(&self, index: usize);
    /// Enables or disables native interaction.
    fn set_enabled(&self, enabled: bool);
    /// Installs, or with `None` removes, the user-selection callback.
    fn set_on_change(&self, callback: Option<Box<dyn Fn(usize)>>);
}

/// The native scrolling container behind `elwindui::ui::ScrollView`.
///
/// The peer hosts Core's `content` subtree in a nested backend host (`native_peer_design.md` §7c)
/// and owns that host's content lifecycle: replacing content retires the previous tree first,
/// and [`ScrollViewPeer::unmount_content`] retires it for good.
pub trait ScrollViewPeer: NativePeer {
    /// Hosts `content`. `owner` is the ScrollView itself; the backend links the content's logical
    /// parent to it while the content is hosted.
    fn set_content(&self, content: Rc<dyn UIElementExt>, owner: Option<Rc<dyn UIElementExt>>);
    /// Enables or disables horizontal scrolling.
    fn set_horizontal_scroll_enabled(&self, enabled: bool);
    /// Enables or disables vertical scrolling.
    fn set_vertical_scroll_enabled(&self, enabled: bool);
    /// Retires the hosted content (popup first, then one unmount). Called when the ScrollView
    /// unmounts; the host accepts no content afterwards.
    fn unmount_content(&self);
    /// The concrete peer, for backend-internal downcasts (tests, hosts).
    fn as_any(&self) -> &dyn Any;
}

/// The native tab container behind `elwindui::ui::TabView`.
///
/// Core owns the ordered item list, the selected index and the selection policy after a removal
/// (`docs/specs/ui_spec.md` TabView). The peer owns each tab's nested content host, keyed by item
/// identity (`Rc::as_ptr`), and shows/hides them.
///
/// A user selection is shown by the peer itself (it activates the selected tab's content) before
/// it reports the index through the select callback. `set_items` and `set_selected_index` are
/// programmatic; a native selection event they raise reaches Core only as an echo of the stored
/// index, which Core drops.
pub trait TabViewPeer: NativePeer {
    /// Reconciles the displayed tabs with `items` by identity: removed tabs retire their content
    /// first, new tabs get a content host for the item's content, and every tab's header and
    /// closable state are refreshed. `owner` is the TabView itself (the hosted content's logical
    /// parent). May re-enter Core synchronously (content unmount hooks); a reentrant call only
    /// replaces the pending list.
    ///
    /// Each item is Core's `TabViewItem` (`item.as_ui_element()` gives its header, closable state
    /// and content).
    fn set_items(&self, items: &[Rc<dyn TabViewItemExt>], owner: Option<Rc<dyn UIElementExt>>);
    /// Selects `index` and activates its content host. Programmatic.
    fn set_selected_index(&self, index: usize);
    /// Updates the header of the tab at `index`.
    fn set_tab_header(&self, index: usize, header: &str);
    /// Updates whether the tab at `index` shows a close affordance.
    fn set_tab_closable(&self, index: usize, closable: bool);
    /// Installs, or with `None` removes, the user-selection callback.
    fn set_on_select(&self, callback: Option<Box<dyn Fn(usize)>>);
    /// Installs, or with `None` removes, the tab close-request callback.
    fn set_on_close(&self, callback: Option<Box<dyn Fn(usize)>>);
    /// Installs, or with `None` removes, the new-tab request callback.
    fn set_on_new_tab(&self, callback: Option<Box<dyn Fn()>>);
    /// Retires every tab's hosted content. Called when the TabView unmounts.
    fn unmount_content(&self);
    /// The concrete peer, for backend-internal downcasts (tests, hosts).
    fn as_any(&self) -> &dyn Any;
}

/// The native menu entry behind `elwindui::ui::MenuItem`.
///
/// Not a [`NativePeer`]: menu objects are not `UIElement`s. The peer of a `MenuItem` is created
/// once, by the first native menu or menu bar that shows it. Core lets a `MenuItem` belong to only
/// one `Menu` at a time, so the peer is attached to at most one native parent (Issue #323); a native
/// context menu that builds its own per-open entries does not attach this peer. `select()` from Core
/// is not routed through the peer.
pub trait MenuItemPeer {
    /// Sets the caption.
    fn set_text(&self, text: &str);
    /// Sets or clears the icon.
    fn set_icon(&self, icon: Option<IconSource>);
    /// Sets the keyboard shortcut (`""` clears it).
    fn set_shortcut(&self, shortcut: &str);
    /// Enables or disables the entry.
    fn set_enabled(&self, enabled: bool);
    /// Installs, or with `None` removes, the user-activation callback.
    fn set_on_select(&self, callback: Option<Box<dyn Fn()>>);
    /// Releases every callback registration. Called once when the `MenuItem` drops.
    fn dispose(&self) {}
    /// The concrete peer, for backend-internal downcasts.
    fn as_any(&self) -> &dyn Any;
}

/// The native menu behind `elwindui::ui::Menu` (a submenu or a context menu).
pub trait MenuPeer {
    /// Reconciles the native entries with `items`, in order, by peer identity.
    fn set_items(&self, items: &[Rc<dyn MenuItemPeer>]);
    /// The concrete peer, for backend-internal downcasts (context-menu presentation).
    fn as_any(&self) -> &dyn Any;
}

/// The native top-level menu entry behind `elwindui::ui::MenuBarItem`.
pub trait MenuBarItemPeer {
    /// Sets the caption.
    fn set_text(&self, text: &str);
    /// Installs the submenu.
    fn set_submenu(&self, submenu: Rc<dyn MenuPeer>);
    /// The concrete peer, for backend-internal downcasts.
    fn as_any(&self) -> &dyn Any;
}

/// The native application menu bar behind `elwindui::ui::MenuBar`.
pub trait MenuBarPeer {
    /// Reconciles the native top-level entries with `items`, in order, by peer identity.
    fn set_items(&self, items: &[Rc<dyn MenuBarItemPeer>]);
    /// The concrete peer, for backend-internal downcasts (the Window installs it).
    fn as_any(&self) -> &dyn Any;
}

/// The native top-level window behind `elwindui::ui::Window`.
///
/// Not a [`NativePeer`]: a Window is a non-`UIElement` root with no measure, background or text
/// style. Created lazily by Core `Window` on its first `show()` (or on the first native-only
/// operation) through [`NativePeerFactory::create_window`], pushed Core's stored logical state,
/// then given its close-request and bounds-changed handlers last
/// (`native_peer_design.md` §7). Core never holds a `RefCell` borrow across a call, so an
/// implementation may re-enter Core synchronously; the §6 last-write-wins rules apply.
///
/// The backend keeps its own retention registry: the `owner` given to `create_window` is
/// upgraded on first show and released on native close.
pub trait WindowPeer {
    /// Makes the window visible. The first call retains the owner in the backend registry.
    fn show(&self);
    /// Hides the window without releasing it.
    fn hide(&self);
    /// Closes the native window and releases the registry entry, if any.
    fn close(&self);
    /// Activates the window without changing its bounds.
    fn activate(&self);
    /// Closes the window's active popup, if any (owner-Window unmount, before content teardown).
    fn close_active_popup(&self);
    /// Sets the title-bar text.
    fn set_title(&self, title: &str);
    /// Installs the menu bar.
    fn set_menu_bar(&self, menu_bar: Rc<dyn MenuBarPeer>);
    /// Hosts `content` as the window's root element.
    fn set_content(&self, content: Rc<dyn UIElementExt>);
    /// Enables or disables an alpha-capable client surface.
    fn set_transparent(&self, transparent: bool);
    /// Pins the window above normal windows, or restores normal Z-order.
    fn set_always_on_top(&self, always_on_top: bool);
    /// The native left edge.
    fn left(&self) -> f32;
    /// Moves the left edge.
    fn set_left(&self, left: f32);
    /// The native top edge.
    fn top(&self) -> f32;
    /// Moves the top edge.
    fn set_top(&self, top: f32);
    /// The native width.
    fn width(&self) -> f32;
    /// Resizes the width.
    fn set_width(&self, width: f32);
    /// The native height.
    fn height(&self) -> f32;
    /// Resizes the height.
    fn set_height(&self, height: f32);
    /// Applies a complete rectangle. Backends with a single native operation override this; the
    /// default sets each edge in turn.
    fn set_bounds(&self, bounds: Rect) {
        self.set_left(bounds.x);
        self.set_top(bounds.y);
        self.set_width(bounds.width);
        self.set_height(bounds.height);
    }
    /// Installs or clears the close-request bridge (`WindowLifecycleHost`). The handler captures
    /// only a `Weak` owner and returns `true` when the generated owner accepted the request.
    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>);
    /// Installs or clears the native move/resize bridge.
    fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>);
    /// Framework-only floating-host title-bar setup (Docking). The default does nothing.
    fn configure_floating_title(&self) -> Result<(), String> {
        Ok(())
    }
    /// Releases every callback registration this peer owns. Called exactly once by Core when the
    /// owning `Window` is dropped; no Core handler may run afterwards. Must be idempotent.
    fn dispose(&self) {}
}

/// Creates the native peers for one backend.
///
/// A backend registers exactly one factory per UI thread from its `init()` through
/// [`register_native_peer_factory`]. Every `create_*` method has a default body that panics with
/// an "unsupported control" diagnostic, so a backend that does not provide a control (for example
/// GTK4 today) fails loudly instead of silently falling back.
pub trait NativePeerFactory {
    /// A short backend name used in diagnostics (for example `"winui3"`).
    fn backend_name(&self) -> &'static str;

    /// Creates the peer for a new `Button`.
    fn create_button(&self) -> Rc<dyn ButtonPeer> {
        unsupported_native_control(self.backend_name(), "Button")
    }

    /// Creates the peer for a new `CheckBox`.
    fn create_check_box(&self) -> Rc<dyn CheckBoxPeer> {
        unsupported_native_control(self.backend_name(), "CheckBox")
    }

    /// Creates the peer for a new `RadioButton`.
    fn create_radio_button(&self) -> Rc<dyn RadioButtonPeer> {
        unsupported_native_control(self.backend_name(), "RadioButton")
    }

    /// Creates the peer for a new `Slider`.
    fn create_slider(&self) -> Rc<dyn SliderPeer> {
        unsupported_native_control(self.backend_name(), "Slider")
    }

    /// Creates the peer for a new `ToggleSwitch`.
    fn create_toggle_switch(&self) -> Rc<dyn ToggleSwitchPeer> {
        unsupported_native_control(self.backend_name(), "ToggleSwitch")
    }

    /// Creates the peer for a new `TextBox`.
    fn create_text_box(&self) -> Rc<dyn TextBoxPeer> {
        unsupported_native_control(self.backend_name(), "TextBox")
    }

    /// Creates the peer for a new `TextArea`.
    fn create_text_area(&self) -> Rc<dyn TextAreaPeer> {
        unsupported_native_control(self.backend_name(), "TextArea")
    }

    /// Creates the peer for a new `PasswordBox`.
    fn create_password_box(&self) -> Rc<dyn PasswordBoxPeer> {
        unsupported_native_control(self.backend_name(), "PasswordBox")
    }

    /// Creates the peer for a new `Dropdown`.
    fn create_dropdown(&self) -> Rc<dyn DropdownPeer> {
        unsupported_native_control(self.backend_name(), "Dropdown")
    }

    /// Creates the peer for a new `ScrollView`.
    fn create_scroll_view(&self) -> Rc<dyn ScrollViewPeer> {
        unsupported_native_control(self.backend_name(), "ScrollView")
    }

    /// Creates the peer for a new `TabView`.
    fn create_tab_view(&self) -> Rc<dyn TabViewPeer> {
        unsupported_native_control(self.backend_name(), "TabView")
    }

    /// Creates the peer for a `MenuItem`.
    fn create_menu_item(&self) -> Rc<dyn MenuItemPeer> {
        unsupported_native_control(self.backend_name(), "MenuItem")
    }

    /// Creates the peer for a `Menu`.
    fn create_menu(&self) -> Rc<dyn MenuPeer> {
        unsupported_native_control(self.backend_name(), "Menu")
    }

    /// Creates the peer for a `MenuBarItem`.
    fn create_menu_bar_item(&self) -> Rc<dyn MenuBarItemPeer> {
        unsupported_native_control(self.backend_name(), "MenuBarItem")
    }

    /// Creates the peer for a `MenuBar`.
    fn create_menu_bar(&self) -> Rc<dyn MenuBarPeer> {
        unsupported_native_control(self.backend_name(), "MenuBar")
    }

    /// Creates the peer for a `Window`. `owner` is the most-derived Window (a generated
    /// host-composition component when there is one); the backend's retention registry upgrades
    /// it on first show and never stores it strongly anywhere else.
    fn create_window(&self, owner: Weak<dyn WindowExt>) -> Rc<dyn WindowPeer> {
        let _ = owner;
        unsupported_native_control(self.backend_name(), "Window")
    }
}

thread_local! {
    static NATIVE_PEER_FACTORY: RefCell<Option<Rc<dyn NativePeerFactory>>> =
        const { RefCell::new(None) };
}

/// Registers `factory` as this UI thread's native peer factory, replacing any previous one.
///
/// Called by each backend's `init()` on the thread that calls `elwindui::init()`. Peers that
/// already exist keep working; only later realizations use the new factory. Replacing a factory
/// is intended for tests (each test thread registers its own mock); production code registers
/// once.
pub fn register_native_peer_factory(factory: Rc<dyn NativePeerFactory>) {
    NATIVE_PEER_FACTORY.with(|cell| *cell.borrow_mut() = Some(factory));
}

/// Removes this thread's native peer factory. Mainly for test hygiene.
pub fn clear_native_peer_factory() {
    NATIVE_PEER_FACTORY.with(|cell| *cell.borrow_mut() = None);
}

/// Whether a native peer factory is registered on the current thread.
pub fn has_native_peer_factory() -> bool {
    NATIVE_PEER_FACTORY.with(|cell| cell.borrow().is_some())
}

/// Runs `create` with this thread's registered factory, for realizing a peer of `control`.
///
/// # Panics
///
/// Panics with a diagnostic naming `control` and the current thread when no factory is
/// registered on this thread. That happens when a native control is realized before
/// `elwindui::init()`, or on a thread other than the UI thread that called it. There is no
/// fallback backend.
pub fn create_native_peer<R>(
    control: &'static str,
    create: impl FnOnce(&dyn NativePeerFactory) -> R,
) -> R {
    // Clone the `Rc` out first so the factory may itself register callbacks or re-enter the
    // registry without hitting a `RefCell` double borrow.
    let factory = NATIVE_PEER_FACTORY.with(|cell| cell.borrow().clone());
    match factory {
        Some(factory) => create(factory.as_ref()),
        None => {
            let thread = std::thread::current();
            panic!(
                "elwindui: cannot create the native peer for `{control}`: no native backend is \
                 registered on thread {:?} ({:?}). Call `elwindui::init()` on the UI thread before \
                 showing native controls, and only use them from that thread.",
                thread.name().unwrap_or("<unnamed>"),
                thread.id(),
            )
        }
    }
}

/// Panics with the standard "unsupported control" diagnostic. Used by the default
/// [`NativePeerFactory`] methods.
pub fn unsupported_native_control(backend: &str, control: &str) -> ! {
    panic!(
        "elwindui: the `{backend}` backend does not support the native control `{control}`. \
         No fallback is used; use a backend that implements `{control}`."
    )
}

/// Reconciles an ordered native list with `new` by `Rc` identity, for peers whose native list
/// can only append and remove (§7c ordered-list contract). When the entries kept from `old`
/// are, in order, a prefix of `new`, it removes the dropped entries and appends the rest of
/// `new`, leaving every kept native entry untouched. Otherwise (an insertion before a kept entry,
/// or a reorder) appending would leave the native order different from `new`, so it removes every
/// entry of `old` and appends all of `new` in order.
pub fn reconcile_ordered_peers<T: ?Sized>(
    old: &[Rc<T>],
    new: &[Rc<T>],
    mut remove: impl FnMut(&Rc<T>),
    mut append: impl FnMut(&Rc<T>),
) {
    let same = |a: &Rc<T>, b: &Rc<T>| std::ptr::addr_eq(Rc::as_ptr(a), Rc::as_ptr(b));
    let kept: Vec<&Rc<T>> = old
        .iter()
        .filter(|entry| new.iter().any(|candidate| same(entry, candidate)))
        .collect();
    // Lists are distinct by contract (`ChildList::assert_absent`). The length check keeps a list
    // that breaks it on the full-rebuild path instead of slicing `new` out of range.
    let kept_is_prefix = kept.len() <= new.len()
        && kept
            .iter()
            .zip(new.iter())
            .all(|(kept, candidate)| same(kept, candidate));
    if kept_is_prefix {
        for entry in old
            .iter()
            .filter(|entry| !new.iter().any(|candidate| same(entry, candidate)))
        {
            remove(entry);
        }
        for entry in &new[kept.len()..] {
            append(entry);
        }
    } else {
        for entry in old {
            remove(entry);
        }
        for entry in new {
            append(entry);
        }
    }
}
