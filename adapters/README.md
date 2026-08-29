# SessionAtlas adapters

`official/` contains the six declarative API v1 manifests compiled into
`sessionatlas-core`. They are the offline baseline for scanning, command
detection, launch/resume, installation, update, platform, and remote behavior.

Do not add executable scripts, native libraries, or generated package-manager
commands here. Adapter schema and security rules are documented in
[`docs/tui-adapter-contract.md`](../docs/tui-adapter-contract.md).

User-imported versions are runtime data and belong under
`~/.sessionatlas/adapters/<id>/<version>/adapter.json`, never in this source
directory.
