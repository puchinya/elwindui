// Narrow XAML AutomationPeer projection for the Core-owned semantic accessibility tree.
//
// No role, state, ID lifetime, traversal, or application action is decided here. Those values are
// supplied by the copied-value callback ABI in accessibility_host.h. The host peer enumerates only
// virtual semantic children, which prevents the native rendering controls in Canvas.Children from
// becoming a second public automation tree.

#include "accessibility_host.h"

#include <algorithm>
#include <limits>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>
#include <winrt/Microsoft.UI.Xaml.Automation.Provider.h>
#include <winrt/Microsoft.UI.Xaml.Automation.Peers.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Foundation.h>

using namespace winrt;
using namespace winrt::Microsoft::UI::Xaml;
using namespace winrt::Microsoft::UI::Xaml::Controls;
using namespace winrt::Microsoft::UI::Xaml::Automation::Peers;

namespace XamlAutomation = winrt::Microsoft::UI::Xaml::Automation;
namespace XamlProvider = winrt::Microsoft::UI::Xaml::Automation::Provider;

namespace {

struct CanvasBridgeState {
    ElwinduiAccessibilityCallbacks callbacks{};
    std::map<std::uint64_t, AutomationPeer> peers;
    std::optional<AutomationPeer> root_peer;
};

std::map<void*, std::shared_ptr<CanvasBridgeState>> g_bridges;

constexpr std::uint32_t kStateDisabled = 1u << 0;
constexpr std::uint32_t kStateFocused = 1u << 1;
constexpr std::uint32_t kStateCheckedOn = 1u << 2;
constexpr std::uint32_t kStateSelected = 1u << 4;
constexpr std::uint32_t kStateReadOnly = 1u << 5;
constexpr std::uint32_t kStateCheckedMixed = 1u << 6;
constexpr std::uint32_t kActionActivate = 1u << 0;
constexpr std::uint32_t kActionSetValue = 1u << 3;
constexpr std::uint32_t kActionSetText = 1u << 4;
constexpr std::uint32_t kActionFocus = 1u << 5;
constexpr std::uint32_t kActionExpand = 1u << 6;
constexpr std::uint32_t kActionCollapse = 1u << 7;
constexpr std::uint32_t kActionSelect = 1u << 8;

constexpr std::uint32_t kPatternInvoke = 1u << 0;
constexpr std::uint32_t kPatternToggle = 1u << 1;
constexpr std::uint32_t kPatternRangeValue = 1u << 2;
constexpr std::uint32_t kPatternValue = 1u << 3;
constexpr std::uint32_t kPatternSelectionItem = 1u << 4;
constexpr std::uint32_t kPatternExpandCollapse = 1u << 5;
constexpr std::uint32_t kUiaElementNotEnabled = 0x80040200u;
constexpr std::uint32_t kUiaElementNotAvailable = 0x80040201u;
constexpr std::uint32_t kUiaNotSupported = 0x80040204u;
constexpr std::uint32_t kUiaInvalidOperation = 0x80131509u;

[[noreturn]] void throw_uia(std::uint32_t code) {
    throw hresult_error(static_cast<hresult>(code));
}

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

struct SemanticPeer : AutomationPeerT<
                          SemanticPeer,
                          XamlProvider::IInvokeProvider,
                          XamlProvider::IToggleProvider,
                          XamlProvider::IRangeValueProvider,
                          XamlProvider::IValueProvider,
                          XamlProvider::ISelectionItemProvider,
                          XamlProvider::IExpandCollapseProvider> {
    SemanticPeer(void* bridge_key, std::uint64_t id) : m_bridge_key(bridge_key), m_id(id) {}

    hstring GetClassNameCore() { return L"ElwindUI.Semantic"; }
    // SemanticPeer is intentionally virtual rather than backed by a XAML FrameworkElement. The
    // AutomationPeer base defaults both flags to false, which would make an otherwise valid
    // virtual peer disappear from UIA's control/content views. A stale peer must not remain
    // visible after its Core record is removed, so participation is queried on every call.
    bool IsControlElementCore() {
        ElwinduiAccessibilityNodeRecord record{};
        return TryGetCurrentRecord(record);
    }
    bool IsContentElementCore() {
        ElwinduiAccessibilityNodeRecord record{};
        return TryGetCurrentRecord(record);
    }

    hstring GetNameCore() {
        ElwinduiAccessibilityNodeRecord record{};
        if (!TryGetCurrentRecord(record)) {
            return {};
        }
        return hstring(copied_text(record.label, record.label_length));
    }

    AutomationControlType GetAutomationControlTypeCore() {
        ElwinduiAccessibilityNodeRecord record{};
        if (!TryGetCurrentRecord(record)) {
            return AutomationControlType::Group;
        }
        return control_type(record.role);
    }

    bool IsEnabledCore() {
        ElwinduiAccessibilityNodeRecord record{};
        return TryGetCurrentRecord(record) && (record.state_flags & kStateDisabled) == 0;
    }

    bool IsKeyboardFocusableCore() {
        ElwinduiAccessibilityNodeRecord record{};
        // Focusability is an advertised Core action, not the node's current focus state.
        return TryGetCurrentRecord(record) && (record.actions_mask & kActionFocus) != 0;
    }

    bool HasKeyboardFocusCore() {
        ElwinduiAccessibilityNodeRecord record{};
        // Checked-On is kStateCheckedOn (bit 2); only the focused state (bit 1) means focused.
        return TryGetCurrentRecord(record) && (record.state_flags & kStateFocused) != 0;
    }

    Windows::Foundation::Rect GetBoundingRectangleCore() {
        ElwinduiAccessibilityNodeRecord record{};
        if (!TryGetCurrentRecord(record)) {
            return {};
        }
        // WinUI composes virtual peers under their AutomationPeer parent and applies that
        // parent's screen origin after GetBoundingRectangleCore returns. Rust has already
        // converted the copied record to physical screen coordinates, so return the equivalent
        // parent-relative rect here; the framework then restores the measured parent origin.
        try {
            auto parent = GetParent();
            if (parent) {
                auto parent_bounds = parent.GetBoundingRectangle();
                return {
                    record.x - parent_bounds.X,
                    record.y - parent_bounds.Y,
                    record.width,
                    record.height,
                };
            }
        } catch (...) {
            // A late/stale parent must not turn a property read into a native exception. The
            // current record is still the best physical value available to this peer.
        }
        return {record.x, record.y, record.width, record.height};
    }

    hstring GetAutomationIdCore() {
        ElwinduiAccessibilityNodeRecord record{};
        if (!TryGetCurrentRecord(record)) {
            return {};
        }
        return hstring(copied_text(record.identifier, record.identifier_length));
    }

    Windows::Foundation::IInspectable GetPatternCore(PatternInterface const& pattern_interface) {
        ElwinduiAccessibilityNodeRecord record{};
        if (!TryGetCurrentRecord(record)) {
            return {};
        }
        const auto pattern = pattern_bit(pattern_interface);
        if (pattern == 0 || (record.patterns_mask & pattern) == 0) {
            return {};
        }
        // Return the peer itself so the UIA bridge can query the provider interface implemented by
        // this semantic peer. This is the same object shape used by WinUI's custom-peer contract.
        return *this;
    }

    void SetFocusCore() {
        DispatchAction(0, kActionFocus);
    }

    // IInvokeProvider
    void Invoke() {
        DispatchAction(kPatternInvoke, kActionActivate);
    }

    // IToggleProvider
    XamlAutomation::ToggleState ToggleState() {
        const auto record = CurrentRecordForPattern(kPatternToggle);
        if ((record.state_flags & kStateCheckedMixed) != 0) {
            return XamlAutomation::ToggleState::Indeterminate;
        }
        return (record.state_flags & kStateCheckedOn) != 0 ? XamlAutomation::ToggleState::On
                                                            : XamlAutomation::ToggleState::Off;
    }

    void Toggle() {
        DispatchAction(kPatternToggle, kActionActivate);
    }

    // IRangeValueProvider and IValueProvider both expose a Value() method with different return
    // types. The generated C++/WinRT ABI converts this single role-dependent result to the type
    // required by the interface that invoked it.
    struct ValueResult {
        double numeric{};
        hstring text{};

        operator double() const { return numeric; }
        operator hstring() const { return text; }
    };

    // IRangeValueProvider
    bool IsReadOnly() {
        return (CurrentValueRecord().state_flags & kStateReadOnly) != 0;
    }

    double LargeChange() {
        CurrentRecordForPattern(kPatternRangeValue);
        return std::numeric_limits<double>::quiet_NaN();
    }

    double Maximum() {
        return CurrentRecordForPattern(kPatternRangeValue).maximum;
    }

    double Minimum() {
        return CurrentRecordForPattern(kPatternRangeValue).minimum;
    }

    double SmallChange() {
        return CurrentRecordForPattern(kPatternRangeValue).step;
    }

    ValueResult Value() {
        const auto record = CurrentValueRecord();
        return ValueResult{record.value, hstring(copied_text(record.value_text, record.value_length))};
    }

    void SetValue(double value) {
        DispatchAction(kPatternRangeValue, kActionSetValue, value);
    }

    // IValueProvider
    void SetValue(hstring const& value) {
        DispatchTextAction(kPatternValue, kActionSetText, value);
    }

    // ISelectionItemProvider
    bool IsSelected() {
        return (CurrentRecordForPattern(kPatternSelectionItem).state_flags & kStateSelected) != 0;
    }

    XamlProvider::IRawElementProviderSimple SelectionContainer() {
        CurrentRecordForPattern(kPatternSelectionItem);
        return nullptr;
    }

    void AddToSelection() {
        DispatchAction(kPatternSelectionItem, kActionSelect);
    }

    void RemoveFromSelection() {
        const auto record = CurrentRecordForPattern(kPatternSelectionItem);
        if ((record.state_flags & kStateDisabled) != 0) {
            throw_uia(kUiaElementNotEnabled);
        }
        throw_uia(kUiaInvalidOperation);
    }

    void Select() {
        DispatchAction(kPatternSelectionItem, kActionSelect);
    }

    // IExpandCollapseProvider
    XamlAutomation::ExpandCollapseState ExpandCollapseState() {
        return CurrentRecordForPattern(kPatternExpandCollapse).state_flags & (1u << 3)
                   ? XamlAutomation::ExpandCollapseState::Expanded
                   : XamlAutomation::ExpandCollapseState::Collapsed;
    }

    void Collapse() {
        DispatchAction(kPatternExpandCollapse, kActionCollapse);
    }

    void Expand() {
        DispatchAction(kPatternExpandCollapse, kActionExpand);
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

private:
    static std::uint32_t pattern_bit(PatternInterface const& pattern_interface) {
        switch (pattern_interface) {
            case PatternInterface::Invoke: return kPatternInvoke;
            case PatternInterface::Toggle: return kPatternToggle;
            case PatternInterface::RangeValue: return kPatternRangeValue;
            case PatternInterface::Value: return kPatternValue;
            case PatternInterface::SelectionItem: return kPatternSelectionItem;
            case PatternInterface::ExpandCollapse: return kPatternExpandCollapse;
            default: return 0;
        }
    }

    bool TryGetCurrentRecord(ElwinduiAccessibilityNodeRecord& record) {
        auto bridge = bridge_for(m_bridge_key);
        return bridge && bridge->callbacks.get_node &&
               bridge->callbacks.get_node(bridge->callbacks.context, m_id, &record);
    }

    ElwinduiAccessibilityNodeRecord CurrentRecordForPattern(std::uint32_t pattern) {
        ElwinduiAccessibilityNodeRecord record{};
        if (!TryGetCurrentRecord(record)) {
            throw_uia(kUiaElementNotAvailable);
        }
        if (pattern != 0 && (record.patterns_mask & pattern) == 0) {
            throw_uia(kUiaNotSupported);
        }
        return record;
    }

    ElwinduiAccessibilityNodeRecord CurrentValueRecord() {
        const auto record = CurrentRecordForPattern(0);
        if ((record.patterns_mask & (kPatternRangeValue | kPatternValue)) == 0) {
            throw_uia(kUiaNotSupported);
        }
        return record;
    }

    struct ActionDispatch {
        void* context{};
        decltype(ElwinduiAccessibilityCallbacks::dispatch_action) callback{};
    };

    ActionDispatch ActionFor(std::uint32_t pattern, std::uint32_t action_kind) {
        ActionDispatch action{};
        {
            const auto record = CurrentRecordForPattern(pattern);
            if ((record.state_flags & kStateDisabled) != 0) {
                throw_uia(kUiaElementNotEnabled);
            }
            if ((record.actions_mask & (1u << action_kind)) == 0) {
                throw_uia(kUiaNotSupported);
            }
            auto bridge = bridge_for(m_bridge_key);
            if (!bridge || !bridge->callbacks.dispatch_action) {
                throw_uia(kUiaInvalidOperation);
            }
            action.context = bridge->callbacks.context;
            action.callback = bridge->callbacks.dispatch_action;
        }
        return action;
    }

    void DispatchAction(
        std::uint32_t pattern,
        std::uint32_t action_kind,
        double numeric_value = 0.0) {
        const auto action = ActionFor(pattern, action_kind);
        if (action.callback(action.context, m_id, action_kind, numeric_value, nullptr, 0) == 0) {
            throw_uia(kUiaInvalidOperation);
        }
    }

    void DispatchTextAction(
        std::uint32_t pattern,
        std::uint32_t action_kind,
        hstring const& value) {
        const auto action = ActionFor(pattern, action_kind);
        const auto* text = reinterpret_cast<char16_t const*>(value.c_str());
        if (action.callback(
                action.context,
                m_id,
                action_kind,
                0.0,
                text,
                static_cast<std::uint32_t>(value.size())) == 0) {
            throw_uia(kUiaInvalidOperation);
        }
    }

    void* m_bridge_key;
    std::uint64_t m_id;
};

struct SemanticRootPeer : FrameworkElementAutomationPeerT<SemanticRootPeer> {
    SemanticRootPeer(FrameworkElement const& owner, void* bridge_key)
        : FrameworkElementAutomationPeerT<SemanticRootPeer>(owner), m_bridge_key(bridge_key) {}

    hstring GetClassNameCore() { return L"ElwindUI.SemanticRoot"; }
    hstring GetNameCore() { return {}; }
    AutomationControlType GetAutomationControlTypeCore() { return AutomationControlType::Group; }

    Windows::Foundation::Rect GetBoundingRectangleCore() {
        ElwinduiAccessibilityNodeRecord record{};
        auto bridge = bridge_for(m_bridge_key);
        if (bridge && bridge->callbacks.get_node &&
            bridge->callbacks.get_node(bridge->callbacks.context, 0, &record) != 0) {
            return {record.x, record.y, record.width, record.height};
        }
        return {};
    }

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

extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_refresh(
    void* bridge_key,
    std::uint32_t structure_changed) {
    auto bridge = bridge_for(bridge_key);
    if (!bridge || !bridge->root_peer) return;
    try {
        // Every effective snapshot change may invalidate cached property values. Only an actual
        // semantic topology change is announced as a structure mutation.
        bridge->root_peer->InvalidatePeer();
        if (structure_changed != 0) {
            bridge->root_peer->RaiseStructureChangedEvent(
                AutomationStructureChangeType::ChildrenInvalidated, nullptr);
        }
    } catch (...) {
        // Refresh is best effort and must never affect the render/input path.
    }
}

extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_detach(void* bridge_key) {
    g_bridges.erase(bridge_key);
}
