import { expect, test } from "@playwright/test";

test("toolbar update notices require a successful local check", async ({ page }, testInfo) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "zh");
    const tools = [
      { toolKey: "codex", toolName: "Codex CLI", installed: true, updateChecked: true, updateAvailable: true, latestVersion: "2.0.0" },
      { toolKey: "claude", toolName: "Claude Code", installed: true, updateChecked: false, updateAvailable: true },
      { toolKey: "pi", toolName: "Pi Coding Agent", installed: true, updateChecked: true, updateAvailable: true, updateCheckError: "offline" },
      { toolKey: "kimi", toolName: "Kimi CLI", installed: false, updateChecked: true, updateAvailable: true },
      { toolKey: "aider", toolName: "Aider", installed: true, updateChecked: true, updateAvailable: false },
    ].map(tool => ({ enabled: true, version: "1.0.0", ...tool }));
    const capabilities = { source: "local", label: "Local", tools };
    window.__updateCalls = 0;
    window.__TAURI__ = {
      core: { invoke: async command => {
        if (command === "local_index_exists") return true;
        if (command === "list_projects") return [];
        if (command === "list_tools") return tools.map(({ toolKey, toolName }) => ({ toolKey, toolName }));
        if (command === "probe_tui_capabilities") return { ...capabilities, tools: tools.map(tool => ({ ...tool, updateChecked: false })) };
        if (command === "check_tui_updates") { window.__updateCalls++; return capabilities; }
        if (command === "list_group_assignments") return {};
        if (command === "get_group_revision") return 0;
        return [];
      } },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });
  await page.goto("/index.html");
  await expect(page.locator("[data-tool-update]")).toHaveCount(1);
  await expect(page.locator('[data-tool-update="codex"]')).toHaveText("可更新");
  await expect(page.locator('[data-tool-update="codex"]')).toHaveAttribute("title", /本机.*1\.0\.0.*2\.0\.0/);
  await page.locator('#filters [data-tool="codex"]').click();
  await expect(page.locator('#filters [data-tool="codex"]')).toHaveClass(/is-active/);
  expect(await page.evaluate(() => window.__updateCalls)).toBe(1);
  await page.locator("#filters").screenshot({ path: testInfo.outputPath("updates.png"), animations: "disabled" });
});

