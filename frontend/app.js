/* ============================================================
   SessionAtlas — AI CLI project workspace · frontend logic
   Layout: projects (left) + project overview (center) + terminals (right).
   Dual-mode: Tauri commands when present, bundled sample data
   in a plain browser (terminal panel shows a demo notice then).
   ============================================================ */

import { t as tr, currentLang, currentLocaleTag } from "./i18n.js";
import { iconSvg, TOOL_ICONS, FILETYPE_ICONS } from "./icons.js";
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
  parseSshConnectionInput,
  projectGroupKey,
  projectCatalogFingerprint,
  projectMatchesFilters,
  sortProjects,
  terminalSessionKey,
} from "./core.js";

const { invoke } = window.__TAURI__?.core ?? {};
const { listen } = window.__TAURI__?.event ?? {};
const HAS_TAURI = typeof invoke === "function";
// xterm UMD: `window.Terminal` is the class; `window.FitAddon` is an object
// whose `.FitAddon` member is the addon class.
const HAS_TERM = typeof window.Terminal === "function"
  && typeof window.FitAddon?.FitAddon === "function";

/* ── sample dataset (browser-only fallback) ─────────────── */
const SAMPLE = [
  { id:"1", path:"C:\\Demo\\atlas-notes", name:"atlas-notes", lastAccessedAt: isoMin(35), gitBranch:"main",
    toolUsages:[{toolKey:"claude",toolName:"Claude Code",lastUsedAt:isoMin(35),sessionCount:12,lastSessionId:"a1b2c3d4"}] },
  { id:"2", path:"C:\\Demo\\terminal-lab", name:"terminal-lab", lastAccessedAt: isoHr(2), gitBranch:"feature/demo",
    toolUsages:[{toolKey:"claude",toolName:"Claude Code",lastUsedAt:isoHr(2),sessionCount:8,lastSessionId:"9e8f7a6b"},{toolKey:"codex",toolName:"Codex CLI",lastUsedAt:isoHr(20),sessionCount:3,lastSessionId:"c0d3cafe"}] },
  { id:"3", path:"C:\\Demo\\api-workbench", name:"api-workbench", lastAccessedAt: isoHr(4), gitBranch:"main",
    toolUsages:[{toolKey:"kimi",toolName:"Kimi CLI",lastUsedAt:isoHr(4),sessionCount:5}] },
  { id:"4", path:"C:\\Demo\\docs-garden", name:"docs-garden", lastAccessedAt: isoHr(6), gitBranch:"main",
    toolUsages:[{toolKey:"claude",toolName:"Claude Code",lastUsedAt:isoHr(6),sessionCount:21}] },
  { id:"5", path:"C:\\Demo\\pixel-board", name:"pixel-board", lastAccessedAt: isoDay(1), gitBranch:"feature/canvas",
    toolUsages:[{toolKey:"aider",toolName:"Aider",lastUsedAt:isoDay(1),sessionCount:2}] },
  { id:"6", path:"C:\\Demo\\cli-playground", name:"cli-playground", lastAccessedAt: isoDay(2), gitBranch:null,
    toolUsages:[{toolKey:"opencode",toolName:"OpenCode",lastUsedAt:isoDay(2),sessionCount:4}] },
  { id:"7", path:"C:\\Demo\\migration-sandbox", name:"migration-sandbox", lastAccessedAt: isoDay(3), gitBranch:"main",
    toolUsages:[{toolKey:"codex",toolName:"Codex CLI",lastUsedAt:isoDay(3),sessionCount:9},{toolKey:"kimi",toolName:"Kimi CLI",lastUsedAt:isoDay(3),sessionCount:7}] },
];
function isoMin(n){return new Date(Date.now()-n*60000).toISOString()}
function isoHr(n){return new Date(Date.now()-n*3600000).toISOString()}
function isoDay(n){return new Date(Date.now()-n*86400000).toISOString()}

const LIST_LIMIT = 2000;

// Dropdown to assign a project to a group (or "未分组"). Rendered in
// both the popover and the right-pane launch panel.
const OVERVIEW_COLLAPSED_KEY = "sessionatlas.overviewCollapsed";

function readOverviewCollapsed() {
  try { return localStorage.getItem(OVERVIEW_COLLAPSED_KEY) === "true"; }
  catch { return false; }
}

function groupPickerHtml(projectId) {
  const currentGid = state.assignments[projectId];
  const ungrouped = tr("group.ungrouped");
  const currentName = currentGid == null
    ? ungrouped
    : (state.groups.find(g => g.id === currentGid)?.name || ungrouped);
  const opts = [`<option value="">${escapeHtml(ungrouped)}</option>`]
    .concat(state.groups.map(g =>
      `<option value="${g.id}" ${g.id === currentGid ? "selected" : ""}>${escapeHtml(g.name)}</option>`))
    .join("");
  return `<div class="group-picker">
    <span class="entry__launch-label">${escapeHtml(tr("entry.label.group"))}</span>
    <select class="group-picker__select" data-group-picker data-project-id="${escapeHtml(projectId)}">
      ${opts}
    </select>
  </div>`;
}
const state = {
  catalog: [], searchResults: null,
  all: [], filtered: [], tools: [],
  tool: "all", recency: "all", query: "",
  selectedId: null, cursor: -1,
  autoTimer: null, searchTimer: null,
  tabs: [],            // {ptyId, title, term, fit, pane, project, usage, dead}
  openingPtys: new Map(), // project+tool → in-flight open/switch promise
  ptyEventsReady: !HAS_TAURI,
  activeTabId: null,   // ptyId of active tab
  openerPrefs: [],          // full list (built-in + custom) from list_opener_prefs
  openerPrefsLoaded: false, // true after first load (so renderLedger knows it's safe to use)
  openerPrefsError: null,   // non-null when the prefs DB is unreachable
  webDevelopmentTools: [], // browser-based development endpoints such as DSH
  webDevelopmentToolsLoaded: false,
  webDevelopmentToolsError: null,
  menuOpenId: null,         // project id whose `...` popover is open, or null
  groups: [],               // [{id, name, sortOrder, memberCount}] from list_groups
  groupRevision: 0,         // optimistic-concurrency revision from prefs.db
  assignments: {},          // {projectId: groupId} from list_group_assignments
  sortOrders: {},           // {projectId: sortOrder} from list_sort_orders (manual order)
  viewMode: "ledger",       // "ledger" | "files" — which view occupies the left pane
  expandedId: null,         // project id whose inline session panel is open, or null
  remoteServers: [],         // [{id, label, user, host, port, identityFile, scanRoots, lastScannedAt, osFamily}] from list_remote_servers
  remoteServerById: {},     // {id: server} for tooltip lookup
  remoteProjects: [],       // last successful remote project list used by catalog
  projectIgnores: [],       // user-managed local/remote directory-tree exclusions
  sessionCleanup: null,     // latest Codex/Claude cleanup analysis
  sessionCleanupTrash: [],  // recoverable cleanup batches
  sessionCleanupSelected: new Set(),
  sessionCleanupLoading: false,
  sessionCleanupStale: false,
  gitStatusByProject: {},   // local/remote-ref GitInfo cache by project id
  gitStatusAtByProject: {}, // monotonic cache timestamps for GitInfo TTL
  remoteScanIds: new Set(), // server ids currently scanning on background workers
  tuiMachines: {},          // {local|remote:<id>: live installed/enabled capabilities}
  tuiCapabilityAt: {},      // monotonic timestamps for the capability TTL
  tuiLoadingKeys: new Set(),
  tuiCheckingKeys: new Set(),
  tuiInstallingKeys: new Set(),
  tuiUpgradingKeys: new Set(),
  tuiAdapterBusyKeys: new Set(),
  tuiAdapterImporting: false,
  tuiAdapterManifestPath: "",
  staleSources: { remote: false, tools: false, servers: false, ignores: false, groups: false, openers: false, webDevelopment: false },
  firstRunError: null,       // preserved across async preference/group renders until a retry succeeds
  settingsView: "menu",     // "menu" | "cleanup" | "tui" | "webDevelopment" | "openers" | "groups" | "remote" | "ignores" | "language"
  overviewCollapsed: readOverviewCollapsed(), // new users default to the full overview
  _dragId: null,            // project id being dragged (transient)
  ledgerRows: [],           // grouped rows used by the bounded ledger renderer
  ledgerHeightByKey: new Map(),
  ledgerLayout: null,
  ledgerRenderRevision: 0,  // invalidates a recycled window when row content changes
  ledgerVirtualWindowSignature: null,
};
const reloadCoordinator = createReloadCoordinator();
const groupMutationQueue = createMutationQueue();
const settingsMutationQueue = createMutationQueue();
const remoteTerminalQueue = createMutationQueue();
const entryDocsGate = createLatestRequestGate();
const entryTreeGate = createLatestRequestGate();
const docModalGate = createLatestRequestGate();
const leftTreeGate = createLatestRequestGate();

// Map of recency-filter key → max age in milliseconds. `null` means no filter.
const RECENCY_CUTOFFS = {
  "24h": 24 * 3600 * 1000,
  "7d":  7  * 86400 * 1000,
  "30d": 30 * 86400 * 1000,
  "all": null,
};

/* ── tool visuals ───────────────────────────────────────── */
const TOOL_DOT = { claude:"dot--claude", codex:"dot--codex", kimi:"dot--kimi", opencode:"dot--opencode", aider:"dot--aider", pi:"dot--pi" };
const TOOL_COLOR = { claude:"#d97757", codex:"#10a37f", kimi:"#6c8aff", opencode:"#e8b339", aider:"#c6f24e", pi:"#f472b6" };
// Two-letter monograms used in the .tool-icon tile. Short, all-caps, and
// distinct so each tool is recognisable in a 16×16 chip. Plain shell
// gets "SH" so the generic strip is also branded.
const TOOL_LABEL = { claude:"CL", codex:"CX", kimi:"KM", opencode:"OC", aider:"AI", pi:"PI", shell:"SH" };
// Small monogram tile used in the right-side commands strip title and
// the session tab pills. Tools with a collected brand logo (claude/codex/
// kimi) render as a single-color brand-glyph SVG tinted with the tool's
// accent colour; the rest keep the 2-letter monogram tile. The coloured
// .dot indicators elsewhere carry the same accent for consistency.
function toolIcon(key) {
  const color = TOOL_COLOR[key] || "var(--bone-mute)";
  const iconKey = TOOL_ICONS[key];
  if (iconKey) {
    return `<span class="tool-icon tool-icon--brand" style="color:${color}" title="${escapeHtml(key || "")}">${iconSvg(iconKey, { size: 13 })}</span>`;
  }
  const label = TOOL_LABEL[key] || (key ? key.slice(0, 2).toUpperCase() : "?");
  return `<span class="tool-icon" style="background:${color}" title="${escapeHtml(key || "")}">${label}</span>`;
}
function toolDotClass(key){ return TOOL_DOT[key] || ""; }
function toolColor(key){
  if (TOOL_COLOR[key]) return TOOL_COLOR[key];
  let h = 0;
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
  return `hsl(${h % 360} 65% 60%)`;
}

/* ── data access ────────────────────────────────────────── */
async function fetchProjects(query) {
  if (HAS_TAURI) {
    const q = (query || "").trim();
    if (q) return await invoke("search_projects", { query: q });
    return await invoke("list_projects", { limit: LIST_LIMIT });
  }
  const q = (query || "").trim().toLowerCase();
  if (!q) return SAMPLE;
  return SAMPLE.filter(p => p.name.toLowerCase().includes(q) || p.path.toLowerCase().includes(q));
}

// Pull remote projects + remote servers from the backend. Failures
// return empty arrays so a missing/broken remote doesn't break the
// ledger (the local projects still load).
async function fetchRemoteProjects() {
  if (!HAS_TAURI) return [];
  return await invoke("list_remote_projects");
}
async function searchRemoteProjects(query) {
  if (!HAS_TAURI) return [];
  return await invoke("search_remote_projects", { query });
}
async function fetchRemoteServers() {
  if (!HAS_TAURI) return [];
  return await invoke("list_remote_servers");
}
async function fetchProjectIgnores() {
  if (!HAS_TAURI) return [];
  return await invoke("list_project_ignores");
}
async function fetchTools() {
  if (HAS_TAURI) {
    return await invoke("list_tools");
  }
  const map = new Map();
  SAMPLE.forEach(p => (p.toolUsages || []).forEach(u => map.set(u.toolKey, u.toolName)));
  return [...map].map(([toolKey, toolName]) => ({ toolKey, toolName }));
}

/* ── time formatting ────────────────────────────────────── */
// Compact relative time, no "ago" suffix. Sub-day durations are computed
// down to the hour with a minute remainder (e.g. "3h 12m") so the value
// stays informative under 24h. `refreshTimeLabels()` re-runs this on a
// timer so labels never go stale while the app is open.
function relTime(iso) {
  const d = new Date(iso); const diff = Date.now() - d.getTime();
  const m = Math.floor(diff/60000), h = Math.floor(diff/3600000), day = Math.floor(diff/86400000);
  if (m < 1) return "now";
  if (h < 1) return `${m}m`;
  if (day < 1) return `${h}h ${m % 60}m`;
  if (day < 7) return `${day}d`;
  return `${Math.floor(day/7)}w`;
}

/* ── opener prefs (external openers) ────────────────────── */
async function loadOpenerPrefs() {
  if (!HAS_TAURI) {
    // Browser demo: a fixed subset so the second sub-row is visible.
    state.openerPrefs = [
      { id: "demo-vscode", type: "builtin", builtinKey: "vscode",
        label: "VSCode",   command: "code {path}",     enabled: true, sortOrder: 10 },
      { id: "demo-finder", type: "builtin", builtinKey: "finder",
        label: "Explorer", command: "explorer {path}", enabled: true, sortOrder: 20 },
    ];
    state.openerPrefsLoaded = true;
    state.openerPrefsError = null;
    renderLedger();
    renderSelectedLaunchPanel();
    return;
  }
  try {
    state.openerPrefs = await invoke("list_opener_prefs");
    state.openerPrefsError = null;
    state.staleSources.openers = false;
  } catch (e) {
    console.warn("list_opener_prefs failed", e);
    state.openerPrefsError = String(e);
    state.staleSources.openers = true;
  }
  state.openerPrefsLoaded = true;
  renderLedger();
  renderSelectedLaunchPanel();
}

/* ── browser-based development tools ───────────────────── */
async function loadWebDevelopmentTools() {
  if (!HAS_TAURI) {
    state.webDevelopmentTools = [];
    state.webDevelopmentToolsLoaded = true;
    state.webDevelopmentToolsError = null;
    if (!drawer.hidden && ["menu", "webDevelopment"].includes(state.settingsView)) renderDrawerBody();
    renderSelectedLaunchPanel();
    return;
  }
  try {
    const tools = await invoke("list_web_development_tools");
    state.webDevelopmentTools = Array.isArray(tools) ? tools : [];
    state.webDevelopmentToolsError = null;
    state.staleSources.webDevelopment = false;
  } catch (error) {
    console.warn("list_web_development_tools failed", error);
    state.webDevelopmentToolsError = String(error);
    state.staleSources.webDevelopment = true;
  }
  state.webDevelopmentToolsLoaded = true;
  if (!drawer.hidden && ["menu", "webDevelopment"].includes(state.settingsView)) renderDrawerBody();
  renderSelectedLaunchPanel();
}

/* ── groups ─────────────────────────────────────────────── */
async function loadGroups() {
  if (!HAS_TAURI) {
    // Browser demo: one example group so the UI is visible.
    state.groups = [
      { id: 1, name: "Active", sortOrder: 10, memberCount: 0 },
    ];
    state.assignments = {};
    state.sortOrders = {};
    applyFilters();
    renderSelectedLaunchPanel();
    return;
  }
  try {
    const [groups, assignments, sortRows, revision] = await Promise.all([
      invoke("list_groups"),
      invoke("list_group_assignments"),
      invoke("list_sort_orders"),
      invoke("get_group_revision"),
    ]);
    state.groups = groups;
    state.assignments = assignments;
    // Flatten sort rows into a {projectId: sortOrder} map. group_key is
    // redundant here — assignment is the source of truth for which bucket a
    // project renders in; this map only supplies the within-bucket order.
    const orders = {};
    for (const r of sortRows) orders[r.projectId] = r.sortOrder;
    state.sortOrders = orders;
    state.groupRevision = revision;
    state.staleSources.groups = false;
  } catch (e) {
    console.warn("loadGroups failed", e);
    state.staleSources.groups = true;
    showActionError(tr("status.staleSources", { sources: "groups" }));
  }
  applyFilters();
  renderSelectedLaunchPanel();
  // Push the now-populated group / assignment state to the tray so the
  // right-click menu groups projects correctly. This runs after every
  // loadGroups() so a later refresh (manual RESCAN → rebuild_assignments,
  // or programmatic reload) keeps the tray in sync.
  syncTrayProjects();
}

/// Re-fetch only the manual sort rows (lighter than loadGroups). Used after
/// an assignment change, since the enhanced `assign_project_to_group` mutates
/// sort rows and the frontend's sortOrders would otherwise go stale.
async function refreshSortOrders() {
  if (!HAS_TAURI) return;
  try {
    const rows = await invoke("list_sort_orders");
    const orders = {};
    for (const r of rows) orders[r.projectId] = r.sortOrder;
    state.sortOrders = orders;
  } catch (e) { /* keep stale on failure */ }
}

async function setProjectGroup(projectId, groupId) {
  const prev = state.assignments[projectId] ?? null;
  // Optimistic update so the UI re-renders immediately.
  const next = { ...state.assignments };
  if (groupId == null) delete next[projectId];
  else next[projectId] = groupId;
  state.assignments = next;
  // Keep group memberCount labels in sync without a server round-trip.
  if (prev !== (groupId ?? null)) {
    for (const g of state.groups) {
      if (g.id === prev) g.memberCount = Math.max(0, (g.memberCount || 0) - 1);
      if (g.id === groupId) g.memberCount = (g.memberCount || 0) + 1;
    }
  }
  applyFilters();
  renderSelectedLaunchPanel();
  // Keep the tray menu in sync with this group edit (project just
  // moved between buckets). Mirrors what reload() does for projects.
  syncTrayProjects();
  if (HAS_TAURI) {
    try {
      await invoke("assign_project_to_group", { projectId, groupId });
      // assign_project_to_group mutates sort rows (appends into a manual
      // group, or clears the row for a non-manual/ungrouped target), so
      // refresh sortOrders to stay in sync with the server.
      await loadGroups();
    } catch (e) {
      console.error("assign_project_to_group failed", e);
      // Revert optimistic update on failure.
      state.assignments = { ...state.assignments };
      if (prev == null) delete state.assignments[projectId];
      else state.assignments[projectId] = prev;
      for (const g of state.groups) {
        if (g.id === prev) g.memberCount = (g.memberCount || 0) + 1;
        if (g.id === groupId) g.memberCount = Math.max(0, (g.memberCount || 0) - 1);
      }
      showActionError(tr("status.groupAssignFailed", { err: e }));
      applyFilters();
      renderSelectedLaunchPanel();
    }
  }
}

async function fireOpener(openerId, projectPath, labelGuess) {
  const opener = state.openerPrefs.find(o => String(o.id) === String(openerId));
  const label = opener?.label || labelGuess || "opener";
  setStatus(tr("status.opening", { label, path: projectPath }));
  if (!HAS_TAURI) {
    setStatus(tr("status.demoWouldOpen", { label, path: projectPath }));
    return;
  }
  try {
    // openerId arrives here as a string (it came from `data-opener-id`
    // on the pill), but the Rust command's `opener_id: i64` rejects
    // string input. Coerce once at the boundary; the local `find` above
    // uses string comparison so it's unaffected.
    await invoke("open_with_opener", { openerId: Number(openerId), path: projectPath });
    setStatus(tr("status.opened", { label, path: projectPath }));
  } catch (e) {
    setStatus(tr("status.openerFailed", { err: e }));
  }
}

/* ── DOM refs ───────────────────────────────────────────── */
const ledger = document.getElementById("ledger");
const ledgerCount = document.getElementById("ledgerCount");
const termsBar = document.getElementById("termsBar");
const termsViewport = document.getElementById("termsViewport");
const termsEmpty = document.getElementById("termsEmpty");
const termsSelectedLaunch = document.getElementById("termsSelectedLaunch");
const termsCount = document.getElementById("termsCount");
const stage = document.getElementById("stage");
const overviewToggleBtn = document.getElementById("overviewToggleBtn");
const entryModal = document.getElementById("entryModal");
const entryModalBody = document.getElementById("entryModalBody");
const entryModalTitle = document.getElementById("entryModalTitle");
const scanProgress = document.getElementById("scanProgress");

/* ── collapsible project overview ──────────────────────── */
function syncOverviewCollapseUI() {
  if (!stage || !overviewToggleBtn || !termsSelectedLaunch) return;
  const collapsed = state.overviewCollapsed;
  const label = tr(collapsed ? "overview.expand" : "overview.collapse");
  stage.classList.toggle("stage--overview-collapsed", collapsed);
  overviewToggleBtn.setAttribute("aria-expanded", String(!collapsed));
  overviewToggleBtn.setAttribute("aria-label", label);
  overviewToggleBtn.setAttribute("title", label);
  overviewToggleBtn.querySelector("span").textContent = collapsed ? "›" : "‹";
  termsSelectedLaunch.setAttribute("aria-hidden", String(collapsed));
  termsSelectedLaunch.inert = collapsed;
}

function setOverviewCollapsed(collapsed, { persist = true } = {}) {
  state.overviewCollapsed = Boolean(collapsed);
  syncOverviewCollapseUI();
  if (persist) {
    try { localStorage.setItem(OVERVIEW_COLLAPSED_KEY, String(state.overviewCollapsed)); }
    catch {}
  }
}

function setupOverviewToggle() {
  if (!overviewToggleBtn) return;
  syncOverviewCollapseUI();
  overviewToggleBtn.addEventListener("click", () => {
    setOverviewCollapsed(!state.overviewCollapsed);
  });
}

/* ── modal focus + background isolation ────────────────── */
// `aria-modal` describes a dialog to assistive technology, but it does not
// prevent keyboard focus from wandering into the dimmed workspace. Keep one
// small shared controller for the entry modal, document preview and settings
// drawer so every overlay behaves the same: background is inert, focus enters
// the surface, Tab wraps inside it, and closing returns to the invoking control.
const dialogReturnFocus = new WeakMap();
const DIALOG_FOCUSABLE = [
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "a[href]",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

function visibleDialog() {
  return [
    document.getElementById("docModal"),
    entryModal,
    document.getElementById("drawer"),
  ].find(surface => surface && !surface.hidden) || null;
}

const OFFLINE_ADAPTER_DEMO = [
  ["claude", "Claude Code", "npm"],
  ["kimi", "Kimi Code", "npm"],
  ["codex", "Codex CLI", "npm"],
  ["opencode", "OpenCode", "npm"],
  ["aider", "Aider", "uv"],
  ["pi", "Pi Coding Agent", "npm"],
];
const TUI_CAPABILITY_TTL_MS = 5 * 60 * 1000;
const TUI_PROBE_CONCURRENCY = 2;
const SEARCH_DEBOUNCE_MS = 120;
function tuiMachineKey(serverId) {
  return serverId == null ? "local" : `remote:${Number(serverId)}`;
}
function tuiMachineForProject(project) {
  return state.tuiMachines[tuiMachineKey(
    project?.source === "remote" ? project.remoteServerId : null,
  )];
}
function tuiCapabilityForProject(project, toolKey) {
  if (!toolKey || toolKey === "shell") return { installed: true, enabled: true };
  return tuiMachineForProject(project)?.tools?.find(tool => tool.toolKey === toolKey) || null;
}
function canLaunchTui(project, toolKey) {
  if (!toolKey || toolKey === "shell") return true;
  const capability = tuiCapabilityForProject(project, toolKey);
  return Boolean(capability?.installed && capability?.enabled);
}
function disabledTuiAttrs(project, toolKey) {
  if (canLaunchTui(project, toolKey)) return "";
  const capability = tuiCapabilityForProject(project, toolKey);
  const title = capability && !capability.installed
    ? tr("tui.launchMissing")
    : capability && !capability.enabled
      ? tr("tui.launchDisabled")
      : tr("tui.launchChecking");
  return `disabled aria-disabled="true" title="${escapeHtml(title)}"`;
}

const _tuiProbeQueue = [];
const _tuiProbeInflight = new Map();
let _tuiProbeActive = 0;

function drainTuiProbeQueue() {
  while (_tuiProbeActive < TUI_PROBE_CONCURRENCY && _tuiProbeQueue.length) {
    const task = _tuiProbeQueue.shift();
    _tuiProbeActive += 1;
    void probeTuiMachine(task.serverId, task.force, task.deferRender)
      .then(task.resolve, task.reject)
      .finally(() => {
        _tuiProbeActive -= 1;
        if (_tuiProbeInflight.get(task.key) === task.promise) _tuiProbeInflight.delete(task.key);
        drainTuiProbeQueue();
      });
  }
}

function refreshTuiMachine(serverId, { force = false, deferRender = false } = {}) {
  const key = tuiMachineKey(serverId);
  const cachedAt = Number(state.tuiCapabilityAt[key] || 0);
  if (!force && state.tuiMachines[key] && Date.now() - cachedAt < TUI_CAPABILITY_TTL_MS) {
    return Promise.resolve(state.tuiMachines[key]);
  }
  const existing = _tuiProbeInflight.get(key);
  if (existing) {
    return existing.then(result => {
      if (!deferRender) {
        if (!drawer.hidden && state.settingsView === "tui") renderDrawerBody();
        applyFilters();
        renderSelectedLaunchPanel();
      }
      return result;
    });
  }
  let resolveTask;
  let rejectTask;
  const promise = new Promise((resolve, reject) => {
    resolveTask = resolve;
    rejectTask = reject;
  });
  _tuiProbeInflight.set(key, promise);
  _tuiProbeQueue.push({ key, serverId, force, deferRender, promise, resolve: resolveTask, reject: rejectTask });
  state.tuiLoadingKeys.add(key);
  if (!drawer.hidden && state.settingsView === "tui") renderDrawerBody();
  drainTuiProbeQueue();
  return promise;
}

async function probeTuiMachine(serverId, force, deferRender) {
  const key = tuiMachineKey(serverId);
  if (!force && state.tuiMachines[key] && Date.now() - Number(state.tuiCapabilityAt[key] || 0) < TUI_CAPABILITY_TTL_MS) {
    state.tuiLoadingKeys.delete(key);
    return state.tuiMachines[key];
  }
  state.tuiLoadingKeys.add(key);
  try {
    let capabilities;
    if (HAS_TAURI) {
      capabilities = await invoke("probe_tui_capabilities", { serverId: serverId ?? null });
    } else {
      capabilities = {
        source: serverId == null ? "local" : "remote",
        serverId: serverId ?? null,
        label: serverId == null ? "Local" : (state.remoteServerById[serverId]?.label || "Remote"),
        tools: OFFLINE_ADAPTER_DEMO.map(([toolKey, toolName, installManager]) => ({
          toolKey, toolName, installed: true, version: "demo", enabled: true,
          adapterEnabled: true, adapterVersion: "1.0.0", adapterSource: "bundled",
          adapterNewestVersion: "1.0.0", adapterUpdateAvailable: false,
          adapterRollbackVersion: null, installAvailable: true, installManager,
          installPackage: toolKey === "aider" ? "aider-chat" : `demo-${toolKey}`,
          latestVersion: null, updateChecked: false, updateAvailable: false, updateCheckError: null,
        })),
      };
    }
    state.tuiMachines[key] = { ...capabilities, error: null };
  } catch (error) {
    state.tuiMachines[key] = {
      source: serverId == null ? "local" : "remote",
      serverId: serverId ?? null,
      label: serverId == null ? "Local" : (state.remoteServerById[serverId]?.label || "Remote"),
      tools: [], error: String(error),
    };
  } finally {
    state.tuiCapabilityAt[key] = Date.now();
    state.tuiLoadingKeys.delete(key);
    if (!deferRender) {
      if (!drawer.hidden && state.settingsView === "tui") renderDrawerBody();
      applyFilters();
      renderSelectedLaunchPanel();
    }
  }
}

async function checkTuiMachineUpdates(serverId) {
  const key = tuiMachineKey(serverId);
  if (state.tuiCheckingKeys.has(key)) return state.tuiMachines[key] || null;
  if (!state.tuiMachines[key]) await refreshTuiMachine(serverId, { force: true });
  state.tuiCheckingKeys.add(key);
  if (!drawer.hidden && state.settingsView === "tui") renderDrawerBody();
  const label = state.tuiMachines[key]?.label
    || (serverId == null ? tr("tui.localMachine") : state.remoteServerById[serverId]?.label)
    || tr("tui.sourceRemote");
  setStatus(tr("status.tuiCheckingUpdates", { machine: label }));
  try {
    let capabilities;
    if (HAS_TAURI) {
      capabilities = await invoke("check_tui_updates", { serverId: serverId ?? null });
    } else {
      capabilities = state.tuiMachines[key];
      capabilities.tools = capabilities.tools.map(tool => ({
        ...tool,
        latestVersion: tool.version,
        updateChecked: tool.installed,
        updateAvailable: false,
        updateCheckError: null,
      }));
    }
    state.tuiMachines[key] = { ...capabilities, error: null };
    state.tuiCapabilityAt[key] = Date.now();
    const updates = capabilities.tools?.filter(tool => tool.updateAvailable).length || 0;
    const errors = capabilities.tools?.filter(tool => tool.installed && tool.updateCheckError).length || 0;
    setStatus(tr("status.tuiUpdatesChecked", { machine: capabilities.label || label, updates, errors }));
    return capabilities;
  } catch (error) {
    showActionError(tr("status.tuiUpdateCheckFailed", { err: error }));
    return state.tuiMachines[key] || null;
  } finally {
    state.tuiCheckingKeys.delete(key);
    if (!drawer.hidden && state.settingsView === "tui") renderDrawerBody();
  }
}

function refreshRemoteTuiAdaptersAfterRegistryChange() {
  for (const server of state.remoteServers) {
    const key = tuiMachineKey(server.id);
    delete state.tuiCapabilityAt[key];
    void refreshTuiMachine(Number(server.id), { force: true });
  }
}

async function mutateLocalTuiAdapter(toolKey, command) {
  if (!toolKey || state.tuiAdapterBusyKeys.has(toolKey)) return;
  const machine = state.tuiMachines.local;
  const tool = machine?.tools?.find(item => item.toolKey === toolKey);
  if (!tool) return;
  const activating = command === "activate_tui_adapter_update";
  const version = activating ? tool.adapterNewestVersion : tool.adapterRollbackVersion;
  const confirmKey = activating ? "tui.adapterActivateConfirm" : "tui.adapterRollbackConfirm";
  if (!window.confirm(tr(confirmKey, { tool: tool.toolName, version }))) return;
  state.tuiAdapterBusyKeys.add(toolKey);
  renderDrawerBody();
  setStatus(tr(activating ? "status.adapterActivating" : "status.adapterRollingBack", {
    tool: tool.toolName, version,
  }));
  try {
    if (HAS_TAURI) {
      state.tuiMachines.local = await invoke(command, { toolKey });
    } else {
      tool.adapterVersion = version || tool.adapterVersion;
      tool.adapterUpdateAvailable = !activating;
    }
    state.tuiCapabilityAt.local = Date.now();
    setStatus(tr(activating ? "status.adapterActivated" : "status.adapterRolledBack", {
      tool: tool.toolName, version,
    }));
    refreshRemoteTuiAdaptersAfterRegistryChange();
  } catch (error) {
    showActionError(tr("status.adapterActionFailed", { err: error }));
  } finally {
    state.tuiAdapterBusyKeys.delete(toolKey);
    renderDrawerBody();
    applyFilters();
    renderSelectedLaunchPanel();
  }
}

function refreshAllTuiCapabilities({ force = false, includeRemote = false } = {}) {
  const validKeys = new Set(["local", ...state.remoteServers.map(server => tuiMachineKey(server.id))]);
  Object.keys(state.tuiMachines).forEach(key => {
    if (!validKeys.has(key)) {
      delete state.tuiMachines[key];
      delete state.tuiCapabilityAt[key];
    }
  });
  const targets = [
    null,
    ...(includeRemote ? state.remoteServers.map(server => Number(server.id)) : []),
  ];
  let cursor = 0;
  let active = 0;
  return new Promise(resolve => {
    const schedule = () => {
      while (active < TUI_PROBE_CONCURRENCY && cursor < targets.length) {
        const serverId = targets[cursor++];
        active += 1;
        void refreshTuiMachine(serverId, { force, deferRender: true }).finally(() => {
          active -= 1;
          schedule();
        });
      }
      if (active === 0 && cursor >= targets.length) {
        if (!drawer.hidden && state.settingsView === "tui") renderDrawerBody();
        applyFilters();
        renderSelectedLaunchPanel();
        resolve();
      }
    };
    schedule();
  });
}

function dialogFocusables(surface) {
  return [...surface.querySelectorAll(DIALOG_FOCUSABLE)]
    .filter(element => element.getClientRects().length > 0);
}

function syncDialogIsolation() {
  const app = document.querySelector(".console");
  if (app) app.inert = Boolean(visibleDialog());
}

function focusDialogStart(surface, preferredSelector = null) {
  const focusTarget = () => {
    if (surface.hidden) return;
    const preferred = preferredSelector
      ? [...surface.querySelectorAll(preferredSelector)]
        .find(element => element.matches(DIALOG_FOCUSABLE)
          && element.getClientRects().length > 0 && !element.disabled)
      : null;
    const target = preferred || dialogFocusables(surface)[0];
    target?.focus({ preventScroll: true });
  };
  // Focus synchronously for keyboard/click activation. Re-apply once after
  // layout as a guard for WebView engines that temporarily reject focus while
  // the dialog changes from `hidden` to visible in the same task.
  focusTarget();
  requestAnimationFrame(() => {
    if (!surface.contains(document.activeElement)) focusTarget();
  });
}

function activateDialog(surface, preferredSelector = null) {
  const current = document.activeElement;
  dialogReturnFocus.set(surface, current instanceof HTMLElement ? current : null);
  syncDialogIsolation();
  focusDialogStart(surface, preferredSelector);
}

function deactivateDialog(surface) {
  const returnTarget = dialogReturnFocus.get(surface);
  dialogReturnFocus.delete(surface);
  syncDialogIsolation();
  if (!visibleDialog() && returnTarget?.isConnected) {
    returnTarget.focus({ preventScroll: true });
  }
}

document.addEventListener("keydown", event => {
  if (event.key !== "Tab") return;
  const surface = visibleDialog();
  if (!surface) return;
  const focusables = dialogFocusables(surface);
  if (!focusables.length) return;
  const first = focusables[0];
  const last = focusables[focusables.length - 1];
  if (!surface.contains(document.activeElement)) {
    event.preventDefault();
    first.focus();
  } else if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}, true);

/* ── ledger rendering ───────────────────────────────────── */
function replaceTextMarker(root, marker, value) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let node;
  while ((node = walker.nextNode())) {
    if (node.nodeValue.includes(marker)) {
      node.nodeValue = node.nodeValue.replaceAll(marker, value);
    }
  }
}

