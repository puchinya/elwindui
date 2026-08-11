# Local work checkpoint

Read only when resuming or pausing incomplete Issue work.

## Commands

Before pausing, record any incomplete spec/design/code/status synchronization and the next required document in the checkpoint. A resumed agent must not infer completion from code alone.

macOS/Linux:

```bash
scripts/agent/save-work-checkpoint.sh <issue-number>
scripts/agent/resume-work.sh <issue-number>
```

Windows PowerShell:

```powershell
.\scripts\agent\save-work-checkpoint.ps1 <issue-number>
.\scripts\agent\resume-work.ps1 <issue-number>
```

The checkpoint is stored at:

```text
.agent-state/issues/<issue-number>/checkpoint.md
```

Keep it short: objective, completed work, current state, one concrete next
action, checks, uncommitted files, and blockers. Do not store reasoning
transcripts, secrets, full logs, or unapproved requirements.

On resume, compare it with the Issue, PR, branch, HEAD, and worktree. Git and
GitHub override stale local state.

Local state is not shared between clones. Before switching machines, add one
concise `## Work checkpoint` Issue comment with branch, HEAD, completed work,
next action, verification summary, and blockers.

Delete the local Issue directory after merge and Issue closure.
