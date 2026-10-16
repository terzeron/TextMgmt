import * as pdfjs from "pdfjs-dist/legacy/build/pdf.mjs";
// Safari 18.x 는 Map.getOrInsertComputed 가 없어 일반 빌드가 깨지므로 legacy 빌드를 쓴다.
// 워커를 설치된 pdfjs-dist 에서 직접 번들 → API 버전과 항상 일치 (버전 drift 방지)
import pdfWorkerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";

pdfjs.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

// JPEG2000(OpenJPEG)/JBIG2 등 이미지 디코딩용 wasm 위치. 미지정 시 이미지 기반 페이지가 렌더되지 않는다.
// (ViewPDF.jsx 도 같은 설정을 갖고 있다. 이 모듈은 ViewPDF 밖에서 PDF 를 그리는 화면용이다.)
/* v8 ignore next -- Vite BASE_URL fallback depends on the runtime bundle. */
export const PDF_WASM_URL = `${window.location.origin}${import.meta.env.BASE_URL}pdf-wasm/`;

export { pdfjs };
