<#
.SYNOPSIS
    Live GUI smoke test for windows-ui-driver.ps1 against the real theme-demo example. Replaces
    the removed tools/test-theme-demo-uia.py. Must run host-context (outside any sandbox) as a
    normal, non-elevated user on an unlocked interactive desktop -- see docs/agents/winui3-e2e.md.

.DESCRIPTION
    Current theme-demo (examples/theme-demo/src/main.rs) only has three theme buttons (Default,
    Ocean, Solarized) that switch BrushStyle-resolved colors and update one text label to the
    active theme's name -- it predates and no longer matches the older Dark/Light/System buttons,
    "Disabled native state" sample, nested TabView, and numeric revision counter that the removed
    Python script exercised (that content was part of an earlier, since-simplified theme-demo).
    Issue #242's approved smoke acceptance covers exactly what current theme-demo provides; the
    disabled-control and nested-TabView cases are not part of this script's required case set (see
    the "Design decisions" section of Issue #242).

    Required cases, run in order:

    1. doctor.
    2. launch theme-demo, discover its window.
    3. UIA tree readiness (wait-for a known element before querying).
    4. UIA `invoke` the Ocean button, verify the label text becomes "Ocean".
    5. Real-mouse `point-click` the Solarized button, verify the label text becomes "Solarized" --
       proves actual pointer delivery, not just a successful injection call. No standalone
       `focus-window` precedes this: winapp's own real-input path establishes foreground itself
       (see docs/design/tools/windows_ui_driver_design.md Section 6).
    6. Screenshot. The default WGC capture-window path has been observed to return a blank/
       near-empty image on at least one host in this repository's own testing; this script
       therefore captures with --capture-screen and records which mode was used.
    7. Terminate (always, in `finally`).

    Every case result is exactly PASS, FAIL, NOT RUN, or BLOCKED -- no other status value is used.
    A required case reported NOT RUN (its precondition never became true) counts against overall
    PASS the same as FAIL/BLOCKED.

.PARAMETER Issue
    When given, full per-action evidence (stdout, stderr, parsed JSON) and session metadata are
    saved under .agent-state/issues/<Issue>/e2e/<head>/<run-id>/.

.NOTES
    Exits non-zero if any required case does not PASS.
#>

param(
    [int]$Issue = 0
)

$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$Driver = Join-Path $PSScriptRoot '..\windows-ui-driver.ps1'
$DemoExe = Join-Path $Root 'target\debug\theme-demo.exe'
$DemoSrc = Join-Path $Root 'examples\theme-demo\src\main.rs'

function Invoke-Driver {
    param([string[]]$DriverArgs)
    # Raw Process.Start + synchronous ReadToEnd/WaitForExit -- the same pattern
    # windows-ui-driver.ps1's own Invoke-WinApp already uses successfully to call the (short-lived)
    # winapp backend. This driver.ps1 invocation itself is also short-lived (it always exits in
    # ~1-1.5s once its own launched grandchild's window is found or its wait-window-timeout elapses),
    # unlike theme-demo.exe. Capturing a nested pwsh child's stdout via PowerShell's own `&` operator
    # (or via Start-Process -Wait) was observed to hang this script indefinitely once that
    # grandchild GUI process was running, even though the identical `windows-ui-driver.ps1 launch`
    # invocation run directly at the top level (not nested inside another pwsh script) reliably
    # completes in ~1.5s every time.
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'pwsh'
    foreach ($a in (@('-NoProfile', '-File', $Driver) + $DriverArgs)) { $psi.ArgumentList.Add($a) }
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardInput = $true
    $psi.UseShellExecute = $false
    $proc = [System.Diagnostics.Process]::Start($psi)
    $proc.StandardInput.Close()
    $stdout = $proc.StandardOutput.ReadToEnd()
    $stderr = $proc.StandardError.ReadToEnd()
    $proc.WaitForExit()
    $exitCode = $proc.ExitCode
    $json = $null
    try { $json = $stdout | ConvertFrom-Json -ErrorAction Stop } catch { $json = $null }
    return @{ ExitCode = $exitCode; StdOut = $stdout; StdErr = $stderr; Json = $json }
}

$script:Results = New-Object System.Collections.Generic.List[object]
function Record {
    param([string]$Case, [string]$Status, [string]$Detail = '', [bool]$Required = $true)
    $script:Results.Add([ordered]@{ case = $Case; status = $Status; required = $Required; detail = $Detail })
    Write-Output "$Status -- $Case$(if ($Detail) { ": $Detail" })"
}

