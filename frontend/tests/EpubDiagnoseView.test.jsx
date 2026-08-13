// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  act,
  cleanup,
  waitFor,
  fireEvent,
} from "@testing-library/react";

afterEach(cleanup);

// ─── 모킹 ───

vi.mock("../src/Common", () => ({
  getApiUrlPrefix: () => "http://localhost:8000",
  jsonGetReq: vi.fn(),
}));

vi.mock("../src/EpubDiagnose", () => ({
  diagnoseEpub: vi.fn(),
}));

vi.mock("../src/PdfDiagnose", () => ({
  diagnosePdf: vi.fn(),
}));

import { jsonGetReq } from "../src/Common";
import { diagnoseEpub } from "../src/EpubDiagnose";
import { diagnosePdf } from "../src/PdfDiagnose";
import EpubDiagnoseView from "../src/EpubDiagnoseView";

const MOCK_BACKEND_DATA = {
  valid: true,
  file_path: "books/test.epub",
  messages: [],
  summary: { fatal: 0, error: 0, warning: 0, usage: 0, info: 0 },
  publication: {
    title: "Test Book",
    creator: "Author",
    publisher: "Publisher",
  },
};

const MOCK_BACKEND_DATA_INVALID = {
  valid: false,
  file_path: "books/bad.epub",
  messages: [
    {
      severity: "ERROR",
      id: "OPF-001",
      message: "Missing required element",
      location: { path: "content.opf", line: 5, column: 10 },
    },
    {
      severity: "WARNING",
      id: "CSS-001",
      message: "CSS font-face issue",
      location: { path: "style.css", line: 1, column: 1 },
    },
  ],
  summary: { fatal: 0, error: 1, warning: 1, usage: 0, info: 0 },
};

const MOCK_BACKEND_DATA_FATAL = {
  valid: false,
  file_path: "books/corrupt.epub",
  messages: [
    {
      severity: "FATAL",
      id: "PKG-004",
      message: "Corrupted EPUB ZIP header",
      location: { path: "corrupt.epub", line: -1, column: -1 },
    },
    {
      severity: "ERROR",
      id: "OPF-030",
      message: "Unique identifier not found",
      location: { path: "content.opf", line: 3, column: 1 },
    },
  ],
  summary: { fatal: 1, error: 1, warning: 0, usage: 0, info: 0 },
};

const MOCK_FRONTEND_DATA = {
  sections: [
    {
      name: "ZIP 구조",
      results: [{ type: "ok", text: "mimetype: application/epub+zip" }],
    },
    {
      name: "OPF 파싱",
      results: [{ type: "ok", text: "브라우저 DOMParser 파싱: 정상" }],
    },
  ],
  summary: { fatal: 0, errors: 0, warnings: 0 },
};

const MOCK_FRONTEND_DATA_WITH_ERRORS = {
  sections: [
    {
      name: "ZIP 구조",
      results: [{ type: "ok", text: "mimetype: application/epub+zip" }],
    },
    {
      name: "OPF 파싱",
      results: [
        {
          type: "error",
          severity: "FATAL",
          text: "브라우저 DOMParser 파싱 실패",
        },
      ],
    },
  ],
  summary: { fatal: 1, errors: 0, warnings: 0 },
};

const MOCK_FRONTEND_DATA_MIXED = {
  sections: [
    {
      name: "ZIP 구조",
      results: [{ type: "ok", text: "mimetype: application/epub+zip" }],
    },
    {
      name: "Spine 파일",
      results: [
        {
          type: "error",
          severity: "ERROR",
          text: 'spine "ch2" → OEBPS/ch2.xhtml ZIP에 없음',
        },
      ],
    },
    {
      name: "OPF 파싱",
      results: [
        { type: "warn", severity: "WARNING", text: "dc:language 없음" },
      ],
    },
  ],
  summary: { fatal: 0, errors: 1, warnings: 1 },
};

const MOCK_PDF_FRONTEND_DATA = {
  sections: [
    { name: "PDF 파싱", results: [{ type: "ok", text: "pdf.js 파싱: 정상" }] },
    {
      name: "메타데이터",
      results: [{ type: "ok", text: "메타데이터 필드 3개 확인" }],
    },
    {
      name: "페이지 구조",
      results: [{ type: "ok", text: "샘플 5개 페이지 접근 정상" }],
    },
    {
      name: "텍스트 추출",
      results: [{ type: "ok", text: "첫 페이지 텍스트 추출: 500자" }],
    },
  ],
  summary: { fatal: 0, errors: 0, warnings: 0 },
};

