use elwindui::platform;

mod elwindui_i18n {
    include!(concat!(env!("OUT_DIR"), "/i18n_support.rs"));
}

include!(concat!(env!("OUT_DIR"), "/notepad_viewmodel.rs"));
include!(concat!(env!("OUT_DIR"), "/notepad_window.rs"));

fn main() {
    let vm = NotepadViewModel::new();
    // Always start with one open tab — `close_tab` refuses to remove the last one, but nothing
    // stops `documents` from being empty right after construction otherwise, and several viewmodel
    // expressions (e.g. `save`'s can_execute) as well as `TabView`'s active-tab lookup assume at
    // least one document exists.
    vm.new_tab_execute();
    let window = NotepadWindow::new(vm);
    window.open();
}
