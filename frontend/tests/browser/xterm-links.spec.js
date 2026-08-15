import { expect, test } from "@playwright/test";

test("vendored xterm links use real buffer line and cell coordinates", async ({ page }) => {
  await page.goto("/index.html");
  const links = await page.evaluate(async () => {
    const host = document.createElement("div");
    host.style.width = "1200px";
    host.style.height = "300px";
    document.body.appendChild(host);
    const terminal = new window.Terminal({ cols: 100, rows: 10 });
    const originalRegister = terminal.registerLinkProvider.bind(terminal);
    let provider;
    terminal.registerLinkProvider = candidate => {
      provider = candidate;
      return originalRegister(candidate);
    };
    terminal.open(host);
    terminal.loadAddon(new window.WebLinksAddon.WebLinksAddon(() => {}));
    await new Promise(resolve => terminal.write(
      "\r\n\r\n\r\n\r\n\r\n\r\n前缀 https://example.com/a https://openai.com/b",
      resolve,
    ));
    const found = await new Promise(resolve => provider.provideLinks(7, resolve));
    terminal.dispose();
    host.remove();
    return found.map(link => ({ text: link.text, range: link.range }));
  });

  expect(links).toHaveLength(2);
  expect(links[0]).toMatchObject({
    text: "https://example.com/a",
    range: { start: { x: 6, y: 7 }, end: { y: 7 } },
  });
  expect(links[1]).toMatchObject({
    text: "https://openai.com/b",
    range: { start: { y: 7 }, end: { y: 7 } },
  });
});

test("vendored provider ignores non-HTTP protocols", async ({ page }) => {
  await page.goto("/index.html");
  const texts = await page.evaluate(async () => {
    const host = document.createElement("div");
    host.style.width = "1000px";
    host.style.height = "200px";
    document.body.appendChild(host);
    const terminal = new window.Terminal({ cols: 100, rows: 5 });
    const originalRegister = terminal.registerLinkProvider.bind(terminal);
    let provider;
    terminal.registerLinkProvider = candidate => {
      provider = candidate;
      return originalRegister(candidate);
    };
    terminal.open(host);
    terminal.loadAddon(new window.WebLinksAddon.WebLinksAddon(() => {}));
    await new Promise(resolve => terminal.write(
      "javascript:alert(1) file:///tmp/a https://safe.example/path",
      resolve,
    ));
    const found = await new Promise(resolve => provider.provideLinks(1, resolve));
    terminal.dispose();
    host.remove();
    return found.map(link => link.text);
  });

  expect(texts).toEqual(["https://safe.example/path"]);
});
