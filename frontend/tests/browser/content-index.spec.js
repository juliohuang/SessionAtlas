import { expect, test } from "@playwright/test";

async function fixture(page, { lang = "en", missing = false } = {}) {
  await page.addInitScript(({ lang, missing }) => {
    localStorage.setItem("sessionatlas.lang", lang);
    const project = { id: "synthetic-project", path: "C:\\fixtures\\synthetic-project", name: "synthetic-project", lastAccessedAt: "2026-09-08T00:00:00Z", toolUsages: [] };
    window.__content = { indexExists: !missing, enabled: true, phase: "ready", searchable: true, indexedFiles: 3, indexedBytes: 4096 };
    window.__calls = [];
    window.__rejectStatus = false;
    window.__failCleanup = false;
    window.__holdMutation = false;
    window.__TAURI__ = {
      core: { invoke: async (command, payload) => {
        window.__calls.push({ command, payload });
        if (command === "local_index_exists") return true;
        if (command === "get_content_index_status") {
          if (window.__rejectStatus) throw new Error("status unavailable");
          if (window.__holdStatusOnce) {
            window.__holdStatusOnce = false;
            const snapshot = { ...window.__content };
            return new Promise(resolve => { window.__resolveStatus = () => resolve(snapshot); });
          }
          return { ...window.__content };
        }
        if (command === "update_content_index") {
          if (window.__holdMutation) await new Promise(resolve => { window.__resolveMutation = resolve; });
          const action = payload.action;
          if (action === "disable") window.__content.enabled = false;
          if (action === "enable") window.__content.enabled = true;
          if (window.__failCleanup) {
            window.__content.phase = "cleanupPending";
            window.__content.searchable = false;
            throw new Error('<img src=x onerror="window.__injected=1"> cleanup busy');
          }
          if (action === "disable" || action === "clear") {
            window.__content.indexedFiles = 0;
            window.__content.indexedBytes = 0;
          }
          if (action === "rebuild") {
            window.__content.indexedFiles = 3;
            window.__content.indexedBytes = 4096;
          }
          window.__content.phase = "ready";
          window.__content.searchable = window.__content.enabled;
          return { ...window.__content };
        }
        if (command === "list_projects") return [project];
        if (command === "search_projects") {
          if (payload.query === "nebula") return window.__content.searchable && window.__content.indexedFiles
            ? [{ ...project, contentMatch: { relativePath: "README.md", snippet: "nebula synthetic content preview" } }] : [];
          return [project];
        }
        if (command === "list_group_assignments") return {};
        if (command === "get_group_revision") return 0;
        if (command === "get_git_info") return { isRepo: false, remotes: [] };
        if (command === "probe_tui_capabilities") return { source: "local", serverId: null, label: "Local", tools: [], adapterDiagnostics: [] };
        if (command.startsWith("list_") || command.startsWith("search_remote")) return [];
        return null;
      } },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  }, { lang, missing });
  await page.goto("/index.html");
  await expect(page.locator("article.entry")).toHaveCount(1);
}

async function openControls(page) {
  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="contentIndex"]').click();
  await expect(page.locator("[data-content-refresh]")).toBeEnabled();
}
const button = (page, action) => page.locator(`[data-content-action="${action}"]`);
async function actions(page) {
  return page.evaluate(() => window.__calls.filter(c => c.command === "update_content_index").map(c => c.payload.action));
}

