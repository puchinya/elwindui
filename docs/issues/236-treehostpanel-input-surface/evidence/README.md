# Issue #236 reviewer evidence

This directory contains the small, reviewer-facing evidence set for the WinUI3 TreeHost input
surface change. Raw command output and run artifacts remain under
`.agent-state/issues/236/e2e/<head>/<run-id>/`.

## Current result

- Implementation branch: `feature/236-treehostpanel-input-surface`
- Base used for the remediation: `origin/master` at `766c2a9ab24632e639e02e232fd2e861d834caad`
- H1–H5 hosted structural helper: compiled in the existing single-Application test binary; live
  execution was blocked because this session did not expose an interactive WinUI3 test window.
- SDP-01 through SDP-05: `BLOCKED` / not executed on the final implementation HEAD. The required
  bounded GUI tester sub-agent was unavailable, and the direct hosted-XAML test process could not
  complete in the available interactive session. No product PASS is claimed.
- Screenshots: none committed because no required SDP postcondition was observed.

The durable case definition is [`tests/e2e/self-drawn-pointer-input.md`](../../../../tests/e2e/self-drawn-pointer-input.md).
The case must be rerun on a normal, non-elevated, unlocked Windows desktop through
`tools/windows-ui-driver` before Issue #236 can be accepted.
