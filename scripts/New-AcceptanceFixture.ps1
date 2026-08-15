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

# Resolve the scanner executable before redirecting SESSIONATLAS_HOME. When no
# path is given the script runs `cargo run` from the workspace (dev fallback);
# the release chain always passes the built target/release/sessionatlas.exe.
if ([string]::IsNullOrWhiteSpace($ScannerPath)) {
    $resolvedScanner = $null
} else {
    $resolvedScanner = [System.IO.Path]::GetFullPath($ScannerPath)
    if (-not (Test-Path -LiteralPath $resolvedScanner -PathType Leaf)) {
        throw "Scanner executable does not exist: $resolvedScanner"
    }
}

# Runs a scanner command (scan/list) against the isolated home. Stdout
# is captured; stderr is left on the console so diagnostics stay visible.
function Invoke-ScannerCommand {
    param(
        [string[]]$Arguments
    )
    if ($null -eq $resolvedScanner) {
        & cargo run --locked -p sessionatlas-cli `
            --manifest-path (Join-Path $repositoryRoot "Cargo.toml") `
            -- @Arguments
    } else {
        & $resolvedScanner @Arguments
    }
}

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

    # 1. The release scanner must exit 0 and create the isolated index.
    $scanOutput = Invoke-ScannerCommand "scan"
    $scanExit = $LASTEXITCODE
    if ($scanExit -ne 0) {
        throw "The isolated scanner exited with code $scanExit."
    }

    $dataRoot = Join-Path $OutputRoot ".sessionatlas"
    $indexPath = Join-Path $dataRoot "index.db"
    if (-not (Test-Path -LiteralPath $indexPath -PathType Leaf)) {
        throw "The isolated scanner did not create index.db."
    }
    $indexInfo = Get-Item -LiteralPath $indexPath
    if ($indexInfo.Length -le 0) {
        throw "index.db exists but is empty."
    }
    $indexDbBytes = $indexInfo.Length
    Write-Output "INDEX_DB_BYTES=$indexDbBytes"

    # This database is newly created for the fixture, so exact UTF-8 marker
    # presence proves both native session IDs reached the persisted index (not
    # just the input manifest) without adding an sqlite3/Python dependency.
    $indexText = [System.Text.Encoding]::UTF8.GetString(
        [System.IO.File]::ReadAllBytes($indexPath))
    foreach ($session in $sessions) {
        if (-not $indexText.Contains($session.Id)) {
            throw "The isolated index does not contain session ID $($session.Id)."
        }
    }
    $indexSessionIdsFound = $sessions.Count
    Write-Output "INDEX_SESSION_IDS_FOUND=$indexSessionIdsFound"

    # 2. Read the index back through the same release scanner and require both
    #    synthetic projects to appear as exactly two indexed projects. This is
    #    the proof that the release CLI wrote the fixture into the isolated DB.
    $listOutput = Invoke-ScannerCommand "list"
    $listExit = $LASTEXITCODE
    if ($listExit -ne 0) {
        throw "The isolated scanner list command exited with code $listExit."
    }
    $listText = ($listOutput -join "`n")
    $listRows = @($listOutput | Where-Object { $_ -match '^[0-9]+\s' }).Count
    if ($listRows -ne 2) {
        throw "Expected 2 listed projects, the list command reported $listRows."
    }
    if ($listText -notmatch 'atlas-alpha' -or $listText -notmatch 'atlas-beta') {
        throw "The isolated index does not contain both synthetic projects."
    }
    Write-Output "LIST_PROJECTS=$listRows"

    # 3. The search table renders each project's last-access time as absolute
    #    UTC wall time. Redirecting NUL to stdin makes the interactive picker
    #    cancel immediately, so the table is still printed and the command
    #    exits 0. This confirms the seeded UTC times were indexed, not merely
    #    seeded on disk.
    if ($null -eq $resolvedScanner) {
        $searchCommand = "cargo run --locked -p sessionatlas-cli " +
            "--manifest-path `"$repositoryRoot\Cargo.toml`" -- search atlas < NUL"
    } else {
        $searchCommand = "`"$resolvedScanner`" search atlas < NUL"
    }
    $searchOutput = & cmd /c $searchCommand 2>&1
    $searchExit = $LASTEXITCODE
    if ($searchExit -ne 0) {
        throw "The isolated scanner search command exited with code $searchExit."
    }
    $searchText = ($searchOutput -join "`n")
    if ($searchText -notmatch '2026-08-15 02:00' -or $searchText -notmatch '2026-08-15 01:00') {
        throw "The indexed projects do not carry both UTC session times."
    }
    if ($searchText -notmatch 'atlas-alpha' -or $searchText -notmatch 'atlas-beta') {
        throw "The search result does not contain both synthetic projects."
    }
    Write-Output "SEARCH_UTC_TIMES_FOUND=2"

    $databaseSidecars = @(
        "$indexPath-journal",
        "$indexPath-wal",
        "$indexPath-shm"
    ) | Where-Object { Test-Path -LiteralPath $_ }
    if ($databaseSidecars.Count -ne 0) {
        throw "The isolated scan left SQLite sidecar files behind."
    }
    Write-Output "INDEX_SIDECARS=0"

    # Tauri initializes the disposable preferences schema on first launch. An empty
    # file makes the intended disposable location visible without requiring a
    # second database implementation in this fixture script.
    $prefsPath = Join-Path $dataRoot "prefs.db"
    New-Item -ItemType File -Path $prefsPath -Force | Out-Null
} finally {
    [Environment]::SetEnvironmentVariable("SESSIONATLAS_HOME", $previousHome, "Process")
}

