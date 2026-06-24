// Monocart Coverage Reports 설정 (단위 테스트용).
// vitest-monocart-coverage(custom provider)가 이 파일을 자동 로드한다.
// raw 리포트는 e2e 커버리지와 병합(merge-coverage.mjs)하기 위한 입력이다.
export default {
  name: "Unit Coverage (Vitest)",
  outputDir: "./coverage-reports/unit",
  // console-details: 파일(모듈/컴포넌트)별 커버리지 + 미커버 라인번호를 터미널에 출력
  // v8: 라인 단위로 볼 수 있는 HTML 리포트, raw: e2e와 병합용 입력
  reports: [
    // statements/branches 컬럼 + Uncovered Lines만 표시 (bytes/functions/lines 제외)
    ["console-details", { metrics: ["statements", "branches"] }],
    ["raw", { outputDir: "raw" }],
    ["v8"],
  ],
  sourceFilter: {
    "**/node_modules/**": false,
    // CSS는 JS 커버리지 대상이 아니며, import된 CSS는 소스 콘텐츠가 없어
    // "not found source content" 경고만 유발하므로 제외한다.
    "**/*.css": false,
    "**/src/**": true,
  },
  cleanCache: true,
};
