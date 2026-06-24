/**
 * E2E 테스트: PDF 전체 보기 페이지의 실제 렌더링 검증
 *
 * 검증 대상:
 *   /viewer/pdf/:entryId (standalone, preview=false) 전체 보기에서
 *   pdf.js가 PDF 바이트를 canvas에 실제로 래스터하는지 확인한다.
 *
 * 신호:
 *   1) 모든 페이지의 page.render().promise 성공 → 툴바에 "총 N쪽 표시" 표시
 *      (렌더 중에는 "렌더링 중... X/N쪽")
 *   2) canvas.getImageData로 실제로 그려진(불투명) 픽셀 존재 확인
 *      → 크기만 잡힌 빈 placeholder가 아니라 실제 래스터가 일어났음을 증명
 *
 * 조건:
 *   - 프로덕션 빌드(dist/) 필요. webServer 설정이 자동 빌드 후 preview 서버 시작.
 *   - /pdf-pages 청크 요청은 page.route()로 실제 유효한 PDF 바이트를 응답한다
 *     (실 백엔드 불필요). 각 페이지에 검은 사각형을 그려 캔버스에 픽셀이 남게 한다.
 */
import { test, expect } from "./coverage-fixture.js";

const TOTAL_PAGES = 3;

/**
 * numPages 페이지짜리 최소 유효 PDF(1.4)를 생성한다.
 * 각 페이지는 200×200 MediaBox 안에 검은 사각형을 채워, 렌더 시 canvas에
 * 불투명 픽셀이 남도록 한다. xref 오프셋을 정확히 계산해 pdf.js가 파싱 가능.
 */
function buildPdf(numPages) {
  const pageDefs = [];
  let objNum = 3; // 1=Catalog, 2=Pages
  for (let i = 0; i < numPages; i++) {
    pageDefs.push({ pageNo: objNum++, contentNo: objNum++ });
  }

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
  const kids = pageDefs.map((p) => `${p.pageNo} 0 R`).join(" ");
  addObj(2, `<< /Type /Pages /Kids [${kids}] /Count ${numPages} >>`);

  for (const { pageNo, contentNo } of pageDefs) {
    addObj(
      pageNo,
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] ` +
        `/Contents ${contentNo} 0 R /Resources << >> >>`,
    );
    const stream = "q 0 0 0 rg 50 50 100 100 re f Q";
    addObj(
      contentNo,
      `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    );
  }

  const xrefOffset = cursor;
  let xref = `xref\n0 ${objNum}\n0000000000 65535 f \n`;
  for (let n = 1; n < objNum; n++) {
    xref += `${String(offsets[n]).padStart(10, "0")} 00000 n \n`;
  }
  xref += `trailer\n<< /Size ${objNum} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  push(xref);

  return Buffer.from(parts.join(""), "latin1");
}

test("PDF 전체 보기: pdf.js가 모든 페이지를 canvas에 실제로 렌더링한다", async ({
  page,
}) => {
  // ── 인증 부트스트랩 모킹 (standalone viewer 안전장치) ──
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

  // ── PDF 청크 모킹: start~end 페이지 수만큼의 유효 PDF를 응답 ──
  // 전체 보기는 1-1 청크(첫 페이지) → 2-N 청크 순으로 요청한다.
  // /pdf-pages는 cross-origin(빌드 시 VITE_API_URL_PREFIX)으로 호출되므로,
  // X-Total-Pages를 JS에서 읽으려면 Access-Control-Expose-Headers가 필요하다.
  const chunkRanges = [];
  await page.route("**/pdf-pages/**", (route) => {
    const url = new URL(route.request().url());
    const start = parseInt(url.searchParams.get("start") || "1", 10);
    const end = parseInt(url.searchParams.get("end") || "1", 10);
    chunkRanges.push(`${start}-${end}`);
    const count = Math.max(1, end - start + 1);
    route.fulfill({
      status: 200,
      contentType: "application/pdf",
      headers: {
        "X-Total-Pages": String(TOTAL_PAGES),
        "Access-Control-Allow-Origin":
          route.request().headers()["origin"] || "*",
        "Access-Control-Allow-Credentials": "true",
        "Access-Control-Expose-Headers": "X-Total-Pages",
      },
      body: buildPdf(count),
    });
  });

  // ── 렌더 실패 콘솔 에러 수집 ──
  const renderErrors = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" && msg.text().includes("렌더링 실패")) {
      renderErrors.push(msg.text());
    }
  });
  page.on("pageerror", (err) => renderErrors.push(err.message));

  // ── 전체 보기 페이지 이동 (standalone, preview=false) ──
  // path 쿼리가 있어야 bookId가 설정된다. category는 생략 → 이전/다음 fetch 스킵.
  await page.goto("/viewer/pdf/12345?path=" + encodeURIComponent("test.pdf"));

  // ── 1) 첫 페이지 렌더 확인: 툴바가 나타나고 canvas placeholder가 생성된다 ──
  await expect(page.locator(".pdf-info")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator("canvas.pdf-page")).toHaveCount(TOTAL_PAGES);

  // ── 2) 전 페이지 렌더 유발: 컨테이너를 끝까지 스크롤 (off-screen 페이지는
  //       IntersectionObserver로 lazy 렌더되므로, 스크롤로 스케줄러를 동작시킨다) ──
  const container = page.locator(".pdf-container");
  for (let i = 0; i < 6; i++) {
    await container.evaluate((el) => {
      el.scrollTop = Math.min(el.scrollTop + el.clientHeight, el.scrollHeight);
    });
    await page.waitForTimeout(300);
  }

  // 모든 페이지의 render().promise가 성공하면 "총 N쪽 표시"로 전환된다
  await expect(page.locator(".pdf-info")).toHaveText(
    `총 ${TOTAL_PAGES}쪽 표시`,
    {
      timeout: 20_000,
    },
  );

  // ── 3) 실제 래스터 검증: 모든 canvas에 그려진 불투명 픽셀이 존재 ──
  //       (크기만 잡힌 빈 placeholder가 아니라 실제 pdf.js 래스터가 일어났음) ──
  const opaqueCounts = await page
    .locator("canvas.pdf-page")
    .evaluateAll((els) =>
      els.map((canvas) => {
        const { width, height } = canvas;
        if (!width || !height) return 0;
        const data = canvas
          .getContext("2d")
          .getImageData(0, 0, width, height).data;
        let opaque = 0;
        for (let i = 3; i < data.length; i += 4) {
          if (data[i] > 0) opaque++;
        }
        return opaque;
      }),
    );

  expect(opaqueCounts).toHaveLength(TOTAL_PAGES);
  for (const [idx, opaque] of opaqueCounts.entries()) {
    expect(
      opaque,
      `페이지 ${idx + 1} canvas에 실제로 그려진 불투명 픽셀이 있어야 함`,
    ).toBeGreaterThan(0);
  }

  // ── 부가 검증 ──
  expect(renderErrors, "PDF 렌더링 실패 에러 없음").toHaveLength(0);
  // 첫 페이지 단독 청크 + 나머지 묶음 청크가 요청됐는지 확인
  expect(chunkRanges).toContain("1-1");
});
