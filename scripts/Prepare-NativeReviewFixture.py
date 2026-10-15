#!/usr/bin/env python3
"""Build an isolated, synthetic SessionAtlas native-review fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys


REPO = Path(__file__).resolve().parents[1]
ALLOWED_FIXTURES = (REPO / "artifacts" / "native-0.1.1-20261015").resolve()
MANIFEST_NAME = "native-review-fixture.json"
FIXTURE_VERSION = 1


def fail(message: str) -> "NoReturn":
    raise RuntimeError(message)


def under(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_checked(args: list[str], *, cwd: Path, env: dict[str, str]) -> None:
    result = subprocess.run(args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        fail(f"Fixture command failed with exit code {result.returncode}: {Path(args[0]).name}")


def git(git_exe: str, args: list[str], *, cwd: Path, env: dict[str, str], hooks: Path) -> None:
    run_checked(
        [git_exe, "-c", f"core.hooksPath={hooks}", "-c", "commit.gpgSign=false", *args],
        cwd=cwd,
        env=env,
    )


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_codex_session(path: Path, session_id: str, cwd: Path, title: str, when: str,
                        *, thread_source: str | None = None,
                        guardian: bool = False) -> None:
    payload: dict[str, object] = {
        "id": session_id,
        "cwd": str(cwd),
        "timestamp": when,
        "title": title,
        "cli": "codex",
    }
    if thread_source:
        payload["thread_source"] = thread_source
    if guardian:
        payload["source"] = {"subagent": {"other": "guardian"}}
    rows = [
        {"timestamp": when, "type": "session_meta", "payload": payload},
        {
            "timestamp": when,
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": f"Synthetic review fixture: {title}",
            },
        },
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows), encoding="utf-8")


def init_project(project: Path, git_exe: str, git_env: dict[str, str], hooks: Path,
                 name: str) -> dict[str, str]:
    project.mkdir(parents=True)
    git(git_exe, ["init", "--quiet", "--initial-branch=main"], cwd=project, env=git_env, hooks=hooks)
    (project / "docs").mkdir()
    (project / "docs" / "README.md").write_text(
        f"# {name}\n\nSynthetic native acceptance project.\n", encoding="utf-8"
    )
    (project / "review.txt").write_text("baseline line\n", encoding="utf-8")
    (project / "docs" / "rename-before.md").write_text("Rename fixture baseline.\n", encoding="utf-8")
    git(git_exe, ["add", "docs/README.md", "docs/rename-before.md", "review.txt"],
        cwd=project, env=git_env, hooks=hooks)
    git(git_exe, ["commit", "--quiet", "-m", "Create synthetic review baseline"],
        cwd=project, env=git_env, hooks=hooks)

    (project / "review.txt").write_text("baseline line\nstaged review line\n", encoding="utf-8")
    old_name = project / "docs" / "rename-before.md"
    new_name = project / "docs" / "rename-after.md"
    old_name.rename(new_name)
    git(git_exe, ["add", "review.txt", "docs/rename-before.md", "docs/rename-after.md"],
        cwd=project, env=git_env, hooks=hooks)
    (project / "review.txt").write_text(
        "baseline line\nstaged review line\nunstaged review line\n", encoding="utf-8"
    )
    (project / "notes-untracked.txt").write_text("Synthetic untracked text.\n", encoding="utf-8")
    (project / "sample.bin").write_bytes(bytes([0, 1, 2, 255, 0, 128]))
    (project / ".env.fixture").write_text("FIXTURE_ONLY=fake-value-not-a-secret\n", encoding="utf-8")
    return {
        "path": str(project),
        "stagedSnippet": "staged review line",
        "unstagedSnippet": "unstaged review line",
        "renamedPath": "docs/rename-after.md",
        "untrackedPath": "notes-untracked.txt",
        "binaryPath": "sample.bin",
        "fakeEnvPath": ".env.fixture",
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("isolated_root", type=Path)
    parser.add_argument("--scanner", required=True, type=Path,
                        help="absolute release sessionatlas.exe inside this repository")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = args.isolated_root.expanduser()
    if not root.is_absolute():
        fail("The isolated root must be an absolute path.")
    root = root.resolve()
    if not under(root, ALLOWED_FIXTURES):
        fail(f"The isolated root must be under {ALLOWED_FIXTURES}")
    if root == ALLOWED_FIXTURES:
        fail("Use a dedicated child directory under the native acceptance artifact root.")
    if root.exists() and any(root.iterdir()):
        fail("The isolated root is non-empty; refusing to overwrite or remove its contents.")

    scanner = args.scanner
    if not scanner.is_absolute():
        fail("--scanner must be an absolute path.")
    scanner = scanner.resolve(strict=True)
    release_dir = (REPO / "target" / "release").resolve()
    if scanner.name.casefold() != "sessionatlas.exe" or not under(scanner, release_dir):
        fail("--scanner must be target/release/sessionatlas.exe in this repository.")
    if not scanner.is_file():
        fail("The release scanner path is not a regular file.")

    rust_source = (REPO / "scripts" / "native-acceptance-tui.rs").resolve(strict=True)
    git_exe = shutil.which("git")
    rustc_exe = shutil.which("rustc")
    if not git_exe or not rustc_exe:
        fail("Fixture preparation requires the existing git and rustc executables on PATH.")

    root.mkdir(parents=True, exist_ok=True)
    projects_dir = root / "projects"
    tools_dir = root / "tools"
    metadata_dir = root / "tool-metadata"
    sessionatlas_dir = root / ".sessionatlas"
    codex_sessions = root / ".codex" / "sessions" / "2026" / "10" / "15"
    hooks = root / "empty-hooks"
    hooks.mkdir()
    projects_dir.mkdir()
    tools_dir.mkdir()
    metadata_dir.mkdir()
    sessionatlas_dir.mkdir()
    codex_sessions.mkdir(parents=True)

    git_config = root / "gitconfig-empty"
    git_config.write_text("", encoding="utf-8")
    # Git location/config overrides must never redirect writes out of the fixture.
    git_env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith("GIT_")}
    git_env.update({
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": str(git_config),
        "GIT_AUTHOR_NAME": "SessionAtlas Synthetic Fixture",
        "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
        "GIT_COMMITTER_NAME": "SessionAtlas Synthetic Fixture",
        "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
    })
    projects = {
        "atlas-alpha": init_project(projects_dir / "atlas-alpha", git_exe, git_env, hooks, "Atlas Alpha"),
        "atlas-beta": init_project(projects_dir / "atlas-beta", git_exe, git_env, hooks, "Atlas Beta"),
    }

    session_specs = [
        ("alpha-early", "atlas-alpha early review", projects["atlas-alpha"]["path"], "2026-10-14T08:00:00Z"),
        ("alpha-latest", "atlas-alpha latest review", projects["atlas-alpha"]["path"], "2026-10-15T09:30:00Z"),
        ("beta-review", "atlas-beta review", projects["atlas-beta"]["path"], "2026-10-15T08:15:00Z"),
    ]
    for session_id, title, project_path, when in session_specs:
        write_codex_session(codex_sessions / f"rollout-{session_id}.jsonl", session_id,
                            Path(project_path), title, when)
    write_codex_session(codex_sessions / "rollout-child.jsonl", "excluded-child",
                        Path(projects["atlas-alpha"]["path"]), "excluded child", "2026-10-15T09:45:00Z",
                        thread_source="child")
    write_codex_session(codex_sessions / "rollout-guardian.jsonl", "excluded-guardian",
                        Path(projects["atlas-beta"]["path"]), "excluded guardian", "2026-10-15T09:50:00Z",
                        guardian=True)

    for project_name, project_data in projects.items():
        metadata = metadata_dir / project_name
        metadata.mkdir()
        write_json(metadata / "metadata.json", {
            "project_path": project_data["path"],
            "last_accessed": "2026-10-15T10:00:00Z",
        })

    adapter_dir = sessionatlas_dir / "adapters" / "acceptance-tui" / "1.0.0"
    write_json(adapter_dir / "adapter.json", {
        "apiVersion": 1,
        "id": "acceptance-tui",
        "name": "Synthetic Acceptance TUI",
        "adapterVersion": "1.0.0",
        "command": "acceptance-tui",
        "versionArgs": ["--version"],
        "launch": {"newArgs": [], "resumeArgs": []},
        "scanner": {"handler": "metadata-v1", "dataDirectory": str(metadata_dir)},
        "platforms": ["windows"],
        "supportsRemote": False,
    })
    write_json(sessionatlas_dir / "config.json", {
        "CustomTools": [],
        "EnabledAdapters": ["codex", "acceptance-tui"],
        "ActiveAdapterVersions": {"acceptance-tui": "1.0.0"},
        "PreferredToolsByPath": {project_data["path"]: "acceptance-tui" for project_data in projects.values()},
        "DefaultTerminal": "auto",
    })

    tool_path = tools_dir / "acceptance-tui.exe"
    compile_env = os.environ.copy()
    compile_env["SESSIONATLAS_FIXTURE_ROOT"] = str(root)
    run_checked([rustc_exe, "--edition=2021", str(rust_source), "-o", str(tool_path)],
                cwd=root, env=compile_env)

    system_root = Path(os.environ.get("SystemRoot", r"C:\Windows"))
    system32 = system_root / "System32"
    git_cmd = Path(git_exe).resolve().parent
    clean_path = os.pathsep.join(str(path) for path in (tools_dir, system32, git_cmd))
    scan_env = {
        key: value for key, value in os.environ.items()
        if key.casefold() in {"systemroot", "windir", "comspec", "temp", "tmp"}
    }
    scan_env.update({
        "SESSIONATLAS_HOME": str(root),
        "PI_CODING_AGENT_DIR": str(root / "pi"),
        "PI_CODING_AGENT_SESSION_DIR": str(root / "pi" / "sessions"),
        "PATH": clean_path,
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": str(git_config),
    })
    run_checked([str(scanner), "scan"], cwd=root, env=scan_env)

    index_path = root / ".sessionatlas" / "index.db"
    if not index_path.is_file():
        fail("The release scanner did not create the isolated fixture index.")
    with sqlite3.connect(f"file:{index_path.as_posix()}?mode=ro", uri=True) as connection:
        indexed_paths = {row[0] for row in connection.execute("SELECT path FROM projects")}
        usages = {
            (row[0], row[1]): (row[2], row[3])
            for row in connection.execute(
                "SELECT projects.path, tool_usages.tool_key, tool_usages.session_count, "
                "tool_usages.last_session_id FROM projects JOIN tool_usages "
                "ON projects.id = tool_usages.project_id"
            )
        }
    expected_paths = {project_data["path"] for project_data in projects.values()}
    if indexed_paths != expected_paths:
        fail("The isolated release scan did not index exactly the two synthetic projects.")
    expected_codex = {
        (projects["atlas-alpha"]["path"], "codex"): (2, "alpha-latest"),
        (projects["atlas-beta"]["path"], "codex"): (1, "beta-review"),
    }
    if any(usages.get(key) != value for key, value in expected_codex.items()):
        fail("The isolated release scan did not retain the expected primary Codex sessions.")
    if any(key[1] == "codex" and value[1] in {"excluded-child", "excluded-guardian"}
           for key, value in usages.items()):
        fail("The isolated release scan indexed a synthetic child or guardian session.")
    if any((project_data["path"], "acceptance-tui") not in usages for project_data in projects.values()):
        fail("The isolated release scan did not enable the synthetic metadata adapter.")

    files: dict[str, str] = {}
    hash_roots = [
        projects_dir,
        root / ".codex",
        metadata_dir,
        sessionatlas_dir / "adapters",
        sessionatlas_dir / "config.json",
        index_path,
        tools_dir,
    ]
    for base in hash_roots:
        candidates = sorted(base.rglob("*")) if base.is_dir() else [base]
        for file_path in candidates:
            if file_path.is_file() and ".git" not in file_path.parts:
                files[str(file_path.relative_to(root))] = sha256(file_path)
    manifest = {
        "fixtureVersion": FIXTURE_VERSION,
        "fixtureRoot": str(root),
        "scannerPath": str(scanner),
        "projectRecords": projects,
        "codexSessionSource": str(codex_sessions),
        "metadataAdapterSource": str(metadata_dir),
        "configPath": str(sessionatlas_dir / "config.json"),
        "indexPath": str(index_path),
        "expectedSessionIds": [spec[0] for spec in session_specs],
        "excludedSessionIds": ["excluded-child", "excluded-guardian"],
        "expectedDiffSnippets": {
            name: {"staged": project_data["stagedSnippet"], "unstaged": project_data["unstagedSnippet"]}
            for name, project_data in projects.items()
        },
        "sha256ByRelativePath": files,
        "scanProjectCount": 2,
        "separationNotice": "All project paths, Git metadata, session records, tool output, and scan index are synthetic fixture data.",
    }
    write_json(root / MANIFEST_NAME, manifest)
    print(f"Synthetic fixture root: {root}")
    print("Scanned synthetic project count: 2")
    print("No user-home session data or logs were read for fixture output.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # Keep scanner output and ambient paths out of errors.
        print(f"Fixture preparation stopped: {error}", file=sys.stderr)
        raise SystemExit(2)
