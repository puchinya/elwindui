# Pointer cancellation and capture loss

This is the durable, backend-neutral verification scenario for the pointer cancellation and
implicit-capture bridge owned by Issue [#180](https://github.com/puchinya/elwindui/issues/180).
It verifies the common cancellation contract using deterministic Core/custom-control evidence,
WinUI3 host tests, and real pointer input where the existing product fixtures expose an observable
postcondition. It does not add an input-injection mechanism or call Core cancellation as a
replacement for a native event.

The WinUI3 execution procedure is [`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md).
The existing normal-pointer and renderer-proxy regression rows are in
[`self-drawn-pointer-input.md`](self-drawn-pointer-input.md) and must be rerun on the exact HEAD
under test; an earlier Issue #236 result is not evidence for this scenario.

## Setup and evidence rules

1. Start from a clean checkout and record `git rev-parse HEAD`, `git rev-parse origin/master`,
   Windows version/build, the normal non-elevated user, and the host-context classification.
2. Run `windows-ui-driver.ps1 doctor` once per session. Record its `winapp_version`,
   `session_id`, and `input_desktop_probe`.
3. Build the existing fixtures with the repository's Windows setup procedure. Use
   `custom-controls-demo.exe` for the custom-control rows and `controls-demo.exe` for the native
   control boundary. Use the existing self-drawn-pointer-input fixture for PC-13 and PC-14.
4. Before every real pointer action, reacquire the current PID/HWND, window rectangle, DPI, and
   target bounds. Derive screen coordinates from the current window rectangle; never reuse a
   coordinate after a resize, move, focus change, or layout mutation.
5. Store raw command JSON, screenshots, runtime details, and the compact result under
   `.agent-state/issues/180/e2e/<head>/<run-id>/`. End every launched process with
   `terminate --pid <pid> --timeout 5`, including blocked and failed cases.

Each row records both action delivery and the application postcondition. `success: true` from the
driver is not a product PASS. Use the vocabulary from `winui3-e2e.md`:

- `PASS`: the required action reached the product and the required postcondition was observed;
- `FAIL`: the action reached the product but the postcondition was wrong;
- `NOT RUN`: the case or required evidence was not executed/collected;
- `BLOCKED`: a host, session, security, tooling, or observability condition prevented the case
  from exercising or proving the required behavior.

## Case matrix

### PC-01 — Native `PointerCanceled`, latest payload

Evidence target: WinUI3 native event plus Core callback observation.

Start a fresh self-drawn gesture in the existing self-drawn fixture, record at least one moved
position, and attempt to cause the OS/native Canvas `PointerCanceled` event with the existing
Windows host/driver facilities. Observe the target's cancellation count and the callback payload.

PASS requires exactly one `on_pointer_canceled`, the latest logical/root/screen position in the
payload, and `button=None`. Core-only `RawPointerEventKind::Canceled` dispatch is not native
evidence. If the existing fixture cannot expose the callback payload or the host/driver cannot
deliver the native event, record the exact attempted action and classify only this row
`BLOCKED`/`NOT RUN`; do not infer PASS from source inspection.

### PC-02 — Native `PointerCaptureLost`

Evidence target: WinUI3 native capture-loss event plus one Core cancellation.

Start an active self-drawn gesture and attempt to cause the Canvas capture to be lost using only
existing native/window actions and the Windows UI driver. Observe the target's cancellation count
and completion state.

PASS requires a delivered native capture-loss action, one Core cancellation, and no normal release
or tap result. If the existing driver cannot hold a real pointer down while performing the required
native capture-loss action, classify `BLOCKED` with that limitation and the exact attempted
mechanism. Do not add a new generic injection architecture for this row.

### PC-03 — Duplicate native sources

Evidence target: the cancellation path followed by a possible native capture-loss callback.

Use the PC-01 or PC-02 gesture only when the host has delivered the first native cancellation,
then retain the same window/session long enough to observe any subsequent capture-loss callback.

PASS requires the Core cancellation count to remain exactly one. A driver success without an
application count/postcondition is not PASS; if PC-01/PC-02 is blocked, this row is `NOT RUN`.

### PC-04 — Native capture release

Evidence target: native capture no longer routes later input to the canceled target.

After a real native cancellation has been observed, move and click a different visible
self-drawn target using a freshly resolved coordinate. Observe that the old target receives no
later moved/released/tapped callback and that the new target receives the fresh sequence.

PASS requires both the native cancellation and the post-cancel routing observation. If no native
cancellation can be delivered, classify `NOT RUN` rather than substituting a synthetic Core call.

### PC-05 — No active gesture

With no pressed pointer, attempt the available native cancellation/capture-loss stimulus and
observe the application log/counter. PASS requires no callback, no target retention, and no state
change. The Core deterministic cancellation no-op test is supporting evidence; it does not replace
the native stimulus when a native stimulus is required.

### PC-06 — Recognition suppression

On a self-drawn target, begin a gesture, observe a moved position, deliver a real cancellation when
available, then deliver the matching release/click sequence. PASS requires no tapped,
double-tapped, or right-tapped result from the canceled gesture. Map the deterministic Core
assertion to the row and record any live native limitation separately.

### PC-07 — Recovery and fresh hit test

After cancellation, move to a different self-drawn target, refresh its geometry, and perform a new
real pointer sequence. PASS requires the new target to receive the sequence and the old captured
target not to receive it, proving that the next move starts with a fresh hit test.

### PC-08 — `CustomGridSplitter` rollback

Launch `custom-controls-demo.exe`, locate the current splitter from a fresh screenshot or the
`Interaction surface` anchor, and perform a real drag. For cancellation evidence, use the same
native cancellation attempt as PC-02; do not call Core cancellation directly. Observe the visible
pane geometry and the status text.

PASS requires `Grid resize completed: ... canceled=true`, with provisional pane movement rolled
back before the completion observation. The deterministic row
`pointer_dispatcher_cancellation_restores_grid_before_notification` is required supporting
evidence. A normal drag is the regression control and must report `canceled=false`.

### PC-09 — `CustomTabView` canceled drag

In `custom-controls-demo.exe`, start a real drag on a visible tab header, attempt the same native
cancellation stimulus, and observe the status text and selected tab. PASS requires
`Tab drag completed: ... canceled=true` and no drag commit or selection mutation caused solely by
the canceled drag. The deterministic tab cancellation row is required supporting evidence.

### PC-10 — Captured subtree removal

Use the deterministic lifecycle/custom-control coverage to press a target, remove its captured
subtree, and observe the ordering of cancellation before unmount and the absence of later dispatch
to the old target. PASS requires no retained target after removal. No live fixture is to be added
just to replace this existing deterministic evidence.

### PC-11 — Host/window teardown

Start a gesture in the existing hosted WinUI3 fixture, close/tear down the host through its normal
window lifecycle, and observe callback count and capture release. PASS requires one cancellation,
released native capture, and normal teardown without duplicate callback. If the window cannot be
closed while an atomic driver drag is active, classify the native portion `BLOCKED` and retain the
deterministic teardown evidence.

### PC-12 — NativeControl boundary

Launch `controls-demo.exe`, navigate to the visible native `Normal` button, confirm the event log is
empty, refresh its current geometry, and perform exactly one real coordinate click. PASS requires
exactly one newly appended `Normal clicked` line and no Core cancellation/tap duplicate. Follow
the NativeControl procedure in SDP-05; UIA invocation is not a substitute for the real click.

### PC-13 — Normal pointer regression

On the exact #180 HEAD, rerun the relevant real-pointer rows from
[`self-drawn-pointer-input.md`](self-drawn-pointer-input.md): SDP-01 tab selection, SDP-02
splitter drag, SDP-03/04 when the shared self-drawn surface is used by the tested backend, and
the normal control portion of SDP-05. PASS requires the original pressed/moved/released capture
behavior and each row's visible postcondition. Record each row separately in the evidence run.

### PC-14 — Renderer proxy regression

On the exact #180 HEAD, rerun the renderer/text-proxy portion of the existing self-drawn pointer
case with a fresh screenshot and current geometry. PASS requires a click/drag over the rendered
text or proxy area to reach the underlying self-drawn target, with no stale or duplicate routing.
If the target cannot be identified reliably from current visual evidence, classify `BLOCKED`.

## Deterministic supporting commands

Run these from the repository root before and after the Windows host run:

```text
cargo test -p elwindui-core cancel
cargo test -p elwindui-custom-controls --test controls pointer_dispatcher_implicit_capture_completes_tab_outside_and_cancels
cargo test -p elwindui-custom-controls --test controls pointer_dispatcher_cancellation_restores_grid_before_notification
cargo test -p elwindui-custom-controls --test controls close_pointer_canceled_does_not_request_close_or_select
```

These tests prove Core/custom-control ordering, tap suppression, latest payload, reentrancy,
subtree/unmount retention, splitter rollback, and tab cancellation. They are not evidence that a
WinUI3 native Canvas event was raised.

## Cleanup and reporting

Report PC-01 through PC-14 one-to-one with the immutable evidence directory, including the exact
action-delivery result and application postcondition. Attach only useful screenshots to Issue
#180; keep raw logs under `.agent-state`. If a native event cannot be deterministically produced,
record the mechanism attempted and create a focused follow-up Issue when an additional driver or
native injection capability is genuinely required. Do not fix a product defect or weaken the PASS
criteria in this verification branch.
