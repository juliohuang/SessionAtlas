import { expect, test } from "@playwright/test";

test("local and remote TUI settings gate launch and install from an allowlisted action", async ({ page }) => {
  await page.addInitScript(() => {
    const capabilities = enabled => ({
      source: "local",
      serverId: null,
      label: "Local",
      tools: [
        { toolKey: "codex", toolName: "Codex CLI", installed: true, version: "1.2.3", enabled, installAvailable: true, installManager: "npm" },
        { toolKey: "claude", toolName: "Claude Code", installed: false, version: null, enabled: false, installAvailable: true, installManager: "npm" },
        { toolKey: "pi", toolName: "Pi Coding Agent", installed: false, version: null, enabled: false, installAvailable: true, installManager: "npm" },
      ],
    });
    window.__tuiCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [{
            id: "local-one",
            path: "C:\\work\\one",
            name: "one",
            lastAccessedAt: "2026-08-17T00:00:00Z",
            gitBranch: "main",
            toolUsages: [{ toolKey: "codex", toolName: "Codex CLI", lastUsedAt: "2026-08-17T00:00:00Z", sessionCount: 1, lastSessionId: "abc" }],
          }];
          if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
          if (["list_remote_projects", "list_remote_servers", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") return capabilities(true);
          if (command === "set_tui_enabled") {
            window.__tuiCalls.push({ command, payload });
            return capabilities(payload.enabled);
          }
          if (command === "install_tui") {
            window.__tuiCalls.push({ command, payload });
            const result = capabilities(true);
            result.tools[1] = { ...result.tools[1], installed: true, enabled: true, version: "2.0.0" };
            return result;
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
  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="tui"]').click();

  const codex = page.locator('[data-tui-tool="codex"]');
  await expect(codex).toContainText("1.2.3");
  await codex.locator("[data-tui-toggle]").uncheck();
  await expect(codex.locator("[data-tui-toggle]")).not.toBeChecked();

  const claude = page.locator('[data-tui-tool="claude"]');
  await expect(claude).toContainText("Not installed");
  await claude.locator("[data-tui-install]").click();
  await expect(claude).toContainText("2.0.0");

  const pi = page.locator('[data-tui-tool="pi"]');
  await expect(pi).toContainText("Not installed");

  const calls = await page.evaluate(() => window.__tuiCalls);
  expect(calls).toEqual([
    { command: "set_tui_enabled", payload: { serverId: null, toolKey: "codex", enabled: false } },
    { command: "install_tui", payload: { serverId: null, toolKey: "claude" } },
  ]);
});