const MOCK_PDF_FRONTEND_DATA_WITH_WARNINGS = {
  sections: [
    { name: "PDF 파싱", results: [{ type: "ok", text: "pdf.js 파싱: 정상" }] },
    {
      name: "메타데이터",
      results: [
        { type: "warn", severity: "WARNING", text: "문서 메타데이터 없음" },
      ],
    },
    {
      name: "텍스트 추출",
      results: [
        {
          type: "warn",
          severity: "WARNING",
          text: "첫 페이지에서 텍스트 추출 불가 (이미지 기반 PDF일 수 있음)",
        },
      ],
    },
  ],
  summary: { fatal: 0, errors: 0, warnings: 2 },
};

function setupMocks({
  backendData = MOCK_BACKEND_DATA,
  backendError = null,
  frontendData = MOCK_FRONTEND_DATA,
  frontendError = null,
  pdfFrontendData = MOCK_PDF_FRONTEND_DATA,
  pdfFrontendError = null,
} = {}) {
  jsonGetReq.mockImplementation((url, payload, resolve, reject) => {
    if (backendError) {
      reject(backendError);
    } else {
      resolve(backendData);
    }
  });

  globalThis.fetch = vi.fn(() =>
    Promise.resolve({
      ok: true,
      arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)),
    }),
  );

  if (frontendError) {
    diagnoseEpub.mockRejectedValue(new Error(frontendError));
  } else {
    diagnoseEpub.mockResolvedValue(frontendData);
  }

  if (pdfFrontendError) {
    diagnosePdf.mockRejectedValue(new Error(pdfFrontendError));
  } else {
    diagnosePdf.mockResolvedValue(pdfFrontendData);
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  globalThis.fetch = vi.fn(() =>
    Promise.resolve({
      ok: true,
      arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)),
    }),
  );
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ─── 테스트 ───

