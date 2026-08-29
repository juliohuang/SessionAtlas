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
  await expect(page.locator('#filters [data-tool="pi"]')).toContainText("Pi Coding Agent");
  expect(errors).toEqual([]);
  expect(externalRequests).toEqual([]);
});

test("order modes keep every project visible while changing what appears first", async ({ page }) => {
  await page.goto("/index.html");
  const rows = page.locator("article.entry");
  const firstName = () => rows.first().locator(".entry__name").textContent();

  await expect(rows).toHaveCount(7);
  await expect(page.locator('[data-project-order="priority"]')).toHaveClass(/is-active/);
  await expect.poll(firstName).toBe("docs-garden");

  await page.locator('[data-project-order="recent"]').click();
  await expect(rows).toHaveCount(7);
  await expect.poll(firstName).toBe("atlas-notes");

  await page.locator('[data-project-order="name"]').click();
  await expect(rows).toHaveCount(7);
  await expect.poll(firstName).toBe("api-workbench");

  await page.locator('[data-project-order="grouped"]').click();
  await expect(rows).toHaveCount(7);
  await expect(page.locator(".ledger__group")).toHaveCount(1);
  await expect(rows.first()).toHaveAttribute("draggable", "true");
});

test("browser demo can find a project by indexed content", async ({ page }) => {
  await page.goto("/index.html");

  await page.locator("#searchInput").fill("full text search");

  await expect(page.locator("article.entry")).toHaveCount(1);
  await expect(page.locator(".entry__name")).toHaveText("atlas-notes");
  await expect(page.locator(".entry__content-path")).toHaveText("README.md");
  await expect(page.locator(".entry__content-snippet")).toContainText("full text search");
});

test("workspace redesign exposes project overview and terminal regions", async ({ page }) => {
  await page.goto("/index.html");

  await expect(page.locator("article.entry")).toHaveCount(7);
  await expect(page).toHaveTitle("SessionAtlas");
  await expect(page.locator(".brand__name")).toHaveText("SessionAtlas");
  await expect(page.locator(".stage__overview")).toBeVisible();
  await expect(page.locator("#termsSelectedLaunch")).toContainText("docs-garden");
  await expect(page.locator(".overview__activity-row")).toHaveCount(1);
  await expect(page.locator(".overview__session-row")).toHaveCount(0);
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

test("project overview collapses into a persistent restore rail", async ({ page }) => {
  await page.goto("/index.html");

  const stage = page.locator("#stage");
  const overview = page.locator(".stage__overview");
  const terminal = page.locator(".stage__right");
  const toggle = page.locator("#overviewToggleBtn");
  const expandedTerminalWidth = (await terminal.boundingBox()).width;

  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#termsSelectedLaunch")).toBeVisible();
  await toggle.click();

  await expect(stage).toHaveClass(/stage--overview-collapsed/);
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator("#termsSelectedLaunch")).toBeHidden();
  expect((await overview.boundingBox()).width).toBeLessThanOrEqual(40);
  expect((await terminal.boundingBox()).width).toBeGreaterThan(expandedTerminalWidth);

  await page.reload();
  await expect(stage).toHaveClass(/stage--overview-collapsed/);
  await expect(toggle).toHaveAttribute("aria-expanded", "false");

  await toggle.click();
  await expect(stage).not.toHaveClass(/stage--overview-collapsed/);
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#termsSelectedLaunch")).toBeVisible();
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
      pathMissing: true,
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
  await expect(page.locator('article.entry[data-id="mock-project"]')).toHaveClass(/is-missing/);
  await expect(page.locator('article.entry[data-id="mock-project"] .entry__missing-badge')).toHaveText("MISSING");
  await expect(page.locator("#ledgerCount")).toContainText("1");
  const calls = await page.evaluate(() => window.__invokeCalls.map(call => call.command));
  expect(calls).toContain("list_projects");
  expect(calls).toContain("list_groups");
  const listPayload = await page.evaluate(() =>
    window.__invokeCalls.find(call => call.command === "list_projects")?.payload,
  );
  expect(listPayload).toEqual({ limit: 10000 });
  expect(errors).toEqual([]);
});

test("activity-only OpenCode sessions do not shadow a resumable main session", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.lang", "en");
    const project = {
      id: "delegated-project",
      path: "C:\\workspace\\delegated-project",
      name: "delegated-project",
      source: "local",
      lastAccessedAt: "2026-08-18T10:00:00Z",
      gitBranch: "main",
      toolUsages: [
        {
          toolKey: "opencode",
          toolName: "OpenCode",
          lastUsedAt: "2026-08-18T10:00:00Z",
          sessionCount: 0,
          lastSessionId: null,
        },
        {
          toolKey: "codex",
          toolName: "Codex CLI",
          lastUsedAt: "2026-08-18T09:00:00Z",
          sessionCount: 1,
          lastSessionId: "codex-main-session",
        },
      ],
    };
    const capabilities = {
      source: "local",
      serverId: null,
      label: "Local",
      tools: ["opencode", "codex"].map(toolKey => ({
        toolKey,
        toolName: toolKey === "codex" ? "Codex CLI" : "OpenCode",
        installed: true,
        enabled: true,
        adapterEnabled: true,
      })),
    };
    window.__ptyCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return [project];
          if (command === "list_tools") return capabilities.tools;
          if ([
            "list_remote_projects", "list_remote_servers", "list_project_ignores",
            "list_groups", "list_sort_orders", "list_opener_prefs",
            "list_web_development_tools",
          ].includes(command)) return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") return capabilities;
          if (command === "pty_spawn") {
            window.__ptyCalls.push({ command, payload });
            return 77;
          }
          if (command === "pty_attach") {
            window.__ptyCalls.push({ command, payload });
            return null;
          }
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  const entry = page.locator('article.entry[data-id="delegated-project"]');
  await expect(entry).toBeVisible();
  await entry.locator("[data-expand-toggle]").click();

  const openCodeCard = entry.locator(".session-card").filter({ hasText: "OpenCode" });
  await expect(openCodeCard).toContainText("Child/delegated activity excluded");
  await expect(openCodeCard.locator("[data-launch-tool=opencode]")).toHaveText("New session");

  const codexCard = entry.locator(".session-card").filter({ hasText: "Codex CLI" });
  await expect(codexCard).toContainText("1 resumable session");
  await expect(codexCard.locator("[data-launch-tool=codex]")).toHaveText("Resume");
  await expect(codexCard.locator("[data-launch-tool=codex]")).toBeEnabled();
  await expect(page.locator("#termsSelectedLaunch .overview__session-row")).toHaveCount(1);

  await entry.dispatchEvent("dblclick");
  await expect.poll(() => page.evaluate(() => window.__ptyCalls
    .find(call => call.command === "pty_attach")?.payload)).toEqual({
    id: 77,
    toolKey: "codex",
    sessionId: "codex-main-session",
  });
});

