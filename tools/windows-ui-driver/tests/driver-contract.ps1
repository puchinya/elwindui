<#
.SYNOPSIS
    Deterministic contract tests for windows-ui-driver.ps1, run against tests/fake-winapp.ps1 --
    no real winapp install, no real GUI process. See docs/agents/winui3-e2e.md for the live-GUI
    tester procedure and tests/e2e/README.md for durable product E2E case ownership, neither of
    which this script replaces.

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

# T2b -- backend launches but `--version` itself fails (broken install, not a missing one).
$env:ELWINDUI_WINAPP_PATH = $FakeBackend
$env:ELWINDUI_FAKE_WINAPP_VERSION_FAIL = '1'
$r = Invoke-Driver @('doctor')
Assert-OneJsonObject $r 'doctor (broken version)'
Assert ($r.Json.success -eq $false) 'doctor (broken version) -- success:false'
Assert ($r.Json.category -eq 'tool_error') 'doctor (broken version) -- category:tool_error'
Assert ($r.Json.winapp_available -ne $true) 'doctor (broken version) -- winapp_available is not true'
Assert ($r.Json.backend_exit_code -eq 3) 'doctor (broken version) -- backend_exit_code preserved (3)'
Assert ($r.Json.backend_stderr -like '*simulated broken install*') 'doctor (broken version) -- backend_stderr preserved'
Assert ($r.ExitCode -eq 1) 'doctor (broken version) -- exit code 1'
Remove-Item Env:ELWINDUI_FAKE_WINAPP_VERSION_FAIL -ErrorAction SilentlyContinue

# T3 (large-stderr deadlock regression) -- Invoke-WinApp must drain the winapp backend's stdout
# and stderr concurrently. A fake backend that writes >= 256 KiB to stderr before exiting would
# deadlock a sequential stdout-then-stderr ReadToEnd() implementation (blocked filling stderr's
# OS pipe buffer while still waiting for stdout EOF). Uses a small bounded process helper, not
# Invoke-Driver's unbounded `&` call, so a regression here fails within a timeout instead of
# hanging the whole test suite.
$BoundedPwshPath = (Get-Process -Id $PID).Path
$largeStderrPsi = New-Object System.Diagnostics.ProcessStartInfo
$largeStderrPsi.FileName = $BoundedPwshPath
foreach ($a in @('-NoProfile', '-File', $Driver, 'search', '--pid', '999', '--query', 'FAKE_LARGE_STDERR')) {
    $largeStderrPsi.ArgumentList.Add($a)
}
$largeStderrPsi.UseShellExecute = $false
$largeStderrPsi.CreateNoWindow = $true
$largeStderrPsi.RedirectStandardOutput = $true
$largeStderrPsi.RedirectStandardError = $true

$largeStderrProc = [System.Diagnostics.Process]::Start($largeStderrPsi)
$largeStdoutTask = $largeStderrProc.StandardOutput.ReadToEndAsync()
$largeStderrTask = $largeStderrProc.StandardError.ReadToEndAsync()
$largeEofReached = $largeStdoutTask.Wait(15000)
Assert $largeEofReached 'T3 -- large-stderr regression: driver stdout reaches EOF within the bounded timeout'
if ($largeEofReached) {
    $largeStderrProc.WaitForExit(5000) | Out-Null
    $largeStdout = $largeStdoutTask.Result
    [void]$largeStderrTask.Result
    $largeJson = $null
    try { $largeJson = $largeStdout.Trim() | ConvertFrom-Json -ErrorAction Stop } catch { $largeJson = $null }
    Assert-OneJsonObject @{ StdOut = $largeStdout; Json = $largeJson } 'T3 -- large-stderr regression'
    Assert ($largeJson.success -eq $false) 'T3 -- large-stderr regression: success:false'
    Assert ($largeJson.category -eq 'target_error') 'T3 -- large-stderr regression: category:target_error (element_not_found token)'
    Assert ($largeJson.backend_exit_code -eq 1) 'T3 -- large-stderr regression: backend exit code preserved (1)'
    Assert ($largeJson.backend_stderr -like '*EEEE*') 'T3 -- large-stderr regression: full backend stderr preserved in the normalized result'
}
else {
    if (-not $largeStderrProc.HasExited) { Stop-Process -Id $largeStderrProc.Id -Force -ErrorAction SilentlyContinue }
}

# DPC-01 -- point-click selector mode routes through winapp's dedicated `ui click`, never a
# zero-distance `ui drag` substitute (Issue #236 delta contract Section 3.1/4.1).
$r = Invoke-Driver @('point-click', '--pid', '999', '--selector', 'FAKE_SELECTOR_CLICK')
Assert-OneJsonObject $r 'point-click (selector mode)'
Assert ($r.Json.success -eq $true) 'point-click (selector mode) -- success:true'
Assert ($r.Json.backend.verb -eq 'click') 'point-click (selector mode) -- routes through ui click, not ui drag'
Assert ($r.ExitCode -eq 0) 'point-click (selector mode) -- exit code 0'

