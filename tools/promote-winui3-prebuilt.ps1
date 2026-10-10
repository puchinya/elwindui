# Transactionally replaces the tracked WinUI3 prebuilt bundle
# (crates/elwindui-backend-winui3/native/prebuilt) with a fully verified, CI-manufactured candidate
# whose native-input digest matches this checkout (Issue #294).
#
#   gh run download <run-id> -n winui3-prebuilt-verified -D .build\winui3-prebuilt-candidate
#   .\tools\promote-winui3-prebuilt.ps1 -BundleRoot .build\winui3-prebuilt-candidate
#
# Stages next to the tracked tree, keeps the previous tree as a backup until the promoted tree
# verifies, and restores it on any failure. Never commits, pushes or opens a PR.
param(
    [Parameter(Mandatory = $true)][string]$BundleRoot,
    # Tests only: throw at the named stage to exercise the rollback path.
    [ValidateSet("", "after-stage", "after-swap")][string]$FaultInjection = ""
)

. (Join-Path $PSScriptRoot "winui3-prebuilt-common.ps1")

$repo = $script:Winui3RepoRoot
$tracked = Join-Path $repo $script:Winui3TrackedPrebuiltRelative
$parent = Split-Path -Parent $tracked
$BundleRoot = [IO.Path]::GetFullPath($BundleRoot)
if ((Test-Winui3PathInside $BundleRoot $tracked) -or (Test-Winui3PathInside $tracked $BundleRoot)) {
    throw "-BundleRoot must not point at or contain the tracked prebuilt tree: $tracked"
}

function Get-TreeHashes([string]$Dir) {
    $map = [ordered]@{}
    if (Test-Path -LiteralPath $Dir) {
        foreach ($file in @(Get-ChildItem -LiteralPath $Dir -Recurse -File -Force | Sort-Object FullName)) {
            $map[$file.FullName.Substring($Dir.Length).Replace('\', '/')] = Get-Winui3Sha256 $file.FullName
        }
    }
    $map
}

function Test-SameTree($Left, $Right) {
    if ($Left.Count -ne $Right.Count) { return $false }
    foreach ($key in $Left.Keys) {
        if (-not $Right.Contains($key) -or $Right[$key] -ne $Left[$key]) { return $false }
    }
    $true
}

$verify = Join-Path $PSScriptRoot "verify-winui3-prebuilt.ps1"
$requireCoff = [bool](Find-Winui3Dumpbin)
if (-not $requireCoff) { Write-Warning "dumpbin.exe not found; COFF machine inspection is skipped (it already ran in CI)." }

# 1. The candidate itself must be a complete, current, CI-manufactured bundle.
& $verify -Root $BundleRoot -RequireCoffMachine:$requireCoff

New-Item -ItemType Directory -Force -Path $parent | Out-Null
$id = [Guid]::NewGuid().ToString("N")
$staging = Join-Path $parent ".prebuilt-staging-$id"
$backup = Join-Path $parent ".prebuilt-backup-$id"
$hadTracked = Test-Path -LiteralPath $tracked
$before = Get-TreeHashes $tracked
$swapped = $false
try {
    # 2. Stage on the tracked tree's filesystem and re-verify the copy.
    Copy-Item -LiteralPath $BundleRoot -Destination $staging -Recurse
    & $verify -Root $staging -RequireCoffMachine:$requireCoff
    if ($FaultInjection -eq "after-stage") { throw "fault injection: after-stage" }

    # 3. Swap: tracked -> backup, staging -> tracked.
    if ($hadTracked) { Move-Item -LiteralPath $tracked -Destination $backup }
    $swapped = $true
    Move-Item -LiteralPath $staging -Destination $tracked
    if ($FaultInjection -eq "after-swap") { throw "fault injection: after-swap" }

    # 4. The promoted tracked tree must verify in place.
    & $verify -RequireCoffMachine:$requireCoff
} catch {
    $failure = $_
    if ($swapped) {
        if (Test-Path -LiteralPath $tracked) { Remove-Item -LiteralPath $tracked -Recurse -Force }
        if ($hadTracked) { Move-Item -LiteralPath $backup -Destination $tracked }
    }
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
    if (-not (Test-SameTree $before (Get-TreeHashes $tracked))) {
        throw "Promotion failed AND the previous bundle could not be restored byte-for-byte: $failure"
    }
    throw "Promotion failed; the previous tracked bundle was restored unchanged: $failure"
}
if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }

Write-Host "Promoted $BundleRoot into $tracked. Review 'git status' / 'git diff --stat' and commit through the Issue workflow."
