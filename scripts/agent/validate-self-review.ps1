[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateRange(1, 2147483647)]
    [int] $IssueNumber
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

trap {
    $message = $_.Exception.Message
    if ($message -match '^error\[[^\]]+\]:') {
        [Console]::Error.WriteLine($message)
        exit 1
    }
    break
}

function Stop-Workflow([string] $Class, [string] $Message) {
    throw "error[$Class]: $Message"
}

function Require-Command([string] $Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command not found: $Name"
    }
}

function Normalize-ChecklistText([string] $Text) {
    return (($Text -replace '\s+', ' ').Trim()).Normalize([System.Text.NormalizationForm]::FormC)
}

function Normalize-DuplicateKey([string] $Text) {
    $normalized = Normalize-ChecklistText $Text
    $builder = [System.Text.StringBuilder]::new()
    foreach ($character in $normalized.ToCharArray()) {
        if ($character -ge [char]'A' -and $character -le [char]'Z') {
            [void] $builder.Append([char]([int]$character + 32))
        }
        else {
            [void] $builder.Append($character)
        }
    }
    return $builder.ToString()
}

function Extract-CanonicalChecklist([string] $Text, [string] $Source) {
    $canonicalBegin = 'ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN'
    $canonicalEnd = 'ELWINDUI_REVIEWER_CHECKLIST_V1_END'
    $normalized = $Text -replace "`r`n", "`n" -replace "`r", "`n"
    $lines = $normalized -split "`n"
    $beginIndices = @()
    $endIndices = @()
    for ($index = 0; $index -lt $lines.Count; $index++) {
        $trimmed = $lines[$index].Trim()
        if ($trimmed -ceq $canonicalBegin) { $beginIndices += $index }
        if ($trimmed -ceq $canonicalEnd) { $endIndices += $index }
    }
    if ($beginIndices.Count -eq 0 -and $endIndices.Count -eq 0) {
        return $null
    }
    if ($beginIndices.Count -gt 1 -or $endIndices.Count -gt 1) {
        Stop-Workflow 'canonical-checklist-multiple' "$Source canonical checklist has multiple blocks"
    }
    if ($beginIndices.Count -ne 1 -or $endIndices.Count -ne 1 -or $endIndices[0] -le $beginIndices[0]) {
        Stop-Workflow 'canonical-checklist-malformed' "$Source canonical checklist markers are incomplete"
    }

    $items = [System.Collections.Generic.List[string]]::new()
    for ($index = $beginIndices[0] + 1; $index -lt $endIndices[0]; $index++) {
        $trimmed = $lines[$index].Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed)) { continue }
        if (-not $trimmed.StartsWith('REVIEW_ITEM:', [System.StringComparison]::Ordinal)) {
            Stop-Workflow 'canonical-checklist-malformed' "$Source canonical checklist contains an unexpected line"
        }
        $item = Normalize-ChecklistText $trimmed.Substring('REVIEW_ITEM:'.Length)
        if ([string]::IsNullOrWhiteSpace($item)) {
            Stop-Workflow 'canonical-checklist-empty' "$Source canonical checklist contains an empty REVIEW_ITEM"
        }
        [void] $items.Add($item)
    }
    if ($items.Count -eq 0) {
        Stop-Workflow 'canonical-checklist-empty' "$Source canonical checklist block is empty"
    }
    return [string[]] $items.ToArray()
}

function Has-StructuredEvidence([string] $Text) {
    $pattern = '(?<![A-Za-z0-9_-])(?:symbol:[^;\s]+::[^;\s]+|path:[^;\s]+|test:[^;\s]+|artifact:[^;\s]+|issue:#[1-9][0-9]*|pr:#[1-9][0-9]*|cmd:[^;\s](?:[^;]*[^;\s])?)(?![A-Za-z0-9_-])'
    return [regex]::IsMatch($Text, $pattern, [System.Text.RegularExpressions.RegexOptions]::CultureInvariant)
}

