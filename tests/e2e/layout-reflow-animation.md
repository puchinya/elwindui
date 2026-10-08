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
| `Toggle reflow spacer` button | verification-only: toggles a 120 DIP spacer before the probe TextBox in a fixed 12 s linear transaction |
| `animation-reflow-probe-textbox` | verification-only 160 DIP-tall NativeControl TextBox at the end of the layout |
| `animation-reflow-probe-focus-state` | shows `Focused` / `Unfocused` for the probe |
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

Discriminating geometry is mandatory: at the moment of a real click, the point must be inside the
probe TextBox's shown rect and outside its target rect, and the probe must gain focus. A point
inside both rects proves nothing and is never PASS. The probe is 160 DIP tall and moves 130 DIP
down over a fixed 12 s linear transaction.

**Coordinate system.** All rects and points are physical screen pixels: UIA bounds as reported by
the driver `search`, and `point-click` coordinates. A screenshot is used only after its
image-to-screen transform is verified (see the screen-capture source below).

**Records.** Every driver command's raw JSON is saved to its own file, and a UTC timestamp is
written to a live timeline file immediately before and after each command (never reconstructed
afterwards).

- `start = (left_0, top_0, right_0, bottom_0)`: stable probe rect before the change (UIA).
- `target = (left_f, top_f, right_f, bottom_f)`: stable probe rect after the change (UIA), with
  `abs(top_f - (top_0 + 130)) <= 1`.
- `click = (x_c, y_c)`, `t_click`: the point actually sent by `point-click`, with
  `x_c = (left_0 + right_0) / 2` and `y_c = top_0 + 124` (6 px above the predicted target top).
- `after = (left_s, top_s, right_s, bottom_s)`, `t_after`, `source`: the probe rect observed
  after the click, where `source` is `uia` (a real `search` started after `point-click` returned)
  or `screen_capture` (a real post-click `capture-window --capture-screen` image).

**Order.** Stable start reads (`start`, focus `Unfocused`) → UIA `invoke` of `Toggle reflow
spacer` → real `point-click (x_c, y_c)` → immediately `capture-window --capture-screen` → UIA
`search` for the probe (`s2`) → focus read → wait → stable `target` read → restore. The UIA `s2`
read must never delay the screenshot. Nothing else (no other toggle, scroll, resize, move, DPI or
monitor change, focus change to another window) may happen between the invoke and `target`.

**Source `uia` (preferred).** `after = s2` when its timeline shows the `search` started after
`point-click` returned. `t_after` is the timeline time before the `search`.

**Source `screen_capture` (only when `s2` is missing or not usable).** The image must be the
post-click capture. Its transform is verified first: the same run's pre-change capture
(`lr04-start.png`) must place the probe top and left edges at the UIA `start` values within 2 px
using `screen = window_origin + image_px` with the window rect from the run's latest
`list-windows` and the 96 DPI scale recorded by `launch`. The post-click probe edges are measured
from pixel data (not estimated by eye) and converted the same way. A missing, black, or blurred
image, an unverifiable transform, or unclear probe edges make this source unusable.

**Checks (3 px margin).** The motion is monotonic downward, so the shown top at `t_click` is at
most `top_s` and the shown bottom at least `bottom_0`.

- C1 shown rect contains the click: `left_s + 3 <= x_c <= right_s - 3`,
  `top_s + 3 <= y_c <= bottom_0 - 3`, `top_0 <= top_s < top_f - 3`, and `t_click < t_after`.
- C2 target rect excludes the click: `y_c <= top_f - 3`, `left_f + 3 <= x_c <= right_f - 3`, and
  `abs(top_f - (top_0 + 130)) <= 1`.
- C3 input took effect: `animation-reflow-probe-focus-state` reads `Unfocused` before and
  `Focused` after. A successful `point-click` command alone is not evidence.

**Classification.** PASS only when C1, C2, and C3 hold from one attempt's own raw files. If no
usable `after` source exists, or `after` was taken at or after the end of the motion, the attempt
is INCONCLUSIVE; a tool or host failure is BLOCKED; an attempt that never ran is NOT RUN; if C1 and
C2 hold but focus does not change, it is FAIL. One retry is allowed after restoring the spacer and
clearing focus, into a separate attempt directory; earlier files are never rewritten and evidence
from other runs or HEADs is never combined. INCONCLUSIVE is never reclassified as PASS.

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
