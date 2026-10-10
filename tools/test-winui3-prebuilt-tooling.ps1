# Negative/transactionality checks for the WinUI3 prebuilt tooling (Issue #294) against temporary
# copies of a verified candidate bundle. The tracked native/prebuilt tree must stay byte-identical
# unless -ExercisePromotionSuccess is given (CI's disposable checkout only).
#
#   .\tools\test-winui3-prebuilt-tooling.ps1 -CandidateRoot <assembled candidate>
param(
    [Parameter(Mandatory = $true)][string]$CandidateRoot,
    [switch]$ExercisePromotionSuccess
)

. (Join-Path $PSScriptRoot "winui3-prebuilt-common.ps1")

$CandidateRoot = [IO.Path]::GetFullPath($CandidateRoot)
$repo = $script:Winui3RepoRoot
$tracked = Join-Path $repo $script:Winui3TrackedPrebuiltRelative
$verify = Join-Path $PSScriptRoot "verify-winui3-prebuilt.ps1"
$assemble = Join-Path $PSScriptRoot "assemble-winui3-prebuilt.ps1"
$promote = Join-Path $PSScriptRoot "promote-winui3-prebuilt.ps1"
$candidateManifest = Read-Winui3Json (Join-Path $CandidateRoot "manifest.json")
$isCi = [bool]$candidateManifest.ci
$work = Join-Path ([IO.Path]::GetTempPath()) ("winui3-prebuilt-tests-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null
$failures = New-Object Collections.Generic.List[string]
$passed = 0

function Get-TrackedHashes {
    $map = [ordered]@{}
    if (Test-Path -LiteralPath $tracked) {
        foreach ($file in @(Get-ChildItem -LiteralPath $tracked -Recurse -File -Force | Sort-Object FullName)) {
            $map[$file.FullName] = Get-Winui3Sha256 $file.FullName
        }
    }
    ($map.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join "`n"
}

function New-Copy([string]$Name) {
    $dir = Join-Path $work $Name
    Copy-Item -LiteralPath $CandidateRoot -Destination $dir -Recurse
    $dir
}

function Expect-Failure([string]$Case, [string]$Pattern, [scriptblock]$Action) {
    try {
        & $Action *> $null
    } catch {
        if ("$_" -match $Pattern) {
            Write-Host "PASS $Case -> rejected: $_"
            $script:passed++
        } else {
            $script:failures.Add("$Case failed with an unexpected error: $_")
        }
        return
    }
    $script:failures.Add("$Case was accepted but must be rejected")
}

function Expect-Success([string]$Case, [scriptblock]$Action) {
    try {
        & $Action *> $null
        Write-Host "PASS $Case"
        $script:passed++
    } catch {
        $script:failures.Add("$Case failed: $_")
    }
}

function Set-ManifestField([string]$Root, [scriptblock]$Mutate) {
    $path = Join-Path $Root "manifest.json"
    $manifest = Read-Winui3Json $path
    & $Mutate $manifest
    Write-Winui3Json $manifest $path
}

# Splits a bundle back into per-target fragments the way manufacture jobs upload them.
function New-FragmentInput([string]$Name, [string]$FromRoot = $CandidateRoot) {
    $root = Join-Path $work $Name
    $manifest = Read-Winui3Json (Join-Path $FromRoot "manifest.json")
    foreach ($target in @($script:Winui3Targets.Keys)) {
        $artifact = Join-Path $root "winui3-native-$($script:Winui3Targets[$target].Arch)"
        New-Item -ItemType Directory -Force -Path $artifact | Out-Null
        Copy-Item -LiteralPath (Join-Path $FromRoot $target) -Destination (Join-Path $artifact $target) -Recurse
        $entry = $manifest.targets.$target
        $fragment = [ordered]@{
            schema_version = 1
            target = $target
            native_abi = $manifest.native_abi
            source_commit = $manifest.source_commit
            source_inputs_sha256 = $manifest.source_inputs_sha256
            windows_app_sdk = $manifest.windows_app_sdk
            windows_app_sdk_winui = $manifest.windows_app_sdk_winui
            win2d = $manifest.win2d
            runner_label = $entry.runner_label
            windows_sdk_version = $entry.windows_sdk_version
            msvc_version = $entry.msvc_version
            cl_version = "test"
            rustc_version = $entry.rustc_version
            lib_sha256 = $entry.lib_sha256
            winmd_sha256 = $entry.winmd_sha256
            pri_sha256 = $entry.pri_sha256
        }
        Write-Winui3Json $fragment (Join-Path $artifact "$target.provenance.json")
    }
    $root
}

function Get-FragmentPath([string]$InputRoot, [string]$Target) {
    Join-Path (Join-Path $InputRoot "winui3-native-$($script:Winui3Targets[$Target].Arch)") "$Target.provenance.json"
}

$x64 = "x86_64-pc-windows-msvc"
$arm64 = "aarch64-pc-windows-msvc"
$trackedBefore = Get-TrackedHashes

try {
    # T7: a valid candidate verifies.
    Expect-Success "T7 candidate verifies" { & $verify -Root $CandidateRoot -AllowLocal:(-not $isCi) }

    # T8: tampering / incompleteness.
    $c = New-Copy "t8-byte"
    $lib = Join-Path $c "$arm64/elwindui_winui3_app_host.lib"
    $bytes = [IO.File]::ReadAllBytes($lib); $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 0xFF; [IO.File]::WriteAllBytes($lib, $bytes)
    Expect-Failure "T8 modified byte" "hash mismatch.*$arm64/elwindui_winui3_app_host.lib" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-delete"
    Remove-Item (Join-Path $c "$x64/resources.pri")
    Expect-Failure "T8 deleted artifact" "missing.*resources.pri" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-empty"
    [IO.File]::WriteAllBytes((Join-Path $c "$x64/Elwindui.WinUI3.Accessibility.winmd"), [byte[]]@())
    Expect-Failure "T8 empty artifact" "empty" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-extra-target"
    Copy-Item -LiteralPath (Join-Path $c $x64) -Destination (Join-Path $c "i686-pc-windows-msvc") -Recurse
    Expect-Failure "T8 unexpected target directory" "Unexpected entry" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-missing-target-dir"
    Remove-Item -Recurse (Join-Path $c $arm64)
    Expect-Failure "T8 missing target directory" "Target directory is missing" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-manifest-target"
    Set-ManifestField $c { param($m) $m.targets.PSObject.Properties.Remove($arm64) }
    Expect-Failure "T8 target missing from manifest" "missing target $arm64" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-abi"
    Set-ManifestField $c { param($m) $m.native_abi = 0 }
    Expect-Failure "T8 old native ABI generation" "native_abi" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-pin"
    Set-ManifestField $c { param($m) $m.win2d = "1.3.0" }
    Expect-Failure "T8 dependency pin mismatch" "win2d" { & $verify -Root $c -AllowLocal }
    $c = New-Copy "t8-no-ci"
    Set-ManifestField $c { param($m) $m.PSObject.Properties.Remove("ci") }
    Expect-Failure "T8 bundle without CI provenance" "GitHub Actions provenance" { & $verify -Root $c }

    # T9: a native input that differs from the manifest digest makes the bundle stale.
    $c = New-Copy "t9-stale"
    Set-ManifestField $c { param($m) $m.source_inputs_sha256 = ("0" * 64) }
    Expect-Failure "T9 stale source-input digest" "stale" { & $verify -Root $c -AllowLocal }
    $mirror = Join-Path $work "t9-inputs"
    foreach ($relative in $script:Winui3SourceInputs) {
        $destination = Join-Path $mirror $relative
        New-Item -ItemType Directory -Force -Path (Split-Path $destination) | Out-Null
        Copy-Item -LiteralPath (Join-Path $repo $relative) -Destination $destination
    }
    $same = Get-Winui3SourceInputsSha256 $mirror
    Add-Content -LiteralPath (Join-Path $mirror "crates/elwindui-backend-winui3/cpp/app_host.cpp") -Value "// changed"
    $changed = Get-Winui3SourceInputsSha256 $mirror
    if ($same -eq (Get-Winui3SourceInputsSha256) -and $changed -ne $same) {
        Write-Host "PASS T9 one changed native input changes source_inputs_sha256"; $passed++
    } else {
        $failures.Add("T9 source-input digest is not sensitive to native input changes")
    }

    # T13/T19: aggregation.
    $allowLocal = -not $isCi
    $in = New-FragmentInput "t13-valid"
    $out = Join-Path $work "t13-valid-out"
    Expect-Success "T13 consistent two-target fragments assemble" { & $assemble -InputRoot $in -OutputRoot $out -AllowLocal:$allowLocal }
    $in = New-FragmentInput "t13-missing"
    Remove-Item -Recurse (Join-Path $in "winui3-native-arm64")
    Expect-Failure "T13 missing architecture fragment" "Missing fragment for $arm64" { & $assemble -InputRoot $in -OutputRoot (Join-Path $work "o1") -AllowLocal }
    $in = New-FragmentInput "t19-duplicate"
    Copy-Item -Recurse (Join-Path $in "winui3-native-x64") (Join-Path $in "winui3-native-x64-again")
    Expect-Failure "T19 duplicate target fragment" "Duplicate fragment" { & $assemble -InputRoot $in -OutputRoot (Join-Path $work "o2") -AllowLocal }
    $in = New-FragmentInput "t19-commit"
    $f = Get-FragmentPath $in $arm64; $j = Read-Winui3Json $f; $j.source_commit = ("1" * 40); Write-Winui3Json $j $f
    Expect-Failure "T19 fragments from different source commits" "different source commits" { & $assemble -InputRoot $in -OutputRoot (Join-Path $work "o3") -AllowLocal }
    $in = New-FragmentInput "t19-digest"
    $f = Get-FragmentPath $in $x64; $j = Read-Winui3Json $f; $j.source_inputs_sha256 = ("2" * 64); Write-Winui3Json $j $f
    Expect-Failure "T19 fragment from different native inputs" "source_inputs_sha256" { & $assemble -InputRoot $in -OutputRoot (Join-Path $work "o4") -AllowLocal }
    $in = New-FragmentInput "t13-corrupt"
    $lib = Join-Path $in "winui3-native-x64/$x64/elwindui_winui3_app_host.lib"
    $bytes = [IO.File]::ReadAllBytes($lib); $bytes[100] = $bytes[100] -bxor 0xFF; [IO.File]::WriteAllBytes($lib, $bytes)
    Expect-Failure "T13 corrupted artifact" "hash" { & $assemble -InputRoot $in -OutputRoot (Join-Path $work "o5") -AllowLocal }
    $in = New-FragmentInput "t13-pin"
    $f = Get-FragmentPath $in $arm64; $j = Read-Winui3Json $f; $j.windows_app_sdk = "1.7.0"; Write-Winui3Json $j $f
    Expect-Failure "T13 dependency version mismatch" "windows_app_sdk" { & $assemble -InputRoot $in -OutputRoot (Join-Path $work "o6") -AllowLocal }
    # Architecture mismatch: the x64 library presented as the ARM64 artifact (hashes rewritten so
    # only the COFF machine check can catch it).
    if (Find-Winui3Dumpbin) {
        $in = New-FragmentInput "t13-arch"
        $armDir = Join-Path $in "winui3-native-arm64/$arm64"
        Copy-Item -Force (Join-Path $in "winui3-native-x64/$x64/elwindui_winui3_app_host.lib") (Join-Path $armDir "elwindui_winui3_app_host.lib")
        $f = Get-FragmentPath $in $arm64; $j = Read-Winui3Json $f; $j.lib_sha256 = Get-Winui3Sha256 (Join-Path $armDir "elwindui_winui3_app_host.lib"); Write-Winui3Json $j $f
        Expect-Failure "T13 architecture mismatch" "COFF machine" { & $assemble -InputRoot $in -OutputRoot (Join-Path $work "o7") -AllowLocal -RequireCoffMachine }
    } else {
        $failures.Add("T13 architecture mismatch needs dumpbin.exe")
    }
    foreach ($name in @("o1", "o2", "o3", "o4", "o5", "o6", "o7")) {
        if (Test-Path (Join-Path $work $name)) { $failures.Add("rejected aggregation still created output $name") }
    }

    # T13/T20: promotion is transactional and only accepts verified CI bundles.
    $c = New-Copy "t20-no-ci"
    Set-ManifestField $c { param($m) $m.PSObject.Properties.Remove("ci") }
    Expect-Failure "T20 promotion rejects a non-CI bundle" "GitHub Actions provenance" { & $promote -BundleRoot $c }
    $c = New-Copy "t20-manipulated"
    [IO.File]::AppendAllText((Join-Path $c "$x64/resources.pri"), "x")
    Expect-Failure "T20 promotion rejects a manipulated candidate" "hash mismatch" { & $promote -BundleRoot $c }
    if ($isCi) {
        Expect-Failure "T13 promotion fault after staging" "restored unchanged" { & $promote -BundleRoot $CandidateRoot -FaultInjection after-stage }
        Expect-Failure "T13 promotion fault after swap" "restored unchanged" { & $promote -BundleRoot $CandidateRoot -FaultInjection after-swap }
    } else {
        Write-Host "SKIP promotion fault tests: candidate has no CI provenance"
    }
    if ((Get-TrackedHashes) -ne $trackedBefore) { $failures.Add("the tracked prebuilt tree changed during negative tests") }
    else { Write-Host "PASS tracked prebuilt tree unchanged by rejected operations"; $passed++ }

    if ($ExercisePromotionSuccess -and $isCi) {
        Expect-Success "T20 verified CI candidate promotes" { & $promote -BundleRoot $CandidateRoot }
        Expect-Success "T20 promoted tree verifies" { & $verify -RequireCoffMachine }
    }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "passed: $passed"
if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Host "FAIL $_" }
    throw "$($failures.Count) prebuilt tooling check(s) failed"
}
