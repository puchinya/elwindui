//! WinUI 3 (Windows App SDK) implementation of the widget surface `elwindui-codegen` targets,
//! mirroring `elwindui-backend-appkit`'s shape (see that crate's doc comment for the overall
//! native-vs-virtual design this implements: `VerticalLayout`/`HorizontalLayout`/
//! `Rectangle`/`Ellipse`/`TextBlock` have no widget here at all, just `elwindui_core::ui::UIElement`
//! values `elwindui-codegen` builds directly (`TextBlock` is self-drawn, using the real XAML
//! `TextBlock` class only as a paint primitive inside `TreeHostPanel::relayout_static`, never as a
//! wrapped builtin widget — see `elwindui-backend-appkit`'s `CATextLayer` use for the same role);
//! only `Window`/`Button`/`TextArea`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`NativeTabView` are real
//! native widgets).
//!
//! Split into `inner` (private — raw WinRT/XAML plumbing, `Inner`-prefixed types) and `native_ui`
//! (public, re-exported here — implements every `elwindui_core::ui` builtin trait this backend
//! provides by composing the matching `inner` type). See each module's own doc comment — mirrors
//! `elwindui-backend-appkit`'s own split exactly.
//!
//! The WinUI projection is generated at build time from the Windows App SDK metadata. `build.rs`
//! resolves the metadata from `WINDOWS_APP_SDK_WINMD` or a normal NuGet package-cache install.

#![cfg(target_os = "windows")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line —
// `elwindui-backend-appkit` carries the identical `#![allow(...)]` for the identical reason.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, clippy::all)]
mod bindings {
    include!(env!("ELWINDUI_WINUI3_BINDINGS"));
}

#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code)]
mod xaml_interop {
    include!(concat!(env!("OUT_DIR"), "/xaml_interop.rs"));
}

#[allow(unused_imports)]
pub(crate) use xaml_interop::Windows;

mod inner;
mod native_ui;

pub use native_ui::*;

// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly (see
// `inner::AnyView`'s own doc comment), so it needs to stay reachable at this crate's own root even
// though the rest of `inner` is private.
pub use inner::AnyView;

/// Initializes the Windows App SDK dynamic dependency for an unpackaged process.
///
/// Call this through [`elwindui::init`](https://docs.rs/elwindui) before constructing the first
/// WinUI 3 object. The operation is idempotent; the App SDK bootstrap remains active until process
/// exit, which is the lifetime required by WinUI 3 and Win2D objects.
pub fn init() -> windows::core::Result<()> {
    use std::sync::OnceLock;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{Error, HRESULT, PCWSTR, s, w};

    // COM apartments are thread-local, while the App SDK dynamic dependency is process-wide.
    // `init` deliberately keeps COM initialized for this UI thread until process exit; XAML must
    // subsequently be created on the same STA thread by `application::run`.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };

    static BOOTSTRAP: OnceLock<std::result::Result<(), HRESULT>> = OnceLock::new();
    let result = BOOTSTRAP.get_or_init(|| unsafe {
        let module = LoadLibraryW(w!("Microsoft.WindowsAppRuntime.Bootstrap.dll"))
            .map_err(|error| error.code())?;
        let proc = GetProcAddress(module, s!("MddBootstrapInitialize"))
            .ok_or_else(|| Error::from_thread().code())?;
        type BootstrapInitialize = unsafe extern "system" fn(u32, PCWSTR, u64) -> HRESULT;
        let initialize: BootstrapInitialize = std::mem::transmute(proc);
        // Windows App SDK 1.8, stable channel. A zero minimum version asks the bootstrapper for
        // the installed compatible package rather than pinning a patch release.
        let result = initialize((1 << 16) | 8, PCWSTR::null(), 0);
        result.ok().map_err(|error| error.code())
    });
    result.map_err(Error::from_hresult)
}

