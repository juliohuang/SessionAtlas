import { expect, test } from "@playwright/test";

test("session cleanup is review-first and Git sync warnings refresh in background", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const project = {
      id: "project-one",
      path: "C:\\work\\one",
      name: "one",
      source: "local",
      lastAccessedAt: "2026-08-17T00:00:00Z",
      gitBranch: "main",
      toolUsages: [{ toolKey: "codex", toolName: "Codex CLI", lastUsedAt: "2026-08-17T00:00:00Z", sessionCount: 2, lastSessionId: "parent" }],
    };
    window.__cleanupCalls = [];
    window.__gitCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          if (command === "get_git_info" || command === "refresh_git_sync") {
            window.__gitCalls.push(command);
          }
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [project];
          if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
          if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: true, branch: "main", remotes: [{ name: "origin", url: "https://example.test/one.git" }], localBranches: [{ name: "main", isCurrent: true }], headShort: "abc123", headSummary: "work", dirty: false, upstream: "origin/main", ahead: 1, behind: 0, remoteChecked: false };
          if (command === "refresh_git_sync") return { branch: "main", headShort: "abc123", dirty: false, upstream: "origin/main", ahead: 2, behind: 3, remoteChecked: true, fetchError: null };
          if (command === "analyze_session_cleanup") return {
            scannedAt: "2026-08-17T00:00:00Z",
            snapshotId: "v1-demo",
            supportedTools: ["codex", "claude"],
            candidates: [
              { key: "codex:active:child", toolKey: "codex", sessionId: "child", parentSessionId: "parent", title: "guardian audit", cliVersion: "0.139.0", agentKind: "guardian", classification: "likely", reasons: ["guardianDelivered"], protections: [], ageDays: 47, sizeBytes: 1024, userTurns: 1, toolCalls: 2, canClean: true },
              { key: "codex:active:root", toolKey: "codex", sessionId: "root", parentSessionId: null, title: "latest project session", cliVersion: "0.139.0", agentKind: "root", classification: "possible", reasons: ["oldVersion"], protections: ["latestForProject"], ageDays: 40, sizeBytes: 2048, userTurns: 4, toolCalls: 9, canClean: false },
            ],
          };
          if (command === "list_session_trash") return [];
          if (command === "quarantine_session_candidates") {
            window.__cleanupCalls.push(payload);
            return { batchId: "demo", createdAt: new Date().toISOString(), sessionCount: payload.keys.length, sizeBytes: 1024 };
          }
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  page.on("dialog", dialog => dialog.accept());
  await page.goto("/index.html");
  await expect(page.locator("#footGitSync")).toContainText("behind 3");
  expect(await page.evaluate(() => window.__gitCalls)).toEqual(["get_git_info", "refresh_git_sync"]);
  await expect(page.locator(".entry__git-state--ahead")).toContainText("push 2");
  await expect(page.locator(".entry__git-state--behind")).toContainText("behind 3");

  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="cleanup"]').click();
  await expect(page.locator(".cleanup-row")).toHaveCount(2);
  await expect(page.locator(".cleanup-row.is-protected input")).toBeDisabled();
  await page.locator("[data-cleanup-select-likely]").click();
  await page.locator("[data-cleanup-run]").click();
  expect(await page.evaluate(() => window.__cleanupCalls)).toEqual([{ snapshotId: "v1-demo", keys: ["codex:active:child"] }]);
});