# Evidence directory (optional).
$RunDir = $null
if ($Issue -gt 0) {
    Push-Location $Root
    $headShort = (git rev-parse --short=12 HEAD).Trim()
    $originMaster = (git rev-parse origin/master 2>$null)
    Pop-Location
    $runId = (Get-Date -AsUTC).ToString('yyyyMMddTHHmmssZ')
    $RunDir = Join-Path $Root ".agent-state\issues\$Issue\e2e\$headShort\$runId"
    New-Item -ItemType Directory -Force -Path $RunDir | Out-Null
}

function Save-DriverEvidence {
    param([string]$Step, $Result)
    if (-not $RunDir) { return }
    if ($null -ne $Result.StdOut) { $Result.StdOut | Out-File -FilePath (Join-Path $RunDir "$Step.stdout") -Encoding utf8 }
    if ($null -ne $Result.StdErr) { $Result.StdErr | Out-File -FilePath (Join-Path $RunDir "$Step.stderr") -Encoding utf8 }
    if ($null -ne $Result.Json) { ($Result.Json | ConvertTo-Json -Depth 16) | Out-File -FilePath (Join-Path $RunDir "$Step.json") -Encoding utf8 }
}

# Build only if missing/stale.
$needsBuild = -not (Test-Path $DemoExe)
if (-not $needsBuild) {
    $needsBuild = (Get-Item $DemoSrc).LastWriteTimeUtc -gt (Get-Item $DemoExe).LastWriteTimeUtc
}
if ($needsBuild) {
    Push-Location $Root
    . .\tools\setup-vs-env.ps1 | Out-Null
    cargo build -p theme-demo 2>&1 | Out-Null
    Pop-Location
    if (-not (Test-Path $DemoExe)) {
        Record 'build theme-demo' 'BLOCKED' 'cargo build did not produce the expected executable'
        exit 1
    }
}

# doctor once.
$doctor = Invoke-Driver @('doctor')
Save-DriverEvidence 'doctor' $doctor
if (-not $doctor.Json.success) {
    Record 'doctor' 'BLOCKED' "category=$($doctor.Json.category) error=$($doctor.Json.error)"
    exit 1
}
Record 'doctor' 'PASS' "winapp $($doctor.Json.winapp_version)"

# Launch once.
$launch = Invoke-Driver @('launch', '--path', $DemoExe, '--wait-window-timeout', '10')
Save-DriverEvidence 'launch' $launch
if (-not $launch.Json.success -or -not $launch.Json.window) {
    Record 'launch theme-demo' 'BLOCKED' 'no unambiguous window discovered within timeout'
    exit 1
}
$processId = $launch.Json.pid
$hwnd = $launch.Json.window.hwnd
Record 'launch theme-demo' 'PASS' "pid=$processId hwnd=$hwnd"

# Session metadata (Issue-scoped runs only).
if ($RunDir) {
    $metadata = [ordered]@{
        head              = $headShort
        origin_master_sha = $originMaster
        os_version        = [System.Environment]::OSVersion.VersionString
        architecture      = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        session_id        = $doctor.Json.session_id
        winapp_version    = $doctor.Json.winapp_version
        pid               = $processId
        hwnd              = $hwnd
        initial_window    = $launch.Json.window
    }
    ($metadata | ConvertTo-Json -Depth 16) | Out-File -FilePath (Join-Path $RunDir 'session-metadata.json') -Encoding utf8
}