/// See docs/elwindui_spec.md 付録T.2 — same async-shaped-but-synchronous-underneath API as
/// AppKit's `platform::file_dialog` (`IFileOpenDialog`/`IFileSaveDialog::Show` block the calling
/// thread until the user closes the dialog; there's no genuine suspend point). Uses the classic
/// Win32 common file dialog COM interfaces (`Win32_UI_Shell` — present in the mainstream `windows`
/// crate) rather than the WinRT `Windows.Storage.Pickers` pickers, since those need
/// `IInitializeWithWindow` interop to attach to a non-UWP top-level `HWND`, which is extra
/// complexity this skips in favor of a path more likely to actually compile as written.
pub mod platform {
    pub mod file_dialog {
        use std::path::PathBuf;
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        };
        use windows::Win32::UI::Shell::{
            FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
        };

        fn ensure_com_initialized() {
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
    }
}

/// WinUI3's `Dispatcher` (docs/elwindui_spec.md 付録P.5): hops back to the UI thread via the
/// current thread's `DispatcherQueue` — the WinUI3/WinAppSDK analog of AppKit's
/// `dispatch2::DispatchQueue::main()`. `application::run()` (below) is what pumps this queue as
/// part of its own message loop, so a job enqueued from any thread is guaranteed to run promptly.
pub struct WinUI3Dispatcher {
    queue: bindings::Microsoft::UI::Dispatching::DispatcherQueue,
}

impl elwindui_core::task::Dispatcher for WinUI3Dispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        let job = std::cell::RefCell::new(Some(job));
        let _ = self.queue.TryEnqueue(
            &bindings::Microsoft::UI::Dispatching::DispatcherQueueHandler::new(move || {
                if let Some(job) = job.borrow_mut().take() {
                    job();
                }
                Ok(())
            }),
        );
    }
}

/// The single entry point that owns "enter the platform message loop" — kept separate from
/// `Window::show()` for the same reason as `elwindui-backend-appkit`'s `application::run()` (see
/// that module's doc comment): it's the one well-defined place to install the task executor before
/// any generated code runs.
pub mod application {
    use super::{WinUI3Dispatcher, bindings};
    use bindings::Microsoft::UI::Xaml::{
        Application, IApplicationFactory, IApplicationOverrides, IApplicationOverrides_Impl,
        LaunchActivatedEventArgs,
    };
    use bindings::Microsoft::UI::Xaml::Markup::{
        IXamlMetadataProvider, IXamlMetadataProvider_Impl, IXamlType, XmlnsDefinition,
    };
    use bindings::Microsoft::UI::Xaml::XamlTypeInfo::XamlControlsXamlMetaDataProvider;
    use elwindui_core::task::LocalExecutor;
    use std::cell::RefCell;
    use windows_core::{IInspectable, Interface, Type};

