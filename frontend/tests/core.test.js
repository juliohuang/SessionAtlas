import assert from "node:assert/strict";
import test from "node:test";

import {
  activateTerminalHttpLink,
  buildPtyAttachRequest,
  buildPtyRemoteSwitchRequest,
  buildPtySpawnRequest,
  buildProjectPublication,
  canSubmitCompleteGroupOrder,
  captureResult,
  coalescePending,
  createLatestRequestGate,
  createMutationQueue,
  createReloadCoordinator,
  findOpenTerminalTab,
  findReusableRemoteTerminalTab,
  mergeProjectSources,
  projectGroupKey,
  projectCatalogFingerprint,
  projectMatchesFilters,
  sortProjects,
  terminalSessionKey,
} from "../core.js";

test("PTY attach sends structured tool metadata instead of a shell command", () => {
  assert.deepEqual(
    buildPtyAttachRequest(7, "codex", "session-123"),
    { id: 7, toolKey: "codex", sessionId: "session-123" },
  );
  assert.deepEqual(
    buildPtyAttachRequest(8, "shell", "ignored"),
    { id: 8, toolKey: null, sessionId: null },
  );
  assert.deepEqual(
    buildPtyAttachRequest(9, "codex", "remote-session", true),
    { id: 9, toolKey: null, sessionId: null },
  );
});

test("remote PTY spawn carries tool metadata for one-time tmux startup", () => {
  const project = {
    source: "remote",
    path: "/srv/project",
  };
  const usage = {
    toolKey: "codex",
    lastSessionId: "session-123",
  };
  const server = {
    id: 17,
    user: "developer",
    host: "example.test",
    port: 2222,
    identityFile: "/keys/id_ed25519",
  };

  assert.deepEqual(buildPtySpawnRequest(project, usage, server, 120, 40), {
    path: "/srv/project",
    cols: 120,
    rows: 40,
    source: "remote",
    remote: {
      serverId: 17,
      user: "developer",
      host: "example.test",
      port: 2222,
      identityFile: "/keys/id_ed25519",
      toolKey: "codex",
      sessionId: "session-123",
    },
  });
});

test("remote PTY switch identifies the existing server connection and target", () => {
  assert.deepEqual(
    buildPtyRemoteSwitchRequest(
      44,
      { source: "remote", remoteServerId: 17, path: "/srv/other-project" },
      { toolKey: "claude", lastSessionId: "session-456" },
    ),
    {
      id: 44,
      path: "/srv/other-project",
      serverId: 17,
      toolKey: "claude",
      sessionId: "session-456",
    },
  );
});

test("local PTY spawn leaves tool startup to attach", () => {
  assert.deepEqual(
    buildPtySpawnRequest(
      { source: "local", path: "C:\\workspace\\project" },
      { toolKey: "claude", lastSessionId: "session-123" },
      null,
      80,
      24,
    ),
    {
      path: "C:\\workspace\\project",
      cols: 80,
      rows: 24,
      source: "local",
      remote: null,
    },
  );
});

test("terminal links require Ctrl and an HTTP(S) URL", () => {
  const opened = [];
  assert.equal(activateTerminalHttpLink({}, "https://example.com", uri => opened.push(uri)), false);
  assert.equal(activateTerminalHttpLink({ ctrlKey: true }, "javascript:alert(1)", uri => opened.push(uri)), false);
  assert.equal(activateTerminalHttpLink({ ctrlKey: true }, "file:///tmp/a", uri => opened.push(uri)), false);
  assert.equal(activateTerminalHttpLink({ ctrlKey: true }, "https://example.com/a", uri => opened.push(uri)), true);
  assert.deepEqual(opened, ["https://example.com/a"]);
});

test("terminal session keys are stable across numeric and string project ids", () => {
  assert.equal(terminalSessionKey(42, "codex"), terminalSessionKey("42", "codex"));
  assert.notEqual(terminalSessionKey("42", "codex"), terminalSessionKey("42", "shell"));
});

