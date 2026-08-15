[CmdletBinding()]
param(
    [string]$OutputRoot,
    [string]$ScannerPath
)

$ErrorActionPreference = "Stop"
$repositoryRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
        "SessionAtlas-Acceptance-" + [guid]::NewGuid().ToString("N"))
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)

if (Test-Path -LiteralPath $OutputRoot) {
    $existing = @(Get-ChildItem -LiteralPath $OutputRoot -Force)
    if ($existing.Count -ne 0) {
        throw "Refusing to reuse non-empty acceptance root: $OutputRoot"
    }
} else {
    New-Item -ItemType Directory -Path $OutputRoot | Out-Null
}

# Keep this as the first output line so acceptance logs identify the isolated
# root without ever printing the real user home or tool configuration.
Write-Output "SESSIONATLAS_ACCEPTANCE_HOME=$OutputRoot"

$projectAlpha = Join-Path $OutputRoot "projects\atlas-alpha"
$projectBeta = Join-Path $OutputRoot "projects\atlas-beta"
$codexSessions = Join-Path $OutputRoot ".codex\sessions\2026\08\15"
New-Item -ItemType Directory -Path (Join-Path $projectAlpha "docs") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $projectBeta "src") -Force | Out-Null
New-Item -ItemType Directory -Path $codexSessions -Force | Out-Null

Set-Content -LiteralPath (Join-Path $projectAlpha "docs\README.md") -Encoding utf8 -Value (
    "# Synthetic acceptance project alpha`n")
Set-Content -LiteralPath (Join-Path $projectBeta "src\demo.txt") -Encoding utf8 -Value (
    "Synthetic acceptance data only.`n")

$sessions = @(
    @{ Name = "alpha"; Id = "acceptance-alpha"; Path = $projectAlpha; Timestamp = "2026-08-15T01:00:00.000Z" },
    @{ Name = "beta"; Id = "acceptance-beta"; Path = $projectBeta; Timestamp = "2026-08-15T02:00:00.000Z" }
)
foreach ($session in $sessions) {
    $record = [ordered]@{
        timestamp = $session.Timestamp
        type = "session_meta"
        payload = [ordered]@{
            id = $session.Id
            timestamp = $session.Timestamp
            cwd = $session.Path
            originator = "sessionatlas_acceptance_fixture"
            cli_version = "0.0.0-fixture"
            source = "cli"
            model_provider = "fixture"
        }
    } | ConvertTo-Json -Depth 4 -Compress
    Set-Content -LiteralPath (Join-Path $codexSessions ("rollout-{0}.jsonl" -f $session.Name)) `
        -Encoding utf8 -Value $record
}

$previousHome = [Environment]::GetEnvironmentVariable("SESSIONATLAS_HOME", "Process")
try {
    [Environment]::SetEnvironmentVariable("SESSIONATLAS_HOME", $OutputRoot, "Process")
    if ([string]::IsNullOrWhiteSpace($ScannerPath)) {
        & dotnet run --project (Join-Path $repositoryRoot "SessionAtlas.csproj") -- scan
    } else {
        $resolvedScanner = [System.IO.Path]::GetFullPath($ScannerPath)
        if (-not (Test-Path -LiteralPath $resolvedScanner -PathType Leaf)) {
            throw "Scanner executable does not exist: $resolvedScanner"
        }
        & $resolvedScanner scan
    }
    if ($LASTEXITCODE -ne 0) {
        throw "The isolated scanner exited with code $LASTEXITCODE."
    }
} finally {
    [Environment]::SetEnvironmentVariable("SESSIONATLAS_HOME", $previousHome, "Process")
}

$dataRoot = Join-Path $OutputRoot ".sessionatlas"
$indexPath = Join-Path $dataRoot "index.db"
if (-not (Test-Path -LiteralPath $indexPath -PathType Leaf)) {
    throw "The isolated scanner did not create index.db."
}

# Tauri initializes the disposable preferences schema on first launch. An empty
# file makes the intended disposable location visible without requiring a
# second database implementation in this fixture script.
$prefsPath = Join-Path $dataRoot "prefs.db"
New-Item -ItemType File -Path $prefsPath -Force | Out-Null

$manifestFiles = Get-ChildItem -LiteralPath $OutputRoot -File -Recurse |
    Where-Object { $_.FullName -ne (Join-Path $OutputRoot "fixture-manifest.json") } |
    Sort-Object FullName |
    ForEach-Object {
        [ordered]@{
            path = [System.IO.Path]::GetRelativePath($OutputRoot, $_.FullName).Replace("\", "/")
            bytes = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }

$manifest = [ordered]@{
    schemaVersion = 1
    generatedAtUtc = [DateTime]::UtcNow.ToString("O")
    acceptanceHome = $OutputRoot
    syntheticProjects = @($projectAlpha, $projectBeta)
    files = @($manifestFiles)
} | ConvertTo-Json -Depth 6
Set-Content -LiteralPath (Join-Path $OutputRoot "fixture-manifest.json") -Encoding utf8 -Value $manifest

Write-Output "INDEX_DB=$indexPath"
Write-Output "PREFS_DB=$prefsPath"
Write-Output "FIXTURE_MANIFEST=$(Join-Path $OutputRoot 'fixture-manifest.json')"
