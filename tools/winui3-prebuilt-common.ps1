# Shared helpers for the WinUI3 prebuilt native artifact tooling (Issue #294).
# Dot-source only; defines functions and constants, changes no state.

$ErrorActionPreference = "Stop"
# MSVC tool output (cl banner, dumpbin) is parsed below; force English regardless of VS language.
$env:VSLANG = "1033"

$script:Winui3RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$script:Winui3CrateRelative = "crates/elwindui-backend-winui3"
$script:Winui3TrackedPrebuiltRelative = "crates/elwindui-backend-winui3/native/prebuilt"
$script:Winui3ArtifactFiles = @(
    "elwindui_winui3_app_host.lib",
    "Elwindui.WinUI3.Accessibility.winmd",
    "resources.pri"
)
$script:Winui3AbiAnchor = "elwindui_winui3_native_abi_v1"

# Exact Cargo TARGET <-> native architecture <-> GitHub-hosted runner mapping.
$script:Winui3Targets = [ordered]@{
    "x86_64-pc-windows-msvc"  = [ordered]@{ Arch = "x64";   Machine = "x64";   Runner = "windows-2025-vs2026" }
    "aarch64-pc-windows-msvc" = [ordered]@{ Arch = "arm64"; Machine = "ARM64"; Runner = "windows-11-vs2026-arm" }
}

# Native generation inputs hashed into `source_inputs_sha256`. Sorted ordinally when hashed.
$script:Winui3SourceInputs = @(
    ".github/workflows/winui3-prebuilt.yml",
    "crates/elwindui-backend-winui3/build.rs",
    "crates/elwindui-backend-winui3/build_support.rs",
    "crates/elwindui-backend-winui3/cpp/accessibility_host.cpp",
    "crates/elwindui-backend-winui3/cpp/accessibility_host.h",
    "crates/elwindui-backend-winui3/cpp/accessibility_semantic_peer.idl",
    "crates/elwindui-backend-winui3/cpp/app_host.cpp",
    "tools/assemble-winui3-prebuilt.ps1",
    "tools/build-winui3-prebuilt.ps1",
    "tools/restore-winui3.ps1",
    "tools/setup-vs-env.ps1",
    "tools/winui3-prebuilt-common.ps1"
)

function Get-Winui3TargetForArch([string]$Arch) {
    foreach ($target in $script:Winui3Targets.Keys) {
        if ($script:Winui3Targets[$target].Arch -eq $Arch) { return $target }
    }
    throw "Unsupported -Arch '$Arch'. Supported values are x64 and arm64."
}

