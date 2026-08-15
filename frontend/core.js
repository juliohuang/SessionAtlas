/**
 * Pure frontend domain functions.
 *
 * Keep this module free of DOM, Tauri and global-state access so it can be
 * exercised with Node's built-in test runner as well as used by app.js.
 */

export function terminalSessionKey(projectId, toolKey) {
  return JSON.stringify([
    String(projectId ?? ""),
    String(toolKey ?? "shell"),
  ]);
}

export function activateTerminalHttpLink(event, uri, open) {
  if (!event?.ctrlKey || !/^https?:\/\//i.test(String(uri ?? ""))) return false;
  open(uri);
  return true;
}

export function buildPtyAttachRequest(id, toolKey, sessionId) {
  const key = String(toolKey ?? "shell");
  const isShell = key === "shell";
  return {
    id,
    toolKey: isShell ? null : key,
    sessionId: isShell ? null : (sessionId || null),
  };
}

export function findOpenTerminalTab(tabs, projectId, toolKey) {
  const projectKey = String(projectId ?? "");
  const requestedTool = String(toolKey ?? "shell");
  return (tabs || []).find(tab =>
    tab.kind === "pty"
    && !tab.dead
    && String(tab.project?.id ?? "") === projectKey
    && String(tab.usage?.toolKey ?? "shell") === requestedTool
  );
}

export function coalescePending(pendingByKey, key, create) {
  const existing = pendingByKey.get(key);
  if (existing) return existing;

  const operation = Promise.resolve().then(create);
  const tracked = operation.finally(() => {
    if (pendingByKey.get(key) === tracked) pendingByKey.delete(key);
  });
  pendingByKey.set(key, tracked);
  return tracked;
}

export function createLatestRequestGate() {
  let current = 0;
  return {
    begin() {
      current += 1;
      return current;
    },
    invalidate() {
      current += 1;
    },
    isCurrent(requestId) {
      return requestId === current;
    },
  };
}

export function createReloadCoordinator() {
  const full = createLatestRequestGate();
  const search = createLatestRequestGate();
  const auto = createLatestRequestGate();
  let fullInFlight = 0;
  return {
    beginFull() {
      search.invalidate();
      auto.invalidate();
      fullInFlight += 1;
      return { kind: "full", id: full.begin() };
    },
    beginSearch() {
      full.invalidate();
      auto.invalidate();
      return { kind: "search", id: search.begin() };
    },
    beginAuto() {
      if (fullInFlight > 0) return null;
      return { kind: "auto", id: auto.begin() };
    },
    end(token) {
      if (token?.kind === "full") fullInFlight = Math.max(0, fullInFlight - 1);
    },
    isCurrent(token) {
      if (!token) return false;
      if (token.kind === "full") return full.isCurrent(token.id);
      if (token.kind === "search") return search.isCurrent(token.id);
      return token.kind === "auto" && auto.isCurrent(token.id);
    },
    invalidateSearch() {
      search.invalidate();
    },
    invalidateFull() {
      full.invalidate();
    },
    invalidateAuto() {
      auto.invalidate();
    },
    isFullInFlight() {
      return fullInFlight > 0;
    },
  };
}

export function createMutationQueue() {
  const tails = new Map();
  return {
    run(key, operation) {
      const previous = tails.get(key) || Promise.resolve();
      const result = previous.then(operation);
      const tail = result.then(
        () => undefined,
        () => undefined,
      );
      tails.set(key, tail);
      tail.finally(() => {
        if (tails.get(key) === tail) tails.delete(key);
      });
      return result;
    },
    isPending(key) {
      return tails.has(key);
    },
  };
}

export async function captureResult(promise) {
  try {
    return { ok: true, value: await promise, error: null };
  } catch (error) {
    return { ok: false, value: null, error };
  }
}

export function projectCatalogFingerprint(projects) {
  return JSON.stringify((projects || []).map(project => ({
    id: project.id,
    source: project.source,
    path: project.path,
    name: project.name,
    lastAccessedAt: project.lastAccessedAt,
    gitBranch: project.gitBranch,
    remoteServerId: project.remoteServerId,
    toolUsages: (project.toolUsages || []).map(usage => ({
      toolKey: usage.toolKey,
      lastUsedAt: usage.lastUsedAt,
      sessionCount: usage.sessionCount,
      lastSessionId: usage.lastSessionId,
    })),
  })));
}

export function mergeProjectSources(localProjects, remoteProjects) {
  const local = (localProjects || []).map(project => ({
    ...project,
    source: "local",
  }));
  const remote = (remoteProjects || []).map(project => ({
    ...project,
    source: "remote",
  }));
  return [...local, ...remote];
}

export function buildProjectPublication(previousCatalog, query, projects, remoteProjects) {
  if (String(query ?? "").trim() !== "") {
    const searchResults = (projects || []).map(project => ({
      ...project,
      source: project.source || "local",
    }));
    return {
      catalog: previousCatalog || [],
      searchResults,
      visibleProjects: searchResults,
    };
  }
  const catalog = mergeProjectSources(projects, remoteProjects);
  return { catalog, searchResults: null, visibleProjects: catalog };
}

export function projectMatchesFilters(project, toolKey, cutoffMs, nowMs) {
  if (toolKey !== "all"
      && !(project.toolUsages || []).some(usage => usage.toolKey === toolKey)) {
    return false;
  }

  if (cutoffMs != null) {
    const timestamp = new Date(project.lastAccessedAt).getTime();
    if (Number.isNaN(timestamp) || nowMs - timestamp > cutoffMs) return false;
  }

  return true;
}

export function projectGroupKey(project, assignments) {
  const groupId = assignments[project.id] ?? null;
  return groupId == null ? "ungrouped" : String(groupId);
}

export function canSubmitCompleteGroupOrder(query, loadedCount, listLimit, memberCount) {
  if (String(query ?? "").trim() !== "") return false;
  if (Number.isFinite(listLimit) && loadedCount >= listLimit) return false;
  return !Number.isFinite(memberCount) || memberCount <= loadedCount;
}

export function sortProjects(projects, sortOrders) {
  const manual = projects.some(project => sortOrders[project.id] != null);
  if (manual) {
    projects.sort((left, right) => {
      const leftOrder = sortOrders[left.id] ?? Infinity;
      const rightOrder = sortOrders[right.id] ?? Infinity;
      if (leftOrder !== rightOrder) return leftOrder - rightOrder;
      return compareRecentFirst(left, right);
    });
  } else {
    projects.sort(compareRecentFirst);
  }
  return projects;
}

function compareRecentFirst(left, right) {
  return right.lastAccessedAt > left.lastAccessedAt
    ? 1
    : right.lastAccessedAt < left.lastAccessedAt
      ? -1
      : 0;
}
