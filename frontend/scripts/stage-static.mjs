import { cp, mkdir, rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const frontendRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputRoot = resolve(frontendRoot, "dist");
const files = [
  "index.html",
  "app.js",
  "core.js",
  "i18n.js",
  "icons.js",
  "lang-init.js",
  "theme-init.js",
  "styles.css",
];

await rm(outputRoot, { recursive: true, force: true });
await mkdir(outputRoot, { recursive: true });
for (const file of files)
  await cp(resolve(frontendRoot, file), resolve(outputRoot, file));
await cp(resolve(frontendRoot, "vendor"), resolve(outputRoot, "vendor"), { recursive: true });
console.log(`staged ${files.length} frontend files and vendor assets into ${outputRoot}`);