function Get-Winui3Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-Winui3Sha256OfBytes([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        ([BitConverter]::ToString($sha.ComputeHash($Bytes)) -replace "-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

# SHA-256 over UTF-8 LF lines "<repo-relative path> <sha256 of raw file bytes>\n", ordinally
# sorted by path. Identical for the same checkout on every runner.
function Get-Winui3SourceInputsSha256([string]$RepoRoot = $script:Winui3RepoRoot) {
    $paths = [string[]]$script:Winui3SourceInputs.Clone()
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $builder = New-Object Text.StringBuilder
    foreach ($relative in $paths) {
        $full = Join-Path $RepoRoot $relative
        if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
            throw "Native generation input is missing: $relative"
        }
        [void]$builder.Append("$relative $(Get-Winui3Sha256 $full)`n")
    }
    Get-Winui3Sha256OfBytes ([Text.Encoding]::UTF8.GetBytes($builder.ToString()))
}

# NuGet pins declared by tools/restore-winui3.ps1 (the single source of the pinned versions).
function Get-Winui3DependencyPins([string]$RepoRoot = $script:Winui3RepoRoot) {
    $text = Get-Content -LiteralPath (Join-Path $RepoRoot "tools/restore-winui3.ps1") -Raw
    $pins = [ordered]@{}
    foreach ($entry in @(
            @("windows_app_sdk", "Microsoft.WindowsAppSDK"),
            @("windows_app_sdk_winui", "Microsoft.WindowsAppSDK.WinUI"),
            @("win2d", "Microsoft.Graphics.Win2D"))) {
        $pattern = 'PackageReference Include="' + [regex]::Escape($entry[1]) + '" Version="([^"]+)"'
        $match = [regex]::Match($text, $pattern)
        if (-not $match.Success) { throw "Pinned version of $($entry[1]) was not found in tools/restore-winui3.ps1" }
        $pins[$entry[0]] = $match.Groups[1].Value
    }
    $pins
}

# Native ABI generation expected by this checkout (`NATIVE_ABI_VERSION` in build_support.rs).
function Get-Winui3ExpectedNativeAbi([string]$RepoRoot = $script:Winui3RepoRoot) {
    $text = Get-Content -LiteralPath (Join-Path $RepoRoot "$script:Winui3CrateRelative/build_support.rs") -Raw
    $match = [regex]::Match($text, 'pub const NATIVE_ABI_VERSION: u32 = (\d+);')
    if (-not $match.Success) { throw "NATIVE_ABI_VERSION was not found in build_support.rs" }
    $abi = [int]$match.Groups[1].Value
    $cpp = Get-Content -LiteralPath (Join-Path $RepoRoot "$script:Winui3CrateRelative/cpp/app_host.cpp") -Raw
    if ($cpp -notmatch "elwindui_winui3_native_abi_v$abi\(\)") {
        throw "cpp/app_host.cpp does not export elwindui_winui3_native_abi_v$abi"
    }
    $abi
}

function Find-Winui3Dumpbin {
    $command = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vs = & $vswhere -latest -products * -property installationPath
        if ($vs) {
            $candidates = Get-ChildItem -Path (Join-Path $vs "VC\Tools\MSVC\*\bin\*\*\dumpbin.exe") -ErrorAction SilentlyContinue |
                Sort-Object FullName
            if ($candidates) { return @($candidates)[-1].FullName }
        }
    }
    $null
}

# Returns the distinct COFF machine names ("x64", "ARM64", ...) of an object library or PE image.
function Get-Winui3CoffMachines([string]$Dumpbin, [string]$Path) {
    $output = & $Dumpbin /nologo /headers $Path 2>&1
    if ($LASTEXITCODE -ne 0) { throw "dumpbin /headers failed for $Path" }
    $machines = @($output | ForEach-Object {
            if ($_ -match '^\s*[0-9A-Fa-f]+ machine \(([^)]+)\)') { $matches[1] }
        } | Sort-Object -Unique)
    if ($machines.Count -eq 0) { throw "dumpbin reported no COFF machine for $Path" }
    $machines
}

function Assert-Winui3CoffMachine([string]$Dumpbin, [string]$Path, [string]$Expected) {
    $machines = @(Get-Winui3CoffMachines $Dumpbin $Path)
    if ($machines.Count -ne 1 -or $machines[0] -ne $Expected) {
        throw "$Path has COFF machine(s) '$($machines -join ', ')'; expected only '$Expected'"
    }
    Write-Host "COFF machine OK: $Path -> $Expected"
}

function Assert-Winui3LibExportsAnchor([string]$Dumpbin, [string]$LibPath, [int]$Abi) {
    $symbol = "elwindui_winui3_native_abi_v$Abi"
    $output = & $Dumpbin /nologo /linkermember:1 $LibPath 2>&1
    if ($LASTEXITCODE -ne 0) { throw "dumpbin /linkermember failed for $LibPath" }
    if (-not ($output | Where-Object { $_ -match "\s$([regex]::Escape($symbol))\s*$" })) {
        throw "$LibPath does not define the native ABI anchor $symbol"
    }
    Write-Host "ABI anchor OK: $LibPath defines $symbol"
}

function Write-Winui3Json([object]$Value, [string]$Path) {
    $json = $Value | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText($Path, ($json -replace "`r`n", "`n") + "`n", (New-Object Text.UTF8Encoding $false))
}

function Read-Winui3Json([string]$Path) {
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Test-Winui3PathInside([string]$Child, [string]$Parent) {
    $c = [IO.Path]::GetFullPath($Child).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $p = [IO.Path]::GetFullPath($Parent).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $c.StartsWith($p, [StringComparison]::OrdinalIgnoreCase)
}
