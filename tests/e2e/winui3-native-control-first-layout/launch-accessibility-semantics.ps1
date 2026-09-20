[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Executable,
    [Parameter(Mandatory = $true)]
    [string] $EvidenceDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$resolvedEvidence = (Resolve-Path -LiteralPath $EvidenceDirectory).Path
$stdoutPath = Join-Path $resolvedEvidence 'accessibility-semantics.stdout.log'
$stderrPath = Join-Path $resolvedEvidence 'accessibility-semantics.stderr.log'
$startedAt = [DateTimeOffset]::UtcNow
$workingDirectory = Split-Path -Parent $resolvedExecutable

$startParameters = @{
    FilePath = $resolvedExecutable
    WorkingDirectory = $workingDirectory
    RedirectStandardOutput = $stdoutPath
    RedirectStandardError = $stderrPath
    Environment = @{
        ELWINDUI_WINUI3_DIAGNOSTICS = '1'
        ELWINDUI_PERF_TRACE = '1'
    }
    PassThru = $true
}
$process = Start-Process @startParameters

[pscustomobject]@{
    success = $true
    pid = $process.Id
    executable = $resolvedExecutable
    working_directory = $workingDirectory
    started_at_utc = $startedAt.ToString('o')
    stdout_path = $stdoutPath
    stderr_path = $stderrPath
} | ConvertTo-Json -Compress
