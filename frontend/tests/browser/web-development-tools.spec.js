import { expect, test } from "@playwright/test";

test("Web development tools require a connection URL and support configure, open, edit, toggle, and delete", async ({ page }) => {
  await page.addInitScript(() => {
    window.__webDevelopmentTools = [];
    window.__webDevelopmentCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [{
            id: "local-one",
            path: "C:\\work\\one",
            name: "one",
            source: "local",
            osFamily: "windows",
            lastAccessedAt: "2026-08-18T00:00:00Z",
            gitBranch: "main",
            toolUsages: [],
          }];
          if (command === "list_tools") return [];
          if ([
            "list_remote_projects", "list_remote_servers", "list_project_ignores",
            "list_groups", "list_sort_orders", "list_opener_prefs",
          ].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") {
            return { source: "local", serverId: null, label: "Local", tools: [] };
          }
          if (command === "list_web_development_tools") {
            return window.__webDevelopmentTools.map(tool => ({ ...tool }));
          }
          if (command === "upsert_web_development_tool") {
            window.__webDevelopmentCalls.push({ command, payload: { ...payload } });
            const existing = payload.toolId == null
              ? null
              : window.__webDevelopmentTools.find(tool => tool.id === payload.toolId);
            const saved = {
              id: existing?.id ?? 1,
              label: payload.label,
              connectionUrl: payload.connectionUrl,
              enabled: payload.enabled,
              sortOrder: existing?.sortOrder ?? 10,
            };
            window.__webDevelopmentTools = [
              ...window.__webDevelopmentTools.filter(tool => tool.id !== saved.id),
              saved,
            ];
            return { ...saved };
          }
          if (command === "set_web_development_tool_enabled") {
            window.__webDevelopmentCalls.push({ command, payload: { ...payload } });
            const tool = window.__webDevelopmentTools.find(item => item.id === payload.toolId);
            if (tool) tool.enabled = payload.enabled;
            return null;
          }
          if (command === "delete_web_development_tool") {
            window.__webDevelopmentCalls.push({ command, payload: { ...payload } });
            window.__webDevelopmentTools = window.__webDevelopmentTools.filter(tool => tool.id !== payload.toolId);
            return null;
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
  const connectionUrl = new URL("/dsh", page.url()).href;

  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="webDevelopment"]').click();
  await expect(page.locator("#webDevelopmentNewForm")).toBeVisible();
  await page.locator('#webDevelopmentNewForm [name="label"]').fill("DSH");
  await page.locator('#webDevelopmentNewForm [name="connectionUrl"]').fill("file:///tmp/dsh");
  await page.locator("#webDevelopmentNewForm").evaluate(form => form.requestSubmit());
  await expect(page.locator(".web-tool-row")).toHaveCount(0);
  await page.locator('#webDevelopmentNewForm [name="connectionUrl"]').fill(connectionUrl);
  await page.locator("#webDevelopmentNewForm").evaluate(form => form.requestSubmit());

  const row = page.locator('[data-web-development-id="1"]');
  await expect(row).toBeVisible();
  await expect(row.locator('[name="label"]')).toHaveValue("DSH");
  await row.locator("[data-web-development-open]").click();
  await expect(page.locator(".web-pane__title")).toHaveText("DSH");
  await expect(page.locator(".web-pane iframe")).toHaveAttribute("src", connectionUrl);

  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="webDevelopment"]').click();
  const editableRow = page.locator('[data-web-development-id="1"]');
  await editableRow.locator('[name="label"]').fill("DSH staging");
  await editableRow.evaluate(form => form.requestSubmit());
  await expect(page.locator('[data-web-development-id="1"] [name="label"]')).toHaveValue("DSH staging");

  await page.locator('[data-web-development-id="1"] [data-web-development-toggle]').uncheck();
  await expect(page.locator('[data-web-development-id="1"] [data-web-development-toggle]')).not.toBeChecked();
  await page.locator('[data-web-development-id="1"] [data-web-development-delete]').click();
  await expect(page.locator('[data-web-development-id="1"]')).toHaveCount(0);

  const calls = await page.evaluate(() => window.__webDevelopmentCalls);
  expect(calls).toEqual([
    {
      command: "upsert_web_development_tool",
      payload: { toolId: null, label: "DSH", connectionUrl, enabled: true },
    },
    {
      command: "upsert_web_development_tool",
      payload: { toolId: 1, label: "DSH staging", connectionUrl, enabled: true },
    },
    {
      command: "set_web_development_tool_enabled",
      payload: { toolId: 1, enabled: false },
    },
    {
      command: "delete_web_development_tool",
      payload: { toolId: 1 },
    },
  ]);
});
