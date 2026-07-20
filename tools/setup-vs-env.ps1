$ErrorActionPreference = "Stop"

$vswhere = Join-Path ${env:ProgramFiles(x86)} `
    "Microsoft Visual Studio\Installer\vswhere.exe"

if (-not (Test-Path $vswhere)) {
    throw "vswhere.exe was not found: $vswhere"
}

$vsPath = & $vswhere `
    -latest `
    -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationPath

if (-not $vsPath) {
    throw "Visual Studio C++ Build Tools were not found."
}

$vsDevCmd = Join-Path $vsPath "Common7\Tools\VsDevCmd.bat"

if (-not (Test-Path $vsDevCmd)) {
    throw "VsDevCmd.bat was not found: $vsDevCmd"
}

$environment = & cmd.exe /d /s /c `
    "`"$vsDevCmd`" -arch=x64 -host_arch=x64 >nul && set"

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

Write-Host "Visual Studio build environment initialized."
Write-Host "VCToolsInstallDir: $env:VCToolsInstallDir"
Write-Host "WindowsSdkDir:     $env:WindowsSdkDir"
Write-Host "WindowsSDKVersion: $env:WindowsSDKVersion"

. (Join-Path $PSScriptRoot "restore-winui3.ps1")
Write-Host "WinUI 3 / Win2D NuGet packages: $env:NUGET_PACKAGES"
