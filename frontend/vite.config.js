import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import basicSsl from "@vitejs/plugin-basic-ssl";

// 설치된 pdfjs-dist의 wasm, CJK CMap, 기본 글꼴을 서빙(dev)·번들(build)한다.
// worker와 보조 데이터의 버전을 일치시켜 이미지와 비내장 글꼴 PDF를 함께 지원한다.
const PDF_WASM_PREFIX = "/pdf-wasm/";
const pdfWasmDir = fileURLToPath(
  new URL("./node_modules/pdfjs-dist/wasm", import.meta.url),
);
const PDF_ASSETS = [
  { prefix: PDF_WASM_PREFIX, dir: pdfWasmDir },
  {
    prefix: "/pdf-cmaps/",
    dir: fileURLToPath(new URL("./node_modules/pdfjs-dist/cmaps", import.meta.url)),
  },
  {
    prefix: "/pdf-standard-fonts/",
    dir: fileURLToPath(new URL("./node_modules/pdfjs-dist/standard_fonts", import.meta.url)),
  },
];

function getCoverVersion() {
  const parts = Object.fromEntries(
    new Intl.DateTimeFormat("en-CA", {
      timeZone: "Asia/Seoul",
      year: "2-digit",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    })
      .formatToParts(new Date())
      .filter(({ type }) => type !== "literal")
      .map(({ type, value }) => [type, value]),
  );
  return `${parts.year}${parts.month}${parts.day}${parts.hour}${parts.minute}`;
}

function pdfWasmPlugin() {
  return {
    name: "pdf-wasm",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const asset = PDF_ASSETS.find(({ prefix }) => req.url?.startsWith(prefix));
        if (!asset) return next();
        const rel = req.url.slice(asset.prefix.length).split("?")[0];
        if (!rel || path.basename(rel) !== rel) return next();
        const file = path.join(asset.dir, rel);
        if (!fs.existsSync(file) || !fs.statSync(file).isFile()) return next();
        res.setHeader(
          "Content-Type",
          file.endsWith(".wasm") ? "application/wasm" :
            file.endsWith(".js") ? "text/javascript" : "application/octet-stream",
        );
        fs.createReadStream(file).pipe(res);
      });
    },
    generateBundle() {
      for (const { prefix, dir } of PDF_ASSETS) {
        for (const name of fs.readdirSync(dir)) {
          this.emitFile({
            type: "asset",
            fileName: `${prefix.slice(1)}${name}`,
            source: fs.readFileSync(path.join(dir, name)),
          });
        }
      }
    },
  };
}

// https://vitejs.dev/config/
export default defineConfig({
  // 사용자가 첫 화면 하단에서 배포 상태를 직접 확인할 수 있도록 빌드 ID를 박는다.
  // APP_BUILD_ID(이미지 빌드 시 주입 가능)가 없으면 빌드 시각을 쓴다.
  define: {
    __APP_BUILD_ID__: JSON.stringify(
      process.env.APP_BUILD_ID || new Date().toISOString(),
    ),
    __COVER_VERSION__: JSON.stringify(getCoverVersion()),
  },
  server: {
    https: true,
    proxy: {
      "/search": {
        target: "http://localhost:8000",
        changeOrigin: true,
        secure: false,
      },
    },
  },
  plugins: [basicSsl(), react(), pdfWasmPlugin()],
  build: {
    // E2E 커버리지 수집 시에는 inline 소스맵으로 빌드해, Playwright가 모은 V8
    // 커버리지를 MCR이 원본 src로 역매핑할 때 외부 .map을 원격 fetch할 필요가 없게 한다.
    // 그 외(배포용 빌드)에는 소스맵을 만들지 않는다. dist 는 nginx 이미지에 통째로
    // 복사되므로 .map 을 남기면 전체 원본 코드가 그대로 서빙된다.
    sourcemap: process.env.E2E_COVERAGE === "1" ? "inline" : false,
    rollupOptions: {
      output: {
        manualChunks: (id) => {
          if (id.includes("pdfjs-dist")) return "pdfjs";
        },
      },
    },
  },
  test: {
    include: ["tests/**/*.test.{js,jsx}"],
    environment: "jsdom",
    setupFiles: ["tests/setup.js"],
    reporters: ["./tests/reporter.js"],
    // jsdom + coverage workers can exceed memory on high-core hosts.
    // Override with VITEST_MAX_WORKERS when a larger runner is available.
    maxWorkers: process.env.VITEST_MAX_WORKERS || 2,
    coverage: {
      provider: "v8",
      include: ["src/**/*.{js,jsx}"],
      reportsDirectory: "coverage-reports/unit",
      reporter: [["text", { skipFull: false }], "html", "lcovonly"],
    },
  },
});
