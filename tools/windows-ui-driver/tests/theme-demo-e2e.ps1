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
    This script tests exactly what the current demo actually provides:

    1. doctor once.
    2. Launch theme-demo once, discover its window.
    3. UIA `invoke` the Ocean button, verify the label text becomes "Ocean" (T5-equivalent).
    4. Real-mouse `point-click` the Solarized button, verify the label text becomes "Solarized"
       (T6-equivalent) -- proves actual pointer delivery, not just a successful injection call.
    5. Capture a screenshot. The default WGC capture-window path has been observed to return a
       blank/near-empty image on at least one host in this repository's own testing; this script
       therefore captures with --capture-screen and records which mode was used.
    6. Terminate.

    "Disabled native state" (T7) and nested-TabView discovery (T8) are reported NOT AVAILABLE --
    current theme-demo has no such elements; this is a repository-reality conflict with the
    original migration contract, reported rather than silently worked around (see the owning
    Issue's completion report).

.PARAMETER Issue
    When given, raw run evidence is saved under .agent-state/issues/<Issue>/e2e/<head>/<run-id>/.

.NOTES
    Exits non-zero if any executed case does not PASS. NOT AVAILABLE cases do not affect the exit
    code -- they are not failures, they are absent product surface.
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
    $null = $proc.StandardError.ReadToEnd()
    $proc.WaitForExit()
    $exitCode = $proc.ExitCode
    $json = $null
    try { $json = $stdout | ConvertFrom-Json -ErrorAction Stop } catch { $json = $null }
    return @{ ExitCode = $exitCode; Json = $json }
}

$script:Results = New-Object System.Collections.Generic.List[object]
function Record {
    param([string]$Case, [string]$Status, [string]$Detail = '')
    $script:Results.Add([ordered]@{ case = $Case; status = $Status; detail = $Detail })
    Write-Output "$Status -- $Case$(if ($Detail) { ": $Detail" })"
}

