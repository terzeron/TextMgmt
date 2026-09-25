/**
 * E2E: 책 보기(/book-view) 내장 EPUB 뷰어 검증.
 *
 * 뷰어 높이는 상수(전체보기 100dvh, 미리보기 60vh)다. 측정·재계산 기계가
 * 없으므로 문서 콘텐츠가 늘어나거나 viewport가 바뀌어도 뷰어가 다시 그려
 * 깜빡이지 않는다. 문서 끝까지 스크롤하면 뷰어가 화면에 정확히 맞는다.
 * 뷰어 iframe은 터치 이벤트를 삼키므로 iframe 안 제스처가 페이지 넘김
 * (가로)과 문서 스크롤(세로)로 연결되어야 한다.
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
  book_id: 123,
  title: "Alice's Adventures in Wonderland",
  file_type: "epub",
  file_path: "alice.epub",
  category: "_epub",
};

async function mockApis(page) {
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
  await page.route("**/categories/_epub**", (route) =>
    route.fulfill({
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
  await page.route("**/categories", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: { _epub: 1 } }),
    }),
  );
  await page.route("**/books/123", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: BOOK }),
    }),
  );
  await page.route("**/similar/123**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: [] }),
    }),
  );
  await page.route("**/preview/123?chapters=0", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/epub+zip",
      body: EPUB_BYTES,
    }),
  );
  await page.route("**/view-history**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success" }),
    }),
  );
}

async function openBookView(page, viewport) {
  await mockApis(page);
  await page.setViewportSize(viewport);
  await page.goto("/book-view/123?category=" + encodeURIComponent("_epub"));
  await page
    .locator(".epub-viewer iframe")
    .waitFor({ state: "visible", timeout: 20_000 });
  await page.waitForTimeout(600);
}

// 뷰어 위에 contentGrowthHeight만큼의 콘텐츠를 추가한다(SimilarBooks 로드 등을 모사)
async function growContentAboveViewer(page, contentGrowthHeight) {
  await page.evaluate((height) => {
    const spacer = document.createElement("div");
    spacer.style.height = `${height}px`;
    document.querySelector("#bottom_panel .card").before(spacer);
  }, contentGrowthHeight);
}

async function getGeometry(page) {
  return page.evaluate(() => {
    const viewer = document.querySelector(".epub-viewer");
    const iframe = document.querySelector(".epub-viewer iframe");
    const meta = document.querySelector("#top_panel");
    const rect = (el) => {
      const r = el?.getBoundingClientRect();
      return r ? { top: r.top, bottom: r.bottom, height: r.height } : null;
    };
    const vv = window.visualViewport;
    return {
      viewportHeight: vv?.height ?? window.innerHeight,
      scrollY: window.scrollY,
      viewer: rect(viewer),
      iframe: rect(iframe),
      meta: rect(meta),
    };
  });
}

test("모바일: 뷰어 높이는 화면 높이와 같고 문서 끝까지 스크롤하면 화면에 정확히 맞는다", async ({
  page,
}) => {
  await openBookView(page, { width: 390, height: 844 });

  const initial = await getGeometry(page);
  expect(initial.viewer.height).toBeCloseTo(initial.viewportHeight, 0);

  // 뷰어를 화면에 맞춰 스크롤하면 뷰어 전체가 화면 안에 들어온다
  await page.evaluate(() => {
    const viewer = document.querySelector(".epub-viewer");
    window.scrollTo(0, viewer.getBoundingClientRect().top + window.scrollY);
  });
  await page.waitForTimeout(400);

  const aligned = await getGeometry(page);
  expect(Math.abs(aligned.viewer.top)).toBeLessThanOrEqual(2);
  expect(aligned.viewer.bottom).toBeLessThanOrEqual(
    aligned.viewportHeight + 1,
  );
  expect(aligned.iframe.bottom).toBeLessThanOrEqual(
    aligned.viewportHeight + 1,
  );
});

