import { expect, test } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

test("project review loads and filters sessions, then previews staged and unstaged diffs", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const project = {
      id: "review-project", name: "Review project", path: "C:\\work\\review", source: "local",
      lastAccessedAt: "2026-10-15T00:00:00Z", toolUsages: [{ toolKey: "codex", toolName: "Codex CLI", lastUsedAt: "2026-10-15T00:00:00Z", sessionCount: 2, lastSessionId: "session-1" }],
    };
    window.__reviewCalls = [];
    window.__TAURI__ = {
      core: { invoke: async (command, payload = {}) => {
        window.__reviewCalls.push({ command, payload });
        if (command === "local_index_exists") return true;
        if (command === "list_projects") return [project];
        if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
        if (command === "probe_tui_capabilities") return { source: "local", serverId: null, label: "Local", tools: [{ toolKey: "codex", toolName: "Codex CLI", installed: true, enabled: true }] };
        if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
        if (command === "list_group_assignments") return {};
        if (command === "get_group_revision") return 0;
        if (command === "list_project_sessions") return { sessions: [
          { toolKey: "codex", toolName: "Codex CLI", sessionId: "session-1", title: "Release plan", preview: "Check staged changes", lastUsedAt: "2026-10-15T00:00:00Z", canResume: true, reason: null },
          { toolKey: "codex", toolName: "Codex CLI", sessionId: "session-2", title: "Bug triage", preview: "Review unstaged work", lastUsedAt: "2026-10-14T00:00:00Z", canResume: true, reason: null },
        ], issues: [] };
        if (command === "list_git_changes") return { files: [{ path: "src/app.js", status: "modified", staged: true, unstaged: true, untracked: false }] };
        if (command === "read_git_diff") return { path: payload.filePath, staged: payload.staged, text: `diff -- ${payload.staged ? "staged" : "unstaged"}`, truncated: false, binary: false };
        if (command === "get_git_info") return { isRepo: true, branch: "main", remotes: [], localBranches: [], dirty: true, upstream: null, ahead: 0, behind: 0, remoteChecked: false };
        if (command === "pty_spawn") return { tabId: 1 };
        return null;
      } },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await page.setViewportSize({ width: 980, height: 640 });
  await page.locator('.entry[data-id="review-project"]').click();
  const opener = page.locator('[data-project-history="review-project"]');
  await opener.click();
  const dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(".project-review__session")).toHaveCount(2);
  await dialog.screenshot({ path: resolve(fileURLToPath(new URL("../../test-results/project-review-history.png", import.meta.url))) });
  await dialog.locator("[data-review-search]").fill("triage");
  await expect(dialog.locator(".project-review__session")).toHaveCount(1);
  await expect(dialog.locator(".project-review__session h3")).toHaveText("Bug triage");

  await dialog.locator('[data-review-tab="changes"]').click();
  await dialog.screenshot({ path: resolve(fileURLToPath(new URL("../../test-results/project-review-changes.png", import.meta.url))) });
  await dialog.locator('[data-file-path="src/app.js"][data-staged="true"]').click();
  await expect(dialog.locator(".project-review__diff-text")).toContainText("diff -- staged");
  await expect(dialog.locator('[data-file-path="src/app.js"][data-staged="true"]')).toBeFocused();
  await dialog.locator('[data-file-path="src/app.js"][data-staged="false"]').click();
  await expect(dialog.locator(".project-review__diff-text")).toContainText("diff -- unstaged");
  await expect(dialog.locator('[data-file-path="src/app.js"][data-staged="false"]')).toBeFocused();
  await dialog.locator('[data-review-tab="history"]').click();
  await dialog.locator('[data-review-tab="changes"]').click();
  await expect(dialog.locator(".project-review__diff-text")).toContainText("diff -- unstaged");
  expect(await page.evaluate(() => window.__reviewCalls.filter(call => call.command === "read_git_diff").map(call => call.payload.staged))).toEqual([true, false]);

  await page.evaluate(() => document.activeElement.blur());
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
});

