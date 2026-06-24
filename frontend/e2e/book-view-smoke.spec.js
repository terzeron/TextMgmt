/**
 * E2E 스모크 테스트: book-view 폴더 트리 렌더링
 *
 * 회귀 방지 대상:
 *   rolldown의 __toESM(module, 1) 버그 — @mui/icons-material CJS 경로 import 시
 *   프로덕션 번들에서 아이콘이 plain object로 노출되어 React Error #130 발생.
 *   아이콘 import를 CJS 경로로 되돌리면 이 테스트가 실패한다.
 *
 * 조건:
 *   - 프로덕션 빌드(dist/)가 필요하다. webServer 설정이 자동으로 빌드 후 preview 서버를 시작한다.
 *   - API 호출은 page.route()로 전부 모킹한다 (실 백엔드 불필요).
 */
import { test, expect } from "./coverage-fixture.js";

const MOCK_CATEGORIES = {
  fiction: 4,
  science: 2,
  comics: 1,
};

test("book-view: 폴더 트리 렌더링 시 React Error #130 없음", async ({
  page,
}) => {
  // ── 인증 모킹 ──────────────────────────────────────────────────────────
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

  // viewer 역할 전용 엔드포인트
  await page.route("**/hidden-categories**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: [] }),
    }),
  );

  // ── 카테고리 트리 모킹 ───────────────────────────────────────────────
  // /categories        → 카테고리별 파일 수 맵  { categoryName: count }
  // /categories/_root  → 최상위 파일 목록 (빈 배열로 단순화)
  // /categories/{name} → 해당 카테고리 파일 목록 (빈 배열로 단순화)
  await page.route("**/categories**", (route) => {
    const url = route.request().url();

    if (url.includes("/categories/_root")) {
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "success", result: [] }),
      });
    }

    if (/\/categories\/[^/]+$/.test(url)) {
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "success", result: [] }),
      });
    }

    // 루트 카테고리 목록: jsonGetReq → processData가 data.result를 resolve에 전달
    // View.jsx: categoryCounts = { "_epub": 5, "fiction": 3, ... }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: MOCK_CATEGORIES }),
    });
  });

  // ── React 오류 수집 ────────────────────────────────────────────────────
  const reactErrors = [];

  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    if (
      text.includes("Element type is invalid") ||
      text.includes("Minified React error #130") ||
      text.includes("React error #130")
    ) {
      reactErrors.push(text);
    }
  });

  page.on("pageerror", (err) => {
    if (
      err.message.includes("Element type is invalid") ||
      err.message.includes("Minified React error")
    ) {
      reactErrors.push(err.message);
    }
  });

  // ── 페이지 이동 및 렌더링 대기 ────────────────────────────────────────
  await page.goto("/book-view");

  // 폴더 트리([role="tree"])가 나타날 때까지 최대 10초 대기
  await expect(page.getByRole("tree")).toBeVisible({ timeout: 10_000 });

  // React 렌더링이 완료될 때까지 잠시 대기
  await page.waitForTimeout(500);

  // ── 검증 ──────────────────────────────────────────────────────────────
  expect(
    reactErrors,
    "React Error #130 감지됨: @mui/icons-material import에 esm/ 경로 사용 확인 필요",
  ).toHaveLength(0);

  // 트리 내 카테고리 항목이 실제로 렌더링됐는지 확인
  await expect(page.getByText("fiction")).toBeVisible();
  await expect(page.getByText("science")).toBeVisible();
});

test("comics-view: 폴더 트리 렌더링 시 React Error #130 없음", async ({
  page,
}) => {
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

  await page.route("**/hidden-categories**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "success", result: [] }),
    }),
  );

  await page.route("**/categories**", (route) => {
    const url = route.request().url();
    if (url.includes("/categories/_root") || /\/categories\/[^/]+$/.test(url)) {
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "success", result: [] }),
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "success",
        result: { action: 2, romance: 5 },
      }),
    });
  });

  const reactErrors = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    if (
      text.includes("Element type is invalid") ||
      text.includes("Minified React error #130")
    ) {
      reactErrors.push(text);
    }
  });
  page.on("pageerror", (err) => {
    if (
      err.message.includes("Element type is invalid") ||
      err.message.includes("Minified React error")
    ) {
      reactErrors.push(err.message);
    }
  });

  await page.goto("/comics-view");
  await expect(page.getByRole("tree")).toBeVisible({ timeout: 10_000 });
  await page.waitForTimeout(500);

  expect(
    reactErrors,
    "React Error #130 감지됨: @mui/icons-material import에 esm/ 경로 사용 확인 필요",
  ).toHaveLength(0);

  await expect(page.getByText("action")).toBeVisible();
  await expect(page.getByText("romance")).toBeVisible();
});