test("모바일: 페이지 로드 시 메타 영역이 보이고 스크롤업으로 다시 접근된다", async ({
  page,
}) => {
  await openBookView(page, { width: 390, height: 844 });

  // 로드 시 뷰어에 끌려가지 않고 문서 맨 위(메타 영역)부터 노출된다
  const initial = await getGeometry(page);
  expect(initial.scrollY).toBe(0);
  expect(initial.meta.top).toBeGreaterThanOrEqual(0);
  expect(initial.meta.bottom).toBeLessThanOrEqual(initial.viewportHeight);

  // 뷰어로 스크롤한 뒤 다시 위로 스크롤하면 메타 영역에 접근된다
  await page.evaluate(() =>
    window.scrollTo(0, document.documentElement.scrollHeight),
  );
  await page.waitForTimeout(300);
  await page.evaluate(() => window.scrollBy(0, -400));
  await page.waitForTimeout(300);

  const scrolled = await getGeometry(page);
  expect(scrolled.scrollY).toBe(0);
  expect(scrolled.meta.bottom).toBeGreaterThan(0);
  expect(scrolled.meta.top).toBeLessThanOrEqual(scrolled.viewportHeight);
});

test("모바일: URL bar 토글(화면 높이 변화)에 높이만 따라가고 진동하지 않는다", async ({
  page,
}) => {
  await openBookView(page, { width: 390, height: 844 });

  await page.setViewportSize({ width: 390, height: 700 });
  await page.waitForTimeout(400);
  const first = await getGeometry(page);
  expect(first.viewer.height).toBeCloseTo(700, 0);

  // 뷰어를 화면에 맞춰 스크롤하면 뷰어 전체가 화면 안에 들어온다
  await page.evaluate(() => {
    const viewer = document.querySelector(".epub-viewer");
    window.scrollTo(0, viewer.getBoundingClientRect().top + window.scrollY);
  });
  await page.waitForTimeout(300);
  const aligned = await getGeometry(page);
  expect(aligned.viewer.bottom).toBeLessThanOrEqual(
    aligned.viewportHeight + 1,
  );

  // 재계산 loop가 없으므로 시간이 흘러도 높이가 변하지 않는다
  await page.waitForTimeout(600);
  const second = await getGeometry(page);
  expect(second.viewer.height).toBe(first.viewer.height);
});

test("데스크톱: 위 콘텐츠가 늘어나도 뷰어 높이(100dvh)는 변하지 않는다", async ({
  page,
}) => {
  await openBookView(page, { width: 1280, height: 900 });

  const before = await getGeometry(page);
  expect(before.viewer.height).toBeCloseTo(before.viewportHeight, 0);

  await growContentAboveViewer(page, 300);
  await page.waitForTimeout(600);

  const after = await getGeometry(page);
  // 측정·재계산이 없으므로 뷰어 높이는 그대로다. 늘어난 콘텐츠는 문서
  // 스크롤로만 다뤄진다.
  expect(after.viewer.height).toBeCloseTo(before.viewer.height, 0);
});

test("데스크톱: 스크롤 상태에서 viewport가 커져도 뷰어가 붕괴하지 않는다", async ({
  page,
}) => {
  await openBookView(page, { width: 1280, height: 664 });

  await growContentAboveViewer(page, 700);
  await page.waitForTimeout(300);

  // 사용자가 아래로 스크롤 → 모바일 Chrome은 URL bar를 숨겨 visualViewport가 커진다
  await page.evaluate(() => window.scrollTo(0, 700));
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.waitForTimeout(600);

  const geometry = await getGeometry(page);
  expect(geometry.viewer.height).toBeGreaterThanOrEqual(300);

  // 뷰어를 화면 안으로 스크롤하면 뷰어 전체가 화면 안에 들어온다
  await page.evaluate(() => {
    window.scrollTo(0, document.documentElement.scrollHeight);
  });
  await page.waitForTimeout(300);

  const scrolled = await getGeometry(page);
  expect(scrolled.viewer.bottom).toBeLessThanOrEqual(
    scrolled.viewportHeight + 1,
  );
});