test("remote review reports its scope without invoking local project history or diff commands", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    window.__reviewCalls = [];
    const remote = { id: "remote-review", name: "Remote project", path: "/srv/work/repo", source: "remote", remoteServerId: 9, lastAccessedAt: "2026-10-15T00:00:00Z", toolUsages: [{ toolKey: "codex", toolName: "Codex CLI", sessionCount: 3, lastSessionId: "remote-native-3", lastUsedAt: "2026-10-15T00:00:00Z" }] };
    window.__TAURI__ = { core: { invoke: async (command, payload = {}) => {
      window.__reviewCalls.push(command);
      if (command === "local_index_exists") return true;
      if (command === "list_projects") return [];
      if (command === "list_remote_projects") return [remote];
      if (command === "list_remote_servers") return [{ id: 9, label: "Review server", user: "tester", host: "example.test", port: 22, identityFile: null }];
      if (command === "probe_tui_capabilities") return { source: "remote", serverId: 9, label: "Remote", tools: [{ toolKey: "codex", toolName: "Codex CLI", installed: true, enabled: true }] };
      if (["list_tools", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
      if (command === "list_group_assignments") return {};
      if (command === "get_group_revision") return 0;
      return null;
    } }, event: { listen: async () => () => {} }, window: { getCurrentWindow: () => ({ isMaximized: async () => false }) } };
  });
  await page.goto("/index.html");
  await page.locator('.entry[data-id="remote-review"]').click();
  await page.locator('[data-project-history="remote-review"]').click();
  const dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await expect(dialog.locator("[data-review-notice]")).toContainText("Remote session history is not available");
  await expect(dialog.locator(".project-review__session")).toContainText("Recorded tool activity");
  await expect(dialog.getByRole("button", { name: "Resume" })).toBeVisible();
  await dialog.locator('[data-review-tab="changes"]').click();
  await expect(dialog.locator("[data-change-notice]")).toContainText("not available for remote projects");
  expect(await page.evaluate(() => window.__reviewCalls.filter(command => ["list_project_sessions", "list_git_changes", "read_git_diff"].includes(command)))).toEqual([]);
});

test("resume preflight uses the selected historical session ID and remains retryable on failure", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const project = { id: "identity-project", name: "Identity project", path: "C:\\work\\identity", source: "local", lastAccessedAt: "2026-10-15T00:00:00Z", toolUsages: [{ toolKey: "codex", toolName: "Codex CLI", sessionCount: 2, lastSessionId: "latest-index-id" }] };
    window.__resumeChecks = [];
    window.__TAURI__ = { core: { invoke: async (command, payload = {}) => {
      if (command === "local_index_exists") return true;
      if (command === "list_projects") return [project];
      if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
      if (command === "probe_tui_capabilities") return { source: "local", serverId: null, label: "Local", tools: [{ toolKey: "codex", toolName: "Codex CLI", installed: true, enabled: true }] };
      if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
      if (command === "list_group_assignments") return {};
      if (command === "get_group_revision") return 0;
      if (command === "list_project_sessions") return { sessions: [{ toolKey: "codex", toolName: "Codex CLI", sessionId: "historical-old-sid", title: "<img src=x onerror=window.__xss=true> Historical session", preview: "Safe preview", lastUsedAt: "2026-10-14T00:00:00Z", canResume: true, reason: null }], issues: [] };
      if (command === "check_session_resume") { window.__resumeChecks.push(payload); return { ready: false, reason: "session_missing", detail: "source file not found" }; }
      return null;
    } }, event: { listen: async () => () => {} }, window: { getCurrentWindow: () => ({ isMaximized: async () => false }) } };
  });
  await page.goto("/index.html");
  await page.locator('.entry[data-id="identity-project"]').click();
  await page.locator('[data-project-history="identity-project"]').click();
  const dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  const title = dialog.locator(".project-review__session h3");
  await expect(title).toContainText("Historical session");
  await expect(title.locator("img")).toHaveCount(0);
  await dialog.getByRole("button", { name: "Resume" }).click();
  await expect(dialog.locator("[data-review-notice]")).toContainText("session is missing from the source data");
  const resume = dialog.getByRole("button", { name: "Resume" });
  await expect(resume).toBeEnabled();
  await resume.click();
  expect(await page.evaluate(() => window.__resumeChecks.map(call => call.sessionId))).toEqual(["historical-old-sid", "historical-old-sid"]);
  expect(await page.evaluate(() => window.__xss)).toBeUndefined();
});