function renderCount() {
  const n = state.all.length;
  const shown = state.filtered.length;
  const searching = state.query.trim().length > 0;
  const recencyOn = state.recency !== "all";
  if (searching) {
    // Keep the translation's trusted <strong> markup, but replace the
    // untrusted query only after parsing, as a text node.
    const marker = "__SESSIONATLAS_QUERY__";
    ledgerCount.innerHTML = tr("ledger.count.matches", { count: shown, query: marker });
    replaceTextMarker(ledgerCount, marker, state.query);
  } else if (recencyOn) {
    const label = state.recency;
    ledgerCount.innerHTML = tr("ledger.count.recency", { shown, total: n, label });
  } else if (n >= LIST_LIMIT) {
    ledgerCount.innerHTML = tr("ledger.count.capped", { limit: LIST_LIMIT });
  } else {
    ledgerCount.innerHTML = tr("ledger.count.entries", { count: n });
  }
}

// Collapsed-group state. Keyed by group id (number) or "ungrouped" for the
// 未分组 bucket. Persisted in localStorage so the user's fold state survives
// across sessions. Defaults to expanded.
const COLLAPSE_KEY = "sessionatlas.collapsedGroups";
function loadCollapsed() {
  try { return new Set(JSON.parse(localStorage.getItem(COLLAPSE_KEY) || "[]")); }
  catch { return new Set(); }
}
function saveCollapsed(set) {
  try { localStorage.setItem(COLLAPSE_KEY, JSON.stringify([...set])); } catch {}
}
const collapsedGroups = loadCollapsed();
function isGroupCollapsed(key) { return collapsedGroups.has(String(key)); }
function toggleGroupCollapse(key) {
  const k = String(key);
  if (collapsedGroups.has(k)) collapsedGroups.delete(k);
  else collapsedGroups.add(k);
  saveCollapsed(collapsedGroups);
  applyFilters(); // updates state.filtered (excludes collapsed) + re-renders
}

// Group header row in the ledger. Clickable to collapse/expand the section.
// `data-group-key` carries the collapse key; keyboard nav skips the header.
function groupHeaderHtml(name, count, key) {
  const collapsed = isGroupCollapsed(key);
  const arrow = collapsed ? "▸" : "▾";
  return `<div class="ledger__group" data-group-toggle data-group-key="${escapeHtml(String(key))}" role="button" tabindex="0" aria-expanded="${!collapsed}">
            <span class="ledger__group__arrow">${arrow}</span>
            <span class="ledger__group__name">${escapeHtml(name)}</span>
            <span class="ledger__group__count">${count}</span>
          </div>`;
}

function renderLedger() {
  // Scrolling is allowed to reuse the existing bounded window, but any
  // content render must invalidate that reuse. This keeps controls such as
  // the expanded queue textarea alive during a scroll while still rebuilding
  // when filters, groups, Git badges, or localized strings change.
  state.ledgerRenderRevision += 1;
  if (state.firstRunError !== null) {
    ledger.innerHTML = `
      <div class="ledger__empty">
        <div class="ledger__empty-num">!</div>
        <div class="ledger__empty-title">${escapeHtml(tr("ledger.firstRunErrorTitle"))}</div>
        <div class="ledger__empty-body">${escapeHtml(tr("ledger.firstRunErrorBody"))}</div>
        <div class="ledger__empty-body ledger__empty-detail">${escapeHtml(tr("ledger.firstRunErrorDetail", { err: state.firstRunError }))}</div>
        <button class="scan-btn" id="firstRunRetry" style="margin-top:14px">${escapeHtml(tr("ledger.tryAgain"))}</button>
      </div>`;
    document.getElementById("firstRunRetry")?.addEventListener("click", () => {
      runLocalScan({ initial: true });
    });
    return;
  }
  if (!state.all.length) state.ledgerRows = [];
  const cutoff = RECENCY_CUTOFFS[state.recency];
  const now = Date.now();
  // `visible` = projects matching tool + recency, IGNORING group collapse.
  // We need the collapse-inclusive set here so a collapsed group still
  // renders its header (with count). Otherwise, when every matching project
  // sits in a collapsed group, the ledger would fall through to the
  // "Archive empty" card and the user would have no header to click to
  // re-expand — the list would look empty even though data is present.
  // `state.filtered` (the nav set) excludes collapsed; this set does not.
  if (!state.ledgerRows.length && state.all.length) {
    state.ledgerRows = buildLedgerRows(state.all.filter(p => matchesFilters(p, cutoff, now)));
  }
  const visible = state.ledgerRows.some(row => row.type === "project");
  if (!visible) {
    const marker = "__SESSIONATLAS_EMPTY_QUERY__";
    const catalogEmpty = !state.query && state.all.length === 0;
    const title = state.query
      ? tr("ledger.emptySearchTitle")
      : tr(catalogEmpty ? "ledger.emptyCatalogTitle" : "ledger.emptyFilteredTitle");
    const body = state.query
      ? tr("ledger.emptyBodySearch", { query: marker })
      : escapeHtml(tr(catalogEmpty ? "ledger.emptyCatalogBody" : "ledger.emptyFilteredBody"));
    ledger.innerHTML = `
      <div class="ledger__empty">
        <div class="ledger__empty-num">∅</div>
        <div class="ledger__empty-title">${escapeHtml(title)}</div>
        <div class="ledger__empty-body">${body}</div>
        ${catalogEmpty ? `<button class="scan-btn" id="emptyScan" style="margin-top:14px">${escapeHtml(tr("ledger.scanNow"))}</button>` : ""}
      </div>`;
    if (state.query) replaceTextMarker(ledger, marker, state.query);
    document.getElementById("emptyScan")?.addEventListener("click", () => runLocalScan());
    return;
  }
  // Render: real groups in their defined order, then "未分组" last.
  // Collapsed groups render only their header (projects hidden) — but
  // they DO render a header, so collapsing a group never makes it vanish.
  renderVirtualLedger();
}

// Keep the cold estimate close to the measured compact CSS contract at the
// desktop ledger width (about 42px including the contained entry margins and
// border). Visited rows are still measured, but a realistic estimate prevents
// a long untouched tail from inventing a phantom scrollbar before it is
// visited.
const LEDGER_GROUP_HEIGHT = 32;
const LEDGER_PROJECT_HEIGHT = 42;
const LEDGER_EXPANDED_HEIGHT = 320;
const LEDGER_OVERSCAN_PX = 480;
let _ledgerRenderFrame = null;

function ledgerVisibleRows() {
  return state.ledgerRows.filter(row =>
    row.type === "group" || !isGroupCollapsed(groupKeyOf(row.project)));
}

function ledgerRowKey(row) {
  return row.type === "group" ? `group:${row.key}` : `project:${row.project.id}`;
}

function estimatedLedgerRowHeight(row) {
  const key = ledgerRowKey(row);
  const measured = Number(state.ledgerHeightByKey.get(key));
  if (measured > 0) return measured;
  return row.type === "group"
    ? LEDGER_GROUP_HEIGHT
    : (row.project.id === state.expandedId ? LEDGER_EXPANDED_HEIGHT : LEDGER_PROJECT_HEIGHT);
}

function ledgerLayoutFor(rows) {
  const offsets = [];
  const heights = [];
  let totalHeight = 0;
  for (const row of rows) {
    offsets.push(totalHeight);
    const height = estimatedLedgerRowHeight(row);
    heights.push(height);
    totalHeight += height;
  }
  return { rows, offsets, heights, totalHeight };
}

function virtualRowHtml(row) {
  const content = row.type === "group"
    ? groupHeaderHtml(row.name, row.count, row.key)
    : entryHtml(row.project);
  // A flow-root wrapper contains the entry/group's vertical margins. Without
  // it, block margin collapsing makes the measured height smaller than the
  // distance to the next row and the virtual offsets drift as the list grows.
  return `<div class="ledger__virtual-row">${content}</div>`;
}

function measureVirtualRows(window, layout, start, previousScrollTop) {
  const oldLayout = state.ledgerLayout;
  const oldAnchorIndex = oldLayout
    ? oldLayout.offsets.findIndex((offset, index) => offset + oldLayout.heights[index] > previousScrollTop)
    : -1;
  const oldAnchorKey = oldAnchorIndex >= 0 ? ledgerRowKey(oldLayout.rows[oldAnchorIndex]) : null;
  const oldAnchorOffset = oldAnchorIndex >= 0 ? oldLayout.offsets[oldAnchorIndex] : 0;
  let changed = false;
  [...window.children].forEach((element, index) => {
    const row = layout.rows[start + index];
    if (!row) return;
    const measured = element.getBoundingClientRect().height;
    if (!(measured > 0)) return;
    const key = ledgerRowKey(row);
    const previous = state.ledgerHeightByKey.get(key);
    if (!previous || Math.abs(previous - measured) > 0.5) {
      state.ledgerHeightByKey.set(key, measured);
      changed = true;
    }
  });
  if (!changed) return false;
  const updated = ledgerLayoutFor(layout.rows);
  state.ledgerLayout = updated;
  if (oldAnchorKey) {
    const newAnchorIndex = updated.rows.findIndex(row => ledgerRowKey(row) === oldAnchorKey);
    if (newAnchorIndex >= 0) {
      const delta = updated.offsets[newAnchorIndex] - oldAnchorOffset;
      if (Math.abs(delta) > 0.5) ledger.scrollTop = Math.max(0, previousScrollTop + delta);
    }
  }
  return true;
}

function renderVirtualLedger() {
  // Dragging owns the current row nodes. Replacing the virtual window while
  // the pointer is moving would cancel the native drag and lose indicators.
  if (state._dragId) return;
  const rows = ledgerVisibleRows();
  if (!rows.length) return;
  const previousScrollTop = ledger.scrollTop;
  const layout = ledgerLayoutFor(rows);
  const { offsets, heights, totalHeight } = layout;
  const viewportHeight = Math.max(ledger.clientHeight || 600, 1);
  const top = Math.max(previousScrollTop - LEDGER_OVERSCAN_PX, 0);
  const bottom = previousScrollTop + viewportHeight + LEDGER_OVERSCAN_PX;
  let start = 0;
  while (start < offsets.length && offsets[start] + heights[start] < top) start += 1;
  let end = start;
  while (end < rows.length && offsets[end] < bottom) end += 1;
  const html = rows.slice(start, end).map(virtualRowHtml).join("");
  const windowSignature = JSON.stringify([
    state.ledgerRenderRevision,
    start,
    end,
    rows.slice(start, end).map(ledgerRowKey),
  ]);
  const spacer = ledger.querySelector(".ledger__virtual-spacer");
  const window = spacer?.querySelector(".ledger__virtual-window");
  if (spacer && window) {
    // Keep the scroll container itself intact while scrolling. Replacing its
    // innerHTML would reset scrollTop and can schedule an endless scroll
    // feedback loop in Chromium/WebView2.
    spacer.style.height = `${totalHeight}px`;
    window.style.transform = `translateY(${offsets[start] || 0}px)`;
    // Keep the current nodes when the bounded range is unchanged. In
    // particular, this preserves an expanded row's textarea value, selection
    // and focus while a nearby scroll event merely updates layout metrics.
    if (state.ledgerVirtualWindowSignature !== windowSignature) {
      window.innerHTML = html;
      state.ledgerVirtualWindowSignature = windowSignature;
    }
  } else {
    ledger.innerHTML = `<div class="ledger__virtual-spacer" style="height:${totalHeight}px"><div class="ledger__virtual-window" style="transform:translateY(${offsets[start] || 0}px)">${html}</div></div>`;
    state.ledgerVirtualWindowSignature = windowSignature;
  }
  const renderedWindow = ledger.querySelector(".ledger__virtual-window");
  if (renderedWindow && measureVirtualRows(renderedWindow, layout, start, previousScrollTop)) {
    requestAnimationFrame(renderVirtualLedger);
  } else {
    state.ledgerLayout = layout;
  }
}

function wireLedgerVirtualization() {
  ledger.addEventListener("scroll", () => {
    if (_ledgerRenderFrame) return;
    _ledgerRenderFrame = requestAnimationFrame(() => {
      _ledgerRenderFrame = null;
      renderVirtualLedger();
    });
  }, { passive: true });
  ledger.addEventListener("click", (event) => {
    const group = event.target.closest("[data-group-toggle]");
    if (group) {
      toggleGroupCollapse(group.dataset.groupKey);
      return;
    }
    const entry = event.target.closest(".entry");
    const treeButton = event.target.closest("[data-tree-btn]");
    if (treeButton && entry) {
      // The files-view delegate runs after this listener; select first so it
      // scopes the tree request to the row whose icon was clicked.
      select(entry.dataset.id);
      return;
    }
    if (event.target.closest("button, a, input, textarea, select, [data-queue-panel]")) return;
    if (entry) select(entry.dataset.id);
  });
  ledger.addEventListener("dblclick", (event) => {
    if (event.target.closest("button, a, input, textarea, select, [data-queue-panel]")) return;
    const entry = event.target.closest(".entry");
    const project = entry && state.all.find(item => item.id === entry.dataset.id);
    if (project) openProjectDefault(project);
  });
  ledger.addEventListener("keydown", (event) => {
    const group = event.target.closest("[data-group-toggle]");
    if (group && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      toggleGroupCollapse(group.dataset.groupKey);
    }
  });
}

function normalizeOsFamily(value) {
  const normalized = String(value || "").trim().toLowerCase();
  if (normalized.includes("windows") || /^(mingw|msys|cygwin)/.test(normalized)) return "windows";
  if (normalized.includes("darwin") || normalized.includes("macos") || normalized.includes("mac os")) return "macos";
  if (normalized.includes("linux")) return "linux";
  if (/(bsd|sunos|solaris|aix|unix)/.test(normalized)) return "unix";
  return "unknown";
}

function currentBrowserOsFamily() {
  return normalizeOsFamily(navigator.userAgentData?.platform || navigator.platform || navigator.userAgent);
}

function operatingSystemPresentation(value) {
  const family = normalizeOsFamily(value);
  return {
    family,
    icon: family === "unknown" ? "unknownOs" : family,
    label: {
      windows: "Windows",
      linux: "Linux",
      macos: "macOS",
      unix: "Unix",
      unknown: "Unknown OS",
    }[family],
  };
}

function machineIdentityIconsHtml(kind, osFamily, className = "") {
  const os = operatingSystemPresentation(osFamily);
  const deviceIcon = kind === "remote" ? "remoteServer" : "localMachine";
  return `<span class="machine-identity ${className}" data-machine-kind="${kind}"
                data-os-family="${os.family}" title="${escapeHtml(os.label)}">
    <span class="machine-identity__device">${iconSvg(deviceIcon, { size: 12 })}</span>
    <span class="machine-identity__os machine-identity__os--${os.family}">${iconSvg(os.icon, { size: 10 })}</span>
  </span>`;
}

function projectSourcePresentation(project) {
  if (project?.source !== "remote") {
    const os = operatingSystemPresentation(project?.osFamily || currentBrowserOsFamily());
    return {
      kind: "local",
      label: tr("entry.sourceLocal"),
      os,
      title: `${tr("entry.sourceLocalTitle", { path: project?.path || "" })} · ${os.label}`,
    };
  }

  const server = state.remoteServerById[project.remoteServerId];
  const label = server?.label || server?.host || tr("entry.sourceRemoteUnknown");
  const connection = server
    ? `${server.user}@${server.host}:${server.port}`
    : project?.path || "";
  const os = operatingSystemPresentation(server?.osFamily || project?.osFamily);
  return {
    kind: "remote",
    label: tr("entry.sourceRemote", { label }),
    os,
    title: `${tr("entry.sourceRemoteTitle", { label, connection })} · ${os.label}`,
  };
}

function projectSourceBadgeHtml(project, className) {
  const source = projectSourcePresentation(project);
  return `<span class="project-source project-source--${source.kind} ${className}"
                data-project-source="${source.kind}"
                data-os-family="${source.os.family}"
                aria-label="${escapeHtml(`${source.label} · ${source.os.label}`)}"
                title="${escapeHtml(source.title)}">
    <span class="project-source__device">${iconSvg(source.kind === "remote" ? "remoteServer" : "localMachine", { size: 10 })}</span>
    <span class="project-source__label">${escapeHtml(source.label)}</span>
    <span class="project-source__os project-source__os--${source.os.family}">${iconSvg(source.os.icon, { size: 9 })}</span>
  </span>`;
}

function projectGitSyncBadgeHtml(project) {
  const info = state.gitStatusByProject[project.id];
  if (!info?.isRepo || !info.upstream) return "";
  const badges = [];
  if (Number(info.ahead) > 0) {
    badges.push(`<span class="entry__git-state entry__git-state--ahead" title="${escapeHtml(tr("git.aheadTitle", { count: info.ahead }))}">${escapeHtml(tr("git.needsPush", { count: info.ahead }))}</span>`);
  }
  if (Number(info.behind) > 0) {
    badges.push(`<span class="entry__git-state entry__git-state--behind" title="${escapeHtml(tr("git.behindTitle", { count: info.behind }))}">${escapeHtml(tr("git.notLatest", { count: info.behind }))}</span>`);
  }
  return badges.length ? `<span class="entry__git-sync">${badges.join("")}</span>` : "";
}

function usageHasResumeTarget(usage) {
  return Boolean(String(usage?.lastSessionId || "").trim());
}

function activityOnlyLabelKey(usage, scope = "entry") {
  const suffix = usage?.toolKey === "opencode"
    ? "auxiliaryActivity"
    : "activityOnly";
  return `${scope}.${suffix}`;
}

// Activity-only tool records stay visible, but a project-level launch should
// prefer a real main-session target from any tool. This keeps a newer
// delegated OpenCode run from shadowing the user's resumable Codex session.
function preferredProjectUsage(project) {
  const usages = project?.toolUsages || [];
  return usages.find(usageHasResumeTarget) || usages[0] || null;
}

function entryHtml(p) {
  const t = relTime(p.lastAccessedAt);
  // The server already returns toolUsages ordered by lastUsedAt DESC, so
  // the first element is the most-recently-used tool for the dot indicator.
  const topUsage = (p.toolUsages || [])[0];
  const dotKey = topUsage?.toolKey;
  const dot = dotKey
    ? `<i class="entry__tool-dot ${toolDotClass(dotKey)}" style="background:${toolColor(dotKey)}"></i>`
    : "";
  // Mark projects that have at least one open terminal session, so the
  // user can tell at a glance which rows have a live process running.
  // File-tree tabs (kind="file") aren't terminal sessions — only PTY tabs
  // count, so opening a markdown file in the right pane doesn't turn the
  // project name green.
  const openTabs = state.tabs.filter(
    t => t.kind === "pty" && !t.dead && t.project.id === p.id,
  );
  const hasSession = openTabs.length > 0;
  const sessionTools = hasSession
    ? Array.from(new Set(openTabs.map(t => t.usage?.toolKey || "shell"))).join(", ")
    : "";
  const sessionMarker = hasSession
    ? `<span class="entry__session" title="${escapeHtml(tr("entry.sessionMarker", { count: openTabs.length, tools: sessionTools }))}"></span>`
    : "";
  const isMenuOpen = state.menuOpenId === p.id;
  const isExpanded = state.expandedId === p.id;
  // Per-tool session panel — one card per recorded tool usage. The Resume
  // pill passes the specific toolKey (and session id, embedded in the
  // data-sid attribute) through to openTerminalTab so the auto-launch
  // command uses the trusted backend's tool-specific resume syntax.
  // Do not build session cards, tool buttons, or the queue textarea for a
  // collapsed row. The virtual renderer only needs the compact article until
  // the user explicitly expands it.
  let expandedPanel = "";
  if (isExpanded) {
  const sessionCards = (p.toolUsages || []).map(u => {
    const sid = u.lastSessionId || "";
    const shortSid = sid ? sid.slice(0, 8) : "";
    const canResume = usageHasResumeTarget(u);
    const sessionSummary = canResume
      ? tr("entry.sessionsCount", { count: u.sessionCount })
      : tr(activityOnlyLabelKey(u));
    const actionLabel = canResume ? tr("entry.resume") : tr("entry.newSession");
    const activityOnlyClass = canResume ? "" : " session-card--activity-only";
    const actionClass = canResume ? "" : " launch-pill--ghost";
    return `
      <div class="session-card${activityOnlyClass}">
        <i class="session-card__dot ${toolDotClass(u.toolKey)}" style="background:${toolColor(u.toolKey)}"></i>
        <div class="session-card__info">
          <div class="session-card__name">${escapeHtml(u.toolName)}</div>
          <div class="session-card__meta">
            <span>${escapeHtml(sessionSummary)}</span>
            <span class="session-card__sep">·</span>
            <span>${escapeHtml(tr("entry.lastPrefix", { time: relTime(u.lastUsedAt) }))}</span>
            ${shortSid ? `<span class="session-card__sep">·</span><span class="session-card__sid" title="${escapeHtml(sid)}">${escapeHtml(shortSid)}</span>` : ""}
          </div>
        </div>
        <div class="session-card__actions">
          <button class="launch-pill${actionClass}" data-launch-tool="${escapeHtml(u.toolKey)}" data-launch-sid="${escapeHtml(sid)}" draggable="false" ${disabledTuiAttrs(p, u.toolKey)}>${escapeHtml(actionLabel)}</button>
        </div>
      </div>`;
  }).join("");
  // Tools recorded for THIS project — the complement of state.tools gives the
  // tools the project has never opened a session with. Offer those as "new
  // session" pills so a user can start e.g. a fresh Kimi session even when
  // only Claude has a recorded usage. They reuse the data-launch-tool path
  // (no data-launch-sid → bare command, no resume selector) so the existing ledger
  // delegated handler launches them with zero logic change.
  const recorded = new Set((p.toolUsages || []).map(u => u.toolKey));
  const newToolPills = (state.tools || [])
    .filter(t => !recorded.has(t.toolKey) && canLaunchTui(p, t.toolKey))
    .map(t => `<button class="launch-pill launch-pill--ghost" data-launch-tool="${escapeHtml(t.toolKey)}" draggable="false">${toolIcon(t.toolKey)}${escapeHtml(t.toolName)}</button>`)
    .join("");
  // Claude task queue panel. Only shown for projects that have a Claude
  // usage record — the queue runs `claude -p` so it only makes sense when
  // the project is a Claude Code project. If a queue tab is already open
  // for this project, the button label switches to "add to queue".
  const hasClaude = (p.toolUsages || []).some(u => u.toolKey === "claude")
    && canLaunchTui(p, "claude");
  const existingQueueTab = hasClaude
    ? state.tabs.find(t => t.isQueue && t.project?.id === p.id)
    : null;
  const queuePanel = hasClaude ? `
      <div class="entry__expanded__queue" data-queue-panel data-project-id="${escapeHtml(p.id)}">
        <span class="entry__expanded__queue-label">${escapeHtml(tr("queue.panelLabel"))}</span>
        <textarea class="entry__expanded__queue-input" data-queue-input rows="3"
          placeholder="${escapeHtml(tr("queue.placeholder"))}"></textarea>
        <div class="entry__expanded__queue-foot">
          <span class="entry__expanded__queue-hint">${escapeHtml(tr("queue.hint"))}</span>
          <button class="launch-pill launch-pill--queue" data-queue-run draggable="false">
            ${existingQueueTab ? escapeHtml(tr("queue.addToQueue")) : escapeHtml(tr("queue.runQueue"))}
          </button>
        </div>
        ${existingQueueTab ? `<div class="entry__expanded__queue-status">${escapeHtml(tr("queue.queueOpen", { idx: existingQueueTab.queueIdx + 1, total: existingQueueTab.queueTotal }))}</div>` : ""}
      </div>` : "";
  expandedPanel = `
    <div class="entry__expanded" ${isExpanded ? "" : "hidden"}>
      <div class="entry__expanded__label">${escapeHtml(tr("entry.label.openSession"))}</div>
      ${sessionCards || `<div class="entry__expanded__empty">${escapeHtml(tr("entry.noInstrumentsHint"))}</div>`}
      ${newToolPills ? `<div class="entry__expanded__new">
        <span class="entry__expanded__new-label">${escapeHtml(tr("entry.newSession"))}</span>
        <div class="entry__expanded__new-pills">${newToolPills}</div>
      </div>` : ""}
      <div class="entry__expanded__shell">
        <button class="launch-pill launch-pill--new" data-launch-tool="shell" draggable="false">${escapeHtml(tr("entry.cliNew"))}</button>
      </div>
      ${queuePanel}
    </div>`;
  }
  const expandLabel = isExpanded ? tr("entry.collapseSessions") : tr("entry.expandSessions");
  return `
    <article class="entry ${state.selectedId===p.id?"is-selected":""} ${isMenuOpen?"is-menu-open":""} ${hasSession?"has-session":""} ${isExpanded?"is-expanded":""}" data-id="${p.id}" draggable="true">
      <div class="entry__body">
        <button type="button" class="entry__expand" data-expand-toggle draggable="false"
          aria-expanded="${isExpanded}" aria-label="${escapeHtml(expandLabel)}" title="${escapeHtml(expandLabel)}">
          <span class="entry__expand-icon" aria-hidden="true">▸</span>
        </button>
        <span class="entry__folder" aria-hidden="true">${folderIconSvg()}</span>
        <div class="entry__name">${escapeHtml(p.name)}</div>
        ${projectSourceBadgeHtml(p, "entry__source")}
        ${projectGitSyncBadgeHtml(p)}
      </div>
      <div class="entry__meta-col">
        ${sessionMarker}
        ${dot}
        <div class="entry__time">${t}</div>
        <button class="entry__tree-btn" data-tree-btn draggable="false" aria-label="${escapeHtml(tr("entry.showFileTree"))}" title="${escapeHtml(tr("entry.showFileTree"))}">${ICON_FILES}</button>
        <button class="entry__menu-btn" data-menu-toggle draggable="false" aria-label="${escapeHtml(tr("entry.more"))}" title="${escapeHtml(tr("entry.more"))}">⋯</button>
      </div>
      ${expandedPanel}
    </article>`;
}

function webDevelopmentLaunchPillsHtml() {
  return state.webDevelopmentTools
    .filter(tool => tool.enabled)
    .map(tool => {
      let host = tr("settings.webDevelopment.webBadge");
      try { host = new URL(tool.connectionUrl).host || host; } catch {}
      return `<button class="launch-pill launch-pill--web"
                data-web-development-launch="${escapeHtml(String(tool.id))}">
                ${escapeHtml(tool.label)}<small>${escapeHtml(host)}</small>
              </button>`;
    }).join("");
}

function entryMenuHtml(p, isOpen) {
  // Path + tags + branch + AI pills + external opener pills, all in one
  // popover. Hidden by default; toggled by the "..." button.
  // Server returns toolUsages in lastUsedAt-DESC order already.
  const usages = p.toolUsages || [];
  const sessAbbr = tr("entry.sessAbbr");
  const tags = usages.map(u =>
    `<span class="tag"><i class="dot ${toolDotClass(u.toolKey)}" style="background:${toolColor(u.toolKey)}"></i>${escapeHtml(u.toolName)}<small>${u.sessionCount}${escapeHtml(sessAbbr).slice(0,1)}</small></span>`).join("");
  const branch = p.gitBranch
    ? `<span class="tag tag--branch">⎇ ${escapeHtml(p.gitBranch)}</span>` : "";

  const pills = usages.map(u =>
    `<button class="launch-pill" data-project-id="${p.id}" data-tool="${escapeHtml(u.toolKey)}" ${disabledTuiAttrs(p, u.toolKey)}>
       <i class="dot ${toolDotClass(u.toolKey)}" style="background:${toolColor(u.toolKey)}"></i>
       ${escapeHtml(u.toolName)}<small>${u.sessionCount} ${escapeHtml(sessAbbr)}</small>
     </button>`).join("");

  const openers = p.source === "remote" ? [] : (state.openerPrefs || []).filter(o => o.enabled);
  const openHint = tr("entry.openHint");
  const openerPills = openers.map(o => {
    const hint = (o.command || "").replace(/\{path\}/g, "").trim() || openHint;
    return `<button class="launch-pill launch-pill--ext"
              data-project-id="${p.id}" data-opener-id="${escapeHtml(String(o.id))}">
              ${escapeHtml(o.label)}<small>${escapeHtml(hint)}</small>
    </button>`;
  }).join("");
  const webDevelopmentPills = webDevelopmentLaunchPillsHtml();

  return `
    ${groupPickerHtml(p.id)}
    <div class="entry-menu__path">${escapeHtml(p.path)}</div>
    <div class="entry-menu__ignore">
      <span class="entry__launch-label">${escapeHtml(tr("entry.label.visibility"))}</span>
      <button class="launch-pill launch-pill--danger" data-ignore-project>
        ${escapeHtml(tr("entry.ignoreTree"))}
      </button>
    </div>
    <div class="entry-menu__tags">${tags}${branch}</div>
    ${p.source === "remote"
      ? `<div class="entry-menu__docs">
          <span class="entry__launch-label">${escapeHtml(tr("entry.label.docs"))}</span>
          <div class="entry-menu__docs-list"><span class="docs-status">${escapeHtml(tr("entry.docsRemote"))}</span></div>
        </div>`
      : `<div class="entry-menu__docs">
          <span class="entry__launch-label">${escapeHtml(tr("entry.label.docs"))}</span>
          <div class="entry-menu__docs-list" data-docs-list>
            <span class="docs-status">${escapeHtml(tr("common.loading"))}</span>
          </div>
        </div>
        <div class="entry-menu__files">
          <span class="entry__launch-label">${escapeHtml(tr("entry.label.files"))}</span>
          <div class="entry-menu__files-tree" data-files-tree>
            <span class="docs-status">${escapeHtml(tr("common.loading"))}</span>
          </div>
        </div>`}
    <div class="entry__launch">
      <span class="entry__launch-label">${escapeHtml(tr("entry.label.openSession"))}</span>
      ${pills || `<span style="color:var(--bone-mute);font-size:12px">${escapeHtml(tr("entry.noInstruments"))}</span>`}
      <button class="launch-pill launch-pill--new" data-project-id="${p.id}" data-tool="shell">${escapeHtml(tr("entry.shellNew"))}</button>
      ${openers.length ? `
        <div class="entry__openers">
          <span class="entry__launch-label entry__launch-label--ember">${escapeHtml(tr("entry.label.openWith"))}</span>
          ${openerPills}
        </div>` : ""}
      ${webDevelopmentPills ? `
        <div class="entry__openers entry__web-development">
          <span class="entry__launch-label">${escapeHtml(tr("entry.label.webDevelopment"))}</span>
          ${webDevelopmentPills}
        </div>` : ""}
    </div>`;
}

/* ── right-side quick commands strip ────────────────────── */
// Per-tool preset list. The sidebar shows commands tailored to the active
// tab's tool: claude/codex/kimi/opencode/aider/pi each get their own workflow
// shortcuts, plain-shell tabs get a shell-flavored set, and `null` (no tab)
// shows universal navigation. Hint = short uppercase category label.
//
// Note: only the first line of a multi-line block is sent. Plain shell can
// take anything; AI tool shells would treat most of these as ordinary text,
// so each tool's list is curated to commands that genuinely help when the
// tool is already running.
const COMMON_COMMANDS_BY_TOOL = {
  shell: {
    title: "SHELL",
    items: [
      { cmd: "clear",                hint: "screen"  },
      { cmd: "pwd",                  hint: "where"   },
      { cmd: "ls -la",               hint: "list"    },
      { cmd: "git status",           hint: "git"     },
      { cmd: "git log --oneline -10",hint: "git"     },
      { cmd: "git diff",             hint: "git"     },
      { cmd: "npm install",          hint: "node"    },
      { cmd: "cargo build",          hint: "rust"    },
      { cmd: "cargo test",           hint: "rust"    },
      { cmd: "python -m pytest",     hint: "python"  },
    ],
  },
  claude: {
    title: "CLAUDE",
    items: [
      { cmd: "/help",                hint: "help"    },
      { cmd: "/status",              hint: "status"  },
      { cmd: "/context",             hint: "ctx"     },
      { cmd: "/clear",               hint: "screen"  },
      { cmd: "/compact",             hint: "memory"  },
      { cmd: "/agents",              hint: "agents"  },
      { cmd: "/mcp",                 hint: "mcp"     },
      { cmd: "/resume",              hint: "resume"  },
      { cmd: "/cost",                hint: "usage"   },
    ],
  },
  codex: {
    title: "CODEX",
    items: [
      { cmd: "/help",                hint: "help"    },
      { cmd: "/status",              hint: "status"  },
      { cmd: "/model",               hint: "model"   },
      { cmd: "/approvals",           hint: "policy"  },
      { cmd: "/clear",               hint: "screen"  },
      { cmd: "/diff",                hint: "diff"    },
      { cmd: "/quit",                hint: "exit"    },
    ],
  },
  kimi: {
    title: "KIMI",
    items: [
      { cmd: "/help",                hint: "help"    },
      { cmd: "/clear",               hint: "screen"  },
      { cmd: "/tools",               hint: "tools"   },
      { cmd: "/context",             hint: "ctx"     },
      { cmd: "/model",               hint: "model"   },
      { cmd: "/session",             hint: "session" },
      { cmd: "/exit",                hint: "exit"    },
    ],
  },
  opencode: {
    title: "OPENCODE",
    items: [
      { cmd: "/help",                hint: "help"    },
      { cmd: "/clear",               hint: "screen"  },
      { cmd: "/models",              hint: "model"   },
      { cmd: "/sessions",            hint: "session" },
      { cmd: "/config",              hint: "config"  },
      { cmd: "/exit",                hint: "exit"    },
    ],
  },
  aider: {
    title: "AIDER",
    items: [
      { cmd: "/help",                hint: "help"    },
      { cmd: "/clear",               hint: "screen"  },
      { cmd: "/add",                 hint: "files"   },
      { cmd: "/drop",                hint: "files"   },
      { cmd: "/ls",                  hint: "files"   },
      { cmd: "/model",               hint: "model"   },
      { cmd: "/commit",              hint: "git"     },
      { cmd: "/undo",                hint: "undo"    },
      { cmd: "/exit",                hint: "exit"    },
    ],
  },
  pi: {
    title: "PI",
    items: [
      { cmd: "/help",                hint: "help"    },
      { cmd: "/model",               hint: "model"   },
      { cmd: "/settings",            hint: "config"  },
      { cmd: "/session",             hint: "session" },
      { cmd: "/tree",                hint: "tree"    },
      { cmd: "/resume",              hint: "resume"  },
      { cmd: "/compact",             hint: "memory"  },
      { cmd: "/quit",                hint: "exit"    },
    ],
  },
};

function activeToolKey() {
  const tab = state.tabs.find(t => t.ptyId === state.activeTabId);
  return tab?.usage?.toolKey || null;
}

function renderCommonCommands() {
  const aside = document.getElementById("termsCommands");
  if (!aside) return;
  const toolKey = activeToolKey();
  // An extension adapter must never inherit shell shortcuts inside its TUI.
  // Official tools keep curated presentation-only presets; unknown adapters
  // get an empty strip until a future adapter API explicitly models shortcuts.
  const preset = toolKey
    ? (COMMON_COMMANDS_BY_TOOL[toolKey] || { title: toolKey.toUpperCase(), items: [] })
    : COMMON_COMMANDS_BY_TOOL.shell;
  // Re-render the body; keep the title node so we can update its text cheaply.
  // Prepend a monogram tile when we're in a tool-specific mode (claude,
  // codex, etc.); shell mode keeps the bare text "SHELL".
  const titleEl = aside.querySelector(".terms__commands__title");
  aside.innerHTML = "";
  titleEl.innerHTML = toolKey
    ? `${toolIcon(toolKey)} <span class="terms__commands__title-text">${escapeHtml(preset.title)}</span>`
    : escapeHtml(tr("terms.shell"));
  aside.appendChild(titleEl);
  for (const c of preset.items) {
    const btn = document.createElement("button");
    btn.className = "terms__cmd-btn";
    btn.type = "button";
    btn.dataset.cmd = c.cmd;
    btn.disabled = !state.activeTabId;
    const hintLabel = tr(`hint.${c.hint}`);
    btn.innerHTML = `${escapeHtml(c.cmd)}<small>${escapeHtml(hintLabel)}</small>`;
    btn.addEventListener("click", () => sendToActivePty(c.cmd));
    aside.appendChild(btn);
  }
}

