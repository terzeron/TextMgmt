import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix, rawJsonGetReq } from "./Common";
import { pdfjs, PDF_DOCUMENT_OPTIONS } from "./pdfjsEnv";
import {
  PREVIEW_HEIGHT,
  PREVIEW_PAGE_COUNT,
  previewScale,
} from "./directoryPreviewLayout";
import { describeHttpError } from "./httpError";
import "./DirectoryPagePreview.css";

// 디렉토리의 첫 파일(목록 정렬 기준 첫 번째)의 앞 10쪽을 왼쪽에서 오른쪽으로 나란히 보여 준다.
// 오른쪽에 자리가 없으면 아래 줄로 넘어간다. PDF 가 아니면 보여 주지 않는다.
export default function DirectoryPagePreview({ category, apiPrefix }) {
  // loading | empty | unsupported | error | ready
  const [state, setState] = useState({ status: "loading" });
  const docRef = useRef(null);
  const canvasRefs = useRef([]);

  // 1) 첫 파일을 찾고 앞 10쪽을 받아 PDF 문서를 연다.
  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading" });
    const fail = (message) => {
      if (!cancelled) setState({ status: "error", message });
    };
    const categoryPath = category
      .split("/")
      .map(encodeURIComponent)
      .join("/");

    rawJsonGetReq(
      `${apiPrefix}/categories/${categoryPath}?limit=1`,
      async (data) => {
        if (cancelled) return;
        if (data.status !== "success" || !Array.isArray(data.result)) {
          fail(data.error || "첫 파일을 불러오지 못했습니다.");
          return;
        }
        const file = data.result[0];
        if (!file) {
          setState({ status: "empty" });
          return;
        }
        const name = (file.file_path || "").split("/").pop() || file.title;
        if (file.file_type !== "pdf") {
          setState({ status: "unsupported", name });
          return;
        }
        try {
          const response = await fetch(
            `${getApiUrlPrefix()}${apiPrefix}/pdf-pages/${file.book_id}?start=1&end=${PREVIEW_PAGE_COUNT}`,
            { credentials: "include" },
          );
          if (!response.ok) throw new Error(await describeHttpError(response));
          const totalPages = parseInt(
            response.headers.get("X-Total-Pages") || "0",
            10,
          );
          const buffer = await response.arrayBuffer();
          if (cancelled) return;
          const doc = await pdfjs.getDocument({
            ...PDF_DOCUMENT_OPTIONS,
            data: buffer,
          }).promise;
          if (cancelled) {
            doc.loadingTask.destroy();
            return;
          }
          docRef.current = doc;
          setState({
            status: "ready",
            name,
            totalPages: totalPages || doc.numPages,
            pageCount: Math.min(doc.numPages, PREVIEW_PAGE_COUNT),
          });
        } catch (err) {
          fail(err?.message || "미리보기를 불러오지 못했습니다.");
        }
      },
      () => fail("첫 파일을 불러오지 못했습니다."),
    );

    return () => {
      cancelled = true;
      docRef.current?.loadingTask.destroy();
      docRef.current = null;
    };
  }, [category, apiPrefix]);

  // 2) canvas 가 만들어진 뒤 쪽을 차례로 그린다. 높이 PREVIEW_HEIGHT 기준 배율이라 폭은 쪽 비율이 정한다.
  const pageCount = state.status === "ready" ? state.pageCount : 0;
  useEffect(() => {
    const doc = docRef.current;
    if (!doc || pageCount === 0) return undefined;
    let cancelled = false;
    const dpr = window.devicePixelRatio || 1;
    (async () => {
      for (let number = 1; number <= pageCount; number += 1) {
        if (cancelled) return;
        try {
          const page = await doc.getPage(number);
          const canvas = canvasRefs.current[number - 1];
          if (cancelled || !canvas) return;
          const base = page.getViewport({ scale: 1 });
          if (!base.width || !base.height) continue;
          const viewport = page.getViewport({
            scale: previewScale(base.width, base.height, dpr),
          });
          canvas.width = Math.max(1, Math.floor(viewport.width));
          canvas.height = Math.max(1, Math.floor(viewport.height));
          await page.render({
            canvasContext: canvas.getContext("2d"),
            viewport,
          }).promise;
        } catch (err) {
          /* v8 ignore next 3 -- render errors after unmount are intentionally silent. */
          if (!cancelled) console.error(`미리보기 ${number}쪽 렌더링 실패:`, err);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pageCount]);

  if (state.status === "empty") return null;
  return (
    <section
      className="dir-page-preview"
      aria-label="디렉토리 첫 파일 미리보기"
    >
      {state.status === "loading" && (
        <div role="status" className="dir-page-preview-note">
          첫 파일 미리보기 불러오는 중...
        </div>
      )}
      {state.status === "unsupported" && (
        <div className="dir-page-preview-note">
          첫 파일({state.name})이 PDF가 아니라 미리보기를 표시하지 않습니다.
        </div>
      )}
      {state.status === "error" && (
        <div role="alert" className="dir-page-preview-error">
          미리보기를 불러오지 못했습니다: {state.message}
        </div>
      )}
      {state.status === "ready" && (
        <>
          <div className="dir-page-preview-caption" title={state.name}>
            {state.name} · 1~{pageCount}쪽 / 총 {state.totalPages}쪽
          </div>
          <div className="dir-page-preview-strip">
            {Array.from({ length: pageCount }, (_, index) => (
              <canvas
                key={index}
                ref={(element) => {
                  canvasRefs.current[index] = element;
                }}
                className="dir-page-preview-page"
                data-testid={`dir-page-preview-${index + 1}`}
                aria-label={`${index + 1}쪽`}
                style={{ height: `${PREVIEW_HEIGHT}px` }}
              />
            ))}
          </div>
        </>
      )}
    </section>
  );
}

DirectoryPagePreview.propTypes = {
  category: PropTypes.string.isRequired,
  apiPrefix: PropTypes.string.isRequired,
};
