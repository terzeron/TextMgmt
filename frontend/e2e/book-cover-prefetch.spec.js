import { test, expect } from "@playwright/test";

const cover = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAABgAAAAkCAIAAAAcrW15AAAAJ0lEQVR4nO3MMQEAAAgDILV/Zy2xwwMC0FsZE3pEIpFIJBKJRO+jA7U2AUf5aesXAAAAAElFTkSuQmCC", "base64");

for (const width of [1440, 390]) {
  test(`하단 3권의 실제 이미지를 스크롤 전에 준비한다: ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    const errors = [], images = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
    await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
    await page.route("**/admin/book-covers/**", async (route) => {
      const url = new URL(route.request().url());
      const endpoint = url.pathname.split("/admin/book-covers")[1];
      let result;
      if (endpoint === "/directories") result = [];
      else if (endpoint === "/missing") result = { items: Array.from({ length: 20 }, (_, index) => ({
        file_path: `${index}.epub`, title: `선행 책 ${index}`, revision: "r1", status: "missing",
      })), scanned: 20, total: 20 };
      else if (endpoint.startsWith("/candidates/")) {
        const index = Number(url.searchParams.get("file_path").split(".")[0]);
        result = { status: "success", candidates: endpoint.endsWith("yes24") ? Array.from({ length: 4 }, (_, candidate) => ({
          candidate_id: `${index}-${candidate}`, title: `후보 ${index}-${candidate}`, detail_url: "https://www.yes24.com/",
        })) : [] };
      } else if (endpoint.startsWith("/candidate-image/")) {
        const id = endpoint.split("/").pop();
        images.push(id);
        // 검색 결과와 실제 이미지 수신 사이에 지연이 있어도 선행 fetch를 완료해야 한다.
        await new Promise((resolve) => setTimeout(resolve, 150));
        return route.fulfill({ contentType: "image/png", body: cover });
      } else return route.fulfill({ status: 404 });
      return route.fulfill({ json: { status: "success", result } });
    });
    await page.goto("/admin");
    await page.getByRole("tab", { name: "책 표지 관리" }).click();
    const rows = page.locator(".book-cover-row");
    await expect(rows).toHaveCount(20);
    await expect(page.getByAltText("후보 0-0")).toBeVisible();
    const upcoming = await rows.evaluateAll((nodes) => nodes.map((node, index) => ({ index, top: node.getBoundingClientRect().top }))
      .filter(({ top }) => top >= innerHeight).slice(0, 3).map(({ index }) => index));
    expect(upcoming).toHaveLength(3);
    for (const index of upcoming) {
      await expect.poll(() => page.getByAltText(`후보 ${index}-0`).evaluateAll((nodes) => nodes.some((img) => img.complete && img.naturalWidth > 0)), { timeout: 8000 }).toBe(true);
      await expect.poll(() => rows.nth(index).locator(".book-cover-candidate .cover-grid-cover").evaluateAll((nodes) => {
        const exposed = nodes.filter((node) => {
          const rect = node.getBoundingClientRect();
          const clip = node.closest(".book-cover-candidates").getBoundingClientRect();
          return rect.right > Math.max(0, clip.left) && rect.left < Math.min(innerWidth, clip.right);
        });
        return exposed.length > 0 && exposed.every((node) => {
          const img = node.querySelector("img");
          return img?.complete && img.naturalWidth > 0;
        });
      }), { timeout: 8000 }).toBe(true);
      expect(await rows.nth(index).evaluate((node) => node.getBoundingClientRect().top)).toBeGreaterThanOrEqual(800);
    }
    expect(images.every((id) => Number(id.split("-")[0]) <= upcoming[2])).toBe(true);
    await rows.nth(upcoming[0]).scrollIntoViewIfNeeded();
    await expect(page.getByAltText(`후보 ${upcoming[0]}-0`)).toBeVisible();
    expect(images.filter((id) => id === `${upcoming[0]}-0`)).toHaveLength(1);
    expect(errors).toEqual([]);
    await page.screenshot({ path: `/tmp/book-cover-prefetch-${width}.png` });
  });
}

test("후보 4개를 가진 서점이 다음 서점과 겹치지 않는다", async ({ page }) => {
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  await page.route("**/admin/book-covers/**", (route) => {
    const endpoint = new URL(route.request().url()).pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [];
    else if (endpoint === "/missing") result = { items: [{ file_path: "book.epub", title: "간격 책", revision: "r1", status: "missing" }], scanned: 1, total: 1 };
    else if (endpoint.startsWith("/candidates/")) {
      const store = endpoint.split("/").pop();
      result = { status: "success", candidates: Array.from({ length: 4 }, (_, index) => ({
        candidate_id: `${store}-${index}`, title: `${store} 후보 ${index}`, detail_url: "https://www.yes24.com/",
      })) };
    } else if (endpoint.startsWith("/candidate-image/")) return route.fulfill({ contentType: "image/png", body: cover });
    else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect(page.getByAltText("yes24 후보 0")).toBeVisible();
  for (const width of [1440, 768, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const next = page.locator('[data-store="aladin"] .book-cover-candidate').first();
    await expect(next).toHaveCount(1);
    await expect.poll(() => page.locator(".book-cover-candidates").evaluate((node) => {
      const lastRect = node.querySelector('[data-store="yes24"] .book-cover-candidate:last-child').getBoundingClientRect();
      const nextRect = node.querySelector('[data-store="aladin"] .book-cover-candidate').getBoundingClientRect();
      return nextRect.left - lastRect.right;
    })).toBeGreaterThanOrEqual(14);
    await page.screenshot({ path: `/tmp/book-cover-spacing-${width}.png` });
  }
});
