import { test, expect } from "@playwright/test";

test("만화 카드의 커버 요청은 같은 파일명도 작품 경로로 구분한다", async ({ page }) => {
  const paths = [
    "8_여성향/[문나영, 이범배] 신혼부부 생활 백서 (1-111화 완)/[0001] 0001화.pdf",
    "8_여성향/[이루이] 시한부의 아이까지 뺏으려 합니다 (1-82화 완)/[0001] 0001화.pdf",
  ];
  const books = paths.map((filePath, index) => ({
    book_id: 410787032 + index,
    file_path: filePath,
    category: filePath.slice(0, filePath.lastIndexOf("/")),
    title: "0001화",
    display_title: filePath.split("/")[1],
    file_type: "pdf",
    updated_time: "2026-10-20T07:59:57.567222",
  }));
  const errors = [];
  const requestedPaths = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/auth/me", (route) => route.fulfill({
    json: { status: "success", result: { role: "admin", email: "test@example.com" } },
  }));
  await page.route("**/category-mappings**", (route) => route.fulfill({
    json: { status: "success", result: {} },
  }));
  await page.route("**/comics/categories**", (route) => route.fulfill({
    json: { status: "success", result: {} },
  }));
  await page.route("**/comics/browse**", (route) => route.fulfill({
    json: { status: "success", result: books, total: books.length, next_cursor: null },
  }));
  await page.route("**/comics/cover/**", (route) => {
    const url = new URL(route.request().url());
    requestedPaths.push(url.searchParams.get("path"));
    return route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="300" height="450"><rect width="300" height="450" fill="#d54848"/></svg>',
    });
  });

  await page.goto("/comics-view");
  for (const book of books) {
    const card = page.locator(".cover-grid-card").filter({ hasText: book.display_title });
    const image = card.locator("img");
    await expect(image).toBeVisible();
    const url = new URL(await image.getAttribute("src"));
    expect(url.searchParams.get("path")).toBe(book.file_path);
    await expect.poll(() => image.evaluate((node) => node.naturalWidth)).toBe(300);
  }
  expect(requestedPaths.sort()).toEqual([...paths].sort());
  expect(errors).toEqual([]);
});
