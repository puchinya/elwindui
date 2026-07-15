//! Same notepad UI as `examples/notepad`, but with everything embedded inline instead of build.rs
//! + separate `.elwind` files: the viewmodel via `#[elwindui::viewmodel]` (a real Rust
//! `struct`+`impl`, see docs/elwindui_spec.md 付録O.2 and `elwindui_codegen::attr_frontend`), and
//! the view via `elwindui::component! { ... }` (still needed for `view { ... }` element trees,
//! which aren't valid Rust expression syntax — that half can't move to plain Rust).

// `#[elwindui::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate macro-to-macro
// reference (`$crate::the_macro!`) to also work cross-crate, which currently requires this lint
// disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` (including via `elwindui::component!`, as here) with a same-crate
// `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveState {
    Unsaved,
    Saving,
    Saved,
}

#[elwindui::viewmodel]
mod notepad_view_model {
    struct NotepadViewModel {
        #[observable(default = String::new())]
        #[length(0..=100000)]
        content: String,

        #[observable(default = "untitled.txt")]
        file_name: String,

        #[observable(default = String::new())]
        current_path: String,

        #[observable(default = SaveState::Unsaved)]
        state: SaveState,

        #[computed(expr = content.chars().count() as i32)]
        char_count: i32,

        #[computed(expr = t!("notepad-window-title", file_name: file_name))]
        window_title: String,

        #[command(can_execute = state != SaveState::Saving)]
        save: Command,

        #[command]
        open: Command,
    }

    impl NotepadViewModel {
        async fn save(&self) {
            state = SaveState::Saving;
            let path = if current_path.is_empty() {
                platform::file_dialog::save().await
            } else {
                Some(std::path::PathBuf::from(current_path.clone()))
            };
            match path {
                Some(path) => {
                    let _ = std::fs::write(&path, content.clone());
                    file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    current_path = path.to_string_lossy().to_string();
                    state = SaveState::Saved;
                }
                None => {
                    state = SaveState::Unsaved;
                }
            }
        }

        async fn open(&self) {
            if let Some(path) = platform::file_dialog::open().await {
                content = std::fs::read_to_string(&path).unwrap_or_default();
                file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                current_path = path.to_string_lossy().to_string();
                state = SaveState::Unsaved;
            }
        }
    }
}

elwindui::component! {
    component NotepadWindow inherits Window {
        #[param]
        #[inject]
        vm: std::rc::Rc<NotepadViewModel>,
    }

    view NotepadWindow {
        Window {
            title: vm.window_title

            content: VerticalLayout {
                HorizontalLayout {
                    Button {
                        text: t!("notepad-menu-save")
                        on_click: vm.save.execute()
                        enabled: vm.save.can_execute
                    }
                    Button {
                        text: t!("notepad-menu-open")
                        on_click: vm.open.execute()
                    }
                }

                TextArea { text: vm.content }

                HorizontalLayout {
                    TextBlock { text: t!("notepad-status-chars", count: vm.char_count) }
                }
            }
        }
    }
}

fn main() {
    elwindui::i18n::declare!();

    let vm = NotepadViewModel::new();
    let window = NotepadWindow::new(vm);
    window.show();
    elwindui::application::run();
}