for (const lang of ["en", "zh"]) {
  test(`content controls lifecycle and screenshots in ${lang}`, async ({ page }, testInfo) => {
    const errors = [];
    page.on("pageerror", e => errors.push(e.message));
    await fixture(page, { lang });
    page.on("dialog", dialog => dialog.accept());
    await openControls(page);
    await expect(page.locator("#drawerTitle")).toHaveText(lang === "zh" ? "内容索引" : "Content index");
    await expect(page.locator("[data-content-status]")).toContainText(lang === "zh" ? "已开启" : "Enabled");
    await page.screenshot({ path: testInfo.outputPath("01-enabled.png"), fullPage: true, animations: "disabled" });
    await button(page, "disable").click();
    await expect(button(page, "enable")).toBeEnabled();
    await expect(page.locator("[data-content-status]")).toContainText(lang === "zh" ? "已关闭" : "Disabled");
    await expect(button(page, "rebuild")).toBeDisabled();
    await page.screenshot({ path: testInfo.outputPath("02-disabled.png"), fullPage: true, animations: "disabled" });
    await page.locator("#drawerBackBtn").click();
    await page.locator('[data-settings-view="contentIndex"]').click();
    await expect(button(page, "enable")).toBeEnabled();
    await button(page, "enable").click();
    await expect(button(page, "rebuild")).toBeEnabled();
    await page.screenshot({ path: testInfo.outputPath("03-reenabled-empty.png"), fullPage: true, animations: "disabled" });
    await button(page, "rebuild").click();
    await expect(page.locator("[data-content-status]")).toContainText(lang === "zh" ? "已重建" : "rebuilt");
    await page.screenshot({ path: testInfo.outputPath("04-rebuilt.png"), fullPage: true, animations: "disabled" });
    await button(page, "clear").click();
    await expect(page.locator("[data-content-status]")).toContainText(lang === "zh" ? "已清理" : "cleared");
    await page.screenshot({ path: testInfo.outputPath("05-cleared.png"), fullPage: true, animations: "disabled" });
    expect(await actions(page)).toEqual(["disable", "enable", "rebuild", "clear"]);
    expect(errors).toEqual([]);
  });
}

test("cancelled confirmation never invokes a content mutation", async ({ page }) => {
  await fixture(page);
  await openControls(page);
  page.on("dialog", dialog => dialog.dismiss());
  await button(page, "disable").click();
  await button(page, "clear").click();
  await button(page, "rebuild").click();
  expect(await actions(page)).toEqual([]);
  await expect(button(page, "disable")).toBeEnabled();
});

test("busy operation blocks duplicate actions even after drawer navigation", async ({ page }, testInfo) => {
  await fixture(page);
  await openControls(page);
  page.on("dialog", dialog => dialog.accept());
  await page.evaluate(() => { window.__holdMutation = true; });
  await button(page, "disable").click();
  await expect(button(page, "disable")).toBeDisabled();
  await expect(page.locator("[data-content-refresh]")).toBeDisabled();
  await page.locator("#drawerBackBtn").click();
  await page.locator('[data-settings-view="contentIndex"]').click();
  await expect(button(page, "clear")).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath("06-busy.png"), fullPage: true, animations: "disabled" });
  await expect.poll(() => page.evaluate(() => typeof window.__resolveMutation)).toBe("function");
  await page.evaluate(() => window.__resolveMutation());
  await expect(button(page, "enable")).toBeEnabled();
  expect(await actions(page)).toEqual(["disable"]);
});

test("failed cleanup shows committed state safely and can retry", async ({ page }, testInfo) => {
  await fixture(page);
  await openControls(page);
  page.on("dialog", dialog => dialog.accept());
  await page.evaluate(() => { window.__failCleanup = true; });
  await button(page, "disable").click();
  await expect(page.locator("[data-content-status]")).toContainText("Cleanup incomplete");
  await expect(page.locator("[role=alert]")).toContainText("cleanup busy");
  await expect(page.locator("#drawerBody img")).toHaveCount(0);
  expect(await page.evaluate(() => window.__injected)).toBeUndefined();
  await expect(button(page, "rebuild")).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath("07-cleanup-failed.png"), fullPage: true, animations: "disabled" });
  await page.evaluate(() => { window.__failCleanup = false; });
  await button(page, "clear").click();
  await expect(page.locator("[role=alert]")).toHaveCount(0);
  await expect(page.locator("[data-content-status]")).toContainText("Disabled");
  await expect(page.locator("[data-content-status]")).toContainText("Ready");
});

