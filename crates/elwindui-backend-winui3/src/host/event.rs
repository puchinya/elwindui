//! WinRT key/modifier -> `elwindui_core::input` translation. Pure value mapping; the dispatch
//! itself lives on `TreeHostPanel` in this module's parent.


use crate::ffi::AnyView;
use super::*;

use crate::bindings;
use crate::render::composition::{
    CompositionClipSpec, CompositionPrimitive, CompositionRenderer, DesiredCompositionIsland,
    DesiredCompositionNode, IslandId,
};
use crate::bindings::Microsoft::UI::Input::InputKeyboardSource;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Button as XamlButton, Canvas, MenuFlyoutItem, MenuFlyoutItemBase, PasswordBox as XamlPasswordBox,
    ScrollMode, ScrollViewer, TabView as XamlTabView, TabViewCloseButtonOverlayMode, TabViewItem,
    TabViewTabCloseRequestedEventArgs, TextBlock, TextBox as XamlTextBox,
};
use crate::bindings::Microsoft::UI::Xaml::Input::{
    CharacterReceivedRoutedEventArgs, KeyEventHandler, KeyboardAccelerator,
};
use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
use crate::bindings::Microsoft::UI::Xaml::{
    FrameworkElement, RoutedEventHandler, SizeChangedEventHandler, UIElement, Window as XamlWindow,
};
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasActiveLayer, CanvasAntialiasing, CanvasBitmap, CanvasBlend, CanvasEdgeBehavior, CanvasImageInterpolation,
    ICanvasResourceCreator,
};
use crate::bindings::Microsoft::UI::Composition::CompositionDrawingSurface;
use crate::bindings::Microsoft::Graphics::Canvas::Brushes::{
    CanvasGradientStop, CanvasImageBrush, CanvasLinearGradientBrush, CanvasRadialGradientBrush,
    CanvasSolidColorBrush, ICanvasBrush,
};
use crate::bindings::Microsoft::Graphics::Canvas::Geometry::{
    CanvasArcSize, CanvasFigureLoop, CanvasFilledRegionDetermination, CanvasGeometry,
    CanvasPathBuilder, CanvasSweepDirection,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    SelectionChangedEventHandler, TextChangedEventHandler,
};
use windows::Foundation::{PropertyValue, Size, TypedEventHandler};
use windows::Graphics::{PointInt32, SizeInt32};
use windows::System::{VirtualKey, VirtualKeyModifiers};
use windows::UI::{Color, Core::CoreVirtualKeyStates};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, IRandomAccessStream};
use elwindui_core::input::{
    FocusState, Key, KeyModifiers, KeyboardDispatcher, RawKeyEvent, RawKeyEventKind,
    RawTextInputEvent, ShortcutRegistry,
};
use elwindui_core::ui::{FocusHost, UIElementExt as _};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::{HSTRING, Interface, Result};

/// `VirtualKey.0`(a fixed `i32` code, the classic Win32 `VK_*` constants) -> `elwindui_core::input::
/// Key` for the named keys `Key` distinguishes; every other key falls back to treating the code as
/// an ASCII letter/digit (`VirtualKey::A`..`VirtualKey::Z`/`VirtualKey::Number0`..`Number9` are
/// numerically identical to their ASCII codes, `0x41..=0x5A`/`0x30..=0x39` — the same convention
/// `InnerMenuItem::set_shortcut` above already relies on). Codes are the standard, long-stable
/// Win32 virtual-key constants (`VK_RETURN`, `VK_TAB`, ...).
pub(crate) fn winui_key(virtual_key: VirtualKey) -> Option<Key> {
    let key = match virtual_key.0 {
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x09 => Key::Tab,
        0x08 => Key::Backspace,
        0x2E => Key::Delete,
        0x20 => Key::Space,
        0x25 => Key::Left,
        0x26 => Key::Up,
        0x27 => Key::Right,
        0x28 => Key::Down,
        0x24 => Key::Home,
        0x23 => Key::End,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x70 => Key::F1,
        0x71 => Key::F2,
        0x72 => Key::F3,
        0x73 => Key::F4,
        0x74 => Key::F5,
        0x75 => Key::F6,
        0x76 => Key::F7,
        0x77 => Key::F8,
        0x78 => Key::F9,
        0x79 => Key::F10,
        0x7A => Key::F11,
        0x7B => Key::F12,
        code @ (0x30..=0x39 | 0x41..=0x5A) => {
            Key::Character((code as u8 as char).to_ascii_lowercase())
        }
        _ => return None,
    };
    Some(key)
}

/// `Microsoft::UI::Input::InputKeyboardSource::GetKeyStateForCurrentThread` (the WinAppSDK/WinUI3
/// desktop replacement for UWP's `CoreWindow.GetKeyState`) -> `elwindui_core::input::KeyModifiers`.
/// `KeyRoutedEventArgs` itself carries no modifier snapshot (unlike AppKit's `NSEvent.
/// modifierFlags()`), so this polls current key state directly instead.
pub(crate) fn winui_modifiers() -> KeyModifiers {
    fn is_down(vk: i32) -> bool {
        InputKeyboardSource::GetKeyStateForCurrentThread(VirtualKey(vk))
            .map(|state| state.contains(CoreVirtualKeyStates::Down))
            .unwrap_or(false)
    }
    KeyModifiers {
        shift: is_down(0x10),                 // VK_SHIFT
        control: is_down(0x11),               // VK_CONTROL
        alt: is_down(0x12),                   // VK_MENU
        meta: is_down(0x5B) || is_down(0x5C), // VK_LWIN / VK_RWIN
    }
}
