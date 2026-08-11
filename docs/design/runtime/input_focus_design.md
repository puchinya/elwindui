# Input and focus design

Related specification: [`../../specs/ui_spec.md`](../../specs/ui_spec.md).

## Input routing

Backends normalize pointer and keyboard messages into backend-neutral event arguments. Hit testing starts at arranged, visible, active roots and walks visual children in reverse paint order. NativeControl messages first resolve their ElwindUI owner.

Routed events use one precomputed route so tree mutation during a handler does not change the current dispatch. Tunnel travels root-to-target, direct dispatches only to the target, and bubble travels target-to-root. `handled` prevents later ordinary handlers while explicitly opt-in handled-event observers may still run.

## Focus

One focus tracker per host owns the focused element. Focus changes validate that the target is active, focusable, and attached; they dispatch loss before gain and synchronize the native widget where one exists.

Native focus callbacks resolve back to the same owner mapping. Re-entrant callbacks caused by programmatic native focus must converge on the already selected owner rather than producing a second transition.

## Keyboard navigation

Tab order is derived from active visual traversal plus `is_tab_stop` and explicit ordering metadata. Collapsed or inactive subtrees are excluded. Directional and shortcut dispatch share the same participation predicate.

Shortcut registration belongs to the mounted host. Unmounting or deactivating a subtree makes its shortcuts unavailable without destroying component state.

## Accessibility

Accessibility adapters expose public roles, names, values, enabled/focus state, and actions from the same owner mapping. Backend-only helper views are hidden unless they represent an independently meaningful public element.
