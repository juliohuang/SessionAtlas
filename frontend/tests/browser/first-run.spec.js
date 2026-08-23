import { expect, test } from "@playwright/test";

async function installFirstRunFixture(page, {
  indexExists,
  scanFailures = 0,
  existingProjects = true,
}) {
  await page.addInitScript(({ hasIndex, failures, seedExistingProjects }) => {
    const project = {
      id: "first-project",
      path: "C:\\workspace\\first-project",
      name: "first-project",
      lastAccessedAt: "2026-08-16T00:00:00Z",
      gitBranch: "main",
      toolUsages: [{
        toolKey: "codex",
        toolName: "Codex CLI",
        lastUsedAt: "2026-08-16T00:00:00Z",
        sessionCount: 1,
        lastSessionId: "first-session",
      }],
    };
    window.__indexExists = hasIndex;
    window.__projects = hasIndex && seedExistingProjects ? [project] : [];
    window.__scanFailuresRemaining = failures;
    window.__invokeCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          window.__invokeCalls.push({ command, payload });
          if (command === "local_index_exists") return window.__indexExists;
          if (command === "scan_projects") {
            if (window.__scanFailuresRemaining > 0) {
              window.__scanFailuresRemaining -= 1;
              throw new Error("no trustworthy session source");
            }
            window.__indexExists = true;
            window.__projects = [project];
            return window.__projects.length;
          }
          if (command === "list_projects" || command === "search_projects") {
            return window.__projects;
          }
          if (command === "list_tools") {
            return [{ toolKey: "codex", toolName: "Codex CLI" }];
          }
          if (command === "list_remote_projects" || command === "search_remote_projects"
              || command === "list_remote_servers" || command === "list_opener_prefs"
              || command === "list_groups" || command === "list_sort_orders") return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  }, {
    hasIndex: indexExists,
    failures: scanFailures,
    seedExistingProjects: existingProjects,
  });
}

test("a missing index is scanned once before the first project load", async ({ page }) => {
  await installFirstRunFixture(page, { indexExists: false });
  await page.goto("/index.html");

  await expect(page.locator('article.entry[data-id="first-project"]')).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("first scan complete");

  const commands = await page.evaluate(() => window.__invokeCalls.map(call => call.command));
  expect(commands.filter(command => command === "scan_projects")).toHaveLength(1);
  expect(commands.indexOf("local_index_exists")).toBeLessThan(commands.indexOf("scan_projects"));
  expect(commands.indexOf("scan_projects")).toBeLessThan(commands.indexOf("list_projects"));
});

test("an existing empty index is not scanned again on launch", async ({ page }) => {
  await installFirstRunFixture(page, { indexExists: true, existingProjects: false });
  await page.goto("/index.html");

  await expect(page.locator(".ledger__empty-title")).toHaveText("No projects yet");
  await expect(page.locator("#emptyScan")).toBeVisible();
  const commands = await page.evaluate(() => window.__invokeCalls.map(call => call.command));
  expect(commands.filter(command => command === "scan_projects")).toHaveLength(0);
});

test("a failed first scan explains the next step and retries the scan", async ({ page }) => {
  await installFirstRunFixture(page, { indexExists: false, scanFailures: 1 });
  await page.goto("/index.html");

  await expect(page.locator(".ledger__empty-title")).toHaveText("Let's build your first index");
  await expect(page.locator("#firstRunRetry")).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("first scan needs attention");

  await page.locator("#firstRunRetry").click();
  await expect(page.locator('article.entry[data-id="first-project"]')).toBeVisible();
  const commands = await page.evaluate(() => window.__invokeCalls.map(call => call.command));
  expect(commands.filter(command => command === "scan_projects")).toHaveLength(2);
});

test("the top rescan button can also recover a failed first scan", async ({ page }) => {
  await installFirstRunFixture(page, { indexExists: false, scanFailures: 1 });
  await page.goto("/index.html");

  await expect(page.locator("#firstRunRetry")).toBeVisible();
  await page.locator("#scanBtn").click();
  await expect(page.locator('article.entry[data-id="first-project"]')).toBeVisible();
  const commands = await page.evaluate(() => window.__invokeCalls.map(call => call.command));
  expect(commands.filter(command => command === "scan_projects")).toHaveLength(2);
});
