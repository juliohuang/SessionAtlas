import { expect, test } from "@playwright/test";

test("remote failure preserves last-known-good while successful empty clears it", async ({ page }) => {
  await page.addInitScript(() => {
    const project = (id, source = "local") => ({
      id,
      source,
      path: `${source}:${id}`,
      name: id,
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      remoteServerId: source === "remote" ? 4 : null,
      toolUsages: [],
    });
    window.__remoteFailure = false;
    window.__remoteProjects = [project("remote-one", "remote")];
    window.__TAURI__ = {
      core: {
        invoke: async (command) => {
          if (command === "list_projects") return [project("local-one")];
          if (command === "search_projects") return [];
          if (command === "list_remote_projects") {
            if (window.__remoteFailure) throw new Error("remote offline");
            return window.__remoteProjects;
          }
          if (command === "search_remote_projects") {
            if (window.__remoteFailure) throw new Error("remote search offline");
            return window.__remoteProjects;
          }
          if (command === "list_remote_servers") {
            return [{ id: 4, label: "Remote", user: "u", host: "h", port: 22 }];
          }
          if (command === "list_tools" || command === "list_groups"
              || command === "list_sort_orders" || command === "list_opener_prefs") return [];
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

  await page.goto("/index.html");
  await expect(page.locator('article.entry[data-id="remote-one"]')).toBeVisible();

  await page.evaluate(() => { window.__remoteFailure = true; });
  await page.locator("#scanBtn").click();
  await expect(page.locator('article.entry[data-id="remote-one"]')).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("stale: remote");

  await page.evaluate(() => {
    window.__remoteFailure = false;
    window.__remoteProjects = [];
  });
  await page.locator("#scanBtn").click();
  await expect(page.locator('article.entry[data-id="remote-one"]')).toHaveCount(0);
  await expect(page.locator('article.entry[data-id="local-one"]')).toBeVisible();
});

test("failed remote search never reuses results from the previous query", async ({ page }) => {
  await page.addInitScript(() => {
    const remote = {
      id: "remote-q1",
      source: "remote",
      path: "ssh:q1",
      name: "q1-result",
      lastAccessedAt: "2026-08-03T00:00:00Z",
      remoteServerId: 1,
      toolUsages: [],
    };
    window.__failRemoteSearch = false;
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          if (command === "list_projects" || command === "search_projects") return [];
          if (command === "list_remote_projects") return [];
          if (command === "search_remote_projects") {
            if (window.__failRemoteSearch) throw new Error("offline");
            return payload.query === "q1" ? [remote] : [];
          }
          if (command === "list_tools" || command === "list_remote_servers"
              || command === "list_groups" || command === "list_sort_orders"
              || command === "list_opener_prefs") return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await page.locator("#searchInput").fill("q1");
  await expect(page.locator('article.entry[data-id="remote-q1"]')).toBeVisible();
  await page.evaluate(() => { window.__failRemoteSearch = true; });
  await page.locator("#searchInput").fill("q2");
  await expect(page.locator('article.entry[data-id="remote-q1"]')).toHaveCount(0);
  await expect(page.locator("#footStatus")).toContainText("stale: remote");
});