$manifestFiles = Get-ChildItem -LiteralPath $OutputRoot -File -Recurse |
    Where-Object { $_.FullName -ne (Join-Path $OutputRoot "fixture-manifest.json") } |
    Sort-Object FullName |
    ForEach-Object {
        [ordered]@{
            path = $_.FullName.Substring($OutputRoot.Length).TrimStart('\', '/').Replace("\", "/")
            bytes = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }

# The seeded input is recorded completely (both project paths, both session
# IDs, both UTC timestamps) so the manifest alone can reproduce the fixture.
$syntheticSessions = @(
    foreach ($session in $sessions) {
        [ordered]@{
            name = $session.Name
            sessionId = $session.Id
            projectPath = $session.Path
            timestampUtc = $session.Timestamp
        }
    }
)

# The manifest must be complete: both projects, both session IDs and both UTC
# timestamps, with nothing blank.
if ($syntheticSessions.Count -ne 2) {
    throw "The fixture manifest is missing a synthetic session record."
}
foreach ($synthetic in $syntheticSessions) {
    if ([string]::IsNullOrWhiteSpace($synthetic.sessionId) -or
        [string]::IsNullOrWhiteSpace($synthetic.projectPath) -or
        [string]::IsNullOrWhiteSpace($synthetic.timestampUtc)) {
        throw "The fixture manifest has an incomplete synthetic session record."
    }
}

$verification = [ordered]@{
    scannerExitCode = 0
    indexDbBytes = $indexDbBytes
    indexSessionIdsFound = $indexSessionIdsFound
    indexSidecarCount = 0
    listProjectCount = $listRows
    searchUtcTimesFound = 2
    searchUtcTimes = @("2026-08-15 01:00", "2026-08-15 02:00")
    manifestSyntheticProjects = 2
    manifestSyntheticSessions = $syntheticSessions.Count
}

$manifest = [ordered]@{
    schemaVersion = 2
    generatedAtUtc = [DateTime]::UtcNow.ToString("O")
    acceptanceHome = $OutputRoot
    syntheticProjects = @($projectAlpha, $projectBeta)
    syntheticSessions = $syntheticSessions
    verification = $verification
    files = @($manifestFiles)
} | ConvertTo-Json -Depth 8
Set-Content -LiteralPath (Join-Path $OutputRoot "fixture-manifest.json") -Encoding utf8 -Value $manifest

Write-Output "INDEX_DB=$indexPath"
Write-Output "PREFS_DB=$prefsPath"
Write-Output "FIXTURE_MANIFEST=$(Join-Path $OutputRoot 'fixture-manifest.json')"
