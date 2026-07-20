# Windows agent instructions

## Build environment

Before running commands that require MSVC or the Windows SDK, import the Visual Studio build environment into the current PowerShell session:

```powershell
. .\tools\setup-vs-env.ps1
```

Run subsequent build, check, and test commands in the same PowerShell session.

## Command execution

Keep Windows commands short and execute one logical operation per command.

Do not combine source reading, generated-file discovery, searching, and formatting into one long PowerShell command.

Avoid repeatedly reading the same source range or searching the same generated file. Reuse previously retrieved output and resolved paths during the current task.

When searching generated bindings, prefer bounded fixed-string searches:

```powershell
rg -F -n -m 1 -A 13 "<pattern>" <file>
```

Use ripgrep's `-m` option to stop searching. Do not rely on `Select-Object -First` to limit the underlying search.

Do not assume Python is inherently faster than PowerShell. If a simple read or search command is slow, the command execution host may be the bottleneck.

If a command stalls:

1. Cancel it.
2. Retry once using a different direct method.
3. Do not repeatedly rewrite and rerun equivalent commands.
4. Continue using available context where possible.
5. Do not wait for a tool to recover or leave the requested implementation unfinished without a concrete external blocker.
