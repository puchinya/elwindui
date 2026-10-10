# Assembles one WinUI3 prebuilt candidate bundle from the per-target outputs of
# tools/build-winui3-prebuilt.ps1 (Issue #294), then fully verifies it.
#
#   .\tools\assemble-winui3-prebuilt.ps1 -InputRoot <downloaded artifacts> -OutputRoot <new dir>
#
# InputRoot is searched recursively for <TARGET>.provenance.json fragments, each with a sibling
# <TARGET>/ directory (GitHub Actions downloads every artifact into its own subdirectory). Exactly one
# fragment per supported target is required, all from the same source commit and native-input digest
# as this checkout. OutputRoot must not exist; it is only created once every check passed. Never
# touches the tracked native/prebuilt tree.
param(
    [Parameter(Mandatory = $true)][string]$InputRoot,
    [Parameter(Mandatory = $true)][string]$OutputRoot,
    [switch]$RequireCoffMachine,
    # Diagnosis/tests only: accept fragments manufactured outside the GitHub-hosted runners. Such a
    # bundle carries no CI provenance and is rejected by tools/promote-winui3-prebuilt.ps1.
    [switch]$AllowLocal
)

. (Join-Path $PSScriptRoot "winui3-prebuilt-common.ps1")

$repo = $script:Winui3RepoRoot
$tracked = Join-Path $repo $script:Winui3TrackedPrebuiltRelative
$InputRoot = [IO.Path]::GetFullPath($InputRoot)
$OutputRoot = [IO.Path]::GetFullPath($OutputRoot)
if ((Test-Winui3PathInside $OutputRoot $tracked) -or (Test-Winui3PathInside $tracked $OutputRoot)) {
    throw "-OutputRoot must not point at or contain the tracked prebuilt tree: $tracked"
}
if (Test-Path -LiteralPath $OutputRoot) { throw "-OutputRoot already exists: $OutputRoot" }
if (-not (Test-Path -LiteralPath $InputRoot -PathType Container)) { throw "-InputRoot does not exist: $InputRoot" }

$abi = Get-Winui3ExpectedNativeAbi
$pins = Get-Winui3DependencyPins
$sourceInputsHash = Get-Winui3SourceInputsSha256
$expectedTargets = @($script:Winui3Targets.Keys)

$fragments = @(Get-ChildItem -LiteralPath $InputRoot -Recurse -File -Filter "*.provenance.json")
$byTarget = [ordered]@{}
foreach ($file in $fragments) {
    $target = $file.Name.Substring(0, $file.Name.Length - ".provenance.json".Length)
    if ($expectedTargets -notcontains $target) { throw "Unsupported target fragment: $($file.FullName)" }
    if ($byTarget.Contains($target)) { throw "Duplicate fragment for ${target}: $($byTarget[$target].FullName) and $($file.FullName)" }
    $byTarget[$target] = $file
}
foreach ($target in $expectedTargets) {
    if (-not $byTarget.Contains($target)) { throw "Missing fragment for $target under $InputRoot" }
}