test("unavailable or malformed status fails closed and supports refresh", async ({ page }, testInfo) => {
  await fixture(page);
  await page.evaluate(() => { window.__rejectStatus = true; });
  await openControls(page);
  await expect(page.locator("[role=alert]")).toContainText("status unavailable");
  await expect(button(page, "disable")).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath("08-status-failed.png"), fullPage: true, animations: "disabled" });
  await page.evaluate(() => { window.__rejectStatus = false; window.__content.phase = "unexpected"; });
  await page.locator("[data-content-refresh]").click();
  await expect(page.locator("[role=alert]")).toContainText("Invalid content index");
  await expect(button(page, "clear")).toBeDisabled();
  await page.evaluate(() => { window.__content.phase = "ready"; });
  await page.locator("[data-content-refresh]").click();
  await expect(button(page, "disable")).toBeEnabled();
});

test("missing project index explains first scan and prevents empty database creation", async ({ page }, testInfo) => {
  await fixture(page, { missing: true });
  await openControls(page);
  await expect(page.locator("[data-content-status]")).toContainText("No project index yet");
  for (const action of ["enable", "disable", "clear", "rebuild"]) await expect(button(page, action)).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath("09-missing.png"), fullPage: true, animations: "disabled" });
  expect(await actions(page)).toEqual([]);
});

test("disabling clears previously displayed content search results", async ({ page }) => {
  await fixture(page);
  await page.locator("#searchInput").fill("nebula");
  await expect(page.locator("article.entry")).toContainText("nebula synthetic content preview");
  await openControls(page);
  page.on("dialog", dialog => dialog.accept());
  await button(page, "disable").click();
  await expect(button(page, "enable")).toBeEnabled();
  await expect(page.locator("article.entry")).toHaveCount(0);
});

test("browser demo makes no claims of real content mutations", async ({ page }, testInfo) => {
  await page.goto("/index.html");
  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="contentIndex"]').click();
  await expect(page.locator("#drawerBody")).toContainText("Browser demo only");
  for (const action of ["enable", "disable", "clear", "rebuild"]) await expect(button(page, action)).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath("10-browser-demo.png"), fullPage: true, animations: "disabled" });
});

test("an older status request cannot revert a completed disable", async ({ page }) => {
  await fixture(page);
  await page.evaluate(() => { window.__holdStatusOnce = true; });
  await page.locator("#settingsBtn").click();
  await page.locator('[data-settings-view="contentIndex"]').click();
  await expect.poll(() => page.evaluate(() => typeof window.__resolveStatus)).toBe("function");
  await page.locator("#drawerBackBtn").click();
  await page.locator('[data-settings-view="contentIndex"]').click();
  await expect(button(page, "disable")).toBeEnabled();
  page.on("dialog", dialog => dialog.accept());
  await button(page, "disable").click();
  await expect(button(page, "enable")).toBeEnabled();
  await page.evaluate(() => window.__resolveStatus());
  await expect(page.locator("[data-content-status]")).toContainText("Disabled");
  await expect(button(page, "disable")).toBeDisabled();
});

test("failed mutation and unavailable status never leave stale controls enabled", async ({ page }) => {
  await fixture(page);
  await openControls(page);
  page.on("dialog", dialog => dialog.accept());
  await page.evaluate(() => { window.__failCleanup = true; window.__rejectStatus = true; });
  await button(page, "disable").click();
  await expect(page.locator("[role=alert]")).toContainText("Could not verify current status");
  for (const action of ["enable", "disable", "clear", "rebuild"]) await expect(button(page, action)).toBeDisabled();
  await expect(page.locator("[data-content-status]")).not.toContainText("Enabled");
  await page.evaluate(() => { window.__rejectStatus = false; });
  await page.locator("[data-content-refresh]").click();
  await expect(page.locator("[data-content-status]")).toContainText("Cleanup incomplete");
});

test("interrupted rebuild is visible and can be retried with keyboard controls", async ({ page }, testInfo) => {
  await fixture(page);
  await page.evaluate(() => { window.__content.phase = "rebuildPending"; window.__content.searchable = false; });
  await openControls(page);
  await expect(page.locator("[data-content-status]")).toContainText("Rebuild incomplete");
  await page.screenshot({ path: testInfo.outputPath("11-rebuild-pending.png"), fullPage: true, animations: "disabled" });
  page.on("dialog", dialog => dialog.accept());
  await button(page, "rebuild").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("[data-content-status]")).toContainText("Content index rebuilt");
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings-view="contentIndex"]')).toBeVisible();
});
