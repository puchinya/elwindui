use elwindui::platform;

mod elwindui_i18n {
    include!(concat!(env!("OUT_DIR"), "/i18n_support.rs"));
}

include!(concat!(env!("OUT_DIR"), "/notepad_viewmodel.rs"));
include!(concat!(env!("OUT_DIR"), "/notepad_window.rs"));

fn main() {
    let vm = NotepadViewModel::new();
    let window = NotepadWindow::new(vm);
    window.open();
}
