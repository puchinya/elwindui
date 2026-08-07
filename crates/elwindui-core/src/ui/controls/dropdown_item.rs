//! `builtin::DropdownItem` — one entry in a `Dropdown`.

/// `DropdownItem`'s own class trait. No `inherits`: like `MenuItem`/`TabViewItem`, a `DropdownItem`
/// has no native view of its own — `Dropdown` reads each item's `text` to rebuild its native
/// widget's item list, the same way `Menu` reads `MenuItem`'s own state rather than the item
/// carrying an independent `AnyView`.
#[elwindui_macros::class(trait_only, sealed)]
#[prop(text: String)]
pub trait DropdownItem {
    fn set_text(&self, text: &str);
}
