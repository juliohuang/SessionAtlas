import { expect, test } from "@playwright/test";

test("disabled installed adapters can be enabled again on local and remote machines", async ({ page }) => {
  await page.addInitScript(() => {
    const localServerId = null;
    const remoteServerId = 42;

    const makeTool = (overrides = {}) => ({
      toolKey: "codex",
      toolName: "Codex CLI",
      installed: true,
      version: "1.2.3",
      enabled: false,
      supported: true,
      supportError: null,
      adapterEnabled: false,
      adapterVersion: "1.0.0",
      adapterSource: "bundled",
      adapterNewestVersion: "1.0.0",
      adapterUpdateAvailable: false,
      adapterRollbackVersion: null,
      installAvailable: true,
      installManager: "npm",
      installPackage: "@openai/codex",
      latestVersion: null,
      updateChecked: false,
      updateAvailable: false,
      updateCheckError: null,
      ...overrides,
    });

    const makeCapabilities = (serverId, label) => ({
      source: serverId == null ? "local" : "remote",
      serverId,
      label,
      // Both rows intentionally start installed but disabled. The custom row
      // models an imported adapter whose executable was installed manually and
      // whose manifest has no package-manager installer.
      tools: [
        makeTool(),
        makeTool({
          toolKey: "custom-agent",
          toolName: "Imported Agent",
          version: "0.3.0",
          adapterEnabled: false,
          adapterVersion: "0.3.0",
          adapterSource: "local",
          installAvailable: false,
          installManager: "",
          installPackage: "",
        }),
      ],
      adapterDiagnostics: [],
    });

    window.__tuiCalls = [];
    window.__tuiCapabilities = {
      local: makeCapabilities(localServerId, "Local"),
      remote: makeCapabilities(remoteServerId, "Build host"),
    };
    const clone = value => JSON.parse(JSON.stringify(value));
    const machineKey = serverId => serverId == null ? "local" : "remote";
    const capabilityFor = serverId => window.__tuiCapabilities[machineKey(serverId)];

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          window.__tuiCalls.push({ command, payload: { ...payload } });
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [];
          if (command === "list_tools") return [
            { toolKey: "codex", toolName: "Codex CLI" },
            { toolKey: "custom-agent", toolName: "Imported Agent" },
          ];
          if (command === "list_remote_projects") return [];
          if (command === "list_remote_servers") return [{
            id: remoteServerId,
            label: "Build host",
            user: "demo",
            host: "build-host.test",
            port: 22,
            identityFile: null,
            scanRoots: "~",
            osFamily: "linux",
            lastScannedAt: null,
          }];
          if ([
            "list_project_ignores",
            "list_groups",
            "list_sort_orders",
            "list_opener_prefs",
            "list_group_assignments",
          ].includes(command)) return command === "list_group_assignments" ? {} : [];
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "list_web_development_tools") return [];
          if (command === "probe_tui_capabilities") {
            return clone(capabilityFor(payload.serverId == null ? null : Number(payload.serverId)));
          }
          if (command === "check_tui_updates") {
            const capabilities = capabilityFor(payload.serverId == null ? null : Number(payload.serverId));
            return clone({
              ...capabilities,
              tools: capabilities.tools.map(tool => ({
                ...tool,
                latestVersion: tool.version,
                updateChecked: tool.installed,
              })),
            });
          }
          if (command === "set_tui_enabled") {
            const serverId = payload.serverId == null ? null : Number(payload.serverId);
            const capabilities = capabilityFor(serverId);
            const tool = capabilities.tools.find(item => item.toolKey === payload.toolKey);
            if (tool) {
              tool.adapterEnabled = Boolean(payload.enabled);
              tool.enabled = Boolean(payload.enabled);
            }
            return clone(capabilities);
          }
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="tui"]').click();

  const local = page.locator('[data-tui-machine="local"]');
  const remote = page.locator('[data-tui-machine="remote:42"]');
  await expect(local).toBeVisible();
  await expect(remote).toBeVisible();

  const localCodex = local.locator('[data-tui-tool="codex"]');
  const localCustom = local.locator('[data-tui-tool="custom-agent"]');
  const remoteCodex = remote.locator('[data-tui-tool="codex"]');
  const remoteCustom = remote.locator('[data-tui-tool="custom-agent"]');

  // A disabled official adapter is still surfaced by the probe and its
  // executable-backed toggle remains available.
  await expect(localCodex.locator("[data-tui-toggle]")).toBeEnabled();
  await expect(localCodex.locator("[data-tui-toggle]")).not.toBeChecked();

  // Imported adapters with no installer use the same enablement path once
  // their executable was detected as installed.
  await expect(localCustom.locator("[data-tui-toggle]")).toBeEnabled();
  await expect(localCustom.locator("[data-tui-install]")).toHaveCount(0);
  await expect(localCustom.locator("[data-tui-toggle]")).not.toBeChecked();

  for (const row of [localCodex, localCustom, remoteCodex, remoteCustom]) {
    const toggle = row.locator("[data-tui-toggle]");
    await expect(toggle).toBeEnabled();
    await expect(toggle).not.toBeChecked();
    await toggle.check();
    await expect(row.locator("[data-tui-toggle]")).toBeChecked();
    await row.locator("[data-tui-toggle]").uncheck();
    await expect(row.locator("[data-tui-toggle]")).not.toBeChecked();
    await row.locator("[data-tui-toggle]").check();
    await expect(row.locator("[data-tui-toggle]")).toBeChecked();
  }

  const calls = await page.evaluate(() => window.__tuiCalls);
  expect(calls.filter(call => call.command === "probe_tui_capabilities").map(call => call.payload.serverId))
    .toEqual(expect.arrayContaining([null, 42]));
  expect(calls.filter(call => call.command === "set_tui_enabled").map(call => ({
    serverId: call.payload.serverId,
    toolKey: call.payload.toolKey,
    enabled: call.payload.enabled,
  }))).toEqual([
    { serverId: null, toolKey: "codex", enabled: true },
    { serverId: null, toolKey: "codex", enabled: false },
    { serverId: null, toolKey: "codex", enabled: true },
    { serverId: null, toolKey: "custom-agent", enabled: true },
    { serverId: null, toolKey: "custom-agent", enabled: false },
    { serverId: null, toolKey: "custom-agent", enabled: true },
    { serverId: 42, toolKey: "codex", enabled: true },
    { serverId: 42, toolKey: "codex", enabled: false },
    { serverId: 42, toolKey: "codex", enabled: true },
    { serverId: 42, toolKey: "custom-agent", enabled: true },
    { serverId: 42, toolKey: "custom-agent", enabled: false },
    { serverId: 42, toolKey: "custom-agent", enabled: true },
  ]);
});
