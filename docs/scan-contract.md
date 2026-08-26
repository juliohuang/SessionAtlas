# SessionAtlas scan contract

This document defines the behavior that scanner and index changes must preserve.
It is intentionally independent of any single tool's current on-disk format.

## Project identity

- A project represents one working directory used by an AI coding tool.
- Its canonical identity is its normalized absolute path.
- Windows path comparison is case-insensitive; Linux path comparison is
  case-sensitive. macOS follows the mounted filesystem where practical.
- A path inside an AI tool's own configuration, cache, or session directory is
  never a project.
- The project ID and `first_seen_at` remain stable across successful rescans.
- A recorded project whose directory no longer exists remains indexed with
  `path_missing = true`; the state is recomputed on reads so post-scan deletion
  is visible without mutating the snapshot database.
- Permission and transient I/O errors are not classified as a missing path.

## Sessions and tool usage

- A session is one distinct tool-native session ID.
- `session_count` is the number of distinct known sessions for one
  `(project, tool)` pair. It is never the number of scans.
- `last_used_at` is the greatest valid activity timestamp among those sessions.
- `last_session_id` belongs to the session that supplied `last_used_at`.
- When timestamps tie, scanners use a deterministic tool-specific tie breaker.
- A tool can be discoverable from historical data even when its executable is
  no longer installed. Launchability is a separate state.

## Full and partial scans

- A full scan is a snapshot of every scanner that was successfully inspected.
- A partial `--tool` scan replaces only that tool's snapshot.
- Failed or unavailable scanners do not erase their previous successful data.
- A successful empty result does remove the scanned tool's previous data.
- A project with no remaining tool usages is removed from the derived index.
- The index and FTS view change atomically: readers see either the previous
  complete snapshot or the next complete snapshot, never a partial scan.
- Repeating a scan over unchanged inputs produces identical database contents.

## Time handling

- Persisted timestamps use UTC ISO-8601.
- Offset-aware source timestamps are converted to UTC.
- Filesystem modification time is a documented fallback, not preferred truth.
- Malformed or missing timestamps produce a diagnostic and use the next
  supported fallback.
- Remote scan time is not presented as project activity time.

## Project content index

- Project name/path FTS is part of the atomic project snapshot. Source/document
  content is a secondary rebuildable cache refreshed after that commit; a
  content-index failure never erases or falsely fails the successful project
  snapshot.
- Search executes only against SQLite FTS5. It never walks project directories
  while the user types.
- Refresh is incremental by relative path plus modification time and file size.
  Changed and deleted files replace/remove their old terms; unchanged files are
  not opened.
- Default bounds are 50,000 walked entries, 2,000 files, 8 MiB indexed text per
  project, 256 KiB per file, and a 32 KiB preview. Hitting a bound is recorded
  as truncated, not reported as a complete content snapshot.
- Dependency/build/cache trees, AI-tool session data, binary/oversized files,
  lockfiles, `.env`, key/certificate and credential-shaped files are excluded.
- `project_content_fts` is contentless: raw source cannot be selected back from
  SQLite. Only FTS terms and a small LZ4-compressed preview used for result
  subtitles are persisted.
- Remote source is not downloaded for content indexing. Remote projects retain
  name/path search only.
- Content search requires a term of at least two characters and consumes at
  most 256 query characters / 12 terms. Name/path search retains one-character
  prefix matching.

## Diagnostics and privacy

- Scanner failures are returned as structured diagnostics containing tool,
  severity, stable code, and a user-actionable message.
- A malformed session does not abort other valid sessions from the same tool.
- Silent catch-all handling is not part of the contract.
- Diagnostics and tests never include prompt text, credentials, or session
  message bodies.
- Automated tests use temporary homes and temporary databases. They must never
  open or mutate the user's real `~/.sessionatlas` directory.

## Fixture policy

- Fixtures contain invented paths, IDs, timestamps, and content.
- Fixtures preserve only the minimum structural fields needed to test parsing.
- Binary stores are represented by schema/seed SQL and created in a temporary
  directory during a test.
- Each format change receives a new fixture rather than rewriting historical
  fixtures in place.

## Scanner outcome states

- `Succeeded` is the only state allowed to replace a tool snapshot. It may
  contain zero projects when the source was inspected successfully.
- `Unavailable` means neither a local source nor a launchable executable was
  found. The prior snapshot is preserved.
- `Failed` means a source exists but could not be inspected safely, or session
  files exist but none produce a valid project. The prior snapshot is
  preserved.
- Stable diagnostic codes currently include `source_unavailable`,
  `source_read_failed`, `session_read_failed`, `malformed_session_record`,
  `missing_project_path`, `missing_session_id`, `timestamp_fallback`,
  `no_valid_sessions`, `config_read_failed`, and
  `unexpected_scanner_failure`.
