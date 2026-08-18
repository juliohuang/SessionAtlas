import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  globalSetup: "./tests/browser-global-setup.mjs",
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  reporter: "line",
  use: {
    baseURL: "http://127.0.0.1:4173",
    browserName: "chromium",
    headless: true,
  },
});
