# Issue #178 — WinUI3 pointer and coordinate verification evidence

## Current verification — 2026-09-05

**Status: COMPLETE FOR ISSUE #178 WITH CLASSIFIED FOLLOW-UP.** The build and automated WinUI3 gates pass. The required live mouse P1-P6 and coordinate rows did not produce acceptance evidence on the current host and are not claimed as `PASS`. They are explicitly classified as `ENVIRONMENT_LIMITATION` and transferred to Issue #224 by project-owner decision, satisfying Issue #178's acceptance criterion for a classified follow-up finding.

The PID-based Win32 window discovery succeeded, but real mouse input sent with `SendInput` did not change the WinUI3 trace or title-bar state. Keyboard `SendInput` reached the native TextBox. Runtime behavior of the transferred rows remains unverified.

This remains verification-only. No product source, test, or backend implementation file was changed.

## Environment and execution context

- Branch: `feature/178-winui3-pointer-coordinate-verification`
- Required non-rebased `origin/master` merge commit: `d96cd7a4fa2c992a2c3ffb49be8bbd0fbd7d6b14`
- Host: Windows `x86_64-pc-windows-msvc`, `rustc 1.97.1 (8bab26f4f 2026-07-14)`, `cargo 1.97.1 (c980f4866 2026-06-30)`
- User token/integrity: standard interactive user, `Medium Mandatory Level`; the probe and helpers ran as the same normal user in SessionId `2`.
- Probe: PID `19416`, process `issue-178-verification-app`, responsive and alive during the fresh run.
- CUA diagnostic state: `apps: []`; only the Codex in-app browser was exposed. CUA was not used as the acceptance boundary.
- Selected window: HWND `0x4C02F6`, title `elwindui #178 Pointer Probe`, window rect `26,26–786,546`, client origin `34,57`, client size `744x481`, `GetDpiForWindow = 96`.
- Monitor: `\\.\DISPLAY49`, primary, bounds `0,0–1020,651`, working area `0,0–1020,611`, 32-bit color. WMI generic monitor entries previously reported `192` logical PPI, but no per-monitor mapping or second monitor was available.

## PID-based HWND discovery (D1)

The agent-local helper `.agent-state/issues/178/find-window.ps1 -ProcessId 19416` called `EnumWindows`, `GetWindowThreadProcessId`, `IsWindowVisible`, `IsWindowEnabled`, `GetWindowRect`, and `GetWindowTextW`. It recorded all four PID-matching candidates in `.agent-state/issues/178/results/window-discovery-fresh.json` and selected deterministically by exact expected title.

| HWND | title | visible | enabled | rect | root / owner | result |
|---|---|---:|---:|---|---|---|
| `0x4C02F6` | `elwindui #178 Pointer Probe` | yes | yes | `26,26–786,546` | root / none | selected by exact title |
| `0x9104C8` | executable path | yes | yes | `27,130–1020,649` | root / none | not selected |
| `0x9051A` | `MSCTFIME UI` | no | no | `0,0–0,0` | root / `0x2B40420` | ineligible |
| `0x2B40420` | `Default IME` | no | no | `0,0–0,0` | root / `0x4C02F6` | ineligible |

`Get-Process` reported `MainWindowHandle = 0x4C02F6` after the window was running. The earlier CUA-side `MainWindowHandle = 0` observation was therefore not treated as conclusive HWND absence.

## Direct OS input path

The agent-local `.agent-state/issues/178/send-input.ps1` used `BringWindowToTop`, `SetForegroundWindow`, `SetCursorPos`, and Win32 `SendInput`. The helper ran outside the agent sandbox as the normal user in SessionId `2`; no direct event handler, dispatcher, or probe callback was invoked.

The helper reported successful insertion for each requested mouse input, but the fresh run produced no pointer events. A pure sequential Win32 `SendInput` title-bar click also returned `2` inserted events and left the probe alive. This establishes the direct mouse-input delivery failure separately from CUA discovery.

Keyboard `SendInput` did work: typing `R178` produced four `native_textbox_change` records, and UI Automation read back TextBox value `R178`. The required mouse click-to-focus was not proven; no `native_textbox_focus` record was emitted.

## Probe and trace

The ignored public-API-only probe contains:

- one self-drawn target with a rendered glyph and blank interior;
- routed `pressed` / `moved` / `released` / `right_tapped` handlers;
- `root_to_screen` / `screen_to_root` calls;
- adjacent native Button and TextBox controls;
- bounded JSONL trace at `.agent-state/issues/178/results/trace.jsonl`.

Fresh trace after the direct-input matrix:

```text
metadata
native_textbox_change × 4
pointer events: 0
right_tapped events: 0
```

## Pointer matrix

All rows below were attempted through the selected HWND with direct Win32 input. Coordinates are screen pixels for this fresh window. `ENVIRONMENT_LIMITATION` means the input operation was sent but no corresponding native pointer/UI state was observed; it is not a product runtime failure claim.

