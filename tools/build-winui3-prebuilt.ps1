# Manufactures the WinUI3 prebuilt native artifacts for ONE native architecture from source
# (Issue #294). Run on a native host of that architecture (CI: .github/workflows/winui3-prebuilt.yml).
#
#   .\tools\build-winui3-prebuilt.ps1 -Arch x64   -StagingRoot .build\winui3-native-staging
#   .\tools\build-winui3-prebuilt.ps1 -Arch arm64 -StagingRoot .build\winui3-native-staging
#
# Produces <StagingRoot>/<TARGET>/{elwindui_winui3_app_host.lib, Elwindui.WinUI3.Accessibility.winmd,
# resources.pri} and <StagingRoot>/<TARGET>.provenance.json. It builds and final-links
# custom-controls-demo in source-native mode on the way. It never writes the tracked
# crates/elwindui-backend-winui3/native/prebuilt tree; on failure the partial staging is removed.
param(
    [Parameter(Mandatory = $true)][string]$Arch,
    [Parameter(Mandatory = $true)][string]$StagingRoot,
    [string]$RunnerLabel = "local",
    [string]$CargoTargetDir
)

. (Join-Path $PSScriptRoot "winui3-prebuilt-common.ps1")

$target = Get-Winui3TargetForArch $Arch
$repo = $script:Winui3RepoRoot
$tracked = Join-Path $repo $script:Winui3TrackedPrebuiltRelative
$StagingRoot = [IO.Path]::GetFullPath($StagingRoot)
if ((Test-Winui3PathInside $StagingRoot $tracked) -or (Test-Winui3PathInside $tracked $StagingRoot)) {
    throw "-StagingRoot must not point at or contain the tracked prebuilt tree: $tracked"
}
$finalDir = Join-Path $StagingRoot $target
$fragmentPath = Join-Path $StagingRoot "$target.provenance.json"
if ((Test-Path -LiteralPath $finalDir) -or (Test-Path -LiteralPath $fragmentPath)) {
    throw "Staging output for $target already exists under $StagingRoot; use a fresh staging root."
}

# Native Rust toolchain for exactly this target (no cross-compilation).
$rustcInfo = & rustc -vV
if ($LASTEXITCODE -ne 0) { throw "rustc -vV failed" }
$rustHost = ($rustcInfo | Where-Object { $_ -like "host: *" }) -replace "^host: ", ""
if ($rustHost -ne $target) { throw "rustc host is '$rustHost'; manufacturing $target requires a native $target toolchain" }
$rustcVersion = (& rustc --version).Trim()

# Native Visual Studio / Windows SDK environment (rejects non-native hosts and unsupported -Arch).
. (Join-Path $PSScriptRoot "setup-vs-env.ps1") -Arch $Arch

$clBanner = (& cmd.exe /d /c "cl.exe 2>&1") -join "`n"
if ($clBanner -notmatch "Version ([0-9.]+) for (\S+)") { throw "Could not read the cl.exe banner: $clBanner" }
$clVersion = $matches[1]
$clTarget = $matches[2]
if ($clTarget -ne $script:Winui3Targets[$target].Machine) {
    throw "cl.exe targets '$clTarget'; manufacturing $target requires '$($script:Winui3Targets[$target].Machine)'"
}
$sdkVersion = "$env:WindowsSDKVersion".TrimEnd('\')
$sdkBin = Join-Path $env:WindowsSdkDir "bin\$sdkVersion\$Arch"
foreach ($tool in @("midl.exe", "cppwinrt.exe", "makepri.exe")) {
    $path = Join-Path $sdkBin $tool
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required $Arch Windows SDK tool is missing: $path"
    }
    Write-Host "SDK tool: $path"
}

$dumpbin = Find-Winui3Dumpbin
if (-not $dumpbin) { throw "dumpbin.exe was not found in the Visual Studio environment" }
$abi = Get-Winui3ExpectedNativeAbi
$sourceInputsHash = Get-Winui3SourceInputsSha256
$sourceCommit = (& git -C $repo rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $sourceCommit -notmatch '^[0-9a-f]{40}$') { throw "Could not resolve the source commit" }
$pins = Get-Winui3DependencyPins
$trackedStatusBefore = (& git -C $repo status --porcelain --ignored -- $script:Winui3TrackedPrebuiltRelative) -join "`n"