test("terminal dedup finds only a live matching project and tool tab", () => {
  const tabs = [
    { kind: "pty", dead: true, project: { id: "p1" }, usage: { toolKey: "codex" } },
    { kind: "pty", dead: false, project: { id: "p1" }, usage: { toolKey: "claude" } },
    { kind: "file", project: { id: "p1" } },
    { kind: "pty", dead: false, project: { id: "p1" }, usage: { toolKey: "codex" }, tabId: 7 },
  ];

  assert.equal(findOpenTerminalTab(tabs, "p1", "codex")?.tabId, 7);
  assert.equal(findOpenTerminalTab(tabs, "p2", "codex"), undefined);
});

test("remote terminal reuse selects the newest live tab on the same server", () => {
  const tabs = [
    { tabId: 1, kind: "pty", dead: false, project: { source: "remote", remoteServerId: 7 } },
    { tabId: 2, kind: "pty", dead: false, project: { source: "remote", remoteServerId: 8 } },
    { tabId: 3, kind: "pty", dead: true, project: { source: "remote", remoteServerId: 7 } },
    { tabId: 4, kind: "pty", dead: false, project: { source: "remote", remoteServerId: 7 } },
    { tabId: 5, kind: "file", project: { source: "remote", remoteServerId: 7 } },
  ];

  assert.equal(findReusableRemoteTerminalTab(tabs, 7)?.tabId, 4);
  assert.equal(findReusableRemoteTerminalTab(tabs, 9), undefined);
});

test("concurrent terminal opens share one in-flight operation and clear it", async () => {
  const pending = new Map();
  let calls = 0;
  let release;
  const create = () => {
    calls += 1;
    return new Promise(resolve => { release = resolve; });
  };

  const first = coalescePending(pending, "p1/codex", create);
  const second = coalescePending(pending, "p1/codex", create);
  assert.equal(first, second);
  await Promise.resolve();
  assert.equal(calls, 1);

  release("ready");
  assert.equal(await first, "ready");
  assert.equal(pending.has("p1/codex"), false);
});

test("only the latest reload request is allowed to publish state", () => {
  const gate = createLatestRequestGate();
  const first = gate.begin();
  const second = gate.begin();

  assert.equal(gate.isCurrent(first), false);
  assert.equal(gate.isCurrent(second), true);

  gate.invalidate();
  assert.equal(gate.isCurrent(second), false);
});

test("full, search and auto requests cannot invalidate higher-information owners", () => {
  const coordinator = createReloadCoordinator();
  const auto = coordinator.beginAuto();
  const full = coordinator.beginFull();
  assert.equal(coordinator.isCurrent(auto), false);
  assert.equal(coordinator.beginAuto(), null);
  assert.equal(coordinator.isCurrent(full), true);

  const search = coordinator.beginSearch();
  assert.equal(coordinator.isCurrent(full), false);
  assert.equal(coordinator.isCurrent(search), true);
  coordinator.end(full);
  assert.equal(coordinator.isFullInFlight(), false);

  const nextAuto = coordinator.beginAuto();
  assert.equal(coordinator.isCurrent(nextAuto), true);
  const nextFull = coordinator.beginFull();
  assert.equal(coordinator.isCurrent(nextAuto), false);
  assert.equal(coordinator.isCurrent(nextFull), true);
  coordinator.end(nextFull);
});

test("entity mutation queue serializes operations and continues after rejection", async () => {
  const queue = createMutationQueue();
  const events = [];
  let releaseFirst;
  const first = queue.run("groups", async () => {
    events.push("first:start");
    await new Promise(resolve => { releaseFirst = resolve; });
    events.push("first:end");
    throw new Error("expected failure");
  });
  const second = queue.run("groups", async () => {
    events.push("second:start");
    return "done";
  });
  await Promise.resolve();
  assert.deepEqual(events, ["first:start"]);
  assert.equal(queue.isPending("groups"), true);
  releaseFirst();
  await assert.rejects(first, /expected failure/);
  assert.equal(await second, "done");
  await Promise.resolve();
  assert.deepEqual(events, ["first:start", "first:end", "second:start"]);
  assert.equal(queue.isPending("groups"), false);
});

test("captured failures stay distinct from successful empty values", async () => {
  assert.deepEqual(await captureResult(Promise.resolve([])), {
    ok: true,
    value: [],
    error: null,
  });
  const failure = new Error("offline");
  const captured = await captureResult(Promise.reject(failure));
  assert.equal(captured.ok, false);
  assert.equal(captured.value, null);
  assert.equal(captured.error, failure);
});

