// `#[elwindui::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate macro-to-macro
// reference (`$crate::the_macro!`) to also work cross-crate, which currently requires this lint
// disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` (including via `elwindui-codegen`'s generated code, as here) with a
// same-crate `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::platform;
use elwindui::ui::WindowExt;

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
        close_active_tab: Command,

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

        fn close_active_tab(&self) {
            self.close_tab_execute(active_tab);
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
                    doc.set_file_name(
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    );
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
                doc.set_file_name(
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                );
                doc.set_current_path(path.to_string_lossy().to_string());
                doc.set_state(SaveState::Unsaved);
            }
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/rounded_panel.rs"));
include!(concat!(env!("OUT_DIR"), "/document_view.rs"));
include!(concat!(env!("OUT_DIR"), "/notepad_window.rs"));

fn main() {
    elwindui::i18n::declare!();

    let vm = NotepadViewModel::new();
    // Always start with one open tab — `close_tab` refuses to remove the last one, but nothing
    // stops `documents` from being empty right after construction otherwise, and several viewmodel
    // expressions (e.g. `save`'s can_execute) as well as `TabView`'s active-tab lookup assume at
    // least one document exists.
    vm.new_tab_execute();
    let window = NotepadWindow::new(vm);
    window.show();

    elwindui::application::run();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression test for the `content` binding: model mutations must publish a typed
    // PropertyChanged event to every view that depends on the property.
    #[test]
    fn observable_setter_notifies_subscribers() {
        let doc = DocumentViewModel::new();
        let notified = std::rc::Rc::new(std::cell::Cell::new(false));
        let notified_handle = notified.clone();
        let _subscription = doc.subscribe_property_changed(move |_| notified_handle.set(true));

        assert!(!notified.get(), "must not fire before any mutation");
        doc.set_content("loaded from disk".to_string());
        assert!(notified.get(), "set_content must notify subscribers");
        assert_eq!(doc.content(), "loaded from disk");
    }

    #[test]
    fn multiple_subscribers_all_fire() {
        let doc = DocumentViewModel::new();
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut subscriptions = Vec::new();
        for _ in 0..3 {
            let calls = calls.clone();
            subscriptions.push(doc.subscribe_property_changed(move |property| {
                if property == DocumentViewModelProperty::content {
                    calls.set(calls.get() + 1);
                }
            }));
        }
        doc.set_content("x".to_string());
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn dropped_property_changed_subscription_stops_receiving_events() {
        let doc = DocumentViewModel::new();
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        {
            let calls = calls.clone();
            let _subscription = doc.subscribe_property_changed(move |_| calls.set(calls.get() + 1));
        }
        doc.set_content("ignored".to_string());
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn cancelling_during_notification_skips_the_cancelled_handler() {
        let doc = DocumentViewModel::new();
        let cancelled_calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let later_subscription = std::rc::Rc::new(std::cell::RefCell::new(
            None::<elwindui::core::reactive::Subscription>,
        ));

        let subscription_to_cancel = later_subscription.clone();
        let _first = doc.subscribe_property_changed(move |_| {
            if let Some(subscription) = subscription_to_cancel.borrow_mut().take() {
                subscription.cancel();
            }
        });
        let cancelled_calls_for_handler = cancelled_calls.clone();
        *later_subscription.borrow_mut() = Some(doc.subscribe_property_changed(move |_| {
            cancelled_calls_for_handler.set(cancelled_calls_for_handler.get() + 1);
        }));

        doc.set_content("x".to_string());
        assert_eq!(cancelled_calls.get(), 0);
    }
}
