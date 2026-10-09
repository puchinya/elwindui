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

# 2a) duration-controlled mouse drags are passed through to the external backend verbatim.
$r = Invoke-Driver @('drag', '--hwnd', '4660', '--from-x', '100', '--from-y', '100', '--to-x', '300', '--to-y', '300', '--duration-ms', '4000')
Assert-OneJsonObject $r 'drag (duration pass-through)'
Assert ($r.Json.success -eq $true) 'drag (duration pass-through) -- success:true'
Assert (($r.Json.backend.receivedArgs -contains '--duration-ms') -and ($r.Json.backend.receivedArgs -contains '4000')) 'drag (duration pass-through) -- exact duration flag/value forwarded'
Assert (($r.Json.backend.receivedArgs -join ' ') -match '--duration-ms 4000') 'drag (duration pass-through) -- duration flag/value are adjacent'

# 2b) omitted duration remains backward compatible and does not add a backend duration flag.
$r = Invoke-Driver @('drag', '--hwnd', '4660', '--from-x', '100', '--from-y', '100', '--to-x', '300', '--to-y', '300')
Assert-OneJsonObject $r 'drag (duration omitted)'
Assert (-not ($r.Json.backend.receivedArgs -contains '--duration-ms')) 'drag (duration omitted) -- no duration flag forwarded'

# 2c) invalid durations fail before the external backend is invoked.
foreach ($invalidDuration in @('0', '-1', '60001', 'not-an-integer')) {
    $callLog = Join-Path $env:TEMP ("elwindui-driver-duration-{0}.log" -f [guid]::NewGuid())
    $env:ELWINDUI_FAKE_WINAPP_CALL_LOG = $callLog
    $r = Invoke-Driver @('drag', '--hwnd', '4660', '--from-x', '100', '--from-y', '100', '--to-x', '300', '--to-y', '300', '--duration-ms', $invalidDuration)
    Assert-OneJsonObject $r "drag (invalid duration $invalidDuration)"
    Assert ($r.ExitCode -eq 1) "drag (invalid duration $invalidDuration) -- exit code 1"
    Assert ($r.Json.category -eq 'usage_error') "drag (invalid duration $invalidDuration) -- category:usage_error"
    Assert (-not (Test-Path -LiteralPath $callLog)) "drag (invalid duration $invalidDuration) -- backend not invoked"
    Remove-Item Env:ELWINDUI_FAKE_WINAPP_CALL_LOG -ErrorAction SilentlyContinue
}

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

# T2c -- set-value forwards the selector, one value token containing spaces, HWND, and JSON
# request flag without a send-keys fallback.
$r = Invoke-Driver @(
    'set-value', '--hwnd', '4660', '--selector', 'FAKE_SET_VALUE_SELECTOR',
    '--value', 'value with spaces'
)
Assert-OneJsonObject $r 'set-value (success)'
Assert ($r.Json.success -eq $true) 'set-value (success) -- success:true'
Assert ($r.Json.backend.forwarded -eq $true) 'set-value (success) -- selector/value/HWND forwarded as exact backend arguments'
Assert ($r.ExitCode -eq 0) 'set-value (success) -- exit code 0'

# T2d -- required set-value arguments fail closed before the backend is invoked.
$r = Invoke-Driver @('set-value', '--hwnd', '4660', '--value', 'value')
Assert-OneJsonObject $r 'set-value (missing selector)'
Assert ($r.Json.success -eq $false) 'set-value (missing selector) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'set-value (missing selector) -- category:usage_error'
$r = Invoke-Driver @('set-value', '--hwnd', '4660', '--selector', 'FAKE_SET_VALUE_SELECTOR')
Assert-OneJsonObject $r 'set-value (missing value)'
Assert ($r.Json.success -eq $false) 'set-value (missing value) -- success:false'
Assert ($r.Json.category -eq 'usage_error') 'set-value (missing value) -- category:usage_error'