describe("EpubDiagnoseView", () => {
  describe("조건부 렌더링", () => {
    it("fileType이 epub/pdf가 아니면 렌더링하지 않는다", () => {
      const { container } = render(
        <EpubDiagnoseView bookId={1} fileType="txt" />,
      );
      expect(container.firstChild).toBeNull();
    });

    it("fileType이 epub이면 카드 헤더를 렌더링한다", () => {
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);
      expect(screen.getByText(/파일 정합성 진단/)).toBeTruthy();
    });

    it("fileType이 pdf이면 카드 헤더를 렌더링한다", () => {
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);
      expect(screen.getByText(/파일 정합성 진단/)).toBeTruthy();
    });

    it("초기 상태에서 카드 본문은 숨겨져 있다", () => {
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);
      expect(screen.queryByText(/Backend 진단/)).toBeNull();
    });
  });

  describe("카드 토글", () => {
    it("헤더 클릭 시 카드 본문이 열린다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText(/Backend 진단/)).toBeTruthy();
        expect(screen.getByText(/Frontend 진단/)).toBeTruthy();
      });
    });

    it("헤더를 두 번 클릭하면 카드 본문이 닫힌다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      await waitFor(() =>
        expect(screen.getByText(/Backend 진단/)).toBeTruthy(),
      );

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      expect(screen.queryByText(/Backend 진단/)).toBeNull();
    });
  });

  describe("진단 실행", () => {
    it("카드 열릴 때 백엔드 API를 호출한다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={42} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      expect(jsonGetReq).toHaveBeenCalledWith(
        "/validate/42",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });

    it("카드 열릴 때 프론트엔드 진단용 fetch를 호출한다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={42} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/download/42",
        expect.objectContaining({ signal: expect.any(AbortSignal) }),
      );
    });

    it("이미 진단이 실행됐으면 카드를 닫았다 열어도 재실행하지 않는다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      // 열기
      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      await waitFor(() => expect(jsonGetReq).toHaveBeenCalledTimes(1));

      // 닫기 + 열기
      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      expect(jsonGetReq).toHaveBeenCalledTimes(1);
    });

    it("bookId 변경 후 다시 진단하면 이전 AbortController를 정리한다", async () => {
      setupMocks();
      const abortSpy = vi.spyOn(AbortController.prototype, "abort");
      const { rerender } = render(
        <EpubDiagnoseView bookId={1} fileType="epub" />,
      );

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      await waitFor(() => expect(jsonGetReq).toHaveBeenCalledTimes(1));

      await act(async () => {
        rerender(<EpubDiagnoseView bookId={2} fileType="epub" />);
      });

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => expect(jsonGetReq).toHaveBeenCalledTimes(2));
      expect(abortSpy).toHaveBeenCalled();
    });
  });

  describe("백엔드 결과 표시", () => {
    it("유효한 EPUB에서 VALID 배지를 표시한다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("VALID")).toBeTruthy();
      });
    });

    it("유효하지 않은 EPUB에서 INVALID 배지와 severity별 그룹 및 메시지를 모두 표시한다", async () => {
      setupMocks({ backendData: MOCK_BACKEND_DATA_INVALID });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("INVALID")).toBeTruthy();
        // severity 그룹 헤더
        expect(screen.getByText("ERROR")).toBeTruthy();
        expect(screen.getByText(/1건 — 스펙 위반/)).toBeTruthy();
        expect(screen.getByText("WARNING")).toBeTruthy();
        expect(screen.getByText(/1건 — 권장사항/)).toBeTruthy();
        // 모든 메시지가 바로 표시됨
        expect(screen.getByText(/Missing required element/)).toBeTruthy();
        expect(screen.getByText(/CSS font-face issue/)).toBeTruthy();
      });
    });

    it("FATAL 메시지도 바로 표시된다", async () => {
      setupMocks({ backendData: MOCK_BACKEND_DATA_FATAL });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("FATAL")).toBeTruthy();
        expect(screen.getByText(/Corrupted EPUB ZIP header/)).toBeTruthy();
        expect(screen.getByText("ERROR")).toBeTruthy();
        expect(screen.getByText(/Unique identifier not found/)).toBeTruthy();
      });
    });

    it("백엔드 에러 시 에러 메시지를 표시한다", async () => {
      setupMocks({ backendError: "epubcheck is not installed" });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("epubcheck is not installed")).toBeTruthy();
      });
    });

    it("publication 메타데이터를 표시한다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText(/Test Book.*Author.*Publisher/)).toBeTruthy();
      });
    });
  });

  describe("프론트엔드 결과 표시", () => {
    it("이상 없을 때 PASS 배지와 이상 없음을 표시한다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("PASS")).toBeTruthy();
        expect(screen.getByText("이상 없음")).toBeTruthy();
      });
    });

    it("FATAL 에러 시 FAIL 배지와 심각도 그룹을 표시한다", async () => {
      setupMocks({ frontendData: MOCK_FRONTEND_DATA_WITH_ERRORS });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("FAIL")).toBeTruthy();
        expect(screen.getByText("FATAL")).toBeTruthy();
        expect(screen.getByText(/1건 — 렌더링 불가/)).toBeTruthy();
        expect(screen.getByText(/브라우저 DOMParser 파싱 실패/)).toBeTruthy();
      });
    });

    it("ERROR/WARNING 혼합 시 심각도별 그룹을 모두 표시한다", async () => {
      setupMocks({ frontendData: MOCK_FRONTEND_DATA_MIXED });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("ERROR")).toBeTruthy();
        expect(screen.getByText(/1건 — 스펙 위반/)).toBeTruthy();
        expect(screen.getByText(/spine.*ZIP에 없음/)).toBeTruthy();
        expect(screen.getByText("WARNING")).toBeTruthy();
        expect(screen.getByText(/1건 — 권장사항/)).toBeTruthy();
        expect(screen.getByText(/dc:language 없음/)).toBeTruthy();
      });
    });

    it("프론트엔드 진단 에러 시 에러 메시지를 표시한다", async () => {
      setupMocks({ frontendError: "서버 응답 오류: 500" });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("서버 응답 오류: 500")).toBeTruthy();
      });
    });
  });

  describe("bookId 변경", () => {
    it("bookId 변경 시 상태가 초기화된다", async () => {
      setupMocks();
      const { rerender } = render(
        <EpubDiagnoseView bookId={1} fileType="epub" />,
      );

      // 카드 열기 + 데이터 로드
      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      await waitFor(() => expect(screen.getByText("VALID")).toBeTruthy());

      // bookId 변경
      await act(async () => {
        rerender(<EpubDiagnoseView bookId={2} fileType="epub" />);
      });

      // 카드가 닫혀야 하고, 이전 데이터가 없어야 함
      expect(screen.queryByText("VALID")).toBeNull();
      expect(screen.queryByText(/Backend 진단/)).toBeNull();
    });
  });

  describe("fetch 취소", () => {
    it("bookId 변경 시 이전 fetch의 AbortController가 abort된다", async () => {
      setupMocks();

      // fetch를 지연시킨다
      let _fetchResolve;
      globalThis.fetch = vi.fn(
        () =>
          new Promise((resolve) => {
            _fetchResolve = resolve;
          }),
      );

      const { rerender } = render(
        <EpubDiagnoseView bookId={1} fileType="epub" />,
      );

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      // fetch가 호출되었지만 아직 resolve하지 않음
      expect(globalThis.fetch).toHaveBeenCalledTimes(1);
      const firstCallSignal = globalThis.fetch.mock.calls[0][1].signal;

      // bookId 변경 → abort 호출됨
      await act(async () => {
        rerender(<EpubDiagnoseView bookId={2} fileType="epub" />);
      });

      expect(firstCallSignal.aborted).toBe(true);
    });
  });

  describe("PDF 지원", () => {
    const MOCK_PDF_BACKEND_DATA = {
      valid: true,
      file_path: "books/test.pdf",
      messages: [],
      summary: { error: 0, warning: 0 },
      publication: {
        title: "PDF Book",
        creator: "PDF Author",
        producer: "Test Producer",
        page_count: 42,
        pdf_version: "1.7",
      },
    };

    const MOCK_PDF_BACKEND_DATA_INVALID = {
      valid: false,
      file_path: "books/bad.pdf",
      messages: [
        { severity: "WARNING", message: "cross-reference table mismatch" },
      ],
      summary: { error: 0, warning: 1 },
      publication: { page_count: 10, pdf_version: "1.5" },
    };

    it("PDF에서 카드를 열면 Backend 진단(pikepdf) 헤더가 표시된다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText(/Backend 진단 \(pikepdf\)/)).toBeTruthy();
      });
    });

    it("PDF에서 Frontend 진단(pdf.js) 헤더가 표시된다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText(/Frontend 진단 \(pdf\.js\)/)).toBeTruthy();
      });
    });

    it("PDF에서 fetch를 호출하여 프론트엔드 진단을 수행한다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      render(<EpubDiagnoseView bookId={42} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/download/42",
        expect.objectContaining({ signal: expect.any(AbortSignal) }),
      );
    });

    it("PDF에서 frontend 진단 정상 시 PASS 배지가 표시된다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("PASS")).toBeTruthy();
        expect(screen.getByText("이상 없음")).toBeTruthy();
      });
    });

    it("PDF에서 frontend 진단 WARNING 시 심각도 그룹이 표시된다", async () => {
      setupMocks({
        backendData: MOCK_PDF_BACKEND_DATA,
        pdfFrontendData: MOCK_PDF_FRONTEND_DATA_WITH_WARNINGS,
      });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("WARNING")).toBeTruthy();
        expect(screen.getByText(/이미지 기반 PDF/)).toBeTruthy();
      });
    });

    it("PDF에서 frontend 진단 FATAL 시 FAIL 배지가 표시된다", async () => {
      const fatalData = {
        sections: [
          {
            name: "PDF 파싱",
            results: [
              {
                type: "error",
                severity: "FATAL",
                text: "pdf.js 파싱 실패: Invalid PDF structure",
              },
            ],
          },
        ],
        summary: { fatal: 1, errors: 0, warnings: 0 },
      };
      setupMocks({
        backendData: MOCK_PDF_BACKEND_DATA,
        pdfFrontendData: fatalData,
      });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("FAIL")).toBeTruthy();
        expect(screen.getByText("FATAL")).toBeTruthy();
        expect(screen.getByText(/Invalid PDF structure/)).toBeTruthy();
      });
    });

    it("PDF에서 frontend 진단 fetch 실패 시 에러 메시지를 표시한다", async () => {
      setupMocks({
        backendData: MOCK_PDF_BACKEND_DATA,
        pdfFrontendError: "pdf.js worker 로드 실패",
      });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("pdf.js worker 로드 실패")).toBeTruthy();
      });
    });

    it("PDF에서 diagnoseEpub이 호출되지 않고 diagnosePdf가 호출된다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(diagnosePdf).toHaveBeenCalled();
      });
      expect(diagnoseEpub).not.toHaveBeenCalled();
    });

    it("EPUB에서 diagnosePdf가 호출되지 않고 diagnoseEpub이 호출된다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(diagnoseEpub).toHaveBeenCalled();
      });
      expect(diagnosePdf).not.toHaveBeenCalled();
    });

    it("EPUB에서는 Frontend 진단(브라우저 DOMParser) 헤더가 표시된다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(
          screen.getByText(/Frontend 진단 \(브라우저 DOMParser\)/),
        ).toBeTruthy();
      });
    });

    it("유효한 PDF에서 VALID 배지와 메타데이터가 표시된다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("VALID")).toBeTruthy();
        expect(screen.getByText(/PDF Book/)).toBeTruthy();
        expect(screen.getByText(/42p/)).toBeTruthy();
        expect(screen.getByText(/PDF 1\.7/)).toBeTruthy();
      });
    });

    it("문제 있는 PDF에서 INVALID 배지와 WARNING 메시지가 표시된다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA_INVALID });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("INVALID")).toBeTruthy();
        expect(screen.getByText("WARNING")).toBeTruthy();
        expect(screen.getByText(/cross-reference table mismatch/)).toBeTruthy();
      });
    });

    it("EPUB에서는 Backend 진단(epubcheck) 헤더가 표시된다", async () => {
      setupMocks();
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText(/Backend 진단 \(epubcheck\)/)).toBeTruthy();
      });
    });

    it("PDF에서 백엔드 에러 시 에러 메시지를 표시한다", async () => {
      setupMocks({ backendError: "Failed to open PDF: not a PDF file" });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(
          screen.getByText("Failed to open PDF: not a PDF file"),
        ).toBeTruthy();
      });
    });

    it("PDF에서 bookId 변경 시 상태가 초기화된다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      const { rerender } = render(
        <EpubDiagnoseView bookId={1} fileType="pdf" />,
      );

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      await waitFor(() => expect(screen.getByText("VALID")).toBeTruthy());

      await act(async () => {
        rerender(<EpubDiagnoseView bookId={2} fileType="pdf" />);
      });

      expect(screen.queryByText("VALID")).toBeNull();
      expect(screen.queryByText(/Backend 진단/)).toBeNull();
    });

    it("PDF에서 카드를 토글할 수 있다", async () => {
      setupMocks({ backendData: MOCK_PDF_BACKEND_DATA });
      render(<EpubDiagnoseView bookId={1} fileType="pdf" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      await waitFor(() =>
        expect(screen.getByText(/Backend 진단/)).toBeTruthy(),
      );

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });
      expect(screen.queryByText(/Backend 진단/)).toBeNull();
    });
  });

  describe("프론트엔드 심각도 표시", () => {
    it("심각도 그룹에 섹션명이 location으로 표시된다", async () => {
      setupMocks({ frontendData: MOCK_FRONTEND_DATA_MIXED });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        // ERROR 그룹의 location에 Spine 파일 섹션명이 표시됨
        expect(screen.getByText("Spine 파일")).toBeTruthy();
        // WARNING 그룹의 location에 OPF 파싱 섹션명이 표시됨
        expect(screen.getByText("OPF 파싱")).toBeTruthy();
      });
    });

    it("FATAL만 있을 때 FAIL 배지와 FATAL 그룹이 표시된다", async () => {
      setupMocks({ frontendData: MOCK_FRONTEND_DATA_WITH_ERRORS });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("FAIL")).toBeTruthy();
        expect(screen.getByText("FATAL")).toBeTruthy();
        // ERROR/WARNING 그룹은 없음
        expect(screen.queryByText(/스펙 위반/)).toBeNull();
        expect(screen.queryByText(/권장사항 미준수/)).toBeNull();
      });
    });

    it("severity 없는 ok/info 항목은 심각도 그룹에 포함되지 않는다", async () => {
      const dataWithOkOnly = {
        sections: [
          {
            name: "ZIP 구조",
            results: [
              { type: "info", text: "ZIP 파일 수: 30" },
              { type: "ok", text: "mimetype: application/epub+zip" },
            ],
          },
        ],
        summary: { fatal: 0, errors: 0, warnings: 0 },
      };
      setupMocks({ frontendData: dataWithOkOnly });
      render(<EpubDiagnoseView bookId={1} fileType="epub" />);

      await act(async () => {
        fireEvent.click(screen.getByText(/파일 정합성 진단/));
      });

      await waitFor(() => {
        expect(screen.getByText("PASS")).toBeTruthy();
        expect(screen.getByText("이상 없음")).toBeTruthy();
        // info/ok 텍스트는 심각도 테이블에 나오지 않음
        expect(screen.queryByText("ZIP 파일 수: 30")).toBeNull();
        expect(screen.queryByText("mimetype: application/epub+zip")).toBeNull();
      });
    });
  });
});

