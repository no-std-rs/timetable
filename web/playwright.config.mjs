import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  testMatch: "*.spec.mjs",
  // Each test owns its browser context, routes and USB mocks. The server only
  // serves files, so independent tests can share it across all four CI cores.
  fullyParallel: true,
  workers: process.env.CI ? 4 : 1,
  use: { baseURL: "http://127.0.0.1:4197", headless: true, channel: "chromium" },
  webServer: {
    command: "node tests/server.mjs",
    url: "http://127.0.0.1:4197",
    reuseExistingServer: false,
  },
  projects: [
    { name: "desktop", use: { viewport: { width: 1440, height: 1050 } } },
    { name: "mobile", use: { viewport: { width: 390, height: 844 }, isMobile: true } },
  ],
});