# Evidence directory (optional).
$RunDir = $null
if ($Issue -gt 0) {
    Push-Location $Root
    $headShort = (git rev-parse --short=12 HEAD).Trim()
    Pop-Location
    $runId = (Get-Date -AsUTC).ToString('yyyyMMddTHHmmssZ')
    $RunDir = Join-Path $Root ".agent-state\issues\$Issue\e2e\$headShort\$runId"
    New-Item -ItemType Directory -Force -Path $RunDir | Out-Null
}
function Save-Evidence {
    param([string]$Name, $Content)
    if ($RunDir) {
        ($Content | ConvertTo-Json -Depth 16) | Out-File -FilePath (Join-Path $RunDir $Name) -Encoding utf8
    }
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
Save-Evidence 'doctor.json' $doctor.Json
if (-not $doctor.Json.success) {
    Record 'doctor' 'BLOCKED' "category=$($doctor.Json.category) error=$($doctor.Json.error)"
    exit 1
}
Record 'doctor' 'PASS' "winapp $($doctor.Json.winapp_version)"

# Launch once.
$launch = Invoke-Driver @('launch', '--path', $DemoExe, '--wait-window-timeout', '10')
Save-Evidence 'launch.json' $launch.Json
if (-not $launch.Json.success -or -not $launch.Json.window) {
    Record 'launch theme-demo' 'BLOCKED' 'no unambiguous window discovered within timeout'
    exit 1
}
$processId = $launch.Json.pid
$hwnd = $launch.Json.window.hwnd
Record 'launch theme-demo' 'PASS' "pid=$processId hwnd=$hwnd"

try {
    # No explicit focus-window here: UIA `invoke` below does not require foreground, and this
    # driver's own focus-window (a plain SetForegroundWindow) is subject to Windows' anti-focus-
    # -stealing restriction when called from a background process -- it was observed to reliably
    # report BLOCKED in exactly that situation, even though the target window is healthy and
    # responsive. The real-mouse path below relies on winapp's own `drag` verb instead, which
    # brings its target to the foreground itself (and fails fast with `foreground_not_target` if it
    # can't) as part of real input delivery -- see docs/design/tools/windows_ui_driver_design.md
    # Section 6.

    # A freshly-appeared HWND does not guarantee the UIA tree underneath it is populated yet --
    # poll for a known element before the first UIA action rather than querying immediately.
    $ready = Invoke-Driver @('wait-for', '--hwnd', $hwnd, '--selector', 'Default', '--timeout-ms', '5000')
    if (-not $ready.Json.success) {
        Record 'UIA tree ready' 'BLOCKED' 'the Default button never became discoverable via UIA'
        exit 1
    }

    # T5-equivalent: UIA invoke + postcondition. Resolve the current selector by search rather than
    # a hardcoded one -- the semantic-slug hash suffix is derived from the element's RuntimeId and
    # is not guaranteed identical across separate app launches.
    $oceanSearch = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Ocean')
    $oceanButton = @($oceanSearch.Json.backend.matches | Where-Object { $_.type -eq 'Button' })[0]
    if (-not $oceanButton) {
        Record 'UIA theme action (Ocean)' 'NOT RUN' 'Ocean button not found before invoke'
        $invoke = $null
    }
    else {
        $invoke = Invoke-Driver @('invoke', '--hwnd', $hwnd, '--selector', $oceanButton.selector)
    }
    Save-Evidence 'invoke-ocean.json' $invoke.Json
    if ($null -eq $invoke) {
        # Already recorded NOT RUN above.
    }
    elseif (-not $invoke.Json.success) {
        Record 'UIA theme action (Ocean)' ($(if ($invoke.Json.category -eq 'environment_blocker') { 'BLOCKED' } else { 'FAIL' })) "category=$($invoke.Json.category)"
    }
    else {
        $search = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Ocean')
        Save-Evidence 'postcondition-ocean.json' $search.Json
        $labelMatches = @($search.Json.backend.matches | Where-Object { $_.type -eq 'Text' -and $_.name -eq 'Ocean' })
        if ($labelMatches.Count -ge 1) {
            Record 'UIA theme action (Ocean)' 'PASS' 'label text became "Ocean"'
        }
        else {
            Record 'UIA theme action (Ocean)' 'FAIL' 'invoke reported success but no "Ocean" label was found afterward'
        }
    }

    # T6-equivalent: real-mouse point-click + postcondition. Re-resolve fresh coordinates first --
    # never reuse a coordinate captured before a preceding UIA action or focus change.
    $search = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Solarized')
    $button = @($search.Json.backend.matches | Where-Object { $_.type -eq 'Button' })[0]
    if (-not $button) {
        Record 'real-mouse theme action (Solarized)' 'NOT RUN' 'Solarized button not found before the click'
    }
    else {
        $cx = [int]($button.x + $button.width / 2)
        $cy = [int]($button.y + $button.height / 2)
        $click = Invoke-Driver @('point-click', '--hwnd', $hwnd, '--x', $cx, '--y', $cy)
        Save-Evidence 'point-click-solarized.json' $click.Json
        if (-not $click.Json.success) {
            Record 'real-mouse theme action (Solarized)' ($(if ($click.Json.category -eq 'environment_blocker') { 'BLOCKED' } else { 'FAIL' })) "category=$($click.Json.category)"
        }
        else {
            $search2 = Invoke-Driver @('search', '--hwnd', $hwnd, '--query', 'Solarized')
            Save-Evidence 'postcondition-solarized.json' $search2.Json
            $labelMatches = @($search2.Json.backend.matches | Where-Object { $_.type -eq 'Text' -and $_.name -eq 'Solarized' })
            if ($labelMatches.Count -ge 1) {
                Record 'real-mouse theme action (Solarized)' 'PASS' 'label text became "Solarized" -- real pointer delivery confirmed, not only a successful injection call'
            }
            else {
                Record 'real-mouse theme action (Solarized)' 'FAIL' 'click reported success but the label never changed to "Solarized" -- cross-reference #224'
            }
        }
    }

    # T7/T8-equivalent: not available in the current demo -- reported, not silently skipped.
    Record 'disabled native-state control' 'NOT AVAILABLE' 'current theme-demo has no disabled-state sample (removed since the migrated Python script was written)'
    Record 'nested TabView discovery' 'NOT AVAILABLE' 'current theme-demo has no TabView (removed since the migrated Python script was written)'

    # Screenshot. --capture-screen is used deliberately: the default WGC capture-window path
    # returned a blank/near-empty PNG on at least one host in this repository's own testing.
    $shotPath = if ($RunDir) { Join-Path $RunDir 'theme-demo.png' } else { Join-Path $env:TEMP 'theme-demo-e2e-shot.png' }
    $shot = Invoke-Driver @('capture-window', '--hwnd', $hwnd, '--capture-screen', '--output', $shotPath)
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
    if ($term.Json.success) {
        Record 'cleanup' 'PASS' "forced=$($term.Json.forced)"
    }
    else {
        Record 'cleanup' 'FAIL' 'process did not exit'
    }
}

$failed = @($script:Results | Where-Object { $_.status -eq 'FAIL' -or $_.status -eq 'BLOCKED' })
if ($RunDir) { Save-Evidence 'results.json' $script:Results }
if ($failed.Count -gt 0) { exit 1 }
exit 0
