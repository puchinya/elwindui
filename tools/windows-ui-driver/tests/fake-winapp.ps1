<#
.SYNOPSIS
    Fake `winapp` backend for tools/windows-ui-driver/tests/driver-contract.ps1. Point
    ELWINDUI_WINAPP_PATH at this file to exercise windows-ui-driver.ps1's own normalization logic
    without a real winapp install or a real GUI process.

.DESCRIPTION
    Responds only to the specific argument shapes driver-contract.ps1 sends; this is test
    infrastructure for the adapter, not a model of winapp's full behavior (see the real syntax in
    docs/design/tools/windows_ui_driver_design.md and the winapp-cli UI Automation reference).
#>

param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$FakeArgs
)

if ($FakeArgs -contains '--version') {
    if ($env:ELWINDUI_FAKE_WINAPP_VERSION_FAIL -eq '1') {
        [Console]::Error.WriteLine('fake-winapp.ps1: simulated broken install (--version failed)')
        exit 3
    }
    Write-Output 'fake-winapp 0.0.0-test'
    exit 0
}

# Scenario selection: driver-contract.ps1 encodes which fake scenario to run as the search/query
# text or selector, since that's the one value windows-ui-driver.ps1 always forwards verbatim.
$joined = $FakeArgs -join ' '

if ($joined -match 'FAKE_SUCCESS') {
    Write-Output '{"success":true,"matchCount":1,"matches":[{"name":"FAKE_SUCCESS","selector":"fake-1"}]}'
    exit 0
}

if ($joined -match 'FAKE_SELECTOR_CLICK') {
    # Regression fixture for windows-ui-driver.ps1's selector-mode `point-click` (Issue #236
    # delta contract Section 4.1/4.2): proves the driver actually invokes `ui click`, not a
    # zero-distance `ui drag`, for this mode. Checked positionally ($FakeArgs[1], the verb right
    # after 'ui'), not via the case-insensitive `-match` used for scenario selection above, since
    # the selector text itself contains the substring "CLICK".
    $verb = if ($FakeArgs.Count -ge 2) { $FakeArgs[1] } else { $null }
    if ($verb -ne 'click') {
        Write-Output (@{ success = $false; verb = $verb; error = "FAKE_SELECTOR_CLICK must route through 'ui click', not '$verb'" } | ConvertTo-Json -Compress)
        exit 1
    }
    $right = [bool]($FakeArgs -contains '--right')
    Write-Output (@{ success = $true; verb = 'click'; right = $right } | ConvertTo-Json -Compress)
    exit 0
}

if ($joined -match 'drag 100,200 100,200') {
    # Regression fixture for windows-ui-driver.ps1's coordinate-mode `point-click` (Issue #236
    # delta contract Section 4.1): proves the pre-existing zero-distance-drag compatibility route
    # is unchanged by adding selector mode.
    $verb = if ($FakeArgs.Count -ge 2) { $FakeArgs[1] } else { $null }
    Write-Output (@{ success = $true; verb = $verb; from = '100,200'; to = '100,200' } | ConvertTo-Json -Compress)
    exit 0
}

if ($joined -match 'FAKE_NO_INTERACTIVE_DESKTOP') {
    $body = '{"success":false,"error":{"code":"no_interactive_desktop","message":"the session desktop is locked or non-interactive"}}'
    [Console]::Error.WriteLine($body)
    exit 1
}

if ($joined -match 'FAKE_FOREGROUND_NOT_TARGET') {
    $body = '{"success":false,"error":{"code":"foreground_not_target","message":"could not bring the target window to the foreground"}}'
    [Console]::Error.WriteLine($body)
    exit 1
}

if ($joined -match 'FAKE_UNKNOWN_ERROR') {
    [Console]::Error.WriteLine('a completely unrecognized backend failure occurred')
    exit 1
}

if ($joined -match 'FAKE_WAIT_TIMEOUT') {
    # Mirrors winapp's own documented contract: search/wait-for still write a fully parseable
    # result envelope to stdout on a "not found"/timeout outcome, with stderr empty, and exit 1.
    Write-Output '{"matchCount":0,"hasMore":false,"matches":[]}'
    exit 1
}

if ($joined -match 'FAKE_LARGE_STDERR') {
    # Regression fixture for windows-ui-driver.ps1's own Invoke-WinApp: writes a small parseable
    # stdout body, then >= 256 KiB to stderr before exiting -- enough to exceed an ordinary OS pipe
    # buffer, reproducing the deadlock class a sequential stdout-then-stderr ReadToEnd() is
    # vulnerable to (blocked writing stderr while the caller is still waiting for stdout EOF).
    Write-Output '{"success":false,"error":{"code":"element_not_found","message":"large stderr regression"}}'
    [Console]::Error.Write(('E' * (256 * 1024)))
    exit 1
}

# Default: unrecognized scenario -- fail loudly so a broken test case is visible, not silently
# treated as one of the above.
[Console]::Error.WriteLine("fake-winapp.ps1: no matching fake scenario for args: $joined")
exit 2
