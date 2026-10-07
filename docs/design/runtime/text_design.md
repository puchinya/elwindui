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

WinUI3 keeps a dedicated untouched thread-local XAML TextBlock for reading live platform default
style values. It is separate from the measurement scratch whose properties are overwritten for
each request. Default-style resolution reads the native properties each time without constructing
a new native object or freezing theme/language defaults in a computed-style cache.

`TextBlock` retains up to eight measured sizes for available-size constraints within one text,
resolved-style, alignment, and registered text-backend generation. Hits move to the most recently
used position; inserting a ninth constraint evicts the oldest entry. This allows alternating
natural-size and resolved-cell Grid probes to reuse their own font metrics without evicting each
other. Text, inherited or local style, alignment, and backend replacement clear the whole local
set; a previously unseen constraint misses only its own size entry.
`UIElement::measure` still runs normally and stores its current desired size; this cache only avoids
repeating deterministic backend font measurement and does not cache arrangement or painting.

## Native controls

`TextBlock.text_wrapping` defaults to `NoWrap` and participates in its measurement-cache
signature. Text measurement and `RenderCommand::Text` carry the same `TextWrapping` value.
Existing RenderContext text helpers retain NoWrap; the explicit wrapping helper supplies the
requested value. WinUI3 sets TextWrapping on both measurement and retained paint TextBlocks;
AppKit uses the matching paragraph line-break mode and CATextLayer wrapping setting.
Docking's selected-document content header explicitly uses Wrap and an Auto middle row so
its retained title grows the header above its 40px minimum without replacing page ownership.

Native controls receive resolved font and foreground values through their backend adapter. `PlatformDefault` clears the native property instead of assigning a hard-coded family or color.

Secure entry is a separate adapter path: AppKit `NSSecureTextField` keeps its system font cascade and secure-mask rendering. Unsupported synthesis such as arbitrary family, spacing, or italic must not replace the secure glyph cascade merely to match ordinary text fields.