function Extract-Checklist([string] $Text, [string] $Source) {
    $normalized = $Text -replace "`r`n", "`n" -replace "`r", "`n"
    $lines = $normalized -split "`n"
    $headingPattern = '^(#{1,6})[ \t]+(?:[0-9]+[.)][ \t]+)?Reviewer Checklist[ \t]*#*[ \t]*$'
    $genericHeadingPattern = '^(#{1,6})(?:[ \t]+.*)?$'
$checkboxPattern = '^[ \t]*[-*][ \t]+\[[ xX]\][ \t]+(.+?)\s*$'
$emptyCheckboxPattern = '^[ \t]*[-*][ \t]+\[[ xX]\][ \t]*$'
    $visible = [System.Collections.Generic.List[bool]]::new()
    $fenced = $false
    foreach ($line in $lines) {
        [void] $visible.Add(-not $fenced)
        if ($line -match '^[ \t]*(```|~~~)') {
            $fenced = -not $fenced
        }
    }
    $items = [System.Collections.Generic.List[string]]::new()
    $found = $false

    for ($index = 0; $index -lt $lines.Count; $index++) {
        if (-not $visible[$index]) {
            continue
        }
        $heading = [regex]::Match($lines[$index], $headingPattern)
        if (-not $heading.Success) {
            continue
        }
        $found = $true
        $level = $heading.Groups[1].Value.Length
        $sectionItems = [System.Collections.Generic.List[string]]::new()
        for ($candidateIndex = $index + 1; $candidateIndex -lt $lines.Count; $candidateIndex++) {
            $candidate = $lines[$candidateIndex]
            if (-not $visible[$candidateIndex]) {
                continue
            }
            $nextHeading = [regex]::Match($candidate, $genericHeadingPattern)
            if ($nextHeading.Success -and $nextHeading.Groups[1].Value.Length -le $level) {
                break
            }
            if ([regex]::IsMatch($candidate, $emptyCheckboxPattern)) {
                Stop-Workflow 'empty-checklist' "$Source Reviewer Checklist has an empty checkbox item"
            }
            $checkbox = [regex]::Match($candidate, $checkboxPattern)
            if ($checkbox.Success) {
                $item = Normalize-ChecklistText $checkbox.Groups[1].Value
                if ([string]::IsNullOrWhiteSpace($item)) {
                    Stop-Workflow 'empty-checklist' "$Source Reviewer Checklist has an empty item"
                }
                [void] $sectionItems.Add($item)
            }
        }
        if ($sectionItems.Count -eq 0) {
            Stop-Workflow 'empty-checklist' "$Source Reviewer Checklist section has zero checkbox items"
        }
        foreach ($item in $sectionItems) {
            [void] $items.Add($item)
        }
    }

    return [pscustomobject]@{
        Found = $found
        Items = [string[]] $items.ToArray()
    }
}

function Extract-ContractChecklist([string] $Text, [string] $Source) {
    $canonicalItems = Extract-CanonicalChecklist $Text $Source
    if ($null -ne $canonicalItems) {
        return [string[]] $canonicalItems
    }
    $result = Extract-Checklist $Text $Source
    return [string[]] $result.Items
}

function Get-ContractItems([string] $Base) {
    $contract = Join-Path $Base 'implementation-contract.md'
    $contractSha = Join-Path $Base 'implementation-contract.sha256'
    $present = (Test-Path -LiteralPath $contract) -or (Test-Path -LiteralPath $contractSha)
    if (-not $present) {
        return [string[]] @()
    }
    if (-not (Test-Path -LiteralPath $contract -PathType Leaf) -or
        -not (Test-Path -LiteralPath $contractSha -PathType Leaf)) {
        Stop-Workflow 'contract-integrity' 'Contract mirror is incomplete.'
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $contract).Hash.ToLowerInvariant()
    $recorded = [System.IO.File]::ReadAllText($contractSha).Trim()
    if ($recorded -notmatch '^([0-9a-f]{64})\s+implementation-contract\.md$' -or $Matches[1] -ne $actual) {
        Stop-Workflow 'contract-integrity' 'Contract mirror integrity check failed.'
    }
    return Extract-ContractChecklist ([System.IO.File]::ReadAllText($contract)) 'contract'
}

