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
