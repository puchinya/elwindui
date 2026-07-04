//! Same notepad UI as `examples/notepad`, but using the proc-macro embedding path
//! (`elwindui::component! { ... }`) instead of build.rs + separate `.elwind` files — the
//! Slint-`slint!`-style alternative described in docs/elwindui_gui_framework_design.md §1 /
//! docs/elwindui_spec.md 付録B.1.

use elwindui::platform;

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

elwindui::component! {
    enum SaveState { Unsaved, Saving, Saved }

    viewmodel NotepadViewModel {
        #[observable]
        #[length(0..=100000)]
        content: String = String::new(),

        #[observable]
        file_name: String = "untitled.txt",

        #[observable]
        current_path: String = String::new(),

        #[observable]
        state: SaveState = SaveState::Unsaved,

        #[computed]
        char_count: i32 = content.chars().count() as i32,

        #[computed]
        window_title: String = t!("notepad-window-title", file_name: file_name),

        #[command(async, can_execute: state != SaveState::Saving)]
        save: Command = command!(async || {
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
        }),

        #[command(async)]
        open: Command = command!(async || {
            if let Some(path) = platform::file_dialog::open().await {
                content = std::fs::read_to_string(&path).unwrap_or_default();
                file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                current_path = path.to_string_lossy().to_string();
                state = SaveState::Unsaved;
            }
        }),
    }

    component NotepadWindow {
        #[param]
        #[inject]
        vm: NotepadViewModel,

        content: String = bind!(vm.content, TwoWay),
    }

    view NotepadWindow {
        Window {
            title: vm.window_title

            Column {
                Row {
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

                TextArea { text: content }

                Row {
                    Text { text: t!("notepad-status-chars", count: vm.char_count) }
                }
            }
        }
    }
}

fn main() {
    let vm = NotepadViewModel::new();
    let window = NotepadWindow::new(vm);
    window.open();
}