# T2e -- a backend failure is normalized through the same UIA error taxonomy as other patterns.
$r = Invoke-Driver @('set-value', '--hwnd', '4660', '--selector', 'FAKE_SET_VALUE_BACKEND_ERROR', '--value', 'value')
Assert-OneJsonObject $r 'set-value (backend error)'
Assert ($r.Json.success -eq $false) 'set-value (backend error) -- success:false'
Assert ($r.Json.category -eq 'target_error') 'set-value (backend error) -- category:target_error'
Assert ($r.ExitCode -eq 1) 'set-value (backend error) -- exit code 1'

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

# T10 -- touch-cancel is one of two bounded direct-injection exceptions (see T11). Usage validation must fail
# before any HWND/session/injection work, and every invocation must retain the driver's one-object
# JSON protocol even when the native API is unavailable on the test host.
$r = Invoke-Driver @('touch-cancel', '--from-x', '10', '--from-y', '20')
Assert-OneJsonObject $r 'touch-cancel (missing hwnd)'
Assert ($r.Json.category -eq 'usage_error') 'touch-cancel (missing hwnd) -- category:usage_error'

$r = Invoke-Driver @('touch-cancel', '--hwnd', '4660', '--from-y', '20')
Assert-OneJsonObject $r 'touch-cancel (missing from-x)'
Assert ($r.Json.category -eq 'usage_error') 'touch-cancel (missing from-x) -- category:usage_error'

$r = Invoke-Driver @('touch-cancel', '--hwnd', '4660', '--from-x', '10', '--from-y', '20', '--to-x', '30')
Assert-OneJsonObject $r 'touch-cancel (incomplete destination pair)'
Assert ($r.Json.category -eq 'usage_error') 'touch-cancel (incomplete destination pair) -- category:usage_error'

$r = Invoke-Driver @('touch-cancel', '--hwnd', '4660', '--from-x', '10', '--from-y', '20', '--hold-ms', '-1')
Assert-OneJsonObject $r 'touch-cancel (negative hold)'
Assert ($r.Json.category -eq 'usage_error') 'touch-cancel (negative hold) -- category:usage_error'

$r = Invoke-Driver @('touch-cancel', '--hwnd', '4660', '--from-x', '10', '--from-y', '20', '--hold-ms', '2001')
Assert-OneJsonObject $r 'touch-cancel (hold above bound)'
Assert ($r.Json.category -eq 'usage_error') 'touch-cancel (hold above bound) -- category:usage_error'

$r = Invoke-Driver @('touch-cancel', '--hwnd', '4660', '--from-x', 'not-a-coordinate', '--from-y', '20')
Assert-OneJsonObject $r 'touch-cancel (malformed coordinate)'
Assert ($r.Json.category -eq 'usage_error') 'touch-cancel (malformed coordinate) -- category:usage_error'

# T11 -- deterministic private backend/classification seam. This does not inject input or add a
# user-facing command; it exercises the pure selection/error taxonomy without requiring a GUI.
$env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'classification'
$r = Invoke-Driver @('doctor')
Assert-OneJsonObject $r 'touch backend classification probe'
Assert ($r.Json.error_87 -eq 'tool_error') 'touch backend classification -- ERROR_INVALID_PARAMETER 87 is tool_error'
Assert ($r.Json.error_50 -eq 'environment_blocker') 'touch backend classification -- ERROR_NOT_SUPPORTED 50 is environment_blocker'
Assert ($r.Json.error_120 -eq 'environment_blocker') 'touch backend classification -- ERROR_CALL_NOT_IMPLEMENTED 120 is environment_blocker'
Assert ($null -eq $r.Json.digitizer -and $null -eq $r.Json.maximum_touches) 'touch backend classification -- physical touch metrics are not capability output'
Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue

$env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'backend-selection'
$r = Invoke-Driver @('doctor')
Assert-OneJsonObject $r 'touch backend selection probe'
Assert ($r.Json.modern_available -eq 'synthetic-pointer') 'touch backend selection -- modern API selects synthetic-pointer'
Assert ($r.Json.modern_entry_point_unavailable -eq 'legacy-touch') 'touch backend selection -- unavailable modern API selects legacy-touch fallback'
Assert ($r.Json.modern_unsupported -eq 'legacy-touch') 'touch backend selection -- unsupported modern API selects legacy-touch fallback'
Assert ($r.Json.modern_invalid_parameter -eq 'tool_error') 'touch backend selection -- ERROR_INVALID_PARAMETER does not fallback'
Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue

# The native lifecycle is intentionally source-inspected here: a real GUI is required to invoke
# the API, but cleanup must remain structurally guaranteed in a finally block and after DOWN
# failures. The live matrix is the evidence for actual delivery, not this deterministic check.
$driverSource = Get-Content -LiteralPath $Driver -Raw
Assert ($driverSource -match 'DestroySyntheticPointerDevice\(\$device\)') 'touch backend lifecycle -- synthetic device destruction is present'
Assert ($driverSource -match '(?s)finally\s*\{.*DestroySyntheticPointerDevice') 'touch backend lifecycle -- destruction is in finally cleanup'
Assert ($driverSource -match 'cleanup_attempted') 'touch backend lifecycle -- best-effort canceled cleanup is reported after post-DOWN failure'
Assert (-not ($driverSource -match 'SM_DIGITIZER|SM_MAXIMUMTOUCHES')) 'touch backend classification -- no physical digitizer capability gate remains'

# T12 -- deterministic synthetic-device destruction ABI/cleanup seam. DestroySyntheticPointerDevice
# returns VOID, so normal managed completion is the only cleanup-success signal and no Win32
# destroy result may be fabricated.
Assert ($driverSource -match 'public static extern void DestroySyntheticPointerDevice\(IntPtr device\)') 'touch backend ABI -- DestroySyntheticPointerDevice is void-compatible'
Assert (-not ($driverSource -match 'bool DestroySyntheticPointerDevice')) 'touch backend ABI -- no bool destroy return remains'
Assert (-not ($driverSource -match 'DestroySyntheticPointerDevice.*GetLastWin32Error')) 'touch backend ABI -- destroy does not consume GetLastWin32Error'

$env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'cleanup'
$r = Invoke-Driver @('doctor')
Assert-OneJsonObject $r 'touch backend cleanup probe'
Assert ($r.Json.normal_success -eq $true) 'touch backend cleanup -- normal destroy leaves sequence success'
Assert ($r.Json.normal_cleanup_device_destroy_success -eq $true) 'touch backend cleanup -- normal destroy records cleanup success'
Assert ($r.Json.success_exception_success -eq $false) 'touch backend cleanup -- destroy exception after success fails the result'
Assert ($r.Json.success_exception_category -eq 'tool_error') 'touch backend cleanup -- destroy exception after success is tool_error'
Assert ($r.Json.success_exception_cleanup_device_destroy_success -eq $false) 'touch backend cleanup -- destroy exception records cleanup failure'
Assert ($r.Json.success_exception_has_error_code -eq $false) 'touch backend cleanup -- no fabricated native error code after destroy exception'
Assert ($r.Json.success_exception_has_cleanup_error -eq $true) 'touch backend cleanup -- managed destroy exception is recorded'
Assert ($r.Json.failure_exception_category -eq 'environment_blocker') 'touch backend cleanup -- original sequence category remains authoritative'
Assert ($r.Json.failure_exception_error -eq 'injection failed') 'touch backend cleanup -- original sequence error remains authoritative'
Assert ($r.Json.failure_exception_error_code -eq 50) 'touch backend cleanup -- original sequence error code remains authoritative'
Assert ($r.Json.failure_exception_cleanup_device_destroy_success -eq $false) 'touch backend cleanup -- failed sequence records cleanup failure'
Assert ($r.Json.failure_exception_has_cleanup_error -eq $true) 'touch backend cleanup -- failed sequence records managed destroy exception'
Assert ($r.Json.failure_exception_has_cleanup_error_code -eq $false) 'touch backend cleanup -- failed sequence has no fabricated destroy error code'
Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue

