# Layout reflow position animation

Owning Issue: [#290](https://github.com/puchinya/elwindui/issues/290). Normative behavior:
[`animation_spec.md` § Layout reflow](../../docs/specs/animation_spec.md#layout-reflow).

## Accepted behavior

A structural change inside an animated transaction moves the surviving siblings below it from
their previously rendered position to their new target position. Layout and semantics switch to
the target immediately; only the shown position (pixels, pointer target, and semantic bounds)
travels. Self-drawn content and a NativeControl move the same way. Reduce Motion snaps.

## Backends

| Backend | Requirement |
|---|---|
| WinUI 3 | required (RF-17) |
| AppKit | required when a macOS host is available (RF-16) |
| GTK4 | not supported by this case |

## Fixture

`examples/animation-demo` (build it first). Stable selectors:

| Selector | Role |
|---|---|
| `Insert / remove panel` button | toggles the self-drawn panel in a 4000 ms ease-in-out transaction (queued layout path) |
| `Insert / remove TextBox` button | toggles the NativeControl in a 5000 ms ease-out transaction; removal begins an exit and flushes layout interactively |
| `Reduce motion` button | toggles `ReduceMotionEnvironment` |
| `animation-reflow-panel-marker` | self-drawn TextBlock directly below the panel |
| `animation-native-textbox` | NativeControl TextBox below the panel marker |
| `animation-reflow-native-marker` | self-drawn TextBlock directly below the TextBox |
| `animation-native-focus-state` | shows `Focused` / `Unfocused` for the TextBox |

Positions are read from the semantic bounds (`y`) reported by the platform accessibility tree.
"Intermediate" means strictly between the start and final `y` with at least 3 px clearance from
both. "Stable" means two reads at least 1 s apart that differ by at most 1 px.

## Cases

### LR-01 Self-drawn removal moves the siblings below it

1. Record the start `y` of the panel marker, the TextBox, and the native marker.
2. Invoke `Insert / remove panel` once (removes the panel).
3. Read the three `y` values at about +1.0 s and +2.0 s after the invoke; capture a screenshot at
   the first sample.
4. Read them again after at least +5.0 s until stable; capture a screenshot.

Expected: every element moves up by the same `Δ > 0`; each sample is intermediate and the second
is closer to the final value than the first; the final values equal start minus `Δ` within 1 px.
The screenshot at the first sample shows the TextBox drawn at its intermediate position.

### LR-02 Reinsertion and mid-flight reversal stay continuous

Precondition: LR-01 final state.

1. Invoke `Insert / remove panel` (reinserts the panel); read the panel marker `y` at about +1.0 s
   (`y_a`).
2. Immediately invoke `Insert / remove panel` again (reverses to removal) and read `y` as soon as
   possible (`y_b`), then until stable.

Expected: `y_a` and `y_b` are both intermediate between the LR-01 final and start values (the
reversal never jumps to either endpoint), later reads move back up, and the stable value equals
the LR-01 final value within 1 px.

### LR-03 NativeControl exit moves the marker below it (interactive flush)

1. Record the native marker start `y`.
2. Invoke `Insert / remove TextBox` (removal with exit transition).
3. Read the native marker `y` at about +1.0 s and +2.5 s; capture a screenshot at the first sample.
4. Read until stable after at least +6.0 s.

Expected: both samples are intermediate and progress upward; the outgoing TextBox stays visible in
the first screenshot and is no longer exposed as an enabled, focusable TextBox; the stable value is
the start minus the TextBox slot. Afterwards invoke `Insert / remove TextBox` again and wait until
stable so the TextBox is back for LR-04.

### LR-04 Pointer input follows the shown NativeControl position

1. Invoke `Insert / remove panel` (state toggles) and at about +1.0 s read the TextBox bounds; it
   must be intermediate relative to its start/final `y` for this toggle.
2. Real-click the center of those bounds immediately.
3. Read `animation-native-focus-state`.

Expected: the focus state is `Focused`. Restore the panel afterwards if needed and wait until
stable.

### LR-05 Reduce Motion snaps

1. Invoke `Reduce motion` and confirm the Reduce motion label reads `true`.
2. Record the panel marker `y`, invoke `Insert / remove panel`, and read `y` as soon as possible
   and again after 1 s.

Expected: the first read already equals the final value within 1 px and the second read is
identical within 1 px (no intermediate frame). Invoke `Reduce motion` again to restore `false`.

### LR-06 Cleanup

Terminate the demo normally. Expected: the process exits without a forced kill.

## Classification

PASS, FAIL, NOT RUN, and BLOCKED follow the platform tester guides
([`winui3-e2e.md`](../../docs/agents/winui3-e2e.md), [`appkit-e2e.md`](../../docs/agents/appkit-e2e.md)).
Timing-sensitive samples may be retried once from a restored stable precondition. Raw evidence
stays under `.agent-state/issues/290/e2e/`.
