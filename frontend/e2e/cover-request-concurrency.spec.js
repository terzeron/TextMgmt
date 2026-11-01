import { test, expect } from "@playwright/test";

test("커버 목록 전환은 이전 요청의 응답을 기다리고 동시 요청 10개를 유지한다", async ({ page }) => {
  let releaseOld, releaseNew;
  const oldBatch = new Promise((resolve) => { releaseOld = resolve; });
  const newBatch = new Promise((resolve) => { releaseNew = resolve; });
  let started = 0, active = 0, peak = 0;
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAABgAAAAkCAIAAAAcrW15AAAAJ0lEQVR4nO3MMQEAAAgDILV/Zy2xwwMC0FsZE3pEIpFIJBKJRO+jA7U2AUf5aesXAAAAAElFTkSuQmCC", "base64");
  await page.route("https://api-tm.terzeron.com/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.startsWith("/cover/")) {
      started++;
      peak = Math.max(peak, ++active);
      await (Number(url.pathname.split("/").pop()) < 1000 ? oldBatch : newBatch);
      active--;
      await route.fulfill({ contentType: "image/png", body: png });
      return;
    }
    const body = { status: "success", result: [] };
    if (url.pathname === "/auth/me") body.result = { role: "admin", name: "E2E", email: "e2e@example.com" };
    else if (url.pathname === "/categories") body.result = { "2_소설한국": 60 };
    else if (url.pathname === "/browse") {
      const offset = url.searchParams.has("uncovered") ? 1000 : 0;
      body.result = Array.from({ length: 60 }, (_, index) => ({
        book_id: offset + index + 1,
        title: `Cover ${index + 1}`,
        category: "2_소설한국",
        file_type: index % 2 ? "epub" : "pdf",
        file_path: `2_소설한국/${offset + index + 1}.${index % 2 ? "epub" : "pdf"}`,
      }));
      body.total = 60;
      body.next_cursor = null;
    }
    await route.fulfill({ json: body });
  });
  try {
    await page.goto("/book-view");
    await expect.poll(() => started).toBe(10);
    await page.getByRole("button", { name: "미커버", exact: true }).click();
    await expect(page.locator('.cover-grid-card[href*="1001?"]')).toBeVisible();
    expect(started).toBe(10);
    releaseOld();
    await expect.poll(() => started).toBe(20);
    expect(peak).toBe(10);
  } finally {
    releaseOld();
    releaseNew();
  }
});
