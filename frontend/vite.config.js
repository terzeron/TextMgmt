import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import basicSsl from "@vitejs/plugin-basic-ssl";

// pdfjs-dist 6.x 는 JPEG2000(OpenJPEG)·JBIG2·ICC 디코딩에 wasm 파일을 사용하며
// getDocument({ wasmUrl }) 로 그 디렉터리를 알려줘야 한다. 설치된 pdfjs-dist 의 wasm 을
// 그대로 /pdf-wasm/ 로 서빙(dev)·번들(build)하여 worker API 버전과 항상 일치시킨다.
const PDF_WASM_PREFIX = "/pdf-wasm/";
const pdfWasmDir = fileURLToPath(
  new URL("./node_modules/pdfjs-dist/wasm", import.meta.url),
);

function pdfWasmPlugin() {
  return {
    name: "pdf-wasm",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (!req.url || !req.url.startsWith(PDF_WASM_PREFIX)) return next();
        const rel = req.url.slice(PDF_WASM_PREFIX.length).split("?")[0];
        const file = path.join(pdfWasmDir, rel);
        if (!file.startsWith(pdfWasmDir) || !fs.existsSync(file)) return next();
        res.setHeader(
          "Content-Type",
          file.endsWith(".wasm") ? "application/wasm" : "text/javascript",
        );
        fs.createReadStream(file).pipe(res);
      });
    },
    generateBundle() {
      for (const name of fs.readdirSync(pdfWasmDir)) {
        this.emitFile({
          type: "asset",
          fileName: `pdf-wasm/${name}`,
          source: fs.readFileSync(path.join(pdfWasmDir, name)),
        });
      }
    },
  };
}

// https://vitejs.dev/config/
export default defineConfig({
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
    coverage: {
      // MCR custom provider로 단위 테스트 커버리지를 V8로 수집한다.
      // MCR 옵션(name/outputDir/reports 등)은 mcr.config.js에서 읽는다.
      // raw 산출물은 e2e 커버리지와 병합(merge-coverage.mjs)하는 데 쓰인다.
      provider: "custom",
      customProviderModule: "vitest-monocart-coverage",
      include: ["src/**/*.{js,jsx}"],
    },
  },
});
