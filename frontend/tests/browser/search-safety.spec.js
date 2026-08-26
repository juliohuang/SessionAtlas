import { expect, test } from "@playwright/test";

const payloads = [
  "&<>\"'",
  "<style>body{display:none}</style><iframe src=https://example.com>",
  '<img src=x onerror="document.body.dataset.injected=1">',
];

for (const language of ["en", "zh"]) {
  for (const payload of payloads) {
    test(`search query is text-only in ${language}: ${payload.slice(0, 12)}`, async ({ page }) => {
      await page.addInitScript(lang => {
        localStorage.setItem("sessionatlas.lang", lang);
      }, language);
      await page.goto("/index.html");
      const initialDisplay = await page.locator("body").evaluate(element => getComputedStyle(element).display);

      await page.locator("#searchInput").fill(payload);
      await expect(page.locator("#ledgerCount")).toContainText(payload);

      const count = page.locator("#ledgerCount");
      await expect(count.locator("style, iframe, img, script")).toHaveCount(0);
      expect(await count.evaluate(element =>
        [...element.querySelectorAll("*")].some(node =>
          [...node.attributes].some(attribute => attribute.name.startsWith("on")),
        ),
      )).toBe(false);
      expect(await page.locator("body").evaluate(element => getComputedStyle(element).display))
        .toBe(initialDisplay);
      expect(await page.locator("body").getAttribute("data-injected")).toBeNull();
    });
  }
}

test("matching search result also renders the query as text", async ({ page }) => {
  await page.goto("/index.html");
  await page.locator("#searchInput").fill("terminal-lab");
  await expect(page.locator("article.entry")).toHaveCount(1);
  await expect(page.locator("#ledgerCount")).toContainText("terminal-lab");
  await expect(page.locator("#ledgerCount").locator("style, iframe, img, script")).toHaveCount(0);
});

test("opaque project IDs stay inert and addressable across entry actions", async ({ page }) => {
  const projectId = String.raw`x"><img src=x onerror="document.body.dataset.injected='1'">[]\# ,:`;
  const ordinaryProjectId = "ordinary-project";
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.addInitScript(id => {
    localStorage.setItem("sessionatlas.projectOrder", "grouped");
    const project = {
      id,
      source: "local",
      path: "C:\\workspace\\opaque-id",
      name: "opaque-id",
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    };
    const ordinaryProject = {
      id: "ordinary-project",
      source: "local",
      path: "C:\\workspace\\ordinary-project",
      name: "ordinary-project",
      lastAccessedAt: "2026-08-02T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    };
    window.__invokeCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          window.__invokeCalls.push({ command, payload });
          if (command === "local_index_exists") return true;
          if (command === "list_projects" || command === "search_projects") return [project, ordinaryProject];
          if (command === "list_tools" || command === "list_remote_projects"
              || command === "search_remote_projects" || command === "list_remote_servers"
              || command === "list_project_ignores" || command === "list_sort_orders"
              || command === "list_web_development_tools") return [];
          if (command === "list_project_docs" || command === "list_dir") return [];
          if (command === "list_groups") return [{ id: 1, name: "Active", sortOrder: 10, memberCount: 2 }];
          if (command === "list_group_assignments") {
            const assignments = Object.create(null);
            assignments[id] = 1;
            assignments["ordinary-project"] = 1;
            return assignments;
          }
          if (command === "list_opener_prefs") return [{ id: 7, label: "Open folder", command: "explorer {path}", enabled: true }];
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") {
            return { source: "local", serverId: null, label: "Local", tools: [] };
          }
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  }, projectId);

  await page.goto("/index.html");
  const initialUnsafeCount = await page.locator("img, script, iframe").count();
  const entries = page.locator("article.entry");
  await expect(entries).toHaveCount(2);
  const entry = entries.filter({ hasText: "opaque-id" });
  const ordinaryEntry = entries.filter({ hasText: "ordinary-project" });
  await expect(entry).toHaveCount(1);
  await expect(ordinaryEntry).toHaveCount(1);
  expect(await entry.getAttribute("data-id")).toBe(projectId);
  expect(await entry.locator("style, iframe, img, script")).toHaveCount(0);
  expect(await entry.evaluate(element =>
    [...element.attributes].some(attribute => attribute.name.startsWith("on")),
  )).toBe(false);
  expect(await page.locator("body").getAttribute("data-injected")).toBeNull();
  expect(await page.locator("img, script, iframe").count()).toBe(initialUnsafeCount);

  await entry.click();
  await expect(entry).toHaveClass(/is-selected/);
  await entry.locator("[data-expand-toggle]").click();
  await expect(entry).toHaveClass(/is-expanded/);

  await page.locator("#searchInput").press("ArrowDown");
  await expect(ordinaryEntry).toHaveClass(/is-selected/);
  expect(await page.locator("article.entry.is-selected").evaluate(element => element.dataset.id)).toBe(ordinaryProjectId);
  await page.waitForTimeout(50);
  await page.locator("#searchInput").press("ArrowUp");
  await page.waitForTimeout(50);
  await expect(page.locator("article.entry.is-selected")).toHaveCount(1);
  expect(await page.locator("article.entry.is-selected").evaluate(element => element.dataset.id)).toBe(projectId);

  await entry.locator("[data-menu-toggle]").click();
  const menu = page.locator("#entryModal");
  await expect(menu).toBeVisible();
  await expect(menu.locator("style, iframe, img, script")).toHaveCount(0);
  expect(await menu.evaluate(element =>
    [...element.querySelectorAll("*")].some(node =>
      [...node.attributes].some(attribute => attribute.name.startsWith("on")),
    ),
  )).toBe(false);
  expect(await menu.locator("[data-project-id]").first().getAttribute("data-project-id")).toBe(projectId);

  await menu.locator("[data-group-picker]").selectOption("1");
  await expect.poll(() => page.evaluate(() =>
    window.__invokeCalls.find(call => call.command === "assign_project_to_group")?.payload.projectId,
  )).toBe(projectId);
  await menu.locator(".launch-pill--ext").click();
  await expect.poll(() => page.evaluate(() =>
    window.__invokeCalls.find(call => call.command === "open_with_opener")?.payload,
  )).toEqual({ openerId: 7, path: "C:\\workspace\\opaque-id" });
  expect(await page.locator("img, script, iframe").count()).toBe(initialUnsafeCount);
  expect(errors).toEqual([]);
});