function Get-Metadata([string] $Text, [string] $Name, [string] $Pattern) {
    $matches = @($Text -split "`n" | Where-Object { $_.StartsWith("${Name}:") })
    if ($matches.Count -ne 1) {
        throw "Self-review metadata $Name is missing or duplicated."
    }
    $match = [regex]::Match($matches[0], $Pattern)
    if (-not $match.Success) {
        throw "Self-review metadata $Name is malformed."
    }
    return $match.Groups[1].Value
}

Require-Command 'git'
Require-Command 'gh'

$root = (& git rev-parse --show-toplevel 2>$null | Out-String).Trim()
if ([string]::IsNullOrWhiteSpace($root)) {
    throw 'Run inside a Git repository.'
}
Set-Location -LiteralPath $root

& gh auth status *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'GitHub CLI is not authenticated; run: gh auth login'
}
$repository = (& gh repo view --json nameWithOwner --jq '.nameWithOwner' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($repository)) {
    throw 'Unable to resolve the current GitHub repository.'
}
$issueJson = (& gh issue view $IssueNumber --repo $repository --json number,body | Out-String).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Unable to load Issue #$IssueNumber."
}
$issue = $issueJson | ConvertFrom-Json
if ([int] $issue.number -ne $IssueNumber -or $null -eq $issue.body) {
    throw "Issue metadata does not match #$IssueNumber."
}

$base = Join-Path $root ".agent-state/issues/$IssueNumber"
$checklistPath = Join-Path $base 'reviewer-checklist.md'
$checklistShaPath = Join-Path $base 'reviewer-checklist.sha256'
$selfReviewPath = Join-Path $base 'self-review.md'
$contractItems = @(Get-ContractItems $base)
$issueResult = Extract-Checklist ([string] $issue.body) 'Issue'
$issueItems = @($issueResult.Items)
if ($contractItems.Count -eq 0 -and $issueItems.Count -eq 0) {
    Stop-Workflow 'missing-checklist' 'No effective Reviewer Checklist exists.'
}

$entries = [System.Collections.Generic.List[object]]::new()
$seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
function Add-Items([string] $Prefix, [string[]] $Items) {
    for ($offset = 0; $offset -lt $Items.Count; $offset++) {
        $item = $Items[$offset]
        $key = Normalize-DuplicateKey $item
        if (-not $seen.Add($key)) {
            Stop-Workflow 'duplicate-checklist-item' "Duplicate effective Reviewer Checklist item: $Prefix$('{0:D3}' -f ($offset + 1))."
        }
        [void] $entries.Add([pscustomobject]@{
            Id = "$Prefix$('{0:D3}' -f ($offset + 1))"
            Item = $item
        })
    }
}
Add-Items 'C' $contractItems
Add-Items 'I' $issueItems

$canonical = -join ($entries | ForEach-Object { "$($_.Id)`t$($_.Item)`n" })
$utf8 = [System.Text.UTF8Encoding]::new($false)
$sha256 = [System.Security.Cryptography.SHA256]::Create()
$fingerprint = (-join ($sha256.ComputeHash($utf8.GetBytes($canonical)) | ForEach-Object { $_.ToString('x2') })).ToLowerInvariant()

if (-not (Test-Path -LiteralPath $checklistPath -PathType Leaf) -or
    -not (Test-Path -LiteralPath $checklistShaPath -PathType Leaf)) {
    Stop-Workflow 'stale-checklist' 'Prepared Reviewer Checklist artifacts are missing.'
}
$preparedSha = [System.IO.File]::ReadAllText($checklistShaPath).Trim()
if ($preparedSha -notmatch '^[0-9a-f]{64}$' -or $preparedSha -ne $fingerprint) {
    Stop-Workflow 'stale-checklist' 'Prepared Reviewer Checklist source is stale.'
}

