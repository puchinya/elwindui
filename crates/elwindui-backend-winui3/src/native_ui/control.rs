//! `NativeControl` — the base class every native leaf in this backend inherits from.
//!
//! Mirrors `elwindui-backend-appkit::native_ui::control` file-for-file (`lib.rs`'s own doc
//! comment) — see that file's identical doc comments for the full rationale behind the
//! `text_style`/`applied`/`sync_text_style` shape below. **Unverifiable on this machine**
//! (`#![cfg(target_os = "windows")]`, `docs/elwindui_font_status.md` §6/§9): written to the same
//! shape as the AppKit side, but never compiled or type-checked here.

use crate::AnyView;
use elwindui_core::graphics::{ComputedTextStyle, TextStyleStorage};
use elwindui_core::ui::{TextStyleOwner, UIElementExt};
use std::any::Any;
use std::cell::RefCell;

/// The backend-owned counterpart to `elwindui_core::ui::NativeControl` (a pure marker trait with no
/// backing struct of its own — measuring/placing a native handle is entirely backend-specific, so
/// `elwindui-core` doesn't define this generically). Holding `handle: AnyView` here once, instead
/// of on each of `TextArea`/`Button`/`TabView` individually, is what lets `inherits = NativeControl`
/// resolve `base`'s field type to this same struct.
#[elwindui_macros::class(struct_only = elwindui_core::ui::NativeControlExt, inherits = elwindui_core::ui::UIElement)]
pub struct NativeControl {
    handle: AnyView,
    text_style: TextStyleStorage,
    /// The style last actually pushed to `handle` — lets `sync_text_style` skip a redundant
    /// XAML call when nothing changed since the previous measure pass.
    applied: RefCell<Option<ComputedTextStyle>>,
}

#[elwindui_macros::class]
impl NativeControl {
    #[overrides]
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.sync_text_style();
        self.handle.measure(available)
    }
    #[overrides]
    fn try_as_native_control(&self) -> Option<&dyn Any> {
        Some(&self.handle)
    }
    #[overrides]
    fn as_text_style_owner(&self) -> Option<&dyn TextStyleOwner> {
        Some(self)
    }
    fn construct(handle: AnyView) -> Self {
        Self {
            base: elwindui_core::ui::UIElement::construct(),
            handle,
            text_style: TextStyleStorage::new(),
            applied: RefCell::new(None),
        }
    }
}

impl NativeControl {
    /// See `elwindui-backend-appkit::native_ui::control::NativeControl::sync_text_style`'s own
    /// doc comment for the full pull-based rationale — identical here.
    pub(crate) fn sync_text_style(&self) {
        if !self.handle.supports_text_style() {
            return;
        }
        let computed = self.resolved_text_style();
        if self.applied.borrow().as_ref() != Some(&computed) {
            // Only cache a style after the native handle accepted it. Otherwise a transient XAML
            // failure would permanently suppress retries on later layout passes.
            if self.handle.apply_text_style(&computed).is_ok() {
                *self.applied.borrow_mut() = Some(computed);
            }
        }
    }
}

impl TextStyleOwner for NativeControl {
    fn text_style_storage(&self) -> &TextStyleStorage {
        &self.text_style
    }
}
