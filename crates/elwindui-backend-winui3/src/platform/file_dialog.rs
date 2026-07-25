//! See docs/elwindui_spec.md 付録T.2 — same async-shaped-but-synchronous-underneath API as
//! AppKit's `platform::file_dialog` (`IFileOpenDialog`/`IFileSaveDialog::Show` block the calling
//! thread until the user closes the dialog; there's no genuine suspend point). Uses the classic
//! Win32 common file dialog COM interfaces (`Win32_UI_Shell` — present in the mainstream `windows`
//! crate) rather than the WinRT `Windows.Storage.Pickers` pickers, since those need
//! `IInitializeWithWindow` interop to attach to a non-UWP top-level `HWND`, which is extra
//! complexity this skips in favor of a path more likely to actually compile as written.

use std::path::PathBuf;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
};

pub(crate) fn ensure_com_initialized() {
    unsafe {
        // Ignore the result: `RPC_E_CHANGED_MODE`/`S_FALSE` both mean COM is already
        // initialized on this thread (fine — this only ever runs on the UI thread), and
        // any other failure surfaces later as the dialog itself failing to create.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

pub async fn open() -> Option<PathBuf> {
    ensure_com_initialized();
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.Show(None).ok()?;
        let item = dialog.GetResult().ok()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        Some(PathBuf::from(path.to_string().ok()?))
    }
}

pub async fn save() -> Option<PathBuf> {
    ensure_com_initialized();
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.Show(None).ok()?;
        let item = dialog.GetResult().ok()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        Some(PathBuf::from(path.to_string().ok()?))
    }
}
