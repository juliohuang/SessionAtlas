import { expect, test } from "@playwright/test";

async function installDeferredBackend(page) {
  await page.addInitScript(() => {
    const makeProject = id => ({
      id,
      path: `C:\\workspace\\${id}`,
      name: id,
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    });
    const projects = [makeProject("p1"), makeProject("p2")];
    window.__pendingCalls = {};
    window.__defer = key => new Promise((resolve, reject) => {
      window.__pendingCalls[key] = { resolve, reject };
    });
    window.__resolveCall = (key, value) => {
      const pending = window.__pendingCalls[key];
      if (!pending) throw new Error(`missing deferred call: ${key}`);
      delete window.__pendingCalls[key];
      pending.resolve(value);
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "list_projects") return projects;
          if (command === "search_projects" || command === "list_remote_projects"
              || command === "search_remote_projects" || command === "list_tools"
              || command === "list_remote_servers" || command === "list_groups"
              || command === "list_sort_orders" || command === "list_opener_prefs") return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          if (command === "list_project_docs") return window.__defer(`docs:${payload.path}`);
          if (command === "read_project_doc") return window.__defer(`doc:${payload.path}:${payload.relPath}`);
          if (command === "list_dir") return window.__defer(`dir:${payload.path}`);
          if (command === "read_text_file") return "file";
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });
}

test("entry docs and tree ignore inverse-completion responses from the previous project", async ({ page }) => {
  await installDeferredBackend(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="p1"] [data-menu-toggle]').dispatchEvent("click");
  await page.locator('article.entry[data-id="p2"] [data-menu-toggle]').dispatchEvent("click");
  await page.evaluate(() => {
    window.__resolveCall("docs:C:\\workspace\\p2", [{ name: "P2.md", relPath: "P2.md", size: 2 }]);
    window.__resolveCall("dir:C:\\workspace\\p2", [{ name: "p2.txt", isDir: false }]);
  });
  await expect(page.locator("#entryModalBody")).toContainText("P2.md");
  await expect(page.locator("#entryModalBody")).toContainText("p2.txt");

  await page.evaluate(() => {
    window.__resolveCall("docs:C:\\workspace\\p1", [{ name: "STALE.md", relPath: "STALE.md", size: 1 }]);
    window.__resolveCall("dir:C:\\workspace\\p1", [{ name: "stale.txt", isDir: false }]);
  });
  await expect(page.locator("#entryModalTitle")).toHaveText("p2");
  await expect(page.locator("#entryModalBody")).not.toContainText("STALE.md");
  await expect(page.locator("#entryModalBody")).not.toContainText("stale.txt");
});

test("doc modal ignores an older read and every close path invalidates pending publication", async ({ page }) => {
  await installDeferredBackend(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="p1"] [data-menu-toggle]').dispatchEvent("click");
  await page.evaluate(() => {
    window.__resolveCall("docs:C:\\workspace\\p1", [{ name: "one.md", relPath: "one.md", size: 1 }]);
    window.__resolveCall("dir:C:\\workspace\\p1", []);
  });
  await page.locator("#entryModalBody .doc-pill").click();
  await page.locator("#docModal [data-doc-modal-close]").last().click();
  await expect(page.locator("#docModal")).toBeHidden();
  await page.evaluate(() => window.__resolveCall("doc:C:\\workspace\\p1:one.md", "# stale"));
  await expect(page.locator("#docModal")).toBeHidden();
  await expect(page.locator("#docModalBody")).not.toContainText("stale");

  await page.locator('article.entry[data-id="p2"] [data-menu-toggle]').dispatchEvent("click");
  await page.evaluate(() => {
    window.__resolveCall("docs:C:\\workspace\\p2", [{ name: "two.md", relPath: "two.md", size: 1 }]);
    window.__resolveCall("dir:C:\\workspace\\p2", []);
  });
  await page.locator("#entryModalBody .doc-pill").click();
  await page.evaluate(() => window.__resolveCall("doc:C:\\workspace\\p2:two.md", "# current"));
  await expect(page.locator("#docModalBody")).toContainText("current");
});

test("left tree keeps the latest root and a collapsed pending directory stays collapsed", async ({ page }) => {
  await installDeferredBackend(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="p1"] [data-tree-btn]').dispatchEvent("click");
  await page.locator("#filesBackBtn").click();
  await page.locator('article.entry[data-id="p2"] [data-tree-btn]').dispatchEvent("click");
  await page.evaluate(() => window.__resolveCall("dir:C:\\workspace\\p2", [{ name: "src", isDir: true }]));
  await expect(page.locator("#stageLeftFilesName")).toHaveText("p2");
  await expect(page.locator("#stageLeftFilesTree")).toContainText("src");
  await page.evaluate(() => window.__resolveCall("dir:C:\\workspace\\p1", [{ name: "stale", isDir: true }]));
  await expect(page.locator("#stageLeftFilesTree")).not.toContainText("stale");

  const dir = page.locator("#stageLeftFilesTree .tree__node--dir");
  await dir.locator(".tree__name").click();
  await dir.locator(".tree__name").click();
  await page.evaluate(() => window.__resolveCall("dir:C:\\workspace\\p2\\src", [{ name: "late.txt", isDir: false }]));
  await expect(dir.locator(".tree__caret")).toHaveText("▸");
  await expect(dir.locator("xpath=following-sibling::*[1]")).toBeHidden();
  await page.locator("#filesBackBtn").click();
  await expect(page.locator('[data-view="ledger"]')).toBeVisible();
});
