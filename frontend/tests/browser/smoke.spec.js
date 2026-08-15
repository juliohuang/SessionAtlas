import { expect, test } from "@playwright/test";

test("browser demo loads the real static application", async ({ page }) => {
  const errors = [];
  const externalRequests = [];
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("pageerror", error => errors.push(error.message));
  page.on("request", request => {
    const url = new URL(request.url());
    if (url.hostname !== "127.0.0.1") externalRequests.push(request.url());
  });

  await page.goto("/index.html");

  await expect(page.locator("#searchInput")).toBeVisible();
  await expect(page.locator("article.entry")).toHaveCount(7);
  await expect(page.locator("#ledgerCount")).toContainText("7");
  expect(errors).toEqual([]);
  expect(externalRequests).toEqual([]);
});

test("workspace redesign exposes project overview and terminal regions", async ({ page }) => {
  await page.goto("/index.html");

  await expect(page.locator("article.entry")).toHaveCount(7);
  await expect(page).toHaveTitle("SessionAtlas");
  await expect(page.locator(".brand__name")).toHaveText("SessionAtlas");
  await expect(page.locator(".stage__overview")).toBeVisible();
  await expect(page.locator("#termsSelectedLaunch")).toContainText("atlas-notes");
  await expect(page.locator(".overview__activity-row")).toHaveCount(1);
  await expect(page.locator(".overview__session-row")).toHaveCount(1);
  await expect(page.locator(".workspace__head")).toBeVisible();
  await expect(page.locator("#termsEmpty")).toBeVisible();
  await expect(page.locator(".foot > .foot__row")).toHaveCount(1);
  await expect(page.locator(".foot > .foot__row > #footGit")).toBeVisible();

  await page.keyboard.press("Control+k");
  await expect(page.locator("#searchInput")).toBeFocused();
  await page.locator("#searchInput").press("Escape");

  await page.locator('article.entry[data-id="2"]').click();
  await expect(page.locator("#termsSelectedLaunch")).toContainText("terminal-lab");
  await expect(page.locator(".overview__activity-row")).toHaveCount(2);
  await expect(page.locator(".overview__session-row")).toHaveCount(2);
  await page.locator('#termsSelectedLaunch [data-tool="shell"]').click();
  await expect(page.locator("#termsSelectedLaunch")).toContainText("terminal-lab");
  await expect(page.locator("#termsEmpty")).toContainText("terminal-lab");
});

test("mocked Tauri mode publishes backend projects", async ({ page }) => {
  const errors = [];
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(() => {
    const project = {
      id: "mock-project",
      path: "C:\\workspace\\mock-project",
      name: "mock-project",
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [{
        toolKey: "codex",
        toolName: "Codex CLI",
        lastUsedAt: "2026-08-03T00:00:00Z",
        sessionCount: 1,
        lastSessionId: null,
      }],
    };
    window.__invokeCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          window.__invokeCalls.push({ command, payload });
          if (command === "list_projects" || command === "search_projects") return [project];
          if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "list_groups" || command === "list_sort_orders"
              || command === "list_remote_projects" || command === "list_remote_servers"
              || command === "list_opener_prefs" || command === "search_remote_projects") return [];
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: {
        getCurrentWindow: () => ({ isMaximized: async () => false }),
      },
    };
  });

  await page.goto("/index.html");

  await expect(page.locator('article.entry[data-id="mock-project"]')).toBeVisible();
  await expect(page.locator("#ledgerCount")).toContainText("1");
  const calls = await page.evaluate(() => window.__invokeCalls.map(call => call.command));
  expect(calls).toContain("list_projects");
  expect(calls).toContain("list_groups");
  expect(errors).toEqual([]);
});

test("primary reload failure clears the previous search count", async ({ page }) => {
  await page.addInitScript(() => {
    const project = {
      id: "reload-project",
      path: "C:\\workspace\\reload-project",
      name: "reload-project",
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    };
    window.__failList = false;
    window.__TAURI__ = {
      core: {
        invoke: async command => {
          if (command === "list_projects") {
            if (window.__failList) throw new Error("index unavailable");
            return [project];
          }
          if (command === "list_tools" || command === "list_remote_projects"
              || command === "list_remote_servers" || command === "list_opener_prefs"
              || command === "search_remote_projects") return [];
          if (command === "search_projects") return [project];
          if (command === "list_groups" || command === "list_sort_orders"
              || command === "list_group_assignments") return [];
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
  await expect(page.locator('article.entry[data-id="reload-project"]')).toBeVisible();
  await page.locator("#searchInput").fill("reload");
  await expect(page.locator("#ledgerCount")).toContainText("1");
  await page.evaluate(() => { window.__failList = true; });
  await page.locator("#searchInput").press("Escape");

  await expect(page.locator(".ledger__empty-title")).toContainText("Archive unavailable");
  await expect(page.locator("#ledgerCount")).toContainText("0");
  await expect(page.locator("#ledgerCount")).not.toContainText("reload");
});

test("search-view reorder submits the complete catalog including hidden members", async ({ page }) => {
  await page.addInitScript(() => {
    const makeProject = (id, name) => ({
      id,
      path: `C:\\workspace\\${id}`,
      name,
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    });
    const catalog = [
      makeProject("p1", "match-one"),
      makeProject("p2", "match-two"),
      makeProject("p3", "hidden"),
    ];
    window.__invokeCalls = [];
    window.__groupRevision = 0;
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          window.__invokeCalls.push({ command, payload });
          if (command === "list_projects") return catalog;
          if (command === "search_projects") return catalog.filter(project => project.name.includes("match"));
          if (command === "list_tools" || command === "list_remote_projects"
              || command === "list_remote_servers" || command === "list_opener_prefs"
              || command === "search_remote_projects") return [];
          if (command === "list_groups") {
            return [{ id: 1, name: "Group", sortOrder: 10, memberCount: 3 }];
          }
          if (command === "list_group_assignments") return { p1: 1, p2: 1, p3: 1 };
          if (command === "list_sort_orders") {
            return [
              { projectId: "p1", groupKey: "1", sortOrder: 10 },
              { projectId: "p2", groupKey: "1", sortOrder: 20 },
              { projectId: "p3", groupKey: "1", sortOrder: 30 },
            ];
          }
          if (command === "get_group_revision") return window.__groupRevision;
          if (command === "move_group_project") {
            window.__groupRevision += 1;
            return { revision: window.__groupRevision, orderedIds: ["p2", "p1", "p3"] };
          }
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await expect(page.locator("article.entry")).toHaveCount(3);
  await page.locator("#searchInput").fill("match");
  await expect(page.locator("article.entry")).toHaveCount(2);

  await page.evaluate(() => {
    const source = document.querySelector('article.entry[data-id="p2"]');
    const target = document.querySelector('article.entry[data-id="p1"]');
    const transfer = new DataTransfer();
    source.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: transfer }));
    const rect = target.getBoundingClientRect();
    target.dispatchEvent(new DragEvent("drop", {
      bubbles: true,
      clientY: rect.top + 1,
      dataTransfer: transfer,
    }));
  });

  await expect.poll(async () => page.evaluate(() =>
    window.__invokeCalls.find(call => call.command === "move_group_project")?.payload,
  )).toMatchObject({
    projectId: "p2",
    targetGroupKey: "1",
    anchorProjectId: "p1",
    placement: "before",
    catalogIds: ["p1", "p2", "p3"],
    expectedRevision: 0,
  });
});
