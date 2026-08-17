# CLAUDE.md

Claude Code must follow the repository-wide rules in [`AGENTS.md`](AGENTS.md). This file is only the Claude Code entry point and does not redefine shared workflow, document authority, or product behavior.

## Communication

Ask all user questions in Japanese.

## Claude Code routing

1. Before entering Plan Mode, creating a plan or task list, or performing broad repository investigation, follow the Mandatory Task Bootstrap in [`AGENTS.md`](AGENTS.md). For a new repository-changing request with no associated Issue, this enters `phase:requirements` before normal planning.
2. For repository-changing work, after the bootstrap is complete, read the active Issue phase label and only the corresponding workflow document listed in [`AGENTS.md`](AGENTS.md).
3. Start document lookup at [`docs/README.md`](docs/README.md), then use the category README to choose the smallest relevant spec/design/status set.
4. Read only the relevant technical guide under [`docs/agents/`](docs/agents/).
5. Use `gh` for GitHub operations and `git` for local branch/commit/push operations, as required by [`AGENTS.md`](AGENTS.md).

Do not scan all specifications or designs, do not treat source code as the normative contract, and do not read `docs_only_human/` during ordinary implementation work.
