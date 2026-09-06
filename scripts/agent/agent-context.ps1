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

function Get-RequiredTimestamp {
    param(
        [Parameter()][object] $Value,
        [Parameter(Mandatory = $true)][string] $Field,
        [Parameter(Mandatory = $true)][int] $IssueNumber,
        [Parameter(Mandatory = $true)][int] $PrNumber
    )

    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) {
        throw "error: Issue #$IssueNumber linked PR #$PrNumber has invalid $Field"
    }

    try {
        return [DateTimeOffset]::Parse(
            [string]$Value,
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind
        )
    }
    catch {
        throw "error: Issue #$IssueNumber linked PR #$PrNumber has invalid $Field"
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

$issue = (& gh issue view $IssueNumber --repo $repository --json labels,url,closedByPullRequestsReferences | ConvertFrom-Json)
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

$prCandidates = @()
foreach ($reference in @($issue.closedByPullRequestsReferences)) {
    $referenceNumberProperty = $reference.PSObject.Properties['number']
    if ($null -eq $referenceNumberProperty -or [int]$referenceNumberProperty.Value -le 0) {
        throw "error: Issue #$IssueNumber has a linked PR reference without a number"
    }
    $referenceNumber = [int]$referenceNumberProperty.Value
    $prOutputLines = @(& gh pr view $referenceNumber --repo $repository --json number,url,state,updatedAt,mergedAt 2>$null)
    $prExitCode = $LASTEXITCODE
    $prOutput = ($prOutputLines | Out-String).Trim()
    if ($prExitCode -ne 0) {
        throw "error: Issue #$IssueNumber linked PR #$referenceNumber lookup failed"
    }
    if ([string]::IsNullOrWhiteSpace($prOutput)) {
        throw "error: Issue #$IssueNumber linked PR #$referenceNumber returned empty metadata"
    }
    try {
        $candidate = $prOutput | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw "error: Issue #$IssueNumber linked PR #$referenceNumber returned invalid metadata"
    }

    if ($null -eq $candidate -or $candidate -is [System.Array]) {
        throw "error: Issue #$IssueNumber linked PR #$referenceNumber returned invalid metadata"
    }
    $numberProperty = $candidate.PSObject.Properties['number']
    $urlProperty = $candidate.PSObject.Properties['url']
    $stateProperty = $candidate.PSObject.Properties['state']
    $candidateNumber = 0
    if (
        $null -eq $numberProperty -or
        $null -eq $urlProperty -or
        $null -eq $stateProperty -or
        -not [int]::TryParse([string]$numberProperty.Value, [ref]$candidateNumber) -or
        $candidateNumber -le 0 -or
        [string]::IsNullOrWhiteSpace([string]$urlProperty.Value) -or
        [string]::IsNullOrWhiteSpace([string]$stateProperty.Value)
    ) {
        throw "error: Issue #$IssueNumber linked PR #$referenceNumber returned incomplete metadata"
    }
    if ($candidateNumber -ne $referenceNumber) {
        throw "error: Issue #$IssueNumber linked PR #$referenceNumber returned mismatched metadata"
    }
    $state = [string]$stateProperty.Value
    if ($state -notin @('OPEN', 'MERGED', 'CLOSED')) {
        throw "error: Issue #$IssueNumber linked PR #$referenceNumber has invalid state"
    }
    $prCandidates += $candidate
}

$openPrs = @()
$mergedPrs = @()
foreach ($candidate in $prCandidates) {
    $number = [int]$candidate.PSObject.Properties['number'].Value
    $state = [string]$candidate.PSObject.Properties['state'].Value
    if ($state -eq 'OPEN') {
        $updatedProperty = $candidate.PSObject.Properties['updatedAt']
        $updatedAt = if ($null -ne $updatedProperty) { $updatedProperty.Value } else { $null }
        $openPrs += [pscustomobject]@{
            Candidate = $candidate
            SortTime = Get-RequiredTimestamp $updatedAt 'updatedAt' $IssueNumber $number
        }
    }
    elseif ($state -eq 'MERGED') {
        $mergedProperty = $candidate.PSObject.Properties['mergedAt']
        $mergedAt = if ($null -ne $mergedProperty) { $mergedProperty.Value } else { $null }
        $mergedPrs += [pscustomobject]@{
            Candidate = $candidate
            SortTime = Get-RequiredTimestamp $mergedAt 'mergedAt' $IssueNumber $number
        }
    }
}

$selectedEntry = $null
if ($openPrs.Count -gt 0) {
    $selectedEntry = $openPrs | Sort-Object -Property SortTime -Descending | Select-Object -First 1
}
elseif ($mergedPrs.Count -gt 0) {
    $selectedEntry = $mergedPrs | Sort-Object -Property SortTime -Descending | Select-Object -First 1
}
$prNumber = if ($null -ne $selectedEntry) { [string]$selectedEntry.Candidate.number } else { '' }
$prUrl = if ($null -ne $selectedEntry) { [string]$selectedEntry.Candidate.url } else { '' }

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
