<#
.SYNOPSIS
    Deterministic contract tests for windows-ui-driver.ps1, run against tests/fake-winapp.ps1 --
    no real winapp install, no real GUI process. See docs/agents/winui3-e2e.md for the live-GUI
    smoke test (tests/theme-demo-e2e.ps1), which this script does not replace.

.NOTES
    No Pester dependency (contract requirement). Exits non-zero on any assertion failure.
#>

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$Driver = Join-Path $PSScriptRoot '..\windows-ui-driver.ps1'
$FakeBackend = Join-Path $PSScriptRoot 'fake-winapp.ps1'

$env:ELWINDUI_WINAPP_PATH = $FakeBackend

$script:FailureCount = 0

function Invoke-Driver {
    param([string[]]$DriverArgs)
    $output = & pwsh -NoProfile -File $Driver @DriverArgs 2>&1
    $exitCode = $LASTEXITCODE
    $stdoutLines = @($output | Where-Object { $_ -is [string] })
    $stdout = ($stdoutLines -join "`n")
    $json = $null
    try { $json = $stdout | ConvertFrom-Json -ErrorAction Stop } catch { $json = $null }
    return @{ ExitCode = $exitCode; StdOut = $stdout; Json = $json }
}

function Assert {
    param([bool]$Condition, [string]$Message)
    if ($Condition) {
        Write-Output "PASS: $Message"
    }
    else {
        Write-Output "FAIL: $Message"
        $script:FailureCount++
    }
}

function Assert-OneJsonObject {
    param($Result, [string]$Label)
    $lineCount = @($Result.StdOut -split "`n" | Where-Object { $_.Trim() -ne '' }).Count
    Assert ($lineCount -eq 1) "$Label -- stdout is exactly one line"
    Assert ($null -ne $Result.Json) "$Label -- stdout parses as one JSON object"
}

# 1) doctor records the fake backend version.
$r = Invoke-Driver @('doctor')
Assert-OneJsonObject $r 'doctor'
Assert ($r.Json.winapp_version -eq 'fake-winapp 0.0.0-test') 'doctor -- records the fake backend version verbatim'
Assert ($r.Json.success -eq $true) 'doctor -- success:true when the fake backend responds to --version'
Assert ($r.ExitCode -eq 0) 'doctor -- exit code 0 on success'

# 2) successful backend JSON remains parseable and normalized.
$r = Invoke-Driver @('search', '--pid', '999', '--query', 'FAKE_SUCCESS')
Assert-OneJsonObject $r 'search (success)'
Assert ($r.Json.success -eq $true) 'search (success) -- success:true'
Assert ($r.Json.backend.matchCount -eq 1) 'search (success) -- backend JSON body preserved (matchCount)'
Assert ($r.ExitCode -eq 0) 'search (success) -- exit code 0'

# 3) no_interactive_desktop becomes environment_blocker.
$r = Invoke-Driver @('search', '--pid', '999', '--query', 'FAKE_NO_INTERACTIVE_DESKTOP')
Assert-OneJsonObject $r 'search (no_interactive_desktop)'
Assert ($r.Json.success -eq $false) 'search (no_interactive_desktop) -- success:false'
Assert ($r.Json.category -eq 'environment_blocker') 'search (no_interactive_desktop) -- category:environment_blocker'
Assert ($r.ExitCode -eq 1) 'search (no_interactive_desktop) -- exit code 1'

# 4) foreground_not_target becomes environment_blocker.
$r = Invoke-Driver @('search', '--pid', '999', '--query', 'FAKE_FOREGROUND_NOT_TARGET')
Assert-OneJsonObject $r 'search (foreground_not_target)'
Assert ($r.Json.success -eq $false) 'search (foreground_not_target) -- success:false'
Assert ($r.Json.category -eq 'environment_blocker') 'search (foreground_not_target) -- category:environment_blocker'

# 5) unknown backend failure is not reclassified as a product failure (i.e. not silently mapped to
#    environment_blocker/tool_error/usage_error as if it were a recognized, specific condition).
$r = Invoke-Driver @('search', '--pid', '999', '--query', 'FAKE_UNKNOWN_ERROR')
Assert-OneJsonObject $r 'search (unknown error)'
Assert ($r.Json.success -eq $false) 'search (unknown error) -- success:false'
Assert ($r.Json.category -eq 'target_error') 'search (unknown error) -- unrecognized failure defaults to target_error, not a fabricated specific category'

# 6/7) diagnostics do not corrupt stdout JSON -- re-verified across every case above via
#      Assert-OneJsonObject, which fails if this script's own -Verbose/-Debug-style noise (or
#      windows-ui-driver.ps1's own) leaked onto stdout instead of stderr.

# 8) non-zero backend exit with valid JSON preserves the JSON body.
$r = Invoke-Driver @('search', '--pid', '999', '--query', 'FAKE_WAIT_TIMEOUT')
Assert-OneJsonObject $r 'search (wait/timeout-shaped 0-match result)'
Assert ($r.ExitCode -eq 1) 'search (0 matches) -- exit code 1 (matches winapp''s own documented contract)'
Assert ($null -ne $r.Json.backend) 'search (0 matches) -- backend JSON body preserved despite exit 1'
Assert ($r.Json.backend.matchCount -eq 0) 'search (0 matches) -- backend.matchCount is 0, not dropped'

# T2 -- missing/invalid backend executable.
$env:ELWINDUI_WINAPP_PATH = 'C:\does\not\exist-fake-winapp.exe'
$r = Invoke-Driver @('doctor')
Assert-OneJsonObject $r 'doctor (missing backend)'
Assert ($r.Json.success -eq $false) 'doctor (missing backend) -- success:false'
Assert ($r.Json.category -eq 'tool_error') 'doctor (missing backend) -- category:tool_error'
Assert ($r.Json.install_command -eq 'winget install Microsoft.winappcli --source winget') 'doctor (missing backend) -- exact WinGet install command present'
Assert ($r.ExitCode -eq 1) 'doctor (missing backend) -- exit code 1'

Remove-Item Env:ELWINDUI_WINAPP_PATH -ErrorAction SilentlyContinue

if ($script:FailureCount -gt 0) {
    Write-Output "`n$script:FailureCount assertion(s) failed."
    exit 1
}
Write-Output "`nAll driver-contract assertions passed."
exit 0
