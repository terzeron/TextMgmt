/**
 * E2E: EPUB 뷰어(좌우 페이지 넘김)의 기본 동작을 실제 epub.js로 검증한다.
 *
 * - 페이지 넘김은 버튼(‹ ›)과 키보드 방향키만이다. 스와이프/터치 팬은
 *   본문 스크롤이 잠겨 아무 동작도 하지 않는다 — 어떤 입력으로 넘겨도
 *   렌더링 결과가 같아야 한다.
 * - 본문이 잘리지 않는다: 글자 크기를 바꾸거나 늦은 이미지/폰트 로드로
 *   본문이 다시 쪼개져도 뷰가 다시 측정된다.
 */
import { readFileSync } from "node:fs";
import { test, expect } from "@playwright/test";

const EPUB_BYTES = readFileSync(
  new URL(
    "../../tests/books/_epub/[Lewis] Alices Adventures in Wonderland.epub",
    import.meta.url,
  ),
);

const BOOK = {
  book_id: 200885711,
  title: "Alice's Adventures in Wonderland",
  file_type: "epub",
  file_path: "test.epub",
  category: "_epub",
};

async function mockApis(page) {
  await page.route("**/auth/me", (r) =>
    r.fulfill({
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
  await page.route("**/auth/refresh", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success" }),
    }),
  );
  await page.route("**/categories/_epub**", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "success",
        result: [BOOK],
        next_cursor: "",
        total: 1,
      }),
    }),
  );
  await page.route("**/categories", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: { _epub: 1 } }),
    }),
  );
  await page.route("**/books/200885711", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: BOOK }),
    }),
  );
  await page.route("**/similar/200885711**", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: [] }),
    }),
  );
  await page.route("**/preview/200885711?chapters=0", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/epub+zip",
      body: EPUB_BYTES,
    }),
  );
  await page.route("**/view-history**", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success" }),
    }),
  );
  await page.route("**/logs/**", (r) =>
    r.fulfill({ status: 200, contentType: "application/json", body: "{}" }),
  );
}

async function openEmbedded(page, viewport = { width: 900, height: 800 }) {
  await mockApis(page);
  await page.setViewportSize(viewport);
  await page.goto("/book-view/200885711?category=" + encodeURIComponent("_epub"));
  await page
    .locator(".epub-viewer iframe")
    .waitFor({ state: "visible", timeout: 20_000 });
  // 텍스트가 동반되는 섹션이 나올 때까지 넘긴다 — 첫 섹션은 표지(이미지만)일
  // 수 있으므로 곧바로 텍스트를 기대하면 안 된다.
  const nextBtn = page.locator(".epub-page-next");
  for (let i = 0; i < 12; i++) {
    const len = await page.evaluate(() => {
      const d = document.querySelector(".epub-viewer iframe")?.contentDocument;
      return d && (d.body?.innerText || "").trim().length;
    });
    if (len && len > 0) return;
    await nextBtn.click();
    await page.waitForTimeout(350);
  }
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const d = document.querySelector(".epub-viewer iframe")?.contentDocument;
        return d && (d.body?.innerText || "").trim().length;
      });
    })
    .toBeGreaterThan(0);
}

function state(page) {
  return page.evaluate(() => {
    const c = document.querySelector(".epub-viewer .epub-container");
    const iframe = document.querySelector(".epub-viewer iframe");
    const doc = iframe?.contentDocument;
    return {
      scrollLeft: Math.round(c?.scrollLeft ?? -1),
      scrollWidth: c?.scrollWidth ?? -1,
      clientWidth: c?.clientWidth ?? -1,
      iframeW: Math.round(iframe?.getBoundingClientRect().width ?? -1),
      bodyScrollW: doc?.body?.scrollWidth ?? -1,
      text: (doc?.body?.innerText || "").trim().slice(0, 80),
      pageInfo: document.querySelector(".epub-page-info")?.textContent,
    };
  });
}

// ──────────────────────────────────────────────────────────────────────────
// 기본 동작
// ──────────────────────────────────────────────────────────────────────────

test("책이 열리고 본문 텍스트가 보인다", async ({ page }) => {
  await openEmbedded(page);
  const st = await state(page);
  expect(st.text.length).toBeGreaterThan(0);
});

test("다음 페이지 버튼으로 본문이 오른쪽으로 간다", async ({ page }) => {
  await openEmbedded(page);
  const nextBtn = page.locator(".epub-page-next");
  const before = await state(page);

  await nextBtn.click();
  await page.waitForTimeout(400);

  const after = await state(page);
  const moved = after.scrollLeft > before.scrollLeft || after.text !== before.text;
  expect(moved).toBe(true);
});

test("이전 페이지 버튼으로 본문이 왼쪽으로 돌아간다", async ({ page }) => {
  await openEmbedded(page);
  const nextBtn = page.locator(".epub-page-next");
  const prevBtn = page.locator(".epub-page-prev");

  await nextBtn.click();
  await page.waitForTimeout(400);
  const mid = await state(page);

  await prevBtn.click();
  await page.waitForTimeout(400);
  const back = await state(page);
  const returned =
    back.scrollLeft < mid.scrollLeft || back.text !== mid.text;
  expect(returned).toBe(true);
});

