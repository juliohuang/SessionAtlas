import { expect, test } from "@playwright/test";

test("2000-project ledger keeps a bounded DOM and fast filtering", async ({ page }) => {
  await page.addInitScript(() => {
    const projects = Array.from({ length: 2000 }, (_, index) => ({
      id: `project-${index}`,
      path: `C:\\workspace\\project-${index}`,
      name: `project-${index}`,
      lastAccessedAt: "2026-08-17T00:00:00Z",
      gitBranch: "main",
      toolUsages: index === 0 ? [{
        toolKey: "claude",
        toolName: "Claude Code",
        sessionCount: 1,
        lastUsedAt: "2026-08-17T00:00:00Z",
        lastSessionId: "session-0",
      }] : [],
    }));
    const assignments = Object.fromEntries(
      projects.map((project, index) => [project.id, index < 1000 ? 1 : 2]),
    );
    const groupOrder = new Map([
      [1, projects.slice(0, 1000).map(project => project.id)],
      [2, projects.slice(1000).map(project => project.id)],
    ]);
    let manualOrder = false;
    let revision = 0;
    const calls = [];
    window.__performanceInvokeCalls = calls;
    const groupRows = () => [1, 2].map(id => ({
      id,
      name: id === 1 ? "Alpha" : "Beta",
      sortOrder: id,
      memberCount: projects.filter(project => assignments[project.id] === id).length,
    }));
    const sortRows = () => manualOrder
      ? projects.map(project => ({
          projectId: project.id,
          groupKey: String(assignments[project.id]),
          sortOrder: (groupOrder.get(assignments[project.id]) || []).indexOf(project.id) * 10 + 10,
        }))
      : [];
    const moveWithinGroup = (projectId, targetGroupKey, anchorProjectId, placement) => {
      const groupId = Number(targetGroupKey);
      const bucket = groupOrder.get(groupId) || [];
      const sourceIndex = bucket.indexOf(projectId);
      if (sourceIndex >= 0) bucket.splice(sourceIndex, 1);
      let anchorIndex = bucket.indexOf(anchorProjectId);
      if (anchorIndex < 0) anchorIndex = bucket.length;
      if (placement === "after") anchorIndex += 1;
      bucket.splice(anchorIndex, 0, projectId);
      groupOrder.set(groupId, bucket);
      manualOrder = true;
    };
    const assignToGroup = (projectId, groupId) => {
      const previous = assignments[projectId];
      if (previous === groupId) return;
      const previousBucket = groupOrder.get(previous) || [];
      const previousIndex = previousBucket.indexOf(projectId);
      if (previousIndex >= 0) previousBucket.splice(previousIndex, 1);
      const nextBucket = groupOrder.get(groupId) || [];
      nextBucket.push(projectId);
      groupOrder.set(groupId, nextBucket);
      assignments[projectId] = groupId;
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          calls.push({ command, payload });
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return projects;
          if (command === "search_projects") {
            const query = String(payload.query || "").toLowerCase();
            return projects.filter(project => project.name.toLowerCase().includes(query));
          }
          if ([
            "list_tools", "list_remote_projects", "list_remote_servers",
            "list_project_ignores",
            "list_opener_prefs", "search_remote_projects",
          ].includes(command)) return [];
          if (command === "list_groups") {
            return groupRows();
          }
          if (command === "list_group_assignments") return { ...assignments };
          if (command === "list_sort_orders") return sortRows();
          if (command === "get_group_revision") return revision;
          if (command === "assign_project_to_group") {
            assignToGroup(String(payload.projectId), payload.groupId == null ? null : Number(payload.groupId));
            revision += 1;
            return null;
          }
          if (command === "move_group_project") {
            moveWithinGroup(
              String(payload.projectId),
              String(payload.targetGroupKey),
              String(payload.anchorProjectId),
              payload.placement,
            );
            revision += 1;
            return { revision, orderedIds: groupOrder.get(Number(payload.targetGroupKey)) || [] };
          }
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") return {
            source: "local",
            serverId: null,
            label: "Local",
            tools: [{
              toolKey: "claude",
              toolName: "Claude Code",
              installed: true,
              enabled: true,
            }],
          };
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await expect(page.locator("#ledger article.entry").first()).toBeVisible();
  const initialMetrics = await page.evaluate(() => ({
    domNodes: document.querySelector("#ledger").querySelectorAll("*").length,
    buttons: document.querySelector("#ledger").querySelectorAll("button").length,
    projectRows: document.querySelectorAll("#ledger article.entry").length,
    compactHeight: document.querySelector("#ledger article.entry")?.getBoundingClientRect().height || 0,
    groupHeight: document.querySelector("#ledger [data-group-toggle]")?.getBoundingClientRect().height || 0,
  }));
  expect(initialMetrics.domNodes).toBeLessThan(6_000);
  expect(initialMetrics.buttons).toBeLessThan(400);
  expect(initialMetrics.projectRows).toBeLessThan(80);
  expect(initialMetrics.compactHeight).toBeGreaterThan(20);
  expect(initialMetrics.groupHeight).toBeGreaterThan(20);

  const groups = page.locator("#ledger [data-group-toggle]");
  // Only the first group header is in the initial pixel window; the second
  // header becomes reachable when Alpha is collapsed (its projects leave the
  // window) or when the user scrolls to that group's range.
  await expect(groups).toHaveCount(1);
  await groups.first().click();
  await expect(groups.first()).toHaveAttribute("aria-expanded", "false");
  await expect(groups).toHaveCount(2);
  await groups.first().click();
  await expect(groups.first()).toHaveAttribute("aria-expanded", "true");
  const firstExpand = page.locator("#ledger article.entry").first().locator("[data-expand-toggle]");
  const disclosureBox = await firstExpand.boundingBox();
  expect(disclosureBox?.width || 0).toBeGreaterThanOrEqual(30);
  expect(disclosureBox?.height || 0).toBeGreaterThanOrEqual(30);
  const controlDragCanceled = await firstExpand.evaluate(button => {
    const event = new DragEvent("dragstart", {
      bubbles: true,
      cancelable: true,
      dataTransfer: new DataTransfer(),
    });
    return !button.dispatchEvent(event);
  });
  expect(controlDragCanceled).toBe(true);
  await firstExpand.click();
  await expect(page.locator("#ledger article.entry.is-expanded")).toHaveCount(1);
  await expect(firstExpand).toHaveAttribute("aria-expanded", "true");
  await expect(firstExpand).toBeFocused();
  const queueInput = page.locator('#ledger [data-queue-panel][data-project-id="project-0"] [data-queue-input]');
  await expect(queueInput).toBeVisible();
  await queueInput.fill("preserve this prompt");
  await queueInput.focus();
  await page.evaluate(() => {
    const list = document.querySelector("#ledger");
    list.scrollTop = 20;
    list.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(80);
  await expect(queueInput).toHaveValue("preserve this prompt");
  expect(await page.evaluate(() => document.activeElement?.matches("[data-queue-input]"))).toBe(true);

  // The collapsed header and the first project of the other group remain in
  // the bounded window. Move that project across the group boundary and make
  // sure the optimistic + authoritative refresh updates header counts.
  await groups.first().click();
  await expect(groups.first()).toHaveAttribute("aria-expanded", "false");
  await page.locator('#ledger article.entry[data-id="project-1000"]')
    .dragTo(groups.first());
  await expect.poll(() => groups.first().locator(".ledger__group__count").textContent())
    .toBe("1001");
  await expect.poll(() => groups.nth(1).locator(".ledger__group__count").textContent())
    .toBe("999");
  // Re-open the moved-to group so the height assertion covers the full
  // catalog. The earlier expansion belonged to project-0, which was hidden
  // while Alpha was collapsed; collapse it before measuring compact rows.
  await groups.first().click();
  await expect(groups.first()).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator('#ledger article.entry[data-id="project-0"]')).toBeVisible();
  await page.locator('#ledger article.entry[data-id="project-0"] [data-expand-toggle]').click();
  await expect(page.locator("#ledger article.entry.is-expanded")).toHaveCount(0);

  await page.evaluate(() => {
    const list = document.querySelector("#ledger");
    list.scrollTop = list.scrollHeight;
    list.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect.poll(() => page.evaluate(() => Boolean(
    document.querySelector('#ledger article.entry[data-id="project-1999"]'),
  ))).toBe(true);
  await page.waitForTimeout(100);
  const bottomMetrics = await page.evaluate(() => {
    const list = document.querySelector("#ledger");
    return {
      height: list.scrollHeight,
      visible: Boolean(list.querySelector('article.entry[data-id="project-1999"]')),
    };
  });
  await page.waitForTimeout(100);
  const stableBottomMetrics = await page.evaluate(() => {
    const list = document.querySelector("#ledger");
    return {
      height: list.scrollHeight,
      visible: Boolean(list.querySelector('article.entry[data-id="project-1999"]')),
    };
  });
  expect(stableBottomMetrics.visible).toBe(true);
  expect(Math.abs(stableBottomMetrics.height - bottomMetrics.height)).toBeLessThanOrEqual(2);
  const expectedCompactHeight = 2 * initialMetrics.groupHeight + 2000 * initialMetrics.compactHeight;
  expect(stableBottomMetrics.height / expectedCompactHeight).toBeGreaterThan(0.85);
  expect(stableBottomMetrics.height / expectedCompactHeight).toBeLessThan(1.15);

  // This fixture intentionally fills LIST_LIMIT. The product refuses a
  // complete manual-order write at that cap because the catalog may be
  // truncated; verify the guard while the bottom rows are mounted. The
  // dedicated bounded-order test below covers a complete catalog.
  await page.evaluate(() => {
    const source = document.querySelector('#ledger article.entry[data-id="project-1999"]');
    const target = document.querySelector('#ledger article.entry[data-id="project-1998"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer }));
    target.dispatchEvent(new DragEvent("drop", {
      bubbles: true,
      dataTransfer,
      clientY: target.getBoundingClientRect().top + 1,
    }));
    source.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer }));
  });
  await page.waitForTimeout(50);
  expect(await page.evaluate(() =>
    window.__performanceInvokeCalls.filter(call => call.command === "move_group_project").length,
  )).toBe(0);

  await page.evaluate(() => {
    window.__filterStartedAt = performance.now();
    const input = document.querySelector("#searchInput");
    input.value = "project-1999";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect(page.locator("#ledgerCount")).toContainText("1");
  const filterMs = await page.evaluate(() => performance.now() - window.__filterStartedAt);
  expect(filterMs).toBeLessThan(300);
});

test("virtualized group move and manual reorder use delegated mutations", async ({ page }) => {
  await page.addInitScript(() => {
    const projects = Array.from({ length: 120 }, (_, index) => ({
      id: `order-project-${index}`,
      path: `C:\\workspace\\order-project-${index}`,
      name: `order-project-${index}`,
      lastAccessedAt: "2026-08-17T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    }));
    const assignments = Object.fromEntries(projects.map(project => [project.id, 1]));
    assignments["order-project-119"] = 2;
    let revision = 0;
    const calls = [];
    window.__performanceInvokeCalls = calls;
    const groupRows = () => [1, 2].map(id => ({
      id,
      name: id === 1 ? "Alpha" : "Beta",
      sortOrder: id,
      memberCount: projects.filter(project => assignments[project.id] === id).length,
    }));
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          calls.push({ command, payload });
          if (command === "local_index_exists") return true;
          if (command === "list_projects") return projects;
          if ([
            "list_tools", "list_remote_projects", "list_remote_servers",
            "list_project_ignores", "list_opener_prefs", "search_remote_projects",
          ].includes(command)) return [];
          if (command === "list_groups") return groupRows();
          if (command === "list_group_assignments") return { ...assignments };
          if (command === "list_sort_orders") return [];
          if (command === "get_group_revision") return revision;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "probe_tui_capabilities") return { source: "local", tools: [] };
          if (command === "assign_project_to_group") {
            assignments[String(payload.projectId)] = payload.groupId == null ? null : Number(payload.groupId);
            revision += 1;
            return null;
          }
          if (command === "move_group_project") {
            revision += 1;
            return { revision, orderedIds: [] };
          }
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });

  await page.goto("/index.html");
  await expect(page.locator('#ledger article.entry[data-id="order-project-0"]')).toBeVisible();
  await page.evaluate(() => {
    const list = document.querySelector("#ledger");
    list.scrollTop = list.scrollHeight;
    list.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator('#ledger article.entry[data-id="order-project-118"]')).toBeVisible();
  await expect(page.locator('#ledger article.entry[data-id="order-project-119"]')).toBeVisible();
  await page.evaluate(() => {
    const source = document.querySelector('#ledger article.entry[data-id="order-project-118"]');
    const target = document.querySelector('#ledger [data-group-toggle][data-group-key="2"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer }));
    target.dispatchEvent(new DragEvent("drop", { bubbles: true, dataTransfer }));
    source.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer }));
  });
  await page.waitForTimeout(100);
  const groupMoveCalls = await page.evaluate(() =>
    window.__performanceInvokeCalls.filter(call => call.command === "assign_project_to_group").length,
  );
  expect(groupMoveCalls).toBeGreaterThan(0);

  await page.evaluate(() => {
    const source = document.querySelector('#ledger article.entry[data-id="order-project-117"]');
    const target = document.querySelector('#ledger article.entry[data-id="order-project-116"]');
    const dataTransfer = new DataTransfer();
    source.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer }));
    target.dispatchEvent(new DragEvent("drop", {
      bubbles: true,
      dataTransfer,
      clientY: target.getBoundingClientRect().top + 1,
    }));
    source.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer }));
  });
  await page.waitForTimeout(100);
  const reorderCalls = await page.evaluate(() =>
    window.__performanceInvokeCalls.filter(call => call.command === "move_group_project").length,
  );
  expect(reorderCalls).toBeGreaterThan(0);
  const renderedOrderRows = await page.locator("#ledger article.entry").count();
  expect(renderedOrderRows).toBeLessThan(80);
});