function sendToActivePty(cmd) {
  const tab = state.tabs.find(t => t.ptyId === state.activeTabId);
  if (!tab || tab.dead) { setStatus(tr("status.noActiveSession")); return; }
  invoke("pty_write", { id: tab.ptyId, data: cmd + "\r" }).catch(e => setStatus(tr("status.writeFailed", { err: e })));
  // Focus the receiving terminal so the user sees the output appear.
  try { tab.term.focus(); } catch {}
}

// Keep the sidebar buttons disabled when no session is active so the user
// gets immediate feedback instead of an error toast on click. Also re-render
// the preset whenever the active tab changes so the list tracks the tool.
function updateCommonCommandsEnabled() {
  renderCommonCommands();
}

// Toggle `body.is-maximized` so CSS can drop the .console centering + edge
// padding when the window is full-screen. Driven by Tauri's window API:
// `isMaximized()` for the initial state, and the `tauri://maximize` /
// `tauri://unmaximize` events for transitions. Browser-demo mode never
// maximizes so we skip.
async function trackMaximizedState() {
  if (!HAS_TAURI) return;
  const T = window.__TAURI__;
  const w = T?.window?.getCurrentWindow?.();
  if (!w) return;
  try {
    const apply = async () => {
      const max = await w.isMaximized().catch(() => false);
      document.body.classList.toggle("is-maximized", max);
    };
    await apply();
    if (typeof listen === "function") {
      listen("tauri://maximize",   () => document.body.classList.add("is-maximized"));
      listen("tauri://unmaximize", () => document.body.classList.remove("is-maximized"));
    }
  } catch { /* non-fatal — just stay in the default padded layout */ }
}

/* ── project docs (markdown preview) ────────────────────── */
// Fetch the markdown files for a project and read its contents through
// the Rust commands. Both swallow errors (the UI shows a soft "no docs"
// status so a missing/permission-denied dir never blocks the entry menu).

async function fetchProjectDocs(path) {
  if (!HAS_TAURI) return [];
  return invoke("list_project_docs", { path });
}

async function fetchProjectDoc(path, relPath) {
  if (!HAS_TAURI) return null;
  return invoke("read_project_doc", { path, relPath });
}

// Human-readable byte size (e.g. 1.2K / 480 / 3.4M).
function formatBytes(n) {
  if (n == null) return "";
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}K`;
  return `${(n / 1024 / 1024).toFixed(1)}M`;
}

// Populate the placeholder we left in entryMenuHtml with actual doc pills.
async function loadProjectDocsIntoEntryMenu(p) {
  const listEl = entryModalBody.querySelector("[data-docs-list]");
  if (!listEl) return;
  const requestId = entryDocsGate.begin();
  const projectId = p.id;
  let docs;
  try {
    docs = await fetchProjectDocs(p.path);
  } catch (e) {
    console.warn("list_project_docs failed", e);
    if (entryDocsGate.isCurrent(requestId) && listEl.isConnected
        && state.menuOpenId === projectId) {
      listEl.innerHTML = `<span class="docs-status docs-status--err">${escapeHtml(tr("status.readFailed", { path: p.path }))}</span>`;
    }
    return;
  }
  if (!entryDocsGate.isCurrent(requestId) || !listEl.isConnected
      || state.menuOpenId !== projectId) return;
  if (!docs.length) {
    listEl.innerHTML = `<span class="docs-status">${escapeHtml(tr("entry.noDocs"))}</span>`;
    return;
  }
  listEl.innerHTML = docs.map(d => `
    <button class="doc-pill" data-doc-path="${escapeHtml(d.relPath)}" title="${escapeHtml(d.relPath)}">
      <span class="doc-pill__icon">¶</span>
      <span class="doc-pill__name">${escapeHtml(d.name)}</span>
      <span class="doc-pill__size">${formatBytes(d.size)}</span>
    </button>
  `).join("");
  listEl.querySelectorAll(".doc-pill").forEach(btn => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const name = btn.querySelector(".doc-pill__name").textContent;
      await openDocModal(p, btn.dataset.docPath, name);
    });
  });
}

async function openDocModal(project, relPath, name) {
  closeEntryMenu();
  const modal = document.getElementById("docModal");
  const title = document.getElementById("docModalTitle");
  const body = document.getElementById("docModalBody");
  title.textContent = `${project.name} · ${name}`;
  body.innerHTML = `<div class="docs-status">${escapeHtml(tr("common.loading"))}</div>`;
  modal.hidden = false;
  activateDialog(modal, "[data-doc-modal-close]");
  const requestId = docModalGate.begin();
  const identity = `${project.id}\0${relPath}`;
  modal.dataset.docIdentity = identity;
  let text;
  try {
    text = await fetchProjectDoc(project.path, relPath);
  } catch (e) {
    console.warn("read_project_doc failed", e);
    if (docModalGate.isCurrent(requestId) && !modal.hidden
        && modal.dataset.docIdentity === identity && body.isConnected) {
      body.innerHTML = `<div class="docs-status docs-status--err">${escapeHtml(tr("status.readFailed", { path: relPath }))}</div>`;
    }
    return;
  }
  if (!docModalGate.isCurrent(requestId) || modal.hidden
      || modal.dataset.docIdentity !== identity || !body.isConnected) return;
  body.innerHTML = renderMarkdown(text);
}

function closeDocModal() {
  docModalGate.invalidate();
  const modal = document.getElementById("docModal");
  modal.hidden = true;
  delete modal.dataset.docIdentity;
  deactivateDialog(modal);
}

/* ── project file tree ───────────────────────────────────── */
// Lazily-expanded directory tree of the selected project, shown in
// the entry modal ("⋯" popup). Root loads on modal open; child
// directories load on click. We never pre-fetch the whole tree.

async function fetchDir(path) {
  if (!HAS_TAURI) return [];
  return invoke("list_dir", { path });
}

function joinPath(parent, name) {
  // Build a sibling path that works on both Windows and Unix without
  // pulling in path.posix/path.win32 — `parent` already has trailing
  // separator if needed, otherwise insert.
  if (!parent) return name;
  const sep = parent.includes("\\") ? "\\" : "/";
  if (parent.endsWith("/") || parent.endsWith("\\")) return parent + name;
  return parent + sep + name;
}

// Render a single node (dir or file) at the given depth.
// VS Code Material Icon Theme-inspired file-type tiles. Each entry
// picks a brand color and a 1-2 char monogram; the SVG paints a 16×16
// rounded square with the label centered. Approximates the VS Code
// feel without bundling the actual icon set (which is ~2k icons +
// licensed SVGs). We cover the file types common in AI/CLI projects
// and a long tail of common ones; unknown extensions fall back to a
// neutral "fi" tile.
const FILE_ICONS = {
  // exact-name entries (matched before extension)
  Dockerfile:    { color: "#384d54", label: "dk" },
  Makefile:      { color: "#427819", label: "mk" },
  ".gitignore":  { color: "#41535b", label: "gi" },
  ".env":        { color: "#ecd575", label: "ev" },
  LICENSE:       { color: "#c1c1c1", label: "©" },
  ".gitattributes":{color: "#41535b", label: "ga" },

  // popular web / TS
  js:    { color: "#f7df1e", label: "js" },
  mjs:   { color: "#f7df1e", label: "js" },
  cjs:   { color: "#f7df1e", label: "js" },
  ts:    { color: "#3178c6", label: "ts" },
  tsx:   { color: "#3178c6", label: "tx" },
  jsx:   { color: "#61dafb", label: "jx" },
  vue:   { color: "#41b883", label: "vu" },
  svelte:{ color: "#ff3e00", label: "sv" },

  // styling
  html:  { color: "#e44d26", label: "<>" },
  htm:   { color: "#e44d26", label: "<>" },
  css:   { color: "#264de4", label: "#" },
  scss:  { color: "#c69",    label: "s"  },
  sass:  { color: "#a53b70", label: "s"  },
  less:  { color: "#1d365d", label: "l"  },

  // data / config
  json:  { color: "#cbcb41", label: "{}" },
  jsonc: { color: "#cbcb41", label: "{}" },
  yaml:  { color: "#cb171e", label: "y"  },
  yml:   { color: "#cb171e", label: "y"  },
  toml:  { color: "#9c4221", label: "to" },
  xml:   { color: "#0060ac", label: "xm" },
  csv:   { color: "#3a7517", label: "cs" },
  sql:   { color: "#e38c00", label: "sq" },

  // systems / compiled
  rs:    { color: "#dea584", label: "rs" },
  go:    { color: "#00add8", label: "go" },
  c:     { color: "#555",    label: "c"  },
  h:     { color: "#555",    label: "h"  },
  cpp:   { color: "#00599c", label: "++" },
  cc:    { color: "#00599c", label: "++" },
  cxx:   { color: "#00599c", label: "++" },
  hpp:   { color: "#00599c", label: "h+" },
  hxx:   { color: "#00599c", label: "h+" },
  cs:    { color: "#178600", label: "cs" },
  swift: { color: "#f05138", label: "sw" },
  kt:    { color: "#a97bff", label: "kt" },
  kts:   { color: "#a97bff", label: "kt" },
  java:  { color: "#b07219", label: "jv" },
  dart:  { color: "#00b4ab", label: "da" },
  zig:   { color: "#ec915c", label: "zi" },
  rb:    { color: "#cc342d", label: "rb" },
  py:    { color: "#3776ab", label: "py" },
  pyi:   { color: "#3776ab", label: "py" },
  php:   { color: "#4f5d95", label: "ph" },
  scala: { color: "#c22d40", label: "sc" },
  lua:   { color: "#000080", label: "lu" },
  r:     { color: "#198ce7", label: "r"  },
  pl:    { color: "#0298c3", label: "pl" },
  ex:    { color: "#a074c4", label: "ex" },
  exs:   { color: "#a074c4", label: "ex" },
  erl:   { color: "#b83998", label: "er" },

  // shell / scripts
  sh:    { color: "#89e051", label: "sh" },
  bash:  { color: "#89e051", label: "sh" },
  zsh:   { color: "#89e051", label: "sh" },
  fish:  { color: "#89e051", label: "sh" },
  ps1:   { color: "#012456", label: "ps" },
  bat:   { color: "#c1f12e", label: "ba" },

  // docs / text
  md:      { color: "#42a5f5", label: "md" },
  markdown:{ color: "#42a5f5", label: "md" },
  mdx:     { color: "#42a5f5", label: "mx" },
  txt:     { color: "#7d7563", label: "tx" },
  rst:     { color: "#7d7563", label: "rs" },
  adoc:    { color: "#7d7563", label: "ad" },

  // images / media
  png:   { color: "#a074c4", label: "im" },
  jpg:   { color: "#a074c4", label: "im" },
  jpeg:  { color: "#a074c4", label: "im" },
  gif:   { color: "#a074c4", label: "im" },
  webp:  { color: "#a074c4", label: "im" },
  bmp:   { color: "#a074c4", label: "im" },
  ico:   { color: "#a074c4", label: "ic" },
  svg:   { color: "#ffb13b", label: "sv" },
  pdf:   { color: "#b30b00", label: "pd" },

  // archives
  zip:   { color: "#888", label: "zp" },
  gz:    { color: "#888", label: "zp" },
  tar:   { color: "#888", label: "zp" },
  bz2:   { color: "#888", label: "zp" },
  xz:    { color: "#888", label: "zp" },
  "7z":  { color: "#888", label: "zp" },
  rar:   { color: "#888", label: "zp" },

  // build / lock
  lock:  { color: "#888", label: "lk" },

  // binaries (warn-ish)
  exe:   { color: "#b0b0b0", label: "ex" },
  dll:   { color: "#b0b0b0", label: "dl" },
  so:    { color: "#b0b0b0", label: "so" },
  dylib: { color: "#b0b0b0", label: "dl" },
  class: { color: "#b0b0b0", label: "cl" },
  jar:   { color: "#b0b0b0", label: "jr" },
  wasm:  { color: "#b0b0b0", label: "wa" },
};
const DEFAULT_FILE_ICON = { color: "#7d7563", label: "fi" };

// Build the 16×16 SVG tile. We use SVG <text> for the monogram so
// the file type reads at a glance; the font stack is monospace so it
// renders consistently across Tauri WebView2 and the browser demo.
function tileIconSvg({ color, label }) {
  return `<svg class="tree__icon-svg" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
    <rect x="1.5" y="1.5" width="13" height="13" rx="1.5" fill="${color}"/>
    <path d="M9 1.5v3.2h3.2" fill="none" stroke="rgba(0,0,0,.28)" stroke-width=".5"/>
    <text x="8" y="11.2" text-anchor="middle" font-size="5.4" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-weight="700" fill="rgba(255,255,255,.94)">${escapeHtml(label)}</text>
  </svg>`;
}

// Folder glyph for the project list (left pane). Each project is a
// directory on disk, so showing a folder chip signals "this is a real
// path" at a glance. Monochrome (currentColor) so it follows the theme;
// tinted dimmer than the tool dot via .entry__folder in styles.css.
function folderIconSvg() {
  return iconSvg("folder", { size: 13 });
}

// Directory glyph for the file tree. Monochrome (currentColor), driven by
// .tree__icon-svg in styles.css. Replaces the old hardcoded tan (#dcb67a).
function dirIconSvg() {
  return iconSvg("folder", { size: 12, class: "tree__icon-svg" });
}

// Pick a file-type icon for `name` (a basename). Tries the collected
// brand logos first (exact name → extension), then falls back to the
// colored 2-letter monogram tile for types without a brand logo.
function fileIconSvg(name) {
  const raw = (name || "");
  const lower = raw.toLowerCase();
  // Strip any path components the caller may have passed.
  const base = lower.split(/[\\/]/).pop() || lower;
  // 1. Brand logo by exact basename (Dockerfile) — case-insensitive.
  const brandExact = FILETYPE_ICONS[base];
  if (brandExact) return brandFileIconSvg(brandExact);
  // 2. Brand logo by extension (rs/ts/py/js/...).
  const dot = base.lastIndexOf(".");
  if (dot > 0) {
    const ext = base.slice(dot + 1);
    const brandExt = FILETYPE_ICONS[ext];
    if (brandExt) return brandFileIconSvg(brandExt);
    // 3. Colored monogram tile by extension.
    const cfg = FILE_ICONS[ext];
    if (cfg) return tileIconSvg(cfg);
  }
  // 4. Colored monogram tile by exact name (e.g. .gitignore, Makefile).
  const exact = FILE_ICONS[base];
  if (exact) return tileIconSvg(exact);
  return tileIconSvg(DEFAULT_FILE_ICON);
}

// A file-type brand logo rendered as a monochrome (currentColor) glyph in
// the tree. Same 12px size / class as the colored letter-tiles so it sits
// flush in the file-tree row. Theme colour is driven by .tree__icon-svg
// in styles.css (light glyph on dark, dark on light).
function brandFileIconSvg(iconKey) {
  return iconSvg(iconKey, { size: 12, class: "tree__icon-svg tree__icon-svg--brand" });
}

function treeNodeHtml(entry, parentPath, depth) {
  const full = joinPath(parentPath, entry.name);
  if (entry.isDir) {
    return `<div class="tree__node tree__node--dir" data-tree-path="${escapeHtml(full)}" data-tree-depth="${depth}">
      <span class="tree__caret" data-tree-toggle>▸</span>
      <span class="tree__icon">${dirIconSvg()}</span>
      <span class="tree__name">${escapeHtml(entry.name)}</span>
    </div>
    <div class="tree__children" data-tree-children hidden></div>`;
  }
  return `<div class="tree__node tree__node--file" data-tree-path="${escapeHtml(full)}" data-tree-depth="${depth}">
    <span class="tree__caret tree__caret--none"></span>
    <span class="tree__icon">${fileIconSvg(entry.name)}</span>
    <span class="tree__name">${escapeHtml(entry.name)}</span>
  </div>`;
}

function treeEmptyHtml() {
  return `<div class="tree__empty">${escapeHtml(tr("files.emptyDir"))}</div>`;
}

// Populate a `<container>` with the project's root tree. Children
// load lazily on caret click. Reusable for both the entry-modal
// placeholder and the left-pane full-height tree.
async function loadFileTreeInto(container, project, gate) {
  if (!container) return;
  const requestId = gate.begin();
  container.innerHTML = `<span class="docs-status">${escapeHtml(tr("common.loading"))}</span>`;
  container._treeProject = project || null;
  container.dataset.treeRootIdentity = project ? `${project.id}\0${project.path}` : "";
  const identity = container.dataset.treeRootIdentity;
  treeStackFor(container).length = 0;
  updateBackBtn(container);
  if (!project) {
    container.innerHTML = `<span class="docs-status">${escapeHtml(tr("files.emptyProject"))}</span>`;
    updateBackBtn(container);
    return;
  }
  let entries;
  try {
    entries = await fetchDir(project.path);
  } catch (e) {
    console.warn("list_dir failed", e);
    if (gate.isCurrent(requestId) && container.isConnected
        && container.dataset.treeRootIdentity === identity) {
      container.innerHTML = `<span class="docs-status docs-status--err">${escapeHtml(tr("status.readFailed", { path: project.path }))}</span>`;
    }
    return;
  }
  if (!gate.isCurrent(requestId) || !container.isConnected
      || container.dataset.treeRootIdentity !== identity) return;
  if (!entries.length) {
    container.innerHTML = treeEmptyHtml();
    updateBackBtn(container);
    return;
  }
  // Root: just the immediate entries. The project name itself is the
  // path header shown by the modal already; tree starts at depth 0.
  container.innerHTML = entries.map(e => treeNodeHtml(e, project.path, 0)).join("");
  // Fresh tree → reset the back-navigation stack for this container.
  treeStackFor(container).length = 0;
  updateBackBtn(container);
  // One delegated click listener per container (avoids stacking on
  // re-renders). onTreeClick is the same handler the modal uses.
  if (!container.dataset.treeWired) {
    container.addEventListener("click", onTreeClick);
    container.dataset.treeWired = "1";
  }
}

// Per-container expand stack for the back button. A stash on the DOM
// node keeps state local without globals.
function treeStackFor(container) {
  if (!container._stack) container._stack = [];
  return container._stack;
}

// Find the closest ancestor that's a file-tree container (either the
// left pane or the modal placeholder). Used by onTreeClick so the
// stack stays attached to whichever tree the click happened in.
function treeContainerFrom(node) {
  return node.closest("[data-files-tree], .stage__left__files__tree");
}

// Collapse the most recently expanded directory in `container`.
// Returns true if anything was collapsed (so the caller can refresh
// the button's enabled state).
function treeGoBack(container) {
  if (!container) return false;
  const stack = treeStackFor(container);
  if (!stack.length) return false;
  const lastPath = stack.pop();
  // CSS.escape handles Windows backslashes / colons in paths.
  const sel = `.tree__node--dir[data-tree-path="${CSS.escape(lastPath)}"]`;
  const node = container.querySelector(sel);
  if (!node) return true; // stack popped even if node vanished (project switched)
  const caret = node.querySelector(".tree__caret");
  const childrenBox = node.nextElementSibling;
  if (caret) caret.textContent = "▸";
  if (childrenBox) childrenBox.hidden = true;
  return true;
}

function updateBackBtn(container) {
  const btn = document.getElementById("filesBackBtn");
  if (!btn) return;
  // The back button only exists in the left pane tree; for the modal's
  // tree the button simply stays disabled. In files view the button also
  // acts as the "back to ledger" affordance (the global deck-level tree
  // button moved into each project entry), so it stays enabled whenever
  // we're in files mode even if the expand stack is empty.
  const stack = container ? treeStackFor(container) : [];
  btn.disabled = stack.length === 0 && state.viewMode !== "files";
}

// Populate the entry modal's file-tree placeholder with the project's
// root. Children are loaded lazily on caret click.
async function loadFileTreeIntoEntryMenu(p) {
  const container = entryModalBody.querySelector("[data-files-tree]");
  await loadFileTreeInto(container, p, entryTreeGate);
}

// Toggle a directory node: load children on first expand, then
// collapse on second click. Subsequent re-expands are instant.
async function onTreeClick(e) {
  // File row → open in the doc modal (markdown for .md, preformatted
  // for everything else).
  const fileNode = e.target.closest(".tree__node--file");
  if (fileNode) {
    // Only respond to clicks on the file row's interactive parts.
    if (!e.target.closest(".tree__name, .tree__icon")) return;
    const container = treeContainerFrom(fileNode);
    const project = container?._treeProject;
    if (!project) return;
    const fullPath = fileNode.dataset.treePath;
    const rel = relFromProject(project.path, fullPath);
    const name = fullPath.split(/[\\/]/).pop() || rel;
    addFileTab(project, rel, name);
    return;
  }

  const dirNode = e.target.closest(".tree__node--dir");
  if (!dirNode) return;
  // Only respond to clicks on the dir row's interactive parts
  // (caret, name, icon) — not the surrounding whitespace.
  if (!e.target.closest(".tree__caret, .tree__name, .tree__icon")) return;
  const path = dirNode.dataset.treePath;
  const caret = dirNode.querySelector(".tree__caret");
  const childrenBox = dirNode.nextElementSibling; // <div data-tree-children>
  if (!childrenBox) return;
  const container = treeContainerFrom(dirNode);
  if (childrenBox.hidden) {
    // Expand
    caret.textContent = "▾";
    childrenBox.hidden = false;
    if (container) {
      const stack = treeStackFor(container);
      if (!stack.includes(path)) stack.push(path);
      updateBackBtn(container);
    }
    if (!childrenBox.dataset.loaded) {
      childrenBox.innerHTML = `<span class="docs-status">${escapeHtml(tr("common.loading"))}</span>`;
      if (!childrenBox._pendingPromise) {
        const rootIdentity = container?.dataset.treeRootIdentity;
        const token = Symbol(path);
        childrenBox._pendingToken = token;
        childrenBox._pendingPromise = fetchDir(path)
          .then(entries => {
            if (childrenBox._pendingToken !== token || !childrenBox.isConnected
                || container?.dataset.treeRootIdentity !== rootIdentity) return;
            childrenBox.innerHTML = entries.length
              ? entries.map(e => treeNodeHtml(e, path, Number(dirNode.dataset.treeDepth) + 1)).join("")
              : treeEmptyHtml();
            childrenBox.dataset.loaded = "1";
          })
          .catch(err => {
            console.warn("list_dir failed", err);
            if (childrenBox._pendingToken === token && childrenBox.isConnected
                && container?.dataset.treeRootIdentity === rootIdentity) {
              childrenBox.innerHTML = `<span class="docs-status docs-status--err">${escapeHtml(tr("status.readFailed", { path }))}</span>`;
            }
          })
          .finally(() => {
            if (childrenBox._pendingToken === token) {
              childrenBox._pendingPromise = null;
              childrenBox._pendingToken = null;
            }
          });
      }
      await childrenBox._pendingPromise;
    }
  } else {
    // Collapse
    caret.textContent = "▸";
    childrenBox.hidden = true;
    if (container) {
      // Manual collapse: drop this path from the stack so the back
      // button doesn't try to re-collapse an already-collapsed node.
      const stack = treeStackFor(container);
      const idx = stack.lastIndexOf(path);
      if (idx >= 0) stack.splice(idx, 1);
      updateBackBtn(container);
    }
  }
}

// Compute the path of `full` relative to `projectRoot`. Strips the
// leading separator so the backend command receives a clean relative
// path. Cross-platform (handles both `\` and `/`).
function relFromProject(projectRoot, full) {
  let rel = full;
  if (projectRoot && (full.startsWith(projectRoot + "\\") || full.startsWith(projectRoot + "/"))) {
    rel = full.slice(projectRoot.length + 1);
  } else if (projectRoot && full.startsWith(projectRoot)) {
    rel = full.slice(projectRoot.length).replace(/^[\\\/]+/, "");
  }
  return rel;
}

// Read a text file via the Rust command and open the doc modal with
// the contents. .md / .markdown get the markdown renderer; everything
// else gets a <pre>-block to avoid mangling code (otherwise `#`, `*`,
// `_` etc. in source code would render as headings / lists / em).

async function fetchTextFile(path, relPath) {
  if (!HAS_TAURI) return null;
  try { return await invoke("read_text_file", { path, relPath }); }
  catch (e) { console.warn("read_text_file failed", e); return null; }
}

// Map file extensions → highlight.js language IDs. The "common"
// vendor build covers ~35 languages; anything not in the map falls
// back to plaintext (no highlighting) but still gets line numbers.
const LANG_MAP = {
  rs: "rust", toml: "ini",
  js: "javascript", mjs: "javascript", cjs: "javascript",
  ts: "typescript", tsx: "typescript", jsx: "javascript",
  json: "json", jsonc: "json",
  yaml: "yaml", yml: "yaml",
  py: "python", pyi: "python",
  go: "go", rb: "ruby", php: "php",
  java: "java", kt: "kotlin", kts: "kotlin", swift: "swift",
  c: "c", h: "c",
  cpp: "cpp", cc: "cpp", cxx: "cpp", hpp: "cpp", hxx: "cpp",
  cs: "csharp", scala: "scala",
  sh: "bash", bash: "bash", zsh: "bash", fish: "bash",
  ps1: "powershell", bat: "dos",
  html: "xml", htm: "xml", xml: "xml", svg: "xml",
  css: "css", scss: "scss", sass: "scss", less: "less",
  sql: "sql",
  md: "markdown", markdown: "markdown", mdx: "markdown",
  lua: "lua", pl: "perl", r: "r",
  dart: "dart", zig: "zig",
  diff: "diff", patch: "diff",
  vim: "vim",
};

function renderFileContent(fileName, text) {
  const lower = (fileName || "").toLowerCase();
  // Markdown files use the existing renderer (no line numbers — markdown
  // rendering doesn't play well with per-line spans anyway).
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) {
    return renderMarkdown(text);
  }
  // Detect language by extension.
  const base = lower.split(/[\\/]/).pop() || lower;
  const dot = base.lastIndexOf(".");
  const ext = dot > 0 ? base.slice(dot + 1) : "";
  const lang = LANG_MAP[ext] || "";
  // Tokenize with highlight.js. Falls back to escaped text if the
  // language isn't recognised or hljs isn't loaded.
  let highlighted;
  if (lang && typeof hljs !== "undefined" && hljs.getLanguage(lang)) {
    try { highlighted = hljs.highlight(text, { language: lang, ignoreIllegals: true }).value; }
    catch (e) { highlighted = escapeHtml(text); }
  } else {
    highlighted = escapeHtml(text);
  }
  // Line numbers — count `\n` + 1, render one <span> per line in a
  // fixed-width gutter. Each line uses <br> separators in the source
  // gutter so it visually aligns with the pre's line breaks.
  const lineCount = (text.match(/\n/g) || []).length + 1;
  let gutter = "";
  for (let i = 1; i <= lineCount; i++) gutter += `<span>${i}</span>${i < lineCount ? "\n" : ""}`;
  return `<div class="code-view">
    <div class="code-gutter" aria-hidden="true">${gutter}</div>
    <pre class="code-pre"><code class="hljs language-${lang || "plaintext"}">${highlighted}</code></pre>
  </div>`;
}

/* ── file-tab right-click context menu ───────────────────── */
// Custom context menu shown when the user right-clicks inside a file
// tab's body. The first item — "Send to command line" — writes a
// `@<rel-path>:<line_range>` reference to the active pty tab (or any
// fallback pty tab) and submits it, so the agent picks it up as a
// user message. Other items: copy file path / absolute path.

let _ctxMenu = null;
let _ctxEscHandler = null;

function closeContextMenu() {
  if (_ctxMenu) { _ctxMenu.remove(); _ctxMenu = null; }
  if (_ctxEscHandler) {
    document.removeEventListener("keydown", _ctxEscHandler, { capture: true });
    _ctxEscHandler = null;
  }
}

// Toggle the inline session panel for a project entry. Only one entry can
// be expanded at a time — opening a different one collapses the previous.
function toggleEntryExpand(projectId) {
  // A disclosure click can follow a tiny pointer movement. If the row's
  // native drag gesture was armed first, renderVirtualLedger() deliberately
  // refuses to replace rows and the expansion appears to do nothing. Clear
  // any transient drag state before changing the disclosure state.
  state._dragId = null;
  ledger.querySelectorAll(".is-dragging")
    .forEach(element => element.classList.remove("is-dragging"));
  clearDropIndicators();
  const previousId = state.expandedId;
  state.expandedId = state.expandedId === projectId ? null : projectId;
  if (previousId) state.ledgerHeightByKey.delete(`project:${previousId}`);
  state.ledgerHeightByKey.delete(`project:${projectId}`);
  state.ledgerLayout = null;
  applyFilters();
  // applyFilters() recycles the virtual row. Restore focus to the replacement
  // control so keyboard users can immediately collapse it again with Space
  // or Enter and mouse users retain a clear visual state.
  requestAnimationFrame(() => {
    const selector = `.entry[data-id="${CSS.escape(String(projectId))}"] [data-expand-toggle]`;
    ledger.querySelector(selector)?.focus({ preventScroll: true });
  });
}

// Compute the line range of the user's current selection inside the
// file's <code> element. Returns null if there's no selection, or the
// selection is outside this file's code, or we can't determine it.
function getCodeSelectionInfo(tab) {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
  const range = sel.getRangeAt(0);
  const code = tab.body.querySelector("code");
  if (!code) return null;
  if (!code.contains(range.startContainer) || !code.contains(range.endContainer)) return null;
  const startAbs = offsetWithinNode(code, range.startContainer, range.startOffset);
  const endAbs = offsetWithinNode(code, range.endContainer, range.endOffset);
  if (startAbs < 0 || endAbs < 0) return null;
  // The code element's textContent is the (highlighted) source — the
  // positions correspond directly to line numbers in the original file.
  const fullText = code.textContent || "";
  const startLine = (fullText.slice(0, startAbs).match(/\n/g) || []).length + 1;
  const endLine   = (fullText.slice(0, endAbs  ).match(/\n/g) || []).length + 1;
  return { relPath: tab.relPath, startLine, endLine };
}

// Walk text descendants of `root` and return the absolute offset of
// `node` (which must be a text node inside root) plus offsetInNode.
function offsetWithinNode(root, node, offsetInNode) {
  if (!root.contains(node)) return -1;
  let abs = 0;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let cur = walker.nextNode();
  while (cur) {
    if (cur === node) return abs + offsetInNode;
    abs += cur.textContent.length;
    cur = walker.nextNode();
  }
  return -1;
}

// Pick the pty tab that should receive the "@ref" message. Prefer the
// currently active tab (which is almost always a pty tab when the
// user is working in a terminal-driven flow); fall back to the most
// recently opened pty tab.
function findActivePtyTab() {
  const active = state.tabs.find(t => t.tabId === state.activeTabId);
  if (active && active.kind === "pty" && !active.dead) return active;
  const allPty = state.tabs.filter(t => t.kind === "pty" && !t.dead);
  if (!allPty.length) return null;
  // ptyId is an incrementing AtomicU32, so the highest id is the newest.
  return allPty.sort((a, b) => b.ptyId - a.ptyId)[0];
}

function sendSelectionToCommandLine(info) {
  const ptyTab = findActivePtyTab();
  if (!ptyTab) {
    setStatus(tr("status.noActivePty"));
    return;
  }
  const range = info.startLine === info.endLine
    ? `${info.startLine}`
    : `${info.startLine}-${info.endLine}`;
  const ref = `@${info.relPath}#${range}`;
  // Write the reference into the terminal prompt without submitting —
  // the user types their question after and presses Enter themselves.
  // (The previous version appended \r which auto-submitted; not what
  // users want when they're using @ref as a context prefix.)
  invoke("pty_write", { id: ptyTab.ptyId, data: ref }).catch(e => setStatus(tr("status.writeFailed", { err: e })));
  // Focus + show the destination tab so the user sees the action land.
  try { ptyTab.term.focus(); } catch {}
  switchTab(ptyTab.tabId);
  setStatus(tr("status.pasted", { ref, tool: ptyTab.usage?.toolKey || "shell" }));
}

function onFileContextMenu(e, tab) {
  // Always suppress the native menu inside a file-pane body — we
  // provide our own.
  e.preventDefault();
  closeContextMenu();

  const info = getCodeSelectionInfo(tab);
  const hasPty = !!findActivePtyTab();
  const refPreview = info
    ? (info.startLine === info.endLine
        ? `${info.relPath}#${info.startLine}`
        : `${info.relPath}#${info.startLine}-${info.endLine}`)
    : "";

  const menu = document.createElement("div");
  menu.className = "ctx-menu";

  // Item: send @ref to the active command line
  const sendItem = document.createElement("button");
  sendItem.className = "ctx-menu__item ctx-menu__item--primary";
  sendItem.disabled = !info || !hasPty;
  const refLabel = info
    ? `@${refPreview}`
    : (hasPty ? tr("ctx.selectLinesFirst") : tr("ctx.noPtyTab"));
  sendItem.innerHTML = `
    <span class="ctx-menu__label">${escapeHtml(tr("ctx.sendToCommandLine"))}</span>
    <span class="ctx-menu__hint">${escapeHtml(refLabel)}</span>`;
  if (!sendItem.disabled) {
    sendItem.addEventListener("click", () => {
      closeContextMenu();
      sendSelectionToCommandLine(info);
    });
  }
  menu.appendChild(sendItem);

  // Item: copy relative file path
  const copyRel = document.createElement("button");
  copyRel.className = "ctx-menu__item";
  copyRel.innerHTML = `
    <span class="ctx-menu__label">${escapeHtml(tr("ctx.copyFilePath"))}</span>
    <span class="ctx-menu__hint">${escapeHtml(tab.relPath)}</span>`;
  copyRel.addEventListener("click", () => {
    closeContextMenu();
    try {
      navigator.clipboard.writeText(tab.relPath);
      setStatus(tr("status.copied", { path: tab.relPath }));
    } catch (e) { setStatus(tr("status.copyFailed", { err: e })); }
  });
  menu.appendChild(copyRel);

  // Item: copy absolute file path
  const copyAbs = document.createElement("button");
  copyAbs.className = "ctx-menu__item";
  copyAbs.innerHTML = `<span class="ctx-menu__label">${escapeHtml(tr("ctx.copyAbsPath"))}</span>`;
  copyAbs.addEventListener("click", () => {
    closeContextMenu();
    try {
      navigator.clipboard.writeText(tab.filePath);
      setStatus(tr("status.copied", { path: tab.filePath }));
    } catch (e) { setStatus(tr("status.copyFailed", { err: e })); }
  });
  menu.appendChild(copyAbs);

  // Position the menu at the cursor, keeping it inside the viewport.
  document.body.appendChild(menu);
  const rect = menu.getBoundingClientRect();
  const vw = window.innerWidth, vh = window.innerHeight;
  const x = Math.min(e.clientX, vw - rect.width  - 4);
  const y = Math.min(e.clientY, vh - rect.height - 4);
  menu.style.left = `${Math.max(4, x)}px`;
  menu.style.top  = `${Math.max(4, y)}px`;
  _ctxMenu = menu;
  // Close on next click anywhere (capture so it beats other click handlers).
  setTimeout(() => document.addEventListener("click", closeContextMenu, { once: true, capture: true }), 0);
  // Close on Esc (capture-phase so it fires before any modal Esc handlers).
  _ctxEscHandler = (e) => { if (e.key === "Escape") { e.stopPropagation(); closeContextMenu(); } };
  document.addEventListener("keydown", _ctxEscHandler, { capture: true });
}

