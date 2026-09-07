<#
.SYNOPSIS
    Fake launch target for tools/windows-ui-driver/tests/driver-contract.ps1's `launch --arg`
    coverage. Not a GUI app; it only records the argv it actually received (order and content) so
    the test can verify windows-ui-driver.ps1's own `--arg` handling without a real executable.

.DESCRIPTION
    Point windows-ui-driver.ps1 `launch --path <pwsh.exe>` at this file via
    `--arg -NoProfile --arg -File --arg <this-file>`, followed by the first positional argument
    (an output file path) and any remaining arguments to record.
#>

param(
    [Parameter(Position = 0, Mandatory = $true)]
    [string]$OutFile,

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$RecordedArgs
)

$payload = @{ args = @($RecordedArgs) } | ConvertTo-Json -Compress
[System.IO.File]::WriteAllText($OutFile, $payload, [System.Text.UTF8Encoding]::new($false))
