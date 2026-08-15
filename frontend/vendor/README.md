# Vendored frontend libraries

These files are loaded locally by `index.html`; production must not fetch them from a CDN.

| File | Upstream package | Version | SHA-256 |
| --- | --- | --- | --- |
| `xterm.js` | `@xterm/xterm` | 6.0.0 | `14903579FF54664CD72F8E8699E6961A6272C21863EC1C3B118CDC8AF5D4A972` |
| `xterm.css` | `@xterm/xterm` | 6.0.0 | `854A7C0FB70E8B1A083C16797AB827299FB18744F5AD34F227B48337E33293C6` |
| `xterm-addon-web-links.js` | `@xterm/addon-web-links` | 0.12.0 | `B74864125EF3889753B94E61EB4DABB99B2A27F920F0C9A7BD53CBE6D10F032A` |
| `addon-fit.js` | `@xterm/addon-fit` | 0.11.0 | `BA3EA256CE0620A0992A197D6C9BAEA64823FC93D8DA07A9E366CA9943C18527` |
| `highlight.min.js` | `@highlightjs/cdn-assets` | 11.12.0 | `8AB71EB09C51F501E5E25157D9CFF100E46CC29BCBFC744D0B746D451FCA7F53` |

Run `npm run sync:vendor` after changing the exact versions in
`frontend/package.json`. The corresponding MIT and BSD-3-Clause license texts
are stored in `vendor/licenses/` and summarized in the repository-level
`THIRD_PARTY_NOTICES.md`.
