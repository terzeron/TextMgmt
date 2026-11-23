import { test, expect } from "@playwright/test";

for (const [basePath, apiPrefix] of [
  ["/book-view", ""],
  ["/comics-view", "/comics"],
]) {
  test(`${basePath}: 소수점과 %가 있는 카테고리의 딥링크를 연다`, async ({ page }) => {
    const category = "1_웹툰/상위 0.001%랭커의 귀환";
    const book = {
      book_id: 410856556,
      title: "상위 0.001%랭커의 귀환 1화",
      category,
      file_type: "png",
      file_path: `${category}/1화.png`,
    };
    const requestedCategories = [];
    const pageErrors = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    await page.addInitScript(() => {
      window.__ENV__ = {
        VITE_API_URL_PREFIX: "https://localhost:4173/api",
        VITE_GOOGLE_CLIENT_ID: "e2e.apps.googleusercontent.com",
      };
    });
    await page.route("**/api/**", async (route) => {
      const url = new URL(route.request().url());
      const path = url.pathname.slice("/api".length);
      const fulfill = (result) => route.fulfill({
        json: { status: "success", result },
      });
      if (path === "/auth/me") return fulfill({ role: "admin", name: "E2E" });
      if (path === apiPrefix + "/categories") return fulfill({ [category]: 1 });
      if (path.startsWith(apiPrefix + "/categories/")) {
        let requestedCategory;
        try {
          requestedCategory = decodeURIComponent(path.slice((apiPrefix + "/categories/").length));
        } catch {
          return route.fulfill({ status: 400, body: "Invalid URL encoding" });
        }
        requestedCategories.push(requestedCategory);
        return fulfill(requestedCategory === category ? [book] : []);
      }
      if (path === apiPrefix + "/books/" + book.book_id) return fulfill(book);
      if (path === apiPrefix + "/download/" + book.book_id) {
        return route.fulfill({
          contentType: "image/svg+xml",
          body: '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>',
        });
      }
      return fulfill([]);
    });

    await page.goto(`${basePath}/${book.book_id}?category=${encodeURIComponent(category)}`);

    const image = page.getByRole("img", { name: "book cover", exact: true });
    await expect(image).toBeVisible();
    await expect.poll(() => image.evaluate((element) => element.naturalWidth)).toBeGreaterThan(0);
    expect(requestedCategories).toContain(category);
    expect(pageErrors).toEqual([]);
    expect(new URL(page.url()).searchParams.get("category")).toBe(category);
  });
}
