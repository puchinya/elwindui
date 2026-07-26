[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateRange(0, 2147483647)]
    [int] $IssueNumber = 0
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Command {
    param([Parameter(Mandatory = $true)][string] $Name)

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command not found: $Name"
    }
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string] $FilePath,
        [Parameter()][string[]] $Arguments = @()
    )

    $output = & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath $($Arguments -join ' ')"
    }

    return (($output | Out-String).Trim())
}

function Get-RootCargoVersion {
    param([Parameter(Mandatory = $true)][string] $CargoTomlPath)

    $currentSection = ''
    $workspaceVersion = $null
    $packageVersion = $null

    foreach ($rawLine in Get-Content -LiteralPath $CargoTomlPath -Encoding UTF8) {
        $line = $rawLine.Trim()

        if ($line -match '^\[(?<section>[^\]]+)\]\s*(?:#.*)?$') {
            $currentSection = $Matches['section'].Trim()
            continue
        }

        $value = $null

        if ($line -match '^\s*version\s*=\s*"(?<value>(?:[^"\\]|\\.)*)"\s*(?:#.*)?$') {
            $value = [regex]::Unescape($Matches['value'])
        }
        elseif ($line -match "^\s*version\s*=\s*'(?<value>[^']*)'\s*(?:#.*)?$") {
            $value = $Matches['value']
        }

        if ($null -eq $value) {
            continue
        }

        if ($currentSection -eq 'workspace.package') {
            $workspaceVersion = $value
        }
        elseif ($currentSection -eq 'package') {
            $packageVersion = $value
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($workspaceVersion)) {
        return $workspaceVersion
    }

    if (-not [string]::IsNullOrWhiteSpace($packageVersion)) {
        return $packageVersion
    }

    throw 'Root Cargo.toml has no string [workspace.package].version or [package].version.'
}

Assert-Command git
Assert-Command gh

$repoRoot = Invoke-Checked git @('rev-parse', '--show-toplevel')
if ([string]::IsNullOrWhiteSpace($repoRoot)) {
    throw 'Run this script inside a Git repository.'
}
Set-Location $repoRoot

& gh auth status *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'GitHub CLI is not authenticated. Run: gh auth login'
}

$cargoToml = Join-Path $repoRoot 'Cargo.toml'
if (-not (Test-Path -LiteralPath $cargoToml -PathType Leaf)) {
    throw 'Cargo.toml was not found at the Git repository root.'
}

$repository = Invoke-Checked gh @(
    'repo', 'view',
    '--json', 'nameWithOwner',
    '--jq', '.nameWithOwner'
)

$version = Get-RootCargoVersion -CargoTomlPath $cargoToml

$milestonesJson = Invoke-Checked gh @(
    'api',
    '--paginate',
    '--slurp',
    "repos/$repository/milestones?state=all&per_page=100"
)

$pages = $milestonesJson | ConvertFrom-Json
$milestones = @()

foreach ($page in @($pages)) {
    foreach ($milestone in @($page)) {
        $milestones += $milestone
    }
}

$matches = @(
    $milestones | Where-Object {
        $_.title -ceq $version
    }
)

if ($matches.Count -gt 1) {
    $numbers = ($matches | ForEach-Object { $_.number }) -join ', '
    throw "Duplicate GitHub Milestones named '$version': $numbers"
}

if ($matches.Count -eq 0) {
    $milestoneNumber = Invoke-Checked gh @(
        'api',
        '--method', 'POST',
        "repos/$repository/milestones",
        '-f', "title=$version",
        '--jq', '.number'
    )

    Write-Host "Created GitHub Milestone '$version' (#$milestoneNumber)."
}
else {
    $milestone = $matches[0]
    $milestoneNumber = [int] $milestone.number

    if ($milestone.state -ne 'open') {
        throw "GitHub Milestone '$version' (#$milestoneNumber) exists but is closed. Reopen it intentionally or update Cargo.toml after deciding the intended release."
    }

    Write-Host "Using GitHub Milestone '$version' (#$milestoneNumber)."
}

if ($IssueNumber -gt 0) {
    & gh issue edit $IssueNumber `
        --repo $repository `
        --milestone $version *> $null

    if ($LASTEXITCODE -ne 0) {
        throw "Failed to assign Issue #$IssueNumber to Milestone '$version'."
    }

    Write-Host "Assigned Issue #$IssueNumber to Milestone '$version'."
}

Write-Output $version
