// Narrow XAML AutomationPeer projection for the Core-owned semantic accessibility tree.
//
// No role, state, ID lifetime, traversal, or application action is decided here. Those values are
// supplied by the copied-value callback ABI in accessibility_host.h. The host peer enumerates only
// virtual semantic children, which prevents the native rendering controls in Canvas.Children from
// becoming a second public automation tree.

#include "accessibility_host.h"

#include <algorithm>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>
#include <winrt/Microsoft.UI.Xaml.Automation.Peers.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Foundation.h>

using namespace winrt;
using namespace winrt::Microsoft::UI::Xaml;
using namespace winrt::Microsoft::UI::Xaml::Controls;
using namespace winrt::Microsoft::UI::Xaml::Automation::Peers;

namespace {

struct CanvasBridgeState {
    ElwinduiAccessibilityCallbacks callbacks{};
    std::map<std::uint64_t, AutomationPeer> peers;
    std::optional<AutomationPeer> root_peer;
};

std::map<void*, std::shared_ptr<CanvasBridgeState>> g_bridges;

std::shared_ptr<CanvasBridgeState> bridge_for(void* bridge_key) {
    auto it = g_bridges.find(bridge_key);
    return it == g_bridges.end() ? nullptr : it->second;
}

std::wstring copied_text(char16_t const* text, std::uint32_t length) {
    return std::wstring(reinterpret_cast<wchar_t const*>(text),
                        reinterpret_cast<wchar_t const*>(text + length));
}

AutomationControlType control_type(std::uint32_t role) {
    switch (role) {
        case 1: return AutomationControlType::Button;
        case 2: return AutomationControlType::Text;
        case 3: return AutomationControlType::Edit;
        case 5: return AutomationControlType::CheckBox;
        case 6: return AutomationControlType::RadioButton;
        case 7: return AutomationControlType::Button;
        case 8: return AutomationControlType::Slider;
        case 9: return AutomationControlType::ComboBox;
        default: return AutomationControlType::Group;
    }
}

struct SemanticPeer : AutomationPeerT<SemanticPeer> {
    SemanticPeer(void* bridge_key, std::uint64_t id) : m_bridge_key(bridge_key), m_id(id) {}

    hstring GetClassNameCore() { return L"ElwindUI.Semantic"; }
    // SemanticPeer is intentionally virtual rather than backed by a XAML FrameworkElement. The
    // AutomationPeer base defaults both flags to false, which would make an otherwise valid
    // virtual peer disappear from UIA's control/content views.
    bool IsControlElementCore() { return true; }
    bool IsContentElementCore() { return true; }

    hstring GetNameCore() {
        auto bridge = bridge_for(m_bridge_key);
        ElwinduiAccessibilityNodeRecord record{};
        if (!bridge || !bridge->callbacks.get_node ||
            !bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record)) {
            return {};
        }
        return hstring(copied_text(record.label, record.label_length));
    }

    AutomationControlType GetAutomationControlTypeCore() {
        auto bridge = bridge_for(m_bridge_key);
        ElwinduiAccessibilityNodeRecord record{};
        if (!bridge || !bridge->callbacks.get_node ||
            !bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record)) {
            return AutomationControlType::Group;
        }
        return control_type(record.role);
    }

    bool IsEnabledCore() {
        auto bridge = bridge_for(m_bridge_key);
        ElwinduiAccessibilityNodeRecord record{};
        return bridge && bridge->callbacks.get_node &&
               bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record) &&
               (record.state_flags & (1u << 0)) == 0;
    }

    bool IsKeyboardFocusableCore() {
        auto bridge = bridge_for(m_bridge_key);
        ElwinduiAccessibilityNodeRecord record{};
        return bridge && bridge->callbacks.get_node &&
               bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record) &&
               (record.state_flags & (1u << 1)) != 0;
    }

    bool HasKeyboardFocusCore() {
        auto bridge = bridge_for(m_bridge_key);
        ElwinduiAccessibilityNodeRecord record{};
        return bridge && bridge->callbacks.get_node &&
               bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record) &&
               (record.state_flags & (1u << 2)) != 0;
    }

    Windows::Foundation::Rect GetBoundingRectangleCore() {
        auto bridge = bridge_for(m_bridge_key);
        ElwinduiAccessibilityNodeRecord record{};
        if (!bridge || !bridge->callbacks.get_node ||
            !bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record)) {
            return {};
        }
        return {record.x, record.y, record.width, record.height};
    }

    Windows::Foundation::Collections::IVector<AutomationPeer> GetChildrenCore() {
        std::vector<AutomationPeer> children;
        auto bridge = bridge_for(m_bridge_key);
        if (!bridge || !bridge->callbacks.child_count || !bridge->callbacks.child_id) {
            return single_threaded_vector<AutomationPeer>(std::move(children));
        }
        auto count = bridge->callbacks.child_count(bridge->callbacks.context, m_id);
        children.reserve(count);
        auto parent = get_strong().as<AutomationPeer>();
        for (std::uint32_t index = 0; index < count; ++index) {
            auto child = bridge->callbacks.child_id(bridge->callbacks.context, m_id, index);
            if (child == 0) continue;
            auto it = bridge->peers.find(child);
            if (it == bridge->peers.end()) {
                it = bridge->peers.emplace(child, make<SemanticPeer>(m_bridge_key, child)).first;
            }
            // Virtual peers have no visual owner from which WinUI can infer this relationship.
            it->second.SetParent(parent);
            children.push_back(it->second);
        }
        return single_threaded_vector<AutomationPeer>(std::move(children));
    }

    // Pattern providers remain in the generated peer surface and are enabled by the same copied
    // action bits in a follow-up projection. This peer never fabricates a provider for an action
    // that Core did not advertise.

