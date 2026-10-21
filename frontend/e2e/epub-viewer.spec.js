/**
 * E2E: EPUB 뷰어(좌우 페이지 넘김)의 기본 동작을 실제 epub.js로 검증한다.
 *
 * - 버튼(‹ ›), 키보드 방향키, 가로 스와이프의 페이지 이동 결과가 같다.
 *   네이티브 가로 팬은 차단해 본문 컬럼의 스크롤이 어긋나지 않게 한다.
 * - 본문이 잘리지 않는다: 글자 크기를 바꾸거나 늦은 이미지/폰트 로드로
 *   본문이 다시 쪼개져도 뷰가 다시 측정된다.
 */
import { readFileSync } from "node:fs";
import JSZip from "jszip";
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

async function mockApis(page, epubBytes = EPUB_BYTES) {
  await page.route("https://accounts.google.com/**", (r) => r.abort());
  await page.route("**/category-mappings**", (r) =>
    r.fulfill({ json: { status: "success", result: {} } }),
  );
  await page.route("**/category-pdf-stats?**", (r) =>
    r.fulfill({ json: { status: "success", result: {} } }),
  );
  await page.route("**/search/bookstore/**", (r) =>
    r.fulfill({ json: { status: "success", result: [] } }),
  );
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
  await page.route("**/preview/200885711?chapters=*", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/epub+zip",
      body: epubBytes,
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

async function openEmbedded(page, viewport = { width: 900, height: 800 }, route = "/book-view/200885711?category=_epub") {
  await mockApis(page);
  await page.setViewportSize(viewport);
  await page.goto(route, { waitUntil: "domcontentloaded" });
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

test("첫 화면에 EPUB 표지를 표시하고 다음 페이지에서 본문으로 간다", async ({ page }) => {
  await mockApis(page);
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto("/book-view/200885711?category=" + encodeURIComponent("_epub"));

  const cover = page.locator(".epub-cover-page img");
  await expect(cover).toBeVisible();
  await expect(cover).toHaveAttribute("alt", "책 표지");
  await expect
    .poll(() => cover.evaluate((image) => image.naturalWidth))
    .toBeGreaterThan(0);

  await page.locator(".epub-page-next").click();
  await expect(page.locator(".epub-cover-page")).toHaveCount(0);
});

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

async function swipe(page, dx, dy = 0) {
  await page.getByTestId("epub-host").scrollIntoViewIfNeeded();
  const box = await page.getByTestId("epub-host").boundingBox();
  const x = box.x + box.width / 2 - dx / 2;
  const y = box.y + box.height / 2 - dy / 2;
  // Playwright WebKit은 네이티브 touchMove를 제공하지 않는다.
  // 실제 본문 DOM에 이벤트를 보내 바인딩과 기본 동작 취소를 검증한다.
  if (page.context().browser().browserType().name() === "webkit") {
    const canceled = await page.evaluate(({ x, y, dx, dy }) => {
      const iframe = document.querySelector(".epub-viewer iframe");
      const rect = iframe.getBoundingClientRect();
      const doc = iframe.contentDocument;
      const target = doc.elementFromPoint(x - rect.x, y - rect.y);
      const emit = (type, progress) => {
        const touch = { identifier: 1, clientX: x + dx * progress, clientY: y + dy * progress };
        const event = new Event(type, { bubbles: true, cancelable: true });
        Object.defineProperties(event, {
          touches: { value: type === "touchend" ? [] : [touch] },
          changedTouches: { value: [touch] },
        });
        target.dispatchEvent(event);
        return event.defaultPrevented;
      };
      emit("touchstart", 0);
      const moveCanceled = emit("touchmove", 1);
      emit("touchend", 1);
      return moveCanceled;
    }, { x, y, dx, dy });
    expect(canceled).toBe(Math.abs(dx) > 8 && Math.abs(dx) > Math.abs(dy) * 1.5);
    await page.waitForTimeout(600);
    return;
  }
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x, y }],
  });
  for (let i = 1; i <= 8; i++) {
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x: x + dx * i / 8, y: y + dy * i / 8 }],
    });
    await page.waitForTimeout(30);
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await cdp.detach();
  await page.waitForTimeout(600);
}