test("local and remote TUI settings gate launch and install from an allowlisted action", async ({ page }, testInfo) => {
  await page.addInitScript(() => {
    window.__codexAdapter = {
      adapterEnabled: true,
      adapterVersion: "1.1.0",
      adapterSource: "local",
      adapterNewestVersion: "1.1.0",
      adapterUpdateAvailable: false,
      adapterRollbackVersion: "1.0.0",
    };
    const bundledAdapter = {
      adapterEnabled: false,
      adapterVersion: "1.0.0",
      adapterSource: "bundled",
      adapterNewestVersion: "1.0.0",
      adapterUpdateAvailable: false,
      adapterRollbackVersion: null,
    };
    const capabilities = enabled => ({
      source: "local",
      serverId: null,
      label: "Local",
      tools: [
        { toolKey: "codex", toolName: "Codex CLI", installed: true, version: "1.2.3", enabled, ...window.__codexAdapter, installAvailable: true, installManager: "npm", installPackage: "@openai/codex", latestVersion: null, updateChecked: false, updateAvailable: false, updateCheckError: null },
        { toolKey: "claude", toolName: "Claude Code", installed: false, version: null, enabled: false, ...bundledAdapter, installAvailable: true, installManager: "npm", installPackage: "@anthropic-ai/claude-code", latestVersion: null, updateChecked: false, updateAvailable: false, updateCheckError: null },
        { toolKey: "pi", toolName: "Pi Coding Agent", installed: false, version: null, enabled: false, ...bundledAdapter, installAvailable: true, installManager: "npm", installPackage: "@earendil-works/pi-coding-agent", latestVersion: null, updateChecked: false, updateAvailable: false, updateCheckError: null },
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
            result.tools[1] = { ...result.tools[1], installed: true, enabled: true, adapterEnabled: true, version: "2.0.0" };
            return result;
          }
          if (command === "rollback_tui_adapter") {
            window.__tuiCalls.push({ command, payload });
            window.__codexAdapter = {
              ...window.__codexAdapter,
              adapterVersion: "1.0.0",
              adapterSource: "bundled",
              adapterUpdateAvailable: true,
              adapterRollbackVersion: null,
            };
            return capabilities(true);
          }
          if (command === "activate_tui_adapter_update") {
            window.__tuiCalls.push({ command, payload });
            window.__codexAdapter = {
              ...window.__codexAdapter,
              adapterVersion: "1.1.0",
              adapterSource: "local",
              adapterUpdateAvailable: false,
              adapterRollbackVersion: "1.0.0",
            };
            return capabilities(true);
          }
          if (command === "install_tui_adapter") {
            window.__tuiCalls.push({ command, payload });
            return capabilities(true);
          }
          if (command === "check_tui_updates") {
            window.__tuiCalls.push({ command, payload });
            const result = capabilities(true);
            result.tools[0] = { ...result.tools[0], latestVersion: "1.3.0", updateChecked: true, updateAvailable: true };
            result.tools[1] = { ...result.tools[1], installed: true, enabled: true, adapterEnabled: true, version: "2.0.0", latestVersion: "2.0.0", updateChecked: true };
            if (window.__tuiCalls.filter(call => call.command === "check_tui_updates").length === 1) {
              result.tools[1] = capabilities(true).tools[1];
            }
            return new Promise(resolve => {
              window.__finishTuiCheck = () => resolve(result);
            });
          }
          if (command === "upgrade_tui") {
            window.__tuiCalls.push({ command, payload });
            const result = capabilities(true);
            result.tools[0] = { ...result.tools[0], version: "1.3.0", latestVersion: "1.3.0", updateChecked: true, updateAvailable: false };
            result.tools[1] = { ...result.tools[1], installed: true, enabled: true, adapterEnabled: true, version: "2.0.0", latestVersion: "2.0.0", updateChecked: true };
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
  await expect.poll(() => page.evaluate(() => typeof window.__finishTuiCheck)).toBe("function");
  await expect(page.locator('[data-tool-update="codex"]')).toHaveCount(0);
  await page.evaluate(() => window.__finishTuiCheck());
  const toolbarUpdate = page.locator('[data-tool-update="codex"]');
  await expect(toolbarUpdate).toBeVisible();
  await expect(toolbarUpdate).toHaveAttribute("title", /1\.2\.3.*1\.3\.0/);
  await toolbarUpdate.click();
  await expect(page.locator('[data-tui-machine="local"]')).toBeVisible();
  await expect(page.locator('#filters [data-tool="all"]')).toHaveClass(/is-active/);

  const codex = page.locator('[data-tui-tool="codex"]');
  await expect(codex).toContainText("1.2.3");
  await codex.locator("[data-tui-toggle]").uncheck();
  await expect(codex.locator("[data-tui-toggle]")).not.toBeChecked();

  const claude = page.locator('[data-tui-tool="claude"]');
  await expect(claude).toContainText("Not installed");
  await expect(claude.locator("[data-tui-toggle]")).toBeDisabled();
  await claude.locator("[data-tui-install]").click();
  await page.locator('.tool-confirm button[value="confirm"]').click();
  await expect(claude).toContainText("2.0.0");

  await codex.locator("[data-tui-adapter-rollback]").click();
  await expect(codex).toContainText("Adapter 1.0.0");
  await codex.locator("[data-tui-adapter-activate]").click();
  await expect(codex).toContainText("Adapter 1.1.0");

  await page.locator('#tuiAdapterForm [name="manifestPath"]').fill("C:\\tmp\\adapter.json");
  await page.locator("#tuiAdapterForm").evaluate(form => form.requestSubmit());
  await expect(page.locator('#tuiAdapterForm [name="manifestPath"]')).toHaveValue("");

  const pi = page.locator('[data-tui-tool="pi"]');
  await expect(pi).toContainText("Not installed");

  const checkUpdates = page.locator('[data-tui-machine="local"] [data-tui-check-updates]');
  await checkUpdates.click();
  await expect(checkUpdates).toBeDisabled();
  await expect(checkUpdates).toContainText("CHECKING");
  await page.evaluate(() => window.__finishTuiCheck());
  await expect(codex).toContainText("Version 1.3.0 is available");
  await codex.locator("[data-tui-upgrade]").click();
  const confirmation = page.locator("dialog.tool-confirm");
  await expect(confirmation).toBeVisible();
  await expect(confirmation).toContainText("1.2.3");
  await expect(confirmation).toContainText("1.3.0");
  await expect(confirmation.locator('[value="cancel"]')).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(confirmation.locator('[value="confirm"]')).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(confirmation.locator("summary")).toBeFocused();
  await confirmation.screenshot({ path: testInfo.outputPath("upgrade-confirm.png"), animations: "disabled" });
  await page.keyboard.press("Escape");
  await expect(confirmation).toHaveCount(0);
  await expect(codex).toBeVisible();
  expect(await page.evaluate(() => window.__tuiCalls.filter(call => call.command === "upgrade_tui"))).toEqual([]);
  await codex.locator("[data-tui-upgrade]").click();
  await page.locator('.tool-confirm button[value="confirm"]').click();
  await expect(codex).toContainText("Latest version 1.3.0");
  await expect(toolbarUpdate).toHaveCount(0);

  const calls = await page.evaluate(() => window.__tuiCalls);
  expect(calls).toEqual([
    { command: "check_tui_updates", payload: { serverId: null } },
    { command: "set_tui_enabled", payload: { serverId: null, toolKey: "codex", enabled: false } },
    { command: "install_tui", payload: { serverId: null, toolKey: "claude" } },
    { command: "rollback_tui_adapter", payload: { toolKey: "codex" } },
    { command: "activate_tui_adapter_update", payload: { toolKey: "codex" } },
    { command: "install_tui_adapter", payload: { manifestPath: "C:\\tmp\\adapter.json" } },
    { command: "check_tui_updates", payload: { serverId: null } },
    { command: "upgrade_tui", payload: { serverId: null, toolKey: "codex" } },
  ]);
});

test("TUI probes are lazy, globally bounded, and TTL cached across remote machines", async ({ page }) => {
  await page.addInitScript(() => {
    const servers = [1, 2, 3].map(id => ({
      id,
      label: `server-${id}`,
      user: "demo",
      host: `server-${id}.test`,
      port: 22,
      identityFile: null,
      scanRoots: "~",
      osFamily: "linux",
      lastScannedAt: null,
    }));
    window.__probeCalls = [];
    window.__probeActive = 0;
    window.__probeMax = 0;
    window.__probeResolvers = {};
    const capabilities = serverId => ({
      source: serverId == null ? "local" : "remote",
      serverId,
      label: serverId == null ? "Local" : `server-${serverId}`,
      tools: [{ toolKey: "codex", toolName: "Codex CLI", installed: true, enabled: true, version: "1.0.0", installAvailable: true, installManager: "npm", installPackage: "@openai/codex" }],
    });
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [{ id: "local", path: "C:\\work\\local", name: "local", source: "local", lastAccessedAt: "2026-08-17T00:00:00Z", toolUsages: [] }];
          if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
          if (command === "list_remote_servers") return servers;
          if (["list_remote_projects", "list_project_ignores", "list_groups", "list_sort_orders", "list_opener_prefs"].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") {
            const serverId = payload.serverId == null ? null : Number(payload.serverId);
            window.__probeCalls.push(serverId);
            window.__probeActive += 1;
            window.__probeMax = Math.max(window.__probeMax, window.__probeActive);
            if (serverId == null) {
              window.__probeActive -= 1;
              return capabilities(serverId);
            }
            return new Promise(resolve => {
              window.__probeResolvers[serverId] = () => {
                window.__probeActive -= 1;
                resolve(capabilities(serverId));
              };
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
  await expect.poll(() => page.evaluate(() => window.__probeCalls.includes(null))).toBe(true);
  expect(await page.evaluate(() => window.__probeCalls.filter(id => id !== null))).toEqual([]);

  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="tui"]').click();
  await expect.poll(() => page.evaluate(() => window.__probeCalls.filter(id => id !== null).length)).toBe(2);
  expect(await page.evaluate(() => window.__probeMax)).toBeLessThanOrEqual(2);
  await page.evaluate(() => {
    window.__probeResolvers[1]?.();
    window.__probeResolvers[2]?.();
  });
  await expect.poll(() => page.evaluate(() => window.__probeCalls.filter(id => id !== null).length)).toBe(3);
  await page.evaluate(() => window.__probeResolvers[3]?.());
  await expect.poll(() => page.locator('[data-tui-machine]').count()).toBe(4);
  const beforeCachedReentry = await page.evaluate(() => window.__probeCalls.length);
  await page.locator("#drawerBackBtn").click();
  await page.locator('[data-settings-view="tui"]').click();
  await expect.poll(() => page.evaluate(() => window.__probeCalls.length)).toBe(beforeCachedReentry);

  const beforeRefresh = await page.evaluate(() => window.__probeCalls.length);
  await page.locator('[data-tui-machine="remote:1"] [data-tui-refresh]').click();
  await expect.poll(() => page.evaluate(() => window.__probeCalls.length)).toBe(beforeRefresh + 1);
  expect(await page.evaluate(() => window.__probeMax)).toBeLessThanOrEqual(2);
});
