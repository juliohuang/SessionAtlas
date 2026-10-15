import { expect, test } from "@playwright/test";

async function openDemo(page, width, height) {
  await page.setViewportSize({ width, height });
  await page.goto("/index.html");
  await expect(page.locator("#searchInput")).toBeVisible();
  await expect(page.locator("article.entry").first()).toBeVisible();
  // Keep the compact-row assertions independent of a pointer hover state.
  await page.mouse.move(0, 0);
}

test("ledger rows and workspace rails fit at the supported compact sizes", async ({ page }) => {
  for (const viewport of [{ width: 1280, height: 720 }, { width: 980, height: 640 }]) {
    await openDemo(page, viewport.width, viewport.height);
    const metrics = await page.evaluate(() => {
      const row = [...document.querySelectorAll("article.entry")]
        .find(element => element.getBoundingClientRect().width > 0);
      if (!row) throw new Error("no visible project row");
      const rect = selector => row.querySelector(selector)?.getBoundingClientRect();
      const name = rect(".entry__name");
      const body = rect(".entry__body");
      const details = rect(".entry__details");
      const overview = document.querySelector(".stage__overview")?.getBoundingClientRect();
      const terminal = document.querySelector(".stage__right")?.getBoundingClientRect();
      const deck = document.querySelector(".deck")?.getBoundingClientRect();
      const controls = ["#filters", "#projectOrderFilters"].map(selector => {
        const element = document.querySelector(selector);
        const bounds = element?.getBoundingClientRect();
        return {
          left: bounds?.left ?? -1,
          right: bounds?.right ?? -1,
          top: bounds?.top ?? -1,
          bottom: bounds?.bottom ?? -1,
          scrollWidth: element?.scrollWidth ?? -1,
          clientWidth: element?.clientWidth ?? -1,
        };
      });
      return {
        nameWidth: name?.width ?? 0,
        nameBottom: name?.bottom ?? 0,
        bodyBottom: body?.bottom ?? 0,
        detailsTop: details?.top ?? 0,
        detailsRight: details?.right ?? 0,
        rowRight: row.getBoundingClientRect().right,
        overviewWidth: overview?.width ?? 0,
        terminalWidth: terminal?.width ?? 0,
        deckBottom: deck?.bottom ?? 0,
        controls,
        pageOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      };
    });

    expect(metrics.nameWidth, `${viewport.width}px project name`).toBeGreaterThanOrEqual(120);
    expect(metrics.detailsTop, `${viewport.width}px metadata row`).toBeGreaterThanOrEqual(metrics.nameBottom - 1);
    expect(metrics.detailsTop, `${viewport.width}px metadata follows body`).toBeGreaterThanOrEqual(metrics.bodyBottom - 1);
    expect(metrics.detailsRight, `${viewport.width}px metadata width`).toBeLessThanOrEqual(metrics.rowRight + 1);
    expect(metrics.terminalWidth, `${viewport.width}px terminal rail`).toBeGreaterThan(metrics.overviewWidth);
    expect(metrics.pageOverflow, `${viewport.width}px document overflow`).toBeLessThanOrEqual(1);
    for (const control of metrics.controls) {
      expect(control.left, `${viewport.width}px control left edge`).toBeGreaterThanOrEqual(-1);
      expect(control.right, `${viewport.width}px control right edge`).toBeLessThanOrEqual(viewport.width + 1);
      expect(control.bottom, `${viewport.width}px control bottom edge`).toBeLessThanOrEqual(metrics.deckBottom + 1);
    }
  }
});

test("theme toggle and language picker keep the workspace usable", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("sessionatlas.theme", "dark");
    localStorage.setItem("sessionatlas.lang", "en");
  });
  await openDemo(page, 980, 640);

  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.locator("#themeBtn").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.locator("#themeBtn").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  await page.locator("#settingsBtn").click();
  await expect(page.locator("#drawer")).toBeVisible();
  await page.locator('[data-settings-view="language"]').click();
  await page.locator('[data-lang-select="zh"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh");
  await page.locator('[data-settings-view="language"]').click();
  await page.locator('[data-lang-select="en"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
});

test("keyboard focus exposes usable row actions", async ({ page }) => {
  await openDemo(page, 1280, 720);
  const row = page.locator("article.entry").first();
  const treeButton = row.locator("[data-tree-btn]");
  const menuButton = row.locator("[data-menu-toggle]");

  await row.locator("[data-expand-toggle]").focus();
  for (const button of [treeButton, menuButton]) {
    await page.keyboard.press("Tab");
    await expect(button).toBeFocused();
    await expect.poll(() => button.evaluate(element => getComputedStyle(element).opacity)).toBe("1");
    await expect.poll(() => button.evaluate(element => getComputedStyle(element).pointerEvents)).toBe("auto");
    await expect(button).toBeEnabled();
  }

  await menuButton.press("Enter");
  await expect(page.locator("#entryModal")).toBeVisible();
  await expect(page.locator("#entryModalTitle")).not.toHaveText("");
  await page.keyboard.press("Escape");
  await expect(page.locator("#entryModal")).toBeHidden();
  await expect(menuButton).toBeFocused();
});

test("overview collapse and restore preserve the terminal rail", async ({ page }) => {
  await openDemo(page, 980, 640);
  const stage = page.locator("#stage");
  const toggle = page.locator("#overviewToggleBtn");
  const terminal = page.locator(".stage__right");
  const expandedWidth = (await terminal.boundingBox()).width;

  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await toggle.click();
  await expect(stage).toHaveClass(/stage--overview-collapsed/);
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator("#termsSelectedLaunch")).toBeHidden();
  expect((await terminal.boundingBox()).width).toBeGreaterThan(expandedWidth);

  await toggle.click();
  await expect(stage).not.toHaveClass(/stage--overview-collapsed/);
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#termsSelectedLaunch")).toBeVisible();
});

test("settings drawer scrolls vertically without horizontal spill", async ({ page }) => {
  await openDemo(page, 980, 640);
  await page.locator("#settingsBtn").click();
  await expect(page.locator("#drawer")).toBeVisible();

  const metrics = await page.evaluate(() => {
    const body = document.querySelector("#drawerBody");
    const panel = document.querySelector(".drawer__panel");
    return {
      bodyScrollable: body.scrollHeight > body.clientHeight,
      bodyHorizontalSpill: body.scrollWidth - body.clientWidth,
      panelHorizontalSpill: panel.scrollWidth - panel.clientWidth,
      pageHorizontalSpill: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    };
  });
  expect(metrics.bodyScrollable).toBe(true);
  expect(metrics.bodyHorizontalSpill).toBeLessThanOrEqual(1);
  expect(metrics.panelHorizontalSpill).toBeLessThanOrEqual(1);
  expect(metrics.pageHorizontalSpill).toBeLessThanOrEqual(1);
});
