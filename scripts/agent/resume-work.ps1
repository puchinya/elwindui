[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateRange(1, 2147483647)]
    [int] $IssueNumber
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string] $FilePath,
        [Parameter()][string[]] $Arguments = @()
    )

    $output = & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed: $FilePath $($Arguments -join ' ')"
    }
    return (($output | Out-String).Trim())
}

$repoRoot = Invoke-Checked git @('rev-parse', '--show-toplevel')
Set-Location $repoRoot

$checkpointFile = Join-Path $repoRoot ".agent-state/issues/$IssueNumber/checkpoint.md"
if (-not (Test-Path -LiteralPath $checkpointFile -PathType Leaf)) {
    throw "Checkpoint not found: $checkpointFile"
}

$repository = Invoke-Checked gh @(
    'repo', 'view', '--json', 'nameWithOwner', '--jq', '.nameWithOwner'
)
$branch = Invoke-Checked git @('branch', '--show-current')
$headCommit = Invoke-Checked git @('rev-parse', 'HEAD')
$status = Invoke-Checked git @('status', '--porcelain')
$workingTree = if ([string]::IsNullOrWhiteSpace($status)) { 'clean' } else { 'dirty' }

$phase = Invoke-Checked gh @(
    'issue', 'view', "$IssueNumber",
    '--repo', $repository,
    '--json', 'labels',
    '--jq', '[.labels[].name | select(startswith("phase:"))][0] // ""'
)

$pullRequest = Invoke-Checked gh @(
    'pr', 'list',
    '--repo', $repository,
    '--head', $branch,
    '--state', 'all',
    '--limit', '1',
    '--json', 'number',
    '--jq', '.[0].number // empty'
)

$text = [IO.File]::ReadAllText($checkpointFile)
$metadata = @{}

if ($text -match '(?s)\A---\r?\n(?<frontmatter>.*?)\r?\n---\r?\n') {
    foreach ($line in ($Matches['frontmatter'] -split "`r?`n")) {
        if ($line -notmatch '^(?<key>[^:]+):\s*(?<value>.*)$') {
            continue
        }

        $key = $Matches['key'].Trim()
        $raw = $Matches['value'].Trim()

        try {
            $value = $raw | ConvertFrom-Json
        }
        catch {
            $value = $raw
        }

        $metadata[$key] = $value
    }
}

$current = [ordered]@{
    repository = $repository
    issue = $IssueNumber
    phase = $phase
    branch = $branch
    head_commit = $headCommit
    working_tree = $workingTree
    pull_request = $pullRequest
}

$differences = New-Object System.Collections.Generic.List[string]
foreach ($entry in $current.GetEnumerator()) {
    $recorded = if ($metadata.ContainsKey($entry.Key)) {
        $metadata[$entry.Key]
    }
    else {
        $null
    }

    if ("$recorded" -cne "$($entry.Value)") {
        $differences.Add(
            "- $($entry.Key): checkpoint='$recorded', current='$($entry.Value)'"
        )
    }
}

Write-Output $text.TrimEnd()
Write-Output ''
Write-Output '---'

if ($differences.Count -gt 0) {
    Write-Output 'Checkpoint status: STALE OR CHANGED'
    $differences | ForEach-Object { Write-Output $_ }
    Write-Output 'Refresh the checkpoint from current Git/GitHub state before editing.'
}
else {
    Write-Output 'Checkpoint status: CURRENT'
}