| Case | OS input attempted | observed trace / state | result |
|---|---|---|---|
| P1 blank-area click | left-click `(146,279)` | no `pressed` / `released` | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| P2 glyph click | left-click `(186,124)` | no `pressed` / `released` | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| P3 captured drag outside | down `(186,124)`, 8 intermediate moves, up `(646,329)` | no `pressed` / `moved` / `released` | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| P4 post-release / native Button | move `(700,340)`, click `(80,419)` | no native Button event; no Core duplicate to assess | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| P5 native TextBox/TextArea | click `(70,419)`, type `R178` | 4 native change events; value `R178`; click-to-focus not observed | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| P6 right tap | right-click `(146,279)` | no `right_tapped`; no context-menu state | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |

The additional title-bar left-click `(729,40)` also left the responsive probe alive and unchanged. Issue #180 cancellation/capture-loss behavior was not intentionally exercised or claimed.

## Coordinate and topology matrix

No `P`, `S`, `R`, or `P'` values were collected because no pointer event reached the probe. No values are synthesized and the `≤1.0 DIP/axis` comparison is not evaluated.

| Scenario | monitor / bounds / DPI | P | S (`screen_position`) | R (`root_to_screen(P)`) | P' (`screen_to_root(S)`) | deltas | result |
|---|---|---|---|---|---|---|---|
| Primary | `DISPLAY49`, `0,0–1020,651`, window DPI `96` | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| Secondary | no active/selectable secondary display | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| Negative desktop coordinates | no active/selectable negative-coordinate monitor | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| Different DPI | no second monitor/per-monitor mapping; WMI generic entries `192` PPI | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |

## Automated build and test evidence

The repository-controlled verification gates remain green and were not rerun for this delta because no product files changed. Commands requiring native Windows metadata were run after `. .\tools\setup-vs-env.ps1`.

| Command | result | evidence |
|---|---|---|
| `cargo fmt --all -- --check` | PASS | clean |
| `rust-analyzer diagnostics .` | PASS | no error diagnostics |
| `cargo check -p elwindui-backend-winui3` | PASS | exit 0 |
| `cargo build -p controls-demo` | PASS | exit 0 |
| `cargo test -p elwindui-core pointer_dispatch_preserves_backend_screen_position_during_capture` | PASS | 1 passed |
| `cargo check --workspace` | PASS | exit 0 |
| `cargo build --workspace` | PASS | exit 0 |
| `cargo test -p elwindui-backend-winui3` | PASS | 42 passed, 0 failed |
| `cargo test --workspace` | PASS | all workspace tests/doctests passed |
| probe `cargo fmt --check` / `cargo check` | PASS | host-context compile succeeded |
| `git diff --check` | PASS | no whitespace errors |

The historical #207 49-error compile blocker is not present on the current master baseline and is retained only as history.

## Acceptance mapping and follow-up

Issue #178 acceptance: **SATISFIED**. Runtime behavior of transferred rows: **UNVERIFIED**. Follow-up owner: **Issue #224**.

| requirement | result | evidence / follow-up |
|---|---|---|
| Issue #178 acceptance criterion: recorded evidence or explicitly classified follow-up | SATISFIED | Every unresolved live row is classified `ENVIRONMENT_LIMITATION -> #224`; project-owner transfer is recorded on Issues #178 and #224 |
| PID-based HWND discovery and deterministic selection | PASS | four candidates recorded; exact-title HWND selected |
| real OS mouse P1-P6 | `ENVIRONMENT_LIMITATION -> #224` (accepted follow-up) | Win32 mouse input sent but not delivered; [#224](https://github.com/puchinya/elwindui/issues/224) is the sole owner |
| native keyboard input | partial evidence only | TextBox value `R178`; mouse focus step unverified; [#224](https://github.com/puchinya/elwindui/issues/224) |
| primary root/screen round trip | `ENVIRONMENT_LIMITATION -> #224` (accepted follow-up) | no pointer sample; [#224](https://github.com/puchinya/elwindui/issues/224) is the sole owner |
| secondary / negative / mixed-DPI topology | `ENVIRONMENT_LIMITATION -> #224` (accepted follow-up) | unavailable on active host; [#224](https://github.com/puchinya/elwindui/issues/224) is the sole owner |
| #180 cancellation/capture-loss separation | PASS (scope separation) | no cancellation behavior intentionally exercised |

Issue #224 is now mechanism-neutral: it requests an interactive Windows session where the running verification window can be identified and exercised with controlled real OS input. Its current scope includes the unresolved Win32 mouse-input delivery requirement and, if that is restored, the genuinely unavailable secondary/negative/mixed-DPI rows. CUA is diagnostic only.

## Workflow state

```text
Issue #178: phase:review; acceptance satisfied by measured evidence + classified follow-up #224
PR #208: ready to merge after final review
Issue #180: separate; not exercised
Issue #224: open; sole owner of transferred live-input/topology verification
implementation regression issues: none
```

No runtime `PASS` is claimed for the transferred rows. Issue #178 can complete because those rows have explicit evidence/classification and a dedicated follow-up owner (#224).
