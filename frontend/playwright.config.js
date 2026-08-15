import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  reporter: "line",
  use: {
    baseURL: "http://127.0.0.1:4173",
    browserName: "chromium",
    headless: true,
  },
  webServer: {
    command: "node tests/static-server.mjs",
    url: "http://127.0.0.1:4173/index.html",
    reuseExistingServer: false,
    timeout: 15_000,
  },
});
