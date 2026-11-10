/**
 * E2E 테스트: 세로로 아주 긴 쪽(웹툰 스트립)을 타일로 나눠 화면 해상도로 그리는지 검증
 *
 * 배경:
 *   쪽당 canvas 면적 상한(3M px) 때문에 690x16383px 웹툰 쪽이 원본의 약 51% 해상도로 그려졌다.
 *   긴 쪽은 세로 타일 여러 장으로 나눠, 타일마다 상한을 지키면서 쪽 전체를 화면 폭 x dpr 로 그린다.
 *
 * 신호:
 *   1) 긴 쪽 자리에 타일 컨테이너(.pdf-page-tiles)와 타일 canvas(.pdf-tile)가 생긴다.
 *   2) 그려진 타일의 비트맵 폭 = 표시 폭 x dpr (해상도 손실 없음).
 *   3) 타일 하나의 면적은 3M px 이하이고, 타일 높이 합이 쪽 전체 높이와 같다.
 *   4) 타일 경계 위아래 행이 줄무늬 패턴대로 이어진다(틈·겹침 없음).
 */
import { test, expect } from "@playwright/test";

// 120화 1쪽과 같은 쪽 크기(pt)
const PAGE_W = 517.5;
const PAGE_H = 12287.25;
const MAX_TILE_PIXELS = 3_000_000;

// 한 쪽짜리 PDF. 10pt 마다 검은 띠(5pt)를 그어, 어느 행이든 검정·흰색 여부를 계산할 수 있게 한다.
function buildTallPdf() {
  const bands = [];
  for (let y = 0; y < PAGE_H; y += 10) bands.push(`0 ${y} ${PAGE_W} 5 re`);
  const stream = `q 0 0 0 rg ${bands.join(" ")} f Q`;
  const parts = [];
  const offsets = {};
  let cursor = 0;
  const push = (s) => {
    parts.push(s);
    cursor += Buffer.byteLength(s, "latin1");
  };
  const addObj = (num, body) => {
    offsets[num] = cursor;
    push(`${num} 0 obj\n${body}\nendobj\n`);
  };
  push("%PDF-1.4\n");
  addObj(1, "<< /Type /Catalog /Pages 2 0 R >>");
  addObj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
  addObj(
    3,
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${PAGE_W} ${PAGE_H}] /Contents 4 0 R /Resources << >> >>`,
  );
  addObj(4, `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`);
  const xrefOffset = cursor;
  let xref = "xref\n0 5\n0000000000 65535 f \n";
  for (let n = 1; n < 5; n++)
    xref += `${String(offsets[n]).padStart(10, "0")} 00000 n \n`;
  xref += `trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  push(xref);
  return Buffer.from(parts.join(""), "latin1");
}

test.use({ deviceScaleFactor: 2, viewport: { width: 760, height: 900 } });

test("긴 쪽은 타일로 나뉘어 표시 폭 x dpr 해상도로 그려진다", async ({
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
  const pdf = buildTallPdf();
  await page.route("**/pdf-pages/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/pdf",
      headers: {
        "X-Total-Pages": "1",
        "Access-Control-Allow-Origin":
          route.request().headers()["origin"] || "*",
        "Access-Control-Allow-Credentials": "true",
        "Access-Control-Expose-Headers": "X-Total-Pages",
      },
      body: pdf,
    }),
  );
  const errors = [];
  page.on("pageerror", (err) => errors.push(err.message));
  page.on("console", (msg) => {
    if (msg.type() === "error" && msg.text().includes("렌더링 실패"))
      errors.push(msg.text());
  });

  await page.goto("/viewer/pdf/12345?path=" + encodeURIComponent("tall.pdf"));

  const container = page.locator(".pdf-page-tiles");
  await expect(container).toBeVisible({ timeout: 20_000 });
  const tiles = page.locator("canvas.pdf-tile");
  const count = await tiles.count();
  expect(count).toBeGreaterThan(1);

  // 맨 위 타일은 화면 안에 있으므로 그려진다
  await expect
    .poll(() => tiles.first().evaluate((c) => c.width))
    .toBeGreaterThan(1);

  const info = await page.evaluate(() => {
    const box = document.querySelector(".pdf-page-tiles");
    const list = [...document.querySelectorAll("canvas.pdf-tile")];
    return {
      cssWidth: box.clientWidth,
      dpr: window.devicePixelRatio,
      first: { width: list[0].width, height: list[0].height },
    };
  });
  // 2) 해상도: 비트맵 폭 = 표시 폭 x dpr (반올림 1px 허용)
  expect(
    Math.abs(info.first.width - info.cssWidth * info.dpr),
  ).toBeLessThanOrEqual(1);
  // 3) 타일 하나의 면적 상한
  expect(info.first.width * info.first.height).toBeLessThanOrEqual(
    MAX_TILE_PIXELS,
  );

  // 첫 경계를 화면 가운데로 스크롤해 둘째 타일도 그리게 한다
  await page.locator(".pdf-container").evaluate((el) => {
    const second = document.querySelectorAll("canvas.pdf-tile")[1];
    el.scrollTop =
      second.getBoundingClientRect().top -
      el.getBoundingClientRect().top +
      el.scrollTop -
      300;
  });
  await expect
    .poll(() => tiles.nth(1).evaluate((c) => c.width))
    .toBeGreaterThan(1);

  // 4) 경계 연속성: 쪽 좌표로 환산한 각 행의 줄무늬 색이 기대와 같다(경계 바로 위 3행, 아래 3행).
  const mismatches = await page.evaluate(
    ({ PAGE_H }) => {
      const list = [...document.querySelectorAll("canvas.pdf-tile")];
      const [a, b] = list;
      const scale = a.width / 517.5;
      const fullH = Math.round(PAGE_H * scale);
      const darkAt = (row) => {
        // PDF 좌표는 아래에서 위로 올라간다. 행 중앙의 쪽 좌표 y
        const y = PAGE_H - (row + 0.5) / scale;
        return y % 10 < 5;
      };
      const sample = (canvas, localRow) => {
        const px = canvas
          .getContext("2d")
          .getImageData(10, localRow, 1, 1).data;
        return px[3] > 128 && px[0] < 128;
      };
      const out = [];
      for (let k = 1; k <= 3; k++) {
        const rowA = a.height - k;
        const rowB = k - 1;
        // 띠 가장자리에 걸친 행은 안티앨리어싱으로 회색이라 판정에서 뺀다
        for (const [canvas, local, global] of [
          [a, rowA, rowA],
          [b, rowB, a.height + rowB],
        ]) {
          const y = PAGE_H - (global + 0.5) / scale;
          const edge = Math.min(y % 5, 5 - (y % 5)) * scale < 1;
          if (!edge && sample(canvas, local) !== darkAt(global))
            out.push(global);
        }
      }
      return { out, fullH };
    },
    { PAGE_H },
  );
  expect(mismatches.out).toEqual([]);

  // 3) 타일 높이 합 = 쪽 전체 높이
  const total = await page.evaluate(() => {
    const box = document.querySelector(".pdf-page-tiles");
    const [w, h] = box.style.aspectRatio
      .split("/")
      .map((v) => Number(v.trim()));
    const sum = [...document.querySelectorAll("canvas.pdf-tile")].reduce(
      (acc, c) => acc + parseFloat(c.style.height),
      0,
    );
    return { w, h, sum };
  });
  // 브라우저가 % 값을 유효숫자 몇 자리로 직렬화하므로 0.001% (32766px 쪽에서 0.3px) 안이면 같다.
  expect(total.sum).toBeCloseTo(100, 3);
  expect(errors).toEqual([]);
});
