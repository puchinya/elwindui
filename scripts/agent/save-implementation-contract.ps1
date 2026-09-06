[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateRange(1, 2147483647)]
    [int] $IssueNumber,

    [Parameter(Position = 1)]
    [string] $ContractFile
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Require-Command([string] $Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command not found: $Name"
    }
}

Require-Command git
Require-Command gh

$root = (& git rev-parse --show-toplevel).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($root)) {
    throw 'Run inside a Git repository.'
}
Set-Location $root

& gh auth status *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'GitHub CLI is not authenticated. Run: gh auth login'
}

$repository = (& gh repo view --json nameWithOwner --jq '.nameWithOwner').Trim()
& gh issue view $IssueNumber --repo $repository *> $null
if ($LASTEXITCODE -ne 0) {
    throw "Issue #$IssueNumber does not exist or is not accessible."
}

$prepare = Join-Path $PSScriptRoot 'prepare-work-evidence.ps1'
& $prepare $IssueNumber *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'prepare-work-evidence.ps1 failed.'
}

$base = ".agent-state/issues/$IssueNumber"
$dest = Join-Path $base 'implementation-contract.md'
$shaFile = Join-Path $base 'implementation-contract.sha256'
$temp = [System.IO.Path]::GetTempFileName()

try {
    if (-not [string]::IsNullOrWhiteSpace($ContractFile)) {
        if (-not (Test-Path -LiteralPath $ContractFile -PathType Leaf)) {
            throw "Contract file not found: $ContractFile"
        }
        [System.IO.File]::WriteAllBytes($temp, [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $ContractFile)))
    }
    else {
        $text = [Console]::In.ReadToEnd()
        if ([string]::IsNullOrEmpty($text)) {
            throw 'Contract input is empty.'
        }
        [System.IO.File]::WriteAllText($temp, $text, [System.Text.UTF8Encoding]::new($false))
    }

    $bytes = [System.IO.File]::ReadAllBytes($temp)
    $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    [void] $utf8.GetString($bytes)

    $newSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $temp).Hash.ToLowerInvariant()

    if (Test-Path -LiteralPath $dest) {
        $existingSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $dest).Hash.ToLowerInvariant()
        if ($existingSha -ne $newSha) {
            throw "Immutable contract mirror already exists with different content: $dest`nexisting sha256: $existingSha`nsupplied sha256: $newSha"
        }
    }
    else {
        [System.IO.File]::WriteAllBytes($dest, $bytes)
    }

    [System.IO.File]::WriteAllText(
        $shaFile,
        "$newSha  implementation-contract.md`n",
        [System.Text.UTF8Encoding]::new($false)
    )

    Write-Output "contract_path=$dest"
    Write-Output "contract_sha256=$newSha"
}
finally {
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
}
