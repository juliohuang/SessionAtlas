export const REVIEW_STRINGS = {
  en: {
    "review.title": "Project review",
    "review.resumeReason.tool_checking": "Checking tool availability. Please wait for the result.",
    "review.history": "Session history",
    "review.changes": "Git changes",
    "review.close": "Close project review",
    "review.historySearch": "Search this project's session history",
    "review.historySearchHint": "Search session titles, previews, and IDs. This does not search project files.",
    "review.toolFilter": "Filter by tool",
    "review.allTools": "All tools",
    "review.loading": "Loading…",
    "review.retry": "Retry",
    "review.loadFailed": "Could not load this project data.",
    "review.incomplete": "Some session sources could not be read. The results may be incomplete.",
    "review.noSessions": "No session history is available for this project.",
    "review.noMatches": "No sessions match these filters.",
    "review.resume": "Resume",
    "review.newSession": "New session",
    "review.noResume": "This session cannot be resumed.",
    "review.starting": "Starting…",
    "review.launchFailed": "Could not start the session.",
    "review.optionalSources": "Some optional session sources are unavailable. Their absence does not mean the project has no sessions.",
    "review.historyIncomplete": "Some session data could not be read completely. Results may be incomplete.",
    "review.auxiliaryFiltered": "Background and delegated sessions are omitted from this history.",
    "review.recordedActivity": "Recorded tool activity, not a verified session history.",
    "review.resumeReason.remote_session_unverified": "The remote session ID cannot be verified locally. Start a new session or check the remote connection in Settings.",
    "review.resumeReason.path_missing": "The project path is unavailable. Restore the folder and rescan, or choose another project.",
    "review.resumeReason.tool_missing": "The tool is unavailable. Install it or review its setup in Settings.",
    "review.resumeReason.tool_disabled": "This tool is disabled. Enable it in Settings or start with another tool.",
    "review.resumeReason.unsupported_adapter": "No supported adapter is configured. Review adapters in Settings or start with another tool.",
    "review.resumeReason.unsupported_platform": "This tool is not supported on this machine. Check its platform settings or choose another tool.",
    "review.resumeReason.unsupported_resume": "This session format cannot be resumed. Start a new session instead.",
    "review.resumeReason.session_missing": "The session is missing from the source data. Restore the source or start a new session.",
    "review.resumeReason.history_incomplete": "Session history could not be checked completely. Restore the source and retry, or start a new session.",
    "review.resumeReason.ssh_unavailable": "The remote connection is unavailable. Check the SSH server and key settings, then retry.",
    "review.resumeReason.check_failed": "The resume check failed. Retry or review the tool and connection settings.",
    "review.resumeReason.launch_failed": "The session did not start. Retry or review the tool settings.",
    "review.resumeReason.native_resume_id_unavailable": "No safe resume ID is available. Start a new session instead.",
    "review.resumeReason.unavailable": "The session source is unavailable. Restore it or start a new session.",
    "review.statusUntracked": "Untracked",
    "review.statusType": "Type changed",
    "review.statusConflict": "Conflict",
    "review.statusCopied": "Copied",
    "review.browserDemo": "Browser demo — synthetic sample data, not read from this project.",
    "review.remoteScope": "Remote session history is not available here. Recorded tool usage is shown below.",
    "review.changesLoading": "Loading Git changes…",
    "review.noChanges": "No working-tree changes found.",
    "review.file": "Changed file",
    "review.staged": "Staged",
    "review.unstaged": "Unstaged",
    "review.untracked": "Untracked",
    "review.selectFile": "Select a changed file to inspect its diff.",
    "review.diffLoading": "Loading diff…",
    "review.diffUnavailable": "Diff is unavailable for this file.",
    "review.binaryDiff": "This file is binary; text diff is unavailable.",
    "review.emptyDiff": "No diff content is available.",
    "review.truncatedDiff": "The diff was shortened to fit the preview.",
    "review.remoteDiff": "Git diff preview is not available for remote projects.",
    "review.statusAdded": "Added",
    "review.statusModified": "Modified",
    "review.statusDeleted": "Deleted",
    "review.statusRenamed": "Renamed",
    "review.statusUnknown": "Changed",
  },
  zh: {
    "review.title": "项目审阅",
    "review.resumeReason.tool_checking": "正在检查工具是否可用，请等待检测结果。",
    "review.history": "会话历史",
    "review.changes": "Git 变更",
    "review.close": "关闭项目审阅",
    "review.historySearch": "搜索此项目的会话历史",
    "review.historySearchHint": "搜索会话标题、预览和 ID。此处不会搜索项目文件。",
    "review.toolFilter": "按工具筛选",
    "review.allTools": "全部工具",
    "review.loading": "正在加载…",
    "review.retry": "重试",
    "review.loadFailed": "无法加载此项目的数据。",
    "review.incomplete": "部分会话来源读取失败，结果可能不完整。",
    "review.noSessions": "此项目没有可用的会话历史。",
    "review.noMatches": "没有符合筛选条件的会话。",
    "review.resume": "继续会话",
    "review.newSession": "新建会话",
    "review.noResume": "此会话无法继续。",
    "review.starting": "正在启动…",
    "review.launchFailed": "无法启动会话。",
    "review.optionalSources": "部分可选会话来源当前不可用。来源缺失不代表项目没有会话。",
    "review.historyIncomplete": "部分会话数据未能完整读取，结果可能不完整。",
    "review.auxiliaryFiltered": "后台和委派会话不会显示在此历史中。",
    "review.recordedActivity": "已记录的工具活动，不代表已核实的会话历史。",
    "review.resumeReason.remote_session_unverified": "无法在本机验证远程会话 ID。可新建会话，或在设置中检查远程连接。",
    "review.resumeReason.path_missing": "项目路径不可用。请恢复文件夹后重新扫描，或选择其他项目。",
    "review.resumeReason.tool_missing": "工具不可用。请安装工具，或在设置中检查配置。",
    "review.resumeReason.tool_disabled": "此工具已停用。请在设置中启用，或选择其他工具。",
    "review.resumeReason.unsupported_adapter": "未配置受支持的适配器。请在设置中检查适配器，或选择其他工具。",
    "review.resumeReason.unsupported_platform": "此工具不支持当前设备。请检查平台设置，或选择其他工具。",
    "review.resumeReason.unsupported_resume": "此会话格式无法继续，请新建会话。",
    "review.resumeReason.session_missing": "来源数据中找不到此会话。请恢复来源数据，或新建会话。",
    "review.resumeReason.history_incomplete": "未能完整检查会话历史。请恢复来源后重试，或新建会话。",
    "review.resumeReason.ssh_unavailable": "远程连接不可用。请检查 SSH 服务器和密钥设置后重试。",
    "review.resumeReason.check_failed": "继续会话检查失败。请重试，或检查工具和连接设置。",
    "review.resumeReason.launch_failed": "会话未能启动。请重试，或检查工具设置。",
    "review.resumeReason.native_resume_id_unavailable": "没有可安全使用的继续会话 ID，请新建会话。",
    "review.resumeReason.unavailable": "会话来源不可用。请恢复来源，或新建会话。",
    "review.statusUntracked": "未跟踪",
    "review.statusType": "类型已变更",
    "review.statusConflict": "冲突",
    "review.statusCopied": "已复制",
    "review.browserDemo": "浏览器演示：以下是合成示例数据，并非从此项目读取。",
    "review.remoteScope": "此处无法读取远程会话历史，下面仅显示已记录的工具使用情况。",
    "review.changesLoading": "正在加载 Git 变更…",
    "review.noChanges": "未发现工作区变更。",
    "review.file": "变更文件",
    "review.staged": "已暂存",
    "review.unstaged": "未暂存",
    "review.untracked": "未跟踪",
    "review.selectFile": "选择一个变更文件以查看差异。",
    "review.diffLoading": "正在加载差异…",
    "review.diffUnavailable": "无法查看此文件的差异。",
    "review.binaryDiff": "这是二进制文件，无法显示文本差异。",
    "review.emptyDiff": "没有可显示的差异内容。",
    "review.truncatedDiff": "差异内容过长，已缩短显示。",
    "review.remoteDiff": "远程项目暂不提供 Git 差异预览。",
    "review.statusAdded": "新增",
    "review.statusModified": "已修改",
    "review.statusDeleted": "已删除",
    "review.statusRenamed": "已重命名",
    "review.statusUnknown": "已变更",
  },
};

