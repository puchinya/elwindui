# Windows Agent Instructions

> **Note**: These instructions are incorporated into [`docs/agents/winui3.md`](winui3.md). Refer to `winui3.md` for full WinUI3 backend and Windows guidelines.

## Build environment

Before running commands that require MSVC or the Windows SDK, import the Visual Studio build environment into the current PowerShell session:

```powershell
. .\tools\setup-vs-env.ps1
```

Run subsequent build, check, and test commands in the same PowerShell session.

## Command execution

Keep Windows commands short and execute one logical operation per command.
Avoid combining source reading, searching, and formatting into long PowerShell commands.
Use `rg -F -n -m 1 -A 13 "<pattern>" <file>` with `-m` to limit search bounds.
