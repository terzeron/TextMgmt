import { readFileSync } from "node:fs";
import { test, expect } from "@playwright/test";

const EPUB_BYTES = readFileSync(
  new URL(
    "../../tests/books/_epub/[Lewis] Alices Adventures in Wonderland.epub",
    import.meta.url,
  ),
);

test("EPUB 전체 보기: viewport 변경 후에도 본문은 잘리지 않는다", async ({
  page,
}) => {
  await page.route("**/auth/me", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "success",
        result: {
          role: "admin",
          name: "E2E Test",
          email: "e2e@example.com",
          picture: "",
        },
      }),
    }),
  );
  await page.route("**/auth/refresh", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success" }),
    }),
  );
  await page.route("**/preview/12345?chapters=0", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/epub+zip",
      headers: {
        "Access-Control-Allow-Origin":
          route.request().headers()["origin"] || "*",
        "Access-Control-Allow-Credentials": "true",
      },
      body: EPUB_BYTES,
    }),
  );

  const browserErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  page.on("pageerror", (error) => browserErrors.push(error.message));

  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto(
    "/viewer/epub/12345?path=" + encodeURIComponent("alice.epub"),
  );

  const viewer = page.locator(".epub-viewer");
  const iframe = viewer.locator("iframe");
  await expect(iframe).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(500);

  // 일정량 넘긴 뒤(viewport 변경)에도 본문이 잘리지 않는다
  const nextBtn = page.locator(".epub-page-next");
  for (let i = 0; i < 5; i++) {
    await nextBtn.click();
    await page.waitForTimeout(200);
  }

  await page.setViewportSize({ width: 900, height: 500 });
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const c = document.querySelector(".epub-viewer .epub-container");
        return c && c.clientHeight <= 500 && c.scrollHeight >= c.clientHeight;
      });
    })
    .toBe(true);

  const stableIframe = await iframe.elementHandle();
  const pageInfo = page.locator(".epub-page-info");
  await expect(pageInfo).not.toHaveText("페이지 계산 중...", {
    timeout: 20_000,
  });
  const stablePageInfo = await pageInfo.textContent();
  await page.waitForTimeout(750);
  expect(
    await page.evaluate(
      (element) => document.querySelector(".epub-viewer iframe") === element,
      stableIframe,
    ),
  ).toBe(true);
  await expect(pageInfo).toHaveText(stablePageInfo);

  expect(browserErrors).toHaveLength(0);
});
