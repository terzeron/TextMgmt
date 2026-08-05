/**
 * PDF 브라우저 진단 모듈
 *
 * pdf.js(pdfjs-dist)를 사용하여 브라우저에서 PDF 구조를 검증한다.
 * EpubDiagnose.js와 동일한 결과 구조를 반환한다.
 */
import * as pdfjs from "pdfjs-dist";
// 워커를 설치된 pdfjs-dist에서 직접 번들 → API 버전과 항상 일치 (버전 drift 방지)
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjs.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

// JPEG2000(OpenJPEG)/JBIG2 등 이미지 디코딩용 wasm 위치 (vite pdf-wasm 플러그인이 서빙·번들).
// pdfjs 6.x 는 getDocument({ wasmUrl }) 로 wasm 디렉터리를 알려줘야 한다.
const PDF_WASM_URL = `${window.location.origin}${import.meta.env.BASE_URL}pdf-wasm/`;

// ─── 1. PDF 파싱 검사 ───

async function checkPdfParsing(buffer) {
  const results = [];
  let pdfDoc;

  try {
    pdfDoc = await pdfjs.getDocument({ data: buffer, wasmUrl: PDF_WASM_URL })
      .promise;
    results.push({ type: "ok", text: "pdf.js 파싱: 정상" });
  } catch (e) {
    const msg = e.message || String(e);
    if (msg.includes("password")) {
      results.push({
        type: "error",
        severity: "ERROR",
        text: `암호화된 PDF — 비밀번호 필요: ${msg}`,
      });
    } else {
      results.push({
        type: "error",
        severity: "FATAL",
        text: `pdf.js 파싱 실패: ${msg}`,
      });
    }
    return { results, pdfDoc: null };
  }

  return { results, pdfDoc };
}

// ─── 2. 메타데이터 검사 ───

async function checkMetadata(pdfDoc) {
  const results = [];

  try {
    const meta = await pdfDoc.getMetadata();
    const info = meta?.info || {};

    const fields = [
      ["Title", "title"],
      ["Author", "author"],
      ["Subject", "subject"],
      ["Creator", "creator"],
      ["Producer", "producer"],
      ["CreationDate", "creation_date"],
      ["ModDate", "mod_date"],
    ];

    let found = 0;
    for (const [key, label] of fields) {
      if (info[key]) {
        results.push({ type: "info", text: `${label}: ${info[key]}` });
        found++;
      }
    }

    if (info.IsAcroFormPresent) {
      results.push({ type: "info", text: "AcroForm(양식) 포함" });
    }

    if (found === 0) {
      results.push({
        type: "warn",
        severity: "WARNING",
        text: "문서 메타데이터 없음",
      });
    } else {
      results.push({ type: "ok", text: `메타데이터 필드 ${found}개 확인` });
    }
  } catch (e) {
    results.push({
      type: "error",
      severity: "ERROR",
      text: `메타데이터 추출 실패: ${e.message}`,
    });
  }

  return results;
}

// ─── 3. 페이지 구조 검사 ───

async function checkPageStructure(pdfDoc) {
  const results = [];
  const numPages = pdfDoc.numPages;
  results.push({ type: "info", text: `총 페이지 수: ${numPages}` });

  if (numPages === 0) {
    results.push({ type: "error", severity: "FATAL", text: "페이지가 없음" });
    return results;
  }

  // 첫 페이지 크기 확인
  try {
    const firstPage = await pdfDoc.getPage(1);
    const [x1, y1, x2, y2] = firstPage.view;
    const w = x2 - x1;
    const h = y2 - y1;
    results.push({
      type: "info",
      text: `첫 페이지 크기: ${Math.round(w)} × ${Math.round(h)} pt`,
    });
  } catch (e) {
    results.push({
      type: "error",
      severity: "ERROR",
      text: `첫 페이지 로드 실패: ${e.message}`,
    });
  }

  // 샘플 페이지 접근 검사 (최대 5개)
  const sampleIndices = [];
  if (numPages <= 5) {
    for (let i = 1; i <= numPages; i++) sampleIndices.push(i);
  } else {
    sampleIndices.push(1);
    sampleIndices.push(Math.floor(numPages * 0.25));
    sampleIndices.push(Math.floor(numPages * 0.5));
    sampleIndices.push(Math.floor(numPages * 0.75));
    sampleIndices.push(numPages);
  }

  let failCount = 0;
  for (const idx of sampleIndices) {
    try {
      await pdfDoc.getPage(idx);
    } catch (e) {
      failCount++;
      results.push({
        type: "error",
        severity: "ERROR",
        text: `페이지 ${idx} 로드 실패: ${e.message}`,
      });
    }
  }

  if (failCount === 0) {
    results.push({
      type: "ok",
      text: `샘플 ${sampleIndices.length}개 페이지 접근 정상`,
    });
  }

  return results;
}

// ─── 4. 텍스트 추출 검사 ───

async function checkTextExtraction(pdfDoc) {
  const results = [];

  try {
    const firstPage = await pdfDoc.getPage(1);
    const textContent = await firstPage.getTextContent();
    const textLength = textContent.items.reduce(
      (sum, item) => sum + (item.str || "").length,
      0,
    );

    if (textLength > 0) {
      results.push({
        type: "ok",
        text: `첫 페이지 텍스트 추출: ${textLength}자`,
      });
    } else {
      results.push({
        type: "warn",
        severity: "WARNING",
        text: "첫 페이지에서 텍스트 추출 불가 (이미지 기반 PDF일 수 있음)",
      });
    }
  } catch (e) {
    results.push({
      type: "error",
      severity: "ERROR",
      text: `텍스트 추출 실패: ${e.message}`,
    });
  }

  return results;
}

// ─── 메인 진단 함수 ───

/**
 * ArrayBuffer를 받아 PDF 진단을 수행하고 구조화된 결과를 반환한다.
 * @param {ArrayBuffer} arrayBuffer - PDF 파일 데이터
 * @returns {Promise<{sections: Array<{name: string, results: Array}>, summary: {fatal: number, errors: number, warnings: number}}>}
 */
export async function diagnosePdf(arrayBuffer) {
  const sections = [];
  let fatal = 0;
  let errors = 0;
  let warnings = 0;

  const countResults = (results) => {
    for (const r of results) {
      if (r.severity === "FATAL") fatal++;
      else if (r.type === "error") errors++;
      if (r.type === "warn") warnings++;
    }
  };

  // 1. PDF 파싱
  const { results: parseResults, pdfDoc } = await checkPdfParsing(arrayBuffer);
  countResults(parseResults);
  sections.push({ name: "PDF 파싱", results: parseResults });
  if (!pdfDoc) return { sections, summary: { fatal, errors, warnings } };

  try {
    // 2. 메타데이터
    const metaResults = await checkMetadata(pdfDoc);
    countResults(metaResults);
    sections.push({ name: "메타데이터", results: metaResults });

    // 3. 페이지 구조
    const pageResults = await checkPageStructure(pdfDoc);
    countResults(pageResults);
    sections.push({ name: "페이지 구조", results: pageResults });

    // 4. 텍스트 추출
    const textResults = await checkTextExtraction(pdfDoc);
    countResults(textResults);
    sections.push({ name: "텍스트 추출", results: textResults });
  } finally {
    // pdfjs 6.x: PDFDocumentProxy 대신 loadingTask로 정리
    pdfDoc.loadingTask.destroy();
  }

  return { sections, summary: { fatal, errors, warnings } };
}