    thread_local! {
        // The generated callback wrapper requires its closure to be `Send`, whereas startup is
        // intentionally UI-thread-local. Keeping it in TLS means the callback captures nothing
        // and startup never acquires an incorrect `Send` bound.
        static STARTUP: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
        // `Application::Start` does not retain the Rust wrapper. Hold it on the UI thread until
        // the XAML event loop exits.
        static XAML_APPLICATION: RefCell<Option<RetainedApplication>> =
            const { RefCell::new(None) };
        static WINDOWS: RefCell<Vec<RetainedWindow>> = const { RefCell::new(Vec::new()) };
        static NEXT_WINDOW_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    struct RetainedWindow {
        id: u64,
        _window: bindings::Microsoft::UI::Xaml::Window,
    }

    // WinUI's application factory needs an aggregate that supplies both lifecycle overrides and
    // XAML metadata. A plain `Application::new()` lacks those interfaces and terminates inside
    // Microsoft.UI.Xaml before a window can be created.
    #[windows_core::implement(IApplicationOverrides, IXamlMetadataProvider)]
    struct XamlApplication {
        provider: XamlControlsXamlMetaDataProvider,
    }

    impl IApplicationOverrides_Impl for XamlApplication_Impl {
        fn OnLaunched(
            &self,
            _args: windows_core::Ref<'_, LaunchActivatedEventArgs>,
        ) -> windows_core::Result<()> {
            Ok(())
        }
    }

    impl IXamlMetadataProvider_Impl for XamlApplication_Impl {
        fn GetXamlType(
            &self,
            r#type: &crate::Windows::UI::Xaml::Interop::TypeName,
        ) -> windows_core::Result<IXamlType> {
            self.provider.GetXamlType(r#type)
        }

        fn GetXamlTypeByFullName(
            &self,
            full_name: &windows_core::HSTRING,
        ) -> windows_core::Result<IXamlType> {
            self.provider.GetXamlTypeByFullName(full_name)
        }

        fn GetXmlnsDefinitions(&self) -> windows_core::Result<windows_core::Array<XmlnsDefinition>> {
            self.provider.GetXmlnsDefinitions()
        }
    }

    struct RetainedApplication {
        application: Application,
        _outer: IInspectable,
        _inner: IInspectable,
    }

    fn compose_application() -> windows_core::Result<RetainedApplication> {
        let outer: IInspectable = XamlApplication {
            provider: XamlControlsXamlMetaDataProvider::new()?,
        }
        .into();
        let factory = windows_core::factory::<Application, IApplicationFactory>()?;
        unsafe {
            let mut inner = core::ptr::null_mut();
            let mut application = core::ptr::null_mut();
            (Interface::vtable(&factory).CreateInstance)(
                Interface::as_raw(&factory),
                Interface::as_raw(&outer),
                &mut inner,
                &mut application,
            )
            .ok()?;
            Ok(RetainedApplication {
                application: Type::from_abi(application)?,
                _outer: outer,
                _inner: Type::from_abi(inner)?,
            })
        }
    }

    pub(crate) fn retain_window(window: &bindings::Microsoft::UI::Xaml::Window) {
        let id = NEXT_WINDOW_ID.with(|next| {
            let id = next.get();
            next.set(id.wrapping_add(1));
            id
        });
        let closed = windows::Foundation::TypedEventHandler::new(move |_, _| {
            release_window(id);
            Ok(())
        });
        window
            .Closed(&closed)
            .expect("Window::Closed event registration");
        WINDOWS.with(|windows| {
            windows.borrow_mut().push(RetainedWindow { id, _window: window.clone() });
        });
    }

    fn release_window(id: u64) {
        let has_windows = WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            windows.retain(|entry| entry.id != id);
            !windows.is_empty()
        });
        if !has_windows {
            XAML_APPLICATION.with(|slot| {
                if let Some(application) = slot.borrow().as_ref() {
                    application
                        .application
                        .Exit()
                        .expect("Microsoft.UI.Xaml.Application::Exit");
                }
            });
        }
    }

    /// Runs `startup` from WinUI's application-initialization callback, then lets
    /// `Microsoft.UI.Xaml.Application::Start` own the native message loop.
    pub fn run<F>(startup: F)
    where
        F: FnOnce() + 'static,
    {
        STARTUP.with(|slot| {
            assert!(slot.borrow().is_none(), "elwindui::application::run may only be called once");
            *slot.borrow_mut() = Some(Box::new(startup));
        });

        let callback = bindings::Microsoft::UI::Xaml::ApplicationInitializationCallback::new(
            move |_| {
                let application = compose_application()?;
                XAML_APPLICATION.with(|slot| *slot.borrow_mut() = Some(application));

                let queue = bindings::Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread()?;
                elwindui_core::task::set_current(LocalExecutor::new(WinUI3Dispatcher { queue }));

                STARTUP.with(|slot| {
                    if let Some(startup) = slot.borrow_mut().take() {
                        startup();
                    }
                });
                Ok(())
            },
        );
        bindings::Microsoft::UI::Xaml::Application::Start(&callback)
            .expect("Microsoft.UI.Xaml.Application::Start");
    }
}
