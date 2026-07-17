//! Same notepad UI as `examples/notepad`, but with everything embedded inline instead of build.rs
//! + separate `.elwind` files: the viewmodel via `#[elwindui::viewmodel]` (a real Rust
//! `struct`+`impl`, see docs/elwindui_spec.md 付録O.2 and `elwindui_codegen::attr_frontend`), and
//! the component/view via `#[elwindui::component(inherits Window)]` on an ordinary `struct` (see
//! `elwindui_codegen::component_frontend`). The `view { ... }` element-tree DSL still isn't valid
//! Rust *expression* syntax, so it can't become a real field *value* — but it can be a field's
//! *type*, spelled `view! { ... }`: a macro invocation is legal Rust in type position, and because
//! `#[elwindui::component]` (an attribute macro) replaces the whole annotated `struct`, that inner
//! `view!` invocation never survives to be expanded — `view` isn't even a real macro anywhere. Its
//! tokens are read back out as `.elwind`-DSL text instead, the same way the (now removed)
//! `elwindui::component!` bang macro treated its whole input as DSL text.

// `#[elwindui::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate macro-to-macro
// reference (`$crate::the_macro!`) to also work cross-crate, which currently requires this lint
// disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` (including via `#[elwindui::component]`, as here) with a same-crate
// `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::platform;
use elwindui::ui::WindowExt;

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
                    file_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
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
                file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                current_path = path.to_string_lossy().to_string();
                state = SaveState::Unsaved;
            }
        }
    }
}

#[elwindui::component(inherits Window)]
struct NotepadWindow {
    #[bindable]
    vm: std::rc::Rc<NotepadViewModel>,

    body: view! {
        title: vm.window_title

        // `Grid` (not `VerticalLayout`) so `TextArea` gets the window's remaining space instead of
        // only its own natural height — `VerticalLayout`'s main axis is always "Auto" sizing, same
        // reasoning as `examples/notepad/src/ui/document_view.elwind`'s own `Grid` usage.
        content: Grid {
            rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0), elwindui::core::layout::GridLength::Auto]
            columns: [elwindui::core::layout::GridLength::Star(1.0)]
            HorizontalLayout {
                Grid::row: 0
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

            TextArea { text: vm.content, Grid::row: 1 }

            HorizontalLayout {
                Grid::row: 2
                TextBlock { text: t!("notepad-status-chars", count: vm.char_count) }
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