// ── minimal markdown renderer ─────────────────────────────
// Handles: fenced code blocks, ATX headings (#–######), bold/italic,
// inline code, links, unordered + ordered lists, blockquotes, paragraphs.
// HTML in the source is escaped; markdown tokens are then expanded.
function renderMarkdown(md) {
  const lines = md.replace(/\r\n?/g, "\n").split("\n");
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block — ```lang ... ```
    if (/^```/.test(line)) {
      const code = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) {
        code.push(lines[i]);
        i++;
      }
      if (i < lines.length) i++; // skip closing fence
      out.push(`<pre class="md-pre"><code class="md-code">${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }

    // Heading — # to ######
    const h = line.match(/^(#{1,6})\s+(.*)$/);
    if (h) {
      const lvl = h[1].length;
      out.push(`<h${lvl} class="md-h md-h${lvl}">${renderInline(h[2])}</h${lvl}>`);
      i++;
      continue;
    }

    // Blockquote — one or more consecutive `> ` lines
    if (/^>\s?/.test(line)) {
      const buf = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        buf.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      out.push(`<blockquote class="md-q">${renderInline(buf.join(" "))}</blockquote>`);
      continue;
    }

    // Unordered list — `- ` or `* `
    if (/^[-*]\s+/.test(line)) {
      const items = [];
      while (i < lines.length && /^[-*]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^[-*]\s+/, ""));
        i++;
      }
      out.push(`<ul class="md-ul">${items.map(it => `<li>${renderInline(it)}</li>`).join("")}</ul>`);
      continue;
    }

    // Ordered list — `1. `, `2. `, ...
    if (/^\d+\.\s+/.test(line)) {
      const items = [];
      while (i < lines.length && /^\d+\.\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\d+\.\s+/, ""));
        i++;
      }
      out.push(`<ol class="md-ol">${items.map(it => `<li>${renderInline(it)}</li>`).join("")}</ol>`);
      continue;
    }

    // Blank line — paragraph break
    if (line.trim() === "") { i++; continue; }

    // Paragraph — collect until blank line or block-level start
    const para = [];
    while (i < lines.length && lines[i].trim() !== "" &&
           !/^(#{1,6}\s|```|>\s?|[-*]\s|\d+\.\s)/.test(lines[i])) {
      para.push(lines[i]);
      i++;
    }
    out.push(`<p class="md-p">${renderInline(para.join(" "))}</p>`);
  }
  return out.join("");
}

// Inline tokens: escape first, then expand images, `code`, **bold**, *italic,
// and links. Images are represented as compact labelled chips rather than
// fetched inside the app: this avoids leaking document-view activity to a
// third-party image host and keeps the strict `img-src 'self'` CSP intact.
// Only HTTP(S) targets become clickable because those are exactly the schemes
// supported by `open_external_url`; relative/mail links remain readable text
// instead of navigating the Tauri webview away from the application.
function renderInline(text) {
  let s = escapeHtml(text);
  const decodeUrl = rawUrl => rawUrl
    .replace(/&amp;/g, "&")
    .replace(/&#39;/g, "'")
    .replace(/&quot;/g, '"');
  const imageToken = (label, rawUrl, rawTarget = null) => {
    const imageUrl = decodeUrl(rawUrl);
    const targetUrl = decodeUrl(rawTarget || rawUrl);
    const safeLabel = label || escapeHtml(imageUrl);
    const body = `<span class="md-image-mark" aria-hidden="true">▧</span><span>${safeLabel}</span>`;
    if (/^https?:\/\//i.test(targetUrl) && isSafeUrl(targetUrl)) {
      return `<a href="${escapeHtml(targetUrl)}" class="md-link md-image-link" target="_blank" rel="noopener noreferrer" title="${escapeHtml(imageUrl)}">${body}</a>`;
    }
    return `<span class="md-image-placeholder" title="${escapeHtml(imageUrl)}">${body}</span>`;
  };
  // Linked image badges must be handled before standalone images or links.
  s = s.replace(/\[!\[([^\]]*)\]\(([^)\s]+)\)\]\(([^)\s]+)\)/g,
    (_, label, imageUrl, targetUrl) => imageToken(label, imageUrl, targetUrl));
  s = s.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g,
    (_, label, imageUrl) => imageToken(label, imageUrl));
  // inline code
  s = s.replace(/`([^`]+)`/g, (_, c) => `<code class="md-inline-code">${c}</code>`);
  // bold (** before italic so ** isn't eaten)
  s = s.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  // italic
  s = s.replace(/(^|[^\*])\*([^*\n]+)\*(?!\*)/g, "$1<em>$2</em>");
  // links — whitelist the scheme. The URL was captured AFTER escaping, so
  // reverse the few entities we produced before validating the scheme.
  s = s.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (m, label, rawUrl) => {
    const url = decodeUrl(rawUrl);
    if (!isSafeUrl(url)) return escapeHtml(label || url);
    const safeLabel = label || escapeHtml(url);
    if (!/^https?:\/\//i.test(url)) {
      return `<span class="md-link-muted" title="${escapeHtml(url)}">${safeLabel}</span>`;
    }
    return `<a href="${escapeHtml(url)}" class="md-link" target="_blank" rel="noopener noreferrer">${safeLabel}</a>`;
  });
  return s;
}

// Allow http(s) and mailto only. Anything else (javascript:, data:, file:,
// vbscript:, …) is dropped so untrusted markdown can't become a clickable
// dangerous link. Relative URLs (no scheme) are also allowed.
function isSafeUrl(url) {
  const u = String(url || "").trim();
  if (u === "") return false;
  // No scheme → relative / protocol-relative / anchor — treat as safe.
  if (!/^[a-z][a-z0-9+.\-]*:/i.test(u)) return true;
  return /^(https?|mailto):/i.test(u);
}

/* ── drag-to-reorder & drag-to-regroup ──────────────────── */
// Drag an entry between rows of a group → reorder (locks that group to
// manual order). Drag an entry onto a group header → move it into that
// group (appends if target is manual, else recency; doesn't lock). Both
// paths persist via prefs.db so order survives rescans.

// Current rendered order (matching + non-collapsed) of a group, sorted.
// Used to compute the new ordered id list on a positional drop.
function renderedGroupOrder(groupKey) {
  const cutoff = RECENCY_CUTOFFS[state.recency];
  const now = Date.now();
  const items = state.all.filter(p =>
    matchesFilters(p, cutoff, now) && groupKeyOf(p) === groupKey);
  sortBucket(items);
  return items.map(p => p.id);
}

// Persist a new manual order for `groupKey` (also updates assignments for a
// cross-group positional move). Optimistic: mutate sortOrders + assignments
// and re-render immediately, then reconcile from the server on success.
async function setGroupOrder(groupKey, orderedIds) {
  const prevOrders = { ...state.sortOrders };
  const prevAssignments = { ...state.assignments };
  // Optimistic: assign 10/20/30… so the bucket re-sorts to the new order.
  orderedIds.forEach((id, i) => { state.sortOrders[id] = (i + 1) * 10; });
  if (groupKey === "ungrouped") {
    orderedIds.forEach(id => delete state.assignments[id]);
  } else {
    const gid = Number(groupKey);
    orderedIds.forEach(id => { state.assignments[id] = gid; });
  }
  applyFilters();
  if (!HAS_TAURI) return;
  try {
    await invoke("set_group_order", { groupKey, orderedIds });
    // Reconcile memberCounts + authoritative sort/assignment state.
    await loadGroups();
  } catch (e) {
    showActionError(tr("status.sortFailed", { err: e }));
    state.sortOrders = prevOrders;
    state.assignments = prevAssignments;
    await reload();
  }
}

function handleDropOnEntry(dragId, targetId, before) {
  const target = state.all.find(p => p.id === targetId);
  if (!target || dragId === targetId) return;
  const groupKey = groupKeyOf(target);
  const group = groupKey === "ungrouped"
    ? null
    : state.groups.find(item => String(item.id) === groupKey);
  if (!canSubmitCompleteGroupOrder(
    "",
    state.catalog.length,
    LIST_LIMIT,
    group?.memberCount,
  )) {
    showActionError(tr("status.sortNeedsFullList"));
    return;
  }
  if (!HAS_TAURI) {
    const order = renderedGroupOrder(groupKey).filter(id => id !== dragId);
    let idx = order.indexOf(targetId);
    if (idx < 0) idx = order.length;
    else if (!before) idx += 1;
    order.splice(idx, 0, dragId);
    setGroupOrder(groupKey, order);
    return;
  }
  moveGroupProject(dragId, groupKey, targetId, before ? "before" : "after");
}

async function moveGroupProject(projectId, targetGroupKey, anchorProjectId, placement) {
  return groupMutationQueue.run("groups", async () => {
    try {
      const result = await invoke("move_group_project", {
        projectId,
        targetGroupKey,
        anchorProjectId,
        placement,
        catalogIds: state.catalog.map(project => project.id),
        expectedRevision: state.groupRevision,
      });
      state.groupRevision = result.revision;
      await loadGroups();
    } catch (error) {
      showActionError(tr("status.sortFailed", { err: error }));
      await loadGroups();
    }
  });
}

function handleDropOnHeader(dragId, key) {
  if (!state.all.some(p => p.id === dragId)) return;
  const gid = key === "ungrouped" ? null : Number(key);
  if ((state.assignments[dragId] ?? null) === (gid ?? null)) return;
  // Reuse the existing assign path — the backend reconciles sort rows.
  setProjectGroup(dragId, gid);
}

function clearDropIndicators() {
  ledger.querySelectorAll(".drop-before,.drop-after")
    .forEach(e => e.classList.remove("drop-before", "drop-after"));
  ledger.querySelectorAll(".ledger__group.is-drop-target")
    .forEach(e => e.classList.remove("is-drop-target"));
}

// Single delegated listener set on the ledger (entries are re-rendered via
// innerHTML, so per-element binding would leak). dragover computes the
// insertion point from the pointer Y relative to the hovered row's midpoint.
function wireDrag() {
  ledger.addEventListener("dragstart", (e) => {
    // Rows are draggable for manual ordering, but their controls must stay
    // normal controls. WebView2 can otherwise promote a slightly imprecise
    // click on the disclosure chevron into a row drag and suppress `click`.
    if (e.target.closest("button, a, input, textarea, select, [contenteditable='true'], [data-queue-panel]")) {
      e.preventDefault();
      state._dragId = null;
      clearDropIndicators();
      return;
    }
    const entry = e.target.closest(".entry");
    if (!entry) return;
    state._dragId = entry.dataset.id;
    entry.classList.add("is-dragging");
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", state._dragId);
    }
  });
  ledger.addEventListener("dragover", (e) => {
    if (!state._dragId) return;
    const entry = e.target.closest(".entry");
    const header = e.target.closest(".ledger__group");
    clearDropIndicators();
    if (entry) {
      e.preventDefault();
      const r = entry.getBoundingClientRect();
      entry.classList.add((e.clientY - r.top) < r.height / 2 ? "drop-before" : "drop-after");
    } else if (header) {
      e.preventDefault();
      header.classList.add("is-drop-target");
    }
  });
  ledger.addEventListener("drop", (e) => {
    if (!state._dragId) return;
    e.preventDefault();
    const dragId = state._dragId;
    const entry = e.target.closest(".entry");
    const header = e.target.closest(".ledger__group");
    clearDropIndicators();
    if (entry) {
      const r = entry.getBoundingClientRect();
      handleDropOnEntry(dragId, entry.dataset.id, (e.clientY - r.top) < r.height / 2);
    } else if (header) {
      handleDropOnHeader(dragId, header.dataset.groupKey);
    }
  });
  ledger.addEventListener("dragend", () => {
    state._dragId = null;
    ledger.querySelectorAll(".is-dragging")
      .forEach(e => e.classList.remove("is-dragging"));
    clearDropIndicators();
    renderVirtualLedger();
  });
}

/* ── terminal tabs ──────────────────────────────────────── */
// Open the "default" tool for a project — first prefer a real resumable main
// session, then fall back to the most-recent activity-only tool.
// If a tab for the same project+tool is already open, just focus it
// instead of creating a duplicate. Projects with no recorded usage
// fall back to a plain shell.
function openProjectDefault(project) {
  if (!project) return;
  const topUsage = preferredProjectUsage(project);
  const toolKey = topUsage?.toolKey || "shell";
  const existing = findOpenTerminalTab(state.tabs, project.id, toolKey);
  if (existing) {
    switchTab(existing.tabId);
    return;
  }
  // Fallback usage record (shape only — openTerminalTab only uses
  // toolKey/toolName to build the auto-launch command).
  const usage = topUsage || { toolKey: "shell", toolName: "shell" };
  openTerminalTab(project, usage);
}

function openTerminalTab(project, usage) {
  const toolKey = usage?.toolKey || "shell";
  const existing = findOpenTerminalTab(state.tabs, project?.id, toolKey);
  if (existing) {
    switchTab(existing.tabId);
    return Promise.resolve(existing);
  }

  const key = terminalSessionKey(project?.id, toolKey);
  return coalescePending(
    state.openingPtys,
    key,
    () => {
      const openOrSwitch = () => {
        const current = findOpenTerminalTab(state.tabs, project?.id, toolKey);
        if (current) {
          switchTab(current.tabId);
          return current;
        }
        if (project?.source === "remote") {
          const reusable = findReusableRemoteTerminalTab(
            state.tabs,
            project.remoteServerId,
          );
          if (reusable) return switchRemoteTerminalTab(reusable, project, usage);
        }
        return openTerminalTabOnce(project, usage);
      };
      if (project?.source !== "remote") return openOrSwitch();
      return remoteTerminalQueue.run(
        `server:${String(project.remoteServerId ?? "")}`,
        openOrSwitch,
      );
    },
  );
}

async function switchRemoteTerminalTab(tab, project, usage) {
  const toolKey = usage?.toolKey || "shell";
  const isShell = toolKey === "shell";
  const title = isShell ? project.name : `${project.name} · ${toolKey}`;
  const serverLabel = state.remoteServerById[project.remoteServerId]?.label
    || project.remoteServerId;
  setStatus(tr("status.switchingRemoteSession", {
    server: serverLabel,
    title,
  }));
  tab.switching = true;
  switchTab(tab.tabId);
  try {
    await invoke(
      "pty_remote_switch",
      buildPtyRemoteSwitchRequest(tab.ptyId, project, usage),
    );
  } catch (e) {
    setStatus(tr("status.remoteSwitchFailed", { err: e }));
    showActionError(tr("status.remoteSwitchFailed", { err: e }));
    return tab;
  } finally {
    tab.switching = false;
  }
  if (tab.dead || !state.tabs.includes(tab)) return tab;

  tab.project = project;
  tab.usage = usage;
  tab.title = title;
  refreshTabButton(tab);
  switchTab(tab.tabId);
  recordTerminalActivity(project, usage);
  setStatus(tr("status.remoteSessionSwitched", {
    server: serverLabel,
    title,
  }));
  return tab;
}

async function openTerminalTabOnce(project, usage) {
  const toolKey = usage?.toolKey || "shell";
  if (!canLaunchTui(project, toolKey)) {
    const capability = tuiCapabilityForProject(project, toolKey);
    const message = capability && !capability.installed
      ? tr("tui.launchMissing")
      : capability && !capability.enabled
        ? tr("tui.launchDisabled")
        : tr("tui.launchChecking");
    showActionError(message);
    return;
  }
  const isShell = toolKey === "shell";
  const title = isShell ? project.name : `${project.name} · ${toolKey}`;
  setStatus(tr("status.openingTool", { tool: toolKey, name: project.name }));

  if (HAS_TAURI && !state.ptyEventsReady) {
    const message = tr("term.eventsFailed");
    showActionError(message);
    return;
  }

  // Browser demo mode: no real terminal backend.
  if (!HAS_TAURI || !HAS_TERM) {
    const note = HAS_TERM
      ? tr("status.demoWouldOpenTerm")
      : tr("status.termLoadFailed");
    termsEmpty.style.display = "flex";
    termsEmpty.innerHTML = `
      <div class="terms__empty-glyph">⌘</div>
      <div class="terms__empty-title">${escapeHtml(title)}</div>
      <div class="terms__empty-body">${escapeHtml(note)}</div>`;
    setStatus(note);
    return;
  }

  setStatus(tr("status.openingTool", { tool: toolKey, name: project.name }));

  // Create the pane + xterm first so we know cols/rows for the PTY.
  const pane = document.createElement("div");
  pane.className = "term-pane";
  termsViewport.appendChild(pane);

  const term = new window.Terminal({
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 13,
    cursorBlink: true,
    scrollback: 5000,
    theme: {
      background: "#0b0f13", foreground: "#dbe2e7", cursor: "#c9f65b",
      selectionBackground: "#c9f65b", selectionForeground: "#0b0f13",
      black:"#0b0f13", red:"#ff7657", green:"#10a37f", yellow:"#e8b339",
      blue:"#6c8aff", magenta:"#c9f65b", cyan:"#5ec8d8", white:"#dbe2e7",
      brightBlack:"#7f8c99", brightRed:"#ff7657", brightGreen:"#10a37f",
      brightYellow:"#e8b339", brightBlue:"#6c8aff", brightMagenta:"#c9f65b",
      brightCyan:"#5ec8d8", brightWhite:"#ffffff",
    },
  });
  const fit = new window.FitAddon.FitAddon();
  term.loadAddon(fit);
  const webLinks = new window.WebLinksAddon.WebLinksAddon((event, uri) => {
    activateTerminalHttpLink(event, uri, openWebTab);
  });
  term.loadAddon(webLinks);
  term.open(pane);
  fit.fit();

  const isRemote = project.source === "remote";
  let ptyId;
  try {
    const server = isRemote
      ? state.remoteServerById[project.remoteServerId]
      : null;
    // Remote tools are launched only when a new deterministic tmux session is
    // created. Reconnects attach without typing a duplicate command into it.
    // Local launches still happen in pty_attach.
    ptyId = await invoke(
      "pty_spawn",
      buildPtySpawnRequest(project, usage, server, term.cols, term.rows),
    );
  } catch (e) {
    try { term.dispose(); } catch {}
    pane.remove();
    renderSelectedLaunchPanel();
    termsEmpty.style.display = "flex";
    termsEmpty.innerHTML = `
      <div class="terms__empty-glyph">!</div>
      <div class="terms__empty-title">${escapeHtml(title)}</div>
      <div class="terms__empty-body">${escapeHtml(tr("term.startFailed", { err: e }))}</div>`;
    setStatus(tr("status.sessionFailed", { err: e }));
    return;
  }

  // tabId is the unified tab identifier (terminal tabs reuse ptyId;
  // file tabs get a generated string). kind discriminates how the
  // tab is rendered and closed.
  const tab = {
    tabId: ptyId, kind: "pty", ptyId, title, term, fit, pane,
    project, usage, dead: false, switching: false, composing: false,
  };
  tab.disposeImeStability = wireTerminalImeStability(tab);
  state.tabs.push(tab);

  // Forward keystrokes to the PTY.
  term.onData((data) => {
    if (tab.dead || tab.switching) return;
    invoke("pty_write", { id: ptyId, data }).catch(() => {});
  });

  addTabButton(tab);
  switchTab(ptyId);
  renderTabCount();

  // The Rust reader starts only after this call. At this point the tab and
  // the global event listener both exist, so the first prompt/output cannot
  // be dropped. `initialInput` is written once before the reader starts.
  try {
    await invoke(
      "pty_attach",
      buildPtyAttachRequest(ptyId, toolKey, usage?.lastSessionId, isRemote),
    );
  } catch (e) {
    // Closing the tab while attach was in flight is an intentional cancel,
    // not a startup failure that should overwrite the current UI status.
    if (!state.tabs.includes(tab)) return;
    tab.dead = true;
    try {
      term.write(`\r\n\x1b[31m${tr("term.startFailed", { err: e })}\x1b[0m\r\n`);
    } catch {}
    const btn = termsBar.querySelector(
      `.term-tab[data-tab-id="${CSS.escape(String(ptyId))}"]`,
    );
    btn?.classList.add("is-dead");
    invoke("pty_kill", { id: ptyId }).catch(() => {});
    setStatus(tr("status.sessionFailed", { err: e }));
    return tab;
  }
  if (tab.dead || !state.tabs.includes(tab)) return tab;

  recordTerminalActivity(project, usage);
  setStatus(tr("status.sessionOpen", { title }));
  return tab;
}

function recordTerminalActivity(project, usage) {
  const toolKey = usage?.toolKey || "shell";
  // Optimistically mark the project as just-touched so its entry__time ticks
  // to "now" immediately. Switching a reused remote connection must also move
  // the green active-session marker from the previous project to this one.
  project.lastAccessedAt = new Date().toISOString();
  renderLedger();
  // For remote sessions, eagerly record the tool-usage row so the
  // session count / last-session-id surfaces in the next
  // list_remote_projects pull. The local equivalent is implicit (the
  // sessionatlas CLI notices the touched dir and writes to index.db), but
  // for remote we own the writer.
  if (project.source === "remote" && toolKey !== "shell") {
    invoke("record_remote_tool_usage", {
      serverId: project.remoteServerId,
      projectId: project.id,
      toolKey,
      toolName: usage?.toolName || toolKey,
      sessionId: usage?.lastSessionId || null,
    }).catch(() => {});
  }
}

// Open a file as a new tab in the right pane (instead of a modal). If
// the file is already open as a tab, focus the existing tab — no
// duplicate. The pane is a `.file-pane` sibling of `.term-pane` inside
// `termsViewport`; it gets `is-active` on switch just like terminals.
async function addFileTab(project, relPath, name) {
  const fullPath = joinPath(project.path, relPath);
  // Dedup: focus existing tab if the same absolute path is already open.
  const existing = state.tabs.find(t => t.kind === "file" && t.filePath === fullPath);
  if (existing) {
    switchTab(existing.tabId);
    return;
  }
  // Generate a unique tabId — terminal tabs use their numeric ptyId;
  // file tabs need a stable string. Date + short random is plenty
  // (no risk of collision within a single browser tab).
  const tabId = `f${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

  // Pane element: absolute-positioned overlay inside termsViewport, like
  // .term-pane. Body holds the rendered content.
  const pane = document.createElement("div");
  pane.className = "file-pane";
  pane.dataset.tabId = tabId;
  const head = document.createElement("div");
  head.className = "file-pane__head";
  head.innerHTML = `<span class="file-pane__path">${escapeHtml(relPath)}</span>`;
  const body = document.createElement("div");
  body.className = "file-pane__body";
  body.innerHTML = `<div class="docs-status">${escapeHtml(tr("common.loading"))}</div>`;
  pane.appendChild(head);
  pane.appendChild(body);
  termsViewport.appendChild(pane);

  const tab = {
    tabId, kind: "file",
    filePath: fullPath, relPath, name,
    title: name,
    project,
    pane, body,
  };
  state.tabs.push(tab);
  addTabButton(tab);
  switchTab(tabId);
  renderTabCount();
  setStatus(tr("status.openedName", { name }));

  // Load content. .md → markdown; everything else → preformatted.
  const text = await fetchTextFile(project.path, relPath);
  if (text == null) {
    body.innerHTML = `<div class="docs-status docs-status--err">${tr("status.readFailed", { path: escapeHtml(relPath) })}</div>`;
    return;
  }
  // Only render into the body if this tab is still around (user might
  // have closed it before the read finished).
  if (state.tabs.some(t => t.tabId === tabId)) {
    body.innerHTML = renderFileContent(name, text);
    // Custom right-click menu (send to command line, copy path, etc.)
    // attached after the body is populated so the listener catches the
    // <code> selection.
    body.addEventListener("contextmenu", (e) => onFileContextMenu(e, tab));
  }
}

function normalizeWebConnectionUrl(value) {
  const candidate = String(value ?? "").trim().replace(/[.,;:!?)\]}>]+$/, "");
  if (!candidate || /[\u0000-\u001f\u007f]/.test(candidate)) return null;
  try {
    const parsed = new URL(candidate);
    if (!['http:', 'https:'].includes(parsed.protocol) || !parsed.hostname) return null;
    if (parsed.username || parsed.password) return null;
    return parsed.href;
  } catch {
    return null;
  }
}