# DPC-02 -- selector mode --button right passes --right through to `ui click`.
$r = Invoke-Driver @('point-click', '--pid', '999', '--selector', 'FAKE_SELECTOR_CLICK', '--button', 'right')
Assert-OneJsonObject $r 'point-click (selector mode, right button)'
Assert ($r.Json.success -eq $true) 'point-click (selector mode, right button) -- success:true'
Assert ($r.Json.backend.right -eq $true) 'point-click (selector mode, right button) -- backend received --right'

# DPC-03 -- coordinate mode is unchanged by adding selector mode: still routes through a
# zero-distance `ui drag` at the same point.
$r = Invoke-Driver @('point-click', '--pid', '999', '--x', '100', '--y', '200')
Assert-OneJsonObject $r 'point-click (coordinate mode)'
Assert ($r.Json.success -eq $true) 'point-click (coordinate mode) -- success:true'
Assert ($r.Json.backend.verb -eq 'drag') 'point-click (coordinate mode) -- still routes through ui drag'
Assert ($r.Json.point.x -eq 100 -and $r.Json.point.y -eq 200) 'point-click (coordinate mode) -- result preserves x/y'

# DPC-04 -- --selector combined with --x/--y fails closed as usage_error.
$r = Invoke-Driver @('point-click', '--pid', '999', '--selector', 'X', '--x', '1', '--y', '2')
Assert-OneJsonObject $r 'point-click (selector + coordinates)'
Assert ($r.Json.success -eq $false) 'point-click (selector + coordinates) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'point-click (selector + coordinates) -- category:usage_error'
Assert ($r.ExitCode -eq 1) 'point-click (selector + coordinates) -- exit code 1'

# DPC-05 -- an incomplete coordinate pair (only --x or only --y), with no --selector, fails closed.
$r = Invoke-Driver @('point-click', '--pid', '999', '--x', '1')
Assert-OneJsonObject $r 'point-click (missing --y)'
Assert ($r.Json.success -eq $false) 'point-click (missing --y) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'point-click (missing --y) -- category:usage_error'

$r = Invoke-Driver @('point-click', '--pid', '999', '--y', '2')
Assert-OneJsonObject $r 'point-click (missing --x)'
Assert ($r.Json.success -eq $false) 'point-click (missing --x) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'point-click (missing --x) -- category:usage_error'

# DPC-06 -- neither --selector nor a coordinate pair fails closed rather than defaulting silently.
$r = Invoke-Driver @('point-click', '--pid', '999')
Assert-OneJsonObject $r 'point-click (no target)'
Assert ($r.Json.success -eq $false) 'point-click (no target) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'point-click (no target) -- category:usage_error'

Remove-Item Env:ELWINDUI_WINAPP_PATH -ErrorAction SilentlyContinue

# T9 -- launch --arg list semantics regression: repeated occurrences preserved in order, and a
# dash-prefixed application argument is passed through rather than misread as a driver flag.
# `launch` never calls winapp, so these do not need ELWINDUI_WINAPP_PATH; they launch pwsh.exe
# itself against tests/fake-app.ps1, which records the argv it actually received.
$FakeApp = Join-Path $PSScriptRoot 'fake-app.ps1'
$PwshPath = (Get-Process -Id $PID).Path
$ArgvOut = Join-Path ([System.IO.Path]::GetTempPath()) ("elwindui-launch-argv-{0}.json" -f ([guid]::NewGuid()))
try {
    $r = Invoke-Driver @(
        'launch', '--path', $PwshPath,
        '--arg', '-NoProfile', '--arg', '-File', '--arg', $FakeApp,
        '--arg', $ArgvOut,
        '--arg', 'one', '--arg', 'two', '--arg', '--some-app-option'
    )
    Assert-OneJsonObject $r 'launch (--arg list)'
    Assert ($r.Json.success -eq $true) 'launch (--arg list) -- success:true'

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $ArgvOut) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    Assert (Test-Path -LiteralPath $ArgvOut) 'launch (--arg list) -- fake-app.ps1 wrote its recorded argv'
    if (Test-Path -LiteralPath $ArgvOut) {
        $recorded = (Get-Content -LiteralPath $ArgvOut -Raw | ConvertFrom-Json).args
        Assert (($recorded -join '|') -eq 'one|two|--some-app-option') 'T9 -- repeated --arg values and a dash-prefixed app arg are preserved, in order, verbatim'
    }
}
finally {
    Remove-Item -LiteralPath $ArgvOut -ErrorAction SilentlyContinue
}

