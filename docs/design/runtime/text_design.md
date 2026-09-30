# Text implementation design

Related specification: [`../../specs/text_style_spec.md`](../../specs/text_style_spec.md).

## Storage and resolution

Each `#[text_style]` class owns `TextStyleStorage`, whose optional local fields distinguish unset from explicit values. Resolution walks the visual-parent chain and overlays properties independently into `ComputedTextStyle`; measurement and painting consume only the fully resolved value.

`TextStyleOwner` and `as_text_style_owner()` provide the internal lookup seam. Text-style resolution always requests `InheritanceKind::Visual`; logical inheritance remains available to other inheritable relations, but does not alter the text cascade.

## Change propagation

Setters compare old and new local values. A changed metric property invalidates measure and paint; a changed foreground invalidates paint. Descendants that inherit the changed property are invalidated through the tree rather than eagerly copying ancestor values.

Theme-backed properties record the Theme revision used for their last synchronization. Only nodes that reference Theme values are revisited when the revision changes.

## Measurement seam

`TextBackend` is the backend-neutral measurement seam. AppKit uses attributed-string measurement and WinUI 3 uses a scratch XAML text element. A deterministic dummy backend supports core tests when no platform backend is registered.

The measurement input is text, constraints, and `ComputedTextStyle`. Backend adapters must use the same conversions for measuring and drawing.

`TextBlock` retains only its most recent measured size, keyed by text, resolved style, available
size, alignment, and the registered text-backend generation. Repeating the same measurement within
one unchanged visual state returns that result without calling the platform text engine again.
Text, inherited or local style, constraints, alignment, and backend replacement each miss the key.
`UIElement::measure` still runs normally and stores its current desired size; this cache only avoids
repeating deterministic backend font measurement and does not cache arrangement or painting.

## Native controls

Native controls receive resolved font and foreground values through their backend adapter. `PlatformDefault` clears the native property instead of assigning a hard-coded family or color.

Secure entry is a separate adapter path: AppKit `NSSecureTextField` keeps its system font cascade and secure-mask rendering. Unsupported synthesis such as arbitrary family, spacing, or italic must not replace the secure glyph cascade merely to match ordinary text fields.