test("__proto__ remains an ordinary project ID key", async ({ page }) => {
  const projectId = "__proto__";
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.addInitScript(id => {
    localStorage.setItem("sessionatlas.projectOrder", "grouped");
    const project = {
      id,
      source: "local",
      path: "C:\\workspace\\proto-project",
      name: "proto-project",
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: null,
      toolUsages: [],
    };
    window.__invokeCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          window.__invokeCalls.push({ command, payload });
          if (command === "local_index_exists") return true;
          if (command === "list_projects" || command === "search_projects") return [project];
          if (command === "list_groups") return [{ id: 1, name: "Active", sortOrder: 10, memberCount: 1 }];
          if (command === "list_group_assignments") return JSON.parse('{"__proto__":1}');
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") {
            return { source: "local", serverId: null, label: "Local", tools: [] };
          }
          if (["list_tools", "list_remote_projects", "search_remote_projects",
            "list_remote_servers", "list_project_ignores", "list_sort_orders",
            "list_opener_prefs", "list_web_development_tools"].includes(command)) return [];
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  }, projectId);

  await page.goto("/index.html");
  await expect(page.locator("article.entry")).toHaveCount(1);
  expect(await page.locator("article.entry").getAttribute("data-id")).toBe(projectId);
  await expect(page.locator(".ledger__group__name")).toHaveText("Active");
  await expect.poll(() => page.evaluate(() => {
    const assignments = [...window.__invokeCalls].reverse()
      .find(call => call.command === "update_tray_projects")?.payload.assignments;
    return assignments && {
      hasOwnProto: Object.prototype.hasOwnProperty.call(assignments, "__proto__"),
      value: assignments["__proto__"],
    };
  })).toEqual({ hasOwnProto: true, value: 1 });
  await page.locator("article.entry").click();
  await expect(page.locator("#termsSelectedLaunch [data-group-picker] option:checked")).toHaveText("Active");
  await page.locator("#termsSelectedLaunch [data-group-picker]").selectOption("");
  await expect.poll(() => page.evaluate(() =>
    window.__invokeCalls.find(call => call.command === "assign_project_to_group")?.payload.projectId,
  )).toBe(projectId);
  expect(await page.locator("article.entry").count()).toBe(1);
  expect(errors).toEqual([]);
});
