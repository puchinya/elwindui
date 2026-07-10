use elwindui::platform;
use elwindui_backend_appkit::builtins::{Menu, MenuBar, MenuBarItem, MenuItem, TabView, TextArea, Window, WindowImpl};
// Required by `ContentControl`'s generated code (`content: Rc<dyn UIElement>` — see
// `content_control.elwind`'s own doc comment, docs/elwindui_spec.md 付録H.2.1a).
use elwindui_core::tree::UIElement;

mod elwindui_i18n {
    include!(concat!(env!("OUT_DIR"), "/i18n_support.rs"));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveState {
    Unsaved,
    Saving,
    Saved,
}

// One open document. Held as `Rc<DocumentViewModel>` inside `NotepadViewModel.documents` (see
// docs/elwindui_builtins_spec.md 付録Y.2) so each tab's edits reach the same shared instance
// rather than a throwaway clone.
#[elwindui::viewmodel]
mod document_view_model {
    struct DocumentViewModel {
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
    }
}

#[elwindui::viewmodel]
mod notepad_view_model {
    struct NotepadViewModel {
        #[observable(default = Vec::new())]
        documents: Vec<DocumentViewModel>,

        #[observable(default = 0usize)]
        active_tab: usize,

        #[command]
        new_tab: Command,

        #[command]
        close_tab: Command,

        #[command]
        select_tab: Command,

        // `documents.len() > 0` (rather than indexing into the active document's `state`) so this
        // is safe to evaluate even in the brief window right after construction, before `main.rs`
        // has called `new_tab_execute()` to open the first tab.
        #[command(can_execute = documents.len() > 0)]
        save: Command,

        #[command]
        open: Command,
    }

    impl NotepadViewModel {
        fn new_tab(&self) {
            documents.push(DocumentViewModel::new());
            active_tab = documents.len() - 1;
        }

        fn close_tab(&self, index: usize) {
            if documents.len() > 1 {
                documents.remove(index);
                if active_tab >= documents.len() {
                    active_tab = documents.len() - 1;
                }
            }
        }

        fn select_tab(&self, index: usize) {
            active_tab = index;
        }

        async fn save(&self) {
            let doc = documents[active_tab].clone();
            doc.set_state(SaveState::Saving);
            let path = if doc.current_path().is_empty() {
                platform::file_dialog::save().await
            } else {
                Some(std::path::PathBuf::from(doc.current_path()))
            };
            match path {
                Some(path) => {
                    let _ = std::fs::write(&path, doc.content());
                    doc.set_file_name(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                    doc.set_current_path(path.to_string_lossy().to_string());
                    doc.set_state(SaveState::Saved);
                }
                None => {
                    doc.set_state(SaveState::Unsaved);
                }
            }
        }

        async fn open(&self) {
            if let Some(path) = platform::file_dialog::open().await {
                let doc = documents[active_tab].clone();
                doc.set_content(std::fs::read_to_string(&path).unwrap_or_default());
                doc.set_file_name(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                doc.set_current_path(path.to_string_lossy().to_string());
                doc.set_state(SaveState::Unsaved);
            }
        }
    }
}

// Real generated Rust for any `elwindui-builtins` component composed via its own `view`
// (`ContentControl`/`Rectangle`/`Ellipse` — docs/elwindui_spec.md 付録H.2.1a) — `elwindui-builtins`
// has no `build.rs` of its own to emit this centrally, so `elwindui-codegen`'s `compile_dir_impl`
// generates it here for every consumer that needs it (`RoundedPanel` below is the first one to).
include!(concat!(env!("OUT_DIR"), "/elwindui_builtins_generated.rs"));

include!(concat!(env!("OUT_DIR"), "/rounded_panel.rs"));
include!(concat!(env!("OUT_DIR"), "/document_view.rs"));
include!(concat!(env!("OUT_DIR"), "/notepad_window.rs"));

fn main() {
    let vm = NotepadViewModel::new();
    // Always start with one open tab — `close_tab` refuses to remove the last one, but nothing
    // stops `documents` from being empty right after construction otherwise, and several viewmodel
    // expressions (e.g. `save`'s can_execute) as well as `TabView`'s active-tab lookup assume at
    // least one document exists.
    vm.new_tab_execute();
    let window = NotepadWindowImpl::new(vm);
    window.show();
    elwindui::application::run();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression test for the `content`-binding-doesn't-refresh bug: `set_content` (however it's
    // reached — here, directly, the same way `NotepadViewModel::open()` reaches it) must notify
    // every `subscribe`r, not just whichever component happened to wire an `on_*` callback to it.
    #[test]
    fn observable_setter_notifies_subscribers() {
        let doc = DocumentViewModel::new();
        let notified = std::rc::Rc::new(std::cell::Cell::new(false));
        let notified_handle = notified.clone();
        doc.subscribe(move || notified_handle.set(true));

        assert!(!notified.get(), "must not fire before any mutation");
        doc.set_content("loaded from disk".to_string());
        assert!(notified.get(), "set_content must notify subscribers");
        assert_eq!(doc.content(), "loaded from disk");
    }

    #[test]
    fn multiple_subscribers_all_fire() {
        let doc = DocumentViewModel::new();
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        for _ in 0..3 {
            let calls = calls.clone();
            doc.subscribe(move || calls.set(calls.get() + 1));
        }
        doc.set_content("x".to_string());
        assert_eq!(calls.get(), 3);
    }
}