// Open a URL as an in-app browser tab. Mirrors addFileTab's structure:
// a `.web-pane` overlay in termsViewport with an iframe. Dedupes by URL
// so clicking the same link twice focuses the existing tab.
function openWebTab(url, titleOverride = "") {
  const cleaned = normalizeWebConnectionUrl(url);
  if (!cleaned) {
    showActionError(tr("status.webConnectionInvalid"));
    return null;
  }
  // Dedup
  const existing = state.tabs.find(t => t.kind === "web" && t.url === cleaned);
  if (existing) { switchTab(existing.tabId); return existing; }
  const tabId = `w${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const configuredTitle = String(titleOverride ?? "").trim();
  let title = configuredTitle;
  if (!title) {
    try { title = new URL(cleaned).hostname || cleaned; }
    catch { title = cleaned.slice(0, 40); }
  }

  const pane = document.createElement("div");
  pane.className = "web-pane";
  pane.dataset.tabId = tabId;

  const head = document.createElement("div");
  head.className = "web-pane__head";
  // Plain-text title + URL. The URL is a link so right-click → open in
  // browser still works when the iframe fails to load (X-Frame-Options
  // blocks, network down, etc.).
  head.innerHTML = `
    <span class="web-pane__icon">${escapeHtml(tr("web.icon"))}</span>
    <span class="web-pane__title">${escapeHtml(title)}</span>
    <a class="web-pane__url" href="${escapeHtml(cleaned)}" target="_blank" rel="noopener noreferrer">${escapeHtml(cleaned)}</a>`;
  const body = document.createElement("div");
  body.className = "web-pane__body";
  // Fallback for sites that block iframe embedding (X-Frame-Options /
  // CSP frame-ancestors): show a clear message with an external link.
  const fallback = document.createElement("div");
  fallback.className = "web-pane__fallback";
  fallback.hidden = true;
  fallback.innerHTML = `<div class="web-pane__fallback-title">${escapeHtml(tr("status.cantEmbed"))}</div>
    <div class="web-pane__fallback-body">${tr("status.cantEmbedBody", { url: escapeHtml(cleaned) })}</div>`;
  const iframe = document.createElement("iframe");
  iframe.src = cleaned;
  iframe.referrerPolicy = "no-referrer";
  // Sandbox WITHOUT allow-same-origin: scripts/forms/popups can run, but the
  // framed page is treated as a unique origin so it cannot reach back into
  // this app's DOM / localStorage / Tauri bridge. (Including both
  // allow-scripts and allow-same-origin defeats the sandbox entirely, since
  // the framed script can then strip its own sandbox attribute.)
  iframe.setAttribute("sandbox", "allow-scripts allow-forms allow-popups");
  iframe.addEventListener("load", () => {
    // Some sites render an empty page intentionally (DENY frame-ancestors)
    // rather than erroring. After a tick we peek at the iframe body — if
    // it's empty AND the URL isn't a file:// / data: / about: page, show
    // the fallback. Same-origin reads fail when the page is cross-origin,
    // which is itself a strong "blocked" signal.
    try {
      const doc = iframe.contentDocument || iframe.contentWindow?.document;
      if (doc && doc.body && doc.body.innerHTML.trim() === "") {
        fallback.hidden = false;
      }
    } catch (_) { /* cross-origin — assume the page loaded fine */ }
  });
  body.appendChild(iframe);
  body.appendChild(fallback);
  pane.appendChild(head);
  pane.appendChild(body);
  termsViewport.appendChild(pane);

  const tab = {
    tabId, kind: "web",
    url: cleaned, title,
    pane, body, iframe,
    project: null,  // browser tabs aren't tied to a project
  };
  state.tabs.push(tab);
  addTabButton(tab);
  switchTab(tabId);
  renderTabCount();
  setStatus(tr("status.openedUrl", { url: cleaned }));
  return tab;
}

function tabButtonContent(tab) {
  // Icon: tool monogram for pty tabs, VS Code-style file-type tile for
  // file tabs, an "WB" tile for browser tabs.
  let icon;
  if (tab.kind === "file") {
    icon = fileIconSvg(tab.name);
  } else if (tab.kind === "web") {
    icon = `<span class="tool-icon" style="background:var(--accent-dim);color:#000" title="${escapeHtml(tab.url || "")}">WB</span>`;
  } else {
    icon = toolIcon(tab.usage?.toolKey || "shell");
  }
  return `
    ${icon}
    <span class="term-tab__name">${escapeHtml(tab.title)}</span>
    <button class="term-tab__close" title="${escapeHtml(tr("terms.closeTab"))}">✕</button>`;
}

function refreshTabButton(tab) {
  const btn = termsBar.querySelector(
    `.term-tab[data-tab-id="${CSS.escape(String(tab.tabId))}"]`,
  );
  if (btn) btn.innerHTML = tabButtonContent(tab);
}

function addTabButton(tab) {
  const btn = document.createElement("div");
  btn.className = "term-tab";
  btn.dataset.tabId = String(tab.tabId);
  btn.innerHTML = tabButtonContent(tab);
  btn.addEventListener("click", (e) => {
    if (e.target.closest(".term-tab__close")) { closeTab(tab.tabId); return; }
    switchTab(tab.tabId);
  });
  termsBar.appendChild(btn);
}

// xterm positions a hidden textarea and a visible composition view beside the
// cursor while an IME is active. Chromium may horizontally scroll one of the
// terminal ancestors to keep that temporary element visible, which makes the
// whole workspace appear to jump left. The application has no intentional
// horizontal scrolling, so keep every ancestor on its zero origin throughout
// composition and refit only after the committed text has landed.
function resetTerminalHorizontalScroll(tab) {
  let node = tab.pane;
  while (node instanceof HTMLElement) {
    if (node.scrollLeft !== 0) node.scrollLeft = 0;
    node = node.parentElement;
  }
  const terminalViewport = tab.term.element?.querySelector(".xterm-viewport");
  if (terminalViewport?.scrollLeft) terminalViewport.scrollLeft = 0;
  const documentScroller = document.scrollingElement;
  if (documentScroller?.scrollLeft) documentScroller.scrollLeft = 0;
}

function refitTerminalAfterComposition(tab) {
  if (tab.dead || tab.composing || state.activeTabId !== tab.tabId) return;
  try {
    tab.fit.fit();
    tab._lastResize = `${tab.term.cols}x${tab.term.rows}`;
    invoke("pty_resize", {
      id: tab.ptyId,
      cols: tab.term.cols,
      rows: tab.term.rows,
    }).catch(() => {});
  } catch {}
}

function wireTerminalImeStability(tab) {
  const textarea = tab.term.textarea;
  if (!textarea) return () => {};

  let resetFrame = 0;
  const scheduleReset = () => {
    if (resetFrame) cancelAnimationFrame(resetFrame);
    resetFrame = requestAnimationFrame(() => {
      resetFrame = 0;
      resetTerminalHorizontalScroll(tab);
    });
  };
  const onCompositionStart = () => {
    tab.composing = true;
    tab.pane.classList.add("is-composing");
    resetTerminalHorizontalScroll(tab);
  };
  const onCompositionUpdate = () => scheduleReset();
  const onCompositionEnd = () => {
    tab.composing = false;
    tab.pane.classList.remove("is-composing");
    scheduleReset();
    requestAnimationFrame(() => refitTerminalAfterComposition(tab));
  };

  textarea.addEventListener("compositionstart", onCompositionStart);
  textarea.addEventListener("compositionupdate", onCompositionUpdate);
  textarea.addEventListener("compositionend", onCompositionEnd);
  return () => {
    textarea.removeEventListener("compositionstart", onCompositionStart);
    textarea.removeEventListener("compositionupdate", onCompositionUpdate);
    textarea.removeEventListener("compositionend", onCompositionEnd);
    if (resetFrame) cancelAnimationFrame(resetFrame);
  };
}

function switchTab(tabId) {
  state.activeTabId = tabId;
  termsEmpty.style.display = "none";
  for (const t of state.tabs) {
    const on = t.tabId === tabId;
    t.pane.classList.toggle("is-active", on);
    if (on && t.kind === "pty") {
      try {
        t.fit.fit();
        // The pane was `display:none` when openTerminalTab first measured
        // it, so pty_spawn got 0×0 cols/rows and the shell/Claude was
        // rendering into a tiny top-left corner. Re-sync the PTY to the
        // xterm size now that the pane is visible.
        invoke("pty_resize", { id: t.ptyId, cols: t.term.cols, rows: t.term.rows }).catch(() => {});
        t._lastResize = `${t.term.cols}x${t.term.rows}`;
        t.term.focus();
      } catch {}
    }
  }
  [...termsBar.querySelectorAll(".term-tab")].forEach(b =>
    b.classList.toggle("is-active", b.dataset.tabId === String(tabId)));
  updateCommonCommandsEnabled();
  renderTermsTitle();
  renderSelectedLaunchPanel();
}

function closeTab(tabId) {
  const idx = state.tabs.findIndex(t => t.tabId === tabId);
  if (idx < 0) return;
  const tab = state.tabs[idx];
  // Tear down pty tabs (kill the shell + dispose xterm); file tabs
  // just need their DOM node removed.
  if (tab.kind === "pty") {
    tab.dead = true;
    tab.disposeImeStability?.();
    if (HAS_TAURI) { invoke("pty_kill", { id: tab.ptyId }).catch(()=>{}); }
    try { tab.term.dispose(); } catch {}
  }
  tab.pane.remove();
  const btn = termsBar.querySelector(`.term-tab[data-tab-id="${CSS.escape(String(tabId))}"]`);
  btn?.remove();
  state.tabs.splice(idx, 1);
  // Re-render the ledger when a PTY tab closes so the project's entry
  // drops its `has-session` class and the name returns to white if no
  // other PTY tab for that project remains. File tabs don't affect the
  // green styling, so we skip the re-render for those.
  if (tab.kind === "pty") renderLedger();
  renderTabCount();
  if (state.activeTabId === tabId) {
    const next = state.tabs[idx] || state.tabs[idx - 1];
    if (next) switchTab(next.tabId);
    else {
      state.activeTabId = null;
      termsEmpty.style.display = "flex";
      // No switchTab happened above, so the terms-bar title didn't get
      // refreshed — do it now so it falls back to the selected project
      // (or the generic "SESSIONS" label if nothing is selected).
      renderTermsTitle();
    }
  }
  updateCommonCommandsEnabled();
  renderSelectedLaunchPanel();
}

function renderTabCount() {
  termsCount.textContent = String(state.tabs.length);
}

// Reflect the active project in the termsBar kicker so the right pane's
// header shows which project the user is looking at. Priority:
//   1. the active tab's project (a session is being shown right now)
//   2. the highlighted ledger entry (state.selectedId, set by click)
//   3. the generic "SESSIONS" label when neither applies
function renderTermsTitle() {
  const titleEl = termsBar.querySelector(".terms__title");
  if (!titleEl) return;
  const activeTab = state.tabs.find(t => t.tabId === state.activeTabId);
  // Browser tabs surface their hostname so the user can see at a glance
  // which page is being viewed (it has no `project`).
  let label;
  if (activeTab) {
    label = activeTab.kind === "web" ? activeTab.title : activeTab.project?.name;
  }
  if (!label && state.selectedId) {
    label = state.all.find(p => p.id === state.selectedId)?.name;
  }
  titleEl.textContent = label || "SESSIONS";
}

/* ── persistent project overview ────────────────────────── */
// The center pane follows the selected project even while a terminal is
// active. It combines project facts, per-tool activity, launch actions,
// group assignment, and links to every open workspace tab.
function renderSelectedLaunchPanel() {
  const project = state.selectedId
    ? state.all.find(p => p.id === state.selectedId)
    : null;
  if (!project) {
    termsSelectedLaunch.innerHTML = `
      <div class="overview__empty">
        <span class="overview__empty-mark">A</span>
        <strong>${escapeHtml(tr("overview.emptyTitle"))}</strong>
        <span>${escapeHtml(tr("overview.emptyBody"))}</span>
      </div>`;
    termsSelectedLaunch.hidden = false;
    return;
  }

  const usages = project.toolUsages || [];
  const openHint = tr("entry.openHint");
  const totalSessions = usages.reduce((sum, usage) => sum + (usage.sessionCount || 0), 0);
  const markParts = (project.name || "A").split(/[^a-zA-Z0-9]+/).filter(Boolean);
  const projectMark = (markParts.length > 1
    ? `${markParts[0][0]}${markParts[1][0]}`
    : (markParts[0] || "A").slice(0, 2)).toUpperCase();
  const branch = project.gitBranch || "—";
  const lastActive = project.lastAccessedAt ? relTime(project.lastAccessedAt) : "—";
  const activityRows = usages.map(u => {
    const usageTime = u.lastUsedAt ? relTime(u.lastUsedAt) : "—";
    const activityMeta = usageHasResumeTarget(u)
      ? tr("overview.activityMeta", { time: usageTime, count: u.sessionCount || 0 })
      : tr(activityOnlyLabelKey(u, "overview"), { time: usageTime });
    return `
      <div class="overview__activity-row">
        <span class="overview__tool-mark" style="--tool-color:${toolColor(u.toolKey)}">
          ${toolIcon(u.toolKey)}
        </span>
        <span class="overview__activity-copy">
          <strong>${escapeHtml(u.toolName)}</strong>
          <small>${escapeHtml(activityMeta)}</small>
        </span>
      </div>`;
  }).join("");
  const recentSessionRows = usages
    .filter(u => u.lastSessionId)
    .sort((a, b) => String(b.lastUsedAt || "").localeCompare(String(a.lastUsedAt || "")))
    .slice(0, 3)
    .map(u => {
      const openTab = state.tabs.find(t =>
        t.kind === "pty" && t.project?.id === project.id && t.usage?.toolKey === u.toolKey);
      const actionAttrs = openTab
        ? `data-switch-tab="${escapeHtml(String(openTab.tabId))}"`
        : `data-project-id="${escapeHtml(project.id)}" data-tool="${escapeHtml(u.toolKey)}"`;
      const disabledAttrs = openTab ? "" : disabledTuiAttrs(project, u.toolKey);
      const actionLabel = openTab ? tr("overview.openTerminal") : tr("entry.resume");
      const rawId = String(u.lastSessionId);
      const shortId = rawId.length > 14 ? `${rawId.slice(0, 8)}…${rawId.slice(-4)}` : rawId;
      const usageTime = u.lastUsedAt ? relTime(u.lastUsedAt) : "—";
      return `
        <div class="overview__session-row">
          <span class="overview__session-glyph" aria-hidden="true">&gt;_</span>
          <span class="overview__session-copy">
            <strong>${escapeHtml(u.toolName)}</strong>
            <small>${escapeHtml(tr("overview.sessionMeta", { id: shortId, time: usageTime }))}</small>
          </span>
          <button class="launch-pill overview__session-resume" ${actionAttrs} ${disabledAttrs}>${escapeHtml(actionLabel)}</button>
        </div>`;
    }).join("");
  const openers = project.source === "remote" ? [] : (state.openerPrefs || []).filter(o => o.enabled);
  const openerPills = openers.map(o => {
    const hint = (o.command || "").replace(/\{path\}/g, "").trim() || openHint;
    return `<button class="launch-pill launch-pill--ext"
              data-project-id="${escapeHtml(project.id)}" data-opener-id="${escapeHtml(String(o.id))}">
              ${escapeHtml(o.label)}<small>${escapeHtml(hint)}</small>
            </button>`;
  }).join("");
  const webDevelopmentPills = webDevelopmentLaunchPillsHtml();
  const otherTabs = state.tabs.map(t => {
    const dotKey = t.usage?.toolKey || "shell";
    // t.project is null for web tabs — fall back to the tab title so the
    // "other sessions" row never crashes on `t.project.name`.
    const projName = t.project?.name || t.title || "";
    const activeClass = t.tabId === state.activeTabId ? "is-active" : "";
    return `<button class="terms__other-tab ${activeClass}" data-switch-tab="${escapeHtml(String(t.tabId))}">
              <i class="dot ${toolDotClass(dotKey)}" style="background:${toolColor(dotKey)}"></i>
              <span class="terms__other-tab__name">${escapeHtml(t.title)}</span>
              <span class="terms__other-tab__proj">${escapeHtml(projName)}</span>
            </button>`;
  }).join("");

  termsSelectedLaunch.innerHTML = `
    <div class="overview__hero">
      <div class="overview__project-mark">${escapeHtml(projectMark)}</div>
      <div class="overview__identity">
        <div class="overview__identity-head">
          <div class="terms__selected-launch__name">${escapeHtml(project.name)}</div>
          ${projectSourceBadgeHtml(project, "overview__source")}
        </div>
        <div class="terms__selected-launch__path">${escapeHtml(project.path)}</div>
      </div>
      <div class="overview__facts">
        <span><small>${escapeHtml(tr("overview.branch"))}</small><strong>⌘ ${escapeHtml(branch)}</strong></span>
        <span><small>${escapeHtml(tr("overview.sessions"))}</small><strong>${totalSessions}</strong></span>
        <span><small>${escapeHtml(tr("overview.lastActive"))}</small><strong>${escapeHtml(lastActive)}</strong></span>
      </div>
    </div>

    <div class="overview__section overview__group">
      <span class="overview__section-title">${escapeHtml(tr("entry.label.group"))}</span>
      <select class="group-picker__select" data-group-picker data-project-id="${escapeHtml(project.id)}">
        <option value="">${escapeHtml(tr("group.ungrouped"))}</option>
        ${state.groups.map(g => `<option value="${g.id}" ${state.assignments[project.id] === g.id ? "selected" : ""}>${escapeHtml(g.name)}</option>`).join("")}
      </select>
    </div>

    <div class="overview__section">
      <div class="overview__section-head">
        <span class="overview__section-title">${escapeHtml(tr("overview.activity"))}</span>
        <span class="overview__section-count">${usages.length}</span>
      </div>
      <div class="overview__activity">
        ${activityRows || `<span class="overview__muted">${escapeHtml(tr("entry.noInstruments"))}</span>`}
      </div>
    </div>

    ${recentSessionRows ? `
      <div class="overview__section overview__sessions">
        <div class="overview__section-head">
          <span class="overview__section-title">${escapeHtml(tr("overview.recentSessions"))}</span>
          <span class="overview__section-count">${recentSessionRows ? usages.filter(u => u.lastSessionId).length : 0}</span>
        </div>
        <div class="overview__session-list">${recentSessionRows}</div>
      </div>` : ""}

    <div class="overview__section">
      <span class="overview__section-title">${escapeHtml(tr("overview.quickActions"))}</span>
      <div class="overview__actions">
        <button class="launch-pill launch-pill--new" data-project-id="${escapeHtml(project.id)}" data-tool="shell">${escapeHtml(tr("entry.shellNew"))}</button>
        <button class="launch-pill" data-overview-files>${escapeHtml(tr("overview.files"))}</button>
        <button class="launch-pill" data-overview-settings>${escapeHtml(tr("overview.settings"))}</button>
        <button class="launch-pill launch-pill--danger" data-ignore-project>${escapeHtml(tr("entry.ignoreTree"))}</button>
        ${openerPills}
        ${webDevelopmentPills}
      </div>
    </div>

    ${otherTabs ? `
      <div class="overview__section terms__selected-launch__other">
        <span class="overview__section-title">${escapeHtml(tr("entry.label.otherSessions", { count: state.tabs.length }))}</span>
        <div class="terms__selected-launch__tabs">${otherTabs}</div>
      </div>` : ""}
  `;
  termsSelectedLaunch.hidden = false;
  wireLaunchPills(termsSelectedLaunch);
  termsSelectedLaunch.querySelector("[data-overview-files]")?.addEventListener("click", () => setViewMode("files"));
  termsSelectedLaunch.querySelector("[data-overview-settings]")?.addEventListener("click", openSettings);
  termsSelectedLaunch.querySelector("[data-ignore-project]")?.addEventListener("click", () => {
    ignoreProjectTree(project);
  });
}

// Centralized launch-pill click wiring — used by the ledger popover
// (per-render) and the right-pane launch panel (per-render).
function wireLaunchPills(container) {
  // Match only the popover / launch-panel pills that carry `data-tool`.
  // The session-card Resume buttons inside each ledger entry also use
  // .launch-pill but use `data-launch-tool` (handled via the ledger's
  // delegated click listener); without this attribute filter the two
  // handlers would race and the eager stopPropagation here would
  // shadow the session-card path.
  container.querySelectorAll(".launch-pill:not(.launch-pill--ext)[data-tool]").forEach(el => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      const projectId = el.dataset.projectId;
      const toolKey = el.dataset.tool;
      const p = state.all.find(x => x.id === projectId);
      if (p) {
        const usage = (p.toolUsages || []).find(u => u.toolKey === toolKey);
        openTerminalTab(p, usage || { toolKey, toolName: toolKey });
      }
    });
  });
  container.querySelectorAll(".launch-pill--ext").forEach(el => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      const projectId = el.dataset.projectId;
      const openerId = el.dataset.openerId;
      const p = state.all.find(x => x.id === projectId);
      if (p) fireOpener(openerId, p.path, el.textContent.trim());
    });
  });
  container.querySelectorAll("[data-web-development-launch]").forEach(el => {
    el.addEventListener("click", (event) => {
      event.stopPropagation();
      const id = Number(el.dataset.webDevelopmentLaunch);
      const tool = state.webDevelopmentTools.find(item => Number(item.id) === id && item.enabled);
      if (tool) openWebTab(tool.connectionUrl, tool.label);
    });
  });
  // "Other open sessions" cards in the right-pane panel. data-switch-tab
  // carries the tab's tabId — a number for pty tabs, a string for web/file
  // tabs. switchTab compares with ===, so coerce numeric ids back to numbers
  // and leave string tabIds as-is.
  container.querySelectorAll("[data-switch-tab]").forEach(el => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      const raw = el.dataset.switchTab;
      const id = /^\d+$/.test(raw) ? Number(raw) : raw;
      switchTab(id);
    });
  });
  // Group-picker dropdown (popover + launch panel). Empty string = 未分组.
  container.querySelectorAll("[data-group-picker]").forEach(el => {
    el.addEventListener("change", () => {
      const projectId = el.dataset.projectId;
      const raw = el.value;
      const groupId = raw === "" ? null : Number(raw);
      setProjectGroup(projectId, groupId);
    });
  });
}

/* ── pty event pump (single listener) ───────────────────── */
async function wirePtyEvents() {
  if (!HAS_TAURI || !listen) return;
  await Promise.all([
    listen("pty-data", (ev) => {
      const { id, data } = ev.payload;
      const tab = state.tabs.find(t => t.ptyId === id);
      if (!tab) return;
      try { tab.term.write(data); } catch {}
    }),
    listen("pty-exit", (ev) => {
      const tab = state.tabs.find(t => t.ptyId === ev.payload.id);
      if (!tab) return;
      // Queue tab: a pty-exit means the current prompt finished (claude -p
      // runs to completion then exits). Fire a notification and, if more
      // prompts remain, spawn the next one into the SAME tab/terminal so
      // the user watches a continuous log. Only when the queue is drained
      // do we mark the tab dead like an ordinary session. This runs BEFORE
      // the dead guard so a finished prompt (which sets dead) still advances.
      if (tab.isQueue) {
        advanceQueue(tab);
        return;
      }
      if (tab.dead) return;
      tab.dead = true;
      const ended = ev.payload.exitCode == null
        ? tr("term.sessionEnded")
        : tr("term.sessionEndedCode", { code: ev.payload.exitCode });
      const suffix = ev.payload.readError ? ` ${tr("term.streamFailed")}` : "";
      try { tab.term.write(`\r\n\x1b[90m${ended}${suffix}\x1b[0m\r\n`); } catch {}
      const btn = termsBar.querySelector(`.term-tab[data-tab-id="${CSS.escape(String(tab.ptyId))}"]`);
      btn?.classList.add("is-dead");
      renderLedger();
      updateCommonCommandsEnabled();
    }),
  ]);
  state.ptyEventsReady = true;
}

/* ── Claude Code task queue ────────────────────────────────── */
// A queue tab is a normal pty tab that runs a SERIES of `claude -p`
// prompts one after another. Each prompt is its own process (spawned
// via pty_spawn with `claudePrint`); when it exits, advanceQueue() is
// called from the pty-exit handler, which fires a notification and
// spawns the next prompt into the SAME terminal so the output reads as
// one continuous log. The tab title carries progress like "name · claude (2/5)".
//
// Detection reliability is the crux: print mode (`claude -p`) exits on
// its own when the prompt is answered, so pty-exit == "task done" with
// no fuzzy output-scraping. Claude's normal permission policy remains active;
// unattended tasks that need approval may stop instead of being auto-approved.

// Build the queue tab and kick off the first prompt. `prompts` is an
// array of non-empty strings (trimmed upstream). Reuses the same xterm
// creation as openTerminalTab, but skips the shell + pendingCmd path —
// claude is launched directly in print mode.
async function openQueueTab(project, prompts) {
  const title = `${project.name} · claude`;
  if (!HAS_TAURI || !HAS_TERM) {
    setStatus(tr("status.demoWouldOpenTerm"));
    return;
  }
  setStatus(tr("queue.starting", { count: prompts.length, name: project.name }));

  const pane = document.createElement("div");
  pane.className = "term-pane";
  termsViewport.appendChild(pane);

  const term = new window.Terminal({
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 13, cursorBlink: true, scrollback: 5000,
    theme: {
      background: "#0e0d0b", foreground: "#ece6d6", cursor: "#c6f24e",
      selectionBackground: "#c6f24e", selectionForeground: "#0e0d0b",
      black:"#0e0d0b", red:"#ff6b3d", green:"#10a37f", yellow:"#e8b339",
      blue:"#6c8aff", magenta:"#c6f24e", cyan:"#5ec8d8", white:"#ece6d6",
      brightBlack:"#7d7563", brightRed:"#ff6b3d", brightGreen:"#10a37f",
      brightYellow:"#e8b339", brightBlue:"#6c8aff", brightMagenta:"#c6f24e",
      brightCyan:"#5ec8d8", brightWhite:"#ffffff",
    },
  });
  const fit = new window.FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(pane);
  fit.fit();

  // Banner so the user knows what's running. Each prompt gets its own
  // header written by spawnQueuePrompt.
  term.write(`\x1b[1;36m${escapeHtml(tr("queue.banner", { name: project.name, count: prompts.length }))}\x1b[0m\r\n\r\n`);

  const usage = { toolKey: "claude", toolName: "Claude Code" };
  // queueIdx is the index of the CURRENTLY running prompt; queueTotal
  // is the snapshot length used for the "(n/total)" title. isQueue
  // routes pty-exit to advanceQueue instead of the dead-session path.
  // tabId is a STABLE string (independent of ptyId) so the tab button
  // keeps its identity across prompts — ptyId changes with each spawn,
  // but switchTab/closeTab/button-matching key off tabId.
  const tab = {
    tabId: `q${Date.now().toString(36)}${Math.random().toString(36).slice(2,6)}`,
    kind: "pty", ptyId: null, title, term, fit, pane,
    project, usage, started: true, dead: false, pendingCmd: null,
    isQueue: true,
    queue: prompts.slice(),
    queueIdx: 0,
    queueTotal: prompts.length,
  };

  // Spawn the first prompt. spawnQueuePrompt fills in tab.ptyId (the
  // numeric session id) and re-binds term.onData to it.
  try {
    await spawnQueuePrompt(tab);
  } catch (e) {
    term.write(`\x1b[31m${escapeHtml(tr("queue.startFailed", { err: e }))}\x1b[0m\r\n`);
    pane.classList.add("is-active");
    termsEmpty.style.display = "none";
    setStatus(tr("status.sessionFailed", { err: e }));
    return;
  }

  addTabButton(tab);
  switchTab(tab.tabId);
  renderTabCount();
  updateQueueTabTitle(tab);
  // Optimistically touch the project so its entry shows "now".
  project.lastAccessedAt = new Date().toISOString();
  renderLedger();
  setStatus(tr("queue.running", { idx: tab.queueIdx + 1, total: tab.queueTotal, name: project.name }));
}

// Spawn a single queue prompt (the one at tab.queueIdx) into the tab's
// existing terminal. Uses claudePrint so the backend runs `claude -p`
// directly and exits on completion. Updates tab.ptyId to the new session
// id and re-binds term.onData so keystrokes route to the fresh process.
// tab.tabId is left untouched (stable across prompts).
async function spawnQueuePrompt(tab) {
  const prompt = tab.queue[tab.queueIdx];
  // Visual separator + header so consecutive runs are distinguishable
  // in the scrollback.
  try {
    tab.term.write(`\x1b[1;33m── ${escapeHtml(tr("queue.taskHeader", { idx: tab.queueIdx + 1, total: tab.queueTotal }))} ──\x1b[0m\r\n`);
    tab.term.write(`\x1b[90m> ${escapeHtml(prompt)}\x1b[0m\r\n\r\n`);
  } catch {}
  const id = await invoke("pty_spawn", {
    path: tab.project.path, cols: tab.term.cols, rows: tab.term.rows,
    source: "local", remote: null, claudePrint: prompt,
  });
  tab.ptyId = id;
  tab.started = true;
  tab.dead = false;
  // Re-bind keystrokes to the new process id. term.onData returns a
  // disposable; we hold the previous one and dispose it so we don't
  // accumulate handlers across prompts.
  if (tab._onDataDisp) { try { tab._onDataDisp.dispose(); } catch {} }
  tab._onDataDisp = tab.term.onData((d) => { invoke("pty_write", { id: tab.ptyId, data: d }).catch(()=>{}); });
  updateQueueTabTitle(tab);
  return id;
}

// Refresh the tab button label to show queue progress, e.g.
// "name · claude (2/5)". Called on each spawn and each advance.
function updateQueueTabTitle(tab) {
  const btn = termsBar.querySelector(`.term-tab[data-tab-id="${CSS.escape(String(tab.tabId))}"]`);
  if (!btn) return;
  const label = btn.querySelector(".term-tab__name");
  if (label) label.textContent = `${tab.title} (${tab.queueIdx + 1}/${tab.queueTotal})`;
}

// Called from the pty-exit handler for queue tabs. Fires a notification,
// advances to the next prompt (or finishes the queue), and keeps the
// terminal alive across prompts.
async function advanceQueue(tab) {
  const doneIdx = tab.queueIdx + 1;
  const projectName = tab.project?.name || "";
  // Native notification for the just-finished prompt.
  fireNotify(
    tr("queue.notifyTaskDone", { name: projectName }),
    tr("queue.notifyTaskDoneBody", { idx: doneIdx, total: tab.queueTotal })
  );
  if (doneIdx < tab.queueTotal) {
    // More to go: advance and spawn the next prompt into this same tab.
    tab.queueIdx = doneIdx;
    updateQueueTabTitle(tab);
    try {
      await spawnQueuePrompt(tab);
      setStatus(tr("queue.running", { idx: tab.queueIdx + 1, total: tab.queueTotal, name: projectName }));
    } catch (e) {
      try { tab.term.write(`\x1b[31m${escapeHtml(tr("queue.advanceFailed", { err: e }))}\x1b[0m\r\n`); } catch {}
      finishQueue(tab);
    }
  } else {
    finishQueue(tab);
  }
}

// Drain the queue: final notification + mark the tab dead (same visual
// treatment as an ordinary ended session). New prompts can still be
// appended later via appendQueuePrompts, which re-spawns.
function finishQueue(tab) {
  const projectName = tab.project?.name || "";
  fireNotify(
    tr("queue.notifyAllDone", { name: projectName }),
    tr("queue.notifyAllDoneBody", { count: tab.queueTotal })
  );
  tab.dead = true;
  try {
    tab.term.write(`\r\n\x1b[1;32m${escapeHtml(tr("queue.allDone", { count: tab.queueTotal }))}\x1b[0m\r\n`);
    tab.term.write(`\x1b[90m${escapeHtml(tr("term.sessionEnded"))}\x1b[0m\r\n`);
  } catch {}
  const btn = termsBar.querySelector(`.term-tab[data-tab-id="${CSS.escape(String(tab.tabId))}"]`);
  btn?.classList.add("is-dead");
  setStatus(tr("queue.allDoneStatus", { name: projectName }));
}

// Append more prompts to a finished (or in-flight) queue tab and resume.
// If the tab was dead, clear the dead styling and spawn the next prompt.
// Used by the expanded-panel "add to queue" textarea when a queue tab
// already exists for the project.
async function appendQueuePrompts(tab, prompts) {
  if (!prompts.length) return;
  const startIdx = tab.queue.length;
  tab.queue.push(...prompts);
  tab.queueTotal = tab.queue.length;
  // If the queue is currently idle (dead), resume from the first newly
  // added prompt. Otherwise advanceQueue will pick them up naturally as
  // it iterates tab.queue.
  if (tab.dead) {
    tab.dead = false;
    tab.queueIdx = startIdx;
    const btn = termsBar.querySelector(`.term-tab[data-tab-id="${CSS.escape(String(tab.tabId))}"]`);
    btn?.classList.remove("is-dead");
    try { tab.term.write(`\r\n\x1b[1;36m${escapeHtml(tr("queue.resumed", { count: prompts.length }))}\x1b[0m\r\n\r\n`); } catch {}
    try {
      await spawnQueuePrompt(tab);
      setStatus(tr("queue.running", { idx: tab.queueIdx + 1, total: tab.queueTotal, name: tab.project?.name || "" }));
    } catch (e) {
      try { tab.term.write(`\x1b[31m${escapeHtml(tr("queue.advanceFailed", { err: e }))}\x1b[0m\r\n`); } catch {}
      finishQueue(tab);
    }
  } else {
    updateQueueTabTitle(tab);
    setStatus(tr("queue.appended", { count: prompts.length, total: tab.queueTotal }));
  }
}

// Thin wrapper around the Rust `notify` command. Silent on failure — a
// missing OS notification shouldn't break the queue.
function fireNotify(title, body) {
  if (!HAS_TAURI) return;
  invoke("notify", { title, body: body || null }).catch(() => {});
}

/* ── refit on viewport resize ───────────────────────────── */
// Push the current ledger + group state to the Rust tray so the
// tray's right-click menu tracks what the user sees on the left. The
// backend caches the data in AppState and rebuilds the menu items in
// place — fast, no tray icon recreate, no menu flicker. Failures are
// silent (e.g. the tray icon doesn't exist in the browser demo).
async function syncTrayProjects() {
  if (!HAS_TAURI) return;
  const projects = state.catalog.map(p => ({
    id: p.id,
    name: p.name,
    path: p.path,
    topTool: (p.toolUsages || [])[0]?.toolKey || null,
  }));
  const groups = state.groups.map(g => ({ id: g.id, name: g.name }));
  // state.assignments is { projectId: groupId } — flatten into the
  // {projectId: groupId} shape the backend expects.
  const assignments = { ...state.assignments };
  try {
    await invoke("update_tray_projects", { projects, groups, assignments });
  } catch (e) { /* tray might not exist in some contexts */ }
}

// Listen for "project:open" emitted by the Rust tray when the user
// picks a project from the right-click tray menu. Opens it the same
// way a double-click on the ledger would (most-recently-used tool).
function wireTrayEvents() {
  if (!HAS_TAURI) return;
  if (typeof listen !== "function") return;
  listen("project:open", (e) => {
    const projectId = e.payload;
    const p = state.all.find(x => x.id === projectId);
    if (p) openProjectDefault(p);
  });
}

function wireResize() {
  if (typeof ResizeObserver === "undefined") return;
  let frame = 0;
  const schedule = () => {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      const tab = state.tabs.find(t => t.tabId === state.activeTabId && t.kind === "pty");
      if (!tab || tab.composing || tab.dead) return;
      try {
        tab.fit.fit();
        const size = `${tab.term.cols}x${tab.term.rows}`;
        if (tab._lastResize === size) return;
        tab._lastResize = size;
        invoke("pty_resize", { id: tab.ptyId, cols: tab.term.cols, rows: tab.term.rows }).catch(()=>{});
      } catch {}
    });
  };
  const ro = new ResizeObserver(schedule);
  ro.observe(termsViewport);
}

/* ── entry modal (⋯ on a project) ───────────────────────── */
// A centered modal dialog replaces the per-row position:fixed popover,
// which was fiddly (clipping, stale-on-scroll, z-fighting). The modal
// shows the same content (group picker + path + tags + launch pills).
function toggleEntryMenu(projectId) {
  if (state.menuOpenId === projectId) closeEntryMenu();
  else { closeEntryMenu(); openEntryMenu(projectId); }
}
function openEntryMenu(projectId) {
  const p = state.all.find(x => x.id === projectId);
  if (!p) return;
  state.menuOpenId = projectId;
  if (state.selectedId !== projectId) select(projectId);
  entryModalTitle.textContent = p.name;
  // entryMenuHtml(p, true) renders the content with the menu visible.
  entryModalBody.innerHTML = entryMenuHtml(p, true);
  entryModal.hidden = false;
  activateDialog(entryModal, "[data-entry-modal-close]");
  wireLaunchPills(entryModalBody);
  // Pills inside the modal fire then close it.
  entryModalBody.querySelectorAll(".launch-pill").forEach(el => {
    el.addEventListener("click", () => closeEntryMenu(), { once: true });
  });
  entryModalBody.querySelector("[data-ignore-project]")?.addEventListener("click", () => {
    ignoreProjectTree(p);
  }, { once: true });
  // Lazily load the project's markdown docs list into the placeholder.
  loadProjectDocsIntoEntryMenu(p);
  // Lazily load the project's file tree (root only) into the placeholder.
  // loadFileTreeInto wires the click handler on its own container.
  loadFileTreeIntoEntryMenu(p);
}
function closeEntryMenu() {
  entryDocsGate.invalidate();
  entryTreeGate.invalidate();
  if (state.menuOpenId == null) return;
  state.menuOpenId = null;
  entryModal.hidden = true;
  entryModalBody.innerHTML = "";
  deactivateDialog(entryModal);
}
function wireEntryMenuDelegation() {
  ledger.addEventListener("click", (e) => {
    const toggle = e.target.closest("[data-menu-toggle]");
    if (toggle) {
      e.stopPropagation();
      const projectId = toggle.closest(".entry")?.dataset.id;
      if (projectId) toggleEntryMenu(projectId);
      return;
    }
    // Per-project tree button: switch to the files view scoped to this
    // project. The .entry's bubble-phase click handler will have already
    // called select(projectId) by the time we get here, which is exactly
    // what refreshLeftPaneTree wants when we enter files mode.
    const treeBtn = e.target.closest("[data-tree-btn]");
    if (treeBtn) {
      setViewMode("files");
      return;
    }
    // Chevron: toggle the inline session panel without selecting the row.
    const expandBtn = e.target.closest("[data-expand-toggle]");
    if (expandBtn) {
      e.stopPropagation();
      const projectId = expandBtn.closest(".entry")?.dataset.id;
      if (projectId) toggleEntryExpand(projectId);
      return;
    }
    // Per-tool launch pill in the expanded session panel. Builds the
    // usage record from data-launch-tool + data-launch-sid and calls
    // openTerminalTab directly (openProjectDefault only uses the top
    // usage, which would always pick the most-recent tool — not what
    // the user asked for when they clicked a specific Resume button).
    const launchBtn = e.target.closest("[data-launch-tool]");
    if (launchBtn) {
      e.stopPropagation();
      const projectId = launchBtn.closest(".entry")?.dataset.id;
      const toolKey = launchBtn.dataset.launchTool || "shell";
      const sid = launchBtn.dataset.launchSid || "";
      const project = state.all.find(x => x.id === projectId);
      if (!project) return;
      const usage = (project.toolUsages || []).find(u => u.toolKey === toolKey)
                 || { toolKey, toolName: toolKey };
      // Preserve lastSessionId even if the lookup missed (e.g. when
      // launching a fresh shell — no recorded usage).
      if (sid && !usage.lastSessionId) usage.lastSessionId = sid;
      openTerminalTab(project, usage);
      return;
    }
    // Claude task queue: read the textarea (one prompt per line), then
    // either open a fresh queue tab or append to an existing one.
    const queueRunBtn = e.target.closest("[data-queue-run]");
    if (queueRunBtn) {
      e.stopPropagation();
      const panel = queueRunBtn.closest("[data-queue-panel]");
      const projectId = panel?.dataset.projectId;
      const project = state.all.find(x => x.id === projectId);
      if (!project) return;
      const ta = panel.querySelector("[data-queue-input]");
      const prompts = (ta?.value || "")
        .split("\n")
        .map(s => s.trim())
        .filter(s => s.length > 0);
      if (!prompts.length) {
        setStatus(tr("queue.noPrompts"));
        return;
      }
      const existing = state.tabs.find(t => t.isQueue && t.project?.id === project.id);
      if (existing) {
        appendQueuePrompts(existing, prompts);
        switchTab(existing.tabId);
      } else {
        openQueueTab(project, prompts);
      }
      // Clear the textarea after submission.
      if (ta) ta.value = "";
    }
  });
  // Backdrop / close button.
  entryModal.addEventListener("click", (e) => {
    if (e.target.closest("[data-entry-modal-close]")) closeEntryMenu();
  });
  // Doc modal close wiring — backdrop + ✕.
  const docModal = document.getElementById("docModal");
  docModal.addEventListener("click", (e) => {
    if (e.target.closest("[data-doc-modal-close]")) closeDocModal();
  });
  // Esc closes the entry modal first, then the doc modal, before any global handler.
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    if (state.menuOpenId != null) { e.stopPropagation(); closeEntryMenu(); return; }
    if (!docModal.hidden) { e.stopPropagation(); closeDocModal(); }
  });
}

/* ── settings drawer ────────────────────────────────────── */
const drawer = document.getElementById("drawer");
const drawerBody = document.getElementById("drawerBody");

function openSettings() {
  if (!drawer.hidden) return; // already open; don't double-bind listeners
  if (HAS_TAURI && !state.openerPrefsLoaded) loadOpenerPrefs();
  if (!state.webDevelopmentToolsLoaded) loadWebDevelopmentTools();
  state.settingsView = "menu";
  renderDrawerBody();
  drawer.hidden = false;
  activateDialog(drawer, "[data-settings-view]");
  document.addEventListener("keydown", escCloseDrawer);
  document.addEventListener("click", backdropCloseDrawer);
}
function closeSettings() {
  if (drawer.hidden) return; // already closed
  drawer.hidden = true;
  state.settingsView = "menu";
  deactivateDialog(drawer);
  document.removeEventListener("keydown", escCloseDrawer);
  document.removeEventListener("click", backdropCloseDrawer);
  // Reconcile with whatever the user changed in the drawer.
  if (HAS_TAURI) {
    loadOpenerPrefs();
    loadWebDevelopmentTools();
  }
}
function escCloseDrawer(e) {
  if (e.key !== "Escape") return;
  // Esc pops one level on a sub-page first, then closes the drawer.
  if (!drawer.hidden) {
    e.stopPropagation();
    if (state.settingsView !== "menu") {
      state.settingsView = "menu";
      renderDrawerBody();
      focusDialogStart(drawer, "[data-settings-view]");
    } else {
      closeSettings();
    }
  }
}
function backdropCloseDrawer(e) {
  if (e.target?.matches?.("[data-drawer-close]")) closeSettings();
}

// Title + kicker rendered into the drawer header based on the current
// settings view. Called from renderDrawerBody so the chrome always
// matches the body.
function renderDrawerHead() {
  const kicker = document.querySelector(".drawer__kicker");
  const title = document.getElementById("drawerTitle");
  const back = document.getElementById("drawerBackBtn");
  if (kicker) kicker.textContent = state.settingsView === "menu" ? tr("drawer.kickerMenu") : tr("drawer.kickerPage");
  if (title) {
    title.textContent = state.settingsView === "menu"
      ? tr("drawer.titleMenu")
      : settingsTitleFor(state.settingsView);
  }
  if (back) back.hidden = state.settingsView === "menu";
}

// Switch to a sub-page (or "menu" to go back to the main menu). Each
// invocation re-renders the body and updates the header.
function setSettingsView(view) {
  state.settingsView = view;
  renderDrawerBody();
  if (view === "tui") void refreshAllTuiCapabilities({ includeRemote: true });
  if (view === "cleanup") void refreshSessionCleanup();
  if (view === "webDevelopment" && !state.webDevelopmentToolsLoaded) void loadWebDevelopmentTools();
  focusDialogStart(drawer, view === "menu" ? "[data-settings-view]" : "#drawerBackBtn");
}

// Catalog of settings sub-pages. Order here is the order they appear in
// the main menu. Adding a new category means adding an entry here + a
// body function below. `titleKey` / `blurbKey` are i18n keys so the menu
// re-localizes when the language changes.
const SETTINGS_VIEWS = {
  cleanup: {
    titleKey: "settings.cleanup.title",
    blurbKey: "settings.cleanup.blurb",
    body: renderSessionCleanupViewBody,
  },
  tui: {
    titleKey: "settings.tui.title",
    blurbKey: "settings.tui.blurb",
    body: renderTuiViewBody,
  },
  webDevelopment: {
    titleKey: "settings.webDevelopment.title",
    blurbKey: "settings.webDevelopment.blurb",
    body: renderWebDevelopmentViewBody,
  },
  remote: {
    titleKey: "settings.remote.title",
    blurbKey: "settings.remote.blurb",
    body: renderRemoteViewBody,
  },
  ignores: {
    titleKey: "settings.ignores.title",
    blurbKey: "settings.ignores.blurb",
    body: renderIgnoresViewBody,
  },
  groups: {
    titleKey: "settings.groups.title",
    blurbKey: "settings.groups.blurb",
    body: renderGroupsViewBody,
  },
  openers: {
    titleKey: "settings.openers.title",
    blurbKey: "settings.openers.blurb",
    body: renderOpenersViewBody,
  },
  language: {
    titleKey: "settings.language.title",
    blurbKey: "settings.language.blurb",
    body: renderLanguageViewBody,
  },
};
const SETTINGS_ORDER = ["language", "cleanup", "tui", "webDevelopment", "remote", "ignores", "groups", "openers"];

// Localized title/blurb for a settings view (used by the menu + header).
function settingsTitleFor(view) {
  const v = SETTINGS_VIEWS[view];
  return v ? tr(v.titleKey) : tr("drawer.titleMenu");
}
function settingsBlurbFor(view) {
  const v = SETTINGS_VIEWS[view];
  return v ? tr(v.blurbKey) : "";
}

// Main menu: a row per category, showing title + short blurb + count.
function renderSettingsMenuBody() {
  const items = SETTINGS_ORDER.map(key => {
    const v = SETTINGS_VIEWS[key];
    const count = settingsCountFor(key);
    return `
      <div class="settings-menu__row" data-settings-view="${key}" role="button" tabindex="0">
        <div class="settings-menu__main">
          <div class="settings-menu__title">${escapeHtml(tr(v.titleKey))}</div>
          <div class="settings-menu__blurb">${escapeHtml(tr(v.blurbKey))}</div>
        </div>
        <div class="settings-menu__count">${count}</div>
      </div>`;
  }).join("");
  return items;
}

// Compact count badge for each category — derived from live state so
// the menu shows the current number of items without a roundtrip.
function settingsCountFor(view) {
  switch (view) {
    case "tui": return String(1 + state.remoteServers.length);
    case "webDevelopment": return String(state.webDevelopmentTools.length);
    case "remote": return String(state.remoteServers.length);
    case "ignores": return String(state.projectIgnores.length);
    case "groups": return String(state.groups.length);
    case "openers": return String((state.openerPrefs || []).length);
    case "language": return currentLang() === "zh" ? "中" : "EN";
    case "cleanup": {
      const candidates = state.sessionCleanup?.candidates || [];
      return candidates.length ? String(candidates.filter(item => item.canClean).length) : "—";
    }
    default: return "";
  }
}

// Sub-page: remote servers list + the "Add SSH server" form, both
// inlined so the user only sees them after drilling into this view.
function publishRemoteServer(server) {
  state.remoteServers = [
    ...state.remoteServers.filter(item => Number(item.id) !== Number(server.id)),
    server,
  ].sort((left, right) => Number(left.id) - Number(right.id));
  state.remoteServerById = Object.fromEntries(state.remoteServers.map(item => [item.id, item]));
}

function renderTuiViewBody() {
  const localOsFamily = state.all.find(project => project.source !== "remote")?.osFamily
    || currentBrowserOsFamily();
  const machines = [
    { key: "local", serverId: null, label: tr("tui.localMachine"), source: "local", osFamily: localOsFamily },
    ...state.remoteServers.map(server => ({
      key: tuiMachineKey(server.id),
      serverId: Number(server.id),
      label: server.label || server.host,
      source: "remote",
      osFamily: server.osFamily,
    })),
  ];
  const adapterForm = `<form id="tuiAdapterForm" class="tui-adapter-form">
    <div class="tui-adapter-form__copy">
      <strong>${escapeHtml(tr("tui.adapterInstallTitle"))}</strong>
      <span>${escapeHtml(tr("tui.adapterInstallHint"))}</span>
    </div>
    <input class="custom-form__input" name="manifestPath" data-tui-adapter-path
           value="${escapeHtml(state.tuiAdapterManifestPath)}"
           placeholder="${escapeHtml(tr("tui.adapterPathPlaceholder"))}"
           aria-label="${escapeHtml(tr("tui.adapterPathAria"))}"
           ${state.tuiAdapterImporting ? "disabled" : ""} />
    <button class="scan-btn" type="submit" ${state.tuiAdapterImporting ? "disabled" : ""}>
      ${escapeHtml(tr(state.tuiAdapterImporting ? "tui.adapterImporting" : "tui.adapterImport"))}
    </button>
  </form>`;
  const adapterDiagnostics = state.tuiMachines.local?.adapterDiagnostics || [];
  const adapterWarnings = adapterDiagnostics.length
    ? `<div class="tui-adapter-diagnostics" role="status">
        <strong>${escapeHtml(tr("tui.adapterDiagnostics"))}</strong>
        ${adapterDiagnostics.map(message => `<span>${escapeHtml(message)}</span>`).join("")}
      </div>`
    : "";
  return `<div class="tui-intro">${escapeHtml(tr("tui.intro"))}</div>${adapterForm}${adapterWarnings}${machines.map(machine => {
    const capabilities = state.tuiMachines[machine.key];
    const loading = state.tuiLoadingKeys.has(machine.key);
    const checking = state.tuiCheckingKeys.has(machine.key);
    const mutating = [...state.tuiInstallingKeys, ...state.tuiUpgradingKeys]
      .some(key => key.startsWith(`${machine.key}:`))
      || state.tuiAdapterImporting
      || state.tuiAdapterBusyKeys.size > 0;
    const scopeValue = machine.serverId == null ? "" : String(machine.serverId);
    let body;
    if (loading && !capabilities) {
      body = `<div class="drawer__empty tui-machine__state">${escapeHtml(tr("tui.detecting"))}</div>`;
    } else if (capabilities?.error) {
      body = `<div class="tui-machine__error">${escapeHtml(capabilities.error)}</div>`;
    } else if (capabilities?.tools?.length) {
      body = capabilities.tools.map(tool => {
        const installKey = `${machine.key}:${tool.toolKey}`;
        const installing = state.tuiInstallingKeys.has(installKey);
        const upgrading = state.tuiUpgradingKeys.has(installKey);
        const adapterBusy = state.tuiAdapterBusyKeys.has(tool.toolKey);
        const busy = installing || upgrading || adapterBusy || checking || state.tuiAdapterImporting;
        const supported = tool.supported !== false;
        const version = tool.version || tr("tui.versionUnknown");
        const status = !supported
          ? tr("tui.unsupported")
          : tool.installed ? tr("tui.installed") : tr("tui.notInstalled");
        const installTitle = !supported
          ? (tool.supportError || tr("tui.unsupported"))
          : tool.installAvailable
            ? tr("tui.installWith", { manager: tool.installManager, package: tool.installPackage })
            : tool.installManager
              ? tr("tui.managerMissing", { manager: tool.installManager })
              : tr("tui.manualInstallHint");
        const upgradeTitle = tool.installAvailable
          ? tr("tui.upgradeWith", { manager: tool.installManager, package: tool.installPackage })
          : tool.installManager
            ? tr("tui.managerMissing", { manager: tool.installManager })
            : tr("tui.manualInstallHint");
        const adapterSource = tr(tool.adapterSource === "local"
          ? "tui.adapterSourceLocal"
          : "tui.adapterSourceBundled");
        const adapterMeta = tr("tui.adapterMeta", {
          version: tool.adapterVersion || tr("tui.versionUnknown"),
          source: adapterSource,
        });
        let updateStatus = "";
        if (tool.installed) {
          if (checking) {
            updateStatus = `<small class="tui-row__update is-checking">${escapeHtml(tr("tui.checkingUpdate"))}</small>`;
          } else if (!tool.updateChecked) {
            updateStatus = `<small class="tui-row__update">${escapeHtml(tr("tui.updateUnchecked"))}</small>`;
          } else if (tool.updateCheckError) {
            updateStatus = `<small class="tui-row__update is-error" title="${escapeHtml(tool.updateCheckError)}">${escapeHtml(tr("tui.updateCheckFailed"))}</small>`;
          } else if (tool.updateAvailable) {
            updateStatus = `<small class="tui-row__update is-update">${escapeHtml(tr("tui.updateAvailable", { version: tool.latestVersion || tr("tui.versionUnknown") }))}</small>`;
          } else {
            updateStatus = `<small class="tui-row__update is-current">${escapeHtml(tr("tui.upToDate", { version: tool.latestVersion || version }))}</small>`;
          }
        }
        return `<div class="tui-row" data-tui-tool="${escapeHtml(tool.toolKey)}">
          <span class="tui-row__icon">${toolIcon(tool.toolKey)}</span>
          <span class="tui-row__main">
            <strong>${escapeHtml(tool.toolName)}</strong>
            <small class="${tool.installed ? "is-installed" : "is-missing"}" title="${escapeHtml(tool.supportError || "")}">${escapeHtml(status)}${tool.installed ? ` · ${escapeHtml(version)}` : ""}</small>
            <small class="tui-row__adapter">${escapeHtml(adapterMeta)}</small>
            ${updateStatus}
          </span>
          <span class="tui-row__actions">
            ${machine.serverId == null && tool.adapterUpdateAvailable ? `<button class="tui-row__adapter-action" data-tui-adapter-activate
              title="${escapeHtml(tr("tui.adapterActivateTitle", { version: tool.adapterNewestVersion }))}" ${busy ? "disabled" : ""}>
              ${escapeHtml(tr(adapterBusy ? "tui.adapterWorking" : "tui.adapterActivate", { version: tool.adapterNewestVersion }))}
            </button>` : ""}
            ${machine.serverId == null && tool.adapterRollbackVersion ? `<button class="tui-row__adapter-action" data-tui-adapter-rollback
              title="${escapeHtml(tr("tui.adapterRollbackTitle", { version: tool.adapterRollbackVersion }))}" ${busy ? "disabled" : ""}>
              ${escapeHtml(tr(adapterBusy ? "tui.adapterWorking" : "tui.adapterRollback", { version: tool.adapterRollbackVersion }))}
            </button>` : ""}
            ${tool.installed && tool.updateAvailable ? `<button class="tui-row__upgrade" data-tui-upgrade data-server-id="${scopeValue}"
              title="${escapeHtml(upgradeTitle)}" ${!tool.installAvailable || busy ? "disabled" : ""}>
              ${escapeHtml(tr(upgrading ? "tui.upgrading" : "tui.upgrade"))}
            </button>` : ""}
            ${!tool.installed ? `<button class="tui-row__install" data-tui-install data-server-id="${scopeValue}"
              title="${escapeHtml(installTitle)}" ${!supported || !tool.installAvailable || busy ? "disabled" : ""}>
              ${escapeHtml(tr(installing ? "tui.installing" : tool.installAvailable ? "tui.install" : "tui.manualInstall"))}
            </button>` : ""}
            <label class="tui-row__toggle" title="${escapeHtml(!supported ? (tool.supportError || tr("tui.unsupported")) : tr(tool.installed ? "tui.enableTitle" : "tui.enableNeedsInstall"))}">
              <input type="checkbox" data-tui-toggle data-server-id="${scopeValue}"
                     ${tool.enabled ? "checked" : ""} ${!supported || !tool.installed || busy ? "disabled" : ""} />
              <span>${escapeHtml(tr("tui.enabled"))}</span>
            </label>
          </span>
        </div>`;
      }).join("");
    } else {
      body = `<div class="drawer__empty tui-machine__state">${escapeHtml(tr("tui.notDetected"))}</div>`;
    }
    return `<section class="tui-machine" data-tui-machine="${machine.key}">
      <div class="tui-machine__head">
        <div class="tui-machine__identity">
          ${machineIdentityIconsHtml(machine.source, machine.osFamily, "tui-machine__icons")}
          <div class="tui-machine__identity-copy">
            <strong>${escapeHtml(machine.label)}</strong>
            <small>${escapeHtml(tr(machine.source === "local" ? "tui.sourceLocal" : "tui.sourceRemote"))}</small>
          </div>
        </div>
        <div class="tui-machine__actions">
          <button class="tui-machine__check" data-tui-check-updates data-server-id="${scopeValue}"
                  ${loading || checking || mutating || !capabilities?.tools?.length ? "disabled" : ""}>
            ${escapeHtml(tr(checking ? "tui.checkingUpdatesShort" : "tui.checkUpdates"))}
          </button>
          <button class="tui-machine__refresh" data-tui-refresh data-server-id="${scopeValue}"
                  ${loading || checking || mutating ? "disabled" : ""}>
            ${escapeHtml(tr(loading ? "tui.detectingShort" : "tui.refresh"))}
          </button>
        </div>
      </div>
      <div class="tui-machine__tools">${body}</div>
    </section>`;
  }).join("")}`;
}

async function ignoreProjectTree(project) {
  if (!project) return false;
  if (!HAS_TAURI) {
    setStatus(tr("status.demoIgnoreUnavailable"));
    return false;
  }
  try {
    await invoke("add_project_ignore", {
      source: project.source === "remote" ? "remote" : "local",
      remoteServerId: project.source === "remote" ? project.remoteServerId : null,
      path: project.path,
    });
    closeEntryMenu();
    await reload();
    if (!drawer.hidden && state.settingsView === "ignores") renderDrawerBody();
    setStatus(tr("status.projectIgnored", { path: project.path }));
    return true;
  } catch (error) {
    showActionError(tr("status.projectIgnoreFailed", { err: error }));
    return false;
  }
}

function setServerFormFeedback(form, message, { error = false } = {}) {
  const feedback = form?.querySelector?.("[data-server-form-feedback]");
  const commandInput = form?.querySelector?.('[name="sshCommand"]');
  const commandField = commandInput?.closest?.(".ssh-command-field");
  if (feedback) {
    feedback.textContent = String(message ?? "");
    feedback.classList.toggle("is-error", error);
  }
  if (commandField) commandField.classList.toggle("is-invalid", error);
  if (commandInput) commandInput.setAttribute("aria-invalid", String(error));
}

function resetServerFormFeedback(form) {
  const feedback = form?.querySelector?.("[data-server-form-feedback]");
  const commandInput = form?.querySelector?.('[name="sshCommand"]');
  const commandField = commandInput?.closest?.(".ssh-command-field");
  if (feedback) {
    feedback.innerHTML = tr("server.hintKeyless");
    feedback.classList.remove("is-error");
  }
  if (commandField) commandField.classList.remove("is-invalid");
  if (commandInput) commandInput.setAttribute("aria-invalid", "false");
}

function setServerFormBusy(form, busy) {
  const submitButton = form?.querySelector?.('[type="submit"]');
  if (!submitButton) return;
  submitButton.disabled = busy;
  submitButton.classList.toggle("is-working", busy);
  submitButton.setAttribute("aria-busy", String(busy));
  submitButton.textContent = tr(busy ? "form.submit.connecting" : "form.submit.addScan");
}

function remoteServerScanPresentation(server) {
  const iso = server?.lastScannedAt;
  const date = iso ? new Date(iso) : null;
  if (!date || Number.isNaN(date.getTime())) {
    return {
      text: tr("server.neverScanned"),
      title: tr("server.neverScannedTitle"),
    };
  }
  const relative = relTime(iso);
  const compact = relative === "now" ? tr("server.justNow") : relative;
  const exact = date.toLocaleString(currentLocaleTag(), {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  return {
    text: tr("server.lastScanned", { time: compact }),
    title: tr("server.lastScannedTitle", { time: exact }),
  };
}

function syncRemoteScanTime(row, server) {
  const element = row?.querySelector?.("[data-server-last-scan]");
  if (!element) return;
  const presentation = remoteServerScanPresentation(server);
  element.textContent = presentation.text;
  element.title = presentation.title;
}

function syncRemoteScanRow(serverId) {
  if (drawer.hidden || state.settingsView !== "remote") return;
  const row = drawerBody.querySelector(`.server-row[data-id="${Number(serverId)}"]`);
  if (!row) return;
  const scanning = state.remoteScanIds.has(Number(serverId));
  row.classList.toggle("is-scanning", scanning);
  const scanButton = row.querySelector("[data-server-scan]");
  if (scanButton) {
    scanButton.disabled = scanning;
    scanButton.textContent = tr(scanning ? "server.scanning" : "server.scan");
    scanButton.title = tr(scanning ? "server.scanningTitle" : "server.scanTitle");
  }
  const deleteButton = row.querySelector("[data-server-del]");
  if (deleteButton) deleteButton.disabled = scanning;
  syncRemoteScanTime(
    row,
    state.remoteServers.find(server => Number(server.id) === Number(serverId)),
  );
}

async function scanRemoteServerInBackground(serverId, { initial = false, probe = null } = {}) {
  const id = Number(serverId);
  const server = state.remoteServers.find(item => Number(item.id) === id);
  const initialName = server?.label || "server";
  if (state.remoteScanIds.has(id)) {
    setStatus(tr("status.remoteScanAlreadyRunning", { name: initialName }));
    return false;
  }

  state.remoteScanIds.add(id);
  syncRemoteScanRow(id);
  setStatus(tr(initial ? "status.serverAddedScanningBackground" : "status.scanningBackground", {
    name: initialName,
  }));
  // Project discovery and installed-TUI detection are independent SSH reads.
  // Start both together so a newly-added server is fully usable after its
  // first background scan instead of waiting for the user to visit TUI
  // settings or select a remote project.
  void refreshTuiMachine(id, { force: true, deferRender: true });

  try {
    const count = await invoke("scan_remote_server", { serverId: id });
    await reload();
    const currentName = state.remoteServers.find(item => Number(item.id) === id)?.label || initialName;
    if (initial && probe?.tmuxAvailable === false) {
      showActionError(tr("status.serverAddedTmuxMissing", { name: currentName }));
    } else {
      const scanResult = tr("status.scanResult", { name: currentName, count });
      const tmuxStatus = initial && probe?.tmuxVersion
        ? ` · ${tr("status.tmuxReady", { version: probe.tmuxVersion })}`
        : "";
      setStatus(`${scanResult}${tmuxStatus}`);
    }
    return true;
  } catch (error) {
    if (initial) {
      showActionError(tr("status.serverAddedScanFailed", { name: initialName, err: error }));
    } else {
      showActionError(error);
    }
    return false;
  } finally {
    state.remoteScanIds.delete(id);
    syncRemoteScanRow(id);
  }
}

function renderRemoteViewBody() {
  const list = state.remoteServers.length
    ? `<div class="drawer__section">
        <div class="drawer__section__label">${escapeHtml(tr("settings.configuredServers"))}</div>
        ${state.remoteServers.map(s => {
          const scanning = state.remoteScanIds.has(Number(s.id));
          const scanTime = remoteServerScanPresentation(s);
          return `
          <div class="server-row ${scanning ? "is-scanning" : ""}" data-id="${s.id}">
            <div class="server-row__main">
              <div class="server-row__identity">
                ${machineIdentityIconsHtml("remote", s.osFamily, "server-row__icons")}
                <div class="server-row__identity-copy">
                  <input class="server-row__label" data-server-rename value="${escapeHtml(s.label)}"
                         aria-label="${escapeHtml(tr("server.nameAria"))}"
                         title="${escapeHtml(tr("server.nameTitle"))}" spellcheck="false" />
                  <span class="server-row__conn">${escapeHtml(s.user)}@${escapeHtml(s.host)}:${s.port}</span>
                </div>
              </div>
              <span class="server-row__scan-time" data-server-last-scan
                    title="${escapeHtml(scanTime.title)}">${escapeHtml(scanTime.text)}</span>
            </div>
            <div class="server-row__actions">
              <button class="server-row__scan" data-server-scan
                      title="${escapeHtml(tr(scanning ? "server.scanningTitle" : "server.scanTitle"))}"
                      ${scanning ? "disabled" : ""}>${escapeHtml(tr(scanning ? "server.scanning" : "server.scan"))}</button>
              <button class="server-row__del" data-server-del title="${escapeHtml(tr("common.delete"))}"
                      ${scanning ? "disabled" : ""}>✕</button>
            </div>
          </div>`;
        }).join("")}
      </div>`
    : `<div class="drawer__empty">${escapeHtml(tr("settings.noRemote"))}</div>`;
  return list + `
    <form id="serverForm" class="custom-form drawer__form server-form">
      <span class="entry__launch-label">${escapeHtml(tr("settings.addSshServer"))}</span>
      <p class="custom-form__hint" data-server-form-feedback role="status" aria-live="polite">${tr("server.hintKeyless")}</p>
      <label class="ssh-command-field">
        <span class="ssh-command-field__prefix" aria-hidden="true">ssh</span>
        <input class="ssh-command-field__input" name="sshCommand"
               data-i18n-placeholder="form.sshCommandPh" data-i18n-aria="form.sshCommandAria"
               autocomplete="off" autocapitalize="none" spellcheck="false" />
      </label>
      <input class="custom-form__input server-form__label" name="label"
             data-i18n-placeholder="form.labelPh" autocomplete="off" />
      <button class="scan-btn" type="submit">${escapeHtml(tr("form.submit.addScan"))}</button>
    </form>`;
}

// Sub-page: user-managed directory trees excluded from the ledger. The
// built-in dot-directory rule is intentionally not removable; manual rules
// preserve their source/server scope and can be restored from this list.
function renderIgnoresViewBody() {
  const automatic = `
    <div class="ignore-rule-note">
      <strong>${escapeHtml(tr("settings.ignoresAutomaticTitle"))}</strong>
      <span>${escapeHtml(tr("settings.ignoresAutomaticBody"))}</span>
    </div>`;
  if (!state.projectIgnores.length) {
    return `${automatic}<div class="drawer__empty">${escapeHtml(tr("settings.noIgnores"))}</div>`;
  }
  const rows = state.projectIgnores.map(rule => `
    <div class="ignore-row" data-id="${escapeHtml(String(rule.id))}">
      <div class="ignore-row__main">
        ${projectSourceBadgeHtml({
          source: rule.source,
          remoteServerId: rule.remoteServerId,
          path: rule.path,
        }, "ignore-row__source")}
        <span class="ignore-row__path" title="${escapeHtml(rule.path)}">${escapeHtml(rule.path)}</span>
      </div>
      <button class="ignore-row__del" data-ignore-del title="${escapeHtml(tr("settings.restoreIgnoredTitle"))}">
        ${escapeHtml(tr("settings.restoreIgnored"))}
      </button>
    </div>`).join("");
  return `${automatic}
    <div class="drawer__section">
      <div class="drawer__section__label">${escapeHtml(tr("settings.ignoredPaths"))}</div>
      ${rows}
    </div>`;
}

// Sub-page: groups list + "Add group" form.
function renderGroupsViewBody() {
  const list = state.groups.length
    ? `<div class="drawer__section">
        <div class="drawer__section__label">${escapeHtml(tr("settings.groupsLabel"))}</div>
        ${state.groups.map(g => `
          <div class="group-row" data-id="${g.id}">
            <input class="group-row__name" data-group-rename value="${escapeHtml(g.name)}" spellcheck="false" />
            <span class="group-row__count">${escapeHtml(tr("group.memberCount", { count: g.memberCount }))}</span>
            <button class="group-row__del" data-group-del title="${escapeHtml(tr("group.deleteTitle"))}">✕</button>
          </div>`).join("")}
      </div>`
    : `<div class="drawer__empty">${escapeHtml(tr("settings.noGroups"))}</div>`;
  return list + `
    <form id="groupForm" class="custom-form drawer__form">
      <span class="entry__launch-label">${escapeHtml(tr("settings.addGroup"))}</span>
      <input class="custom-form__input custom-form__input--wide" name="name"
             data-i18n-placeholder="form.groupNamePh" autocomplete="off" />
      <button class="scan-btn" type="submit">${escapeHtml(tr("form.submit.add"))}</button>
    </form>`;
}

// Sub-page: browser-based development tools. These endpoints are separate
// from TUI adapters because they do not execute a local binary; opening one
// creates a sandboxed web tab after both backend and frontend URL checks.
function renderWebDevelopmentViewBody() {
  if (!state.webDevelopmentToolsLoaded) {
    return `<div class="drawer__empty">${escapeHtml(tr("common.loading"))}</div>`;
  }
  const error = state.webDevelopmentToolsError
    ? `<div class="web-development__error" role="alert">${escapeHtml(tr("settings.webDevelopment.loadFailed", { error: state.webDevelopmentToolsError }))}</div>`
    : "";
  const rows = state.webDevelopmentTools.length
    ? `<div class="drawer__section">
        <div class="drawer__section__label">${escapeHtml(tr("settings.webDevelopment.configured"))}</div>
        ${state.webDevelopmentTools.map(tool => `
          <form class="web-tool-row" data-web-development-form data-web-development-id="${escapeHtml(String(tool.id))}">
            <label class="web-tool-row__toggle">
              <input type="checkbox" data-web-development-toggle ${tool.enabled ? "checked" : ""} />
              <span class="web-tool-row__mark" aria-hidden="true">WB</span>
              <span>${escapeHtml(tool.label)}</span>
            </label>
            <label class="web-tool-row__field">
              <span>${escapeHtml(tr("settings.webDevelopment.name"))}</span>
              <input class="custom-form__input" name="label" value="${escapeHtml(tool.label)}"
                     maxlength="128" required autocomplete="off" />
            </label>
            <label class="web-tool-row__field web-tool-row__field--url">
              <span>${escapeHtml(tr("settings.webDevelopment.connectionUrl"))}</span>
              <input class="custom-form__input" name="connectionUrl" type="url"
                     value="${escapeHtml(tool.connectionUrl)}" required autocomplete="off" spellcheck="false" />
            </label>
            <div class="web-tool-row__actions">
              <button type="button" data-web-development-open>${escapeHtml(tr("settings.webDevelopment.open"))}</button>
              <button type="submit">${escapeHtml(tr("settings.webDevelopment.save"))}</button>
              <button type="button" class="web-tool-row__delete" data-web-development-delete>${escapeHtml(tr("common.delete"))}</button>
            </div>
          </form>`).join("")}
      </div>`
    : `<div class="drawer__empty">${escapeHtml(tr("settings.webDevelopment.none"))}</div>`;
  return `${error}
    <div class="web-development__intro">${escapeHtml(tr("settings.webDevelopment.intro"))}</div>
    ${rows}
    <form id="webDevelopmentNewForm" class="custom-form drawer__form" data-web-development-form data-web-development-id="">
      <span class="entry__launch-label">${escapeHtml(tr("settings.webDevelopment.add"))}</span>
      <input class="custom-form__input" name="label" maxlength="128" required
             placeholder="${escapeHtml(tr("settings.webDevelopment.namePlaceholder"))}" autocomplete="off" />
      <input class="custom-form__input custom-form__input--wide" name="connectionUrl" type="url" required
             placeholder="${escapeHtml(tr("settings.webDevelopment.urlPlaceholder"))}"
             autocomplete="off" spellcheck="false" />
      <button class="scan-btn" type="submit">${escapeHtml(tr("form.submit.add"))}</button>
      <span class="web-development__hint">${escapeHtml(tr("settings.webDevelopment.urlHint"))}</span>
    </form>`;
}

// Sub-page: openers list + "Add custom opener" form.
function renderOpenersViewBody() {
  const list = (!state.openerPrefs || !state.openerPrefs.length)
    ? `<div class="drawer__empty">${escapeHtml(tr("settings.noOpeners"))}</div>`
    : `<div class="drawer__section">
        <div class="drawer__section__label">${escapeHtml(tr("settings.openersLabel"))}</div>
        ${state.openerPrefs.map(o => `
          <div class="opener-row" data-id="${escapeHtml(String(o.id))}">
            <label class="opener-row__toggle">
              <input type="checkbox" data-opener-toggle ${o.enabled ? "checked" : ""} />
              <span class="opener-row__dot ${o.type === 'custom' ? 'opener-row__dot--custom' : ''}"></span>
              <span class="opener-row__label">${escapeHtml(o.label)}</span>
              <span class="opener-row__kind">${escapeHtml(tr(o.type === "custom" ? "opener.custom" : "launch.openersBuiltIn"))}</span>
            </label>
            <input class="opener-row__cmd" data-opener-cmd value="${escapeHtml(o.command)}" spellcheck="false" />
            ${o.type === "custom"
              ? `<button class="opener-row__del" data-opener-del title="${escapeHtml(tr("common.delete"))}">✕</button>`
              : `<span class="opener-row__builtin-tag">${escapeHtml(tr("launch.openersBuiltIn"))}</span>`}
          </div>`).join("")}
      </div>`;
  return list + `
    <form id="customForm" class="custom-form drawer__form">
      <span class="entry__launch-label">${escapeHtml(tr("settings.addCustomOpener"))}</span>
      <input class="custom-form__input" name="label" data-i18n-placeholder="form.openerLabelPh" autocomplete="off" />
      <input class="custom-form__input custom-form__input--wide" name="command"
             data-i18n-placeholder="form.openerCmdPh"
             autocomplete="off" spellcheck="false" />
      <button class="scan-btn" type="submit">${escapeHtml(tr("form.submit.add"))}</button>
    </form>`;
}

// Sub-page: interface language picker. The selected language is expressed by
// the whole row's highlight instead of a checkbox/checkmark. Native buttons
// keep Enter/Space activation, while radio semantics expose the exclusive
// choice to assistive technology.
function renderLanguageViewBody() {
  const cur = currentLang();
  const rows = [
    { code: "zh", label: tr("settings.language.zh") },
    { code: "en", label: tr("settings.language.en") },
  ].map(o => `
    <button class="settings-menu__row lang-row ${o.code === cur ? "is-active" : ""}"
            type="button" data-lang-select="${o.code}" role="radio"
            aria-checked="${o.code === cur}">
      <div class="settings-menu__main">
        <div class="settings-menu__title">${escapeHtml(o.label)}</div>
      </div>
    </button>`).join("");
  return `<div class="drawer__section lang-picker" role="radiogroup"
               aria-label="${escapeHtml(tr("lang.rowTitle"))}">
    <div class="drawer__section__label" aria-hidden="true">${escapeHtml(tr("lang.rowTitle"))}</div>
    ${rows}
  </div>`;
}

async function refreshSessionCleanup({ force = false } = {}) {
  if (state.sessionCleanupLoading) return;
  if (state.sessionCleanup && !force && !state.sessionCleanupStale) {
    if (HAS_TAURI) {
      try {
        state.sessionCleanupTrash = await invoke("list_session_trash");
        if (!drawer.hidden && state.settingsView === "cleanup") renderDrawerBody();
      } catch (error) {
        showActionError(tr("cleanup.analysisFailed", { err: error }));
      }
    }
    return;
  }
  state.sessionCleanupLoading = true;
  if (!drawer.hidden && state.settingsView === "cleanup") renderDrawerBody();
  try {
    if (HAS_TAURI) {
      const [analysis, trash] = await Promise.all([
        invoke("analyze_session_cleanup"),
        invoke("list_session_trash"),
      ]);
      state.sessionCleanup = analysis;
      state.sessionCleanupTrash = trash;
      state.sessionCleanupStale = false;
    } else {
      state.sessionCleanup = {
        scannedAt: new Date().toISOString(),
        supportedTools: ["codex", "claude"],
        candidates: [
          { key:"codex:active:demo-child", toolKey:"codex", sessionId:"demo-child", parentSessionId:"demo-parent", title:"review generated patch", cliVersion:"0.139.0", agentKind:"guardian", classification:"likely", reasons:["guardianDelivered"], protections:[], ageDays:48, sizeBytes:32100, userTurns:1, toolCalls:2, canClean:true },
          { key:"claude:active:demo-short", toolKey:"claude", sessionId:"demo-short", title:"quick question", agentKind:"root", classification:"possible", reasons:["shortSingleTurn"], protections:["latestForProject"], ageDays:38, sizeBytes:8400, userTurns:1, toolCalls:0, canClean:false },
        ],
      };
      state.sessionCleanupTrash = [];
      state.sessionCleanupStale = false;
    }
    state.sessionCleanupSelected = new Set(
      [...state.sessionCleanupSelected].filter(key =>
        state.sessionCleanup.candidates.some(item => item.key === key && item.canClean)),
    );
  } catch (error) {
    showActionError(tr("cleanup.analysisFailed", { err: error }));
  } finally {
    state.sessionCleanupLoading = false;
    if (!drawer.hidden && state.settingsView === "cleanup") renderDrawerBody();
  }
}

function cleanupReasonText(key) {
  const translated = tr(`cleanup.reason.${key}`);
  return translated === `cleanup.reason.${key}` ? key : translated;
}

function renderSessionCleanupViewBody() {
  if (state.sessionCleanupLoading && !state.sessionCleanup) {
    return `<div class="drawer__empty cleanup-loading">${escapeHtml(tr("cleanup.analyzing"))}</div>`;
  }
  const candidates = state.sessionCleanup?.candidates || [];
  const actionable = candidates.filter(item => item.classification !== "keep");
  const likely = actionable.filter(item => item.classification === "likely").length;
  const possible = actionable.filter(item => item.classification === "possible").length;
  const protectedCount = actionable.filter(item => !item.canClean).length;
  const selectedSize = actionable
    .filter(item => state.sessionCleanupSelected.has(item.key))
    .reduce((sum, item) => sum + Number(item.sizeBytes || 0), 0);
  const rows = actionable.map(item => {
    const checked = state.sessionCleanupSelected.has(item.key);
    const reasonText = item.reasons.map(cleanupReasonText).join(" · ");
    const protectionText = item.protections.map(cleanupReasonText).join(" · ");
    const parent = item.parentSessionId ? tr("cleanup.childAgent") : tr("cleanup.rootSession");
    return `<label class="cleanup-row ${item.canClean ? "" : "is-protected"}" data-cleanup-key="${escapeHtml(item.key)}">
      <input type="checkbox" data-cleanup-select ${checked ? "checked" : ""} ${item.canClean ? "" : "disabled"} />
      <span class="cleanup-row__main">
        <span class="cleanup-row__title">${escapeHtml(item.title || item.sessionId.slice(0, 12))}</span>
        <span class="cleanup-row__meta">${escapeHtml(item.toolKey)} · ${escapeHtml(parent)} · ${escapeHtml(tr("cleanup.age", { days: item.ageDays }))} · ${escapeHtml(formatBytes(item.sizeBytes))}</span>
        <span class="cleanup-row__reason">${escapeHtml(reasonText)}${protectionText ? ` · ${escapeHtml(tr("cleanup.protected", { reason: protectionText }))}` : ""}</span>
      </span>
      <span class="cleanup-row__class cleanup-row__class--${item.classification}">${escapeHtml(tr(`cleanup.class.${item.classification}`))}</span>
    </label>`;
  }).join("");
  const trash = state.sessionCleanupTrash.map(batch => `
    <div class="cleanup-trash-row">
      <span>${escapeHtml(new Date(batch.createdAt).toLocaleString(currentLocaleTag()))} · ${escapeHtml(tr("cleanup.sessions", { count: batch.sessionCount }))} · ${escapeHtml(formatBytes(batch.sizeBytes))}</span>
      <button type="button" data-cleanup-restore="${escapeHtml(batch.batchId)}">${escapeHtml(tr("cleanup.restore"))}</button>
    </div>`).join("");
  return `<div class="cleanup-intro">${escapeHtml(tr("cleanup.intro"))}</div>
    <div class="cleanup-summary">
      <span class="cleanup-summary__likely">${escapeHtml(tr("cleanup.likelyCount", { count: likely }))}</span>
      <span>${escapeHtml(tr("cleanup.possibleCount", { count: possible }))}</span>
      <span>${escapeHtml(tr("cleanup.protectedCount", { count: protectedCount }))}</span>
      <button type="button" data-cleanup-refresh ${state.sessionCleanupLoading ? "disabled" : ""}>${escapeHtml(tr(state.sessionCleanupLoading ? "cleanup.analyzing" : "cleanup.reanalyze"))}</button>
    </div>
    <div class="cleanup-actions">
      <button type="button" data-cleanup-select-likely>${escapeHtml(tr("cleanup.selectLikely"))}</button>
      <button type="button" class="cleanup-actions__primary" data-cleanup-run ${state.sessionCleanupSelected.size ? "" : "disabled"}>${escapeHtml(tr("cleanup.moveSelected", { count: state.sessionCleanupSelected.size, size: formatBytes(selectedSize) }))}</button>
    </div>
    <div class="cleanup-list">${rows || `<div class="drawer__empty">${escapeHtml(tr("cleanup.noCandidates"))}</div>`}</div>
    <div class="drawer__section cleanup-trash">
      <div class="drawer__section__label">${escapeHtml(tr("cleanup.recovery"))}</div>
      ${trash || `<div class="drawer__empty">${escapeHtml(tr("cleanup.noRecovery"))}</div>`}
    </div>`;
}

function renderDrawerBody() {
  if (state.openerPrefsError && !state.openerPrefs.length && state.settingsView !== "menu") {
    drawerBody.innerHTML = `<div class="drawer__empty">${tr("settings.prefsUnavailable", { error: escapeHtml(state.openerPrefsError) })}</div>`;
    renderDrawerHead();
    renderDrawerFoot();
    if (!drawer.hidden && !drawer.contains(document.activeElement)) {
      focusDialogStart(drawer, "#drawerBackBtn");
    }
    return;
  }
  if (state.settingsView === "menu") {
    drawerBody.innerHTML = renderSettingsMenuBody();
  } else {
    const view = SETTINGS_VIEWS[state.settingsView];
    drawerBody.innerHTML = view ? view.body() : "";
  }
  renderDrawerHead();
  // Startup reconciliation and language changes can repaint an already-open
  // drawer. If that removed the focused row, restore focus inside the dialog
  // instead of silently dropping keyboard users back onto <body>.
  if (!drawer.hidden && !drawer.contains(document.activeElement)) {
    const preferred = state.settingsView === "menu"
      ? "[data-settings-view]"
      : "#drawerBackBtn";
    focusDialogStart(drawer, preferred);
  }
}

/* One-shot delegated listeners attached at module init. Replaces the
   per-row bindings the original renderDrawerBody set up on every paint. */
const _debouncedCmdTimers = new Map();
function wireDrawerDelegation() {
  drawerBody.addEventListener("change", async (e) => {
    const cb = e.target;
    if (cb?.matches?.("[data-cleanup-select]")) {
      const key = cb.closest("[data-cleanup-key]")?.dataset.cleanupKey;
      if (key) {
        if (cb.checked) state.sessionCleanupSelected.add(key);
        else state.sessionCleanupSelected.delete(key);
        renderDrawerBody();
      }
      return;
    }
    if (cb?.matches?.("[data-tui-toggle]")) {
      const row = cb.closest("[data-tui-tool]");
      const toolKey = row?.dataset.tuiTool;
      const serverId = cb.dataset.serverId === "" ? null : Number(cb.dataset.serverId);
      const key = tuiMachineKey(serverId);
      const previous = !cb.checked;
      cb.disabled = true;
      try {
        if (HAS_TAURI) {
          state.tuiMachines[key] = await invoke("set_tui_enabled", {
            serverId, toolKey, enabled: cb.checked,
          });
        } else {
          const tool = state.tuiMachines[key]?.tools?.find(item => item.toolKey === toolKey);
          if (tool) {
            tool.adapterEnabled = cb.checked;
            tool.enabled = tool.installed && cb.checked;
          }
        }
        setStatus(tr(cb.checked ? "status.tuiEnabled" : "status.tuiDisabled", { tool: toolKey }));
        renderDrawerBody();
        applyFilters();
        renderSelectedLaunchPanel();
      } catch (error) {
        cb.checked = previous;
        cb.disabled = false;
        showActionError(tr("status.tuiToggleFailed", { err: error }));
      }
      return;
    }
    if (cb?.matches?.("[data-web-development-toggle]")) {
      const row = cb.closest("[data-web-development-id]");
      const id = Number(row?.dataset.webDevelopmentId);
      const tool = state.webDevelopmentTools.find(item => Number(item.id) === id);
      if (!tool) return;
      const previous = tool.enabled;
      const checked = cb.checked;
      tool.enabled = checked;
      renderSelectedLaunchPanel();
      await settingsMutationQueue.run(`web-development-enabled:${id}`, async () => {
        try {
          if (HAS_TAURI) {
            await invoke("set_web_development_tool_enabled", { toolId: id, enabled: checked });
          }
          setStatus(tr("status.webDevelopmentEnabled", { name: tool.label }));
        } catch (error) {
          tool.enabled = previous;
          cb.checked = previous;
          renderSelectedLaunchPanel();
          showActionError(tr("status.webDevelopmentSaveFailed", { err: error }));
        }
      });
      return;
    }
    if (!cb?.matches?.("[data-opener-toggle]")) return;
    const id = cb.closest(".opener-row").dataset.id;
    const checked = cb.checked;
    const opener = state.openerPrefs.find(x => String(x.id) === String(id));
    const previous = opener?.enabled ?? !checked;
    await settingsMutationQueue.run(`opener-enabled:${id}`, async () => {
      try {
        if (HAS_TAURI) {
          await invoke("set_opener_enabled", { prefId: Number(id), enabled: checked });
        }
        if (opener) opener.enabled = checked;
        renderLedger();
      } catch (error) {
        cb.checked = previous;
        showActionError(error);
      }
    });
  });
  drawerBody.addEventListener("input", (e) => {
    const inp = e.target;
    if (inp?.matches?.("[data-tui-adapter-path]")) {
      state.tuiAdapterManifestPath = inp.value;
      return;
    }
    if (inp?.matches?.('#serverForm [name="sshCommand"]')) {
      resetServerFormFeedback(inp.form);
      return;
    }
    if (inp?.matches?.("[data-opener-cmd]")) {
      const id = inp.closest(".opener-row").dataset.id;
      clearTimeout(_debouncedCmdTimers.get(id));
      _debouncedCmdTimers.set(id, setTimeout(async () => {
        const draft = inp.value;
        await settingsMutationQueue.run(`opener-command:${id}`, async () => {
          try {
            if (HAS_TAURI) {
              await invoke("set_opener_command", { prefId: Number(id), command: draft });
            }
            const o = state.openerPrefs.find(x => String(x.id) === String(id));
            if (o) o.command = draft;
            inp.classList.remove("is-unsaved");
            renderLedger();
          } catch (error) {
            inp.classList.add("is-unsaved");
            showActionError(error);
          }
        });
      }, 350));
      return;
    }
    if (inp?.matches?.("[data-group-rename]")) {
      const id = inp.closest(".group-row").dataset.id;
      clearTimeout(_debouncedCmdTimers.get(`g${id}`));
      _debouncedCmdTimers.set(`g${id}`, setTimeout(async () => {
        const draft = inp.value;
        await settingsMutationQueue.run(`group-rename:${id}`, async () => {
          try {
            if (HAS_TAURI) {
              await invoke("rename_group", { groupId: Number(id), name: draft });
            }
            const g = state.groups.find(x => String(x.id) === String(id));
            if (g) g.name = draft;
            inp.classList.remove("is-unsaved");
            applyFilters();
          } catch (error) {
            inp.classList.add("is-unsaved");
            showActionError(error);
          }
        });
      }, 350));
      return;
    }
    if (inp?.matches?.("[data-server-rename]")) {
      const id = Number(inp.closest(".server-row").dataset.id);
      clearTimeout(_debouncedCmdTimers.get(`s${id}`));
      _debouncedCmdTimers.set(`s${id}`, setTimeout(async () => {
        const draft = inp.value;
        await settingsMutationQueue.run(`server-rename:${id}`, async () => {
          try {
            if (HAS_TAURI) {
              await invoke("rename_remote_server", { serverId: id, label: draft });
            }
            const server = state.remoteServers.find(item => Number(item.id) === id);
            if (server) server.label = draft.trim();
            inp.classList.remove("is-unsaved");
            applyFilters();
            renderSelectedLaunchPanel();
          } catch (error) {
            inp.classList.add("is-unsaved");
            showActionError(error);
          }
        });
      }, 350));
    }
  });
  drawerBody.addEventListener("click", async (e) => {
    if (e.target.closest("[data-cleanup-refresh]")) {
      void refreshSessionCleanup({ force: true });
      return;
    }
    if (e.target.closest("[data-cleanup-select-likely]")) {
      const likely = (state.sessionCleanup?.candidates || [])
        .filter(item => item.classification === "likely" && item.canClean)
        .map(item => item.key);
      state.sessionCleanupSelected = new Set(likely);
      renderDrawerBody();
      return;
    }
    if (e.target.closest("[data-cleanup-run]")) {
      const keys = [...state.sessionCleanupSelected];
      if (!keys.length || !window.confirm(tr("cleanup.confirm", { count: keys.length }))) return;
      try {
        if (HAS_TAURI) {
          const snapshotId = state.sessionCleanup?.snapshotId;
          if (!snapshotId) throw new Error("session analysis is stale; analyze again");
          await invoke("quarantine_session_candidates", { snapshotId, keys });
        }
        state.sessionCleanup.candidates = state.sessionCleanup.candidates.filter(item => !state.sessionCleanupSelected.has(item.key));
        state.sessionCleanupStale = true;
        state.sessionCleanupSelected.clear();
        setStatus(tr("cleanup.moved", { count: keys.length }));
        if (HAS_TAURI) state.sessionCleanupTrash = await invoke("list_session_trash");
        renderDrawerBody();
        if (HAS_TAURI) void reload();
      } catch (error) {
        showActionError(tr("cleanup.moveFailed", { err: error }));
      }
      return;
    }
    const restore = e.target.closest("[data-cleanup-restore]");
    if (restore) {
      if (!window.confirm(tr("cleanup.restoreConfirm"))) return;
      try {
        const count = HAS_TAURI
          ? await invoke("restore_session_trash", { batchId: restore.dataset.cleanupRestore })
          : 1;
        state.sessionCleanupStale = true;
        setStatus(tr("cleanup.restored", { count }));
        if (HAS_TAURI) state.sessionCleanupTrash = await invoke("list_session_trash");
        renderDrawerBody();
        if (HAS_TAURI) void reload();
      } catch (error) {
        showActionError(tr("cleanup.restoreFailed", { err: error }));
      }
      return;
    }
    const adapterActivate = e.target.closest("[data-tui-adapter-activate]");
    if (adapterActivate) {
      const toolKey = adapterActivate.closest("[data-tui-tool]")?.dataset.tuiTool;
      void mutateLocalTuiAdapter(toolKey, "activate_tui_adapter_update");
      return;
    }
    const adapterRollback = e.target.closest("[data-tui-adapter-rollback]");
    if (adapterRollback) {
      const toolKey = adapterRollback.closest("[data-tui-tool]")?.dataset.tuiTool;
      void mutateLocalTuiAdapter(toolKey, "rollback_tui_adapter");
      return;
    }
    const tuiCheckUpdates = e.target.closest("[data-tui-check-updates]");
    if (tuiCheckUpdates) {
      const serverId = tuiCheckUpdates.dataset.serverId === "" ? null : Number(tuiCheckUpdates.dataset.serverId);
      void checkTuiMachineUpdates(serverId);
      return;
    }
    const tuiRefresh = e.target.closest("[data-tui-refresh]");
    if (tuiRefresh) {
      const serverId = tuiRefresh.dataset.serverId === "" ? null : Number(tuiRefresh.dataset.serverId);
      void refreshTuiMachine(serverId, { force: true });
      return;
    }
    const tuiInstall = e.target.closest("[data-tui-install]");
    if (tuiInstall) {
      const row = tuiInstall.closest("[data-tui-tool]");
      const toolKey = row?.dataset.tuiTool;
      const serverId = tuiInstall.dataset.serverId === "" ? null : Number(tuiInstall.dataset.serverId);
      const machineKey = tuiMachineKey(serverId);
      const machine = state.tuiMachines[machineKey];
      const tool = machine?.tools?.find(item => item.toolKey === toolKey);
      if (!tool) return;
      if (!window.confirm(tr("tui.installConfirm", {
        tool: tool.toolName,
        package: tool.installPackage,
        manager: tool.installManager,
        machine: machine.label || tr("tui.localMachine"),
      }))) return;
      const installKey = `${machineKey}:${toolKey}`;
      state.tuiInstallingKeys.add(installKey);
      renderDrawerBody();
      setStatus(tr("status.tuiInstalling", { tool: tool.toolName, machine: machine.label }));
      try {
        if (HAS_TAURI) {
          state.tuiMachines[machineKey] = await invoke("install_tui", { serverId, toolKey });
        } else {
          tool.installed = true;
          tool.adapterEnabled = true;
          tool.enabled = true;
          tool.version = "demo";
        }
        setStatus(tr("status.tuiInstalled", { tool: tool.toolName, machine: machine.label }));
      } catch (error) {
        showActionError(tr("status.tuiInstallFailed", { err: error }));
      } finally {
        state.tuiInstallingKeys.delete(installKey);
        renderDrawerBody();
        applyFilters();
        renderSelectedLaunchPanel();
      }
      return;
    }
    const tuiUpgrade = e.target.closest("[data-tui-upgrade]");
    if (tuiUpgrade) {
      const row = tuiUpgrade.closest("[data-tui-tool]");
      const toolKey = row?.dataset.tuiTool;
      const serverId = tuiUpgrade.dataset.serverId === "" ? null : Number(tuiUpgrade.dataset.serverId);
      const machineKey = tuiMachineKey(serverId);
      const machine = state.tuiMachines[machineKey];
      const tool = machine?.tools?.find(item => item.toolKey === toolKey);
      if (!tool?.installed || !tool.updateAvailable) return;
      if (!window.confirm(tr("tui.upgradeConfirm", {
        tool: tool.toolName,
        machine: machine.label || tr("tui.localMachine"),
        version: tool.latestVersion || tr("tui.versionUnknown"),
        package: tool.installPackage,
        manager: tool.installManager,
      }))) return;
      const upgradeKey = `${machineKey}:${toolKey}`;
      state.tuiUpgradingKeys.add(upgradeKey);
      renderDrawerBody();
      setStatus(tr("status.tuiUpgrading", { tool: tool.toolName, machine: machine.label }));
      try {
        if (HAS_TAURI) {
          state.tuiMachines[machineKey] = await invoke("upgrade_tui", { serverId, toolKey });
        } else {
          tool.version = tool.latestVersion || tool.version;
          tool.updateAvailable = false;
        }
        state.tuiCapabilityAt[machineKey] = Date.now();
        setStatus(tr("status.tuiUpgraded", { tool: tool.toolName, machine: machine.label }));
      } catch (error) {
        showActionError(tr("status.tuiUpgradeFailed", { err: error }));
      } finally {
        state.tuiUpgradingKeys.delete(upgradeKey);
        renderDrawerBody();
        applyFilters();
        renderSelectedLaunchPanel();
      }
      return;
    }
    const webDevelopmentOpen = e.target.closest("[data-web-development-open]");
    if (webDevelopmentOpen) {
      const id = Number(webDevelopmentOpen.closest("[data-web-development-id]")?.dataset.webDevelopmentId);
      const tool = state.webDevelopmentTools.find(item => Number(item.id) === id);
      if (!tool) return;
      closeSettings();
      openWebTab(tool.connectionUrl, tool.label);
      return;
    }
    const webDevelopmentDelete = e.target.closest("[data-web-development-delete]");
    if (webDevelopmentDelete) {
      const id = Number(webDevelopmentDelete.closest("[data-web-development-id]")?.dataset.webDevelopmentId);
      const tool = state.webDevelopmentTools.find(item => Number(item.id) === id);
      if (!tool || !window.confirm(tr("settings.webDevelopment.deleteConfirm", { name: tool.label }))) return;
      try {
        if (HAS_TAURI) await invoke("delete_web_development_tool", { toolId: id });
        state.webDevelopmentTools = state.webDevelopmentTools.filter(item => Number(item.id) !== id);
        renderDrawerBody();
        renderSelectedLaunchPanel();
        setStatus(tr("status.webDevelopmentDeleted", { name: tool.label }));
      } catch (error) {
        showActionError(tr("status.webDevelopmentDeleteFailed", { err: error }));
      }
      return;
    }
    const openerBtn = e.target.closest("[data-opener-del]");
    if (openerBtn) {
      const id = openerBtn.closest(".opener-row").dataset.id;
      try {
        if (HAS_TAURI) await invoke("delete_custom_opener", { prefId: Number(id) });
      } catch (error) {
        showActionError(error);
        return;
      }
      await loadOpenerPrefs();
      renderDrawerBody();
      applyFilters();
      return;
    }
    const ignoreBtn = e.target.closest("[data-ignore-del]");
    if (ignoreBtn) {
      const id = Number(ignoreBtn.closest(".ignore-row").dataset.id);
      try {
        if (HAS_TAURI) await invoke("delete_project_ignore", { ignoreId: id });
        await reload();
        renderDrawerBody();
        setStatus(tr("status.projectIgnoreRemoved"));
      } catch (error) {
        showActionError(tr("status.projectIgnoreRemoveFailed", { err: error }));
      }
      return;
    }
    const groupBtn = e.target.closest("[data-group-del]");
    if (groupBtn) {
      const id = groupBtn.closest(".group-row").dataset.id;
      try {
        if (HAS_TAURI) await invoke("delete_group", { groupId: Number(id) });
      } catch (error) {
        showActionError(error);
        return;
      }
      await loadGroups();
      renderDrawerBody();
      return;
    }
    // Remote-server rows: SCAN button re-runs the discover pipeline,
    // delete removes the server (cascades its projects + tool usages).
    const serverScan = e.target.closest("[data-server-scan]");
    if (serverScan) {
      const id = Number(serverScan.closest(".server-row").dataset.id);
      void scanRemoteServerInBackground(id);
      return;
    }
    const serverDel = e.target.closest("[data-server-del]");
    if (serverDel) {
      const id = Number(serverDel.closest(".server-row").dataset.id);
      try {
        if (HAS_TAURI) await invoke("delete_remote_server", { serverId: id });
      } catch (error) {
        showActionError(error);
        return;
      }
      await reload();
      renderDrawerBody();
      return;
    }
    // Language picker: switch immediately and re-render everything.
    const langRow = e.target.closest("[data-lang-select]");
    if (langRow) {
      setLanguage(langRow.dataset.langSelect);
      setSettingsView("menu");
      return;
    }
    // Main-menu row: drill into the corresponding sub-page. Keyboard
    // activation (Enter/Space) is also handled by the role="button"
    // attribute, which dispatches a synthetic click.
    const menuRow = e.target.closest("[data-settings-view]");
    if (menuRow) {
      const target = menuRow.dataset.settingsView;
      if (target && target !== state.settingsView) setSettingsView(target);
      return;
    }
  });
}

// One-shot delegated handler for the drawer header's back button.
// Lives in the header itself (not the body) so it survives body re-renders.
function wireDrawerBackButton() {
  const back = document.getElementById("drawerBackBtn");
  if (!back) return;
  back.addEventListener("click", () => {
    if (state.settingsView !== "menu") setSettingsView("menu");
  });
}

// Single delegated submit handler on the drawer body — survives
// innerHTML rebuilds (each renderDrawerBody call replaces the
// drawerBody contents, so per-form listeners would be lost). Dispatch
// by the closest <form>'s id to the right submit handler.
async function submitDrawerForm(form, e) {
  const fd = new FormData(form);
  if (form.matches("[data-web-development-form]")) {
    const rawId = form.dataset.webDevelopmentId || "";
    const toolId = rawId ? Number(rawId) : null;
    const label = (fd.get("label") || "").toString().trim();
    const rawUrl = (fd.get("connectionUrl") || "").toString().trim();
    const connectionUrl = normalizeWebConnectionUrl(rawUrl);
    if (!label) {
      setStatus(tr("status.webDevelopmentNameRequired"));
      form.querySelector('[name="label"]')?.focus();
      return;
    }
    if (!connectionUrl) {
      setStatus(tr("status.webConnectionInvalid"));
      form.querySelector('[name="connectionUrl"]')?.focus();
      return;
    }
    const existing = toolId == null
      ? null
      : state.webDevelopmentTools.find(item => Number(item.id) === toolId);
    try {
      let saved;
      if (HAS_TAURI) {
        saved = await invoke("upsert_web_development_tool", {
          toolId,
          label,
          connectionUrl,
          enabled: existing?.enabled ?? true,
        });
      } else {
        saved = {
          id: toolId ?? `demo-web-${Date.now()}`,
          label,
          connectionUrl,
          enabled: existing?.enabled ?? true,
          sortOrder: existing?.sortOrder ?? (state.webDevelopmentTools.length + 1) * 10,
        };
      }
      state.webDevelopmentTools = [
        ...state.webDevelopmentTools.filter(item => String(item.id) !== String(saved.id)),
        saved,
      ].sort((left, right) => Number(left.sortOrder) - Number(right.sortOrder));
      if (toolId == null) form.reset();
      renderDrawerBody();
      renderSelectedLaunchPanel();
      setStatus(tr("status.webDevelopmentSaved", { name: saved.label }));
    } catch (error) {
      showActionError(tr("status.webDevelopmentSaveFailed", { err: error }));
    }
    return;
  }
  switch (form.id) {
    case "tuiAdapterForm": {
      const manifestPath = (fd.get("manifestPath") || "").toString().trim();
      state.tuiAdapterManifestPath = manifestPath;
      if (!manifestPath) {
        setStatus(tr("status.adapterPathRequired"));
        form.querySelector('[name="manifestPath"]')?.focus();
        return;
      }
      if (!HAS_TAURI) {
        setStatus(tr("status.adapterDesktopOnly"));
        return;
      }
      if (!window.confirm(tr("tui.adapterImportConfirm", { path: manifestPath }))) return;
      state.tuiAdapterImporting = true;
      renderDrawerBody();
      setStatus(tr("status.adapterImporting"));
      try {
        state.tuiMachines.local = await invoke("install_tui_adapter", { manifestPath });
        state.tuiCapabilityAt.local = Date.now();
        state.tuiAdapterManifestPath = "";
        setStatus(tr("status.adapterImported"));
        refreshRemoteTuiAdaptersAfterRegistryChange();
      } catch (error) {
        showActionError(tr("status.adapterImportFailed", { err: error }));
      } finally {
        state.tuiAdapterImporting = false;
        renderDrawerBody();
        applyFilters();
        renderSelectedLaunchPanel();
      }
      break;
    }
    case "customForm": {
      const label = (fd.get("label") || "").toString().trim();
      const command = (fd.get("command") || "").toString().trim();
      if (!label || !command) { setStatus(tr("status.labelCmdRequired")); return; }
      try {
        if (HAS_TAURI) {
          await invoke("upsert_custom_opener", { label, command, enabled: true });
        }
      } catch (error) {
        showActionError(error);
        return;
      }
      form.reset();
      await loadOpenerPrefs();
      renderDrawerBody();
      renderLedger();
      break;
    }
    case "serverForm": {
      const parsed = parseSshConnectionInput(fd.get("sshCommand"));
      if (!parsed.ok) {
        const errorKey = {
          empty: "status.sshCommandRequired",
          unterminatedQuote: "status.sshUnterminatedQuote",
          missingOptionValue: "status.sshMissingOptionValue",
          duplicateOption: "status.sshDuplicateOption",
          unsupportedOption: "status.sshUnsupportedOption",
          extraArgument: "status.sshExtraArgument",
          destination: "status.sshDestinationRequired",
          port: "status.portRange",
        }[parsed.error] || "status.sshDestinationRequired";
        const message = tr(errorKey, { value: parsed.detail || "" });
        setStatus(message);
        setServerFormFeedback(form, message, { error: true });
        form.querySelector('[name="sshCommand"]')?.focus();
        return;
      }
      const { user, host, port, identityFile } = parsed.value;
      const requestedLabel = (fd.get("label") || "").toString().trim();
      const label = requestedLabel || host;
      if (!HAS_TAURI) {
        const message = tr("status.demoRemoteUnavailable");
        setStatus(message);
        setServerFormFeedback(form, message, { error: true });
        form.reset();
        return;
      }
      // Disable only during the connection probe + save. The potentially long
      // project discovery starts after persistence and continues in the
      // background, so the form and the rest of the window recover promptly.
      setServerFormBusy(form, true);
      try {
        // Pre-check connectivity + passwordless auth BEFORE persisting the
        // server, so a host that can't be reached (or lacks key auth) never
        // lands in the DB as a zombie record. The command returns a friendly,
        // bilingual, actionable message on failure.
        const probingMessage = tr("status.probing", { name: label });
        setStatus(probingMessage);
        setServerFormFeedback(form, probingMessage);
        const probe = await invoke("test_remote_connection", {
          user, host, port: port || null, identityFile: identityFile || null,
        });

        const addingMessage = tr("status.adding", { name: label });
        setStatus(addingMessage);
        setServerFormFeedback(form, addingMessage);
        const server = await invoke("add_remote_server", {
          label, user, host,
          port: port || null,
          identityFile: identityFile || null,
          scanRoots: null,
          osFamily: probe.osFamily || "unknown",
        });
        form.reset();
        publishRemoteServer(server);
        renderDrawerBody();
        void scanRemoteServerInBackground(server.id, { initial: true, probe });
      } catch (e) {
        // Pre-check (or add/scan) failed: show the classified message but
        // keep the form values so the user can fix and retry.
        setServerFormFeedback(form, String(e ?? ""), { error: true });
        showActionError(e);
      } finally {
        setServerFormBusy(form, false);
      }
      break;
    }
    case "groupForm": {
      const name = (fd.get("name") || "").toString().trim();
      if (!name) { setStatus(tr("status.groupNameRequired")); return; }
      try {
        if (HAS_TAURI) await invoke("create_group", { name });
      } catch (error) {
        showActionError(error);
        return;
      }
      form.reset();
      await loadGroups();
      renderDrawerBody();
      renderLedger();
      break;
    }
    default:
      console.warn("submitDrawerForm: unknown form", form.id);
  }
}

// Submit listener attached once at module init. Forms inside the
// drawer body are recreated on every render, but the listener on
// drawerBody itself stays put — clicks bubble from the form to
// drawerBody, where the dispatcher picks them up by form id.
function wireDrawerForms() {
  drawerBody.addEventListener("submit", (e) => {
    const form = e.target.closest("form");
    if (!form || !drawerBody.contains(form)) return;
    e.preventDefault();
    submitDrawerForm(form, e);
  });
}

/* ── filtering ──────────────────────────────────────────── */
// Tool + recency predicate only — does NOT consider group collapse.
// Shared by `applyFilters` (which additionally hides collapsed groups so
// keyboard nav skips them) and `renderLedger` (which does NOT hide
// collapsed groups, so their headers still render with a count). Keeping
// this in one place ensures nav and rendering agree on what "matches the
// active filters" means.
function matchesFilters(p, cutoff, now) {
  return projectMatchesFilters(p, state.tool, cutoff, now);
}
// Collapse key for a project: its group id, or "ungrouped". Always a string
// so it matches collapse keys and backend group_key ("ungrouped" | str(gid)).
function groupKeyOf(p) {
  return projectGroupKey(p, state.assignments);
}

// Sort a bucket (array of projects, mutated in place) by the group's order
// mode: manual (>=1 member has a sortOrders entry) → sortOrder asc with
// missing rows falling to the end (Infinity), ties broken by recency DESC;
// otherwise recency DESC (the server default). Shared by applyFilters (nav
// order) and renderLedger (display order) so they never diverge.
function sortBucket(items) {
  return sortProjects(items, state.sortOrders);
}

function buildLedgerRows(matching, { includeCollapsed = true } = {}) {
  const buckets = new Map();
  for (const project of matching) {
    const key = groupKeyOf(project);
    if (!buckets.has(key)) buckets.set(key, []);
    buckets.get(key).push(project);
  }
  const rows = [];
  for (const group of state.groups) {
    const items = buckets.get(String(group.id));
    if (!items?.length) continue;
    sortBucket(items);
    rows.push({ type: "group", key: String(group.id), name: group.name, count: items.length });
    if (includeCollapsed || !isGroupCollapsed(String(group.id))) {
      rows.push(...items.map(project => ({ type: "project", project })));
    }
  }
  const ungrouped = buckets.get("ungrouped");
  if (ungrouped?.length) {
    sortBucket(ungrouped);
    rows.push({ type: "group", key: "ungrouped", name: tr("group.ungrouped"), count: ungrouped.length });
    if (includeCollapsed || !isGroupCollapsed("ungrouped")) {
      rows.push(...ungrouped.map(project => ({ type: "project", project })));
    }
  }
  return rows;
}

function applyFilters() {
  const cutoff = RECENCY_CUTOFFS[state.recency];
  const now = Date.now();
  const matching = state.all.filter(p => matchesFilters(p, cutoff, now));
  // Build `filtered` in render order: real groups in their defined order,
  // then 未分组; within each group apply sortBucket; skip collapsed groups
  // (the cursor must skip them). This keeps ↑↓ nav aligned with what's
  // visually top-to-bottom in the ledger, including under manual order.
  state.ledgerRows = buildLedgerRows(matching);
  const ordered = state.ledgerRows
    .filter(row => row.type === "project" && !isGroupCollapsed(groupKeyOf(row.project)))
    .map(row => row.project);
  state.filtered = ordered;
  const idx = state.filtered.findIndex(p => p.id === state.selectedId);
  state.cursor = state.filtered.length ? (idx >= 0 ? idx : 0) : -1;
  renderCount();
  renderLedger();
}

async function reload() {
  const query = state.query;
  const request = query.trim()
    ? reloadCoordinator.beginSearch()
    : reloadCoordinator.beginFull();
  try {
    const searching = query.trim().length > 0;
    const [projectsResult, toolsResult, remoteResult, serversResult, ignoresResult] = await Promise.all([
      captureResult(fetchProjects(query)),
      captureResult(fetchTools()),
      captureResult(searching ? searchRemoteProjects(query) : fetchRemoteProjects()),
      captureResult(fetchRemoteServers()),
      captureResult(fetchProjectIgnores()),
    ]);
    if (!reloadCoordinator.isCurrent(request)) return;
    if (!projectsResult.ok) throw projectsResult.error;

    let remoteList;
    if (remoteResult.ok) {
      remoteList = remoteResult.value || [];
      state.staleSources.remote = false;
      if (!searching) state.remoteProjects = remoteList;
    } else {
      remoteList = searching ? [] : state.remoteProjects;
      state.staleSources.remote = true;
    }
    if (toolsResult.ok) {
      state.tools = toolsResult.value || [];
      state.staleSources.tools = false;
    } else {
      state.staleSources.tools = true;
    }
    if (serversResult.ok) {
      state.remoteServers = serversResult.value || [];
      state.remoteServerById = Object.fromEntries(state.remoteServers.map(s => [s.id, s]));
      state.staleSources.servers = false;
      void refreshAllTuiCapabilities({ includeRemote: state.settingsView === "tui" });
    } else {
      state.staleSources.servers = true;
    }
    if (ignoresResult.ok) {
      state.projectIgnores = ignoresResult.value || [];
      state.staleSources.ignores = false;
    } else {
      state.staleSources.ignores = true;
    }
    const projects = searching
      ? [
          ...(projectsResult.value || []).map(project => ({ ...project, source: project.source || "local" })),
          ...remoteList.map(project => ({ ...project, source: "remote" })),
        ]
      : projectsResult.value;
    const publication = buildProjectPublication(
      state.catalog,
      query,
      projects,
      remoteList,
    );
    state.catalog = publication.catalog;
    state.searchResults = publication.searchResults;
    state.all = publication.visibleProjects;
    renderTools();
    renderMeta(state.all);
    applyFilters();
    if (!state.all.find(p => p.id === state.selectedId)) {
      if (state.filtered.length) {
        select(state.filtered[0].id);
      } else {
        state.selectedId = null;
        state.cursor = -1;
        renderSelectedLaunchPanel();
        renderTermsTitle();
        updateCommonCommandsEnabled();
      }
    }
    const stale = Object.entries(state.staleSources)
      .filter(([, value]) => value)
      .map(([key]) => key);
    setStatus(stale.length
      ? tr("status.staleSources", { sources: stale.join(", ") })
      : (searching ? tr("status.searchHits", { count: state.all.length }) : tr("status.connected")));
    // Keep the system tray's project submenu in sync with the live
    // state. Best-effort — the tray doesn't exist in the browser
    // demo, where the invoke is a no-op.
    syncTrayProjects();
  } catch (e) {
    if (reloadCoordinator.isCurrent(request)) showError(e);
  } finally {
    reloadCoordinator.end(request);
  }
}

/* ── selection / keyboard ───────────────────────────────── */
// Toggle the `is-selected` class on the affected articles only — no DOM
// rebuild. Lets `↑↓` nav and row-click feel instant on large archives.
function updateSelectionHighlight(prevId, newId) {
  if (prevId === newId) return;
  const prev = prevId ? ledger.querySelector(`.entry[data-id="${prevId}"]`) : null;
  const next = newId ? ledger.querySelector(`.entry[data-id="${newId}"]`) : null;
  prev?.classList.remove("is-selected");
  next?.classList.add("is-selected");
}
function select(id) {
  const prev = state.selectedId;
  state.selectedId = id;
  state.cursor = state.filtered.findIndex(p => p.id === id);
  const selectedProject = state.all.find(project => project.id === id);
  // Capability probing is lazy for remote machines: selecting a remote
  // project is the first point at which its launch buttons need an answer.
  // The probe itself is single-flight and TTL-cached, so keyboard navigation
  // cannot create a network burst.
  if (selectedProject) {
    void refreshTuiMachine(
      selectedProject.source === "remote" && selectedProject.remoteServerId != null
        ? Number(selectedProject.remoteServerId)
        : null,
    );
  }
  updateSelectionHighlight(prev, id);
  // Right pane reacts: if a tab for this project is open, switch to it;
  // otherwise show the launch panel so the user can pick a tool or jump
  // to another open session. Skip web/file tabs (project may be null or
  // belong to a different project) so we only auto-switch to a real
  // session tab tied to this project.
  const projectTab = state.tabs.find(t => t.kind === "pty" && t.project?.id === id);
  if (projectTab) {
    switchTab(projectTab.ptyId);
  } else {
    // No tab for the selected project. The currently-active term-pane is
    // position:absolute/inset:0 over the whole viewport — it would cover
    // the launch panel, making the click look like a no-op. Deactivate
    // the pane (and clear activeTabId) so the launch panel is actually
    // visible, matching the empty-state path.
    const activeTab = state.tabs.find(t => t.tabId === state.activeTabId);
    if (activeTab) activeTab.pane.classList.remove("is-active");
    state.activeTabId = null;
    termsEmpty.style.display = "flex";
  }
  renderSelectedLaunchPanel();
  updateCommonCommandsEnabled();
  // Footer reacts: refresh git info for the newly selected project.
  refreshFootGit();
  // File-tree view (when active) follows the selected project.
  if (state.viewMode === "files") refreshLeftPaneTree();
  // Terms-bar kicker reflects the selected project.
  renderTermsTitle();
}
function moveCursor(delta) {
  if (!state.filtered.length) return;
  state.cursor = (state.cursor + delta + state.filtered.length) % state.filtered.length;
  select(state.filtered[state.cursor].id);
  let el = ledger.querySelector(`.entry[data-id="${state.filtered[state.cursor].id}"]`);
  if (!el) {
    const targetId = state.filtered[state.cursor].id;
    const layoutIndex = state.ledgerLayout?.rows.findIndex(row =>
      row.type === "project" && row.project.id === targetId,
    );
    const targetOffset = layoutIndex >= 0 ? state.ledgerLayout.offsets[layoutIndex] : 0;
    ledger.scrollTop = Math.max(0, targetOffset);
    renderVirtualLedger();
    el = ledger.querySelector(`.entry[data-id="${targetId}"]`);
  }
  el?.scrollIntoView({ block: "nearest" });
}

/* ── meta strip ─────────────────────────────────────────── */
function renderMeta(projects) {
  const tools = new Set();
  let sessions = 0;
  projects.forEach(p => (p.toolUsages||[]).forEach(u => { tools.add(u.toolKey); sessions += u.sessionCount||0; }));
  document.getElementById("metaTools").textContent = String(tools.size).padStart(2,"0");
  document.getElementById("metaProjects").textContent = String(projects.length).padStart(2,"0");
  document.getElementById("metaSessions").textContent = String(sessions).padStart(3,"0");
  const latest = projects.reduce((m,p)=> p.lastAccessedAt > (m||"") ? p.lastAccessedAt : m, "");
  document.getElementById("metaRecent").textContent = latest ? relTime(latest) : "—";
  document.getElementById("syncStamp").textContent = latest
    ? new Date(latest).toLocaleString(currentLocaleTag(),{month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit"})
    : "—";
}

/* ── status & errors ────────────────────────────────────── */
function setStatus(msg) { document.getElementById("footStatus").textContent = msg; }

/* ── foot: git info for selected project ────────────────── */
// One-line summary of the currently selected project's git state: branch,
// short SHA + last commit subject, and a clickable pill for each remote.
// If the project isn't a git repo, we surface that explicitly. If it is
// but has no remotes, we expose a "+ add remote" affordance that toggles
// an inline name/url form (Enter submits, Esc cancels).
//
// `footGit` row is hidden until `refreshFootGit` populates it, so the
// existing foot row (meta / status / keys) shows alone when nothing is
// selected.
const _footGit = {
  row:        document.getElementById("footGit"),
  branch:     document.getElementById("footGitBranch"),
  sync:       document.getElementById("footGitSync"),
  head:       document.getElementById("footGitHead"),
  remotes:    document.getElementById("footGitRemotes"),
  addBtn:     document.getElementById("footGitAdd"),
  current:    null,   // project currently shown (id) or null
  info:       null,   // last fetched GitInfo for current
};

const GIT_INFO_TTL_MS = 180000;
const GIT_SELECTION_DEBOUNCE_MS = 300;
let _gitSelectionTimer = null;
let _gitSelectionCancel = null;
let _gitSelectionToken = 0;
const _gitInflight = new Map();

function patchGitSyncBadge(projectId) {
  const entry = ledger.querySelector(`.entry[data-id="${CSS.escape(String(projectId))}"]`);
  if (!entry) return;
  const body = entry.querySelector(".entry__body");
  if (!body) return;
  const current = body.querySelector(".entry__git-sync");
  const project = state.all.find(item => item.id === projectId);
  const html = project ? projectGitSyncBadgeHtml(project) : "";
  if (current) {
    if (html) current.outerHTML = html;
    else current.remove();
  } else if (html) {
    const source = body.querySelector(".entry__source");
    source?.insertAdjacentHTML("afterend", html);
  }
}

function renderCachedFootGit(project, info) {
  if (_footGit.current !== project.id) return;
  _footGit.info = info;
  renderFootGit(info);
  patchGitSyncBadge(project.id);
}

function refreshFootGit({ force = false, allowFetch = true } = {}) {
  // Close any open add-remote form / branch menu so a selection change
  // always starts clean.
  closeAddRemoteForm();
  closeBranchMenu();
  const token = ++_gitSelectionToken;
  if (_gitSelectionTimer) {
    clearTimeout(_gitSelectionTimer);
    _gitSelectionTimer = null;
    _gitSelectionCancel?.(null);
    _gitSelectionCancel = null;
  }
  const p = state.all.find(x => x.id === state.selectedId);
  _footGit.current = p ? p.id : null;
  if (!p) {
    _footGit.row.hidden = true;
    _footGit.info = null;
    return Promise.resolve(null);
  }
  if (!HAS_TAURI) {
    renderFootGit({ isRepo: true, branch: "demo", remotes: [], headShort: null, headSummary: null, dirty: false, upstream: "origin/demo", ahead: 1, behind: 0, remoteChecked: true });
    return Promise.resolve(_footGit.info);
  }
  const cached = state.gitStatusByProject[p.id];
  const cacheAge = Date.now() - Number(state.gitStatusAtByProject[p.id] || 0);
  if (cached && !force && cached.remoteChecked !== false && cacheAge >= 0 && cacheAge < GIT_INFO_TTL_MS) {
    renderCachedFootGit(p, cached);
    return Promise.resolve(cached);
  }
  if (!allowFetch) {
    const existing = cached || (_footGit.current === p.id ? _footGit.info : null);
    if (existing) renderCachedFootGit(p, existing);
    else _footGit.row.hidden = true;
    return Promise.resolve(existing || null);
  }
  // Keep a previous snapshot visible while the debounced request is pending.
  if (cached) renderCachedFootGit(p, cached);
  // A fast A -> B -> A selection must attach before A's existing sync can
  // settle. Waiting for the regular debounce here creates a race where the
  // first promise finishes, is removed, and the timer starts a duplicate
  // fetch whose resolver/UI state no longer belongs to the original request.
  if (_gitInflight.has(p.id)) {
    return loadFootGit(p, token, { force }).catch(() => null);
  }
  return new Promise(resolve => {
    _gitSelectionCancel = resolve;
    _gitSelectionTimer = setTimeout(() => {
      _gitSelectionCancel = null;
      _gitSelectionTimer = null;
      void loadFootGit(p, token, { force }).then(resolve, () => resolve(null));
    }, GIT_SELECTION_DEBOUNCE_MS);
  });
}

async function loadFootGit(p, token, { force = false } = {}) {
  const existing = _gitInflight.get(p.id);
  if (existing) {
    const quick = state.gitStatusByProject[p.id];
    if (quick?.remoteChecked === false
      && _footGit.current === p.id
      && token === _gitSelectionToken) {
      renderCachedFootGit(p, quick);
    }
    try {
      const info = await existing;
      if (info) {
        state.gitStatusByProject[p.id] = info;
        state.gitStatusAtByProject[p.id] = Date.now();
      }
      if (_footGit.current === p.id && token === _gitSelectionToken && info) {
        renderCachedFootGit(p, info);
      }
      return info;
    } catch (error) {
      if (_footGit.current === p.id && token === _gitSelectionToken) {
        renderFootGit({ isRepo: false, remotes: [], error: String(error) });
      }
      throw error;
    }
  }
  if (_footGit.current !== p.id || token !== _gitSelectionToken) return null;
  const promise = (async () => {
    if (p.source === "remote") {
      const provisional = {
        isRepo: true,
        branch: p.gitBranch,
        remotes: [],
        headShort: null,
        headSummary: null,
        dirty: false,
        upstream: "remote",
        ahead: 0,
        behind: 0,
        remoteChecked: false,
        fetchError: null,
      };
      // Keep the remote project's provisional identity in the same cache as
      // local quick snapshots.  A fast A→B→A selection can then repaint
      // the correct branch while the one in-flight SSH refresh is reused.
      state.gitStatusByProject[p.id] = provisional;
      state.gitStatusAtByProject[p.id] = Date.now();
      if (_footGit.current === p.id && token === _gitSelectionToken) {
        renderCachedFootGit(p, provisional);
      }
      return invoke("refresh_remote_git_info", { serverId: Number(p.remoteServerId), path: p.path });
    }
    const info = await invoke("get_git_info", { path: p.path });
    state.gitStatusByProject[p.id] = info;
    state.gitStatusAtByProject[p.id] = Date.now();
    if (_footGit.current === p.id && token === _gitSelectionToken) {
      renderCachedFootGit(p, info);
    }
    if (!info.isRepo || !info.upstream) {
      return { ...info, remoteChecked: true };
    }
    try {
      const sync = await invoke("refresh_git_sync", { path: p.path });
      return { ...info, ...sync };
    } catch (error) {
      return { ...info, remoteChecked: true, fetchError: String(error) };
    }
  })();
  _gitInflight.set(p.id, promise);
  try {
    const info = await promise;
    state.gitStatusByProject[p.id] = info;
    state.gitStatusAtByProject[p.id] = Date.now();
    if (_footGit.current !== p.id || token !== _gitSelectionToken) return info;
    renderCachedFootGit(p, info);
    return info;
  } catch (error) {
    if (_footGit.current !== p.id || token !== _gitSelectionToken) throw error;
    renderFootGit({ isRepo: false, remotes: [], error: String(error) });
    throw error;
  } finally {
    if (_gitInflight.get(p.id) === promise) _gitInflight.delete(p.id);
  }
}

function renderFootGit(info) {
  _footGit.row.hidden = false;
  // Branch
  if (info.isRepo) {
    _footGit.branch.textContent = `⎇ ${info.branch || tr("git.detached")}`;
    _footGit.branch.classList.toggle("is-dirty", !!info.dirty);
  } else {
    _footGit.branch.textContent = tr("git.notARepo");
    _footGit.branch.classList.remove("is-dirty");
  }
  _footGit.sync.className = "foot__git-sync";
  _footGit.sync.title = "";
  if (!info.isRepo) {
    _footGit.sync.textContent = "";
  } else if (!info.upstream) {
    _footGit.sync.textContent = tr("git.noUpstream");
    _footGit.sync.classList.add("is-muted");
  } else if (Number(info.behind) > 0) {
    _footGit.sync.textContent = tr("git.notLatest", { count: info.behind });
    _footGit.sync.classList.add("is-behind");
  } else if (Number(info.ahead) > 0) {
    _footGit.sync.textContent = tr("git.needsPush", { count: info.ahead });
    _footGit.sync.classList.add("is-ahead");
  } else if (!info.remoteChecked) {
    _footGit.sync.textContent = tr("git.checking");
    _footGit.sync.classList.add("is-muted");
  } else if (info.fetchError) {
    _footGit.sync.textContent = tr("git.checkFailed");
    _footGit.sync.title = info.fetchError;
    _footGit.sync.classList.add("is-warning");
  } else {
    _footGit.sync.textContent = tr("git.synced");
    _footGit.sync.classList.add("is-synced");
  }
  // Head summary: short SHA + last subject, dimmed.
  const headText = info.isRepo
    ? [info.headShort, info.headSummary].filter(Boolean).join(" · ")
    : (info.error || "");
  _footGit.head.textContent = headText;
  _footGit.head.title = headText;
  // Remotes.
  _footGit.remotes.innerHTML = "";
  if (info.isRepo) {
    for (const r of info.remotes) {
      _footGit.remotes.appendChild(makeRemotePill(r.name, r.url));
    }
  }
  // Add-remote button: shown whenever the project has no remotes yet,
  // including non-git directories. For non-git dirs the backend will
  // `git init` first then attach the remote in one step, so the user
  // gets a one-click "make this a git project" affordance. Once at
  // least one remote exists, the button disappears (mirrors the no-
  // delete guardrail — fewer buttons, more deliberate operations).
  const currentProject = state.all.find(project => project.id === _footGit.current);
  _footGit.addBtn.hidden = currentProject?.source === "remote" || info.remotes.length > 0;
  _footGit.addBtn.title = info.isRepo
    ? tr("git.addRemoteTitle_repo")
    : tr("git.addRemoteTitle_init");
}

function makeRemotePill(name, url) {
  // Read-only pill: clicking opens the URL in the browser. Deleting a
  // remote from the UI is intentionally disabled — the user must run
  // `git remote remove <name>` in the project terminal to keep the
  // operation deliberate.
  const a = document.createElement("button");
  a.className = "foot__git-remote";
  a.type = "button";
  a.dataset.name = name;
  a.dataset.url = url;
  a.title = tr("git.remoteTooltip", { name, url });
  a.innerHTML = `<span class="foot__git-remote__name">${escapeHtml(name)}</span>
                 <span class="foot__git-remote__url">${escapeHtml(url)}</span>`;
  a.addEventListener("click", () => openRemoteUrl(url));
  return a;
}

async function openRemoteUrl(url) {
  if (!HAS_TAURI) { setStatus(tr("status.demoWouldOpenUrl", { url })); return; }
  try {
    await invoke("open_external_url", { url });
  } catch (e) {
    showActionError(tr("status.openFailed", { err: e }));
  }
}

// Markdown links (rendered by renderInline) carry class="md-link". Route
// their clicks through the Tauri `open_external_url` command (which whitelists
// http(s)) instead of letting the webview navigate, so the document CSP can
// stay strict and a README link can never navigate the app window away.
document.addEventListener("click", (e) => {
  const a = e.target.closest("a.md-link");
  if (!a) return;
  const href = a.getAttribute("href") || "";
  if (/^https?:\/\//i.test(href)) {
    e.preventDefault();
    openRemoteUrl(href);
  }
});

let _addRemoteForm = null; // current open form node (or null)

function openAddRemoteForm() {
  if (_addRemoteForm) { _addRemoteForm.querySelector("input").focus(); return; }
  const p = state.all.find(x => x.id === _footGit.current);
  if (!p) return;
  // Hide the + button while the form is open to keep the row compact.
  _footGit.addBtn.hidden = true;
  const form = document.createElement("form");
  form.className = "foot__git-form";
  form.innerHTML = `
    <span class="foot__git-form__label">${escapeHtml(tr("git.addRemoteLabel"))}</span>
    <input class="foot__git-form__name" type="text" data-i18n-placeholder="git.addRemoteNamePh" autocomplete="off" spellcheck="false" />
    <input class="foot__git-form__url"  type="text" data-i18n-placeholder="git.addRemoteUrlPh" autocomplete="off" spellcheck="false" />
    <button class="foot__git-form__submit" type="submit">${escapeHtml(tr("git.addRemoteSubmit"))}</button>
    <button class="foot__git-form__cancel" type="button">×</button>
  `;
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = form.querySelector(".foot__git-form__name").value.trim();
    const url  = form.querySelector(".foot__git-form__url").value.trim();
    if (!name || !url) { setStatus(tr("status.nameUrlRequired")); return; }
    try {
      await invoke("add_git_remote", { path: p.path, name, url });
      setStatus(tr("status.addedRemote", { name }));
      closeAddRemoteForm();
      delete state.gitStatusByProject[p.id];
      delete state.gitStatusAtByProject[p.id];
      await refreshFootGit({ force: true });
    } catch (err) {
      showActionError(tr("status.addRemoteFailed", { err }));
    }
  });
  form.querySelector(".foot__git-form__cancel").addEventListener("click", closeAddRemoteForm);
  _footGit.remotes.parentNode.insertBefore(form, _footGit.addBtn);
  _addRemoteForm = form;
  form.querySelector("input").focus();
}

function closeAddRemoteForm() {
  if (!_addRemoteForm) return;
  _addRemoteForm.remove();
  _addRemoteForm = null;
  _footGit.addBtn.hidden = false;
}

// Esc closes the inline form (highest-priority before the global Esc handler
// closes the entry modal or clears search).
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && (_addRemoteForm || _branchMenu)) {
    e.stopPropagation();
    closeAddRemoteForm();
  }
});
document.getElementById("footGitAdd").addEventListener("click", openAddRemoteForm);

/* ── left-pane view toggle (ledger ↔ file tree) ─────────── */
// Folder SVG used by the per-project tree button in each ledger entry.
// Defined here so entryHtml (declared earlier in the file) can reference
// it; module-level `const` is in TDZ until this line runs, which happens
// before init() invokes the first renderLedger().
const ICON_FILES   = iconSvg("folder", { size: 14 });

function setViewMode(mode) {
  if (mode !== "ledger" && mode !== "files") return;
  state.viewMode = mode;
  // Toggle the two left-pane views.
  for (const view of document.querySelectorAll(".stage__left__view")) {
    view.hidden = view.dataset.view !== mode;
  }
  // Refresh the file tree whenever we enter files mode (picks up the
  // currently selected project and any pending path changes). Re-evaluate
  // the files-back button so it stays enabled in files mode even with an
  // empty expand stack — it now doubles as the "back to ledger" affordance.
  if (mode === "files") {
    refreshLeftPaneTree();
  } else {
    leftTreeGate.invalidate();
    updateBackBtn(document.getElementById("stageLeftFilesTree"));
  }
}

function refreshLeftPaneTree() {
  const tree = document.getElementById("stageLeftFilesTree");
  const nameEl = document.getElementById("stageLeftFilesName");
  const p = state.all.find(x => x.id === state.selectedId);
  nameEl.textContent = p ? p.name : "—";
  loadFileTreeInto(tree, p, leftTreeGate);
}

function setupViewToggle() {
  // Back button: collapse the most recently expanded directory in the
  // left-pane tree. When the stack is empty AND we're in files mode, fall
  // back to the ledger view — the global deck-level tree button moved into
  // each project entry, so the back arrow is now the only way to return
  // to the project list without selecting a different entry.
  const backBtn = document.getElementById("filesBackBtn");
  backBtn.addEventListener("click", () => {
    const tree = document.getElementById("stageLeftFilesTree");
    if (treeGoBack(tree)) { updateBackBtn(tree); return; }
    if (state.viewMode === "files") setViewMode("ledger");
  });
}

/* ── foot: branch switcher ──────────────────────────────── */
// Click the branch label → popover lists local branches (current marked,
// others clickable). Picking a branch runs `git switch <name>` on the
// backend (which itself refuses to clobber uncommitted changes), then
// refreshes the foot row. Closes on outside click, Esc, or selection
// change.

let _branchMenu = null;

function openBranchMenu() {
  if (_branchMenu) { closeBranchMenu(); return; }
  if (!_footGit.info || !_footGit.info.isRepo) return;
  const branches = _footGit.info.localBranches || [];
  if (branches.length === 0) return; // detached HEAD or empty repo — no switchable branches

  const menu = document.createElement("div");
  menu.className = "foot__git-branch-menu";
  menu.setAttribute("role", "menu");
  menu.innerHTML = `
    <div class="foot__git-branch-menu__head">${escapeHtml(tr("branch.menuHead"))}</div>
    ${branches.map(b => `
      <button class="foot__git-branch-item ${b.isCurrent ? "is-current" : ""}"
              type="button" data-branch="${escapeHtml(b.name)}"
              ${b.isCurrent ? "disabled" : ""}>
        <span class="foot__git-branch-item__check">${b.isCurrent ? "✓" : ""}</span>
        <span class="foot__git-branch-item__name">${escapeHtml(b.name)}</span>
      </button>
    `).join("")}
  `;
  // Position the popover above the branch button.
  const btn = _footGit.branch;
  btn.parentNode.appendChild(menu);
  // Defer geometry read to after the node is in the DOM.
  requestAnimationFrame(() => {
    const r = btn.getBoundingClientRect();
    const row = btn.closest(".foot__row--git").getBoundingClientRect();
    menu.style.left = `${r.left - row.left}px`;
    menu.style.minWidth = `${Math.max(r.width, 140)}px`;
  });
  menu.querySelectorAll(".foot__git-branch-item").forEach(el => {
    el.addEventListener("click", async (e) => {
      e.stopPropagation();
      const name = el.dataset.branch;
      closeBranchMenu();
      await checkoutBranch(name);
    });
  });
  _branchMenu = menu;
  // Close on the next outside-click.
  setTimeout(() => document.addEventListener("click", closeBranchMenu, { once: true }), 0);
}

function closeBranchMenu() {
  if (_branchMenu) { _branchMenu.remove(); _branchMenu = null; }
}

async function checkoutBranch(name) {
  const p = state.all.find(x => x.id === _footGit.current);
  if (!p) return;
  if (!HAS_TAURI) { setStatus(tr("status.demoWouldSwitch", { name })); return; }
  try {
    await invoke("checkout_branch", { path: p.path, name });
    setStatus(tr("status.switched", { name }));
    delete state.gitStatusByProject[p.id];
    delete state.gitStatusAtByProject[p.id];
    await refreshFootGit({ force: true });
  } catch (e) {
    showActionError(tr("status.checkoutFailed", { err: e }));
  }
}

_footGit.branch.addEventListener("click", (e) => {
  e.stopPropagation();
  openBranchMenu();
});
function clearCatalogAfterError() {
  // A failed primary catalog read invalidates the visible ledger. Clear the
  // derived catalog/count as well; otherwise a previous search count can stay
  // in the footer while the ledger shows an error, which is misleading and
  // makes Escape-after-failure look as if the old query is still active.
  state.catalog = [];
  state.searchResults = null;
  state.all = [];
  state.filtered = [];
  state.cursor = -1;
  state.selectedId = null;
  renderCount();
  renderMeta([]);
}

function showError(msg) {
  const text = String(msg ?? "");
  clearCatalogAfterError();
  state.firstRunError = null;
  setStatus(tr("status.errorPrefix", { text }));
  ledger.innerHTML = `
    <div class="ledger__empty">
      <div class="ledger__empty-num">!</div>
      <div class="ledger__empty-title">${escapeHtml(tr("ledger.errorTitle"))}</div>
      <div class="ledger__empty-body">${escapeHtml(text)}</div>
      <button class="scan-btn" id="errRetry" style="margin-top:14px">${escapeHtml(tr("ledger.retry"))}</button>
    </div>`;
  document.getElementById("errRetry")?.addEventListener("click", () => reload());
}

function showFirstRunError(msg) {
  const text = String(msg ?? "");
  clearCatalogAfterError();
  state.firstRunError = text;
  setStatus(tr("status.firstRunScanFailed"));
  renderLedger();
}

function showActionError(msg) {
  const text = String(msg ?? "");
  setStatus(tr("status.errorPrefix", { text }));
}

/* ── auto-refresh ───────────────────────────────────────── */
function latestStamp(list){ return list.reduce((m,p)=> p.lastAccessedAt > (m||"") ? p.lastAccessedAt : m, ""); }
function startAutoRefresh() {
  if (!HAS_TAURI) return;
  stopAutoRefresh();
  state.autoTimer = setInterval(async () => {
    if (state.query.trim()) return;
    // Skip while a rescan is running — the rescan handler owns the list
    // refresh and we don't want to mutate state.all mid-scan.
    if (scanProgress && !scanProgress.hidden) return;
    const request = reloadCoordinator.beginAuto();
    if (!request) return;
    try {
      const [projects, remoteResult] = await Promise.all([
        invoke("list_projects", { limit: LIST_LIMIT }),
        captureResult(invoke("list_remote_projects")),
      ]);
      if (!reloadCoordinator.isCurrent(request)) return;
      const remoteProjects = remoteResult.ok ? (remoteResult.value || []) : state.remoteProjects;
      state.staleSources.remote = !remoteResult.ok;
      const nextProjects = mergeProjectSources(projects, remoteProjects);
      if (projectCatalogFingerprint(nextProjects) === projectCatalogFingerprint(state.catalog)) return;
      const prev = state.selectedId;
      if (remoteResult.ok) state.remoteProjects = remoteProjects;
      state.catalog = nextProjects;
      state.searchResults = null;
      state.all = state.catalog;
      renderMeta(state.all);
      applyFilters();
      if (!state.all.find(p => p.id === prev) && state.filtered.length) select(state.filtered[0].id);
    } catch (e) { /* silent */ }
  }, 60000);
}
function stopAutoRefresh(){ if (state.autoTimer){ clearInterval(state.autoTimer); state.autoTimer = null; } }

/* ── relative-time auto-refresh ─────────────────────────── */
// Re-compute the compact relative times in place every minute so labels
// like "3h 12m" tick forward without a data reload. Only patches the
// `.entry__time` text nodes + the meta "last" cell — preserves scroll
// position and any open `⋯` popover (no innerHTML rebuild).
let timeTimer = null;
function refreshTimeLabels() {
  const byId = new Map(state.all.map(p => [p.id, p]));
  for (const el of ledger.querySelectorAll(".entry")) {
    const p = byId.get(el.dataset.id);
    if (!p) continue;
    const timeEl = el.querySelector(".entry__time");
    if (timeEl) timeEl.textContent = relTime(p.lastAccessedAt);
  }
  const latest = state.all.reduce((m, p) => p.lastAccessedAt > (m || "") ? p.lastAccessedAt : m, "");
  const mr = document.getElementById("metaRecent");
  if (mr) mr.textContent = latest ? relTime(latest) : "—";
  if (!drawer.hidden && state.settingsView === "remote") {
    for (const row of drawerBody.querySelectorAll(".server-row[data-id]")) {
      const server = state.remoteServers.find(item => Number(item.id) === Number(row.dataset.id));
      syncRemoteScanTime(row, server);
    }
  }
}
function startTimeRefresh() {
  if (timeTimer) clearInterval(timeTimer);
  timeTimer = setInterval(refreshTimeLabels, 60000);
}
function stopTimeRefresh() { if (timeTimer) { clearInterval(timeTimer); timeTimer = null; } }

/* ── utils ──────────────────────────────────────────────── */
// Light/dark theme toggle. The inline head script in index.html sets
// `<html data-theme>` synchronously before CSS paints (no FOUC). This
// module just keeps the toggle button icon in sync and persists the user's
// choice to localStorage. OS preference is followed only when the user
// hasn't explicitly picked.
const THEME_KEY = "sessionatlas.theme";
const ICON_SUN  = `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4l1.4-1.4M17 7l1.4-1.4"/></svg>`;
const ICON_MOON = `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8z"/></svg>`;

// Show the icon of the TARGET theme: sun in dark (click → light), moon in
// light (click → dark). Standard toggle convention.
function syncThemeButton() {
  const theme = document.documentElement.dataset.theme || "dark";
  const btn = document.getElementById("themeBtn");
  if (!btn) return;
  btn.innerHTML = theme === "light" ? ICON_MOON : ICON_SUN;
  const label = theme === "light" ? tr("theme.toDark") : tr("theme.toLight");
  btn.setAttribute("title", label);
  btn.setAttribute("aria-label", label);
}

function setupThemeToggle() {
  syncThemeButton();
  const btn = document.getElementById("themeBtn");
  if (!btn) return;
  btn.addEventListener("click", () => {
    const next = (document.documentElement.dataset.theme || "dark") === "light" ? "dark" : "light";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem(THEME_KEY, next); } catch {}
    syncThemeButton();
  });
  // Follow OS preference only while the user hasn't explicitly chosen.
  if (window.matchMedia) {
    window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", e => {
      let stored = null;
      try { stored = localStorage.getItem(THEME_KEY); } catch {}
      if (stored) return;
      document.documentElement.dataset.theme = e.matches ? "light" : "dark";
      syncThemeButton();
    });
  }
}

function escapeHtml(s) { return String(s??"").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c])); }

/* ── i18n: static-HTML localization + language switching ──── */

// Walk the static HTML in index.html and apply the current language to any
// node carrying a `data-i18n` (text), `data-i18n-placeholder`, or
// `data-i18n-aria` (aria-label) / `data-i18n-title` attribute. Called once
// at boot and again whenever the language changes, so the chrome that
// lives outside the JS-rendered regions (top deck, footer, modal headers)
// stays in sync with the chosen language.
function localizeStaticHtml() {
  document.querySelectorAll("[data-i18n]").forEach(el => {
    el.textContent = tr(el.dataset.i18n);
  });
  document.querySelectorAll("[data-i18n-placeholder]").forEach(el => {
    el.setAttribute("placeholder", tr(el.dataset.i18nPlaceholder));
  });
  document.querySelectorAll("[data-i18n-title]").forEach(el => {
    el.setAttribute("title", tr(el.dataset.i18nTitle));
  });
  document.querySelectorAll("[data-i18n-aria]").forEach(el => {
    el.setAttribute("aria-label", tr(el.dataset.i18nAria));
  });
  document.documentElement.lang = currentLang();
}

// Switch the interface language: persist the choice, update <html lang>,
// re-render every JS-built region, re-localize static HTML, and push the
// new language to the system tray so its labels follow. `lang` is "en"
// or "zh". Also re-syncs the theme button label (it carries translated text).
function setLanguage(lang) {
  const next = lang === "zh" ? "zh" : "en";
  document.documentElement.lang = next;
  try { localStorage.setItem("sessionatlas.lang", next); } catch {}
  applyLocalizedUI();
  pushLangToTray();
}

// Re-render every visible region so its text picks up the new language.
// Mirrors the render fns `reload()` orchestrates, but without a data
// re-fetch. Safe to call before `state.all` is populated (the render fns
// guard against empty arrays). Also re-localizes dynamic inputs that the
// drawer injects via `data-i18n-placeholder`.
function applyLocalizedUI() {
  localizeStaticHtml();
  // Re-apply placeholders to dynamically-injected form inputs (drawer is
  // rebuilt by renderDrawerBody, which runs below, so this mostly covers
  // the case where a sub-page is open).
  renderTools();
  if (state.all && state.all.length) renderMeta(state.all);
  applyFilters();                       // → renderCount + renderLedger
  renderCommonCommands();
  renderTermsTitle();
  if (!drawer.hidden) { renderDrawerBody(); renderDrawerHead(); }
  renderSelectedLaunchPanel();
  syncOverviewCollapseUI();
  refreshFootGit({ allowFetch: false });
  syncThemeButton();                    // theme button label is translated
}

// Push the current language to the Rust tray menu so Show / Quit / the
// ungrouped submenu label follow the chosen language. Best-effort: the
// tray doesn't exist in the browser demo, where the invoke is a no-op.
function pushLangToTray() {
  if (!HAS_TAURI) return;
  invoke("set_tray_language", { lang: currentLang() }).catch(() => {});
}

/* ── tool chips (dynamic) ───────────────────────────────── */
// Track the last signature so the 60s auto-refresh doesn't repaint the chip
// strip on every tick when state.tools is unchanged.
let _lastToolsSig = null;
function renderTools() {
  const sig = JSON.stringify(state.tools) + "|" + state.tool;
  if (sig === _lastToolsSig) return;
  _lastToolsSig = sig;
  const nav = document.getElementById("filters");
  const chips = state.tools.map(t =>
    `<button class="chip" data-tool="${escapeHtml(t.toolKey)}"><i class="dot ${toolDotClass(t.toolKey)}" style="background:${toolColor(t.toolKey)}"></i>${escapeHtml(t.toolName)}</button>`
  ).join("");
  nav.innerHTML = `<button class="chip ${state.tool==="all"?"is-active":""}" data-tool="all">ALL</button>${chips}`;
}

/* ── wire UI ────────────────────────────────────────────── */
const searchInput = document.getElementById("searchInput");
searchInput.addEventListener("input", e => {
  state.query = e.target.value;
  reloadCoordinator.invalidateFull();
  reloadCoordinator.invalidateSearch();
  reloadCoordinator.invalidateAuto();
  clearTimeout(state.searchTimer);
  state.searchTimer = setTimeout(reload, SEARCH_DEBOUNCE_MS);
});

// Wire a chip-strip nav: clicking a chip updates `state[key]` from
// `chip.dataset[key]`, re-renders the strip's active class, and re-applies
// the filters. Used by the tool filter and the recency filter.
function wireChipGroup(navId, key, render) {
  document.getElementById(navId).addEventListener("click", e => {
    const chip = e.target.closest(".chip"); if (!chip) return;
    state[key] = chip.dataset[key];
    render();
    applyFilters();
  });
}
function renderRecency() {
  document.querySelectorAll("#recencyFilters .chip").forEach(c => {
    c.classList.toggle("is-active", c.dataset.recency === state.recency);
  });
}
wireChipGroup("filters", "tool", renderTools);
wireChipGroup("recencyFilters", "recency", renderRecency);

const scanBtn = document.getElementById("scanBtn");
function setLocalScanBusy(busy) {
  scanBtn.classList.toggle("is-working", busy);
  scanBtn.disabled = busy;
  scanBtn.setAttribute("aria-busy", String(busy));
  if (scanProgress) scanProgress.hidden = !busy;
}

async function runLocalScan({ initial = false } = {}) {
  const recoveringFirstRun = state.firstRunError !== null;
  reloadCoordinator.invalidateFull();
  reloadCoordinator.invalidateSearch();
  reloadCoordinator.invalidateAuto();
  setLocalScanBusy(true);
  setStatus(tr(initial ? "status.firstRunScanning" : "status.scanningInstruments"));
  try {
    // Run the scan WITHOUT touching state.all — the ledger stays exactly
    // as-is so the user doesn't see their project list vanish or flicker
    // mid-scan. The progress bar is the only visible feedback. We refresh
    // the list once, after `sessionatlas scan` has finished writing the index.
    if (HAS_TAURI) { await invoke("scan_projects"); }
    else { await new Promise(r => setTimeout(r, 700)); }
    state.firstRunError = null;
    await reload();
    const stale = Object.entries(state.staleSources)
      .filter(([, value]) => value)
      .map(([key]) => key);
    setStatus(stale.length
      ? tr("status.staleSources", { sources: stale.join(", ") })
      : tr(initial ? "status.firstRunComplete" : "status.scanComplete", { count: state.all.length }));
    if (initial || recoveringFirstRun) startAutoRefresh();
    return true;
  } catch (e) {
    if (initial) showFirstRunError(e);
    else showActionError(tr("status.scanFailed", { err: e }));
    return false;
  }
  finally {
    setLocalScanBusy(false);
  }
}

scanBtn.addEventListener("click", () => runLocalScan());

document.addEventListener("keydown", e => {
  // Dialogs and focused controls own their keystrokes. Without this guard,
  // pressing Enter on a project action also fell through to the global
  // "open selected project" shortcut, launching an unintended terminal.
  if (visibleDialog()) return;
  const target = e.target instanceof Element ? e.target : null;
  const isTextEntry = Boolean(target?.matches("input, textarea, select, [contenteditable='true']"));
  const isControl = Boolean(target?.closest("button, a[href]"));
  if ((isTextEntry && target !== searchInput) || isControl) return;
  const ctrlSearch = (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k";
  const slashSearch = e.key === "/" && document.activeElement !== searchInput;
  if (ctrlSearch || slashSearch) {
    e.preventDefault();
    searchInput.focus();
    if (ctrlSearch) searchInput.select();
  }
  else if (e.key === "Escape") { searchInput.value=""; state.query=""; reloadCoordinator.invalidateSearch(); reloadCoordinator.invalidateAuto(); clearTimeout(state.searchTimer); reload(); searchInput.blur(); }
  else if (e.key === "ArrowDown") { e.preventDefault(); moveCursor(1); }
  else if (e.key === "ArrowUp") { e.preventDefault(); moveCursor(-1); }
  else if (e.key === "g" && document.activeElement !== searchInput) { e.preventDefault(); openSettings(); }
  else if (e.key === "Enter" && state.cursor >= 0) {
    const p = state.filtered[state.cursor];
    if (p) {
      const usage = (p.toolUsages||[]).slice().sort((a,b)=>new Date(b.lastUsedAt)-new Date(a.lastUsedAt))[0];
      openTerminalTab(p, usage || { toolKey: "shell" });
    }
  }
});

/* ── boot ───────────────────────────────────────────────── */
(async function boot() {
  setStatus(tr("status.boot", { tauri: HAS_TAURI, term: HAS_TERM }));
  setupThemeToggle();
  setupOverviewToggle();
  setupViewToggle();
  localizeStaticHtml();        // localize chrome text + placeholders from <html lang>
  await trackMaximizedState();
  try {
    await wirePtyEvents();
  } catch (e) {
    state.ptyEventsReady = false;
    showActionError(tr("term.eventsFailed", { err: e }));
  }
  wireTrayEvents();
  wireResize();
  wireEntryMenuDelegation();
  wireLedgerVirtualization();
  wireDrawerDelegation();
  wireDrawerBackButton();
  wireDrawerForms();
  wireDrag();
  renderCommonCommands();
  document.getElementById("settingsBtn").addEventListener("click", openSettings);
  let localIndexReady = true;
  let localCatalogLoaded = false;
  if (HAS_TAURI) {
    try {
      // `false` is the only value that starts a scan. This keeps older test
      // fixtures and mixed-version installations from mistaking an unknown
      // command result for a missing index.
      const indexExists = await invoke("local_index_exists");
      if (indexExists === false) {
        localIndexReady = await runLocalScan({ initial: true });
        localCatalogLoaded = localIndexReady;
      }
    } catch (e) {
      localIndexReady = false;
      showFirstRunError(e);
    }
  }
  if (localIndexReady && !localCatalogLoaded) await reload();
  loadOpenerPrefs();
  loadWebDevelopmentTools();
  loadGroups();
  pushLangToTray();            // keep the tray menu's language in sync on startup
  if (localIndexReady) startAutoRefresh();
  startTimeRefresh();
})();
