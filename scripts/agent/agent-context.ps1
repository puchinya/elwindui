[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateRange(1, 2147483647)]
    [int] $IssueNumber
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
$defaultBranch = (& gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name').Trim()
$branch = (& git branch --show-current).Trim()
$head = (& git rev-parse HEAD).Trim()
$status = (& git status --porcelain | Out-String).Trim()
$worktree = if ([string]::IsNullOrWhiteSpace($status)) { 'clean' } else { 'dirty' }

$issue = (& gh issue view $IssueNumber --repo $repository --json labels,url | ConvertFrom-Json)
$phaseValues = @($issue.labels | ForEach-Object { $_.name } | Where-Object { $_ -like 'phase:*' } | Select-Object -First 1)
$phase = if ($phaseValues.Count -gt 0) { [string]$phaseValues[0] } else { '' }

$workflow = switch ($phase) {
    'phase:requirements' { 'docs/agent-workflow/requirements.md' }
    'phase:design' { 'docs/agent-workflow/design.md' }
    'phase:ready' { 'docs/agent-workflow/implementation.md' }
    'phase:implementation' { 'docs/agent-workflow/implementation.md' }
    'phase:review' { 'docs/agent-workflow/review.md' }
    default { '' }
}

$prs = @(& gh pr list --repo $repository --state all --limit 100 --json number,url,state,updatedAt,closingIssuesReferences | ConvertFrom-Json)
$linkedPrs = @($prs | Where-Object {
    $references = @($_.closingIssuesReferences)
    $references | Where-Object { [int]$_.number -eq $IssueNumber } | Select-Object -First 1
})
$openPrs = @($linkedPrs | Where-Object { $_.state -eq 'OPEN' })
$mergedPrs = @($linkedPrs | Where-Object { $_.state -eq 'MERGED' })
$selectedPr = $null
if ($openPrs.Count -gt 0) {
    $selectedPr = $openPrs | Sort-Object -Property updatedAt -Descending | Select-Object -First 1
}
elseif ($mergedPrs.Count -gt 0) {
    $selectedPr = $mergedPrs | Sort-Object -Property updatedAt -Descending | Select-Object -First 1
}
$prNumber = if ($null -ne $selectedPr) { [string]$selectedPr.number } else { '' }
$prUrl = if ($null -ne $selectedPr) { [string]$selectedPr.url } else { '' }

$base = ".agent-state/issues/$IssueNumber"
$contract = Join-Path $base 'implementation-contract.md'
$shaFile = Join-Path $base 'implementation-contract.sha256'
$contractStatus = 'absent'
$contractSha = ''

if ((Test-Path -LiteralPath $contract) -or (Test-Path -LiteralPath $shaFile)) {
    $contractStatus = 'invalid'
    if ((Test-Path -LiteralPath $contract -PathType Leaf) -and (Test-Path -LiteralPath $shaFile -PathType Leaf)) {
        $contractSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $contract).Hash.ToLowerInvariant()
        $recorded = (Get-Content -Raw -LiteralPath $shaFile).Trim()
        if ($recorded -match '^([0-9a-f]{64})\s+implementation-contract\.md$') {
            if ($Matches[1] -eq $contractSha) {
                $contractStatus = 'ok'
            }
        }
    }
}

Write-Output "repository=$repository"
Write-Output "issue=$IssueNumber"
Write-Output "phase=$phase"
Write-Output "workflow=$workflow"
Write-Output "branch=$branch"
Write-Output "head=$head"
Write-Output "default_branch=$defaultBranch"
Write-Output "worktree=$worktree"
Write-Output "pr_number=$prNumber"
Write-Output "pr_url=$prUrl"
Write-Output "contract_path=$contract"
Write-Output "contract_sha256=$contractSha"
Write-Output "contract_status=$contractStatus"
