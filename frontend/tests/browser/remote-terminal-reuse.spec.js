import { expect, test } from "@playwright/test";

async function installRemoteTerminalFixture(page) {
  await page.addInitScript(() => {
    const usage = (toolKey, lastSessionId) => ({
      toolKey,
      toolName: toolKey === "codex" ? "Codex CLI" : "Claude Code",
      lastUsedAt: "2026-08-16T00:00:00Z",
      sessionCount: 1,
      lastSessionId,
    });
    window.__remoteProjects = [
      {
        id: "remote-one",
        path: "/srv/project-one",
        name: "project-one",
        lastAccessedAt: "2026-08-16T00:00:00Z",
        gitBranch: "main",
        remoteServerId: 7,
        toolUsages: [usage("codex", "codex-session")],
      },
      {
        id: "remote-two",
        path: "/srv/project-two",
        name: "project-two",
        lastAccessedAt: "2026-08-15T00:00:00Z",
        gitBranch: "feature/two",
        remoteServerId: 7,
        toolUsages: [usage("claude", "claude-session")],
      },
    ];
    window.__invokeCalls = [];
    window.__rejectRemoteSwitch = false;
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          window.__invokeCalls.push({ command, payload });
          if (command === "list_projects" || command === "search_projects") return [];
          if (command === "list_remote_projects" || command === "search_remote_projects") {
            return window.__remoteProjects;
          }
          if (command === "list_remote_servers") {
            return [{
              id: 7,
              label: "development-box",
              user: "tester",
              host: "192.0.2.7",
              port: 22,
              identityFile: null,
            }];
          }
          if (command === "list_tools") {
            return [
              { toolKey: "codex", toolName: "Codex CLI" },
              { toolKey: "claude", toolName: "Claude Code" },
            ];
          }
          if (command === "pty_spawn") return 101;
          if (command === "pty_remote_switch" && window.__rejectRemoteSwitch) {
            throw new Error("switch rejected");
          }
          if (command === "list_groups" || command === "list_sort_orders"
              || command === "list_opener_prefs") return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });
}

test("same remote server reuses one SSH PTY and switches its tmux target", async ({ page }) => {
  await installRemoteTerminalFixture(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="remote-one"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="codex"]').click();
  await expect(page.locator(".term-tab")).toHaveCount(1);
  await expect(page.locator(".term-tab__name")).toHaveText("project-one · codex");

  await page.locator('article.entry[data-id="remote-two"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="claude"]').click();

  await expect(page.locator(".term-tab")).toHaveCount(1);
  await expect(page.locator(".term-tab__name")).toHaveText("project-two · claude");
  await expect(page.locator("#footStatus")).toContainText("development-box");

  const calls = await page.evaluate(() => window.__invokeCalls);
  expect(calls.filter(call => call.command === "pty_spawn")).toHaveLength(1);
  expect(calls.filter(call => call.command === "pty_attach")).toHaveLength(1);
  const switches = calls.filter(call => call.command === "pty_remote_switch");
  expect(switches).toEqual([{
    command: "pty_remote_switch",
    payload: {
      id: 101,
      path: "/srv/project-two",
      serverId: 7,
      toolKey: "claude",
      sessionId: "claude-session",
    },
  }]);
});

test("failed remote tmux switch keeps the original tab target", async ({ page }) => {
  await installRemoteTerminalFixture(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="remote-one"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="codex"]').click();
  await expect(page.locator(".term-tab__name")).toHaveText("project-one · codex");

  await page.evaluate(() => { window.__rejectRemoteSwitch = true; });
  await page.locator('article.entry[data-id="remote-two"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="claude"]').click();

  await expect(page.locator(".term-tab")).toHaveCount(1);
  await expect(page.locator(".term-tab__name")).toHaveText("project-one · codex");
  await expect(page.locator("#footStatus")).toContainText("switch rejected");
});
