import { test, expect } from "@playwright/test";

const coverPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==", "base64");
const candidatePngs = [
  "iVBORw0KGgoAAAANSUhEUgAAABgAAAAkCAIAAAAcrW15AAAAJ0lEQVR4nO3MMQEAAAgDILV/Zy2xwwMC0FsZE3pEIpFIJBKJRO+jA7U2AUf5aesXAAAAAElFTkSuQmCC",
  "iVBORw0KGgoAAAANSUhEUgAAAAwAAAASCAIAAADgy6hbAAAAF0lEQVR4nGP8z0AYMBGhZlTRqCKqKgIAx/MBIw2t0OcAAAAASUVORK5CYII=",
  "iVBORw0KGgoAAAANSUhEUgAAAAYAAAAJCAIAAACe+MrKAAAAEklEQVR4nGP8z4AOmDBEBocQALAEARGCdtBgAAAAAElFTkSuQmCC",
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAE0lEQVR4nGP8z8DAwMDAxIBMAQAUQAEF3SN5DgAAAABJRU5ErkJggg==",
].map((base64) => Buffer.from(base64, "base64"));

test("표지 관리: 오른쪽 서점은 가로 스크롤로 노출될 때 검색하고 이미지를 재사용한다", async ({ page }) => {
  await page.setViewportSize({ width: 780, height: 900 });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  const stores = ["yes24", "aladin", "kyobo", "naver", "ridi", "munpia", "naverseries", "joara"];
  const searches = [], images = [];
  await page.route("**/admin/book-covers/**", async (route) => {
    const endpoint = new URL(route.request().url()).pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [];
    else if (endpoint === "/missing") result = { items: [{ file_path: "lazy.epub", title: "가로 스크롤 책", revision: "r1", status: "missing" }], scanned: 1, total: 1 };
    else if (endpoint.startsWith("/candidates/")) {
      const store = endpoint.split("/").pop();
      searches.push(store);
      result = { status: "success", candidates: [{ candidate_id: String(stores.indexOf(store) + 1).repeat(64), title: `${store} 표지`, detail_url: "https://www.yes24.com/" }] };
    } else if (endpoint.startsWith("/candidate-image/")) {
      images.push(endpoint.split("/").pop());
      return route.fulfill({ contentType: "image/png", body: candidatePngs[0] });
    } else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect.poll(() => stores.slice(0, 4).every((store) => searches.includes(store))).toBe(true);
  await expect(page.getByAltText("yes24 표지")).toBeVisible();
  expect(searches.filter((store) => stores.indexOf(store) >= 4)).toEqual([]);
  for (const store of stores.slice(4)) {
    const section = page.locator(`.book-cover-store[data-store="${store}"]`);
    await section.evaluate((node) => {
      const clip = node.closest(".book-cover-candidates");
      clip.scrollLeft += node.getBoundingClientRect().left - clip.getBoundingClientRect().left;
    });
    await expect(section.getByAltText(`${store} 표지`)).toBeVisible();
    expect(searches.filter((name) => name === store)).toHaveLength(1);
    expect(images.filter((id) => id === String(stores.indexOf(store) + 1).repeat(64))).toHaveLength(1);
  }
  await page.locator('.book-cover-store[data-store="ridi"]').evaluate((node) => {
    const clip = node.closest(".book-cover-candidates");
    clip.scrollLeft += node.getBoundingClientRect().left - clip.getBoundingClientRect().left;
  });
  await expect(page.getByAltText("ridi 표지")).toBeVisible();
  expect(searches.filter((store) => store === "ridi")).toHaveLength(1);
  expect(images.filter((id) => id === "5".repeat(64))).toHaveLength(1);
  expect(errors).toEqual([]);
});

test("표지 관리: 화면 밖의 책은 5초 후에 미리 검색한다", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", name: "테스트", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  const searches = [];
  await page.route("**/admin/book-covers/**", async (route) => {
    const url = new URL(route.request().url());
    const endpoint = url.pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [];
    else if (endpoint === "/missing") result = { items: Array.from({ length: 20 }, (_, index) => ({
      file_path: `책${index}.epub`, title: `책 ${index}`, revision: "r1", status: "missing",
    })), scanned: 20, total: 20 };
    else if (endpoint.startsWith("/candidates/")) {
      searches.push({ index: Number(url.searchParams.get("file_path").match(/\d+/)[0]), time: Date.now() });
      result = { status: "not_found", candidates: [] };
    } else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  const start = Date.now();
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect(page.getByRole("heading", { name: "책 0", exact: true })).toBeVisible();
  const visible = await page.locator(".book-cover-row").evaluateAll((rows) => rows.map((node, index) => ({ index, rect: node.getBoundingClientRect() }))
    .filter(({ rect }) => rect.bottom > 0 && rect.top < innerHeight).map(({ index }) => index));
  await expect.poll(() => searches.length).toBeGreaterThan(0);
  expect(searches.every(({ index }) => visible.includes(index))).toBe(true);
  await expect.poll(() => searches.some(({ index }) => !visible.includes(index)), { timeout: 8000 }).toBe(true);
  const background = searches.filter(({ index }) => !visible.includes(index));
  expect(background.every(({ time }) => time - start >= 4800)).toBe(true);
  expect(new Set(background.map(({ index }) => index)).size).toBeLessThanOrEqual(Math.ceil(visible.length * 0.25));
});

test("표지 관리: 자동 추가 검사, viewport fetch 제한과 저장 후 완료 확인", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", name: "테스트 관리자", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  const imageRequests = [];
  const searches = [];
  const listingCursors = [];
  let saved = false;
  await page.route("**/admin/book-covers/**", async (route) => {
    const url = new URL(route.request().url());
    const endpoint = url.pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [{ id: "fiction", label: "소설", fileType: "folder", children: [] }];
    else if (endpoint === "/missing") {
      const cursor = url.searchParams.get("cursor");
      listingCursors.push(cursor);
      const offset = Number(cursor || 0);
      result = {
        items: Array.from({ length: 30 }, (_, index) => ({ file_path: `책${offset + index}.epub`, title: `표지 없는 책 ${offset + index}`, revision: "r1", status: "missing", book_id: 1000 + offset + index, category: "_root" })),
        scanned: offset + 30, total: 60, next_cursor: offset === 0 ? "30" : null,
      };
    }
    else if (endpoint.startsWith("/candidates/")) {
      const store = endpoint.split("/").pop();
      const index = Number(url.searchParams.get("file_path").match(/\d+/)[0]);
      searches.push({ store, index });
      result = { status: store === "yes24" ? "success" : "not_found", candidates: store === "yes24" ? Array.from({ length: 4 }, (_, candidate) => ({
        candidate_id: (index * 4 + candidate).toString(16).padStart(64, "0"),
        title: `서점 후보 ${index}-${candidate}`, author: "저자", detail_url: "https://www.yes24.com/",
      })) : [] };
    } else if (endpoint.startsWith("/candidate-image/")) {
      imageRequests.push(endpoint.split("/").pop());
      const candidate = parseInt(endpoint.split("/").pop(), 16) % 4;
      return route.fulfill({ contentType: "image/png", body: candidatePngs[candidate] });
    } else if (endpoint === "/apply") { saved = true; result = { revision: "r2" }; }
    else if (endpoint === "/preview") return route.fulfill({ contentType: "image/png", body: coverPng });
    else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect(page.getByRole("tree")).toBeVisible();
  const directoryWidth = await page.locator(".book-cover-directory").evaluate((node) => node.getBoundingClientRect().width);
  const panelWidth = await page.locator(".book-cover-panel").evaluate((node) => node.getBoundingClientRect().width);
  expect(panelWidth / directoryWidth).toBeCloseTo(5, 1);
  const firstHeader = page.locator(".book-cover-row-header").first();
  await expect(firstHeader.getByRole("heading")).toHaveCSS("font-size", "20px");
  await expect(firstHeader.getByRole("button", { name: "편집" })).toBeEnabled();
  await expect(firstHeader.getByRole("button", { name: "조회" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "다음 EPUB 검사" })).toHaveCount(0);
  const first = page.getByRole("button", { name: "YES24 서점 후보 0-0 표지 선택" });
  await expect(first).toBeEnabled();
  await expect(first).toHaveAttribute("title", "해상도 1위 / 4개");
  await expect(first).toHaveCSS("background-color", "rgb(0, 255, 0)");
  await expect(first).toHaveCSS("padding", "5px");
  expect(await first.evaluate((node) => node.getBoundingClientRect().width)).toBe(84);
  const ownCover = page.locator(".book-cover-current .cover-grid-cover").first();
  expect(await ownCover.evaluate((node) => node.getBoundingClientRect().width)).toBe(90);
  const ownTop = await ownCover.evaluate((node) => node.getBoundingClientRect().top);
  const candidateTop = await first.evaluate((node) => node.getBoundingClientRect().top);
  expect(ownTop).toBeCloseTo(candidateTop, 1);
  for (const magnifier of [page.getByRole("button", { name: "표지 없는 책 0 내 책 표지 확대", exact: true }),
    page.getByRole("button", { name: "YES24 서점 후보 0-0 표지 확대", exact: true })]) {
    await expect(magnifier).toHaveCSS("position", "absolute");
    const distances = await magnifier.evaluate((node) => {
      const cover = node.parentElement.querySelector(".cover-grid-cover").getBoundingClientRect();
      const button = node.getBoundingClientRect();
      return { top: button.top - cover.top, right: cover.right - button.right };
    });
    expect(distances.top).toBeCloseTo(2, 1);
    expect(distances.right).toBeCloseTo(2, 1);
  }
  await expect(page.getByRole("button", { name: "표지 없는 책 0 내 책 표지 확대", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "YES24 서점 후보 0-1 표지 선택" })).toHaveCSS("background-color", "rgb(170, 255, 0)");
  await expect(page.getByRole("button", { name: "YES24 서점 후보 0-2 표지 선택" })).toHaveCSS("background-color", "rgb(255, 170, 0)");
  await expect(page.getByRole("button", { name: "YES24 서점 후보 0-3 표지 선택" })).toHaveCSS("background-color", "rgb(255, 0, 0)");
  await expect(page.locator(".book-cover-row").first().getByText("24 × 36 px")).toBeVisible();
  await page.waitForTimeout(700);
  const visible = await page.locator(".book-cover-candidate .cover-grid-cover").evaluateAll((nodes) => nodes.filter((node) => {
    const rect = node.getBoundingClientRect();
    const clip = node.closest(".book-cover-candidates").getBoundingClientRect();
    return rect.bottom > 0 && rect.top < innerHeight && rect.right > Math.max(0, clip.left) && rect.left < Math.min(innerWidth, clip.right);
  }).length);
  expect(imageRequests.length, JSON.stringify({ visible, images: imageRequests.map((id) => parseInt(id, 16)), searches })).toBeLessThanOrEqual(visible + Math.ceil(visible * 0.25));
  expect(searches.every(({ index }) => index < 6)).toBe(true);
  expect(listingCursors).toEqual([""]);
  const beforeZoom = imageRequests.length;
  await first.hover();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.locator(".book-cover-row").first().getByRole("button", { name: "YES24 서점 후보 0-0 표지 확대", exact: true }).click();
  const expanded = page.getByRole("dialog", { name: "서점 후보 0-0 표지 확대" });
  await expect(expanded).toBeVisible();
  const image = expanded.getByAltText("서점 후보 0-0 확대 표지");
  await expect(image).toHaveCSS("max-width", "none");
  expect(await image.evaluate((node) => ({ width: node.getBoundingClientRect().width, height: node.getBoundingClientRect().height }))).toEqual({ width: 24, height: 36 });
  expect(imageRequests.length).toBe(beforeZoom);
  await page.mouse.move(0, 0);
  await expect(expanded).toBeVisible();
  await expanded.getByRole("button", { name: "서점 후보 0-0 확대 표지 선택" }).click();
  const completed = page.getByRole("button", { name: "표지 없는 책 0 표지 저장 완료 · 행 숨기기" });
  await expect(completed).toBeEnabled();
  expect(saved).toBe(true);
  await page.getByRole("button", { name: "표지 없는 책 0 내 책 표지 확대", exact: true }).click();
  const ownModal = page.getByRole("dialog", { name: "표지 없는 책 0 내 책 표지 확대", exact: true });
  await expect(ownModal.getByAltText("표지 없는 책 0 내 책 확대 표지")).toBeVisible();
  await expect(ownModal.getByRole("button", { name: /확대 표지 선택/ })).toHaveCount(0);
  await ownModal.getByRole("button", { name: "확대 닫기" }).click();
  await expect(page.getByRole("heading", { name: "표지 없는 책 0", exact: true })).toBeVisible();
  await page.screenshot({ path: "/tmp/tm-cover-admin-completed.png", fullPage: false });
  await completed.click();
  await expect(page.getByRole("heading", { name: "표지 없는 책 0", exact: true })).toHaveCount(0);
  // 서점 후보가 나타나면서 행 높이가 바뀌므로 실제 목록 끝까지 계속 스크롤한다.
  await expect.poll(async () => {
    await page.locator(".book-cover-scan-trigger").scrollIntoViewIfNeeded();
    return listingCursors.length;
  }).toBe(2);
  await expect(page.getByRole("heading", { name: "표지 없는 책 59", exact: true })).toHaveCount(1);
  expect(listingCursors).toEqual(["", "30"]);
  await expect(page.getByRole("heading", { name: "표지 없는 책 1", exact: true })).toHaveCount(1);
  expect(errors).toEqual([]);
});

test("표지 관리: 직접 URL, 미리보기 삭제, 교체, 재검색과 서점 재시도", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  let searches = 0, cleared = 0, applied, replaced, retries = 0;
  const id = "a".repeat(64);
  await page.route("**/admin/book-covers/**", async (route) => {
    const url = new URL(route.request().url());
    const endpoint = url.pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [];
    else if (endpoint === "/missing") result = { items: [{ title: "직접 입력 책", file_path: "book.epub", revision: "r1", book_id: 101, category: "_root", status: "missing" }], scanned: 1, total: 1 };
    else if (endpoint.startsWith("/candidates/")) {
      if (endpoint.endsWith("aladin")) {
        if (url.searchParams.get("refresh") === "true") {
          retries += 1;
          await new Promise((resolve) => setTimeout(resolve, 300));
          return route.fulfill({ json: { status: "success", result: { status: "not_found", candidates: [] } } });
        }
        return route.fulfill({ json: { status: "success", result: { status: "error", error: "서점 접근 실패", candidates: [] } } });
      }
      const yes24 = endpoint.endsWith("yes24");
      if (yes24) searches += 1;
      result = { status: yes24 ? "success" : "not_found", candidates: yes24 ? [{ candidate_id: id, title: "서점 표지", author: "저자", detail_url: "https://www.yes24.com/" }] : [] };
    } else if (endpoint.startsWith("/candidate-image/") || endpoint === "/preview") return route.fulfill({ contentType: "image/png", body: candidatePngs[0] });
    else if (endpoint === "/cache/clear") { cleared += 1; result = { cleared: true }; }
    else if (endpoint === "/apply-url") { applied = route.request().postDataJSON(); result = { revision: "r2" }; }
    else if (endpoint === "/apply-image") { replaced = route.request().postDataJSON(); result = { revision: "r3" }; }
    else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  const candidate = page.getByRole("button", { name: "YES24 서점 표지 표지 선택" });
  await expect(candidate).toBeEnabled();
  await page.getByRole("button", { name: "재시도", exact: true }).click();
  await expect(page.getByRole("button", { name: "재시도 중…" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "재시도 중…" })).toHaveCount(0);
  expect(retries).toBe(1);
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: "직접 입력 책 임시 표지 삭제" }).click();
  expect(cleared).toBe(0);
  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "직접 입력 책 임시 표지 삭제" }).click();
  await expect(candidate).toBeEnabled();
  expect(cleared).toBe(0);
  expect(searches).toBe(1);
  await page.getByRole("button", { name: "재검색", exact: true }).click();
  await expect(candidate).toBeEnabled();
  expect(searches).toBe(2);
  await page.getByRole("button", { name: "직접 입력", exact: true }).click();
  const modal = page.getByRole("dialog", { name: "표지 직접 입력" });
  await expect(modal).toBeVisible();
  await modal.getByLabel("표지 이미지 URL").fill("https://example.org/cover.png");
  await modal.getByRole("button", { name: "표지 적용" }).click();
  const completed = page.getByRole("button", { name: /직접 입력 책 표지 저장 완료/ });
  await expect(completed).toBeEnabled();
  expect(applied).toEqual({ file_path: "book.epub", revision: "r1", url: "https://example.org/cover.png" });
  const trash = page.getByRole("button", { name: "직접 입력 책 임시 표지 삭제" });
  expect(await trash.evaluate((node) => node.parentElement.className)).toBe("book-cover-status");
  page.once("dialog", (dialog) => dialog.accept());
  await trash.click();
  await expect(completed).toHaveCount(0);
  await expect(candidate).toBeEnabled();
  expect(cleared).toBe(1);
  await expect(page.getByRole("button", { name: "재검색", exact: true })).toBeEnabled();
  await candidate.click();
  await expect(completed).toBeEnabled();
  expect(replaced.file_path).toBe("book.epub");
  expect(replaced.revision).toBe("r2");
  expect(Buffer.from(replaced.image_data, "base64")).toEqual(candidatePngs[0]);
  await completed.click();
  await expect(page.getByRole("heading", { name: "직접 입력 책", exact: true })).toHaveCount(0);
});