test("large catalogs keep a bounded ledger while scrolling reaches the directory tail", async ({ page }) => {
  await page.addInitScript(() => {
    const projects = Array.from({ length: 2_000 }, (_, index) => ({
      id: `large-${index}`,
      path: `C:\\workspace\\large-${index}`,
      name: `large-project-${String(index).padStart(4, "0")}`,
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    }));
    window.__TAURI__ = {
      core: {
        invoke: async (command) => {
          if (command === "list_projects") return projects;
          if (command === "list_tools" || command === "list_remote_projects"
              || command === "list_remote_servers" || command === "list_opener_prefs"
              || command === "search_remote_projects" || command === "list_groups"
              || command === "list_sort_orders") return [];
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
  await expect(page.locator("#ledgerCount")).toContainText("2000");
  await expect(page.locator("#ledger")).toHaveClass(/is-large/);
  const initialCount = await page.locator("#ledger article.entry").count();
  expect(initialCount).toBeLessThan(200);

  await page.locator("#ledger").evaluate(element => {
    element.scrollTop = element.scrollHeight;
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect.poll(() => page.locator('#ledger article.entry[data-id="large-1999"]').count()).toBe(1);
  const tailCount = await page.locator("#ledger article.entry").count();
  expect(tailCount).toBeLessThan(200);
});

test("content search shows the matching file and a text-only subtitle", async ({ page }) => {
  await page.addInitScript(() => {
    const base = {
      id: "content-project",
      path: "C:\\workspace\\content-project",
      name: "content-project",
      lastAccessedAt: "2026-08-16T00:00:00Z",
      gitBranch: "main",
      toolUsages: [{
        toolKey: "codex",
        toolName: "Codex CLI",
        lastUsedAt: "2026-08-16T00:00:00Z",
        sessionCount: 1,
        lastSessionId: null,
      }],
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          if (command === "list_projects") return [base];
          if (command === "search_projects") {
            return [{
              ...base,
              contentMatch: {
                relativePath: "src/search.rs",
                snippet: `fn ${payload.query}_symbol() { <unsafe-tag> }`,
              },
            }];
          }
          if (command === "list_tools") return [{ toolKey: "codex", toolName: "Codex CLI" }];
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "list_group_assignments") return {};
          if (command === "list_groups" || command === "list_sort_orders"
              || command === "list_remote_projects" || command === "list_remote_servers"
              || command === "list_opener_prefs" || command === "search_remote_projects") return [];
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await page.locator("#searchInput").fill("distinctive_content");

  await expect(page.locator(".entry__content-path")).toHaveText("src/search.rs");
  await expect(page.locator(".entry__content-snippet")).toContainText("distinctive_content_symbol");
  await expect(page.locator("unsafe-tag")).toHaveCount(0);
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
  await page.locator('[data-project-order="grouped"]').click();
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
