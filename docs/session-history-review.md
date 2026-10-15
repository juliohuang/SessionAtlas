# Session history and read-only Git review

Select a project and choose **Session history** or **Git changes** from the
project overview. The same actions are available in its expanded ledger row.

## Session history

History search filters the selected project's session titles, short user-task
previews, native IDs, and tools. The main search box continues to search project
names, paths, source files, and documents; it does not search conversations.

History is read on demand from the six official tool stores. It is not written
to `index.db`, the project scanner cache, or a new conversation database. Titles
come from available native title metadata or the first usable user task; a
missing title is displayed as an untitled session. Kimi and OpenCode can show
metadata without a user-message preview. Aider records activity without
inventing a native session resume ID. Custom adapter history formats currently
report an unsupported-format notice; their existing launch workflow is retained.

Reads have depth, entry, file, byte, record, and time limits. The catalog returns
at most 200 sessions. Large JSONL transcripts use a bounded prefix; skipped or
inaccessible data produces an explicit notice. The catalog is a bounded view,
not a guarantee that every historical conversation has been read. Recent source
files are prioritized. Child, guardian, delegated exec, and sidechain sessions
are excluded from resume targets. System instructions, attachments, tool outputs,
and recognized credential shapes are excluded from user previews.

Directory availability and adapter resume support affect each resume action.
Before a history action launches, the backend checks the directory, active
adapter/platform/selection, executable, native ID and matching local history.
Failures remain in the panel with recovery guidance. An incomplete read is not
reported as proof that a session was deleted. Provider login and the CLI's own
acceptance of a recorded ID remain runtime checks performed by that CLI.

Historical local sessions have distinct terminal identities. Selecting an old
session does not silently focus another session from the same tool. Selecting
the same already-open historical session reuses its terminal. Default project
opening retains its existing behavior.

Remote history displays recorded tool usage rather than fetching remote
conversation files. Remote actions check the configured SSH target and project
directory, and retain the existing per-server PTY and persistent tmux workflow.
Native remote session content/IDs are not verified by the local history reader.
Remote full-history search and Git diff preview are not included in this change.

## Git changes

The panel lists staged, unstaged and untracked changes. A file changed in both
the index and working tree has separate selectors for those two diffs. Paths
are repository-root-relative, including when the selected project is a nested
directory. Renamed files retain both paths for diff selection.

Status and diff use bounded argument-array processes. The review never stages,
commits, checks out, fetches, or pushes. External diff programs and text conversion
are disabled. Text previews for untracked files are bounded to 256 KiB; binary
files, sensitive filename shapes and unsafe links are not displayed. Excessively
large Git output is an explicit error rather than an empty/clean result. Source
changes during a read can require retry; this is a live view, not an immutable
review snapshot.

## Validation and release boundary

Run `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`npm --prefix frontend run check`, `npm --prefix frontend test`, and
`npm --prefix frontend run build:static`.

Rust tests use temporary history sources and Git repositories. Browser tests
use synthetic data or mocked Tauri commands and exercise filtering, recovery,
staged/unstaged selectors, safe text rendering, request races, localization and
keyboard behavior. These checks do not establish real provider authentication,
SSH-server compatibility, packaged Tauri/WebView2 acceptance, or a newly published
installer. Existing Beta installers do not automatically gain these changes.