# T11 -- capture-sequence is the second bounded in-process exception (timed click/capture/UIA steps). Usage
# and target validation must fail closed before any input, capture, or output directory creation.
$SeqDir = Join-Path ([System.IO.Path]::GetTempPath()) ("seq-contract-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $SeqDir | Out-Null
try {
    $okSteps = Join-Path $SeqDir 'ok.json'
    '[{"op":"sleep","ms":1}]' | Set-Content -LiteralPath $okSteps -Encoding utf8
    $badOp = Join-Path $SeqDir 'bad-op.json'
    '[{"op":"teleport"}]' | Set-Content -LiteralPath $badOp -Encoding utf8
    $badSleep = Join-Path $SeqDir 'bad-sleep.json'
    '[{"op":"sleep","ms":60001}]' | Set-Content -LiteralPath $badSleep -Encoding utf8
    $notJson = Join-Path $SeqDir 'not-json.json'
    'not json' | Set-Content -LiteralPath $notJson -Encoding utf8

    $r = Invoke-Driver @('capture-sequence', '--steps', $okSteps, '--output-dir', (Join-Path $SeqDir 'out0'))
    Assert-OneJsonObject $r 'capture-sequence (missing hwnd)'
    Assert ($r.Json.category -eq 'usage_error') 'capture-sequence (missing hwnd) -- category:usage_error'

    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--output-dir', (Join-Path $SeqDir 'out1'))
    Assert-OneJsonObject $r 'capture-sequence (missing steps)'
    Assert ($r.Json.category -eq 'usage_error') 'capture-sequence (missing steps) -- category:usage_error'

    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', $notJson, '--output-dir', (Join-Path $SeqDir 'out2'))
    Assert ($r.Json.category -eq 'usage_error') 'capture-sequence (steps not JSON) -- category:usage_error'

    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', $badOp, '--output-dir', (Join-Path $SeqDir 'out3'))
    Assert ($r.Json.category -eq 'usage_error') 'capture-sequence (unknown op) -- category:usage_error'

    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', $badSleep, '--output-dir', (Join-Path $SeqDir 'out4'))
    Assert ($r.Json.category -eq 'usage_error') 'capture-sequence (sleep above bound) -- category:usage_error'

    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', $okSteps, '--output-dir', (Join-Path $SeqDir 'out5'))
    Assert-OneJsonObject $r 'capture-sequence (no such window)'
    Assert ($r.Json.category -eq 'target_error') 'capture-sequence (no such window) -- category:target_error'
    Assert (-not (Test-Path (Join-Path $SeqDir 'out5'))) 'capture-sequence (no such window) -- no output directory is created'
}
finally {
    Remove-Item -LiteralPath $SeqDir -Recurse -Force -ErrorAction SilentlyContinue
}

# R4 -- capture-sequence evidence safety (PR #291 round 4). Names are validated before any window,
# desktop, or file-system work; outputs are created with CreateNew; failures keep one stdout JSON.
$R4 = Join-Path ([System.IO.Path]::GetTempPath()) ("seq-r4-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $R4 | Out-Null
function New-R4Steps([string]$Json) { $f = Join-Path $R4 ("steps-" + [guid]::NewGuid().ToString('N') + '.json'); $Json | Set-Content -LiteralPath $f -Encoding utf8; return $f }
function Get-R4Hash([string]$Path) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash }
try {
    $sentinel = Join-Path $R4 'sentinel.png'
    [System.IO.File]::WriteAllBytes($sentinel, [byte[]](1, 2, 3, 4))
    $sentinelHash = Get-R4Hash $sentinel

    # R4-T01 -- invalid capture names are usage_error before any side effect, even with a bogus HWND.
    $badNames = @('null', '""', '"../escape"', '"..\\escape"', '"C:abs"', '"a.b"', '" "', '"frame 1"', '"ä"', '"CON"', '"lpt1"', ('"' + ('a' * 65) + '"'), '7')
    foreach ($n in $badNames) {
        $steps = New-R4Steps ('[{"op":"capture","name":' + $n + '}]')
        $out = Join-Path $R4 ('out-t01-' + [guid]::NewGuid().ToString('N'))
        $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', $steps, '--output-dir', $out)
        Assert-OneJsonObject $r "R4-T01 name $n"
        Assert ($r.Json.category -eq 'usage_error' -and $r.ExitCode -eq 1) "R4-T01 name $n -- usage_error, exit 1"
        Assert (-not (Test-Path $out)) "R4-T01 name $n -- no output directory"
    }
    Assert ((Get-R4Hash $sentinel) -eq $sentinelHash) 'R4-T01 -- nearby sentinel unchanged'

    # R4-T02 -- case-insensitive duplicates are rejected before execution.
    foreach ($json in @('[{"op":"capture","name":"Frame"},{"op":"capture","name":"frame"}]', '[{"op":"capture","name":"shot"},{"op":"sleep","ms":1},{"op":"capture","name":"shot"}]')) {
        $out = Join-Path $R4 ('out-t02-' + [guid]::NewGuid().ToString('N'))
        $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', (New-R4Steps $json), '--output-dir', $out)
        Assert ($r.Json.category -eq 'usage_error' -and $r.Json.error -match 'duplicates') "R4-T02 $json -- usage_error (duplicate)"
        Assert (-not (Test-Path $out)) "R4-T02 $json -- no output directory"
    }
    $env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'sequence-names'
    try {
        $r = Invoke-Driver @('capture-sequence', '--steps', (New-R4Steps '[{"op":"capture","name":"lr01_1600"},{"op":"capture","name":"frame-01"}]'))
        Assert ($r.Json.success -eq $true -and $null -eq $r.Json.name_error) 'R4-T02 -- legal names lr01_1600 / frame-01 pass the shared preflight'
    }
    finally { Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue }

    # R4-T03 -- a path-escaping name cannot reach a file outside the output directory.
    $out = Join-Path $R4 'out-t03'
    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', (New-R4Steps '[{"op":"capture","name":"../sentinel"}]'), '--output-dir', $out)
    Assert ($r.Json.category -eq 'usage_error') 'R4-T03 -- ../sentinel is usage_error'
    Assert ((Get-R4Hash $sentinel) -eq $sentinelHash) 'R4-T03 -- external sentinel SHA-256 unchanged'
    Assert (-not (Test-Path $out)) 'R4-T03 -- no output directory'

    # R4-T04 -- an existing output dir is refused; a CreateNew PNG collision is tool_error and the
    # pre-existing file keeps its bytes.
    $existing = Join-Path $R4 'existing'
    New-Item -ItemType Directory -Path $existing | Out-Null
    $kept = Join-Path $existing 'frame.png'
    [System.IO.File]::WriteAllBytes($kept, [byte[]](9, 9, 9))
    $keptHash = Get-R4Hash $kept
    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', (New-R4Steps '[{"op":"capture","name":"frame"}]'), '--output-dir', $existing)
    Assert ($r.Json.category -eq 'usage_error' -and $r.Json.error -match 'already exists') 'R4-T04 -- existing --output-dir is usage_error'
    Assert ((Get-R4Hash $kept) -eq $keptHash) 'R4-T04 -- existing file unchanged'

    $env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'sequence-fault'
    try {
        $pc = Join-Path $R4 'probe-png'
        $r = Invoke-Driver @('capture-sequence', '--fault', 'png-collision', '--output-dir', $pc)
        Assert-OneJsonObject $r 'R4-T04 png collision probe'
        Assert ($r.Json.run.success -eq $false -and $r.Json.run.category -eq 'tool_error' -and $r.Json.run.failure_stage -eq 'save-png') 'R4-T04 -- CreateNew PNG collision is tool_error at save-png'
        Assert ($r.Json.pre_hashes.'b.png' -eq $r.Json.post_hashes.'b.png') 'R4-T04 -- colliding pre-existing b.png unchanged'
        $saved = @($r.Json.run.files | ForEach-Object { Split-Path -Leaf $_ })
        Assert (($saved -join ',') -eq 'a.png,c.png') 'R4-T06 -- files lists only the frames actually saved (a, c)'
        Assert ($r.Json.run.result_json_persisted -eq $true) 'R4-T06 -- result JSON still persisted after a PNG failure'
        Assert ($r.Json.all_tracked_disposed -eq $true -and $r.Json.tracked_count -ge 6) 'R4-T06 -- every Bitmap/Graphics disposed after a PNG failure'

        # R4-T05 -- a capture exception after the Bitmap is allocated stops the sequence.
        $ce = Join-Path $R4 'probe-capture'
        $r = Invoke-Driver @('capture-sequence', '--fault', 'capture-exception', '--output-dir', $ce)
        Assert-OneJsonObject $r 'R4-T05 capture exception probe'
        $run = $r.Json.run
        Assert ($run.success -eq $false -and $run.category -eq 'tool_error' -and $run.failure_stage -eq 'step') 'R4-T05 -- tool_error at the failing step'
        Assert ($run.failed_step_index -eq 1 -and $run.failed_step_op -eq 'capture') 'R4-T05 -- failed step index/op recorded'
        $failedRec = @($run.steps | Where-Object { $_.index -eq 1 })[0]
        Assert ($failedRec.status -eq 'failed' -and $null -ne $failedRec.t_start_ms -and $null -ne $failedRec.t_end_ms -and $failedRec.error -match 'injected') 'R4-T05 -- failed step carries status, error, and timing'
        Assert (-not (@($r.Json.executed) -contains 'click:after-b')) 'R4-T05 -- no later step (click) runs after the failure'
        Assert ($r.Json.tracked_count -ge 4 -and $r.Json.all_tracked_disposed -eq $true) 'R4-T05 -- the failed Bitmap/Graphics and the retained frame are disposed'
        $ceFiles = @($run.files | ForEach-Object { Split-Path -Leaf $_ })
        Assert (($ceFiles -join ',') -eq 'a.png') 'R4-T05 -- the frame captured before the failure is kept'

        # R4-T06 -- a result JSON collision is reported, never claimed as persisted or successful.
        $rc = Join-Path $R4 'probe-result'
        $r = Invoke-Driver @('capture-sequence', '--fault', 'result-collision', '--output-dir', $rc)
        Assert-OneJsonObject $r 'R4-T06 result collision probe'
        Assert ($r.Json.run.success -eq $false -and $r.Json.run.result_json_persisted -eq $false -and $r.Json.run.category -eq 'tool_error' -and $r.Json.run.failure_stage -eq 'result-json') 'R4-T06 -- unpersisted result JSON is tool_error, success:false'
        Assert ($r.Json.pre_hashes.'sequence-result.json' -eq $r.Json.post_hashes.'sequence-result.json') 'R4-T06 -- pre-existing sequence-result.json unchanged'

        # R4-T07 -- after a failure the same directory is refused and a new one is independent.
        $before = @{}; foreach ($f in Get-ChildItem -LiteralPath $ce -File) { $before[$f.Name] = Get-R4Hash $f.FullName }
        $r = Invoke-Driver @('capture-sequence', '--fault', 'capture-exception', '--output-dir', $ce)
        Assert ($r.Json.category -eq 'usage_error') 'R4-T07 -- rerun into the same directory is refused (probe)'
        Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue
        $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', (New-R4Steps '[{"op":"capture","name":"a"}]'), '--output-dir', $ce)
        Assert ($r.Json.category -eq 'usage_error') 'R4-T07 -- rerun into the same directory is refused (command)'
        $after = @{}; foreach ($f in Get-ChildItem -LiteralPath $ce -File) { $after[$f.Name] = Get-R4Hash $f.FullName }
        Assert ((($before.Keys | Sort-Object) -join ',') -eq (($after.Keys | Sort-Object) -join ',') -and @($before.Keys | Where-Object { $before[$_] -ne $after[$_] }).Count -eq 0) 'R4-T07 -- earlier PNG/result files unchanged'
        $env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'sequence-fault'
        $ce2 = Join-Path $R4 'probe-capture-2'
        $r = Invoke-Driver @('capture-sequence', '--fault', 'capture-exception', '--output-dir', $ce2)
        Assert ((Test-Path (Join-Path $ce2 'a.png')) -and (Test-Path (Join-Path $ce2 'sequence-result.json'))) 'R4-T07 -- a new directory receives its own independent evidence'
    }
    finally { Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue }
}
finally {
    Remove-Item -LiteralPath $R4 -Recurse -Force -ErrorAction SilentlyContinue
}

# R5 -- capture-sequence UIA root initialization (PR #291 round 5, A-09). The root is resolved inside
# Invoke-SequenceWithRoot; a failure is an `initialization` result persisted with CreateNew, no step
# runs, and stdout stays one JSON object with exit 1.
$R5 = Join-Path ([System.IO.Path]::GetTempPath()) ("seq-r5-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $R5 | Out-Null
function Get-R5Saved([string]$Dir) { return (Get-Content -Raw -LiteralPath (Join-Path $Dir 'sequence-result.json') | ConvertFrom-Json) }
try {
    $env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'sequence-init-fault'

    # R5-T01 -- a categorized resolver failure stops before any step and is persisted.
    $d1 = Join-Path $R5 't01'
    $r = Invoke-Driver @('capture-sequence', '--fault', 'throw', '--output-dir', $d1)
    Assert-OneJsonObject $r 'R5-T01'
    Assert ($r.ExitCode -eq 1 -and $r.Json.success -eq $false) 'R5-T01 -- exit 1, success:false'
    Assert ($r.Json.category -eq 'tool_error' -and $r.Json.failure_stage -eq 'initialization') 'R5-T01 -- tool_error / initialization'
    Assert ($null -eq $r.Json.failed_step_index -and $null -eq $r.Json.failed_step_op) 'R5-T01 -- failed_step_index/op are null'
    Assert (@($r.Json.steps).Count -eq 0 -and @($r.Json.files).Count -eq 0) 'R5-T01 -- steps and files are empty'
    Assert (@($r.Json.probe_executed).Count -eq 0) 'R5-T01 -- no step (including the click recorder) ran'
    Assert ($r.Json.result_json_persisted -eq $true -and (Test-Path (Join-Path $d1 'sequence-result.json'))) 'R5-T01 -- sequence-result.json persisted'
    $saved = Get-R5Saved $d1
    Assert ($saved.category -eq $r.Json.category -and $saved.error -eq $r.Json.error -and $saved.failure_stage -eq 'initialization') 'R5-T01 -- saved primary cause matches stdout'
    Assert ($r.Json.error -match 'injected UIA root failure') 'R5-T01 -- original error kept'
    Assert (@(Get-ChildItem -LiteralPath $d1 -Filter '*.png').Count -eq 0) 'R5-T01 -- no PNG created'

    # R5-T02 -- a null root is target_error.
    $d2 = Join-Path $R5 't02'
    $r = Invoke-Driver @('capture-sequence', '--fault', 'null', '--output-dir', $d2)
    Assert-OneJsonObject $r 'R5-T02'
    Assert ($r.ExitCode -eq 1 -and $r.Json.category -eq 'target_error' -and $r.Json.failure_stage -eq 'initialization') 'R5-T02 -- null root is target_error / initialization, exit 1'
    Assert (@($r.Json.probe_executed).Count -eq 0 -and (Test-Path (Join-Path $d2 'sequence-result.json'))) 'R5-T02 -- no step ran, result persisted'

    # R5-T03 -- a plain exception with a gone HWND is target_error after the re-check.
    $d3 = Join-Path $R5 't03'
    $r = Invoke-Driver @('capture-sequence', '--fault', 'race', '--output-dir', $d3)
    Assert-OneJsonObject $r 'R5-T03'
    Assert ($r.ExitCode -eq 1 -and $r.Json.category -eq 'target_error' -and $r.Json.failure_stage -eq 'initialization') 'R5-T03 -- HWND gone: target_error / initialization'
    Assert ($r.Json.error -match 'injected element-not-available') 'R5-T03 -- injected cause kept'
    Assert (@($r.Json.probe_executed).Count -eq 0) 'R5-T03 -- no step ran'

    # RD5-04 live-HWND branch: a plain exception while the window still exists is tool_error.
    $d3b = Join-Path $R5 't03-alive'
    $r = Invoke-Driver @('capture-sequence', '--fault', 'alive', '--output-dir', $d3b)
    if ($r.Json.probe_skipped) { Write-Output "SKIP: R5 alive-HWND branch -- $($r.Json.probe_skipped)" }
    else { Assert ($r.Json.category -eq 'tool_error' -and $r.Json.failure_stage -eq 'initialization') 'R5 (RD5-04) -- HWND alive: tool_error / initialization' }

    # R5-T04 -- the result JSON collides: primary cause kept, existing bytes unchanged.
    $d4 = Join-Path $R5 't04'
    $r = Invoke-Driver @('capture-sequence', '--fault', 'throw-collision', '--output-dir', $d4)
    Assert-OneJsonObject $r 'R5-T04'
    Assert ($r.ExitCode -eq 1 -and $r.Json.category -eq 'tool_error' -and $r.Json.failure_stage -eq 'initialization') 'R5-T04 -- primary tool_error / initialization kept'
    Assert ($r.Json.result_json_persisted -eq $false -and -not [string]::IsNullOrEmpty($r.Json.result_json_error)) 'R5-T04 -- result_json_persisted:false with result_json_error'
    Assert ($r.Json.error -match 'injected UIA root failure') 'R5-T04 -- error is still the injected root failure'
    Assert ($r.Json.probe_pre_hash -eq $r.Json.probe_post_hash) 'R5-T04 -- pre-existing sequence-result.json bytes unchanged'

    # R5-T05 -- a valid root proceeds to Invoke-SequenceGuarded exactly as before.
    $d5 = Join-Path $R5 't05'
    $r = Invoke-Driver @('capture-sequence', '--fault', 'valid', '--output-dir', $d5)
    Assert-OneJsonObject $r 'R5-T05'
    Assert ($r.ExitCode -eq 0 -and $r.Json.success -eq $true -and $r.Json.result_json_persisted -eq $true) 'R5-T05 -- success, exit 0, result persisted'
    Assert ((@($r.Json.probe_executed) -join ',') -eq 'sleep,read,click') 'R5-T05 -- every step ran once, in order'
    Assert ((@($r.Json.steps | ForEach-Object { $_.status }) -join ',') -eq 'ok,ok,ok') 'R5-T05 -- step records are the usual guarded records'
    Assert ($null -eq $r.Json.failure_stage) 'R5-T05 -- no failure_stage on success'

    # R5-T06 -- a failed run's directory is never reused; a new directory is independent.
    $before = Get-FileHash -LiteralPath (Join-Path $d1 'sequence-result.json') -Algorithm SHA256
    $r = Invoke-Driver @('capture-sequence', '--fault', 'throw', '--output-dir', $d1)
    Assert ($r.Json.category -eq 'usage_error') 'R5-T06 -- rerun into the failed directory is refused (probe)'
    Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue
    $okSteps = Join-Path $R5 'ok.json'
    '[{"op":"sleep","ms":1}]' | Set-Content -LiteralPath $okSteps -Encoding utf8
    $r = Invoke-Driver @('capture-sequence', '--hwnd', '1', '--steps', $okSteps, '--output-dir', $d1)
    Assert ($r.Json.category -eq 'usage_error') 'R5-T06 -- rerun into the failed directory is refused (command)'
    $after = Get-FileHash -LiteralPath (Join-Path $d1 'sequence-result.json') -Algorithm SHA256
    Assert ($before.Hash -eq $after.Hash) 'R5-T06 -- the failed run''s result file is unchanged'
    $env:ELWINDUI_DRIVER_CONTRACT_PROBE = 'sequence-init-fault'
    $d6 = Join-Path $R5 't06-new'
    $r = Invoke-Driver @('capture-sequence', '--fault', 'throw', '--output-dir', $d6)
    Assert ($r.Json.failure_stage -eq 'initialization' -and (Test-Path (Join-Path $d6 'sequence-result.json'))) 'R5-T06 -- a new directory stores its own failure result'
}
finally {
    Remove-Item Env:ELWINDUI_DRIVER_CONTRACT_PROBE -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $R5 -Recurse -Force -ErrorAction SilentlyContinue
}

if ($script:FailureCount -gt 0) {
    Write-Output "`n$script:FailureCount assertion(s) failed."
    exit 1
}
Write-Output "`nAll driver-contract assertions passed."
exit 0
