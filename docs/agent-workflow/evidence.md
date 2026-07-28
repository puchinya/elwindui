# Screenshots and logs

Read only when capturing, storing, or publishing screenshots or logs.

## Storage

Temporary local material:

```text
.agent-state/issues/<issue-number>/
  screenshots/
  logs/
```

Durable review evidence:

```text
docs/issues/<issue-number>-<slug>/evidence/
```

Test baseline images belong with the owning tests, not in Issue evidence.

## Rules

- Keep investigation screenshots and raw logs under `.agent-state/`.
- Commit only small evidence that is needed for review or future reference.
- Add a short `README.md` beside committed evidence with OS, scenario, commit,
  command, and result.
- Do not commit full build logs, repeated warnings, dumps, or large image sets.
- Put concise excerpts in the Issue or PR and use CI artifacts for large logs,
  videos, dumps, and image sets.
- Before cross-machine handoff, commit required small evidence or upload it as
  a named CI artifact.
- Never store secrets, tokens, private data, or unnecessary user-specific
  paths.