try {
    # A freshly-appeared HWND does not guarantee the UIA tree underneath it is populated yet --
    # poll for a known element before the first UIA action rather than querying immediately.
    $ready = Invoke-Driver @('wait-for', '--hwnd', $hwnd, '--selector', 'Default', '--timeout-ms', '5000')
    Save-DriverEvidence 'ready' $ready
    if (-not $ready.Json.success) {
        Record 'UIA tree ready' 'BLOCKED' 'the Default button never became discoverable via UIA'
        exit 1
    }
    Record 'UIA tree ready' 'PASS'

    # UIA invoke + postcondition. Resolve the current selector by search rather than a hardcoded
    # one -- the semantic-slug hash suffix is derived from the element's RuntimeId and is not
    # guaranteed identical across separate app launches.
    $oceanSearch = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Ocean')
    Save-DriverEvidence 'search-ocean' $oceanSearch
    $oceanButton = @($oceanSearch.Json.backend.matches | Where-Object { $_.type -eq 'Button' })[0]
    if (-not $oceanButton) {
        Record 'UIA theme action (Ocean)' 'NOT RUN' 'Ocean button not found before invoke'
    }
    else {
        $invoke = Invoke-Driver @('invoke', '--hwnd', $hwnd, '--selector', $oceanButton.selector)
        Save-DriverEvidence 'invoke-ocean' $invoke
        if (-not $invoke.Json.success) {
            Record 'UIA theme action (Ocean)' ($(if ($invoke.Json.category -eq 'environment_blocker') { 'BLOCKED' } else { 'FAIL' })) "category=$($invoke.Json.category)"
        }
        else {
            $search = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Ocean')
            Save-DriverEvidence 'postcondition-ocean' $search
            $labelMatches = @($search.Json.backend.matches | Where-Object { $_.type -eq 'Text' -and $_.name -eq 'Ocean' })
            if ($labelMatches.Count -ge 1) {
                Record 'UIA theme action (Ocean)' 'PASS' 'label text became "Ocean"'
            }
            else {
                Record 'UIA theme action (Ocean)' 'FAIL' 'invoke reported success but no "Ocean" label was found afterward'
            }
        }
    }

    # Real-mouse point-click + postcondition. Re-resolve fresh coordinates first -- never reuse a
    # coordinate captured before a preceding UIA action. No standalone focus-window here: winapp's
    # own real-input path establishes foreground itself and fails fast (environment_blocker) if it
    # can't (see docs/design/tools/windows_ui_driver_design.md Section 6).
    $search = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Solarized')
    Save-DriverEvidence 'search-solarized' $search
    $button = @($search.Json.backend.matches | Where-Object { $_.type -eq 'Button' })[0]
    if (-not $button) {
        Record 'real-mouse theme action (Solarized)' 'NOT RUN' 'Solarized button not found before the click'
    }
    else {
        $cx = [int]($button.x + $button.width / 2)
        $cy = [int]($button.y + $button.height / 2)
        $click = Invoke-Driver @('point-click', '--hwnd', $hwnd, '--x', $cx, '--y', $cy)
        Save-DriverEvidence 'point-click-solarized' $click
        if (-not $click.Json.success) {
            Record 'real-mouse theme action (Solarized)' ($(if ($click.Json.category -eq 'environment_blocker') { 'BLOCKED' } else { 'FAIL' })) "category=$($click.Json.category)"
        }
        else {
            $search2 = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Solarized')
            Save-DriverEvidence 'postcondition-solarized' $search2
            $labelMatches = @($search2.Json.backend.matches | Where-Object { $_.type -eq 'Text' -and $_.name -eq 'Solarized' })
            if ($labelMatches.Count -ge 1) {
                Record 'real-mouse theme action (Solarized)' 'PASS' 'label text became "Solarized" -- real pointer delivery confirmed, not only a successful injection call'
            }
            else {
                Record 'real-mouse theme action (Solarized)' 'FAIL' 'click reported success but the label never changed to "Solarized" -- cross-reference #224'
            }
        }
    }

    # Screenshot. --capture-screen is used deliberately: the default WGC capture-window path
    # returned a blank/near-empty PNG on at least one host in this repository's own testing.
    $shotPath = if ($RunDir) { Join-Path $RunDir 'theme-demo.png' } else { Join-Path $env:TEMP 'theme-demo-e2e-shot.png' }
    $shot = Invoke-Driver @('capture-window', '--hwnd', $hwnd, '--capture-screen', '--output', $shotPath)
    Save-DriverEvidence 'capture' $shot
    if ($shot.Json.success -and $shot.Json.file_exists -and $shot.Json.file_size -gt 2048) {
        Record 'screenshot' 'PASS' "mode=$($shot.Json.capture_mode) path=$shotPath size=$($shot.Json.file_size)"
    }
    elseif (-not $shot.Json.success -and $shot.Json.category -eq 'environment_blocker') {
        Record 'screenshot' 'BLOCKED' "category=$($shot.Json.category) -- --capture-screen brings the window to the foreground itself and is subject to the same foreground-lock restriction as focus-window"
    }
    else {
        Record 'screenshot' 'FAIL' "success=$($shot.Json.success) mode=$($shot.Json.capture_mode) size=$($shot.Json.file_size)"
    }
}
finally {
    $term = Invoke-Driver @('terminate', '--pid', $processId, '--timeout', '5')
    Save-DriverEvidence 'terminate' $term
    if ($term.Json.success) {
        Record 'cleanup' 'PASS' "forced=$($term.Json.forced)"
    }
    else {
        Record 'cleanup' 'FAIL' 'process did not exit'
    }
}

# Only PASS/FAIL/NOT RUN/BLOCKED are ever recorded above. A required case that is anything other
# than PASS -- including NOT RUN, whose precondition simply never became true -- prevents an
# overall PASS.
$failed = @($script:Results | Where-Object { $_.required -and $_.status -ne 'PASS' })
if ($RunDir) { (($script:Results | ForEach-Object { [pscustomobject]$_ }) | ConvertTo-Json -Depth 8) | Out-File -FilePath (Join-Path $RunDir 'results.json') -Encoding utf8 }
if ($failed.Count -gt 0) { exit 1 }
exit 0
