#!/usr/bin/env python3
"""Read-only cross-checks for the 0.1.1 native acceptance evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sqlite3
import subprocess
import sys
from typing import Any


REPO = Path(__file__).resolve().parents[1]
ARTIFACTS = (REPO / "artifacts" / "native-0.1.1-20261015").resolve()
OUTPUT_NAME = "native-review-verification.json"
EXCLUDED_FIXTURE_HASHES = {
    ".sessionatlas/index.db": "setup/output database; verified separately with SQLite mode=ro",
    "tool-metadata/atlas-alpha/metadata.json": "expected metadata id removal; validated semantically",
    "tool-metadata/atlas-beta/metadata.json": "expected metadata id removal; validated semantically",
}


class VerificationError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def is_within(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def safe_evidence_file(value: Path, evidence_root: Path, label: str) -> Path:
    if not value.is_absolute():
        value = evidence_root / value
    resolved = value.resolve(strict=True)
    if not is_within(resolved, evidence_root):
        raise VerificationError(f"{label} must resolve inside the evidence root")
    if not resolved.is_file():
        raise VerificationError(f"{label} must be a regular file")
    return resolved


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as stream:
        return json.load(stream)


def normalize_rel(value: str) -> str:
    return PurePosixPath(value.replace("\\", "/")).as_posix()


def fixture_path(fixture: Path, relative: str) -> Path:
    normalized = normalize_rel(relative)
    if normalized.startswith("/") or any(part in ("", ".", "..") for part in normalized.split("/")):
        raise VerificationError("fixture manifest contains an unsafe relative path")
    candidate = (fixture / Path(*normalized.split("/"))).resolve(strict=True)
    if not is_within(candidate, fixture):
        raise VerificationError("fixture manifest path escapes the fixture root")
    return candidate


def baseline_delta(before: Path, after: Path) -> dict[str, Any]:
    left = load_json(before)
    right = load_json(after)
    changes = []
    left_files = left.get("files", {})
    right_files = right.get("files", {})
    for name in sorted(set(left_files) | set(right_files)):
        old = left_files.get(name, {})
        new = right_files.get(name, {})
        if old != new:
            changes.append({
                "file": name,
                "beforeExists": old.get("exists", False),
                "afterExists": new.get("exists", False),
                "beforeBytes": old.get("bytes"),
                "afterBytes": new.get("bytes"),
                "hashChanged": old.get("sha256") != new.get("sha256"),
            })

    def installs(snapshot: dict[str, Any]) -> list[dict[str, Any]]:
        return sorted(
            ({
                "displayName": entry.get("values", {}).get("DisplayName"),
                "displayVersion": str(entry.get("values", {}).get("DisplayVersion", "")),
            } for entry in snapshot.get("uninstall", [])),
            key=lambda item: (str(item["displayName"]), str(item["displayVersion"])),
        )

    old_installs = installs(left)
    new_installs = installs(right)
    return {
        "fileChanges": changes,
        "installEntriesBefore": old_installs,
        "installEntriesAfter": new_installs,
        "shortcutCountBefore": len(left.get("shortcuts", {})),
        "shortcutCountAfter": len(right.get("shortcuts", {})),
        "installVersionChanged": old_installs != new_installs,
    }


def bundle_artifacts() -> dict[str, Any]:
    bundle = REPO / "target" / "release" / "bundle"
    candidates = {
        "msi": sorted((bundle / "msi").glob("*0.1.1*.msi")),
        "nsis": sorted((bundle / "nsis").glob("*0.1.1*.exe")),
    }
    result: dict[str, Any] = {}
    for kind, files in candidates.items():
        if len(files) != 1 or not files[0].is_file():
            result[kind] = {"passed": False, "candidateCount": len(files)}
            continue
        path = files[0].resolve(strict=True)
        result[kind] = {
            "passed": True,
            "file": path.relative_to(REPO).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
        }
    return result


def verify_payload(payload: Path, after: Path) -> dict[str, Any]:
    manifest_path = payload / "payload-manifest.json"
    if not manifest_path.is_file():
        raise VerificationError("payload manifest is missing")
    manifest = load_json(manifest_path)
    files = manifest.get("files", [])
    msi_result = bundle_artifacts()["msi"]
    msi_matches = bool(
        msi_result.get("passed")
        and msi_result.get("sha256") == manifest.get("sha256")
        and Path(str(manifest.get("package", ""))).name == Path(msi_result.get("file", "")).name
    )

    unpacked = []
    for item in files:
        name = item.get("name")
        if not isinstance(name, str) or Path(name).name != name:
            unpacked.append({"name": str(name), "passed": False, "reason": "unsafe manifest filename"})
            continue
        path = (payload / name).resolve(strict=False)
        if not is_within(path, payload) or not path.is_file():
            unpacked.append({"name": name, "passed": False, "reason": "payload file missing or outside payload directory"})
            continue
        actual = sha256(path)
        unpacked.append({
            "name": name,
            "passed": actual == item.get("sha256") and path.stat().st_size == item.get("bytes"),
            "bytes": path.stat().st_size,
            "sha256": actual,
            "expectedBytes": item.get("bytes"),
            "expectedSha256": item.get("sha256"),
        })

    after_data = load_json(after)
    install_entries = [
        item for item in after_data.get("uninstall", [])
        if item.get("values", {}).get("DisplayName") == manifest.get("properties", {}).get("ProductName")
    ]
    version_matches = any(
        str(item.get("values", {}).get("DisplayVersion", "")) == str(manifest.get("properties", {}).get("ProductVersion", ""))
        for item in install_entries
    )
    install_dir_value = next((
        item.get("values", {}).get("InstallLocation")
        for item in install_entries
        if item.get("values", {}).get("InstallLocation")
    ), None)
    installed = []
    if install_dir_value:
        install_dir = Path(str(install_dir_value).strip('"'))
        try:
            resolved_install_dir = install_dir.resolve(strict=True)
        except OSError:
            resolved_install_dir = None
        for item in files:
            name = item.get("name")
            if not isinstance(name, str) or Path(name).name != name:
                continue
            candidate = install_dir / name
            try:
                path = candidate.resolve(strict=True)
                exists = (
                    path.is_file()
                    and resolved_install_dir is not None
                    and path.parent == resolved_install_dir
                )
            except OSError:
                exists = False
                path = candidate
            if not exists:
                installed.append({"name": name, "passed": False, "reason": "installed file missing"})
                continue
            actual = sha256(path)
            installed.append({
                "name": name,
                "passed": actual == item.get("sha256") and path.stat().st_size == item.get("bytes"),
                "bytes": path.stat().st_size,
                "sha256": actual,
                "expectedBytes": item.get("bytes"),
                "expectedSha256": item.get("sha256"),
            })

    return {
        "product": manifest.get("properties", {}).get("ProductName"),
        "productVersion": manifest.get("properties", {}).get("ProductVersion"),
        "manifestPackage": Path(str(manifest.get("package", ""))).name,
        "manifestPackageSha256": manifest.get("sha256"),
        "msiMatchesPayloadManifest": msi_matches,
        "unpackedPayloadFiles": unpacked,
        "installedVersionMatchesPayload": version_matches,
        "installedFiles": installed,
    }


def verify_fixture_hashes(fixture: Path, manifest_path: Path, manifest: dict[str, Any]) -> dict[str, Any]:
    expected = manifest.get("sha256ByRelativePath", {})
    normalized_expected = {normalize_rel(key): value for key, value in expected.items()}
    exclusions = {normalize_rel(key): reason for key, reason in EXCLUDED_FIXTURE_HASHES.items()}
    immutable_results = []
    project_results = []
    separate_results = []
    for relative, expected_hash in sorted(normalized_expected.items()):
        path = fixture_path(fixture, relative)
        actual_hash = sha256(path)
        if relative.startswith("projects/"):
            project_results.append({"path": relative, "passed": actual_hash == expected_hash})
        elif relative in exclusions:
            separate_results.append({
                "path": relative,
                "classification": exclusions[relative],
                "initialManifestSha256": expected_hash,
                "currentSha256": actual_hash,
                "hashChanged": actual_hash != expected_hash,
            })
        else:
            immutable_results.append({"path": relative, "passed": actual_hash == expected_hash})

    metadata_checks = []
    project_records = manifest.get("projectRecords", {})
    for project_name in ("atlas-alpha", "atlas-beta"):
        relative = f"tool-metadata/{project_name}/metadata.json"
        value = load_json(fixture_path(fixture, relative))
        expected_path = project_records.get(project_name, {}).get("path")
        metadata_checks.append({
            "project": project_name,
            "hasResumeId": bool(value.get("id") or value.get("session_id")),
            "projectPathMatchesManifest": value.get("project_path") == expected_path,
            "passed": not (value.get("id") or value.get("session_id")) and value.get("project_path") == expected_path,
        })

    manifest_hash_before = sha256(manifest_path)
    index_relative = ".sessionatlas/index.db"
    index_path = fixture_path(fixture, index_relative)
    index_hash = sha256(index_path)
    return {
        "manifestSha256": manifest_hash_before,
        "projectInputFiles": project_results,
        "immutableSourceFiles": immutable_results,
        "metadataIdRemoval": metadata_checks,
        "indexDatabaseSetupOutput": {
            "path": index_relative,
            "initialManifestSha256": normalized_expected.get(index_relative),
            "currentSha256": index_hash,
            "hashChanged": index_hash != normalized_expected.get(index_relative),
            "excludedFromImmutableInputComparison": True,
        },
        "specialHashDifferences": separate_results,
        "manifestHashAfterCheck": sha256(manifest_path),
        "manifestUnchangedDuringCheck": sha256(manifest_path) == manifest_hash_before,
    }


def git_environment(fixture: Path, git_exe: str) -> tuple[dict[str, str], list[str]]:
    removed = sorted(key for key in os.environ if key.upper().startswith("GIT_"))
    allowed = {"systemroot", "windir", "comspec", "temp", "tmp"}
    env = {key: value for key, value in os.environ.items() if key.casefold() in allowed}
    system_root = Path(env.get("SystemRoot", r"C:\Windows"))
    env["PATH"] = os.pathsep.join((str(system_root / "System32"), str(Path(git_exe).resolve().parent)))
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    global_config = (fixture / "gitconfig-empty").resolve(strict=True)
    if not is_within(global_config, fixture) or not global_config.is_file():
        raise VerificationError("fixture global Git config is missing or escapes the fixture")
    env["GIT_CONFIG_GLOBAL"] = str(global_config)
    env["GIT_OPTIONAL_LOCKS"] = "0"
    return env, removed


def run_git(git_exe: str, args: list[str], project: Path, env: dict[str, str]) -> str:
    result = subprocess.run(
        [git_exe, "--no-optional-locks", *args],
        cwd=project,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    if result.returncode:
        raise VerificationError(f"read-only Git check failed ({Path(args[0]).name}, exit {result.returncode})")
    return result.stdout


def verify_git(fixture: Path, manifest: dict[str, Any]) -> dict[str, Any]:
    git_exe = shutil.which("git")
    if not git_exe:
        raise VerificationError("git executable is not available")
    env, removed = git_environment(fixture, git_exe)
    results = []
    for project_name in ("atlas-alpha", "atlas-beta"):
        project = (fixture / "projects" / project_name).resolve(strict=True)
        git_dir = (project / ".git").resolve(strict=True)
        if not is_within(git_dir, fixture) or not (git_dir / "index").is_file():
            raise VerificationError("fixture Git directory is missing or escapes the fixture")
        index_path = (git_dir / "index").resolve(strict=True)
        if not is_within(index_path, git_dir):
            raise VerificationError("fixture Git index escapes its fixture repository")
        before = sha256(index_path)
        status = run_git(git_exe, ["status", "--porcelain=v1", "--untracked-files=all"], project, env)
        staged = run_git(git_exe, ["diff", "--cached", "--numstat"], project, env)
        unstaged = run_git(git_exe, ["diff", "--numstat"], project, env)
        staged_names = run_git(git_exe, ["diff", "--cached", "--name-status", "--find-renames"], project, env)
        staged_text = run_git(git_exe, ["diff", "--cached", "--", "review.txt"], project, env)
        unstaged_text = run_git(git_exe, ["diff", "--", "review.txt"], project, env)
        after = sha256(index_path)
        statuses = [line[:2] for line in status.splitlines() if line]
        paths = [line[3:] for line in status.splitlines() if line]
        expected_untracked = {".env.fixture", "notes-untracked.txt", "sample.bin"}
        passed = (
            before == after
            and "MM" in statuses
            and sum(code.startswith("R") for code in statuses) == 1
            and {path for code, path in zip(statuses, paths) if code == "??"} == expected_untracked
            and "staged review line" in staged_text
            and "unstaged review line" in unstaged_text
            and "docs/rename-before.md" in staged_names
            and "docs/rename-after.md" in staged_names
            and "review.txt" in staged
            and "review.txt" in unstaged
        )
        results.append({
            "project": project_name,
            "passed": passed,
            "statusCodes": statuses,
            "stagedDiffHasExpectedLine": "staged review line" in staged_text,
            "unstagedDiffHasExpectedLine": "unstaged review line" in unstaged_text,
            "stagedRenamePresent": "docs/rename-before.md" in staged_names and "docs/rename-after.md" in staged_names,
            "untrackedFiles": sorted(path for code, path in zip(statuses, paths) if code == "??"),
            "gitIndexSha256Before": before,
            "gitIndexSha256After": after,
            "gitIndexUnchanged": before == after,
        })
    return {"ambientGitVariablesRemoved": removed, "projects": results}


def codex_metadata(fixture: Path, manifest: dict[str, Any]) -> dict[str, Any]:
    source_value = Path(str(manifest.get("codexSessionSource", "")))
    source = source_value.resolve(strict=True)
    if not is_within(source, fixture):
        raise VerificationError("Codex fixture source escapes the fixture root")
    primary: list[str] = []
    auxiliary: list[str] = []
    for path in sorted(source.glob("*.jsonl")):
        resolved_file = path.resolve(strict=True)
        if not is_within(resolved_file, fixture) or not resolved_file.is_file():
            raise VerificationError("Codex fixture session file escapes the fixture root")
        with resolved_file.open("r", encoding="utf-8") as stream:
            first_line = stream.readline()
        record = json.loads(first_line)
        payload = record.get("payload", {})
        session_id = payload.get("id")
        thread_source = payload.get("thread_source")
        subagent = payload.get("source", {}).get("subagent", {}) if isinstance(payload.get("source"), dict) else {}
        is_aux = thread_source in {"child", "subagent", "guardian", "guardian_review"} or subagent.get("other") == "guardian"
        (auxiliary if is_aux else primary).append(session_id)
    expected_primary = set(manifest.get("expectedSessionIds", []))
    expected_auxiliary = set(manifest.get("excludedSessionIds", []))
    return {
        "primaryIdsMatchManifest": set(primary) == expected_primary,
        "auxiliaryIdsMatchManifest": set(auxiliary) == expected_auxiliary,
        "primaryCount": len(primary),
        "auxiliaryCount": len(auxiliary),
        "primaryIds": sorted(primary),
        "auxiliaryIds": sorted(auxiliary),
    }


def verify_sqlite(fixture: Path, manifest: dict[str, Any]) -> dict[str, Any]:
    db = (fixture / ".sessionatlas" / "index.db").resolve(strict=True)
    before_hash = sha256(db)
    with sqlite3.connect(f"file:{db.as_posix()}?mode=ro", uri=True) as connection:
        project_paths = {row[0] for row in connection.execute("SELECT path FROM projects")}
        usages = connection.execute(
            "SELECT projects.path, tool_usages.tool_key, tool_usages.session_count, "
            "tool_usages.last_session_id FROM projects JOIN tool_usages "
            "ON projects.id = tool_usages.project_id"
        ).fetchall()
    after_hash = sha256(db)
    expected_projects = {
        str(value.get("path")) for value in manifest.get("projectRecords", {}).values()
    }
    norm = lambda paths: {os.path.normcase(os.path.normpath(path)) for path in paths}
    expected_usage = {
        (os.path.normcase(os.path.normpath(str(manifest["projectRecords"]["atlas-alpha"]["path"]))), "codex"): (2, "alpha-latest"),
        (os.path.normcase(os.path.normpath(str(manifest["projectRecords"]["atlas-beta"]["path"]))), "codex"): (1, "beta-review"),
    }
    usage_map = {
        (os.path.normcase(os.path.normpath(row[0])), row[1]): (row[2], row[3])
        for row in usages
    }
    acceptance_present = all(
        (os.path.normcase(os.path.normpath(str(record["path"]))), "acceptance-tui") in usage_map
        and usage_map[(os.path.normcase(os.path.normpath(str(record["path"]))), "acceptance-tui")][0] == 0
        for record in manifest.get("projectRecords", {}).values()
    )
    codex_ok = all(usage_map.get(key) == expected for key, expected in expected_usage.items())
    alpha_norm = os.path.normcase(os.path.normpath(str(manifest["projectRecords"]["atlas-alpha"]["path"])))
    aux_ids = set(manifest.get("excludedSessionIds", []))
    observed_last_ids = {value[1] for (path, tool), value in usage_map.items() if tool == "codex"}
    auxiliary_excluded = (
        codex_ok
        and not (observed_last_ids & aux_ids)
        and observed_last_ids == {"alpha-latest", "beta-review"}
    )
    return {
        "readOnlyMode": True,
        "projectCount": len(project_paths),
        "projectPathsMatchManifest": norm(project_paths) == norm(expected_projects),
        "codexUsages": [
            {"project": "atlas-alpha" if os.path.normcase(os.path.normpath(row[0])) == alpha_norm else "atlas-beta",
             "sessionCount": row[2], "lastSessionId": row[3]}
            for row in usages if row[1] == "codex"
        ],
        "codexIdsAndCountsMatch": codex_ok,
        "auxiliarySessionsExcludedByCountsAndLatestIds": auxiliary_excluded,
        "acceptanceTuiUsagePresentWithoutSessionIds": acceptance_present,
        "indexSha256Before": before_hash,
        "indexSha256After": after_hash,
        "indexUnchangedDuringRead": before_hash == after_hash,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence_root", type=Path, help="native acceptance artifact directory")
    parser.add_argument("--payload", required=True, type=Path, help="extracted payload directory with payload-manifest.json")
    parser.add_argument("--after", required=True, type=Path, help="post-install baseline JSON inside evidence root")
    parser.add_argument("--overwrite-output", action="store_true", help="replace this verifier's prior JSON report")
    args = parser.parse_args()

    evidence_root = args.evidence_root.resolve(strict=True)
    if evidence_root != ARTIFACTS and not is_within(evidence_root, ARTIFACTS):
        raise VerificationError("evidence root must be within artifacts/native-0.1.1-20261015")
    payload_argument = args.payload if args.payload.is_absolute() else evidence_root / args.payload
    payload = safe_evidence_file(payload_argument / "payload-manifest.json", evidence_root, "payload manifest").parent
    if any(part.casefold() == "webview" for part in payload.parts):
        raise VerificationError("payload directory cannot be inside the WebView runtime profile")
    after = safe_evidence_file(args.after, evidence_root, "after snapshot")
    before = safe_evidence_file(evidence_root / "before.json", evidence_root, "before snapshot")
    fixture = safe_evidence_file(evidence_root / "fixture" / "native-review-fixture.json", evidence_root, "fixture manifest").parent
    fixture_manifest_path = fixture / "native-review-fixture.json"
    output = evidence_root / OUTPUT_NAME
    if output.exists():
        if not args.overwrite_output:
            raise VerificationError(f"output already exists: {OUTPUT_NAME}; pass --overwrite-output to refresh it")
        existing = load_json(output)
        if existing.get("evidenceRoot") != evidence_root.name or existing.get("checks") is None:
            raise VerificationError("existing output is not a recognized verifier report")

    manifest_hash_before = sha256(fixture_manifest_path)
    fixture_manifest = load_json(fixture_manifest_path)
    package = verify_payload(payload, after)
    bundle = bundle_artifacts()
    fixtures = verify_fixture_hashes(fixture, fixture_manifest_path, fixture_manifest)
    git_results = verify_git(fixture, fixture_manifest)
    synthetic_meta = codex_metadata(fixture, fixture_manifest)
    db = verify_sqlite(fixture, fixture_manifest)
    snapshot_change = baseline_delta(before, after)
    manifest_hash_after = sha256(fixture_manifest_path)

    checks = {
        "finalMsiAndNsisFound": bundle.get("msi", {}).get("passed", False) and bundle.get("nsis", {}).get("passed", False),
        "msiMatchesSelectedPayload": package.get("msiMatchesPayloadManifest", False),
        "allUnpackedPayloadFilesMatch": all(item.get("passed") for item in package.get("unpackedPayloadFiles", [])),
        "installedVersionMatchesPayload": package.get("installedVersionMatchesPayload", False),
        "installedFilesMatchPayload": bool(package.get("installedFiles")) and all(item.get("passed") for item in package.get("installedFiles", [])),
        "projectInputHashesMatchInitialManifest": bool(fixtures.get("projectInputFiles")) and all(item.get("passed") for item in fixtures.get("projectInputFiles", [])),
        "otherImmutableFixtureInputsMatch": all(item.get("passed") for item in fixtures.get("immutableSourceFiles", [])),
        "metadataIdsRemovedAsExpected": all(item.get("passed") for item in fixtures.get("metadataIdRemoval", [])),
        "fixtureManifestUnchanged": manifest_hash_before == manifest_hash_after and fixtures.get("manifestUnchangedDuringCheck", False),
        "gitStateChecksPass": all(item.get("passed") for item in git_results.get("projects", [])),
        "syntheticSessionMetadataMatchesManifest": synthetic_meta.get("primaryIdsMatchManifest") and synthetic_meta.get("auxiliaryIdsMatchManifest"),
        "sqliteCountsAndUsagesPass": db.get("projectCount") == 2 and db.get("projectPathsMatchManifest") and db.get("codexIdsAndCountsMatch") and db.get("auxiliarySessionsExcludedByCountsAndLatestIds") and db.get("acceptanceTuiUsagePresentWithoutSessionIds") and db.get("indexUnchangedDuringRead"),
    }
    report = {
        "verifier": "Verify-NativeReviewEvidence.py",
        "schemaVersion": 1,
        "evidenceRoot": evidence_root.name,
        "afterSnapshot": after.name,
        "payloadDirectory": payload.name,
        "bundleArtifacts": bundle,
        "payloadAndInstallation": package,
        "baselineChanges": snapshot_change,
        "fixtureHashes": fixtures,
        "fixtureGit": git_results,
        "syntheticCodexMetadata": synthetic_meta,
        "fixtureSqlite": db,
        "checks": checks,
        "passed": all(checks.values()),
        "notes": [
            "No WebView2 runtime profile files were read.",
            "The fixture manifest was read but not modified.",
            "SQLite was opened with mode=ro; Git ran with ambient GIT_* variables removed and optional locks disabled.",
        ],
    }
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": output.name, "passed": report["passed"], "checks": checks}, ensure_ascii=False, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Verification stopped: {error}", file=sys.stderr)
        raise SystemExit(2)