test("local refresh keeps remote projects and assigns canonical sources", () => {
  const merged = mergeProjectSources(
    [{ id: "local", source: "stale" }],
    [{ id: "remote" }],
  );

  assert.deepEqual(
    merged.map(project => [project.id, project.source]),
    [["local", "local"], ["remote", "remote"]],
  );
});

test("search publication never replaces the complete project catalog", () => {
  const catalog = [{ id: "catalog", source: "local" }];
  const searched = buildProjectPublication(
    catalog,
    "needle",
    [{ id: "match" }],
    [{ id: "ignored-remote-list" }],
  );
  assert.equal(searched.catalog, catalog);
  assert.deepEqual(searched.searchResults, [{ id: "match", source: "local" }]);
  assert.equal(searched.visibleProjects, searched.searchResults);

  const refreshed = buildProjectPublication(
    catalog,
    "",
    [{ id: "local" }],
    [{ id: "remote" }],
  );
  assert.equal(refreshed.searchResults, null);
  assert.deepEqual(
    refreshed.catalog.map(project => [project.id, project.source]),
    [["local", "local"], ["remote", "remote"]],
  );
  assert.equal(refreshed.visibleProjects, refreshed.catalog);
});

test("project filters combine tool and recency semantics", () => {
  const now = Date.parse("2026-07-30T12:00:00Z");
  const project = {
    lastAccessedAt: "2026-07-30T11:00:00Z",
    toolUsages: [{ toolKey: "codex" }],
  };

  assert.equal(projectMatchesFilters(project, "all", null, now), true);
  assert.equal(projectMatchesFilters(project, "codex", 2 * 60 * 60 * 1000, now), true);
  assert.equal(projectMatchesFilters(project, "claude", null, now), false);
  assert.equal(projectMatchesFilters(project, "codex", 30 * 60 * 1000, now), false);
});

test("catalog fingerprint detects branch and usage changes at equal count and timestamp", () => {
  const base = [{
    id: "p1",
    source: "local",
    path: "/repo",
    name: "repo",
    lastAccessedAt: "2026-08-03T00:00:00Z",
    gitBranch: "main",
    toolUsages: [{ toolKey: "codex", lastUsedAt: "2026-08-03T00:00:00Z", sessionCount: 1 }],
  }];
  const changed = structuredClone(base);
  changed[0].gitBranch = "feature";
  assert.notEqual(projectCatalogFingerprint(base), projectCatalogFingerprint(changed));
  changed[0].gitBranch = "main";
  changed[0].toolUsages[0].sessionCount = 2;
  assert.notEqual(projectCatalogFingerprint(base), projectCatalogFingerprint(changed));
});

test("group key uses the assignment or the ungrouped sentinel", () => {
  assert.equal(projectGroupKey({ id: "p1" }, { p1: 7 }), "7");
  assert.equal(projectGroupKey({ id: "p2" }, { p1: 7 }), "ungrouped");
});

test("group reorder requires an unfiltered, complete ledger", () => {
  assert.equal(canSubmitCompleteGroupOrder("needle", 10, 2000, 10), false);
  assert.equal(canSubmitCompleteGroupOrder("", 2000, 2000, 10), false);
  assert.equal(canSubmitCompleteGroupOrder("", 10, 2000, 11), false);
  assert.equal(canSubmitCompleteGroupOrder("", 10, 2000, 10), true);
  assert.equal(canSubmitCompleteGroupOrder("", 10, 2000, undefined), true);
});

test("manual project order wins, with recent projects used as the tie breaker", () => {
  const projects = [
    { id: "new", lastAccessedAt: "2026-07-30T12:00:00Z" },
    { id: "first", lastAccessedAt: "2026-07-29T12:00:00Z" },
    { id: "unranked", lastAccessedAt: "2026-07-31T12:00:00Z" },
  ];

  sortProjects(projects, { first: 0, new: 1 });
  assert.deepEqual(projects.map(project => project.id), ["first", "new", "unranked"]);
});