if (-not (Test-Path -LiteralPath $selfReviewPath -PathType Leaf)) {
    throw 'Self-review artifact is missing.'
}
$selfReview = [System.IO.File]::ReadAllText($selfReviewPath)
$null = Get-Metadata $selfReview 'Issue' "^Issue: #($IssueNumber)$"
$reviewSha = Get-Metadata $selfReview 'Checklist-SHA256' '^Checklist-SHA256: ([0-9a-f]{64})$'
if ($reviewSha -ne $fingerprint) {
    throw 'Self-review checklist SHA does not match the current source.'
}
$reviewedHead = Get-Metadata $selfReview 'Reviewed-HEAD' '^Reviewed-HEAD: ([0-9a-f]{40})$'

$dirty = (& git status --porcelain --untracked-files=all | Out-String).Trim()
if (-not [string]::IsNullOrWhiteSpace($dirty)) {
    Stop-Workflow 'dirty-worktree' 'Repository-controlled worktree is dirty.'
}
$currentHead = (& git rev-parse HEAD | Out-String).Trim()
if ($reviewedHead -ne $currentHead) {
    Stop-Workflow 'stale-head' 'Reviewed-HEAD is stale.'
}
& git cat-file -e "$reviewedHead^{commit}" *> $null
if ($LASTEXITCODE -ne 0) {
    Stop-Workflow 'stale-head' 'Reviewed-HEAD is not a valid commit.'
}

$expected = [System.Collections.Generic.HashSet[string]]::new()
foreach ($entry in $entries) {
    [void] $expected.Add($entry.Id)
}
$results = @{}
foreach ($line in ($selfReview -split "`n")) {
    if (-not $line.StartsWith('- ')) {
        continue
    }
    $parts = $line.Substring(2).Split('|', 3)
    if ($parts.Count -ne 3) {
        throw 'Self-review contains a malformed result entry.'
    }
    $itemId = $parts[0].Trim()
    $status = $parts[1].Trim()
    $detail = $parts[2].Trim()
    if ($results.ContainsKey($itemId)) {
        Stop-Workflow 'duplicate-result-id' "Self-review contains duplicate result ID: $itemId"
    }
    if (-not $expected.Contains($itemId)) {
        Stop-Workflow 'unknown-result-id' "Self-review contains unknown result ID: $itemId"
    }
    $results[$itemId] = [pscustomobject]@{ Status = $status; Detail = $detail }
}

$missing = @($expected | Where-Object { -not $results.ContainsKey($_) } | Sort-Object)
if ($missing.Count -gt 0) {
    Stop-Workflow 'missing-result-id' "Self-review is missing result IDs: $($missing -join ', ')"
}

$passCount = 0
$naCount = 0
foreach ($itemId in $results.Keys) {
    $result = $results[$itemId]
    if ($result.Status -eq 'PASS') {
        if (-not $result.Detail.StartsWith('Evidence:', [System.StringComparison]::Ordinal) -or [string]::IsNullOrWhiteSpace($result.Detail.Substring(9)) -or -not (Has-StructuredEvidence $result.Detail.Substring(9).Trim())) {
            Stop-Workflow 'missing-evidence' "PASS item $itemId requires at least one valid structured Evidence token."
        }
        $passCount++
    }
    elseif ($result.Status -eq 'N/A') {
        if (-not $result.Detail.StartsWith('Reason:', [System.StringComparison]::Ordinal) -or [string]::IsNullOrWhiteSpace($result.Detail.Substring(7))) {
            Stop-Workflow 'missing-na-reason' "N/A item $itemId requires a concrete Reason:"
        }
        $naCount++
    }
    elseif ($result.Status -eq 'PENDING') {
        Stop-Workflow 'pending-item' "Self-review item $itemId is PENDING."
    }
    elseif ($result.Status -eq 'FAIL') {
        Stop-Workflow 'failed-item' "Self-review item $itemId is FAIL."
    }
    else {
        throw "Self-review item $itemId has invalid status: $($result.Status)"
    }
}

Write-Output 'self_review_status=pass'
Write-Output "reviewed_head=$reviewedHead"
Write-Output "review_checklist_sha256=$fingerprint"
Write-Output "pass=$passCount"
Write-Output "na=$naCount"
Write-Output 'fail=0'