test("Git quick snapshot replaces the previous project before delayed sync completes", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const projects = [
      { id: "project-one", path: "C:\\work\\one", name: "one", source: "local", lastAccessedAt: "2026-08-17T00:00:00Z", toolUsages: [] },
      { id: "project-two", path: "C:\\work\\two", name: "two", source: "local", lastAccessedAt: "2026-08-16T00:00:00Z", toolUsages: [] },
    ];
    window.__resolveGitSync = {};
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return projects;
          if (command === "list_tools") return [];
          if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") {
            const second = String(payload.path).endsWith("two");
            return {
              isRepo: true,
              branch: second ? "two" : "one",
              remotes: [{ name: "origin", url: `https://example.test/${second ? "two" : "one"}.git` }],
              localBranches: [{ name: second ? "two" : "one", isCurrent: true }],
              headShort: second ? "222222" : "111111",
              headSummary: second ? "second project" : "first project",
              dirty: false,
              upstream: "origin/main",
              ahead: 0,
              behind: 0,
              remoteChecked: false,
            };
          }
          if (command === "refresh_git_sync") {
            return new Promise(resolve => {
              const key = String(payload.path).endsWith("one") ? "one" : "two";
              window.__resolveGitSync[key] = () => resolve({
                branch: key,
                headShort: key === "one" ? "111111" : "222222",
                dirty: false,
                upstream: "origin/main",
                ahead: 0,
                behind: key === "one" ? 1 : 4,
                remoteChecked: true,
                fetchError: null,
              });
            });
          }
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await expect(page.locator("#footGitBranch")).toContainText("one");
  await page.locator('#ledger article.entry[data-id="project-two"]').click();
  await expect(page.locator("#footGitBranch")).toContainText("two");
  await expect(page.locator("#footGitHead")).toContainText("second project");
  await expect(page.locator("#footGitHead")).not.toContainText("first project");
  await page.locator('#ledger article.entry[data-id="project-one"]').click();
  await expect(page.locator("#footGitBranch")).toContainText("one");
  await expect(page.locator("#footGitHead")).toContainText("first project");
  await expect(page.locator("#footGitHead")).not.toContainText("second project");
  await page.evaluate(() => {
    window.__resolveGitSync.one?.();
    window.__resolveGitSync.two?.();
  });
  await expect(page.locator("#footGitSync")).toContainText("behind 1");
  await page.locator('#ledger article.entry[data-id="project-two"]').click();
  await expect(page.locator("#footGitSync")).toContainText("behind 4");
});

test("opening Claude stays independent while the background Git read is pending", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const project = {
      id: "project-one",
      path: "C:\\work\\one",
      name: "one",
      source: "local",
      lastAccessedAt: "2026-08-17T00:00:00Z",
      toolUsages: [{
        toolKey: "claude",
        toolName: "Claude Code",
        lastUsedAt: "2026-08-17T00:00:00Z",
        sessionCount: 1,
        lastSessionId: "session-one",
      }],
    };
    window.__gitReadStarted = false;
    window.__gitReadSettled = false;
    window.__ptyCalls = [];
    window.__resolveGitRead = null;
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [project];
          if (command === "list_tools") return [{ toolKey: "claude", toolName: "Claude Code" }];
          if ([
            "list_remote_projects", "list_remote_servers", "list_project_ignores",
            "list_groups", "list_sort_orders", "list_opener_prefs",
            "list_web_development_tools",
          ].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "probe_tui_capabilities") {
            return {
              source: "local",
              serverId: null,
              label: "Local",
              tools: [{
                toolKey: "claude",
                toolName: "Claude Code",
                installed: true,
                version: "test",
                enabled: true,
                adapterEnabled: true,
                installAvailable: true,
                installManager: "npm",
              }],
            };
          }
          if (command === "get_git_info") {
            window.__gitReadStarted = true;
            return new Promise(resolve => {
              window.__resolveGitRead = () => {
                window.__gitReadSettled = true;
                resolve({ isRepo: false, remotes: [], localBranches: [], remoteChecked: true });
              };
            });
          }
          if (["pty_spawn", "pty_attach"].includes(command)) {
            window.__ptyCalls.push({ command, payload });
            return command === "pty_spawn" ? 301 : null;
          }
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await expect.poll(() => page.evaluate(() => window.__gitReadStarted)).toBe(true);
  await expect(page.locator('#termsSelectedLaunch [data-tool="claude"]')).toBeEnabled();
  await page.locator('#termsSelectedLaunch [data-tool="claude"]').click();

  await expect.poll(() => page.evaluate(() => window.__ptyCalls.map(call => call.command)))
    .toContain("pty_spawn");
  expect(await page.evaluate(() => window.__gitReadSettled)).toBe(false);

  await page.evaluate(() => window.__resolveGitRead?.());
  await expect.poll(() => page.evaluate(() => window.__gitReadSettled)).toBe(true);
});
