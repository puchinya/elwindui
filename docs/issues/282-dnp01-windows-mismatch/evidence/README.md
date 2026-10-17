# Issue #282 reviewer evidence index

Reviewer-facing index for the Windows/WinUI3 DNP-01 tab-selection mismatch investigation.
Raw evidence (driver stdout/stderr/exit codes, UTC timings, screenshots, `manifest.json`,
`clicks.jsonl`) stays local under `.agent-state/issues/282/e2e/<head-short>/<run-id>/` and is not
copied into the repository. The session table and classification are in [`summary.md`](summary.md).

## Scope and fixed inputs

- Case: [`tests/e2e/docking-native-parity.md`](../../../../tests/e2e/docking-native-parity.md) DNP-01.
  Windows/WinUI3 only; AppKit #280 is out of scope.
- **Tested product HEAD (primary aggregate):** `a445183524e4f688273a50b8fcccd61dbd87bdbe`
  (`origin/master` when the maintainer decided on 2026-10-17). It was built and run with that
  HEAD's own `docking-demo` and `tools/windows-ui-driver/`. The contract originally fixed
  `0366fc8271289801dd109c6431a00296a13154ab`. The maintainer then moved the tested HEAD to master
  ([decision](https://github.com/puchinya/elwindui/issues/282#issuecomment-6229287707)). The three
  sessions already run on `0366fc8` are kept as a separate set and are not part of the primary
  aggregate.
- Binary: `target/debug/docking-demo.exe`, prebuilt native mode (default), SHA-256
  `7acc82e056564c6b195aa6cc59587ce370ad14fe4c11db95b4834243117bd8b4`.
- Host: Windows 10 Pro 10.0.19045, RDP logon session 2, normal non-elevated user, unlocked
  interactive desktop, 96 DPI, window 960x640.
- Driver: repository `tools/windows-ui-driver/windows-ui-driver.ps1` at the tested HEAD
  (SHA-256 `70b83add2a80b2d3d042c09cbde3e0b8f1b000af82b9a4d2fbee5e16f9503acb` on master;
  `2e9ee260e25d9ed48ec241a783db8741f7b115648b899ed1127b38b71a6a7312` on `0366fc8`).
  External `winapp` 0.6.1 (package executable SHA-256 `d661c306e1909fd54a3c0d2a3c0a6330ab470cbf5a5fb80598efffcf33968a5c`).
  `driver-contract.ps1` exit 0. `doctor`: success, `input_desktop_probe=available`.
- Real input: one `point-click` per click. `point-click` is a zero-distance `winapp ui drag`. No
  sleeps, no synthetic selection, and no re-clicks.
- Reset on master: the maintainer-approved adaptation is UIA `search "Tools"` → invoke →
  `search "Reset layout"` → invoke, using the returned selectors
  ([decision](https://github.com/puchinya/elwindui/issues/282#issuecomment-6229439239)). In
  master's redesigned demo, `Reset layout` is only shown after `Tools` is pressed.

## Commands per session

`launch` → `list-windows` → reset (`search`/`invoke`) → `capture-window --capture-screen`, then
for each click: `list-windows` → before `capture-window` → `list-windows` (stale-geometry guard) →
`point-click` → immediate after `capture-window` → `list-windows`. On an anomaly, one extra
read-only `capture-window` (`late`). Every session ends with `terminate --pid <pid> --timeout 5`.
The GUI work was done by a bounded tester sub-agent following a fixed instruction sheet. The
main agent cross-checked every run's mechanical evidence and spot-checked screenshots.

## Result

- Master `a445183`: **10/10 sessions PASS, 200/200 real clicks PASS**, cleanup never forced.
  Conclusion: **`not_reproduced_in_scope`**, limited to this HEAD, host, and session range. This
  does not prove the defect is absent.
- Old HEAD `0366fc8` (separate set): S1 PASS 20/20, S2 PASS 20/20, S3 NOT RUN after 1 click
  (`unclassified`; see [`summary.md`](summary.md)).
- Historical #279 click-18 FAIL on `7b33b4d…`: kept unchanged as distinct evidence. It is not
  part of either set.

## Unverified

- No other Windows hosts, physical (non-RDP) console sessions, DPI other than 96, or ARM64.
- Source-native WinUI3 build mode was not run.
- The cause of the old-HEAD S3 spontaneous layout change and of the historical click-18 mismatch
  remains unclassified.
