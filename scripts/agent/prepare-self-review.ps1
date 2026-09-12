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

function Write-Utf8NoBom([string] $Path, [string] $Text) {
    [System.IO.File]::WriteAllText($Path, $Text, [System.Text.UTF8Encoding]::new($false))
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

function Extract-Checklist([string] $Text, [string] $Source) {
    $normalized = $Text -replace "`r`n", "`n" -replace "`r", "`n"
    $lines = $normalized -split "`n"
    $headingPattern = '^(?:(#{1,6})[ \t]+(?:[0-9]+[.)][ \t]+)?|[0-9]+[.)][ \t]+)Reviewer Checklist[ \t]*#*[ \t]*$'
    $genericHeadingPattern = '^(#{1,6})(?:[ \t]+.*)?$|^[0-9]+[.)][ \t]+.*$'
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
        if ($level -eq 0) { $level = 1 }
        $sectionItems = [System.Collections.Generic.List[string]]::new()
        $templateMarkers = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        for ($candidateIndex = $index + 1; $candidateIndex -lt $lines.Count; $candidateIndex++) {
            $candidate = $lines[$candidateIndex]
            if (-not $visible[$candidateIndex]) {
                continue
            }
            $nextHeading = [regex]::Match($candidate, $genericHeadingPattern)
            $nextLevel = $nextHeading.Groups[1].Value.Length
            if ($nextHeading.Success -and (($nextLevel -eq 0) -or $nextLevel -le $level)) {
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
            } elseif ($candidate.Trim() -in @('- PASS:', '- N/A:', '- FAIL:')) {
                [void] $templateMarkers.Add($candidate.Trim())
            }
        }
        if ($sectionItems.Count -eq 0) {
            if ($templateMarkers.Count -eq 3) {
                continue
            }
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

$prepare = Join-Path $PSScriptRoot 'prepare-work-evidence.ps1'
& $prepare $IssueNumber *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'prepare-work-evidence.ps1 failed.'
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
function Add-Items([string] $Prefix, [string] $Source, [string[]] $Items) {
    for ($offset = 0; $offset -lt $Items.Count; $offset++) {
        $item = $Items[$offset]
        $key = Normalize-DuplicateKey $item
        if (-not $seen.Add($key)) {
            Stop-Workflow 'duplicate-checklist-item' "Duplicate effective Reviewer Checklist item: $Source item $($Prefix)$('{0:D3}' -f ($offset + 1))."
        }
        [void] $entries.Add([pscustomobject]@{
            Id = "$Prefix$('{0:D3}' -f ($offset + 1))"
            Source = $Source
            Item = $item
        })
    }
}
Add-Items 'C' 'contract' $contractItems
Add-Items 'I' 'issue' $issueItems

$canonical = -join ($entries | ForEach-Object { "$($_.Id)`t$($_.Item)`n" })
$utf8 = [System.Text.UTF8Encoding]::new($false)
$sha256 = [System.Security.Cryptography.SHA256]::Create()
$fingerprint = (-join ($sha256.ComputeHash($utf8.GetBytes($canonical)) | ForEach-Object { $_.ToString('x2') })).ToLowerInvariant()

$snapshot = [System.Collections.Generic.List[string]]::new()
[void] $snapshot.Add('# Reviewer Checklist')
[void] $snapshot.Add('')
[void] $snapshot.Add("Issue: #$IssueNumber")
[void] $snapshot.Add("Checklist-SHA256: $fingerprint")
[void] $snapshot.Add('')
foreach ($entry in $entries) {
    [void] $snapshot.Add("- $($entry.Id) | $($entry.Source) | $($entry.Item)")
}
Write-Utf8NoBom $checklistPath (($snapshot -join "`n") + "`n")

$oldFingerprint = if (Test-Path -LiteralPath $checklistShaPath) { [System.IO.File]::ReadAllText($checklistShaPath).Trim() } else { '' }
$changed = ($oldFingerprint -ne $fingerprint) -or -not (Test-Path -LiteralPath $selfReviewPath)
Write-Utf8NoBom $checklistShaPath "$fingerprint`n"

if ($changed) {
    $review = [System.Collections.Generic.List[string]]::new()
    [void] $review.Add('# Self-review')
    [void] $review.Add('')
    [void] $review.Add("Issue: #$IssueNumber")
    [void] $review.Add("Checklist-SHA256: $fingerprint")
    [void] $review.Add('Reviewed-HEAD: ')
    [void] $review.Add('')
    foreach ($entry in $entries) {
        [void] $review.Add("- $($entry.Id) | PENDING |")
    }
    Write-Utf8NoBom $selfReviewPath (($review -join "`n") + "`n")
}

Write-Output "review_checklist_path=.agent-state/issues/$IssueNumber/reviewer-checklist.md"
Write-Output "review_checklist_sha256=$fingerprint"
Write-Output "self_review_path=.agent-state/issues/$IssueNumber/self-review.md"
Write-Output "checklist_changed=$(if ($changed) { 1 } else { 0 })"
Write-Output "items=$($entries.Count)"
