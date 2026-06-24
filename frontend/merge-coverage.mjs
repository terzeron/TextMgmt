/**
 * 단위 테스트(Vitest)와 E2E(Playwright)의 V8 raw 커버리지를 하나로 병합한다.
 *
 * 입력:
 *   - coverage-reports/unit/raw            (vitest-monocart-coverage)
 *   - coverage-reports/e2e/<test>/raw      (coverage-fixture.js, 테스트별)
 * 출력:
 *   - coverage-reports/merged              (v8 UI + lcovonly + console-summary)
 */
import fs from "node:fs";
import path from "node:path";
import { CoverageReport } from "monocart-coverage-reports";

const root = path.resolve("coverage-reports");
const inputDir = [];

const unitRaw = path.join(root, "unit", "raw");
if (fs.existsSync(unitRaw)) inputDir.push(unitRaw);

const e2eDir = path.join(root, "e2e");
if (fs.existsSync(e2eDir)) {
  for (const entry of fs.readdirSync(e2eDir)) {
    const raw = path.join(e2eDir, entry, "raw");
    if (fs.existsSync(raw)) inputDir.push(raw);
  }
}

if (inputDir.length === 0) {
  console.error(
    "병합할 raw 커버리지가 없습니다. 먼저 `npm run test`와 `npm run coverage:e2e`를 실행하세요.",
  );
  process.exit(1);
}

console.log("Merging coverage from:");
for (const d of inputDir) console.log("  -", path.relative(root, d));

const coverageOptions = {
  name: "Merged Coverage (Unit + E2E)",
  inputDir,
  outputDir: path.join(root, "merged"),
  // node_modules 제외, 프로젝트 src만 포함
  entryFilter: { "**/node_modules/**": false, "**/*": true },
  sourceFilter: {
    "**/node_modules/**": false,
    "**/*.css": false,
    "**/src/**": true,
    // 그 외(e2e 번들 런타임 등)는 제외해 app 모듈/컴포넌트만 노출
    "**": false,
  },
  // 단위/E2E의 동일 파일 경로를 src/ 기준으로 통일해 병합되도록 한다.
  sourcePath: (filePath) => {
    const i = filePath.lastIndexOf("src/");
    return i >= 0 ? filePath.slice(i) : filePath;
  },
  // console-details: 파일(모듈/컴포넌트)별 커버리지 + 미커버 라인번호를 터미널에 출력
  // v8: 라인 단위로 탐색 가능한 HTML, lcovonly: CI/외부 도구 연동용
  reports: [
    // statements/branches 컬럼 + Uncovered Lines만 표시 (bytes/functions/lines 제외)
    ["console-details", { metrics: ["statements", "branches"] }],
    ["v8"],
    ["lcovonly"],
  ],
};

await new CoverageReport(coverageOptions).generate();
