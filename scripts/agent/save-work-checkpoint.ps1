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

foreach ($name in @('git', 'gh')) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        throw "Required command not found: $name"
    }
}

$repoRoot = Invoke-Checked git @('rev-parse', '--show-toplevel')
Set-Location $repoRoot

& gh auth status *> $null
if ($LASTEXITCODE -ne 0) {
    throw 'GitHub CLI is not authenticated. Run: gh auth login'
}

$repository = Invoke-Checked gh @(
    'repo', 'view', '--json', 'nameWithOwner', '--jq', '.nameWithOwner'
)

& gh issue view $IssueNumber --repo $repository *> $null
if ($LASTEXITCODE -ne 0) {
    throw "Issue #$IssueNumber does not exist or is not accessible."
}

$branch = Invoke-Checked git @('branch', '--show-current')
$headCommit = Invoke-Checked git @('rev-parse', 'HEAD')
$baseBranch = Invoke-Checked gh @(
    'repo', 'view', '--json', 'defaultBranchRef', '--jq', '.defaultBranchRef.name'
)
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

& "$PSScriptRoot\prepare-work-evidence.ps1" $IssueNumber | Out-Null

$checkpointFile = Join-Path $repoRoot ".agent-state/issues/$IssueNumber/checkpoint.md"

function Quote-YamlString {
    param([string] $Value)
    return ($Value | ConvertTo-Json -Compress)
}

$frontmatter = @(
    '---'
    'schema: 1'
    "repository: $(Quote-YamlString $repository)"
    "issue: $IssueNumber"
    "phase: $(Quote-YamlString $phase)"
    "branch: $(Quote-YamlString $branch)"
    "base_branch: $(Quote-YamlString $baseBranch)"
    "head_commit: $(Quote-YamlString $headCommit)"
    "updated_at: $(Quote-YamlString ([DateTimeOffset]::Now.ToString('o')))"
    'platform: "windows"'
    "working_tree: $(Quote-YamlString $workingTree)"
    "pull_request: $(Quote-YamlString $pullRequest)"
    '---'
) -join "`n"

$defaultBody = @'
# Objective

- TODO

# Completed

- TODO

# Current state

- TODO

# Next action

- TODO: name the next file, symbol, test, investigation, or command.

# Verification

## Passed

- None

## Failed

- None

## Not run

- None

# Uncommitted work

- None

# Blockers

- None
'@

if (Test-Path -LiteralPath $checkpointFile) {
    $current = [IO.File]::ReadAllText($checkpointFile)
    if ($current -match '(?s)\A---\r?\n.*?\r?\n---\r?\n(?<body>.*)\z') {
        $body = $Matches['body']
    }
    else {
        $body = $current
    }
}
else {
    $body = $defaultBody
}

if ($workingTree -eq 'dirty') {
    $statusBlock = (
        $status -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
        ForEach-Object { "- ``$_``" }
    ) -join "`n"

    $body = [regex]::Replace(
        $body,
        '(?s)# Uncommitted work\r?\n.*?(?=\r?\n# Blockers)',
        "# Uncommitted work`n`n$statusBlock`n",
        1
    )
}

[IO.File]::WriteAllText(
    $checkpointFile,
    "$frontmatter`n`n$($body.TrimStart())",
    (New-Object Text.UTF8Encoding($false))
)

Write-Output $checkpointFile