test.describe("모바일 터치", () => {
  test.use({ isMobile: true, hasTouch: true });
  test("본문의 터치는 받으면서 책의 스크립트와 CSP 우회는 차단한다", async ({ page }) => {
    const zip = await JSZip.loadAsync(EPUB_BYTES);
    for (const entry of Object.values(zip.files).filter((entry) => /\.x?html$/.test(entry.name))) {
      const html = await entry.async("string");
      zip.file(entry.name, html.replace(/<head[^>]*>/, '$&<script>parent.epubScriptRan = true;</script><meta http-equiv="refresh" content="0;url=https://epub-test.invalid/escape" />')
        .replace(/<body([^>]*)>/, '<body$1 onload="parent.epubScriptRan = true">'));
    }
    await mockApis(page, await zip.generateAsync({ type: "nodebuffer" }));
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/viewer/epub/200885711?path=alice.epub", { waitUntil: "domcontentloaded" });
    await expect(page.locator(".epub-viewer iframe")).toBeVisible();
    await page.evaluate(() => {
      const doc = document.querySelector(".epub-viewer iframe").contentDocument;
      if (doc.querySelector("script, [onload], meta[http-equiv='refresh']")) throw new Error("정제되지 않은 본문");
      window.epubTouchCount = 0;
      window.epubCspViolations = [];
      doc.addEventListener("touchstart", () => window.epubTouchCount++);
      doc.addEventListener("securitypolicyviolation", (event) => window.epubCspViolations.push(event.violatedDirective));
      // 정제 이후 실행 코드를 넣어도 CSP가 차단해야 한다.
      const script = doc.createElement("script");
      script.textContent = "parent.epubScriptRan = true";
      doc.head.appendChild(script);
      const nestedFrame = doc.createElement("iframe");
      nestedFrame.style.display = "none";
      nestedFrame.src = URL.createObjectURL(new Blob(["<script>parent.parent.epubScriptRan = true</script>"], { type: "text/html" }));
      doc.body.appendChild(nestedFrame);
    });
    await expect.poll(() => page.evaluate(() => window.epubCspViolations.some((value) => value.startsWith("script-src")))).toBe(true);
    await expect.poll(() => page.evaluate(() => window.epubCspViolations.includes("frame-src"))).toBe(true);
    const box = await page.getByTestId("epub-host").boundingBox();
    await page.touchscreen.tap(box.x + box.width / 2, box.y + box.height / 2);
    await expect.poll(() => page.evaluate(() => window.epubTouchCount)).toBe(1);
    expect(await page.evaluate(() => window.epubScriptRan)).toBeUndefined();
  });

  for (const { name, viewport, route } of [
    { name: "미리보기 태블릿", viewport: { width: 900, height: 800 }, route: "/book-view/200885711?category=_epub" },
    { name: "미리보기 휴대전화", viewport: { width: 390, height: 844 }, route: "/book-view/200885711?category=_epub" },
    { name: "편집 휴대전화", viewport: { width: 390, height: 844 }, route: "/book-edit/200885711?category=_epub" },
    { name: "전체보기 휴대전화", viewport: { width: 390, height: 844 }, route: "/viewer/epub/200885711?path=alice.epub" },
  ]) {
    test(`가로 스와이프는 버튼과 같은 한 쪽을 넘긴다 (${name})`, async ({ page }, testInfo) => {
      const errors = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await openEmbedded(page, viewport, route);
      if (await page.getByTestId("epub-cover-page").count()) {
        await page.locator(".epub-page-next").click();
      }
      // 앞표지·목차를 지나 여러 쪽짜리 본문에서 같은 한 쪽 이동을 비교한다.
      for (let i = 0; i < 12; i++) {
        const current = await state(page);
        if (current.scrollWidth > current.clientWidth * 3 && current.text.length > 40) break;
        await page.locator(".epub-page-next").click();
        await page.waitForTimeout(350);
      }
      await expect.poll(async () => {
        const current = await state(page);
        return current.scrollWidth > current.clientWidth * 3 && current.text.length > 40;
      }).toBe(true);
      const before = await state(page);
      await page.locator(".epub-page-next").click();
      await expect.poll(async () => {
        const current = await state(page);
        return current.scrollLeft !== before.scrollLeft || current.text !== before.text;
      }).toBe(true);
      const next = await state(page);
      await page.locator(".epub-page-prev").click();
      const previousPosition = { scrollLeft: before.scrollLeft, text: before.text };
      const nextPosition = { scrollLeft: next.scrollLeft, text: next.text };
      await expect.poll(() => state(page)).toMatchObject(previousPosition);

      await swipe(page, -160);
      await expect.poll(() => state(page)).toMatchObject(nextPosition);
      await swipe(page, 160);
      await expect.poll(() => state(page)).toMatchObject(previousPosition);
      await swipe(page, 0, 160);
      expect(await state(page)).toMatchObject(previousPosition);
      await swipe(page, -15);
      expect(await state(page)).toMatchObject(previousPosition);
      expect(errors).toEqual([]);
      const screenshot = testInfo.outputPath("epub-swipe.png");
      await page.screenshot({ path: screenshot });
      await testInfo.attach("epub-swipe", { path: screenshot, contentType: "image/png" });
    });
  }

  test("전체보기 표지에서도 스와이프로 본문에 진입한다", async ({ page }) => {
    await mockApis(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/viewer/epub/200885711?path=alice.epub");
    await expect(page.getByTestId("epub-cover-page")).toBeVisible();
    await swipe(page, 160);
    await expect(page.getByTestId("epub-cover-page")).toBeVisible();
    await swipe(page, -160);
    await expect(page.getByTestId("epub-cover-page")).toHaveCount(0);
    await expect(page.locator(".epub-viewer iframe")).toBeVisible();
  });
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
