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
    sourcemap: true,
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
      provider: "v8",
      reporter: ["text"],
      include: ["src/**/*.{js,jsx}"],
    },
  },
});
