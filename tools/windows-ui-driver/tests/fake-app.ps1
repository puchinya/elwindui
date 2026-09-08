<#
.SYNOPSIS
    Fake launch target for tools/windows-ui-driver/tests/driver-contract.ps1's `launch --arg`
    coverage and its child-stdio-ownership regression. Not a GUI app.

.DESCRIPTION
    Point windows-ui-driver.ps1 `launch --path <pwsh.exe>` at this file via
    `--arg -NoProfile --arg -File --arg <this-file>`, followed by any of this script's own named
    switches (also passed through `--arg`, since `--arg`'s value is taken verbatim even when it
    starts with `-`/`--`), then the first positional argument (an output file path), then any
    remaining arguments to record.

    With no switches, it records argv and exits immediately (the original `launch --arg` case).
    With `-HoldSeconds`, it stays alive for that many seconds before exiting, optionally writing a
    short marker to stdout/stderr first -- used to prove the driver's own stdout reaches EOF
    independent of this long-lived child's lifetime, and that the child's own output never leaks
    into the driver's captured JSON stdout.
#>

param(
    [double]$HoldSeconds = 0,

    [switch]$WriteStdoutMarker,

    [switch]$WriteStderrMarker,

    [Parameter(Position = 0, Mandatory = $true)]
    [string]$OutFile,

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$RecordedArgs
)

$payload = @{ args = @($RecordedArgs) } | ConvertTo-Json -Compress
[System.IO.File]::WriteAllText($OutFile, $payload, [System.Text.UTF8Encoding]::new($false))

if ($WriteStdoutMarker) { Write-Output 'FAKE_APP_STDOUT_MARKER' }
if ($WriteStderrMarker) { [Console]::Error.WriteLine('FAKE_APP_STDERR_MARKER') }
if ($HoldSeconds -gt 0) { Start-Sleep -Seconds $HoldSeconds }
