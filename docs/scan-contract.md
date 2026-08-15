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
