[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateRange(1, 2147483647)]
    [int] $IssueNumber
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = (& git rev-parse --show-toplevel | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($repoRoot)) {
    throw 'Run this script inside a Git repository.'
}
Set-Location $repoRoot

$excludeFile = (& git rev-parse --git-path info/exclude | Out-String).Trim()
$excludeDirectory = Split-Path -Parent $excludeFile

if (-not (Test-Path -LiteralPath $excludeDirectory)) {
    New-Item -ItemType Directory -Path $excludeDirectory -Force | Out-Null
}
if (-not (Test-Path -LiteralPath $excludeFile)) {
    New-Item -ItemType File -Path $excludeFile -Force | Out-Null
}

if (-not (Select-String -LiteralPath $excludeFile -SimpleMatch '.agent-state/' -Quiet)) {
    Add-Content -LiteralPath $excludeFile -Value '.agent-state/'
}

$base = Join-Path $repoRoot ".agent-state/issues/$IssueNumber"
$screenshots = Join-Path $base 'screenshots'
$logs = Join-Path $base 'logs'

New-Item -ItemType Directory -Path $screenshots -Force | Out-Null
New-Item -ItemType Directory -Path $logs -Force | Out-Null

Write-Output $screenshots
Write-Output $logs
