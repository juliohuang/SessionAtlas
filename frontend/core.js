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

function tokenizeSshInput(input) {
  const tokens = [];
  let current = "";
  let quote = null;
  let started = false;

  for (const character of String(input ?? "")) {
    if (quote !== null) {
      if (character === quote) {
        quote = null;
      } else {
        current += character;
      }
      started = true;
      continue;
    }

    if (character === '"' || character === "'") {
      quote = character;
      started = true;
    } else if (/\s/.test(character)) {
      if (started) {
        tokens.push(current);
        current = "";
        started = false;
      }
    } else {
      current += character;
      started = true;
    }
  }

  if (quote !== null) return { ok: false, error: "unterminatedQuote" };
  if (started) tokens.push(current);
  return { ok: true, tokens };
}

/**
 * Parse the small, intentionally-bounded SSH syntax accepted by the remote
 * server form. The returned fields are still validated by the Rust command;
 * this parser never executes the supplied text as a shell command.
 */
export function parseSshConnectionInput(input) {
  const tokenized = tokenizeSshInput(input);
  if (!tokenized.ok) return tokenized;

  const tokens = [...tokenized.tokens];
  if (/^ssh(?:\.exe)?$/i.test(tokens[0] || "")) tokens.shift();
  if (!tokens.length) return { ok: false, error: "empty" };

  let destination = null;
  let port = null;
  let identityFile = null;
  let optionsEnded = false;

  const setOption = (kind, value) => {
    if (!value) return { ok: false, error: "missingOptionValue", detail: kind };
    if (kind === "-p") {
      if (port !== null) return { ok: false, error: "duplicateOption", detail: kind };
      if (!/^\d+$/.test(value)) return { ok: false, error: "port" };
      const parsed = Number(value);
      if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
        return { ok: false, error: "port" };
      }
      port = parsed;
    } else {
      if (identityFile !== null) return { ok: false, error: "duplicateOption", detail: kind };
      identityFile = value;
    }
    return null;
  };

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (!optionsEnded && token === "--") {
      optionsEnded = true;
      continue;
    }

    if (!optionsEnded && (token === "-p" || token === "-i")) {
      const error = setOption(token, tokens[index + 1]);
      if (error) return error;
      index += 1;
      continue;
    }

    if (!optionsEnded && (/^-p.+/.test(token) || /^-i.+/.test(token))) {
      const kind = token.slice(0, 2);
      const error = setOption(kind, token.slice(2));
      if (error) return error;
      continue;
    }

    if (!optionsEnded && token.startsWith("-")) {
      return { ok: false, error: "unsupportedOption", detail: token };
    }

    if (destination !== null) {
      return { ok: false, error: "extraArgument", detail: token };
    }
    destination = token;
  }

  if (!destination) return { ok: false, error: "destination" };
  const separator = destination.lastIndexOf("@");
  if (separator <= 0 || separator === destination.length - 1) {
    return { ok: false, error: "destination" };
  }

  const user = destination.slice(0, separator);
  let host = destination.slice(separator + 1);
  let inlinePort = null;
  let inlinePortText = null;

  // Accept the familiar `user@host:port` shorthand as well as OpenSSH's
  // `-p PORT`. Bracketed IPv6 keeps its brackets so the Rust validator and
  // SSH destination builder continue to distinguish address colons from the
  // optional port separator.
  if (host.startsWith("[")) {
    const bracketEnd = host.indexOf("]");
    if (bracketEnd < 0) return { ok: false, error: "destination" };
    const suffix = host.slice(bracketEnd + 1);
    if (suffix) {
      if (!suffix.startsWith(":")) return { ok: false, error: "destination" };
      inlinePortText = suffix.slice(1);
      host = host.slice(0, bracketEnd + 1);
    }
  } else {
    const firstColon = host.indexOf(":");
    const lastColon = host.lastIndexOf(":");
    if (firstColon === lastColon && firstColon >= 0) {
      if (firstColon === 0) return { ok: false, error: "destination" };
      inlinePortText = host.slice(firstColon + 1);
      host = host.slice(0, firstColon);
    }
  }

  if (inlinePortText !== null) {
    if (!/^\d+$/.test(inlinePortText)) return { ok: false, error: "port" };
    inlinePort = Number(inlinePortText);
    if (!Number.isInteger(inlinePort) || inlinePort < 1 || inlinePort > 65535) {
      return { ok: false, error: "port" };
    }
    if (port !== null) return { ok: false, error: "duplicateOption", detail: "-p" };
    port = inlinePort;
  }

  return {
    ok: true,
    value: {
      user,
      host,
      port,
      identityFile,
    },
  };
}

