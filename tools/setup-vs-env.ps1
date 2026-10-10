param(
    # Native target/host architecture. x64 (default) and arm64 only; the host must be the same
    # architecture (no cross-compilation, no x64 emulation on ARM64).
    [string]$Arch = "x64"
)

$ErrorActionPreference = "Stop"

switch ($Arch) {
    "x64" {
        $vsComponent = "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"
        $expectedOsArch = "X64"
    }
    "arm64" {
        $vsComponent = "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
        $expectedOsArch = "Arm64"
    }
    default {
        throw "Unsupported -Arch '$Arch'. Supported values are x64 and arm64 (x86 and ARM64EC are not supported)."
    }
}

$osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$processArch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
if ($osArch -ne $expectedOsArch) {
    throw "-Arch $Arch requires a native $expectedOsArch Windows host; this host is $osArch."
}
if ($processArch -ne $expectedOsArch) {
    throw "-Arch $Arch requires a native $expectedOsArch PowerShell process; this process is $processArch (emulated)."
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} `
    "Microsoft Visual Studio\Installer\vswhere.exe"

if (-not (Test-Path $vswhere)) {
    throw "vswhere.exe was not found: $vswhere"
}

$vsPath = & $vswhere `
    -latest `
    -products * `
    -requires $vsComponent `
    -property installationPath

if (-not $vsPath) {
    throw "Visual Studio C++ Build Tools with component $vsComponent were not found."
}

$vsDevCmd = Join-Path $vsPath "Common7\Tools\VsDevCmd.bat"

if (-not (Test-Path $vsDevCmd)) {
    throw "VsDevCmd.bat was not found: $vsDevCmd"
}

$environment = & cmd.exe /d /s /c `
    "`"$vsDevCmd`" -arch=$Arch -host_arch=$Arch >nul && set"

if ($LASTEXITCODE -ne 0) {
    throw "VsDevCmd.bat failed with exit code $LASTEXITCODE."
}

foreach ($line in $environment) {
    if ($line -match '^([^=]+)=(.*)$') {
        [Environment]::SetEnvironmentVariable(
            $matches[1],
            $matches[2],
            [EnvironmentVariableTarget]::Process
        )
    }
}

$hostDir = if ($Arch -eq "x64") { "Hostx64" } else { "Hostarm64" }
$cl = Join-Path $env:VCToolsInstallDir "bin\$hostDir\$Arch\cl.exe"
if (-not (Test-Path $cl)) {
    throw "Native $Arch-hosted $Arch-targeting cl.exe was not found: $cl"
}

Write-Host "Visual Studio build environment initialized."
Write-Host "Architecture:      $Arch (host $osArch)"
Write-Host "VCToolsInstallDir: $env:VCToolsInstallDir"
Write-Host "VCToolsVersion:    $env:VCToolsVersion"
Write-Host "cl.exe:            $cl"
Write-Host "WindowsSdkDir:     $env:WindowsSdkDir"
Write-Host "WindowsSDKVersion: $env:WindowsSDKVersion"

. (Join-Path $PSScriptRoot "restore-winui3.ps1")
Write-Host "WinUI 3 / Win2D NuGet packages: $env:NUGET_PACKAGES"
