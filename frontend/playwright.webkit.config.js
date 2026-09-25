import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: "list",
  use: {
    baseURL: "https://localhost:4173",
    ignoreHTTPSErrors: true,
    browserName: "webkit",
    launchOptions: { args: [] },
  },
  webServer: {
    command: "npm run preview",
    url: "https://localhost:4173",
    reuseExistingServer: true,
    timeout: 30_000,
    ignoreHTTPSErrors: true,
  },
});