$records = [ordered]@{}
$commit = $null
foreach ($target in $expectedTargets) {
    $file = $byTarget[$target]
    $fragment = Read-Winui3Json $file.FullName
    $dir = Join-Path $file.DirectoryName $target
    if ($fragment.schema_version -ne 1) { throw "${target}: fragment schema_version '$($fragment.schema_version)' is not 1" }
    if ($fragment.target -ne $target) { throw "${target}: fragment declares target '$($fragment.target)'" }
    if ($fragment.native_abi -ne $abi) { throw "${target}: fragment native_abi '$($fragment.native_abi)' != expected $abi" }
    if ("$($fragment.source_commit)" -notmatch '^[0-9a-f]{40}$') { throw "${target}: invalid source_commit '$($fragment.source_commit)'" }
    if ($null -eq $commit) { $commit = $fragment.source_commit }
    elseif ($fragment.source_commit -ne $commit) { throw "Fragments come from different source commits: $commit and $($fragment.source_commit) ($target)" }
    if ($fragment.source_inputs_sha256 -ne $sourceInputsHash) {
        throw "${target}: fragment source_inputs_sha256 $($fragment.source_inputs_sha256) != this checkout's $sourceInputsHash"
    }
    foreach ($key in $pins.Keys) {
        if ($fragment.$key -ne $pins[$key]) { throw "${target}: fragment $key '$($fragment.$key)' != pinned '$($pins[$key])'" }
    }
    $expectedRunner = $script:Winui3Targets[$target].Runner
    foreach ($field in @("runner_label", "windows_sdk_version", "msvc_version", "rustc_version")) {
        if (-not "$($fragment.$field)") { throw "${target}: fragment $field is empty" }
    }
    if ($fragment.runner_label -ne $expectedRunner -and -not $AllowLocal) {
        throw "${target}: fragment runner_label '$($fragment.runner_label)' is not '$expectedRunner'"
    }
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { throw "${target}: artifact directory is missing: $dir" }
    foreach ($item in @(Get-ChildItem -LiteralPath $dir -Force)) {
        if ($item.PSIsContainer -or $script:Winui3ArtifactFiles -notcontains $item.Name) {
            throw "${target}: unexpected artifact entry $($item.FullName)"
        }
    }
    foreach ($pair in @(
            @("lib_sha256", "elwindui_winui3_app_host.lib"),
            @("winmd_sha256", "Elwindui.WinUI3.Accessibility.winmd"),
            @("pri_sha256", "resources.pri"))) {
        $path = Join-Path $dir $pair[1]
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -eq 0) {
            throw "${target}: artifact $($pair[1]) is missing or empty"
        }
        $actual = Get-Winui3Sha256 $path
        if ($fragment.($pair[0]) -ne $actual) { throw "${target}: $($pair[1]) hash $actual != fragment $($fragment.($pair[0]))" }
    }
    $records[$target] = [ordered]@{
        Dir = $dir
        Entry = [ordered]@{
            runner_label = $fragment.runner_label
            windows_sdk_version = $fragment.windows_sdk_version
            msvc_version = $fragment.msvc_version
            rustc_version = $fragment.rustc_version
            lib_sha256 = $fragment.lib_sha256
            winmd_sha256 = $fragment.winmd_sha256
            pri_sha256 = $fragment.pri_sha256
        }
    }
}

$manifest = [ordered]@{
    schema_version = 1
    native_abi = $abi
    source_commit = $commit
    source_inputs_sha256 = $sourceInputsHash
    windows_app_sdk = $pins.windows_app_sdk
    windows_app_sdk_winui = $pins.windows_app_sdk_winui
    win2d = $pins.win2d
    targets = [ordered]@{}
}
if ($env:GITHUB_RUN_ID) {
    $manifest["ci"] = [ordered]@{
        repository = $env:GITHUB_REPOSITORY
        workflow_ref = $env:GITHUB_WORKFLOW_REF
        event = $env:GITHUB_EVENT_NAME
        run_id = $env:GITHUB_RUN_ID
        run_attempt = $env:GITHUB_RUN_ATTEMPT
        run_url = "$env:GITHUB_SERVER_URL/$env:GITHUB_REPOSITORY/actions/runs/$env:GITHUB_RUN_ID"
    }
}
foreach ($target in $expectedTargets) { $manifest.targets[$target] = $records[$target].Entry }

$partial = "$OutputRoot.partial-" + [Guid]::NewGuid().ToString("N")
try {
    New-Item -ItemType Directory -Force -Path $partial | Out-Null
    foreach ($target in $expectedTargets) {
        $destination = Join-Path $partial $target
        New-Item -ItemType Directory -Force -Path $destination | Out-Null
        foreach ($file in $script:Winui3ArtifactFiles) {
            Copy-Item -LiteralPath (Join-Path $records[$target].Dir $file) -Destination (Join-Path $destination $file)
        }
    }
    Write-Winui3Json $manifest (Join-Path $partial "manifest.json")
    & (Join-Path $PSScriptRoot "verify-winui3-prebuilt.ps1") -Root $partial -RequireCoffMachine:$RequireCoffMachine -AllowLocal:$AllowLocal
    Move-Item -LiteralPath $partial -Destination $OutputRoot
} catch {
    if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Recurse -Force }
    throw
}

Write-Host "Assembled WinUI3 prebuilt candidate: $OutputRoot"
