# Third-party notices

SessionAtlas includes and links against third-party open-source software.
This notice supplements, and does not replace, the license terms supplied by
those projects.

## Vendored browser assets

The following files are redistributed in `frontend/vendor/` so the desktop app
does not need a CDN at runtime:

| Component | Version | License | Local license text |
| --- | --- | --- | --- |
| `@xterm/xterm` | 6.0.0 | MIT | `frontend/vendor/licenses/xterm-LICENSE.txt` |
| `@xterm/addon-fit` | 0.11.0 | MIT | `frontend/vendor/licenses/addon-fit-LICENSE.txt` |
| `@xterm/addon-web-links` | 0.12.0 | MIT | `frontend/vendor/licenses/addon-web-links-LICENSE.txt` |
| `@highlightjs/cdn-assets` | 11.12.0 | BSD-3-Clause | `frontend/vendor/licenses/highlightjs-LICENSE.txt` |

Exact asset hashes and the reproducible copy command are documented in
`frontend/vendor/README.md`. Run `npm ci && npm run sync:vendor` from
`frontend/` to reproduce the checked-in files.

## Compiled dependencies

The .NET, Rust, and development-only JavaScript dependencies used to build the
application are declared in `*.csproj`, `src-tauri/Cargo.lock`, and
`frontend/package-lock.json`. Their upstream copyright and license terms
continue to apply. Release automation produces an SPDX software bill of
materials so recipients can inspect the complete dependency inventory for a
specific build.

SessionAtlas does not claim ownership of third-party names, trademarks, or
logos. Claude, Codex, Kimi, OpenCode, Aider, Tauri, xterm.js, and other names
belong to their respective owners.
