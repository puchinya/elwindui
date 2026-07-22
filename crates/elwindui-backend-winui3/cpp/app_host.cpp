// Minimal C++/WinRT application host for the WinUI 3 backend.
//
// Why this exists: microsoft/windows-rs#3404 — `windows-rs` has no support for WinRT "composable
// class" aggregation (subclassing a WinRT runtime class like `Application`). Two from-scratch Rust
// attempts were tried and removed (see `git log -- crates/elwindui-backend-winui3/src/composed_application.rs`
// for the deleted file's own detailed doc comment): a `#[windows_core::implement]`-based
// `Application`, and a from-scratch manual COM aggregation with correct outer->inner
// `QueryInterface` forwarding. Both compiled and ran without crashing, and both left
// `Application.Resources` reproducibly broken — any touch of it failed with
// `Error 0x80004002 in ifactory->QueryInterface(Microsoft.UI.Xaml.Media.AcrylicBrush)` — which rules
// out COM identity/forwarding as the cause and points at something `ApplicationT<App>` (cppwinrt's
// own, real, widely-used composable-class support) does that neither Rust attempt replicated.
//
// Scope (deliberately minimal — see the project's own docs/agents/winui3_current_state.md "Do not
// use a C++ adapter before exhausting this direct windows-bindgen approach": this exists only
// because that direct approach was exhausted for this one specific problem, not as a general
// precedent):
//   - Host the WinUI 3 `Application` via a real `ApplicationT<App>`.
//   - Register `XamlControlsResources` into `Application.Resources` once, in `OnLaunched`.
//   - Call back into Rust (a plain C ABI function pointer) to do everything else — window
//     creation, controls, layout, rendering, event routing all stay in Rust; nothing WinUI-specific
//     beyond hosting `Application` itself belongs here.

#include <cstdio>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.Markup.h>
#include <winrt/Microsoft.UI.Xaml.XamlTypeInfo.h>

using namespace winrt;
using namespace winrt::Microsoft::UI::Xaml;
using namespace winrt::Microsoft::UI::Xaml::Controls;
using namespace winrt::Microsoft::UI::Xaml::Markup;
using namespace winrt::Microsoft::UI::Xaml::XamlTypeInfo;

namespace {

// Set once by `elwindui_winui3_run` before `Application::Start` runs, invoked exactly once from
// `App::OnLaunched` after `Application.Resources` is ready. Everything from here on (window
// creation, the event loop's own message pumping via `Application::Start`) is Rust's/WinUI's.
void (*g_startup)() = nullptr;

// `ApplicationT<App>` alone is not enough — a real WinUI 3 project's build-generated
// `XamlMetaDataProvider` (from its XAML markup compiler step, which this backend has none of) is
// what normally supplies `IXamlMetadataProvider` alongside `IApplicationOverrides`. Without it,
// `Application::Start` crashes with the same stowed exception (`0xC000027B`) the project's own
// Rust-side history already hit for a bare, metadata-provider-less composed `Application` — see
// `Application::Current()`'s doc comment in `lib.rs`. `XamlControlsXamlMetaDataProvider` (the same
// stock type the DSL's own `.elwind` XAML-flavored markup would resolve custom types through) is
// delegated to here, mirroring the Rust `IXamlMetadataProvider` implementation this file replaces.
struct App : ApplicationT<App, IXamlMetadataProvider> {
    App() : m_provider(XamlControlsXamlMetaDataProvider()) {}

    void OnLaunched(Microsoft::UI::Xaml::LaunchActivatedEventArgs const&) {
        // Registers `XamlControlsResources` into `Application.Resources` exactly once, before any
        // Window/control exists — matches what a normal project's `App.xaml` does implicitly.
        // Failure here is fatal: every native control would otherwise construct with no default
        // `ControlTemplate` at all (see the module doc comment above for the investigation this
        // came out of), so this reports and rethrows rather than continuing silently broken.
        try {
            ResourceDictionary resources;
            resources.MergedDictionaries().Append(XamlControlsResources());
            Application::Current().Resources(resources);
        } catch (hresult_error const& error) {
            fprintf(stderr, "elwindui: failed to install default WinUI 3 control resources "
                             "(XamlControlsResources): 0x%08X %s\n",
                    static_cast<unsigned int>(error.code()), to_string(error.message()).c_str());
            throw;
        }

        if (g_startup != nullptr) {
            g_startup();
        }
    }

    IXamlType GetXamlType(Windows::UI::Xaml::Interop::TypeName const& type) {
        return m_provider.GetXamlType(type);
    }

    IXamlType GetXamlType(hstring const& fullName) {
        return m_provider.GetXamlType(fullName);
    }

    com_array<XmlnsDefinition> GetXmlnsDefinitions() {
        return m_provider.GetXmlnsDefinitions();
    }

   private:
    XamlControlsXamlMetaDataProvider m_provider;
};

}  // namespace

extern "C" __declspec(dllexport) void elwindui_winui3_run(void (*startup)()) {
    g_startup = startup;
    // COM apartment initialization happens on the Rust side before this is called (see
    // `elwindui::init()`) — reused as-is, not re-initialized here.
    Application::Start([](auto&&) { winrt::make<App>(); });
}
