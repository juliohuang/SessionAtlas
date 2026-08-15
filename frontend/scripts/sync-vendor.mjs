import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const frontendRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const vendorRoot = join(frontendRoot, "vendor");
const licensesRoot = join(vendorRoot, "licenses");
mkdirSync(licensesRoot, { recursive: true });

const copies = [
  ["node_modules/@xterm/xterm/lib/xterm.js", "vendor/xterm.js"],
  ["node_modules/@xterm/xterm/css/xterm.css", "vendor/xterm.css"],
  ["node_modules/@xterm/addon-fit/lib/addon-fit.js", "vendor/addon-fit.js"],
  [
    "node_modules/@xterm/addon-web-links/lib/addon-web-links.js",
    "vendor/xterm-addon-web-links.js",
  ],
  ["node_modules/@highlightjs/cdn-assets/highlight.min.js", "vendor/highlight.min.js"],
  ["node_modules/@xterm/xterm/LICENSE", "vendor/licenses/xterm-LICENSE.txt"],
  ["node_modules/@xterm/addon-fit/LICENSE", "vendor/licenses/addon-fit-LICENSE.txt"],
  [
    "node_modules/@xterm/addon-web-links/LICENSE",
    "vendor/licenses/addon-web-links-LICENSE.txt",
  ],
  [
    "node_modules/@highlightjs/cdn-assets/LICENSE",
    "vendor/licenses/highlightjs-LICENSE.txt",
  ],
];

for (const [source, destination] of copies) {
  copyFileSync(join(frontendRoot, source), join(frontendRoot, destination));
}

process.stdout.write(`updated ${copies.length} vendored files\n`);
