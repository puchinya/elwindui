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
#include <string>
#include <utility>
#include <vector>
#include <winrt/Microsoft.UI.Xaml.Automation.Peers.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Windows.Foundation.h>

using namespace winrt;
using namespace winrt::Microsoft::UI::Xaml;
using namespace winrt::Microsoft::UI::Xaml::Controls;
using namespace winrt::Microsoft::UI::Xaml::Automation::Peers;

namespace {

struct CanvasBridgeState {
    ElwinduiAccessibilityCallbacks callbacks{};
    std::map<std::uint64_t, AutomationPeer> peers;
};

std::map<void*, std::shared_ptr<CanvasBridgeState>> g_bridges;

std::shared_ptr<CanvasBridgeState> bridge_for(void* canvas) {
    auto it = g_bridges.find(canvas);
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
    SemanticPeer(void* canvas, std::uint64_t id) : m_canvas(canvas), m_id(id) {}

    hstring GetClassNameCore() { return L"ElwindUI.Semantic"; }

    hstring GetNameCore() {
        auto bridge = bridge_for(m_canvas);
        ElwinduiAccessibilityNodeRecord record{};
        if (!bridge || !bridge->callbacks.get_node ||
            !bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record)) {
            return {};
        }
        return hstring(copied_text(record.label, record.label_length));
    }

    AutomationControlType GetAutomationControlTypeCore() {
        auto bridge = bridge_for(m_canvas);
        ElwinduiAccessibilityNodeRecord record{};
        if (!bridge || !bridge->callbacks.get_node ||
            !bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record)) {
            return AutomationControlType::Group;
        }
        return control_type(record.role);
    }

    bool IsEnabledCore() {
        auto bridge = bridge_for(m_canvas);
        ElwinduiAccessibilityNodeRecord record{};
        return bridge && bridge->callbacks.get_node &&
               bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record) &&
               (record.state_flags & (1u << 0)) == 0;
    }

    bool IsKeyboardFocusableCore() {
        auto bridge = bridge_for(m_canvas);
        ElwinduiAccessibilityNodeRecord record{};
        return bridge && bridge->callbacks.get_node &&
               bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record) &&
               (record.state_flags & (1u << 1)) != 0;
    }

    bool HasKeyboardFocusCore() {
        auto bridge = bridge_for(m_canvas);
        ElwinduiAccessibilityNodeRecord record{};
        return bridge && bridge->callbacks.get_node &&
               bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record) &&
               (record.state_flags & (1u << 2)) != 0;
    }

    Windows::Foundation::Rect GetBoundingRectangleCore() {
        auto bridge = bridge_for(m_canvas);
        ElwinduiAccessibilityNodeRecord record{};
        if (!bridge || !bridge->callbacks.get_node ||
            !bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record)) {
            return {};
        }
        return {record.x, record.y, record.width, record.height};
    }

    com_array<AutomationPeer> GetChildrenCore() {
        std::vector<AutomationPeer> children;
        auto bridge = bridge_for(m_canvas);
        if (!bridge || !bridge->callbacks.child_count || !bridge->callbacks.child_id) {
            return children;
        }
        auto count = bridge->callbacks.child_count(bridge->callbacks.context, m_id);
        children.reserve(count);
        for (std::uint32_t index = 0; index < count; ++index) {
            auto child = bridge->callbacks.child_id(bridge->callbacks.context, m_id, index);
            if (child == 0) continue;
            auto it = bridge->peers.find(child);
            if (it == bridge->peers.end()) {
                it = bridge->peers.emplace(child, make<SemanticPeer>(m_canvas, child)).first;
            }
            children.push_back(it->second);
        }
        return com_array<AutomationPeer>(std::move(children));
    }

    // Pattern providers remain in the generated peer surface and are enabled by the same copied
    // action bits in a follow-up projection. This peer never fabricates a provider for an action
    // that Core did not advertise.

private:
    void* m_canvas;
    std::uint64_t m_id;
};

struct SemanticRootPeer : FrameworkElementAutomationPeerT<SemanticRootPeer> {
    SemanticRootPeer(FrameworkElement const& owner, void* canvas)
        : FrameworkElementAutomationPeerT<SemanticRootPeer>(owner), m_canvas(canvas) {}

    hstring GetClassNameCore() { return L"ElwindUI.SemanticRoot"; }
    hstring GetNameCore() { return {}; }
    AutomationControlType GetAutomationControlTypeCore() { return AutomationControlType::Group; }

    com_array<AutomationPeer> GetChildrenCore() {
        std::vector<AutomationPeer> children;
        auto bridge = bridge_for(m_canvas);
        if (!bridge || !bridge->callbacks.child_count || !bridge->callbacks.child_id) {
            return children;
        }
        auto count = bridge->callbacks.child_count(bridge->callbacks.context, 0);
        children.reserve(count);
        for (std::uint32_t index = 0; index < count; ++index) {
            auto id = bridge->callbacks.child_id(bridge->callbacks.context, 0, index);
            if (id == 0) continue;
            auto it = bridge->peers.find(id);
            if (it == bridge->peers.end()) {
                it = bridge->peers.emplace(id, make<SemanticPeer>(m_canvas, id)).first;
            }
            children.push_back(it->second);
        }
        return com_array<AutomationPeer>(std::move(children));
    }

private:
    void* m_canvas;
};

struct AccessibilityCanvas : CanvasT<AccessibilityCanvas> {
    AutomationPeer OnCreateAutomationPeer() {
        return make<SemanticRootPeer>(*this, get_abi(*this));
    }
};

}  // namespace

extern "C" __declspec(dllexport) void* elwindui_winui3_accessibility_canvas_create() {
    try {
        auto canvas = make<AccessibilityCanvas>();
        auto inspectable = canvas.as<IInspectable>();
        auto key = get_abi(inspectable);
        g_bridges.emplace(key, std::make_shared<CanvasBridgeState>());
        return detach_abi(inspectable);
    } catch (...) {
        return nullptr;
    }
}

extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_set_callbacks(
    void* canvas,
    ElwinduiAccessibilityCallbacks const* callbacks) {
    auto bridge = bridge_for(canvas);
    if (!bridge) return;
    bridge->callbacks = callbacks ? *callbacks : ElwinduiAccessibilityCallbacks{};
}

extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_detach(void* canvas) {
    g_bridges.erase(canvas);
}
