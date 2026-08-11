# Text implementation design

Related specification: [`../../specs/text_style_spec.md`](../../specs/text_style_spec.md).

## Storage and resolution

Each `#[text_style]` class owns `TextStyleStorage`, whose optional local fields distinguish unset from explicit values. Resolution walks the public inheritance relationship and overlays properties independently into `ComputedTextStyle`; measurement and painting consume only the fully resolved value.

`TextStyleOwner` and `as_text_style_owner()` provide the internal lookup seam. `inheritance_parent(kind)` distinguishes logical content inheritance from visual fallback so backend helper nodes do not alter observable cascade behavior.

## Change propagation

Setters compare old and new local values. A changed metric property invalidates measure and paint; a changed foreground invalidates paint. Descendants that inherit the changed property are invalidated through the tree rather than eagerly copying ancestor values.

Theme-backed properties record the Theme revision used for their last synchronization. Only nodes that reference Theme values are revisited when the revision changes.

## Measurement seam

`TextBackend` is the backend-neutral measurement seam. AppKit uses attributed-string measurement and WinUI 3 uses a scratch XAML text element. A deterministic dummy backend supports core tests when no platform backend is registered.

The measurement input is text, constraints, and `ComputedTextStyle`. Backend adapters must use the same conversions for measuring and drawing.

## Native controls

Native controls receive resolved font and foreground values through their backend adapter. `PlatformDefault` clears the native property instead of assigning a hard-coded family or color.

Secure entry is a separate adapter path: AppKit `NSSecureTextField` keeps its system font cascade and secure-mask rendering. Unsupported synthesis such as arbitrary family, spacing, or italic must not replace the secure glyph cascade merely to match ordinary text fields.
