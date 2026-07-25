//! See docs/elwindui_spec.md 付録T.2. Modal file panels (`runModal`) are themselves synchronous
//! (they block until the user closes the panel), so these `async fn`s never actually suspend —
//! they resolve on the first poll. That's enough for `#[command(async)]` bodies that just need to
//! `.await` a dialog result; it is not a general-purpose async executor (nothing here can yield
//! across a real I/O wait), which is what `elwindui-core`'s planned `Dispatcher`/`spawn`
//! (docs/elwindui_gui_framework_design.md §7.3) is for.

use crate::ffi::mtm;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use std::path::PathBuf;

pub async fn open() -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm());
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel
        .URL()
        .and_then(|url| url.path())
        .map(|p| PathBuf::from(p.to_string()))
}

pub async fn save() -> Option<PathBuf> {
    let panel = NSSavePanel::savePanel(mtm());
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel
        .URL()
        .and_then(|url| url.path())
        .map(|p| PathBuf::from(p.to_string()))
}
