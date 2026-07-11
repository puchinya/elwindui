//! Same notepad UI as `examples/notepad`, but with everything embedded inline instead of build.rs
//! + separate `.elwind` files: the viewmodel via `#[elwindui::viewmodel]` (a real Rust
//! `struct`+`impl`, see docs/elwindui_spec.md 付録O.2 and `elwindui_codegen::attr_frontend`), and
//! the view via `elwindui::component! { ... }` (still needed for `view { ... }` element trees,
//! which aren't valid Rust expression syntax — that half can't move to plain Rust).

use elwindui::platform;
use elwindui_backend_appkit::builtins::{ButtonImpl, TextAreaImpl, Window, WindowImpl};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveState {
    Unsaved,
    Saving,
    Saved,
}

mod elwindui_i18n {
    pub use fluent_bundle::FluentValue;

    fn load_bundle() -> fluent_bundle::FluentBundle<fluent_bundle::FluentResource> {
        let ftl_string =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/strings/en.ftl")).to_string();
        let res = fluent_bundle::FluentResource::try_new(ftl_string)
            .unwrap_or_else(|(_, errors)| panic!("invalid .ftl file: {errors:?}"));
        let langid: unic_langid::LanguageIdentifier = "en".parse().expect("valid language id");
        let mut bundle = fluent_bundle::FluentBundle::new(vec![langid]);
        bundle.add_resource(res).expect("adding ftl resource");
        bundle
    }

    thread_local! {
        static BUNDLE: fluent_bundle::FluentBundle<fluent_bundle::FluentResource> = load_bundle();
    }

    pub fn t(key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
        BUNDLE.with(|bundle| {
            let mut fluent_args = fluent_bundle::FluentArgs::new();
            for (name, value) in args {
                fluent_args.set(*name, value.clone());
            }
            let msg = bundle
                .get_message(key)
                .unwrap_or_else(|| panic!("missing fluent message `{key}`"));
            let pattern = msg.value().unwrap_or_else(|| panic!("fluent message `{key}` has no value"));
            let mut errors = Vec::new();
            let result = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
            result.into_owned()
        })
    }
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
    let vm = NotepadViewModel::new();
    let window = NotepadWindowImpl::new(vm);
    window.show();
    elwindui::application::run();
}
