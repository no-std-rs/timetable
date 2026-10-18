import { defineConfig } from "@playwright/test";
var stdin_default = defineConfig({ testDir: "./tests", testMatch: "*.spec.mjs", workers: 1, use: { baseURL: "http://127.0.0.1:4197", headless: true, channel: "chromium" }, webServer: { command: "node tests/server.mjs", url: "http://127.0.0.1:4197", reuseExistingServer: false }, projects: [{ name: "desktop", use: { viewport: { width: 1440, height: 1050 } } }, { name: "mobile", use: { viewport: { width: 390, height: 844 }, isMobile: true } }] });
export {
  stdin_default as default
};