$partial = Join-Path $StagingRoot (".partial-$target-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $partial | Out-Null
$saved = @{
    Native = $env:ELWINDUI_WINUI3_BUILD_NATIVE
    Export = $env:ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR
    TargetDir = $env:CARGO_TARGET_DIR
}
try {
    if (-not $CargoTargetDir) { $CargoTargetDir = Join-Path $repo ".build\winui3-prebuilt-$target" }
    $env:ELWINDUI_WINUI3_BUILD_NATIVE = "1"
    $env:ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR = $partial
    $env:CARGO_TARGET_DIR = $CargoTargetDir
    Push-Location $repo
    try {
        & cargo build -p custom-controls-demo --release --target $target
        if ($LASTEXITCODE -ne 0) { throw "source-native cargo build of custom-controls-demo for $target failed" }
    } finally {
        Pop-Location
    }

    $exported = Join-Path $partial $target
    $entries = @(Get-ChildItem -LiteralPath $partial -Force)
    if ($entries.Count -ne 1 -or $entries[0].Name -ne $target) {
        throw "Source build exported unexpected entries: $($entries.Name -join ', ')"
    }
    foreach ($file in $script:Winui3ArtifactFiles) {
        $path = Join-Path $exported $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -eq 0) {
            throw "Source build did not export a non-empty $file for $target"
        }
    }
    $lib = Join-Path $exported "elwindui_winui3_app_host.lib"
    Assert-Winui3CoffMachine $dumpbin $lib $script:Winui3Targets[$target].Machine
    Assert-Winui3LibExportsAnchor $dumpbin $lib $abi
    $exe = Join-Path $CargoTargetDir "$target\release\custom-controls-demo.exe"
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Source-mode link did not produce $exe" }
    Assert-Winui3CoffMachine $dumpbin $exe $script:Winui3Targets[$target].Machine

    $fragment = [ordered]@{
        schema_version = 1
        target = $target
        native_abi = $abi
        source_commit = $sourceCommit
        source_inputs_sha256 = $sourceInputsHash
        windows_app_sdk = $pins.windows_app_sdk
        windows_app_sdk_winui = $pins.windows_app_sdk_winui
        win2d = $pins.win2d
        runner_label = $RunnerLabel
        windows_sdk_version = $sdkVersion
        msvc_version = "$env:VCToolsVersion"
        cl_version = $clVersion
        rustc_version = $rustcVersion
        lib_sha256 = Get-Winui3Sha256 $lib
        winmd_sha256 = Get-Winui3Sha256 (Join-Path $exported "Elwindui.WinUI3.Accessibility.winmd")
        pri_sha256 = Get-Winui3Sha256 (Join-Path $exported "resources.pri")
    }

    $trackedStatusAfter = (& git -C $repo status --porcelain --ignored -- $script:Winui3TrackedPrebuiltRelative) -join "`n"
    if ($trackedStatusAfter -ne $trackedStatusBefore) { throw "The tracked prebuilt tree changed during manufacturing" }
    if ((Get-Winui3SourceInputsSha256) -ne $sourceInputsHash) { throw "Native generation inputs changed during manufacturing" }

    Move-Item -LiteralPath $exported -Destination $finalDir
    Write-Winui3Json $fragment $fragmentPath
} catch {
    if (Test-Path -LiteralPath $finalDir) { Remove-Item -LiteralPath $finalDir -Recurse -Force }
    if (Test-Path -LiteralPath $fragmentPath) { Remove-Item -LiteralPath $fragmentPath -Force }
    throw
} finally {
    if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Recurse -Force }
    $env:ELWINDUI_WINUI3_BUILD_NATIVE = $saved.Native
    $env:ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR = $saved.Export
    $env:CARGO_TARGET_DIR = $saved.TargetDir
}

Write-Host "Manufactured $target into $finalDir"
Get-Content -LiteralPath $fragmentPath
