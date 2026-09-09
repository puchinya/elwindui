# Self-drawn pointer input

## Product behavior

Self-drawn ElwindUI content hosted inside a backend's native tree responds to real OS pointer
input exactly like a native control would: a real mouse click/drag over self-drawn content (a
`CustomTabView` header, a `CustomGridSplitter` handle, a Docking tab) reaches the intended
ElwindUI target, and a real mouse click over a genuine native control embedded in the same tree
is still delivered to that native control exactly once, with no duplicate self-drawn forwarding.

This is the regression Issue #236 fixed on WinUI3: a real OS pointer over *blank* self-drawn area
(no XAML render projection under it) was silently dropped before ever reaching Core, because the
backend's root native canvas had no hit-testable surface for XAML to resolve such a hit to.

## Backends

| Backend | Requirement |
|---|---|
| WinUI3 | Required — this is the backend Issue #236 regressed on. |
| AppKit | Supported — this scenario's semantics already hold there per `docs/status/custom_controls_status.md` / `docs/status/docking_status.md`; a native AppKit run through `docs/agents/appkit-e2e.md` is not required to close #236 and has not been executed as part of this case's initial authoring. |
| GTK4 | Unsupported until that backend can host and drive this scenario. |

## Execution

Execute through the platform tester procedure for the backend under test — `docs/agents/winui3-e2e.md`
for WinUI3, `docs/agents/appkit-e2e.md` for AppKit — using that backend's own repository-owned
driver (`tools/windows-ui-driver/` / `tools/macos-ui-driver/`). This file defines only the
scenario/setup/action/expected-postcondition/result semantics; exact driver commands, coordinate
derivation, and evidence layout belong to the platform tester procedure, not here.

Every scenario batch reports one of `PASS` / `FAIL` / `NOT RUN` / `BLOCKED`:

- **PASS** — the action was delivered and the application postcondition below was actually
  observed. A driver command's own `success: true` is not sufficient by itself.
- **FAIL** — the action reached the product (input delivered, or a UIA query resolved its target)
  but the resulting product state was wrong.
- **NOT RUN** — the case, or its required evidence, was never executed/collected.
- **BLOCKED** — a host/tool/session/security/foreground condition prevented the action from
  exercising the product at all.

For self-drawn UI, use UIA only to locate stable rendered text/native geometry and to observe
postconditions; the action under test itself (a tab click, a splitter/tab drag, a native control
click) must always be real mouse input, never a UIA `invoke` substituted for it.

## SDP-01 — CustomTabView real-pointer selection

Application: `custom-controls-demo` (`target/debug/custom-controls-demo.exe`).

Setup: fresh launch; wait until the tab header whose exact text is `Inspector` and the initial
status text are discoverable.

Precondition: the visible status text is the initial
`Selected tab: Overview · click a header, close affordance, or divider to exercise callbacks`.

Action: a real point-click at the center of the exact `Inspector` header's current bounds
(recomputed immediately before the click).

Expected postcondition:

- the status text becomes `Selected tab: Inspector · selected_index callback received 1`;
- the `Inspector` page content has non-zero visible bounds after selection.

This scenario must not use UIA `invoke` for the selection action itself.

## SDP-02 — CustomGridSplitter real-pointer drag

Application: `custom-controls-demo`, same process may be reused from SDP-01 after restoring a
known state.

Stable anchor: the element whose exact text is `Interaction surface`.

Action: a real drag starting at the splitter handle immediately to the left of the `Interaction
surface` region, moving horizontally by a fixed positive offset (recomputed from the anchor's
current bounds and the active DPI scale immediately before the drag — see the tester procedure for
coordinate derivation).

Expected postcondition:

- the status text matches `Grid resize completed: cumulative delta=<non-zero>px canceled=false ·
  panes resized`, with a positive, non-zero effective delta (allowing only normal pointer rounding
  tolerance, not a zero/silent completion);
- re-querying the `Interaction surface` anchor shows its screen-space left edge moved materially
  (at least 20 logical px, scaled) in the drag direction, proving an actual Grid mutation and not
  only a callback emission.

PASS requires both the callback/status state and the geometry change.

## SDP-03 — Docking real-pointer tab selection

Application: `docking-demo` (`target/debug/docking-demo.exe`).

Setup: fresh launch; wait for the exact-name header elements `Document A` and `Document B`.

Precondition: `Document A editor` is the visible selected document content.

Action: a real point-click at the center of the exact `Document B` header's current bounds.

Expected postcondition:

- `Document B editor` becomes visibly arranged with non-zero bounds;
- the demo's status transitions to `Committed a live layout change`.

Do not pass solely because the driver's click command reported `success: true`.

## SDP-04 — Docking real-pointer tab drag/reorder

Application: `docking-demo`, reusing SDP-03's process after restoring the known
`Document A`, `Document B` tab order.

Precondition: `Document A` header's x-position is less than `Document B` header's x-position
(re-queried immediately before the action).

Action: a real drag starting at the center of the `Document B` header, releasing in the left half
of the `Document A` header's current bounds — sufficiently beyond the drag-start threshold to
register as a reorder rather than a click.

Expected postcondition:

- the demo's status transitions to `Committed a live layout change`;
- re-querying both headers afterward shows `Document B` header's x-position is now less than
  `Document A` header's x-position.

The spatial-order assertion is mandatory; the generic status message alone does not prove the
intended reorder. If the current Docking implementation intentionally normalizes a same-group
reorder differently, report the exact conflict rather than redefining this scenario's expected
postcondition unilaterally — a Docking semantics change is outside Issue #236.

## SDP-05 — NativeControl remains native-only / exactly once

Application: `controls-demo` (`target/debug/controls-demo.exe`).

Purpose: verify the self-drawn input surface does not cause a real native XAML child to be
forwarded into Core's self-drawn dispatch path as well — the mixed-tree half of Issue #236's
acceptance criterion.

Setup: navigate to the tab whose header is exactly `Button`; confirm that page is visible; confirm
its event log is initially empty.

Action: exactly one real mouse click on the native button whose text is exactly `Normal`.

Expected postcondition: the event log contains exactly one appended line, `Normal clicked` — never
two lines from one gesture.

The postcondition must be observed from the application's own displayed event log, not from driver
action success alone.

## Cleanup

Always terminate every launched process through the platform driver's `terminate` (or equivalent)
command, even after an abnormal result. Do not leave a launched application running after a
completed run.