export function filterReviewSessions(sessions, query = "", toolKey = "") {
  const needle = String(query).trim().toLocaleLowerCase();
  return (Array.isArray(sessions) ? sessions : []).filter(session => {
    if (toolKey && session.toolKey !== toolKey) return false;
    if (!needle) return true;
    return [session.title, session.preview, session.sessionId]
      .some(value => String(value || "").toLocaleLowerCase().includes(needle));
  });
}

export function gitStatusLabelKey(status, staged, untracked = false) {
  if (untracked) return "review.statusUntracked";
  const value = String(status || "");
  const xy = value.length >= 2 ? value : ({ added: "A ", modified: " M", deleted: " D", renamed: "R " })[value.toLowerCase()] || "  ";
  const code = xy[staged ? 0 : 1];
  return ({ A: "review.statusAdded", M: "review.statusModified", D: "review.statusDeleted", R: "review.statusRenamed", C: "review.statusCopied", T: "review.statusType", U: "review.statusConflict" })[code] || "review.statusUnknown";
}

const focusableSelector = 'button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
const isRemote = project => project?.source === "remote" || project?.remoteServerId != null;

export function createProjectReviewPanel({ invoke, hasTauri, tr, canLaunch, launchReason, launch, onError } = {}) {
  const translate = key => tr?.(key) || REVIEW_STRINGS.en[key] || key;
  const root = document.createElement("section");
  root.className = "project-review";
  root.hidden = true;
  root.innerHTML = `
    <div class="project-review__backdrop" data-review-backdrop></div>
    <section class="project-review__panel" role="dialog" aria-modal="true" aria-labelledby="reviewTitle" tabindex="-1">
      <header class="project-review__head">
        <div><span class="project-review__eyebrow">SESSIONATLAS</span><h2 id="reviewTitle" data-review-title></h2><p data-review-path></p></div>
        <button class="project-review__close" type="button" data-review-close></button>
      </header>
      <div class="project-review__tabs" role="tablist">
        <button id="reviewHistoryTab" type="button" role="tab" data-review-tab="history" aria-controls="reviewHistory"></button>
        <button id="reviewChangesTab" type="button" role="tab" data-review-tab="changes" aria-controls="reviewChanges"></button>
      </div>
      <div class="project-review__body">
        <section id="reviewHistory" role="tabpanel" aria-labelledby="reviewHistoryTab" tabindex="0" data-review-pane="history">
          <div class="project-review__history-tools">
            <label class="project-review__search"><span class="sr-only" data-review-search-label></span><input type="search" data-review-search /></label>
            <label class="project-review__select"><span class="sr-only" data-review-tool-label></span><select data-review-tool></select></label>
          </div>
          <p class="project-review__hint" data-review-hint></p>
          <div class="project-review__notice" data-review-notice hidden></div>
          <div class="project-review__list" data-review-list></div>
        </section>
        <section id="reviewChanges" role="tabpanel" aria-labelledby="reviewChangesTab" tabindex="0" data-review-pane="changes" hidden>
          <div class="project-review__notice" data-change-notice hidden></div>
          <div class="project-review__changes">
            <div class="project-review__files" data-review-files></div>
            <div class="project-review__diff" data-review-diff></div>
          </div>
        </section>
      </div>
    </section>`;
  document.body.append(root);

  const $ = selector => root.querySelector(selector);
  const panel = $(".project-review__panel");
  const tabs = [...root.querySelectorAll("[data-review-tab]")];
  const state = { open: false, project: null, mode: "history", epoch: 0, historySeq: 0, changesSeq: 0, diffSeq: 0, launchSeq: 0, previousFocus: null, previousInert: false, sessions: [], issues: [], files: [], selected: null, diffCache: new Map(), diffErrors: new Map(), diffLoading: null, historyState: "idle", changesState: "idle", demo: false, error: null, launchError: null, pendingLaunches: new Set() };

  function localize() {
    $("[data-review-title]").textContent = translate("review.title");
    $("[data-review-close]").textContent = "\u00d7";
    $("[data-review-close]").setAttribute("aria-label", translate("review.close"));
    tabs[0].textContent = translate("review.history");
    tabs[1].textContent = translate("review.changes");
    root.querySelector("[role=tablist]").setAttribute("aria-label", translate("review.title"));
    $("[data-review-search]").placeholder = translate("review.historySearch");
    $("[data-review-search-label]").textContent = translate("review.historySearch");
    $("[data-review-tool-label]").textContent = translate("review.toolFilter");
    $("[data-review-hint]").textContent = translate("review.historySearchHint");
    renderHistory();
    renderChanges();
  }

  function setMode(mode) {
    const nextMode = mode === "changes" ? "changes" : "history";
    if (nextMode !== state.mode) {
      if (state.historyState === "loading") state.historyState = "idle";
      if (state.changesState === "loading") state.changesState = "idle";
      state.historySeq += 1;
      state.changesSeq += 1;
      state.diffSeq += 1;
      state.diffLoading = null;
    }
    state.mode = nextMode;
    tabs.forEach(tab => {
      const active = tab.dataset.reviewTab === state.mode;
      tab.setAttribute("aria-selected", String(active));
      tab.tabIndex = active ? 0 : -1;
    });
    root.querySelectorAll("[data-review-pane]").forEach(pane => { pane.hidden = pane.dataset.reviewPane !== state.mode; });
    if (state.mode === "history") ensureHistory();
    else {
      ensureChanges();
      loadSelectedDiffIfMissing();
    }
  }

  function resetForProject(project, mode) {
    state.epoch += 1;
    state.historySeq += 1;
    state.changesSeq += 1;
    state.diffSeq += 1;
    state.launchSeq += 1;
    state.project = project;
    state.sessions = [];
    state.issues = [];
    state.files = [];
    state.selected = null;
    state.diffCache.clear();
    state.diffErrors.clear();
    state.diffLoading = null;
    state.historyState = "idle";
    state.changesState = "idle";
    state.error = null;
    state.launchError = null;
    state.pendingLaunches.clear();
    state.demo = !hasTauri;
    $("[data-review-path]").textContent = project?.path || "";
    $("[data-review-search]").value = "";
    renderToolFilter();
    renderHistory();
    renderChanges();
    setMode(mode);
  }

  async function ensureHistory(retry = false) {
    if (!state.open || !state.project || state.historyState === "loading" || (!retry && state.historyState !== "idle")) return;
    if (isRemote(state.project)) { state.historyState = "remote"; renderHistory(); return; }
    const seq = ++state.historySeq;
    const epoch = state.epoch;
    state.historyState = "loading";
    renderHistory();
    try {
      if (state.demo) {
        state.sessions = [
          { toolKey: "codex", sessionId: "demo-session-01", title: "Sample planning session", preview: "Synthetic sample preview for browser mode.", lastUsedAt: "2026-01-15T10:00:00Z", canResume: true, reason: null },
          { toolKey: "claude", sessionId: "demo-session-02", title: "Sample review session", preview: "This content is not read from the selected project.", lastUsedAt: "2026-01-14T08:30:00Z", canResume: false, reason: "unavailable" },
        ];
      } else {
        const result = await invoke("list_project_sessions", { path: state.project.path });
        if (!state.open || epoch !== state.epoch || seq !== state.historySeq) return;
        state.sessions = Array.isArray(result?.sessions) ? result.sessions : [];
        state.issues = Array.isArray(result?.issues) ? result.issues : [];
      }
      if (!state.open || epoch !== state.epoch || seq !== state.historySeq) return;
      state.historyState = "ready";
    } catch (error) {
      if (!state.open || epoch !== state.epoch || seq !== state.historySeq) return;
      state.historyState = "error";
      state.error = error;
      onError?.(error);
    }
    renderToolFilter();
    renderHistory();
  }

  async function ensureChanges(retry = false) {
    if (!state.open || !state.project || state.changesState === "loading" || (!retry && state.changesState !== "idle")) return;
    if (isRemote(state.project)) { state.changesState = "remote"; renderChanges(); return; }
    const seq = ++state.changesSeq;
    const epoch = state.epoch;
    state.changesState = "loading";
    renderChanges();
    try {
      if (state.demo) {
        state.files = [
          { path: "src/sample.js", status: "modified", staged: false, unstaged: true, untracked: false },
          { path: "README.sample.md", status: "untracked", staged: false, unstaged: false, untracked: true },
        ];
      } else {
        const result = await invoke("list_git_changes", { path: state.project.path });
        if (!state.open || epoch !== state.epoch || seq !== state.changesSeq) return;
        state.files = Array.isArray(result?.files) ? result.files : [];
      }
      if (!state.open || epoch !== state.epoch || seq !== state.changesSeq) return;
      state.changesState = "ready";
    } catch (error) {
      if (!state.open || epoch !== state.epoch || seq !== state.changesSeq) return;
      state.changesState = "error";
      state.error = error;
      onError?.(error);
    }
    renderChanges();
    loadSelectedDiffIfMissing();
  }

  function renderToolFilter() {
    const select = $("[data-review-tool]");
    const old = select.value;
    const tools = [...new Map(state.sessions.map(item => [item.toolKey, item.toolName || item.toolKey]).filter(([key]) => key)).entries()];
    select.replaceChildren(new Option(translate("review.allTools"), ""));
    tools.forEach(([key, name]) => select.add(new Option(name, key)));
    select.value = tools.some(([key]) => key === old) ? old : "";
  }

  function statusText(status, staged, untracked = false) {
    return translate(gitStatusLabelKey(status, staged, untracked));
  }

  function reasonText(reason) {
    const normalized = String(reason || "").toLowerCase();
    const key = normalized === "native_resume_id_unavailable" ? "review.resumeReason.native_resume_id_unavailable" : `review.resumeReason.${normalized}`;
    return normalized && REVIEW_STRINGS.en[key] ? translate(key) : translate("review.noResume");
  }

  function issueSummary(issues) {
    const optional = [];
    const incomplete = [];
    let auxiliary = false;
    for (const issue of issues) {
      if (issue.code === "auxiliary_filtered") { auxiliary = true; continue; }
      if (["source_unavailable", "invalid_source", "unsupported_format"].includes(issue.code)) optional.push(issue.toolKey);
      else incomplete.push(issue.toolKey);
    }
    return { optional: [...new Set(optional.filter(Boolean))], incomplete: [...new Set(incomplete.filter(Boolean))], auxiliary };
  }

  function appendIssueNotice(notice, key, tools) {
    if (!key) return;
    const message = document.createElement("p");
    message.textContent = tools?.length ? `${translate(key)} (${tools.join(", ")})` : translate(key);
    notice.append(message);
    notice.hidden = false;
  }

  function launchFromButton(control, project, usage, labelKey, pendingKey) {
    if (control.disabled || state.pendingLaunches.size) return;
    const seq = ++state.launchSeq;
    const epoch = state.epoch;
    const currentProject = state.project;
    state.launchError = null;
    state.pendingLaunches.add(pendingKey);
    control.disabled = true;
    control.textContent = translate("review.starting");
    Promise.resolve().then(() => launch?.(project, usage)).then(() => {
      if (!state.open || epoch !== state.epoch || seq !== state.launchSeq || state.project !== currentProject) return;
      state.pendingLaunches.delete(pendingKey);
      close();
    }).catch(error => {
      if (!state.open || epoch !== state.epoch || seq !== state.launchSeq || state.project !== currentProject) return;
      state.pendingLaunches.delete(pendingKey);
      state.launchError = error;
      onError?.(error);
      renderHistory();
    });
  }

  function launchButton(labelKey, project, usage, pendingKey, kind = "") {
    const isPending = state.pendingLaunches.has(pendingKey);
    const control = button(translate(isPending ? "review.starting" : labelKey), event =>
      launchFromButton(event.currentTarget, project, usage, labelKey, pendingKey), kind);
    control.dataset.reviewActionKey = pendingKey;
    control.disabled = state.pendingLaunches.size > 0;
    return control;
  }

  function renderHistory() {
    const focusKey = document.activeElement?.closest?.("[data-review-action-key]")?.dataset.reviewActionKey;
    renderHistoryContent();
    if (focusKey) [...$(`[data-review-list]`).querySelectorAll("[data-review-action-key]")]
      .find(control => control.dataset.reviewActionKey === focusKey)?.focus({ preventScroll: true });
  }

  function renderHistoryContent() {
    if (!root.isConnected) return;
    const notice = $("[data-review-notice]");
    notice.replaceChildren();
    notice.hidden = true;
    const list = $("[data-review-list]");
    list.replaceChildren();
    if (state.demo) { notice.hidden = false; notice.textContent = translate("review.browserDemo"); }
    const preserveNotice = (key, detail = "") => {
      const p = document.createElement("p");
      p.textContent = `${translate(key)}${detail ? ` ${detail}` : ""}`;
      notice.append(p); notice.hidden = false;
    };
    if (state.launchError) preserveNotice("review.launchFailed", String(state.launchError?.message || state.launchError || ""));
    if (state.historyState === "loading") { list.textContent = translate("review.loading"); return; }
    if (state.historyState === "error") {
      addRetry(list, state.error ? `${translate("review.loadFailed")} ${String(state.error?.message || state.error)}` : translate("review.loadFailed"), () => ensureHistory(true));
      return;
    }
    if (state.historyState === "remote") {
      preserveNotice("review.remoteScope");
      (state.project?.toolUsages || []).forEach(usage => {
        const row = document.createElement("article"); row.className = "project-review__session";
        const heading = document.createElement("strong"); heading.textContent = usage.toolName || usage.toolKey || "";
        const copy = document.createElement("div"); copy.className = "project-review__session-copy";
        const meta = document.createElement("p"); meta.textContent = `${usage.sessionCount ?? 0} · ${usage.lastUsedAt || ""}`;
        const activity = document.createElement("p"); activity.className = "project-review__preview"; activity.textContent = translate("review.recordedActivity");
        copy.append(heading, meta, activity); row.append(copy);
        const actions = document.createElement("div"); actions.className = "project-review__actions";
        if (usage.lastSessionId && canLaunch?.(state.project, usage.toolKey)) {
          actions.append(launchButton("review.resume", state.project, {
            toolKey: usage.toolKey, toolName: usage.toolName, sessionCount: 1, lastSessionId: usage.lastSessionId, lastUsedAt: usage.lastUsedAt,
          }, `resume:${usage.toolKey}:${usage.lastSessionId}`, "primary"));
        } else {
          const reason = document.createElement("span"); reason.className = "project-review__reason";
          reason.textContent = reasonText(usage.lastSessionId ? (launchReason?.(state.project, usage.toolKey) || "tool_disabled") : "native_resume_id_unavailable"); actions.append(reason);
        }
        if (canLaunch?.(state.project, usage.toolKey)) actions.append(launchButton("review.newSession", state.project, {
          toolKey: usage.toolKey, toolName: usage.toolName, sessionCount: 0, lastSessionId: null, lastUsedAt: null,
        }, `new:${usage.toolKey}`));
        row.append(actions); list.append(row);
      });
      if (!state.project?.toolUsages?.length) list.append(textNode(translate("review.noSessions")));
      return;
    }
    if (state.issues.length) {
      const summary = issueSummary(state.issues);
      appendIssueNotice(notice, summary.optional.length ? "review.optionalSources" : null, summary.optional);
      appendIssueNotice(notice, summary.incomplete.length ? "review.historyIncomplete" : null, summary.incomplete);
      appendIssueNotice(notice, summary.auxiliary ? "review.auxiliaryFiltered" : null, null);
    }
    if (!state.sessions.length) { list.append(textNode(translate("review.noSessions"))); return; }
    const visible = filterReviewSessions(state.sessions, $("[data-review-search]").value, $("[data-review-tool]").value);
    if (!visible.length) { list.append(textNode(translate("review.noMatches"))); return; }
    visible.forEach(session => {
      const row = document.createElement("article"); row.className = "project-review__session";
      const copy = document.createElement("div"); copy.className = "project-review__session-copy";
      const title = document.createElement("h3"); title.textContent = session.title || session.sessionId || "";
      const meta = document.createElement("p"); meta.textContent = [session.toolName || session.toolKey, session.lastUsedAt, session.sessionId].filter(Boolean).join(" · ");
      copy.append(title, meta);
      if (session.preview) { const preview = document.createElement("p"); preview.className = "project-review__preview"; preview.textContent = session.preview; copy.append(preview); }
      row.append(copy);
      const actions = document.createElement("div"); actions.className = "project-review__actions";
      const allowed = !!session.canResume && !!session.sessionId && !!canLaunch?.(state.project, session.toolKey);
      if (allowed) actions.append(launchButton("review.resume", state.project, {
        toolKey: session.toolKey, toolName: session.toolName, sessionCount: 1, lastSessionId: session.sessionId, lastUsedAt: session.lastUsedAt,
      }, `resume:${session.toolKey}:${session.sessionId}`, "primary"));
      else {
        const reason = document.createElement("span"); reason.className = "project-review__reason";
        reason.textContent = reasonText(session.canResume
          ? (launchReason?.(state.project, session.toolKey) || "tool_disabled")
          : session.reason);
        actions.append(reason);
      }
      if (canLaunch?.(state.project, session.toolKey)) actions.append(launchButton("review.newSession", state.project, {
        toolKey: session.toolKey, toolName: session.toolName, sessionCount: 0, lastSessionId: null, lastUsedAt: null,
      }, `new:${session.toolKey}`));
      row.append(actions); list.append(row);
    });
  }

  function renderChanges() {
    if (!root.isConnected) return;
    const activeFile = document.activeElement?.matches?.(".project-review__file")
      ? { path: document.activeElement.dataset.filePath, staged: document.activeElement.dataset.staged }
      : null;
    const notice = $("[data-change-notice]");
    notice.hidden = true; notice.replaceChildren();
    const files = $("[data-review-files]"); files.replaceChildren();
    const diff = $("[data-review-diff]"); diff.replaceChildren();
    if (state.changesState === "loading") { files.textContent = translate("review.changesLoading"); return; }
    if (state.changesState === "error") { addRetry(files, `${translate("review.loadFailed")} ${String(state.error?.message || state.error || "")}`, () => ensureChanges(true)); return; }
    if (state.changesState === "remote") { notice.hidden = false; notice.textContent = translate("review.remoteDiff"); return; }
    if (!state.files.length) { files.textContent = translate("review.noChanges"); diff.textContent = translate("review.selectFile"); return; }
    for (const file of state.files) {
      for (const staged of [true, false]) {
        if (!(staged ? file.staged : file.unstaged || file.untracked)) continue;
        const item = document.createElement("button"); item.type = "button"; item.className = "project-review__file";
        item.setAttribute("aria-pressed", String(state.selected?.path === file.path && state.selected?.staged === staged));
        item.dataset.filePath = file.path; item.dataset.staged = String(staged);
        const label = document.createElement("span"); label.textContent = file.path;
        const badge = document.createElement("small"); badge.textContent = `${staged ? translate("review.staged") : file.untracked ? translate("review.untracked") : translate("review.unstaged")} · ${statusText(file.status, staged, file.untracked)}`;
        item.append(label, badge); item.addEventListener("click", () => selectDiff(file, staged)); files.append(item);
      }
    }
    renderDiffPane();
    if (activeFile) {
      [...files.querySelectorAll(".project-review__file")]
        .find(item => item.dataset.filePath === activeFile.path && item.dataset.staged === activeFile.staged)
        ?.focus({ preventScroll: true });
    }
  }

  async function selectDiff(file, staged) {
    state.selected = { path: file.path, staged };
    const key = diffKey(state.selected);
    state.diffSeq += 1;
    state.diffLoading = null;
    if (state.diffCache.has(key)) { renderChanges(); return; }
    const seq = state.diffSeq;
    const epoch = state.epoch;
    state.diffLoading = key;
    state.diffErrors.delete(key);
    renderChanges();
    try {
      let result;
      if (state.demo) result = { path: file.path, staged, text: `diff --git a/${file.path} b/${file.path}\n+// synthetic browser sample`, truncated: false, binary: false };
      else result = await invoke("read_git_diff", { path: state.project.path, filePath: file.path, staged });
      if (!state.open || epoch !== state.epoch || seq !== state.diffSeq || state.selected?.path !== file.path || state.selected?.staged !== staged) return;
      state.diffCache.set(key, result || {});
      state.diffLoading = null;
    } catch (error) {
      if (!state.open || epoch !== state.epoch || seq !== state.diffSeq) return;
      onError?.(error);
      state.diffErrors.set(key, error);
      state.diffLoading = null;
    }
    renderChanges();
  }

  function diffKey(selected) { return JSON.stringify([selected.staged, selected.path]); }

  function loadSelectedDiffIfMissing() {
    if (state.mode !== "changes" || state.changesState !== "ready" || !state.selected || state.diffLoading) return;
    const key = diffKey(state.selected);
    if (state.diffCache.has(key) || state.diffErrors.has(key)) return;
    const file = state.files.find(item => item.path === state.selected?.path);
    if (file) void selectDiff(file, state.selected.staged);
  }

  function renderDiffPane() {
    const diff = $("[data-review-diff]");
    diff.replaceChildren();
    if (!state.selected) { diff.textContent = translate("review.selectFile"); return; }
    const key = diffKey(state.selected);
    if (state.diffLoading === key) { diff.textContent = translate("review.diffLoading"); return; }
    if (state.diffErrors.has(key)) {
      const error = state.diffErrors.get(key);
      addRetry(diff, `${translate("review.diffUnavailable")} ${String(error?.message || error)}`, () => {
        const file = state.files.find(item => item.path === state.selected?.path);
        if (file) selectDiff(file, state.selected.staged);
      });
      return;
    }
    const result = state.diffCache.get(key);
    if (!result) { diff.textContent = translate("review.selectFile"); return; }
    const notices = [];
    if (result.binary) notices.push(translate("review.binaryDiff"));
    else if (!result.text) notices.push(translate("review.emptyDiff"));
    if (result.truncated) notices.push(translate("review.truncatedDiff"));
    notices.forEach(message => { const p = document.createElement("p"); p.className = "project-review__diff-notice"; p.textContent = message; diff.append(p); });
    const pre = document.createElement("pre"); pre.className = "project-review__diff-text"; pre.textContent = result.text || "";
    diff.append(pre);
  }

  function addRetry(target, message, retry) { const p = textNode(message); const b = button(translate("review.retry"), retry); target.append(p, b); }
  function textNode(text) { const p = document.createElement("p"); p.className = "project-review__empty"; p.textContent = text; return p; }
  function button(label, action, kind = "") { const b = document.createElement("button"); b.type = "button"; b.className = `project-review__button ${kind ? `project-review__button--${kind}` : ""}`; b.textContent = label; b.addEventListener("click", action); return b; }
  function refreshCapabilities() {
    if (!state.open || state.mode !== "history" || state.pendingLaunches.size || !["ready", "remote"].includes(state.historyState)) return;
    renderHistory();
  }

  function open(project, mode = "history") {
    state.open = true;
    state.previousFocus = document.activeElement;
    state.previousFocusAction = state.previousFocus?.dataset?.projectHistory != null
      ? { kind: "history", projectId: state.previousFocus.dataset.projectHistory }
      : state.previousFocus?.dataset?.projectChanges != null
        ? { kind: "changes", projectId: state.previousFocus.dataset.projectChanges }
        : null;
    const appShell = document.querySelector(".console");
    state.previousInert = !!appShell?.inert;
    if (appShell) appShell.inert = true;
    root.hidden = false;
    resetForProject(project, mode);
    requestAnimationFrame(() => panel.focus());
  }
  function close() {
    if (!state.open) return;
    state.open = false;
    state.epoch += 1;
    state.historySeq += 1;
    state.changesSeq += 1;
    state.diffSeq += 1;
    state.launchSeq += 1;
    root.hidden = true;
    const appShell = document.querySelector(".console");
    if (appShell) appShell.inert = state.previousInert;
    if (state.previousFocus?.isConnected) state.previousFocus.focus?.();
    else if (state.previousFocusAction) {
      const attr = state.previousFocusAction.kind === "history" ? "data-project-history" : "data-project-changes";
      [...document.querySelectorAll(`[${attr}]`)]
        .find(control => control.dataset[state.previousFocusAction.kind === "history" ? "projectHistory" : "projectChanges"] === state.previousFocusAction.projectId)
        ?.focus();
    }
    state.previousFocus = null;
    state.previousFocusAction = null;
  }
  function onKeydown(event) {
    if (!state.open) return;
    if (event.key === "Escape") { event.stopImmediatePropagation(); event.preventDefault(); close(); return; }
    if ((event.key === "ArrowLeft" || event.key === "ArrowRight") && event.target?.matches?.('[role="tab"]')) {
      event.stopImmediatePropagation(); event.preventDefault();
      const index = tabs.indexOf(event.target);
      const next = tabs[(index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length];
      setMode(next.dataset.reviewTab); next.focus(); return;
    }
    if (event.key !== "Tab") return;
    event.stopImmediatePropagation();
    const items = [...panel.querySelectorAll(focusableSelector)].filter(item => item.offsetParent !== null);
    if (!items.length) { event.preventDefault(); panel.focus(); return; }
    const first = items[0], last = items[items.length - 1];
    if (!panel.contains(document.activeElement)) { event.preventDefault(); (event.shiftKey ? last : first).focus(); return; }
    if (event.shiftKey && (document.activeElement === first || document.activeElement === panel)) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel)) { event.preventDefault(); first.focus(); }
  }

  document.addEventListener("keydown", onKeydown, true);
  $("[data-review-close]").addEventListener("click", close);
  $("[data-review-backdrop]").addEventListener("click", close);
  tabs.forEach(tab => tab.addEventListener("click", () => setMode(tab.dataset.reviewTab)));
  $("[data-review-search]").addEventListener("input", renderHistory);
  $("[data-review-tool]").addEventListener("change", renderHistory);
  localize();
  return { open, close, localize, refreshCapabilities };
}
