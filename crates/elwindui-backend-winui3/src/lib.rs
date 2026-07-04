//! WinUI 3 (Windows App SDK) implementation of `elwindui-core`'s backend traits, Windows-only.
//! See docs/elwindui_spec.md 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! Left as a stub: the mainstream `windows` crate (windows-rs) does not expose XAML UI control
//! bindings (`Microsoft.UI.Xaml`/`Windows.UI.Xaml.Controls`) — its ~691 features cover Win32 and
//! WinRT APIs generated from the Windows SDK metadata, but not the separate Windows App SDK/WinUI3
//! metadata that `Button`/`StackPanel`/`TextBox` etc. come from (confirmed by inspecting the
//! crate's published feature list; no `UI_Xaml*` feature exists). A real implementation needs
//! either a dedicated WinUI3 bindings crate or custom `windows-bindgen` metadata for the Windows
//! App SDK, and can only be built/verified on Windows.
