import { test, expect } from "@playwright/test";

// 글꼴과 ToUnicode를 내장하지 않은 UHC PDF. CMap이 빠지면 페이지 렌더는
// 성공하지만 한글만 사라지므로, 성공 상태와 별개로 실제 잉크 픽셀을 검사한다.
function buildKoreanPdf(numPages) {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Count ${numPages} /Kids [${Array.from({ length: numPages }, (_, i) => `${6 + i * 2} 0 R`).join(" ")}] >>`,
    "<< /Type /Font /Subtype /Type0 /BaseFont /Batang /Encoding /KSCms-UHC-H /DescendantFonts [4 0 R] >>",
    "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Batang /CIDSystemInfo << /Registry (Adobe) /Ordering (Korea1) /Supplement 1 >> /FontDescriptor 5 0 R /DW 1000 >>",
    "<< /Type /FontDescriptor /FontName /Batang /Flags 6 /FontBBox [-100 -300 1200 1000] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>",
  ];
  for (let i = 0; i < numPages; i++) {
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 3 0 R >> >> /Contents ${7 + i * 2} 0 R >>`);
    const stream = "BT /F1 28 Tf 50 100 Td <C7D1B1DB> Tj ET"; // 한글
    objects.push(`<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`);
  }
  let pdf = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((object, i) => {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${i + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${offsets.length}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  pdf += `trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf);
}

test("비내장 UHC 글꼴 PDF의 첫 쪽과 후속 청크에 한글을 그린다", async ({ page, request }) => {
  // 빌드 산출물에서 CMap과 기본 글꼴이 실제 바이너리로 제공되는지도 검사한다.
  for (const asset of [
    "pdf-cmaps/KSCms-UHC-H.bcmap",
    "pdf-cmaps/Adobe-Korea1-UCS2.bcmap",
    "pdf-standard-fonts/LiberationSans-Regular.ttf",
  ]) {
    const response = await request.get(`/${asset}`);
    expect(response.ok()).toBe(true);
    expect(response.headers()["content-type"]).not.toContain("text/html");
    expect((await response.body()).length).toBeGreaterThan(100);
  }
  await page.route("**/auth/me", (route) => route.fulfill({
    json: { status: "success", result: { role: "admin", name: "E2E", email: "e2e@example.com" } },
  }));
  await page.route("**/auth/refresh", (route) => route.fulfill({ json: { status: "success" } }));
  const ranges = [];
  await page.route("**/pdf-pages/**", (route) => {
    const url = new URL(route.request().url());
    const start = Number(url.searchParams.get("start"));
    const end = Number(url.searchParams.get("end"));
    ranges.push(`${start}-${end}`);
    return route.fulfill({
      contentType: "application/pdf",
      headers: { "X-Total-Pages": "3", "Access-Control-Expose-Headers": "X-Total-Pages" },
      body: buildKoreanPdf(end - start + 1),
    });
  });
  const errors = [];
  page.on("console", (msg) => {
    if (/translateFont failed|Error during font loading|Unable to load (CMap|font)|렌더링 실패/.test(msg.text())) errors.push(msg.text());
  });
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/viewer/pdf/12345?path=test.pdf");
  await expect(page.locator("canvas.pdf-page")).toHaveCount(3);
  for (const canvas of await page.locator("canvas.pdf-page").all()) {
    await canvas.scrollIntoViewIfNeeded();
    await expect.poll(() => canvas.evaluate((el) => {
      const pixels = el.getContext("2d").getImageData(0, 0, el.width, el.height).data;
      let ink = 0;
      for (let i = 0; i < pixels.length; i += 4) {
        if (pixels[i + 3] > 0 && pixels[i] < 128 && pixels[i + 1] < 128 && pixels[i + 2] < 128) ink++;
      }
      return ink;
    })).toBeGreaterThan(100);
  }
  expect(ranges).toContain("1-1");
  expect(ranges).toContain("2-3");
  expect(errors).toEqual([]);
});
