# WinUI3 Docking native interaction parity

Issue [#226](https://github.com/puchinya/elwindui/issues/226) durable, backend-neutral product E2E
case. WinUI3 execution is required for #226. AppKit may reuse this case when shared-runtime
changes invalidate or extend its native Docking evidence. GTK4 is not a native-floating acceptance
backend while native Window support is unavailable.

This case accepts product behavior, not driver return values. Native execution follows
[the WinUI3 tester guide](../../docs/agents/winui3-e2e.md) and uses
[the repository Windows UI driver](../../tools/windows-ui-driver/README.md). UIA may locate or
observe targets and activate native menu/button controls when pointer delivery is not the subject.
Real mouse input is required for self-drawn clicks, drags, splitter gestures, capture continuity,
and right-click context requests. Coordinates come from fresh HWND/window geometry and visible
target geometry immediately before each action. Reacquire HWNDs and geometry after every topology,
move, resize, dock, undock, create, or close operation.

## Result vocabulary

Every row is exactly one of:

- PASS — the action reached the product and the required application/window/model-visible
  postcondition was observed.
- FAIL — the native action reached the product but the postcondition was wrong.
- NOT RUN — the action or required evidence was not executed or collected.
- BLOCKED — a host, tool, session, security, foreground, or equivalent capability prevented
  exercising the product.

A driver success: true is never product PASS. A controlled retry is allowed at most once, only
after restoring the row's documented precondition.

## Fixture and common rules

Use examples/docking-demo without adding public Docking API. Its authored controls are the
capability fixture:

- Error List: can_close = false.
- Output: can_float = false.
- Git Changes: can_dock = false.
- Document A and Document B: can_pin = false.
- Solution Explorer and Terminal: positive controls for permitted operations.

Each independent reset restores the authored layout through the normal demo action. Use fresh
screenshots when self-drawn chrome is not exposed through UIA. Capture popup/context overlays with
the screen-capture mode. Do not infer a product result from a button/menu action alone.

Store immutable evidence in
.agent-state/issues/226/e2e/<head-short>/<run-id>/. Each run records repository HEAD,
origin/master, Windows version/build/session, winapp version and one doctor result, PID,
all live HWNDs with geometry, action JSON, required screenshots, numeric snapshot bounds,
splitter displacement/duration/tool capability, native-close/removal HWND sets, row result, and
cleanup forced=true|false.

Cleanup terminates every launched process through the repository driver with the documented
timeout, including blocked and failed runs. Do not overwrite an earlier run directory.

## DNP-01 — rapid tab selection

Setup: reset the authored layout with Document A selected.

Action: perform 20 alternating real clicks on the exact visible Document B and Document A headers,
without artificial delay beyond driver command completion. Re-resolve target geometry when layout
changes.

PASS requires every delivered click to leave a valid selected page, the final page to match the
final clicked header, no duplicate/ghost content, crash, hang, or unintended reorder, and a
responsive main window.

## DNP-02 — same-group reorder

Setup: reset with visible header order Document A, Document B.

Action: real-drag Document B into the left half of Document A's header.

PASS requires visible header order Document B, Document A; both pages present; the dragged item
active; and one valid layout change. A split into another group is not PASS.

## DNP-03 — cross-group Center

Drag a dockable document tab into the center/header insertion region of another group.

PASS requires that it becomes a tab of that destination group at the resolved insertion position
and that no split is created.

## DNP-04L/T/R/B — group-relative Split targets

From reset, run four independent subcases. Drop a dockable item into the intended Left, Top,
Right, or Bottom split band of an interior destination group.

PASS requires a visible new pane on the requested side of that destination group.

## DNP-05L/T/R/B — surface-root Dock targets

From reset, run four independent subcases. Drop a dockable item into the main DockSurface root
edge band for Left, Top, Right, or Bottom.

PASS requires the new pane to attach at the requested surface-root edge, not merely split relative
to the nearest group.

## DNP-06 — item tear-out and outside-bounds continuity

Real-drag a floatable tab from the main window to a valid usable-desktop point outside the main
HWND.

PASS requires the drag to complete after leaving the source bounds, exactly one new floating HWND,
visible/interactable item content there, and a usable source main window.

## DNP-07 — whole-group tear-out

Use the group title-bar drag handle.

PASS requires the complete group to move to one floating HWND with all contained tabs/pages and
selection preserved.

## DNP-08 — main to existing floating

Create one floating target, then real-drag a dockable item from main into its Center target.

PASS requires the item to join the floating group and both main and floating HWNDs to remain usable.

## DNP-09 — floating to main

Real-drag a dockable item from a floating HWND into a main-window Center or valid root target.

PASS requires the item in main, the floating source to update or disappear when emptied, and no
stale source HWND. After every move, resize, dock, undock, close, or other topology change,
rediscover the live HWND set and target geometry; an invalidated cached HWND is not by itself a
BLOCKED result. The chosen main target must also be visibly reachable and not covered by the
source floating HWND; if a setup places the source over that target, restore the setup before
classifying the action. If a delivered native move leaves the logical floating root without its
required live native HWND, classify the row FAIL.

## DNP-10 — floating to floating

Create two independent floating HWNDs. Real-drag a dockable item from floating A into floating B.

PASS requires A to update/disappear when emptied, B to contain the item and remain interactive,
page identity to be preserved, and the main process to survive.

## DNP-11 — simultaneous floating windows

Maintain at least two non-empty floating windows concurrently.

PASS requires the main HWND and both floating HWNDs to be enumerated and each to accept an
interaction affecting only its expected state.

## DNP-12 — native floating move/resize callback

Move and resize one floating HWND with repository driver window-control commands.

PASS requires OS-reported bounds to change, subsequent Docking save/snapshot behavior to reflect
the new logical bounds, and no duplicate floating root/window.

## DNP-13 — snapshot A/B/C native bounds persistence

1. Place a floating HWND at geometry A and record current OS bounds.
2. Save a snapshot through the normal demo action.
3. Move/resize to materially different geometry B.
4. Restore the snapshot.
5. Record geometry C.

PASS requires B materially different from A; C matching A for left/top/width/height within
max(2 px, ceil(2 * dpi / 96)); and restored content/selection remaining interactive. Snapshot
evidence must not persist HWND or native-object identity.

## DNP-14 — capability gates

Run independent resets as needed.

- Error List: Close cannot remove it.
- Output: item tear-out/Float cannot create a floating root.
- A group containing Output: whole-group Float is rejected.
- Git Changes: docking into another group/root is rejected.
- A group containing Git Changes: whole-group Dock is rejected.
- Document A or Document B: Auto Hide / Pin is disabled or rejected.
- Terminal or Solution Explorer: corresponding permitted operations succeed as positive controls.

PASS requires every rejected operation to make no structural change and no fallback placement or
stale preview.

## DNP-15 — auto-hide, open, and pin back

Use Solution Explorer. Request Auto Hide / Pin, activate its auto-hide strip item, observe the
side-aware overlay, then use the visible pin affordance to return it. Auto-hide chrome may be
self-drawn and need not be UIA-discoverable: when UIA does not expose the strip or overlay, use a
fresh whole-screen capture and current window geometry to identify and real-click the visible
target. UIA non-discoverability alone is not BLOCKED.

PASS requires one overlay, an interactive item, and return to the remembered/default live placement
without duplication. If the normal action is delivered but the required strip/overlay is absent,
classify FAIL; classify BLOCKED only when host/tool/capture conditions prevent identifying or
clicking a visually present target after the required fallback.

## DNP-16A/B/C/D — context close actions

Each subcase starts from reset:

- A: Close on one closeable tab.
- B: Close Others.
- C: Close Tabs to Left.
- D: Close Tabs to Right.

Open the context menu with a real right click. Select the exact native menu item by unique UIA
match when available; otherwise capture the screen and real-click the identified menu row. If
neither location is reliable, classify the subcase BLOCKED.

PASS requires exactly the expected tabs to disappear and protected tabs to remain.

## DNP-17 — context menu native lifetime

Open a tab context menu, let the originating pointer command return, then activate one permitted
action.

PASS requires the native menu to remain valid, exactly one callback delivery, a successful action,
and a healthy process. Do not add a strong callback cycle.

## DNP-18 — column-resize Splitter native drag

Perform a normal real-mouse short drag as a regression control.

PASS for the short control requires the boundary to move by a non-zero expected x displacement and
the persisted model not to jump again on release.

Then perform a genuinely continuous left-mouse tracking run of at least three seconds if the
approved driver/tool version can express it. --hold-ms and --dwell-ms are not substitutes for
movement duration. If the approved tool cannot express the run, classify only the long-run subrow
BLOCKED with the exact version limitation and leave #226 open.

## DNP-19 — row-resize Splitter native drag

Same as DNP-18 for the orthogonal y-axis path, with the same continuous-tracking and BLOCKED
rule.

## DNP-20 — light/dark theme preservation

Create a non-default live layout with at least one moved or floating item. Switch light to dark
to light through normal demo controls and use bounded screenshot checkpoints.

PASS requires readable Docking chrome visibly following the selected theme, unchanged selection,
group ownership, floating HWND count, and layout topology, with no duplicate callbacks/windows.

## DNP-21 — native title-bar close, allowed

Float a closeable item or group. Freshly identify the native title-bar Close button for that
floating HWND and activate it.

PASS requires the request to be accepted; the floating HWND to disappear; all contained closeable
items to be removed once; no stale second floating HWND; a live process; and a subsequent
successful main-window selection/action.

## DNP-22 — native title-bar close veto

Float a root containing Error List and activate that floating HWND's native title-bar Close.

PASS requires close veto, the HWND to remain, all model contents to remain, no partial item close,
and main/floating UI to continue responding.

## DNP-23 — programmatic floating removal/redock

Float a permitted item, then use normal demo reset/restore behavior that removes the floating root
without clicking its title-bar Close.

PASS requires the stale floating HWND to disappear, the item to return to its expected
authored/restored placement, no crash or use-after-free, and a subsequent main interaction.

## DNP-24 — repeated lifecycle and stale HWND check

Repeat create -> interact -> remove/close for a floating host at least three times, alternating
native close and programmatic removal.

PASS requires each list-windows --pid <pid> result to contain only the expected live HWND set,
no accumulating HWND count, no closed HWND addressable as live, a successful final main
interaction, and normal final process termination without force.

## DNP-25 — composed exactly-once Window-release evidence

Native part: DNP-21, DNP-23, and DNP-24 must PASS.

Deterministic backend part: run
cargo test -p elwindui-backend-winui3 hosted_button_text_and_window_lifecycle_regressions_work
and retain the Docking deterministic tests for stable host identity, veto, one-host close, and
redock.

PASS requires both layers. Do not invent a second runtime trace solely to expose the private app
registry unless an observed defect requires such instrumentation and design approves it. Report
exactly one four-state DNP-25 result, derived from the required native rows and deterministic
evidence; do not replace it with descriptive partial prose.

## Cleanup and reporting

The tester does not edit product code, durable case definitions, Issue/PR state, or commit/push.
Failure diagnosis and any architecture-preserving repair belong to the main agent. Required native
rows are not considered complete until the action-delivery record and product postcondition
evidence are both present.

The final matrix must report DNP-01 through DNP-25 as PASS, FAIL, NOT RUN, or BLOCKED. Any FAIL,
NOT RUN, or BLOCKED required row prevents claiming Issue #226 completion.
