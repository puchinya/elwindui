# Issue #282 DNP-01 Windows session summary

Related: Issue [#282](https://github.com/puchinya/elwindui/issues/282), source Issue
[#279](https://github.com/puchinya/elwindui/issues/279) / PR
[#281](https://github.com/puchinya/elwindui/pull/281), AppKit
[#280](https://github.com/puchinya/elwindui/issues/280) (not in scope). Index and fixed inputs:
[`README.md`](README.md).

## Primary aggregate — master `a445183524e4f688273a50b8fcccd61dbd87bdbe`

Run directories are under `.agent-state/issues/282/e2e/a445183524e4/`.

| Session | Run id | Outcome | Clicks sent / PASS | First anomaly | Cleanup forced |
|---|---|---|---|---|---|
| 1 (attempt 1) | `20261017T035845Z-s01-a1` | NOT RUN | 0 / 0 | reset not possible: `Reset layout` hidden behind `Tools` on master | false |
| 1 (attempt 2) | `20261017T035939Z-s01-a2` | NOT RUN | 0 / 0 | same, before the procedure was adapted | false |
| 1 (attempt 3) | `20261017T040653Z-s01-a3` | PASS | 20 / 20 | none | false |
| 2 | `20261017T041829Z-s02-a1` | PASS | 20 / 20 | none | false |
| 3 | `20261017T043601Z-s03-a1` | PASS | 20 / 20 | none | false |
| 4 | `20261017T044642Z-s04-a1` | PASS | 20 / 20 | none | false |
| 5 | `20261017T045725Z-s05-a1` | PASS | 20 / 20 | none | false |
| 6 | `20261017T050807Z-s06-a1` | PASS | 20 / 20 | none | false |
| 7 | `20261017T051849Z-s07-a1` | PASS | 20 / 20 | none | false |
| 8 | `20261017T052933Z-s08-a1` | PASS | 20 / 20 | none | false |
| 9 | `20261017T054033Z-s09-a1` | PASS | 20 / 20 | none | false |
| 10 | `20261017T055116Z-s10-a1` | PASS | 20 / 20 | none | false |

- Planned 10, completed 10, PASS 10, FAIL 0, BLOCKED 0. Session 1 had two NOT RUN attempts
  (procedure conflict with the redesigned master demo, no clicks sent) before the
  maintainer-approved Reset adaptation.
- Every click: one window, same HWND, window geometry unchanged before and after, before and
  after captures both 960x640, target point at least 8 px inside the tab header read from the
  before image, `point-click` `success:true` with backend exit 0. After `point-click` returned,
  the after capture started within 7–15 ms. Both the selected tab and the visible body
  (`Document A editor` / `Document B editor`) matched the target. The `Active item` line was
  not used as evidence.
- Window position varied between sessions (52–182 px offsets), so the target coordinates were
  re-derived each time.
- Classification: **`not_reproduced_in_scope`**.

## Separate set — old fixed HEAD `0366fc8271289801dd109c6431a00296a13154ab`

Run directories are under `.agent-state/issues/282/e2e/0366fc827128/`. These runs are not
counted in the primary aggregate.

| Session | Run id | Outcome | Clicks sent / PASS | First anomaly |
|---|---|---|---|---|
| 1 | `20261017T032452Z-s01-a1` | PASS | 20 / 20 | none |
| 2 | `20261017T033556Z-s02-a1` | PASS | 20 / 20 | none |
| 3 | `20261017T034621Z-s03-a1` | NOT RUN | 1 / 1 | before click 2 |

S3 detail:
- Click 1 (target B) was PASS: the after capture showed Document B selected with
  `Document B editor`.
- In the before capture for click 2, about 13 s later, with no input from the harness and the
  same HWND and geometry, the vertical document/tool splitter had moved about 112 px left. Every
  panel body was blank (document, Solution Explorer, Error List, Output), while the Document B
  header was still drawn as selected.
- Click 2 was not sent.
- Layer: **`unclassified`**. A second `docking-demo` process, not launched by the harness, was
  found running on the same desktop about a minute later. Whether there was manual desktop/RDP
  interaction during S3 has not been confirmed.

## Historical #279 FAIL (reference only, unchanged)

The #279 run on `7b33b4d6625d36d293d3f7efba56cc3e01f667d1` failed at click 18: target
Document A, but Document B stayed selected. Its raw evidence remains under
`.agent-state/issues/279/e2e/7b33b4d6625d/20260926T105357Z/`. The after-click-18 capture was
960x567 instead of 960x640, with the Windows taskbar visible over the window bottom, which is
consistent with a display/desktop change during that click. No independent proof of input
delivery exists, so it stays **`unclassified`**. It is a different commit from both sets above.

## Follow-ups

None created. No product, target-geometry, or driver defect was established with enough evidence
for a separate remediation Issue (contract AD-01 / §6). If the S3-type spontaneous layout change
or a click-18-type mismatch recurs, open a new Issue with that run's evidence.
