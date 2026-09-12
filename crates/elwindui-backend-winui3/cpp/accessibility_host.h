#pragma once

#include <cstddef>
#include <cstdint>

// This ABI is deliberately a copied-value boundary. Rust owns the semantic tree and action
// policy; the C++/WinRT side only asks for records and projects them into AutomationPeer objects.
struct ElwinduiAccessibilityNodeRecord {
    std::uint64_t id;
    std::uint32_t role;
    std::uint32_t state_flags;
    std::uint32_t actions_mask;
    double value;
    double minimum;
    double maximum;
    double step;
    std::uint8_t has_range;
    float x;
    float y;
    float width;
    float height;
    std::uint32_t label_length;
    char16_t label[256];
    std::uint32_t value_length;
    char16_t value_text[256];
};

struct ElwinduiAccessibilityCallbacks {
    void* context;
    std::uint64_t (*revision)(void* context);
    std::uint32_t (*child_count)(void* context, std::uint64_t parent_id);
    std::uint64_t (*child_id)(void* context, std::uint64_t parent_id, std::uint32_t index);
    std::uint32_t (*get_node)(
        void* context,
        std::uint64_t id,
        ElwinduiAccessibilityNodeRecord* record);
    std::uint32_t (*dispatch_action)(
        void* context,
        std::uint64_t id,
        std::uint32_t action_kind,
        double numeric_value,
        const char16_t* text,
        std::uint32_t text_length);
};

extern "C" __declspec(dllexport) void* elwindui_winui3_accessibility_canvas_create();
extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_set_callbacks(
    void* canvas,
    const ElwinduiAccessibilityCallbacks* callbacks);
extern "C" __declspec(dllexport) void elwindui_winui3_accessibility_canvas_detach(void* canvas);
