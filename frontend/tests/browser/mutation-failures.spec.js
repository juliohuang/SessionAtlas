import { expect, test } from "@playwright/test";

async function installMutationFixture(page) {
  await page.addInitScript(() => {
    const project = {
      id: "p1",
      path: "C:\\workspace\\p1",
      name: "project-one",
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    };
    window.__rejectCommands = [];
    window.__invokeCalls = [];
    window.__remoteServers = [];
    window.__tmuxProbe = {
      home: "/home/tester",
      tmuxAvailable: true,
      tmuxVersion: "tmux 3.4",
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          window.__invokeCalls.push({ command, payload });
          if (window.__rejectCommands.includes(command)) throw new Error(`${command} denied`);
          if (command === "list_projects" || command === "search_projects") return [project];
          if (command === "list_tools" || command === "list_remote_projects"
              || command === "search_remote_projects") return [];
          if (command === "list_remote_servers") return window.__remoteServers;
          if (command === "test_remote_connection") return window.__tmuxProbe;
          if (command === "add_remote_server") {
            const server = {
              id: 11,
              label: payload.label,
              user: payload.user,
              host: payload.host,
              port: payload.port || 22,
              identityFile: payload.identityFile,
              scanRoots: [],
            };
            window.__remoteServers = [server];
            return server;
          }
          if (command === "list_opener_prefs") {
            return [{
              id: 7,
              type: "custom",
              builtinKey: null,
              label: "Editor",
              command: "editor {path}",
              enabled: true,
              sortOrder: 10,
            }];
          }
          if (command === "list_groups") {
            return [{ id: 3, name: "Original", sortOrder: 10, memberCount: 1 }];
          }
          if (command === "list_group_assignments") return { p1: 3 };
          if (command === "list_sort_orders") return [];
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

async function openSettingsView(page, view) {
  await page.locator("#settingsBtn").click();
  await page.locator(`[data-settings-view="${view}"]`).click();
}

test("typing in a settings form does not trigger the Escape-only drawer handler", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");

  const label = page.locator('#serverForm [name="label"]');
  await label.pressSequentially("E2E remote server");

  await expect(label).toHaveValue("E2E remote server");
  await expect(page.locator("#drawerTitle")).toHaveText("Remote servers");
  await expect(page.locator("#drawer")).toBeVisible();

  await label.press("Escape");
  await expect(page.locator('[data-settings-view="remote"]')).toBeVisible();
  await expect(page.locator("#drawer")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#drawer")).toBeHidden();
});

test("failed opener toggle restores the checkbox without replacing the ledger", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();
  await openSettingsView(page, "openers");
  await page.evaluate(() => window.__rejectCommands.push("set_opener_enabled"));

  const checkbox = page.locator('[data-opener-toggle]');
  await expect(checkbox).toBeChecked();
  await checkbox.click();

  await expect(checkbox).toBeChecked();
  await expect(page.locator("#footStatus")).toContainText("set_opener_enabled denied");
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();
});

test("failed opener create and delete preserve the draft and existing row", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "openers");
  await page.evaluate(() => window.__rejectCommands.push("upsert_custom_opener", "delete_custom_opener"));

  await page.locator('#customForm [name="label"]').fill("Draft opener");
  await page.locator('#customForm [name="command"]').fill("draft {path}");
  await page.locator('#customForm [type="submit"]').click();

  await expect(page.locator('#customForm [name="label"]')).toHaveValue("Draft opener");
  await expect(page.locator('#customForm [name="command"]')).toHaveValue("draft {path}");
  await expect(page.locator('.opener-row[data-id="7"]')).toBeVisible();

  await page.locator('.opener-row[data-id="7"] [data-opener-del]').click();
  await expect(page.locator('.opener-row[data-id="7"]')).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("delete_custom_opener denied");
});

test("failed group rename and create preserve canonical name and form draft", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "groups");
  await page.evaluate(() => window.__rejectCommands.push("rename_group", "create_group"));

  const rename = page.locator('.group-row[data-id="3"] [data-group-rename]');
  await rename.fill("Unsaved rename");
  await expect(rename).toHaveClass(/is-unsaved/);
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();

  await page.locator('#groupForm [name="name"]').fill("Draft group");
  await page.locator('#groupForm [type="submit"]').click();
  await expect(page.locator('#groupForm [name="name"]')).toHaveValue("Draft group");
  await expect(page.locator('.group-row[data-id="3"]')).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("create_group denied");
});

test("server add followed by scan failure is explicit partial success", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");
  await page.evaluate(() => window.__rejectCommands.push("scan_remote_server"));

  await page.locator('#serverForm [name="label"]').fill("Remote A");
  await page.locator('#serverForm [name="user"]').fill("tester");
  await page.locator('#serverForm [name="host"]').fill("example.test");
  await page.locator('#serverForm [type="submit"]').click();

  await expect(page.locator("#footStatus")).toContainText("was added, but its initial scan failed");
  await expect(page.locator('.server-row[data-id="11"]')).toBeVisible();
  await expect(page.locator('#serverForm [name="label"]')).toHaveValue("");
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();
});

test("server add warns when the remote machine has no tmux", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");
  await page.evaluate(() => {
    window.__tmuxProbe = {
      home: "/home/tester",
      tmuxAvailable: false,
      tmuxVersion: null,
    };
  });

  await page.locator('#serverForm [name="label"]').fill("Remote without tmux");
  await page.locator('#serverForm [name="user"]').fill("tester");
  await page.locator('#serverForm [name="host"]').fill("example.test");
  await page.locator('#serverForm [type="submit"]').click();

  await expect(page.locator("#footStatus")).toContainText("tmux is not installed");
  await expect(page.locator("#footStatus")).toContainText("sudo apt install tmux");
  await expect(page.locator('.server-row[data-id="11"]')).toBeVisible();
});

test("successful write followed by reconciliation failure keeps last-known-good rows", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "groups");
  await page.evaluate(() => window.__rejectCommands.push("list_groups"));

  await page.locator('#groupForm [name="name"]').fill("Committed group");
  await page.locator('#groupForm [type="submit"]').click();
  await expect(page.locator('.group-row[data-id="3"]')).toBeVisible();
  await expect(page.locator('#groupForm [name="name"]')).toHaveValue("");
  await expect(page.locator("#footStatus")).toContainText("stale: groups");

  await page.locator("#drawerBackBtn").click();
  await page.locator('[data-settings-view="openers"]').click();
  await page.evaluate(() => {
    window.__rejectCommands = window.__rejectCommands.filter(command => command !== "list_groups");
    window.__rejectCommands.push("list_opener_prefs");
  });
  await page.locator('#customForm [name="label"]').fill("Committed opener");
  await page.locator('#customForm [name="command"]').fill("cmd {path}");
  await page.locator('#customForm [type="submit"]').click();
  await expect(page.locator('.opener-row[data-id="7"]')).toBeVisible();
  await expect(page.locator('#customForm [name="label"]')).toHaveValue("");
});