test("stale session-history responses are discarded after closing and selecting another project", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const projects = [
      { id: "slow-project", name: "Slow", path: "C:\\work\\slow", source: "local", lastAccessedAt: "2026-10-15T00:00:00Z", toolUsages: [] },
      { id: "fast-project", name: "Fast", path: "C:\\work\\fast", source: "local", lastAccessedAt: "2026-10-14T00:00:00Z", toolUsages: [] },
    ];
    window.__historyResolvers = {};
    window.__TAURI__ = { core: { invoke: async (command, payload = {}) => {
      if (command === "local_index_exists") return true;
      if (command === "list_projects") return projects;
      if (command === "list_tools") return [];
      if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
      if (command === "list_group_assignments") return {};
      if (command === "get_group_revision") return 0;
      if (command === "list_project_sessions") {
        if (payload.path.endsWith("slow")) return new Promise(resolve => { window.__historyResolvers.slow = resolve; });
        return { sessions: [{ toolKey: "codex", sessionId: "fast", title: "Fast result", preview: "new", lastUsedAt: "2026-10-15T00:00:00Z", canResume: false, reason: "unsupported_resume" }], issues: [] };
      }
      return null;
    } }, event: { listen: async () => () => {} }, window: { getCurrentWindow: () => ({ isMaximized: async () => false }) } };
  });
  await page.goto("/index.html");
  await page.locator('.entry[data-id="slow-project"]').click();
  await page.locator('[data-project-history="slow-project"]').click();
  const dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await expect(dialog.locator("[data-review-list]")).toContainText("Loading");
  await page.keyboard.press("Escape");
  await page.locator('.entry[data-id="fast-project"]').click();
  await page.locator('[data-project-history="fast-project"]').click();
  await expect(dialog.locator(".project-review__session h3")).toHaveText("Fast result");
  await page.evaluate(() => window.__historyResolvers.slow({ sessions: [{ toolKey: "codex", sessionId: "stale", title: "Stale result", preview: "old", lastUsedAt: "2026-10-13T00:00:00Z", canResume: false, reason: "unsupported_resume" }], issues: [] }));
  await expect(dialog.locator(".project-review__session h3")).toHaveText("Fast result");
  await expect(dialog).toContainText("C:\\work\\fast");
});

