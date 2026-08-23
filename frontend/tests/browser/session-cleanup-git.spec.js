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
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [project];
          if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
          if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: true, branch: "main", remotes: [{ name: "origin", url: "https://example.test/one.git" }], localBranches: [{ name: "main", isCurrent: true }], headShort: "abc123", headSummary: "work", dirty: false, upstream: "origin/main", ahead: 1, behind: 0, remoteChecked: false };
          if (command === "refresh_git_info") return { isRepo: true, branch: "main", remotes: [{ name: "origin", url: "https://example.test/one.git" }], localBranches: [{ name: "main", isCurrent: true }], headShort: "abc123", headSummary: "work", dirty: false, upstream: "origin/main", ahead: 2, behind: 3, remoteChecked: true };
          if (command === "analyze_session_cleanup") return {
            scannedAt: "2026-08-17T00:00:00Z",
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
  await expect(page.locator(".entry__git-state--ahead")).toContainText("push 2");
  await expect(page.locator(".entry__git-state--behind")).toContainText("behind 3");

  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="cleanup"]').click();
  await expect(page.locator(".cleanup-row")).toHaveCount(2);
  await expect(page.locator(".cleanup-row.is-protected input")).toBeDisabled();
  await page.locator("[data-cleanup-select-likely]").click();
  await page.locator("[data-cleanup-run]").click();
  expect(await page.evaluate(() => window.__cleanupCalls)).toEqual([{ keys: ["codex:active:child"] }]);
});
