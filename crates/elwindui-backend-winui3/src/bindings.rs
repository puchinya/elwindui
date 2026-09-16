//! The generated WinRT projection.
//!
//! `build.rs` runs `windows-bindgen` over the Windows App SDK / Windows SDK `.winmd` files and
//! writes both halves to `$OUT_DIR` and mirrors them into the ignored crate-local `.generated`
//! directory; the stable mirror is `include!`d here rather than checked in. Keeping the source
//! path stable lets rust-analyzer inspect the same generated projection as rustc. Kept out of
//! `lib.rs` so the crate root stays pure wiring.

#[allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    dead_code,
    clippy::all
)]
mod generated {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.generated/bindings.rs"
    ));
}
pub(crate) use generated::*;
// The generated WinUI projection and the separately-generated XAML interop projection both expose
// a top-level `Windows` module. Keep the value types used by WinUI text properties available under
// an unambiguous crate-private alias instead of accidentally importing the interop-only module.
pub(crate) use generated::Windows::UI::Text as winui_text;

#[allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    dead_code
)]
pub(crate) mod xaml_interop {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.generated/xaml_interop.rs"
    ));
}
#[allow(unused_imports)]
pub(crate) use xaml_interop::Windows;