// ─── 취소/에러/로딩 경로 ───

describe("EpubDiagnoseView 취소 및 에러 경로", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("bookId 가 없으면 카드를 열어도 진단을 실행하지 않는다", async () => {
    setupMocks();
    render(<EpubDiagnoseView fileType="epub" />);

    await act(async () => {
      fireEvent.click(screen.getByText(/파일 정합성 진단/));
    });

    expect(jsonGetReq).not.toHaveBeenCalled();
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it("요청이 진행 중이면 스피너를 표시한다", async () => {
    // resolve/reject 를 호출하지 않아 로딩 상태가 유지된다
    jsonGetReq.mockImplementation(() => {});
    globalThis.fetch = vi.fn(() => new Promise(() => {}));

    render(<EpubDiagnoseView bookId={1} fileType="epub" />);
    await act(async () => {
      fireEvent.click(screen.getByText(/파일 정합성 진단/));
    });

    await waitFor(() => {
      expect(document.querySelectorAll(".spinner-border").length).toBe(2);
    });
  });

  it("다운로드 응답이 실패면 서버 응답 오류 메시지를 표시한다", async () => {
    setupMocks();
    globalThis.fetch = vi.fn(() => Promise.resolve({ ok: false, status: 502 }));

    render(<EpubDiagnoseView bookId={1} fileType="epub" />);
    await act(async () => {
      fireEvent.click(screen.getByText(/파일 정합성 진단/));
    });

    await waitFor(() => {
      expect(screen.getByText("서버 응답 오류: 502")).toBeTruthy();
    });
  });

  it("AbortError 는 에러 메시지로 표시하지 않는다", async () => {
    setupMocks();
    const abortErr = new Error("aborted");
    abortErr.name = "AbortError";
    globalThis.fetch = vi.fn(() => Promise.reject(abortErr));

    render(<EpubDiagnoseView bookId={1} fileType="epub" />);
    await act(async () => {
      fireEvent.click(screen.getByText(/파일 정합성 진단/));
    });

    await waitFor(() => {
      expect(screen.getByText(/Frontend 진단/)).toBeTruthy();
    });
    expect(screen.queryByText("aborted")).toBeNull();
  });

  it("언마운트 후 도착한 backend 응답은 무시한다", async () => {
    let backendResolve;
    let backendReject;
    jsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      backendResolve = resolve;
      backendReject = reject;
    });
    globalThis.fetch = vi.fn(() => new Promise(() => {}));

    const { unmount } = render(<EpubDiagnoseView bookId={1} fileType="epub" />);
    await act(async () => {
      fireEvent.click(screen.getByText(/파일 정합성 진단/));
    });

    unmount(); // abortRef.current.abort() 실행
    // 이제 도착한 콜백은 signal.aborted 로 조기 반환된다 (경고 없이 통과해야 함)
    await act(async () => {
      backendResolve({ valid: true, messages: [] });
      backendReject("late error");
    });
  });

  it("언마운트 후 도착한 frontend 결과는 무시한다", async () => {
    setupMocks();
    let releaseBuffer;
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        arrayBuffer: () => new Promise((res) => (releaseBuffer = res)),
      }),
    );

    const { unmount } = render(<EpubDiagnoseView bookId={1} fileType="epub" />);
    await act(async () => {
      fireEvent.click(screen.getByText(/파일 정합성 진단/));
    });

    unmount();
    await act(async () => {
      releaseBuffer(new ArrayBuffer(8));
    });
  });

  it("severity/message 가 없는 backend 메시지도 INFO 로 묶어 표시한다", async () => {
    setupMocks({
      backendData: {
        valid: false,
        file_path: "books/x.epub",
        messages: [{ id: "NO-SEV" }],
        summary: { fatal: 0, error: 0, warning: 0, usage: 0, info: 1 },
      },
    });

    render(<EpubDiagnoseView bookId={1} fileType="epub" />);
    await act(async () => {
      fireEvent.click(screen.getByText(/파일 정합성 진단/));
    });

    await waitFor(() => {
      expect(screen.getByText("INFO")).toBeTruthy();
    });
    expect(screen.getByText(/1건 — 참고 정보/)).toBeTruthy();
  });
});