test("historical session IDs open distinct PTYs and duplicate resume reuses the exact same tab", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const project = { id: "pty-review", name: "PTY review", path: "C:\\work\\pty-review", source: "local", lastAccessedAt: "2026-10-15T00:00:00Z", toolUsages: [{ toolKey: "codex", toolName: "Codex CLI", sessionCount: 2 }] };
    window.__ptyReviewCalls = [];
    let nextPty = 100;
    window.__TAURI__ = { core: { invoke: async (command, payload = {}) => {
      window.__ptyReviewCalls.push({ command, payload });
      if (command === "local_index_exists") return true;
      if (command === "list_projects") return [project];
      if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
      if (command === "probe_tui_capabilities") return { source: "local", serverId: null, label: "Local", tools: [{ toolKey: "codex", toolName: "Codex CLI", installed: true, enabled: true }] };
      if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
      if (command === "list_group_assignments") return {};
      if (command === "get_group_revision") return 0;
      if (command === "list_project_sessions") return { sessions: [
        { toolKey: "codex", toolName: "Codex CLI", sessionId: "old-session-a", title: "Older session A", preview: "A", lastUsedAt: "2026-10-14T00:00:00Z", canResume: true, reason: null },
        { toolKey: "codex", toolName: "Codex CLI", sessionId: "old-session-b", title: "Older session B", preview: "B", lastUsedAt: "2026-10-13T00:00:00Z", canResume: true, reason: null },
      ], issues: [] };
      if (command === "check_session_resume") return { ready: true, reason: "ready", detail: "", verified: true };
      if (command === "pty_spawn") return ++nextPty;
      if (command === "pty_attach") return null;
      return null;
    } }, event: { listen: async () => () => {} }, window: { getCurrentWindow: () => ({ isMaximized: async () => false }) } };
  });
  await page.goto("/index.html");
  await page.locator('.entry[data-id="pty-review"]').click();
  const opener = page.locator('[data-project-history="pty-review"]');
  await opener.click();
  let dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await dialog.locator('.project-review__session').filter({ hasText: "Older session A" }).getByRole("button", { name: "Resume" }).click();
  await expect(page.locator(".term-tab")).toHaveCount(1);
  await opener.click();
  dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await dialog.locator('.project-review__session').filter({ hasText: "Older session A" }).getByRole("button", { name: "Resume" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.locator(".term-tab")).toHaveCount(1);
  await opener.click();
  dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await dialog.locator('.project-review__session').filter({ hasText: "Older session B" }).getByRole("button", { name: "Resume" }).click();
  await expect(page.locator(".term-tab")).toHaveCount(2);
  const calls = await page.evaluate(() => window.__ptyReviewCalls);
  expect(calls.filter(call => call.command === "check_session_resume").map(call => call.payload.sessionId)).toEqual(["old-session-a", "old-session-a", "old-session-b"]);
  expect(calls.filter(call => call.command === "pty_attach").map(call => call.payload.sessionId)).toEqual(["old-session-a", "old-session-b"]);
  expect(calls.filter(call => call.command === "pty_spawn")).toHaveLength(2);
});

