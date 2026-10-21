import * as pdfjs from "pdfjs-dist/legacy/build/pdf.mjs";
// Safari 18.x 는 Map.getOrInsertComputed 가 없어 일반 빌드가 깨지므로 legacy 빌드를 쓴다.
// 워커를 설치된 pdfjs-dist 에서 직접 번들 → API 버전과 항상 일치 (버전 drift 방지)
import pdfWorkerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";

pdfjs.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

// 설치된 pdfjs-dist의 보조 데이터를 모든 PDF 로딩 경로에서 함께 사용한다.
// CMap이 없으면 KSCms-UHC-H 등 CJK 인코딩의 글꼴 로딩이 실패해 문자가 사라진다.
/* v8 ignore next -- Vite BASE_URL fallback depends on the runtime bundle. */
export const PDF_WASM_URL = `${window.location.origin}${import.meta.env.BASE_URL}pdf-wasm/`;
export const PDF_DOCUMENT_OPTIONS = {
  cMapUrl: `${window.location.origin}${import.meta.env.BASE_URL}pdf-cmaps/`,
  cMapPacked: true,
  standardFontDataUrl: `${window.location.origin}${import.meta.env.BASE_URL}pdf-standard-fonts/`,
  wasmUrl: PDF_WASM_URL,
};

export { pdfjs };
