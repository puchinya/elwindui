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

# Default: unrecognized scenario -- fail loudly so a broken test case is visible, not silently
# treated as one of the above.
[Console]::Error.WriteLine("fake-winapp.ps1: no matching fake scenario for args: $joined")
exit 2