test("resume action appears when the delayed tool capability probe completes", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const project = { id: "delayed-capability", name: "Delayed capability", path: "C:\\work\\delayed", source: "local", toolUsages: [] };
    window.__resolveReviewCapabilities = null;
    window.__TAURI__ = { core: { invoke: async (command, payload = {}) => {
      if (command === "local_index_exists") return true;
      if (command === "list_projects") return [project];
      if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
      if (command === "probe_tui_capabilities") return new Promise(resolve => { window.__resolveReviewCapabilities = resolve; });
      if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
      if (command === "list_group_assignments") return {};
      if (command === "get_group_revision") return 0;
      if (command === "list_project_sessions") return { sessions: [{ toolKey: "codex", toolName: "Codex CLI", sessionId: "delayed-session", title: "Capability arrives later", preview: "history loaded first", lastUsedAt: "2026-10-15T00:00:00Z", canResume: true, reason: null }], issues: [] };
      return null;
    } }, event: { listen: async () => () => {} }, window: { getCurrentWindow: () => ({ isMaximized: async () => false }) } };
  });
  await page.goto("/index.html");
  await page.locator('.entry[data-id="delayed-capability"]').click();
  await page.locator('[data-project-history="delayed-capability"]').click();
  const dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await expect(dialog.locator(".project-review__session h3")).toHaveText("Capability arrives later");
  await expect(dialog.locator(".project-review__reason")).toContainText("Checking tool availability");
  await expect(dialog.locator(".project-review__reason")).not.toContainText("review.resumeReason");
  await expect(dialog.getByRole("button", { name: "Resume" })).toHaveCount(0);
  await page.evaluate(() => window.__resolveReviewCapabilities({ source: "local", serverId: null, label: "Local", tools: [{ toolKey: "codex", toolName: "Codex CLI", installed: true, enabled: true }] }));
  await expect(dialog.getByRole("button", { name: "Resume" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(page.locator('[data-project-history="delayed-capability"]')).toBeFocused();
});

for (const capabilityCase of [
  { name: "missing", capability: { installed: false, enabled: true }, expected: "The tool is unavailable" },
  { name: "disabled", capability: { installed: true, enabled: false }, expected: "This tool is disabled" },
]) {
  test(`history resume state explains a ${capabilityCase.name} tool`, async ({ page }) => {
    await page.addInitScript(({ capability }) => {
      localStorage.setItem("sessionatlas.lang", "en");
      const project = { id: "tool-state-review", name: "Tool state review", path: "C:\\work\\tool-state", source: "local", toolUsages: [] };
      window.__TAURI__ = { core: { invoke: async command => {
        if (command === "local_index_exists") return true;
        if (command === "list_projects") return [project];
        if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
        if (command === "probe_tui_capabilities") return { source: "local", serverId: null, label: "Local", tools: [{ toolKey: "codex", toolName: "Codex CLI", ...capability }] };
        if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
        if (command === "list_group_assignments") return {};
        if (command === "get_group_revision") return 0;
        if (command === "list_project_sessions") return { sessions: [{ toolKey: "codex", toolName: "Codex CLI", sessionId: "historical-valid-id", title: "Valid historical session", preview: "Can resume if installed", lastUsedAt: "2026-10-15T00:00:00Z", canResume: true, reason: null }], issues: [] };
        return null;
      } }, event: { listen: async () => () => {} }, window: { getCurrentWindow: () => ({ isMaximized: async () => false }) } };
    }, { capability: capabilityCase.capability });
    await page.goto("/index.html");
    await page.locator('.entry[data-id="tool-state-review"]').click();
    await page.locator('[data-project-history="tool-state-review"]').click();
    const dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
    await expect(dialog.locator(".project-review__session h3")).toHaveText("Valid historical session");
    await expect(dialog.locator(".project-review__reason")).toContainText(capabilityCase.expected);
    await expect(dialog.locator(".project-review__reason")).not.toContainText("review.resumeReason");
    await expect(dialog.locator(".project-review__actions .project-review__button")).toHaveCount(0);
  });
}

test("Chinese dark-theme history snapshot uses localized review labels", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "zh");
    localStorage.setItem("sessionatlas.theme", "dark");
    const project = { id: "localized-review", name: "Localized review", path: "C:\\work\\localized", source: "local", toolUsages: [] };
    window.__TAURI__ = { core: { invoke: async command => {
      if (command === "local_index_exists") return true;
      if (command === "list_projects") return [project];
      if (command === "list_tools") return [];
      if (command === "probe_tui_capabilities") return { source: "local", serverId: null, label: "Local", tools: [] };
      if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
      if (command === "list_group_assignments") return {};
      if (command === "get_group_revision") return 0;
      if (command === "list_project_sessions") return { sessions: [{ toolKey: "codex", toolName: "Codex CLI", sessionId: "localized-session", title: "安全的历史会话", preview: "示例内容", lastUsedAt: "2026-10-15T00:00:00Z", canResume: false, reason: "unsupported_resume" }], issues: [] };
      return null;
    } }, event: { listen: async () => () => {} }, window: { getCurrentWindow: () => ({ isMaximized: async () => false }) } };
  });
  await page.goto("/index.html");
  await page.setViewportSize({ width: 980, height: 640 });
  await page.locator('.entry[data-id="localized-review"]').click();
  await page.locator('[data-project-history="localized-review"]').click();
  const dialog = page.locator('.project-review [role="dialog"][aria-modal="true"]');
  await expect(dialog.locator(".project-review__head h2")).toHaveText("项目审阅");
  await expect(dialog.locator("[data-review-tab=history]")).toHaveText("会话历史");
  await dialog.screenshot({ path: resolve(fileURLToPath(new URL("../../test-results/project-review-zh-dark-history.png", import.meta.url))) });
});
