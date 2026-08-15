import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(scriptDir, "..", "..");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    if (result.stdout) process.stderr.write(result.stdout);
    throw new Error(`${command} exited with ${result.status}`);
  }
  return result.stdout.trim();
}

const targetTriple = run("rustc", ["--print", "host-tuple"]);
const runtimeByTarget = new Map([
  ["x86_64-pc-windows-msvc", "win-x64"],
  ["aarch64-pc-windows-msvc", "win-arm64"],
  ["x86_64-unknown-linux-gnu", "linux-x64"],
  ["aarch64-unknown-linux-gnu", "linux-arm64"],
  ["x86_64-apple-darwin", "osx-x64"],
  ["aarch64-apple-darwin", "osx-arm64"],
]);
const runtime = runtimeByTarget.get(targetTriple);
if (!runtime) {
  throw new Error(`unsupported Rust host target for the scanner sidecar: ${targetTriple}`);
}

const extension = targetTriple.includes("windows") ? ".exe" : "";
const publishDir = join(repoRoot, "artifacts", "sidecar", targetTriple);
const publishedBinary = join(publishDir, `sessionatlas${extension}`);
const sidecarBinary = join(
  repoRoot,
  "src-tauri",
  "binaries",
  `sessionatlas-${targetTriple}${extension}`,
);

const sourceInputs = [
  join(repoRoot, "SessionAtlas.csproj"),
  join(repoRoot, "Program.cs"),
  join(repoRoot, "CLI"),
  join(repoRoot, "Core"),
  join(repoRoot, "Models"),
];

function newestMtime(path) {
  const stat = statSync(path);
  if (!stat.isDirectory()) return stat.mtimeMs;
  const result = spawnSync("git", ["ls-files", `${path.slice(repoRoot.length + 1).replaceAll("\\", "/")}/**`], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  if (result.status !== 0) throw new Error("failed to enumerate scanner sources");
  return result.stdout
    .split(/\r?\n/)
    .filter(Boolean)
    .map((file) => statSync(join(repoRoot, file)).mtimeMs)
    .reduce((latest, current) => Math.max(latest, current), stat.mtimeMs);
}

const newestSource = Math.max(...sourceInputs.map(newestMtime));
const sidecarIsFresh =
  existsSync(sidecarBinary) && statSync(sidecarBinary).mtimeMs >= newestSource;

if (!sidecarIsFresh) {
  mkdirSync(publishDir, { recursive: true });
  run("dotnet", [
    "publish",
    "SessionAtlas.csproj",
    "-c",
    "Release",
    "-r",
    runtime,
    "--self-contained",
    "true",
    "--nologo",
    "-o",
    publishDir,
  ]);
  if (!existsSync(publishedBinary)) {
    throw new Error(`dotnet publish did not create ${publishedBinary}`);
  }
  mkdirSync(dirname(sidecarBinary), { recursive: true });
  copyFileSync(publishedBinary, sidecarBinary);
}

process.stdout.write(`${sidecarBinary}\n`);
