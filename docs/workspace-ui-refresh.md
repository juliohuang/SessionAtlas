# Workspace UI refresh

## Design intent

The September 2026 refresh keeps the existing charcoal/lime identity and all
project, adapter, SSH and terminal workflows. It does not add a frontend
framework, a network font, or an image-based application surface.

A concept was generated with the built-in image generation tool using only
synthetic project names and terminal text. Local design artifacts are in the
ignored `artifacts/ui-design/` directory (`workspace-concept.png` and
`prompt.txt`). The concept is a visual reference, not an application screenshot
or evidence that its example terminal commands ran.

## Layout and interaction contracts

- Search and global actions are separate regions; scan progress still uses the
  existing busy state, progress bar and accessible attributes.
- Project names own the first row. Device/OS, tool, activity time, terminal
  status and Git warnings occupy a wrapping metadata row. Long names retain a
  full-text tooltip. Hover and keyboard focus reveal row actions.
- Desktop columns reserve useful width for projects and a compact overview;
  remaining width belongs to the terminal. The overview restores from a 40px
  collapsed rail. The supported minimum window remains 980 × 640.
- Below 1180px, tool filters and sort controls split into separate rows.
  Expanded sessions, overview and settings keep their own scrolling surfaces.
- Git background updates patch the metadata row in place and refresh virtual
  row measurements; they must not recreate queue inputs or steal focus.
- Terminal status keeps distinct shape icons and accessible descriptions;
  color is supplementary. Existing vendored tool artwork remains unchanged.
- Dark and light themes use shared tokens, including foreground contrast for
  filled primary actions. Interface copy uses a readable sans-serif stack;
  paths, terminal output and commands retain monospace. Decorative grain is
  removed, and busy/status motion honors reduced-motion preferences.

## Verification

Run from `frontend/`:

```powershell
npm.cmd run check
npm.cmd run test:unit
npm.cmd run test:browser
npm.cmd run build:static
```

Browser tests use the demo or mocked Tauri commands. They do not clean actual
sessions, contact SSH machines, or install/upgrade real tools. Browser visual
acceptance does not replace a packaged Tauri/WebView2 acceptance run.