# T3 -- malformed trailing --arg (no value at all) must fail closed as usage_error, not be
# silently dropped and not launch any process.
$r = Invoke-Driver @('launch', '--path', $PwshPath, '--arg')
Assert-OneJsonObject $r 'launch (trailing --arg)'
Assert ($r.Json.success -eq $false) 'launch (trailing --arg) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'launch (trailing --arg) -- category:usage_error'
Assert ($r.ExitCode -eq 1) 'launch (trailing --arg) -- exit code 1'

# T4 -- a required option (--path) whose apparent value is actually the next driver flag must
# fail closed as usage_error, not silently become the boolean true and attempt to launch "True".
$r = Invoke-Driver @('launch', '--path', '--wait-window-timeout', '10')
Assert-OneJsonObject $r 'launch (--path swallowed by next flag)'
Assert ($r.Json.success -eq $false) 'launch (--path swallowed by next flag) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'launch (--path swallowed by next flag) -- category:usage_error'
Assert ($r.ExitCode -eq 1) 'launch (--path swallowed by next flag) -- exit code 1'

# T5/T6 -- child stdio ownership regression: a caller capturing the (nested) driver's own stdout
# must see EOF as soon as the driver itself exits, never held open by a long-lived launched child,
# and the child's own stdout/stderr must never leak into the driver's captured JSON stdout. This
# reproduces the original nested-caller hang (theme-demo-e2e.ps1 -> windows-ui-driver.ps1 launch
# -> a long-lived GUI process) with a fake child standing in for the GUI process.
$NestedArgvOut = Join-Path ([System.IO.Path]::GetTempPath()) ("elwindui-nested-argv-{0}.json" -f ([guid]::NewGuid()))
$childPid = $null
try {
    $childArgs = @('-NoProfile', '-File', $FakeApp, '-HoldSeconds', '5', '-WriteStdoutMarker', '-WriteStderrMarker', $NestedArgvOut)
    $nestedDriverArgs = New-Object System.Collections.Generic.List[string]
    $nestedDriverArgs.Add('-NoProfile'); $nestedDriverArgs.Add('-File'); $nestedDriverArgs.Add($Driver)
    $nestedDriverArgs.Add('launch'); $nestedDriverArgs.Add('--path'); $nestedDriverArgs.Add($PwshPath)
    foreach ($a in $childArgs) { $nestedDriverArgs.Add('--arg'); $nestedDriverArgs.Add($a) }

    $nestedPsi = New-Object System.Diagnostics.ProcessStartInfo
    $nestedPsi.FileName = $PwshPath
    foreach ($a in $nestedDriverArgs) { $nestedPsi.ArgumentList.Add($a) }
    $nestedPsi.UseShellExecute = $false
    $nestedPsi.CreateNoWindow = $true
    $nestedPsi.RedirectStandardOutput = $true
    $nestedPsi.RedirectStandardError = $true

    $nestedProc = [System.Diagnostics.Process]::Start($nestedPsi)
    $stdoutTask = $nestedProc.StandardOutput.ReadToEndAsync()
    $stderrTask = $nestedProc.StandardError.ReadToEndAsync()
    # Bounded timeout: the nested driver itself must exit and its stdout must reach EOF in well
    # under this, regardless of the 5s-held child -- a regression here means the caller is blocked
    # on the child again.
    $eofReached = $stdoutTask.Wait(15000)
    Assert $eofReached 'T5 -- nested driver stdout reaches EOF within the bounded timeout, independent of the long-lived child'

    if ($eofReached) {
        $nestedProc.WaitForExit(5000) | Out-Null
        $nestedStdout = $stdoutTask.Result
        $nestedStderr = $stderrTask.Result
        $nestedJson = $null
        try { $nestedJson = $nestedStdout.Trim() | ConvertFrom-Json -ErrorAction Stop } catch { $nestedJson = $null }
        Assert ($null -ne $nestedJson) 'T5 -- nested driver stdout parses as one JSON object'
        Assert ($nestedJson.success -eq $true) 'T5 -- nested driver launch success:true'
        if ($null -ne $nestedJson) { $childPid = [int]$nestedJson.pid }

        Assert ($null -ne $childPid -and (Get-Process -Id $childPid -ErrorAction SilentlyContinue)) 'T5 -- the long-lived child is still alive when the nested driver''s stdout reaches EOF'

        Assert (-not ($nestedStdout -match 'FAKE_APP_STDOUT_MARKER')) 'T6 -- the child''s own stdout marker does not leak into the driver''s captured stdout JSON'
        Assert (-not ($nestedStderr -match 'FAKE_APP_STDERR_MARKER')) 'T6 -- the child''s own stderr marker does not leak into the driver''s captured stderr'
    }
}
finally {
    if ($childPid) { Stop-Process -Id $childPid -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $NestedArgvOut -ErrorAction SilentlyContinue
}

if ($script:FailureCount -gt 0) {
    Write-Output "`n$script:FailureCount assertion(s) failed."
    exit 1
}
Write-Output "`nAll driver-contract assertions passed."
exit 0
