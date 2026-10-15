import { test, expect } from "@playwright/test";

const coverPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==", "base64");
const candidatePngs = [
  "iVBORw0KGgoAAAANSUhEUgAAABgAAAAkCAIAAAAcrW15AAAAJ0lEQVR4nO3MMQEAAAgDILV/Zy2xwwMC0FsZE3pEIpFIJBKJRO+jA7U2AUf5aesXAAAAAElFTkSuQmCC",
  "iVBORw0KGgoAAAANSUhEUgAAAAwAAAASCAIAAADgy6hbAAAAF0lEQVR4nGP8z0AYMBGhZlTRqCKqKgIAx/MBIw2t0OcAAAAASUVORK5CYII=",
  "iVBORw0KGgoAAAANSUhEUgAAAAYAAAAJCAIAAACe+MrKAAAAEklEQVR4nGP8z4AOmDBEBocQALAEARGCdtBgAAAAAElFTkSuQmCC",
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAE0lEQVR4nGP8z8DAwMDAxIBMAQAUQAEF3SN5DgAAAABJRU5ErkJggg==",
].map((base64) => Buffer.from(base64, "base64"));

test("표지 관리: 초기 빈 칸의 일시 노출로 뒤쪽 서점을 먼저 검색하지 않는다", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  const firstStores = ["yes24", "aladin", "kyobo", "naver"];
  const searches = [];
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  await page.route("**/admin/book-covers/**", async (route) => {
    const endpoint = new URL(route.request().url()).pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [];
    else if (endpoint === "/missing") result = { items: [{ file_path: "[랜들 개릿] 다아시 경의 모험.epub", title: "랜들 개릿 다아시 경의 모험", revision: "r1", status: "missing" }], scanned: 1, total: 1 };
    else if (endpoint.startsWith("/candidates/")) {
      const store = endpoint.split("/").pop();
      searches.push(store);
      if (firstStores.includes(store)) {
        await pending;
        result = { status: "success", candidates: Array.from({ length: 4 }, (_, index) => ({
          candidate_id: (firstStores.indexOf(store) * 4 + index).toString(16).padStart(64, "0"),
          title: `${store} 후보 ${index}`, detail_url: "https://www.yes24.com/",
        })) };
      } else result = { status: "not_found", candidates: [] };
    } else if (endpoint.startsWith("/candidate-image/")) return route.fulfill({ contentType: "image/png", body: candidatePngs[0] });
    else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  try {
    await expect(page.getByRole("heading", { name: "랜들 개릿 다아시 경의 모험", exact: true })).toBeVisible();
    const google = page.getByRole("link", { name: "구글" });
    expect(new URL(await google.getAttribute("href")).searchParams.get("q")).toBe("랜들 개릿 다아시 경의 모험");
    const own = page.locator(".book-cover-current");
    const save = own.getByRole("button", { name: "랜들 개릿 다아시 경의 모험 저장 확인" });
    await expect(own.getByText("— × — px")).toBeVisible();
    await expect(own.getByText("— B")).toBeVisible();
    await expect(own.getByText("서점 표지를 선택하세요")).toHaveCount(0);
    await expect(save).toBeVisible();
    await expect(save).toBeDisabled();
    await expect(save.locator('svg[data-icon="floppy-disk"]')).toBeVisible();
    const saveRect = await save.boundingBox();
    const trashRect = await own.getByRole("button", { name: "랜들 개릿 다아시 경의 모험 임시 표지 삭제" }).boundingBox();
    expect(trashRect.x).toBeGreaterThan(saveRect.x + saveRect.width);
    expect(saveRect.width).toBe(28);
    await expect.poll(() => searches.filter((store) => firstStores.includes(store)).length).toBe(4);
    expect(searches.filter((store) => !firstStores.includes(store))).toEqual([]);
  } finally { release(); }
  await expect(page.getByAltText("yes24 후보 0")).toBeVisible();
  const ridi = page.locator('.book-cover-store[data-store="ridi"]');
  await expect.poll(() => ridi.evaluate((node) => node.getBoundingClientRect().left >= node.closest(".book-cover-candidates").getBoundingClientRect().right)).toBe(true);
  expect(searches.filter((store) => !firstStores.includes(store))).toEqual([]);
  await ridi.evaluate((node) => {
    const clip = node.closest(".book-cover-candidates");
    clip.scrollLeft += node.getBoundingClientRect().left - clip.getBoundingClientRect().left;
  });
  await expect.poll(() => searches.includes("ridi")).toBe(true);
});

test("표지 관리: 카테고리 옆 교체 토글로 기존 표지를 비교하고 교체한다", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  const listings = [], searches = [];
  let applied;
  let releaseSave;
  const pendingSave = new Promise((resolve) => { releaseSave = resolve; });
  await page.route("**/admin/book-covers/**", async (route) => {
    const url = new URL(route.request().url());
    const endpoint = url.pathname.split("/admin/book-covers")[1];
    let result;
    if (endpoint === "/directories") result = [];
    else if (endpoint === "/missing") {
      const covered = url.searchParams.get("covered") === "true";
      listings.push(covered);
      result = { items: covered ? [{ file_path: "covered.epub", title: "기존 표지 책", revision: "r1", has_cover: true, status: "covered", cover_image: { width: 24, height: 36, byte_size: candidatePngs[0].length } }] : [], scanned: 1, total: 1 };
    } else if (endpoint.startsWith("/candidates/")) {
      searches.push(url.searchParams.get("include_covered"));
      result = { status: endpoint.endsWith("yes24") ? "success" : "not_found", candidates: endpoint.endsWith("yes24") ? [{ candidate_id: "a".repeat(64), title: "교체 후보", detail_url: "https://www.yes24.com/" }] : [] };
    } else if (endpoint.startsWith("/candidate-image/") || endpoint === "/preview") return route.fulfill({ contentType: "image/png", body: candidatePngs[0] });
    else if (endpoint === "/apply-image") { applied = route.request().postDataJSON(); await pendingSave; result = { revision: "r2", cover_image: { width: 24, height: 36, byte_size: candidatePngs[0].length } }; }
    else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect(page.getByText("검사한 EPUB에는 표지 없는 책이 없습니다.")).toBeVisible();
  const toggle = page.getByRole("switch", { name: "표지 교체" });
  await expect(toggle).not.toBeChecked();
  const category = page.locator(".book-cover-directory-mode small");
  const categoryRect = await category.boundingBox();
  const toggleRect = await toggle.boundingBox();
  expect(toggleRect.x).toBeGreaterThan(categoryRect.x + categoryRect.width);
  expect(Math.abs((toggleRect.y + toggleRect.height / 2) - (categoryRect.y + categoryRect.height / 2))).toBeLessThan(1);
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(page.getByAltText("기존 표지 책 현재 표지")).toBeVisible();
  await expect(page.getByRole("button", { name: "기존 표지 책 현재 표지", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: /표지 저장 완료/ })).toHaveCount(0);
  const own = page.locator(".book-cover-current");
  const confirm = own.getByRole("button", { name: "기존 표지 책 저장 확인" });
  await expect(own.getByText("24 × 36 px")).toBeVisible();
  await expect(confirm).toBeVisible();
  await expect(confirm).toBeDisabled();
  const candidate = page.getByRole("button", { name: "YES24 교체 후보 표지 선택" });
  await expect(candidate).toBeEnabled();
  expect(searches.every((value) => value === "true")).toBe(true);
  await candidate.click();
  try {
    await expect(confirm.getByRole("status", { name: "기존 표지 책 저장 중" })).toBeVisible();
    await expect(confirm).toBeDisabled();
    await expect(confirm).toHaveAttribute("aria-busy", "true");
  } finally { releaseSave(); }
  await expect.poll(() => applied?.revision).toBe("r1");
  expect(Buffer.from(applied.image_data, "base64")).toEqual(candidatePngs[0]);
  const completed = page.getByRole("button", { name: "기존 표지 책 표지 저장 완료 · 행 숨기기" });
  await expect(completed).toBeEnabled();
  await expect(own.getByText("24 × 36 px")).toBeVisible();
  await expect(own.getByText(`${candidatePngs[0].length} B`, { exact: true })).toBeVisible();
  await expect(own.getByText("저장 완료 · 표지를 눌러 숨기기")).toHaveCount(0);
  await expect(confirm.getByRole("status")).toHaveCount(0);
  await expect(confirm.locator('svg[data-icon="floppy-disk"]')).toBeVisible();
  const metadataRect = await own.locator(".book-cover-image-size").boundingBox();
  for (const button of [confirm, own.getByRole("button", { name: "기존 표지 책 임시 표지 삭제" })]) {
    const rect = await button.boundingBox();
    expect(rect.y).toBeGreaterThanOrEqual(metadataRect.y + metadataRect.height);
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(confirm).toBeVisible();
  const mobileInfo = await own.locator(".book-cover-image-size").boundingBox();
  expect((await confirm.boundingBox()).y).toBeGreaterThanOrEqual(mobileInfo.y + mobileInfo.height);
  await page.screenshot({ path: "/mnt/data/artifacts/book-cover/completed-size-actions.png", fullPage: false });
  await own.getByRole("button", { name: "기존 표지 책 내 책 표지 확대", exact: true }).click();
  const mobileZoom = page.getByRole("dialog", { name: "기존 표지 책 내 책 표지 확대", exact: true });
  await expect(mobileZoom.getByAltText("기존 표지 책 내 책 확대 표지")).toHaveCSS("width", "800px");
  expect(await mobileZoom.locator(".book-cover-zoom-panel").evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
  await page.screenshot({ path: "/mnt/data/artifacts/book-cover/zoom800-mobile.png", fullPage: false });
  await mobileZoom.getByRole("button", { name: "확대 닫기" }).click();
  await confirm.click();
  await expect(page.getByRole("heading", { name: "기존 표지 책", exact: true })).toHaveCount(0);
  await toggle.click();
  await expect(page.getByText("검사한 EPUB에는 표지 없는 책이 없습니다.")).toBeVisible();
  expect(listings).toEqual([false, true, false]);
});

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
    } else if (endpoint === "/apply") { saved = true; result = { revision: "r2", cover_image: { width: 1, height: 1, byte_size: coverPng.length } }; }
    else if (endpoint === "/preview") return route.fulfill({ contentType: "image/png", body: coverPng });
    else return route.fulfill({ status: 404 });
    return route.fulfill({ json: { status: "success", result } });
  });
  await page.goto("/admin");
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect(page.getByRole("tree")).toBeVisible();
  const directoryWidth = await page.locator(".book-cover-directory").evaluate((node) => node.getBoundingClientRect().width);
  const panelWidth = await page.locator(".book-cover-panel").evaluate((node) => node.getBoundingClientRect().width);
  expect(Math.round(directoryWidth)).toBe(360);
  expect(panelWidth).toBeGreaterThan(directoryWidth * 2);
  const firstHeader = page.locator(".book-cover-row-header").first();
  await expect(firstHeader.getByRole("heading")).toHaveCSS("font-size", "20px");
  await expect(firstHeader.getByRole("button", { name: "편집" })).toBeEnabled();
  await expect(firstHeader.getByRole("button", { name: "조회" })).toBeEnabled();
  const google = firstHeader.getByRole("link", { name: "구글" });
  await expect(google).toHaveAttribute("href", `https://www.google.com/search?tbm=isch&q=${encodeURIComponent("표지 없는 책 0")}`);
  await expect(google).toHaveAttribute("target", "_blank");
  await expect(google).toHaveAttribute("rel", "noopener noreferrer");
  const researchRect = await firstHeader.getByRole("button", { name: "재검색" }).boundingBox();
  const googleRect = await google.boundingBox();
  expect(googleRect.x).toBeGreaterThan(researchRect.x + researchRect.width);
  await expect(page.getByRole("button", { name: "다음 EPUB 검사" })).toHaveCount(0);
  const first = page.getByRole("button", { name: "YES24 서점 후보 0-0 표지 선택" });
  await expect(first).toBeEnabled();
  await expect(first).toHaveAttribute("title", "해상도 1위 / 4개");
  await expect(first).toHaveCSS("background-color", "rgb(0, 255, 0)");
  await expect(first).toHaveCSS("padding", "5px");
  expect(await first.evaluate((node) => node.getBoundingClientRect().width)).toBe(105);
  const ownCover = page.locator(".book-cover-current .cover-grid-cover").first();
  expect(await ownCover.evaluate((node) => node.getBoundingClientRect().width)).toBe(113);
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
  expect(await image.evaluate((node) => ({ width: node.getBoundingClientRect().width, height: node.getBoundingClientRect().height }))).toEqual({ width: 800, height: 1200 });
  await expect(expanded.getByText("원본 크기 · 24 × 36 px")).toBeVisible();
  expect(await expanded.locator(".book-cover-zoom-panel").evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(true);
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
  await expect(ownModal.getByAltText("표지 없는 책 0 내 책 확대 표지")).toHaveCSS("width", "800px");
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
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.locator(".book-cover-current").first().evaluate((node) => node.getBoundingClientRect().width)).toBe(86);
  expect(await page.locator(".book-cover-candidate").first().evaluate((node) => node.getBoundingClientRect().width)).toBe(105);
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
  expect(await trash.evaluate((node) => node.parentElement.className)).toBe("book-cover-current-actions");
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

test("표지 관리: 핸들 드래그로 디렉토리 너비를 조절하고 두 pane이 독립 스크롤된다", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 700 });
  await page.route("**/auth/me", (route) => route.fulfill({ json: { status: "success", result: { role: "admin", email: "test@example.com" } } }));
  await page.route("**/auth/sessions**", (route) => route.fulfill({ json: { status: "success", result: { sessions: [], total: 0 } } }));
  await page.route("**/admin/book-covers/**", (route) => {
    const endpoint = new URL(route.request().url()).pathname.split("/admin/book-covers")[1];
    const result = endpoint === "/directories"
      ? [{ id: "fiction", label: "소설", fileType: "folder", children: [] }]
      : { items: [], scanned: 0, total: 0 };
    return route.fulfill({ json: { status: "success", result } });
  });
  const open = async () => {
    await page.goto("/admin");
    await page.getByRole("tab", { name: "책 표지 관리" }).click();
    await expect(page.getByRole("tree")).toBeVisible();
  };
  await open();
  const left = page.locator(".split-layout-left");
  const handle = page.getByRole("separator");
  await expect(left).toHaveCSS("overflow-y", "auto");
  await expect(page.locator(".split-layout-right")).toHaveCSS("overflow-y", "auto");
  const before = (await left.boundingBox()).width;
  const box = await handle.boundingBox();
  const y = box.y + 40;
  await page.mouse.move(box.x + box.width / 2, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 60, y, { steps: 5 });
  await page.mouse.up();
  const after = (await left.boundingBox()).width;
  expect(after - before).toBeGreaterThan(50);
  await page.reload();
  await page.getByRole("tab", { name: "책 표지 관리" }).click();
  await expect(page.getByRole("tree")).toBeVisible();
  expect(Math.abs((await left.boundingBox()).width - after)).toBeLessThan(2);

  // 터치 경로: 합성 pointer 이벤트(pointerType=touch). 실기기 터치는 검증하지 못한다.
  const b2 = await handle.boundingBox();
  const x = b2.x + b2.width / 2;
  const touch = { pointerType: "touch", pointerId: 7, isPrimary: true, clientY: b2.y + 40 };
  await handle.dispatchEvent("pointerdown", { ...touch, clientX: x });
  await handle.dispatchEvent("pointermove", { ...touch, clientX: x - 40 });
  await handle.dispatchEvent("pointerup", { ...touch, clientX: x - 40 });
  expect((await left.boundingBox()).width).toBeLessThan(after - 30);
});
