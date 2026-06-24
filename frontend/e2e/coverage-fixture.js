/**
 * E2E 커버리지 수집 픽스처.
 *
 * E2E_COVERAGE=1 일 때만 동작한다. 각 테스트에서 Chromium V8 JS 커버리지를
 * 수집해 MCR raw 리포트로 출력한다(테스트별 디렉터리). merge-coverage.mjs가
 * 단위 테스트 raw와 함께 병합한다.
 *
 * 일반 e2e 실행(E2E_COVERAGE 미설정)에서는 오버헤드 없이 그대로 통과한다.
 */
import { test as base, expect } from "@playwright/test";
import { CoverageReport } from "monocart-coverage-reports";

const COLLECT = process.env.E2E_COVERAGE === "1";

function sanitize(title) {
  return title.replace(/[^a-z0-9]+/gi, "_").slice(0, 80) || "test";
}

export const test = base.extend({
  // 두 번째 인자는 Playwright의 use 함수지만, eslint react-hooks 규칙이
  // 식별자 `use`를 React Hook으로 오인하므로 runTest로 명명한다.
  page: async ({ page }, runTest, testInfo) => {
    // page.coverage는 Chromium 계열에서만 제공된다.
    const canCover = COLLECT && page.coverage;
    if (canCover) {
      await page.coverage.startJSCoverage({ resetOnNavigation: false });
    }

    await runTest(page);

    if (canCover) {
      const jsCoverage = await page.coverage.stopJSCoverage();
      // Playwright 엔트리는 ranges에 start/end를 쓰지만 MCR(V8 처리기)은
      // 네이티브 V8 형식(startOffset/endOffset)을 기대한다. 명시적으로 변환한다.
      const coverageData = jsCoverage
        // rolldown 런타임 헬퍼 청크는 app 모듈/컴포넌트가 아니므로 수집 단계에서 제외
        .filter((it) => !(it.url || "").includes("rolldown-runtime"))
        .map((it) => ({
          url: it.url,
          source: it.source,
          functions: (it.functions || [])
            .map((fn) => ({
              functionName: fn.functionName,
              isBlockCoverage: fn.isBlockCoverage,
              ranges: (fn.ranges || [])
                .filter(
                  (r) =>
                    typeof r.start === "number" && typeof r.end === "number",
                )
                .map((r) => ({
                  startOffset: r.start,
                  endOffset: r.end,
                  count: r.count,
                })),
            }))
            // root range가 없는 함수는 MCR 병합 시 오류를 유발하므로 제외
            .filter((fn) => fn.ranges.length > 0),
        }));
      const report = new CoverageReport({
        name: `E2E: ${testInfo.title}`,
        outputDir: `./coverage-reports/e2e/${sanitize(testInfo.title)}`,
        reports: [["raw", { outputDir: "raw" }]],
        cleanCache: true,
        logging: "off",
      });
      await report.add(coverageData);
      await report.generate();
    }
  },
});

export { expect };
