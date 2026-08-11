# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Communication

When asking the user a question (clarifying questions, `AskUserQuestion`, plan checkpoints, etc.), always ask in Japanese.

## Repository Policy & Single Source of Truth

- Repository-wide workflow policies are defined in [`AGENTS.md`](AGENTS.md).
- Follow the Issue-driven workflow and phase routing outlined in `AGENTS.md`.

## Document Routing & Context Minimization

- **Primary document router**: Use [`docs/README.md`](docs/README.md) to locate relevant specifications, designs, status reports, and technical rules.
- **Code-first investigation**: Start task research from target source code and symbols to locate the affected implementation and understand its current state. This is an investigation strategy (see [`AGENTS.md`](AGENTS.md)), not a source-of-truth rule.
- **Do not read large documents in full**: Search [`docs/specs/`](docs/specs/), [`docs/design/`](docs/design/), and [`docs/status/`](docs/status/) using `rg` first and read only the relevant ranges.
- **Technical agent rules**: Read only the relevant domain rules under [`docs/agents/`](docs/agents/) (`common.md`, `codegen.md`, `class-model.md`, `backend-common.md`, `appkit.md`, `winui3.md`, `testing.md`).
- **Sibling backends**: Do not inspect sibling backend code unless cross-backend parity or shared behavior changes are explicitly requested.
