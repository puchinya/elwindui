param([string]$PackagesDirectory = (Join-Path $PSScriptRoot '..\.nuget\packages'))

$ErrorActionPreference = 'Stop'
$packages = [IO.Path]::GetFullPath($PackagesDirectory)
# Windows App SDK 1.8's runtime package and its WinUI metadata use distinct patch labels, but
# this is the exact package pair declared by the official 1.8.260209005 runtime dependency.
$runtime = Join-Path $packages 'microsoft.windowsappsdk\1.8.260209005'
$appSdk = Join-Path $packages 'microsoft.windowsappsdk.winui\1.8.260204000\metadata\Microsoft.UI.Xaml.winmd'
$win2d = Join-Path $packages 'microsoft.graphics.win2d\1.4.0\lib\uap10.0\Microsoft.Graphics.Canvas.winmd'
if (-not ((Test-Path $runtime) -and (Test-Path $appSdk) -and (Test-Path $win2d))) {
    $project = Join-Path $env:TEMP 'elwindui-winui3-restore.csproj'
    @'
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><TargetFramework>net8.0-windows10.0.19041.0</TargetFramework></PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Microsoft.WindowsAppSDK" Version="1.8.260209005" />
    <PackageReference Include="Microsoft.WindowsAppSDK.WinUI" Version="1.8.260204000" />
    <PackageReference Include="Microsoft.Graphics.Win2D" Version="1.4.0" />
  </ItemGroup>
</Project>
'@ | Set-Content -LiteralPath $project -Encoding utf8
    & dotnet restore $project --packages $packages
    if ($LASTEXITCODE -ne 0) { throw 'WinUI 3 / Win2D NuGet restore failed.' }
}
$env:NUGET_PACKAGES = $packages