private:
    void* m_bridge_key;
    std::uint64_t m_id;
};

struct SemanticRootPeer : FrameworkElementAutomationPeerT<SemanticRootPeer> {
    SemanticRootPeer(FrameworkElement const& owner, void* bridge_key)
        : FrameworkElementAutomationPeerT<SemanticRootPeer>(owner), m_bridge_key(bridge_key) {}

    hstring GetClassNameCore() { return L"ElwindUI.SemanticRoot"; }
    hstring GetNameCore() { return {}; }
    AutomationControlType GetAutomationControlTypeCore() { return AutomationControlType::Group; }

    Windows::Foundation::Collections::IVector<AutomationPeer> GetChildrenCore() {
        std::vector<AutomationPeer> children;
        auto bridge = bridge_for(m_bridge_key);
        if (!bridge || !bridge->callbacks.child_count || !bridge->callbacks.child_id) {
            return single_threaded_vector<AutomationPeer>(std::move(children));
        }
        auto count = bridge->callbacks.child_count(bridge->callbacks.context, 0);
        children.reserve(count);
        auto parent = get_strong().as<AutomationPeer>();
        for (std::uint32_t index = 0; index < count; ++index) {
            auto id = bridge->callbacks.child_id(bridge->callbacks.context, 0, index);
            if (id == 0) continue;
            auto it = bridge->peers.find(id);
            if (it == bridge->peers.end()) {
                it = bridge->peers.emplace(id, make<SemanticPeer>(m_bridge_key, id)).first;
            }
            // Keep the virtual peer graph connected even though semantic nodes are not native
            // XAML children.
            it->second.SetParent(parent);
            children.push_back(it->second);
        }
        return single_threaded_vector<AutomationPeer>(std::move(children));
    }

private:
    void* m_bridge_key;
};

struct AccessibilityCanvas : CanvasT<AccessibilityCanvas> {
    AutomationPeer OnCreateAutomationPeer() {
        auto inspectable = get_strong().as<Windows::Foundation::IInspectable>();
        auto key = get_abi(inspectable);
        auto peer = make<SemanticRootPeer>(*this, key);
        if (auto bridge = bridge_for(key)) {
            bridge->root_peer = peer;
        }
        return peer;
    }
};

}  // namespace

extern "C" __declspec(dllexport) void* elwindui_winui3_accessibility_canvas_create() {
    try {
        auto canvas = make<AccessibilityCanvas>();
        auto inspectable = canvas.as<Windows::Foundation::IInspectable>();
        auto key = get_abi(inspectable);
        g_bridges.emplace(key, std::make_shared<CanvasBridgeState>());
        return detach_abi(inspectable);
    } catch (...) {
        return nullptr;
    }
}

extern "C" __declspec(dllexport) std::uint32_t elwindui_winui3_accessibility_canvas_set_callbacks(
    void* bridge_key,
    ElwinduiAccessibilityCallbacks const* callbacks) {
    auto bridge = bridge_for(bridge_key);
    if (!bridge) return 0;
    bridge->callbacks = callbacks ? *callbacks : ElwinduiAccessibilityCallbacks{};
    return 1;
}

extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_notify_tree_changed(
    void* bridge_key) {
    auto bridge = bridge_for(bridge_key);
    if (!bridge || !bridge->root_peer) return;
    try {
        // The initial peer can be queried while the TreeHost still has no Core tree. Invalidate
        // that cached empty result before announcing the rebuilt virtual structure.
        bridge->root_peer->InvalidatePeer();
        bridge->root_peer->RaiseStructureChangedEvent(
            AutomationStructureChangeType::ChildrenInvalidated, nullptr);
    } catch (...) {
        // Accessibility notifications are best effort and must never affect the render/input path.
    }
}

extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_detach(void* bridge_key) {
    g_bridges.erase(bridge_key);
}
