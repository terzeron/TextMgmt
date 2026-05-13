import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: "list",
  use: {
    baseURL: "https://localhost:4173",
    ignoreHTTPSErrors: true,
    // Use system Chrome because Playwright-bundled Chromium is unavailable on Ubuntu 26.04
    channel: "chrome",
    launchOptions: {
      executablePath: "/usr/bin/google-chrome",
      args: ["--no-sandbox", "--disable-setuid-sandbox"],
    },
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "npm run preview",
    url: "https://localhost:4173",
    reuseExistingServer: true,
    timeout: 30_000,
    ignoreHTTPSErrors: true,
  },
});
