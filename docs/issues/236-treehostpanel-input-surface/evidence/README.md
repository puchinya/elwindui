# Issue #236 reviewer evidence

This directory contains the small, reviewer-facing evidence set for the WinUI3 TreeHost input
surface change. Raw command output and run artifacts remain under
`.agent-state/issues/236/e2e/<head>/<run-id>/`.

## Current result

- Implementation branch: `feature/236-treehostpanel-input-surface`
- Tested implementation HEAD: `711bbf5f292655bb524715ed0e8946c77db71af9`
- Base used for the remediation: `origin/master` at `766c2a9ab24632e639e02e232fd2e861d834caad`
- Windows run: non-elevated interactive desktop, winapp `0.6.1`, doctor session `1`; immutable raw
  evidence is under `.agent-state/issues/236/e2e/711bbf5f292655bb524715ed0e8946c77db71af9/20260916T020000Z/`.
- Launch policy: every application was launched by the driver using an absolute executable path,
  explicit repository-root `--cwd`, and `--wait-window-timeout 30`; no application was pre-started
  or kept running asynchronously outside the driver.
- SDP-01: `FAIL` under the durable case contract. The real click changed the visible page to
  Inspector, but the required status text was not observable through UIA (`matchCount: 0`).
- SDP-02: `FAIL` under the durable case contract. The real drag moved the splitter by 40 px, but
  the required status text was not observable through UIA (`matchCount: 0`).
- SDP-03: `PASS`; Document B became visible and the live-layout status was observed.
- SDP-04: `FAIL`; the live-layout status was observed, but Docking created a vertical group rather
  than the required horizontal header reversal. Docking semantics are outside #236.
- SDP-05: `BLOCKED` and a remaining implementation item; the Button page was visible, but exact
  UIA searches for `Button` and `Normal` returned zero elements. The required native click and
  exactly-once `Normal clicked` observation were not performed because guessing coordinates would
  invalidate the case.
- Screenshots for the executed run remain in the immutable Issue-scoped evidence directory; no
  product PASS is claimed for the incomplete cases.

The durable case definition is [`tests/e2e/self-drawn-pointer-input.md`](../../../../tests/e2e/self-drawn-pointer-input.md).
The case must be rerun on a normal, non-elevated, unlocked Windows desktop through
`tools/windows-ui-driver` before Issue #236 can be accepted.
