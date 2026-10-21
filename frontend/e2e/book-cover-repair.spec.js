import { test, expect } from "@playwright/test";

test("손상된 기존 표지를 경고하고 서점 후보로 교체한다", async ({ page }) => {
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAABgAAAAkCAIAAAAcrW15AAAAJ0lEQVR4nO3MMQEAAAgDILV/Zy2xwwMC0FsZE3pEIpFIJBKJRO+jA7U2AUf5aesXAAAAAElFTkSuQmCC", "base64");
  const errors = [], searches = [], previews = [];
  let applied;
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  await page.route("**/admin/book-covers/**", (route) => {
    const url = new URL(route.request().url());
    const endpoint = url.pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [];
    else if (endpoint === "/missing") result = { items: [{
      file_path: "book.epub", title: "미술의 마음", revision: "r1", status: "covered", has_cover: true,
      cover_error: "기존 표지 이미지를 읽을 수 없습니다", cover_image: null,
    }], scanned: 1, total: 1 };
    else if (endpoint.startsWith("/candidates/")) {
      searches.push(url.searchParams.get("include_covered"));
      result = { status: "success", candidates: endpoint.endsWith("yes24") ? [{
        candidate_id: "a".repeat(64), title: "새 표지", detail_url: "https://www.yes24.com/",
      }] : [] };
    } else if (endpoint.startsWith("/candidate-image/")) return route.fulfill({ contentType: "image/png", body: png });
    else if (endpoint === "/preview") {
      previews.push(url.searchParams.get("v"));
      return route.fulfill({ contentType: "image/png", body: png });
    } else if (endpoint === "/apply-image") {
      applied = route.request().postDataJSON();
      result = { revision: "r2", cover_image: { width: 24, height: 36, byte_size: png.length } };
    } else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect(page.getByText("표지 읽기 오류", { exact: true })).toBeVisible();
  const warning = page.getByRole("alert").filter({ hasText: "기존 표지 이미지를 읽을 수 없습니다" });
  await expect(warning).toBeVisible();
  await page.getByRole("button", { name: "직접 입력", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "표지 직접 입력" })).toBeVisible();
  await page.getByRole("button", { name: "취소", exact: true }).click();
  const candidate = page.getByRole("button", { name: "YES24 새 표지 표지 선택", exact: true });
  await expect(candidate).toBeEnabled();
  expect(searches.every((value) => value === "true")).toBe(true);
  expect(previews).toEqual([]);
  await candidate.click();
  const confirm = page.getByRole("button", { name: "미술의 마음 표지 저장 · 행 숨기기" });
  await expect(confirm).toBeEnabled();
  expect(applied).toBeUndefined();
  await expect(warning).toHaveCount(0);
  await page.screenshot({ path: "/tmp/book-cover-repair.png" });
  await confirm.click();
  await expect(page.getByRole("heading", { name: "미술의 마음", exact: true })).toHaveCount(0);
  expect(applied).toEqual({ file_path: "book.epub", revision: "r1", image_data: png.toString("base64") });
  expect(previews).toEqual([]);
  expect(errors).toEqual([]);
});
