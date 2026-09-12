# Core-owned accessibility semantics

This is the single backend-neutral durable scenario for the first accessibility baseline. The
AppKit and WinUI 3 runners may use different driver commands, but they must execute these same
cases and compare the same semantic postconditions.

## Fixture

Launch a clean application window containing these logical Core elements. Every identifier below is
stable across runs and is an accessibility identifier, not a native view identifier.

| Element | Identifier | Required semantic expectation |
| --- | --- | --- |
| `TextBlock` | `a11y-text` | Static text, value is displayed text, no action |
| `Button` | `a11y-button` | Button, label `Activate`, Activate and Focus |
| `TextArea` | `a11y-text-area` | Text input, current value, SetText and Focus |
| `CheckBox` | `a11y-check-box` | Check box, checked state, Activate and Focus |
| `Slider` | `a11y-slider` | Slider, current/min/max/step, SetValue/Increment/Decrement/Focus |
| self-drawn `Canvas` | `a11y-canvas` | explicit Group or application-selected role, label `Canvas semantic`, no native dependency |
| hidden subtree | `a11y-hidden` | absent from the semantic tree, including descendants |
| exiting `TextArea` | `a11y-exiting-text-area` | absent semantically immediately after logical removal |

The fixture exposes a small observable application-state readout with identifier
`a11y-result`. The button, check box, slider, and text-area actions update that readout through the
normal Core event/property path.

## Shared procedure and assertions

1. Launch from a clean process and acquire the host semantic root.
2. Enumerate semantic descendants by identifier. Each expected visible element is discoverable
   exactly once; no raw native control is an additional public node.
3. Assert role, name/label, value, checked state, enabled state, and bounds agree with the fixture.
   Bounds are host-root-relative in Core and screen-converted by the platform adapter; the runner
   must not infer title-bar or window-decoration coordinates.
4. Invoke `a11y-button` with Activate. Assert `a11y-result` changes once and the semantic tree
   remains valid.
5. Invoke `a11y-check-box` with Activate. Assert both application state and checked semantics
   change once.
6. Invoke `a11y-slider` with SetValue. Assert the normal value path updates application state and
   the reported value. Exercise Increment or Decrement once when the advertised step is present.
7. Invoke Focus on `a11y-text-area`. Assert Core focus and platform accessibility focus identify
   the same element.
8. Invoke SetText on `a11y-text-area`. Assert the normal text/property path updates both the
   application readout and semantic value.
9. Assert `a11y-canvas` is discoverable exactly once although it has no native control child.
10. Assert `a11y-hidden` and all descendants are absent.
11. Logically remove `a11y-exiting-text-area` while its visual exit transition is active. Assert it
    is absent from the semantic tree immediately, while a bounded visual checkpoint may still find
    its fading pixels.
12. Wait for the transition completion checkpoint. Assert the exiting visual is absent and the
    stale accessibility identifier cannot dispatch an action.
13. Tear down the window and assert cleanup succeeds; late platform queries return unavailable.

## Evidence and result classification

Run the complete procedure twice per backend from clean launch. Each run records the tested commit,
backend, driver version, host/window identity, and evidence paths. Classify a run as:

- `PASS`: all Core snapshot, adapter, action, focus, duplicate-suppression, transition, and cleanup
  assertions pass;
- `FAIL`: the product or adapter violates a shared assertion;
- `NOT RUN`: a planned run was not attempted;
- `BLOCKED`: an external host, permission, SDK, authentication, or driver prerequisite prevents a
  meaningful run.

The failure record must distinguish Core snapshot failure, adapter mismatch, duplicate native
exposure, driver limitation, and environment limitation. Password-box fixtures are intentionally
excluded from diagnostic output so no plaintext can enter evidence or logs.