function buildRemotePtyOptions(usage, server) {
  if (!server) return null;
  const toolKey = String(usage?.toolKey || "shell");
  return {
    serverId: server.id,
    user: server.user,
    host: server.host,
    port: server.port,
    identityFile: server.identityFile || null,
    toolKey,
    sessionId: toolKey !== "shell" ? (usage?.lastSessionId || null) : null,
  };
}

export function buildPtySpawnRequest(project, usage, server, cols, rows) {
  const isRemote = project?.source === "remote";
  return {
    path: project?.path,
    cols,
    rows,
    source: isRemote ? "remote" : "local",
    remote: isRemote ? buildRemotePtyOptions(usage, server) : null,
  };
}

export function buildPtyRemoteSwitchRequest(id, project, usage) {
  return {
    id,
    path: project?.path,
    serverId: project?.remoteServerId,
    toolKey: String(usage?.toolKey || "shell"),
    sessionId: usage?.toolKey && usage.toolKey !== "shell"
      ? (usage.lastSessionId || null)
      : null,
  };
}

export function buildPtyAttachRequest(id, toolKey, sessionId, remote = false) {
  const key = String(toolKey ?? "shell");
  const isShell = remote || key === "shell";
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

export function findReusableRemoteTerminalTab(tabs, remoteServerId) {
  const serverKey = String(remoteServerId ?? "");
  return [...(tabs || [])].reverse().find(tab =>
    tab.kind === "pty"
    && !tab.dead
    && !tab.isQueue
    && tab.project?.source === "remote"
    && String(tab.project?.remoteServerId ?? "") === serverKey
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
    pathMissing: project.pathMissing === true,
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

// Order modes used by the project ledger. All modes keep the complete
// matching catalog visible; they only change which projects appear first.
// "priority" favors work that needs attention now: live sessions, paths
// that still exist, recent activity tiers, and then repeat usage.
export function sortProjectsForView(projects, mode, sortOrders, activeProjectIds = new Set()) {
  if (mode === "grouped") return sortProjects(projects, sortOrders);
  if (mode === "name") {
    return projects.sort((left, right) => {
      const byName = String(left.name || "").localeCompare(String(right.name || ""), undefined, {
        sensitivity: "base",
        numeric: true,
      });
      return byName || compareRecentFirst(left, right);
    });
  }
  if (mode === "recent") return projects.sort(compareRecentFirst);

  return projects.sort((left, right) => {
    const active = Number(activeProjectIds.has(right.id)) - Number(activeProjectIds.has(left.id));
    if (active) return active;

    const available = Number(Boolean(left.pathMissing)) - Number(Boolean(right.pathMissing));
    if (available) return available;

    const leftTimestamp = projectTimestamp(left);
    const rightTimestamp = projectTimestamp(right);
    const activityTier = recencyTier(rightTimestamp) - recencyTier(leftTimestamp);
    if (activityTier) return activityTier;

    const sessions = projectSessionCount(right) - projectSessionCount(left);
    if (sessions) return sessions;

    const recent = rightTimestamp - leftTimestamp;
    if (recent) return recent;
    return String(left.name || "").localeCompare(String(right.name || ""), undefined, {
      sensitivity: "base",
      numeric: true,
    });
  });
}

function projectTimestamp(project) {
  const value = Date.parse(project.lastAccessedAt || "");
  return Number.isFinite(value) ? value : 0;
}

function recencyTier(timestamp, now = Date.now()) {
  const age = Math.max(0, now - timestamp);
  if (age <= 24 * 60 * 60 * 1000) return 3;
  if (age <= 7 * 24 * 60 * 60 * 1000) return 2;
  if (age <= 30 * 24 * 60 * 60 * 1000) return 1;
  return 0;
}

function projectSessionCount(project) {
  return (project.toolUsages || []).reduce(
    (total, usage) => total + (Number(usage.sessionCount) || 0),
    0,
  );
}

function compareRecentFirst(left, right) {
  return right.lastAccessedAt > left.lastAccessedAt
    ? 1
    : right.lastAccessedAt < left.lastAccessedAt
      ? -1
      : 0;
}
