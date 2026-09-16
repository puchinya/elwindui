# Issue #236 reviewer evidence

This directory contains the small, reviewer-facing evidence set for the WinUI3 TreeHost input
surface change. Raw command output and run artifacts remain under
`.agent-state/issues/236/e2e/<head>/<run-id>/`.

## Current result

- Implementation branch: `feature/236-treehostpanel-input-surface`
- Tested implementation HEAD: `98e5f9e204bfc0b5ae120c56b5a72f57e674fcde`
- Base used for the remediation: `origin/master` at `766c2a9ab24632e639e02e232fd2e861d834caad`
- Windows run: non-elevated interactive desktop, winapp `0.6.1`, doctor session `1`; immutable raw
  evidence is under `.agent-state/issues/236/e2e/98e5f9e204bfc0b5ae120c56b5a72f57e674fcde/20260916T124325Z/`.
- Launch policy: every application was launched by the driver using an absolute executable path,
  explicit repository-root `--cwd`, and `--wait-window-timeout 30`; no application was pre-started
  or kept running asynchronously outside the driver.
- SDP-01: `PASS`; one real click at screen `(259,232)` changed the visible page to Inspector with
  non-zero selected content. See `SDP-01/result.json` and its before/after screenshots in the raw
  run directory.
- SDP-02: `PASS`; one real drag from `(593,404)` to `(633,404)` moved the visible splitter by
  approximately 39 screen pixels, above the required 20-pixel threshold. See `SDP-02/result.json`.
- SDP-03: `PASS`; one real click at screen `(242,181)` visibly selected Document B with non-zero
  content and a live-layout change. See `SDP-03/result.json`.
- SDP-04: `PASS`; one real drag from `(268,207)` to `(128,207)` moved Document B into the upper
  vertical docking group and Document A into the lower group. The current vertical-group result
  is valid under the revised contract. See `SDP-04/result.json`.
- SDP-05: `BLOCKED`; after one real navigation click, fresh screenshots and current window
  geometry did not reliably identify the visible Normal native Button, and UIA searches returned
  zero matches. The acceptance click was not performed, so exactly-once native action is not
  claimed. See `SDP-05/result.json`. Issue #260 UIA discoverability is not an Issue #236
  acceptance dependency and was not changed by this remediation.

The earlier `711bbf5f292655bb524715ed0e8946c77db71af9` run at
`.agent-state/issues/236/e2e/711bbf5f292655bb524715ed0e8946c77db71af9/20260916T020000Z/` is
superseded by the final run above. Its historical results remain factual: SDP-01 and SDP-02 were
recorded as FAIL because the old case required UIA status text, SDP-03 was PASS, SDP-04 was FAIL
under the old horizontal-reversal expectation, and SDP-05 was BLOCKED without a guessed click.
Those old results are not rewritten under the revised contract.

The durable case definition is [`tests/e2e/self-drawn-pointer-input.md`](../../../../tests/e2e/self-drawn-pointer-input.md).
The case must be rerun on a normal, non-elevated, unlocked Windows desktop through
`tools/windows-ui-driver`; the final run above is the current implementation evidence. AppKit was
not run in this remediation.
