fn main() {
    println!("cargo:rerun-if-changed=src/ui");
    println!("cargo:rerun-if-changed=src/main.rs");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    // `src/main.rs` defines `NotepadViewModel`/`Document` via `#[elwindui::viewmodel]` (ordinary
    // Rust, not `.elwind`) — passing it here lets `notepad_window.elwind`'s `vm.documents` /
    // `vm.save.execute()` / `vm.save.can_execute` references get checked against their real
    // shape instead of going unvalidated until `rustc` hits a missing-method error.
    elwindui_codegen::compile_dir_with_extra_viewmodels("src/ui", &out_dir, &["src/main.rs"])
        .expect("compiling .elwind sources");
}
