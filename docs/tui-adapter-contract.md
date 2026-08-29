# SessionAtlas TUI adapter contract (API v1)

SessionAtlas models every AI terminal tool through a declarative adapter. The
adapter owns tool identity, version detection, new/resume argv, optional
package-manager metadata, scanner selection, platform support, and remote
support. Core process, PTY, SSH, quoting, storage, and permission boundaries
remain inside SessionAtlas.

API v1 deliberately does **not** load JavaScript, native libraries, WASM, shell
scripts, hooks, or arbitrary installer commands. This keeps an adapter update
smaller and safer than an application update while preserving one trusted
execution boundary.

## Manifest

An adapter is one UTF-8 JSON file named `adapter.json`:

```json
{
  "apiVersion": 1,
  "id": "myagent",
  "name": "My Agent",
  "adapterVersion": "1.0.0",
  "command": "myagent",
  "versionArgs": ["--version"],
  "launch": {
    "newArgs": [],
    "resumeArgs": ["--resume", "{sessionId}"]
  },
  "manager": "npm",
  "package": "@example/myagent",
  "scanner": {
    "handler": "metadata-v1",
    "dataDirectory": "~/.myagent/sessions"
  },
  "platforms": ["windows", "macos", "linux"],
  "supportsRemote": true
}
```

Fields:

| Field | Contract |
| --- | --- |
| `apiVersion` | Must be integer `1`. |
| `id` | Stable lowercase tool key accepted by SessionAtlas tool-key validation; maximum 64 characters. |
| `name` | Display label; maximum 128 characters and no control characters. |
| `adapterVersion` | Semantic version for the **adapter contract**, independent of the installed TUI version. |
| `command` | Exactly one executable token. Shell programs, script wrappers, shell metacharacters, base arguments, and unbalanced quoting are rejected. |
| `versionArgs` | Exactly one safe version argument: `--version`, `-V`, `-v`, or `version`. Defaults to `--version`; evaluator-style arguments are rejected. |
| `launch.newArgs` | Fixed argv for a new session. |
| `launch.resumeArgs` | Fixed argv for resume. When non-empty it must contain exactly one complete `{sessionId}` token. No other placeholders are accepted. |
| `manager` / `package` | Optional fixed one-click installer. Both must be present together. API v1 accepts only `npm` or `uv` and validates the package name. |
| `scanner.handler` | One bounded scanner implementation selected by name. See below. |
| `platforms` | Non-empty unique subset of `windows`, `macos`, and `linux`. |
| `supportsRemote` | Whether detection, installation, upgrade, and launch are allowed over a configured SSH machine. |

Unknown fields are rejected so a misspelling cannot silently weaken the
contract. The manifest is limited to 256 KiB.

## Scanner handlers

- `builtin.claude`, `builtin.codex`, `builtin.kimi`, `builtin.opencode`,
  `builtin.aider`, and `builtin.pi` bridge the mature built-in parsers. They are
  accepted only when the handler suffix exactly matches the official adapter
  ID and cannot override a data directory.
- `metadata-v1` is the extension handler for user adapters. It requires
  `scanner.dataDirectory`. Each direct child directory represents one project.
  An optional `metadata.json` in that directory may provide `project_path` (or
  `cwd`), `last_accessed`, and `id` (or `session_id`). Malformed metadata
  degrades to directory metadata with a scan diagnostic rather than executing
  code.

Adding a parser for a new proprietary session format still requires a reviewed
SessionAtlas core change. A manifest cannot grant itself filesystem parsing or
code-execution capabilities.

API v1 also keeps icons, colors, and official quick-command shortcuts as
presentation concerns. Unknown adapters receive a generated monogram/color and
no inherited shell shortcuts; those fields are not an execution extension
surface.

## Registry, activation, and rollback

Six official manifests live in `adapters/official/` and are compiled into both
the CLI and desktop app as the offline baseline. Imported versions are stored
immutably at:

```text
~/.sessionatlas/adapters/<id>/<adapterVersion>/adapter.json
```

The settings page accepts an absolute path to a local `adapter.json`, explains
that the declared executable will be version-probed, asks for confirmation,
validates the file in the Rust backend, stages it with create-new and atomic
rename semantics, then records the active version in
`~/.sessionatlas/config.json`. A version directory is never overwritten.
Re-importing different bytes under the same ID/version fails.

An imported update must be newer than the active semantic version. The UI can
switch back to the nearest older installed version and later reactivate the
newest installed version. Rollback changes only the active-version pointer; it
does not delete manifests, TUI packages, sessions, or index data.

API v1 uses explicit local manifest import. It does not yet provide an online
marketplace, automatic adapter download, or signature trust store. Users must
confirm the import and should accept manifests only from a trusted source.
Although a manifest cannot embed executable code, it selects which already
installed program SessionAtlas will probe and later launch. Automatic probes
are restricted to one executable and one recognized version argument; launch
arguments are used only when the user explicitly starts a session.

## Per-machine selection and TUI lifecycle

Adapter activation and TUI installation are separate version domains:

- `adapterVersion` controls SessionAtlas integration behavior.
- the detected command version controls the installed third-party TUI package.

Local adapter selections are stored in `config.json`; remote selections are
stored per SSH server in `prefs.db`. Bundled adapters are selected by default,
while newly imported custom adapters start unselected. Selection alone never
makes a command launchable: the backend requires the adapter to support the
machine and the TUI executable to be detected before it can be enabled.

If a manifest declares a supported fixed package manager, “Install” runs that
fixed npm/uv operation after showing the exact manager/package and receiving
confirmation, re-probes the executable, and only then enables it. A manifest
without installer metadata remains usable, but the
user must install its CLI manually and detect it again. TUI update checks and
TUI package upgrades remain explicit, confirmed operations and are independent
from adapter imports and rollbacks.

## Execution safety

- Local detection/install/upgrade launches use program-plus-argv
  `ProcessSpec` values; adapter text never becomes a local shell command.
- Automatic version detection accepts only one executable token followed by
  one of `--version`, `-V`, `-v`, or `version`; interpreter evaluator forms such
  as `python -c` and `node -e` cannot pass manifest validation.
- Remote operations are assembled only from a validated manifest and fixed
  npm/uv templates, then losslessly POSIX-quoted by trusted backend code.
- The frontend sends only `toolKey`, optional server ID, or an absolute local
  manifest path. It cannot send process argv or installer shell text.
- `platforms` and `supportsRemote` are enforced during detection, enable,
  install, upgrade, scan, and launch—not merely displayed as metadata.
- Unknown IDs, disabled adapters, unsupported machines, unsafe session IDs,
  and missing executables fail closed before a TUI launch.

The broader process and SSH rules are documented in
[`execution-security-contract.md`](./execution-security-contract.md).

## Verification checklist

For adapter-related changes, run:

```bash
cargo test -p sessionatlas-core adapter
cargo test -p sessionatlas-core --test launcher_contract
cargo test -p sessionatlas-tauri
npm --prefix frontend run check
npm --prefix frontend run test:browser -- tests/browser/tui-tools.spec.js
```

The tests cover official launch/resume contracts, unsafe manifest rejection,
immutable install and rollback selection, scanner-factory registration,
fixed local/remote install commands, per-machine enable gates, background
probes, and the settings UI import/activate/rollback flow without installing a
real TUI.
