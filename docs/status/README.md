# Status index

`docs/status` contains concise **current** implementation, backend support, known gaps, and verification state. It is not a requirements/design authority and not a PR/evidence history archive.

| Question | Status |
|---|---|
| Cross-cutting features and major gaps | `implementation_status.md` |
| Backend support and current verification | `backend_status.md` |
| Control implementation matrix | `control_status.md` |
| Custom controls prerequisite/state | `custom_controls_status.md` |
| DockingControl/layout model state | `docking_status.md` |
| Codegen/LSP/preview/hot-reload/GUI-driver state | `tooling_status.md` |

Desired behavior is in `../specs/README.md`; durable architecture is in `../design/README.md`.

## Compact status schema

Keep only what is needed to answer:

- what exists now;
- what is supported now;
- what is missing/blocked now;
- what verification class/state is established now;
- where the latest authoritative Issue/PR/evidence is located when one concise link is useful.

Do not retain PR-by-PR chronology, review-remediation narratives, historical diagnostic/test counts, old SHAs, raw command output, investigation transcripts, or architecture/specification explanations already owned elsewhere.

An active unresolved Issue link, concise latest evidence link, or concise current environment limitation is allowed when it still matters to the current state.

When historical evidence and a current fact are mixed in one paragraph, preserve the current fact and remove the history around it. Do not mechanically delete Issue/PR references without checking whether they identify an active gap.

Status must never redefine desired behavior or durable architecture.
