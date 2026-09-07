# Local work checkpoint

Read only when pausing or resuming incomplete Issue work.

## Commands

macOS/Linux:

```bash
scripts/agent/save-work-checkpoint.sh <issue-number>
scripts/agent/resume-work.sh <issue-number>
scripts/agent/agent-context.sh <issue-number>
```

Windows PowerShell:

```powershell
.\scripts\agent\save-work-checkpoint.ps1 <issue-number>
.\scripts\agent\resume-work.ps1 <issue-number>
.\scripts\agent\agent-context.ps1 <issue-number>
```

The checkpoint is:

```text
.agent-state/issues/<issue-number>/checkpoint.md
```

A supplied Implementation Contract, when present, is separately preserved exactly as:

```text
.agent-state/issues/<issue-number>/implementation-contract.md
.agent-state/issues/<issue-number>/implementation-contract.sha256
```

Do not copy the contract body into the checkpoint.

Keep the checkpoint short: objective, completed work, current state, one concrete next action, checks, uncommitted files, and blockers. Do not store reasoning transcripts, secrets, full logs, or unapproved requirements.

## Resume rule

On resume:

1. run the resume helper and compact `agent-context` helper;
2. compare local state with Issue, PR, branch, HEAD, worktree, and contract integrity;
3. Git/GitHub override stale checkpoint state;
4. if a contract mirror exists and is valid, re-read the exact contract before continuing material implementation/review decisions;
5. if `contract_status=invalid`, stop contract-dependent work and resolve the integrity conflict instead of regenerating/overwriting it silently.

`reviewer-checklist.md` and `self-review.md` are local derived workflow state, not requirements/design authority. After resume, do not trust a prior `PASS` blindly: rerun `prepare-self-review.*` and the final `validate-self-review.*` against current sources and HEAD. Do not copy the full checklist or result entries into checkpoint comments.

Local state is not shared between clones. Before changing machines, add one concise `## Work checkpoint` Issue comment with branch, HEAD, completed work, next action, verification summary, and blockers. Do not paste the full contract.

Delete the local Issue directory after merge and Issue closure.
