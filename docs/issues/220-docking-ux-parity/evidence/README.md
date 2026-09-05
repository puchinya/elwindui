# Issue #220 / PR #221 AppKit E2E Evidence

This directory contains the small reviewer-visible evidence set for the final AppKit closure.
Raw command output remains under `.agent-state/issues/220/e2e/` on the verification host and is
not committed.

## Session and repository identity

- Issue: [#220](https://github.com/puchinya/elwindui/issues/220)
- PR: [#221](https://github.com/puchinya/elwindui/pull/221)
- Native-tested PR head: `f677d6970a61286aa68a23fbaf30c0d0992089e3`
- Final post-master-sync PR head: `ce2c066` (the final synchronized evidence tree; the delivery metadata commit follows)
- Integrated `origin/master`: `1517ecd92b8a3b477d6f5b0cdcfc14a9ab2ac5bf`
- `origin/master` is an ancestor of the final PR head: yes
- macOS: `26.6.2 (Build 25G83)`
- Architecture: `arm64`
- Tester: `ElwindUI AppKit E2E Tester` using `gpt-5.6-luna` / standard `medium` reasoning
- Driver binary SHA-256: `2c42195cbceb82b1159b900813fdc50e1cd6877c0c43912503003cdc44f2e587`
- Driver source fingerprint: `b35e53836f876fd6ec86833d1aa03da4726c9f4cbb72ae30e45149d3bbc78506`
- Driver freshness: `SYNCED`
- `doctor.accessibility`: `true`
- `doctor.screen_recording`: `true`

## Required native results

- Floating A -> B: PASS. The real 45-step drag moved Document A from source floating window
  `19319` to target floating window `19321`; the source disappeared, the destination retained
  Document B and Document A, Document A was active and interactive, and the process survived
  native close. See [floating-to-floating-after.png](floating-to-floating-after.png).
- Snapshot bounds: PASS. PID `5687`, MAIN `19554`, FLOAT `19570`; A was `(120,120,647,355)`,
  B was `(520,360,527,275)`, and C was `(120,120,647,355)`. Every C component is within two
  points of A and C differs from B.
- Horizontal Splitter: PASS. 12 seconds and 480 tracking steps; see
  [splitter-horizontal-mid.png](splitter-horizontal-mid.png).
- Vertical Splitter: PASS. 12 seconds and 480 tracking steps; see
  [splitter-vertical-mid.png](splitter-vertical-mid.png).
- Menu wrapper lifetime: PASS. The main-thread native selection path retained the native item
  after the caller's `Rc` was dropped and reported `native_item_retained=true` and
  `callback_count=1`, with empty runtime stderr.
- Native close/process survival: PASS. The remaining floating host closed natively and the demo
  process remained alive.

## Validity audit

The master merge was performed with `git merge origin/master`, without rebasing or force-pushing.
The merge changed only WinUI3 implementation/tests and workflow/status documentation in the
master-only side. It did not change the AppKit backend, Core, Custom Controls, Docking,
`docking-demo`, or `macos-ui-driver` paths used by the accepted AppKit cases. The existing native
evidence therefore remains valid and no AppKit case required a rerun after synchronization.

The checked-in driver binary was preserved unchanged during this remediation. Its source/bin
freshness is checked by `tools/macos-ui-driver/verify-e2e-binary.sh`; future driver-source changes
must rebuild the binary, preserve mode `100755`, update `bin/PROVENANCE.md`, and rerun host-context
`doctor`.
