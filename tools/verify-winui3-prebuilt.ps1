# Verifies a WinUI3 prebuilt native bundle (Issue #294): completeness, manifest provenance, artifact
# hashes, native ABI generation and freshness against this checkout's native generation inputs.
#
#   .\tools\verify-winui3-prebuilt.ps1                       # tracked native/prebuilt tree
#   .\tools\verify-winui3-prebuilt.ps1 -Root <candidate>     # explicit candidate bundle
#   .\tools\verify-winui3-prebuilt.ps1 -RequireCoffMachine   # also inspect COFF machine + ABI symbol
#
# A bundle must come from the GitHub-hosted runners (exact runner labels and a `ci` provenance
# block) unless -AllowLocal is given for diagnosis/tests.
#
# Read-only. Exits non-zero (throws) on the first failure.
param(
    [string]$Root,
    [switch]$RequireCoffMachine,
    [switch]$AllowLocal
)

. (Join-Path $PSScriptRoot "winui3-prebuilt-common.ps1")

if (-not $Root) { $Root = Join-Path $script:Winui3RepoRoot $script:Winui3TrackedPrebuiltRelative }
$Root = [IO.Path]::GetFullPath($Root)
if (-not (Test-Path -LiteralPath $Root -PathType Container)) { throw "Prebuilt bundle root does not exist: $Root" }

$expectedTargets = @($script:Winui3Targets.Keys)

# Exactly manifest.json plus one directory per supported target; anything else would be published
# silently.
$entries = @(Get-ChildItem -LiteralPath $Root -Force)
foreach ($entry in $entries) {
    $allowed = ($entry.Name -eq "manifest.json" -and -not $entry.PSIsContainer) -or
        ($entry.PSIsContainer -and $expectedTargets -contains $entry.Name)
    if (-not $allowed) { throw "Unexpected entry in prebuilt bundle: $($entry.FullName)" }
}
$manifestPath = Join-Path $Root "manifest.json"
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw "manifest.json is missing: $manifestPath" }

foreach ($target in $expectedTargets) {
    $dir = Join-Path $Root $target
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { throw "Target directory is missing: $dir" }
    foreach ($item in @(Get-ChildItem -LiteralPath $dir -Force)) {
        if ($item.PSIsContainer -or $script:Winui3ArtifactFiles -notcontains $item.Name) {
            throw "Unexpected entry in $target bundle: $($item.FullName)"
        }
    }
    foreach ($file in $script:Winui3ArtifactFiles) {
        $path = Join-Path $dir $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Artifact is missing: $path" }
        if ((Get-Item -LiteralPath $path).Length -eq 0) { throw "Artifact is empty: $path" }
    }
}

$manifest = Read-Winui3Json $manifestPath
if ($manifest.schema_version -ne 1) { throw "manifest schema_version is '$($manifest.schema_version)'; expected 1" }
$abi = Get-Winui3ExpectedNativeAbi
if ($manifest.native_abi -ne $abi) { throw "manifest native_abi is '$($manifest.native_abi)'; this checkout expects $abi" }
if ("$($manifest.source_commit)" -notmatch '^[0-9a-f]{40}$') { throw "manifest source_commit is not a full commit SHA: '$($manifest.source_commit)'" }
$pins = Get-Winui3DependencyPins
foreach ($key in $pins.Keys) {
    if ($manifest.$key -ne $pins[$key]) { throw "manifest $key is '$($manifest.$key)'; tools/restore-winui3.ps1 pins '$($pins[$key])'" }
}
$sourceHash = Get-Winui3SourceInputsSha256
if ($manifest.source_inputs_sha256 -ne $sourceHash) {
    throw "Prebuilt bundle is stale: manifest source_inputs_sha256 $($manifest.source_inputs_sha256) != current native generation inputs $sourceHash. Regenerate through .github/workflows/winui3-prebuilt.yml."
}

if (-not $AllowLocal) {
    if (-not $manifest.ci -or "$($manifest.ci.run_url)" -notmatch '^https://\S+/actions/runs/\d+$') {
        throw "manifest has no GitHub Actions provenance (ci.run_url); only CI-manufactured bundles are accepted"
    }
}

$manifestTargets = @($manifest.targets.PSObject.Properties.Name)
foreach ($name in $manifestTargets) {
    if ($expectedTargets -notcontains $name) { throw "manifest lists unsupported target $name" }
}
$dumpbin = $null
if ($RequireCoffMachine) {
    $dumpbin = Find-Winui3Dumpbin
    if (-not $dumpbin) { throw "-RequireCoffMachine needs dumpbin.exe (Visual Studio C++ tools)" }
}
foreach ($target in $expectedTargets) {
    $entry = $manifest.targets.$target
    if (-not $entry) { throw "manifest is missing target $target" }
    foreach ($field in @("runner_label", "windows_sdk_version", "msvc_version", "rustc_version")) {
        if (-not "$($entry.$field)") { throw "manifest $target.$field is empty" }
    }
    if (-not $AllowLocal -and $entry.runner_label -ne $script:Winui3Targets[$target].Runner) {
        throw "manifest $target.runner_label is '$($entry.runner_label)'; expected $($script:Winui3Targets[$target].Runner)"
    }
    $dir = Join-Path $Root $target
    foreach ($pair in @(
            @("lib_sha256", "elwindui_winui3_app_host.lib"),
            @("winmd_sha256", "Elwindui.WinUI3.Accessibility.winmd"),
            @("pri_sha256", "resources.pri"))) {
        $actual = Get-Winui3Sha256 (Join-Path $dir $pair[1])
        if ($entry.($pair[0]) -ne $actual) {
            throw "Artifact hash mismatch for $target/$($pair[1]): manifest $($entry.($pair[0])), actual $actual"
        }
    }
    if ($RequireCoffMachine) {
        $lib = Join-Path $dir "elwindui_winui3_app_host.lib"
        Assert-Winui3CoffMachine $dumpbin $lib $script:Winui3Targets[$target].Machine
        Assert-Winui3LibExportsAnchor $dumpbin $lib $abi
    }
}

Write-Host "WinUI3 prebuilt bundle verified: $Root"
Write-Host "  source_commit:        $($manifest.source_commit)"
Write-Host "  source_inputs_sha256: $sourceHash"
Write-Host "  native_abi:           $abi"
foreach ($target in $expectedTargets) {
    $entry = $manifest.targets.$target
    Write-Host "  ${target}: lib $($entry.lib_sha256) winmd $($entry.winmd_sha256) pri $($entry.pri_sha256)"
}