test("키보드 방향키로 넘긴다", async ({ page }) => {
  await openEmbedded(page);
  const before = await state(page);
  await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(400);
  const after = await state(page);
  expect(
    after.scrollLeft > before.scrollLeft || after.text !== before.text,
  ).toBe(true);
});

test("스와이프(터치 팬)로는 넘어가지 않는다 — 어떤 입력이든 같은 결과", async ({
  page,
}) => {
  await openEmbedded(page);
  const before = await state(page);

  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: 300, y: 400 }],
  });
  for (let i = 1; i <= 8; i++) {
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x: 300 - i * 25, y: 400 }],
    });
    await page.waitForTimeout(30);
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await page.waitForTimeout(600);

  const after = await state(page);
  // 본문 스크롤이 잠겨 터치 팬으로는 아무것도 이동하지 않는다
  expect(after.scrollLeft).toBe(before.scrollLeft);
  expect(after.text).toBe(before.text);
});

test("글자 크기를 키워도 본문이 잘리지 않는다", async ({ page }) => {
  await openEmbedded(page);

  // 긴 섹션(여러 컬럼)으로 이동
  const nextBtn = page.locator(".epub-page-next");
  for (let i = 0; i < 12; i++) {
    const st = await state(page);
    if (st.scrollWidth > st.clientWidth * 2 && st.text.length > 40) break;
    await nextBtn.click();
    await page.waitForTimeout(300);
  }
  const before = await state(page);
  expect(before.scrollWidth).toBeGreaterThan(before.clientWidth * 2);

  // A+ → 본문이 다시 쪼개진다 → 뷰가 다시 측정되어 잘림이 없어야 한다
  await page.click("[aria-label='글자 크기 늘리기']");
  await expect
    .poll(async () => {
      const st = await state(page);
      return st.bodyScrollW - st.iframeW;
    })
    .toBeLessThanOrEqual(2);

  // 마지막 컬럼까지 도달 가능하다
  await page.evaluate(() => {
    const c = document.querySelector(".epub-viewer .epub-container");
    c.scrollLeft = c.scrollWidth;
  });
  await page.waitForTimeout(300);
  const tail = await state(page);
  expect(tail.text.length).toBeGreaterThan(0);
});

test("standalone(전체보기)에서도 본문이 보인다", async ({ page }) => {
  await mockApis(page);
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto(
    "/viewer/epub/200885711?path=" +
      encodeURIComponent("test.epub") +
      "&category=" +
      encodeURIComponent("_epub"),
  );
  await page
    .locator(".epub-viewer iframe")
    .waitFor({ state: "visible", timeout: 20_000 });
  await page.waitForTimeout(500);

  const nextBtn = page.locator(".epub-page-next");
  for (let i = 0; i < 10; i++) {
    const st = await state(page);
    if (st.text.length > 0) break;
    await nextBtn.click();
    await page.waitForTimeout(300);
  }
  const st = await state(page);
  expect(st.text.length).toBeGreaterThan(0);
});

// ──────────────────────────────────────────────────────────────────────────
// 읽기 위치 저장/복원
// ──────────────────────────────────────────────────────────────────────────

test("저장된 위치를 복원한다", async ({ page }) => {
  await openEmbedded(page);

  // 여러 페이지 넘긴 뒤 저장을 기다린다
  const nextBtn = page.locator(".epub-page-next");
  for (let i = 0; i < 6; i++) {
    await nextBtn.click();
    await page.waitForTimeout(300);
  }
  const before = await state(page);
  expect(before.pageInfo).not.toBe("페이지 계산 중...");

  // 새로고침 → 저장된 위치로 복원
  await page.reload();
  await page
    .locator(".epub-viewer iframe")
    .waitFor({ state: "visible", timeout: 20_000 });
  await expect
    .poll(async () => {
      const st = await state(page);
      return st.pageInfo;
    })
    .not.toBe("페이지 계산 중...");

  const restored = await state(page);
  expect(restored.pageInfo).toBe(before.pageInfo);
});

// ──────────────────────────────────────────────────────────────────────────
// 메타 영역과 공존 — 뷰어 위의 메타 영역으로 스크롤업이 된다
// ──────────────────────────────────────────────────────────────────────────

test("메타 영역은 위에 있고 스크롤업으로 다시 접근된다", async ({ page }) => {
  await openEmbedded(page, { width: 1280, height: 800 });

  // 로드 시 메타 영역이 보인다
  const metaTop = await page.evaluate(
    () => document.querySelector("#top_panel")?.getBoundingClientRect().top,
  );
  expect(metaTop).toBeGreaterThanOrEqual(0);

  // 문서 끝까지 갔다가 다시 위로 스크롤하면 메타에 접근된다
  await page.evaluate(() => {
    window.scrollTo(0, document.documentElement.scrollHeight);
  });
  await page.waitForTimeout(300);
  await page.evaluate(() => window.scrollBy(0, -500));
  await page.waitForTimeout(300);

  const meta = await page.evaluate(
    () => document.querySelector("#top_panel")?.getBoundingClientRect().top,
  );
  expect(meta).toBeGreaterThanOrEqual(0);
});
