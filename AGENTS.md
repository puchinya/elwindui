# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Communication

When asking the user a question (clarifying questions, `AskUserQuestion`, plan checkpoints, etc.), always ask in Japanese.

## Issue-driven development workflow

- Every repository-changing task must be associated with one GitHub Issue.
- Do not create an Issue for explanation, research, or exploratory discussion unless explicitly requested.
- Determine the active phase from Issue labels / PR state and read **only** the required workflow document:
  - `phase:requirements`: read [`docs/agent-workflow/requirements.md`](docs/agent-workflow/requirements.md)
  - `phase:design`: read [`docs/agent-workflow/design.md`](docs/agent-workflow/design.md)
  - `phase:ready` / `phase:implementation`: read [`docs/agent-workflow/implementation.md`](docs/agent-workflow/implementation.md)
  - `phase:review` / open PR: read [`docs/agent-workflow/review.md`](docs/agent-workflow/review.md)
- Pause / resume checkpoint rules: [`docs/agent-workflow/checkpoint.md`](docs/agent-workflow/checkpoint.md) (read only when pausing/resuming).
- Screenshot & log storage rules: [`docs/agent-workflow/evidence.md`](docs/agent-workflow/evidence.md) (read only when capturing evidence).

## Context Minimization & Document Routing

- **Primary document router**: Use [`docs/README.md`](docs/README.md) to locate relevant specifications, designs, status reports, and technical rules.
- **Code-first investigation**: Start task research from target source code and symbols to locate the affected implementation and understand its current state. This is an investigation strategy, not a source-of-truth rule for normative behavior. When desired behavior or public contracts matter, consult the relevant [`docs/specs/`](docs/specs/) section.
- **Do not read large documents in full**: Search [`docs/specs/`](docs/specs/), [`docs/design/`](docs/design/), and [`docs/status/`](docs/status/) using ripgrep (`rg`) first and read only the required section ranges.
- **Do not inspect sibling backends**: When working on one backend (e.g. AppKit), do not read sibling backend code (e.g. WinUI3) unless cross-backend parity or shared behavior changes are explicitly requested.
- **Human-only docs**: Do not load files in `docs_only_human/` during ordinary agent tasks.

## Technical Domain Agent Rules

When implementing or editing code, read only the relevant agent guide under [`docs/agents/`](docs/agents/):

- General Rust rules: [`docs/agents/common.md`](docs/agents/common.md)
- DSL / Codegen rules: [`docs/agents/codegen.md`](docs/agents/codegen.md)
- Class hierarchy & `#[class]`: [`docs/agents/class-model.md`](docs/agents/class-model.md)
- Backend architecture & layering: [`docs/agents/backend-common.md`](docs/agents/backend-common.md)
- AppKit (macOS): [`docs/agents/appkit.md`](docs/agents/appkit.md)
- WinUI 3 (Windows): [`docs/agents/winui3.md`](docs/agents/winui3.md)
- Testing & verification commands: [`docs/agents/testing.md`](docs/agents/testing.md)

## Document Authority

Different document classes answer different questions. Do not use one class as a substitute for another.

- **Normative behavior and public contracts**:
  [`docs/specs/`](docs/specs/) is authoritative. It defines what ElwindUI should do, including adopted specifications that may not yet be fully implemented.
- **Implementation architecture**:
  [`docs/design/`](docs/design/) defines how the normative specifications are intended to be implemented. Design documents must conform to `docs/specs/` and do not override them.
- **Current implementation status**:
  Source code is the actual current implementation. Tests provide executable evidence of implemented behavior. [`docs/status/`](docs/status/) summarizes implementation progress, backend support, known gaps, and verification state. A difference between source/status and a specification does not by itself mean the specification should be changed.
- **Agent rules and workflow**:
  [`docs/agents/`](docs/agents/) and [`docs/agent-workflow/`](docs/agent-workflow/) define how agents should perform work. They do not redefine product specifications.

When code, design, status, and specifications disagree:

1. Do not silently treat the current code as the desired behavior.
2. Determine whether the difference is:
   - an implementation gap,
   - stale design/status documentation, or
   - an intentionally approved specification change.
3. Only change a normative specification when the task explicitly requires or approves a specification change.
