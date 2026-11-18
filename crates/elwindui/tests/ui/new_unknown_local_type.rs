// PR #312 contract v3 (D3): `new!` of a same-crate qualified path that is neither a registered
// generated Component nor a class-backed Component with a constructor ABI stays a static error.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![allow(unexpected_cfgs)]

fn main() {
    let _ = elwindui::new!(crate::NoSuchComponent(note: "x"));
}
