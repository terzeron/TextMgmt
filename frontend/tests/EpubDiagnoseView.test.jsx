// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, cleanup, waitFor, fireEvent } from '@testing-library/react';

afterEach(cleanup);

// ─── 모킹 ───

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
    jsonGetReq: vi.fn(),
}));

vi.mock('../src/EpubDiagnose', () => ({
    diagnoseEpub: vi.fn(),
}));

import { jsonGetReq } from '../src/Common';
import { diagnoseEpub } from '../src/EpubDiagnose';
import EpubDiagnoseView from '../src/EpubDiagnoseView';

const MOCK_BACKEND_DATA = {
    valid: true,
    file_path: 'books/test.epub',
    messages: [],
    summary: { fatal: 0, error: 0, warning: 0, usage: 0, info: 0 },
    publication: { title: 'Test Book', creator: 'Author', publisher: 'Publisher' },
};

const MOCK_BACKEND_DATA_INVALID = {
    valid: false,
    file_path: 'books/bad.epub',
    messages: [
        { severity: 'ERROR', id: 'OPF-001', message: 'Missing required element', location: { path: 'content.opf', line: 5, column: 10 } },
        { severity: 'WARNING', id: 'CSS-001', message: 'CSS font-face issue', location: { path: 'style.css', line: 1, column: 1 } },
    ],
    summary: { fatal: 0, error: 1, warning: 1, usage: 0, info: 0 },
};

const MOCK_BACKEND_DATA_FATAL = {
    valid: false,
    file_path: 'books/corrupt.epub',
    messages: [
        { severity: 'FATAL', id: 'PKG-004', message: 'Corrupted EPUB ZIP header', location: { path: 'corrupt.epub', line: -1, column: -1 } },
        { severity: 'ERROR', id: 'OPF-030', message: 'Unique identifier not found', location: { path: 'content.opf', line: 3, column: 1 } },
    ],
    summary: { fatal: 1, error: 1, warning: 0, usage: 0, info: 0 },
};

const MOCK_FRONTEND_DATA = {
    sections: [
        { name: 'ZIP 구조', results: [{ type: 'ok', text: 'mimetype: application/epub+zip' }] },
        { name: 'OPF 파싱', results: [{ type: 'ok', text: '브라우저 DOMParser 파싱: 정상' }] },
    ],
    summary: { fatal: 0, errors: 0, warnings: 0 },
};

const MOCK_FRONTEND_DATA_WITH_ERRORS = {
    sections: [
        { name: 'ZIP 구조', results: [{ type: 'ok', text: 'mimetype: application/epub+zip' }] },
        { name: 'OPF 파싱', results: [{ type: 'error', severity: 'FATAL', text: '브라우저 DOMParser 파싱 실패' }] },
    ],
    summary: { fatal: 1, errors: 0, warnings: 0 },
};

const MOCK_FRONTEND_DATA_MIXED = {
    sections: [
        { name: 'ZIP 구조', results: [{ type: 'ok', text: 'mimetype: application/epub+zip' }] },
        { name: 'Spine 파일', results: [{ type: 'error', severity: 'ERROR', text: 'spine "ch2" → OEBPS/ch2.xhtml ZIP에 없음' }] },
        { name: 'OPF 파싱', results: [{ type: 'warn', severity: 'WARNING', text: 'dc:language 없음' }] },
    ],
    summary: { fatal: 0, errors: 1, warnings: 1 },
};

function setupMocks({ backendData = MOCK_BACKEND_DATA, backendError = null, frontendData = MOCK_FRONTEND_DATA, frontendError = null } = {}) {
    jsonGetReq.mockImplementation((url, payload, resolve, reject) => {
        if (backendError) {
            reject(backendError);
        } else {
            resolve(backendData);
        }
    });

    globalThis.fetch = vi.fn(() => Promise.resolve({
        ok: true,
        arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)),
    }));

    if (frontendError) {
        diagnoseEpub.mockRejectedValue(new Error(frontendError));
    } else {
        diagnoseEpub.mockResolvedValue(frontendData);
    }
}

beforeEach(() => {
    vi.clearAllMocks();
    globalThis.fetch = vi.fn(() => Promise.resolve({
        ok: true,
        arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)),
    }));
});

afterEach(() => {
    vi.restoreAllMocks();
});

// ─── 테스트 ───

describe('EpubDiagnoseView', () => {

    describe('조건부 렌더링', () => {
        it('fileType이 epub이 아니면 렌더링하지 않는다', () => {
            const { container } = render(<EpubDiagnoseView bookId={1} fileType="pdf" />);
            expect(container.firstChild).toBeNull();
        });

        it('fileType이 epub이면 카드 헤더를 렌더링한다', () => {
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);
            expect(screen.getByText(/파일 정합성 진단/)).toBeTruthy();
        });

        it('초기 상태에서 카드 본문은 숨겨져 있다', () => {
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);
            expect(screen.queryByText(/Backend 진단/)).toBeNull();
        });
    });

    describe('카드 토글', () => {
        it('헤더 클릭 시 카드 본문이 열린다', async () => {
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

        it('헤더를 두 번 클릭하면 카드 본문이 닫힌다', async () => {
            setupMocks();
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });
            await waitFor(() => expect(screen.getByText(/Backend 진단/)).toBeTruthy());

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });
            expect(screen.queryByText(/Backend 진단/)).toBeNull();
        });
    });

    describe('진단 실행', () => {
        it('카드 열릴 때 백엔드 API를 호출한다', async () => {
            setupMocks();
            render(<EpubDiagnoseView bookId={42} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            expect(jsonGetReq).toHaveBeenCalledWith(
                '/validate/42',
                null,
                expect.any(Function),
                expect.any(Function)
            );
        });

        it('카드 열릴 때 프론트엔드 진단용 fetch를 호출한다', async () => {
            setupMocks();
            render(<EpubDiagnoseView bookId={42} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/download/42',
                expect.objectContaining({ signal: expect.any(AbortSignal) })
            );
        });

        it('이미 진단이 실행됐으면 카드를 닫았다 열어도 재실행하지 않는다', async () => {
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
    });

    describe('백엔드 결과 표시', () => {
        it('유효한 EPUB에서 VALID 배지를 표시한다', async () => {
            setupMocks();
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('VALID')).toBeTruthy();
            });
        });

        it('유효하지 않은 EPUB에서 INVALID 배지와 severity별 그룹 및 메시지를 모두 표시한다', async () => {
            setupMocks({ backendData: MOCK_BACKEND_DATA_INVALID });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('INVALID')).toBeTruthy();
                // severity 그룹 헤더
                expect(screen.getByText('ERROR')).toBeTruthy();
                expect(screen.getByText(/1건 — 스펙 위반/)).toBeTruthy();
                expect(screen.getByText('WARNING')).toBeTruthy();
                expect(screen.getByText(/1건 — 권장사항/)).toBeTruthy();
                // 모든 메시지가 바로 표시됨
                expect(screen.getByText(/Missing required element/)).toBeTruthy();
                expect(screen.getByText(/CSS font-face issue/)).toBeTruthy();
            });
        });

        it('FATAL 메시지도 바로 표시된다', async () => {
            setupMocks({ backendData: MOCK_BACKEND_DATA_FATAL });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('FATAL')).toBeTruthy();
                expect(screen.getByText(/Corrupted EPUB ZIP header/)).toBeTruthy();
                expect(screen.getByText('ERROR')).toBeTruthy();
                expect(screen.getByText(/Unique identifier not found/)).toBeTruthy();
            });
        });

        it('백엔드 에러 시 에러 메시지를 표시한다', async () => {
            setupMocks({ backendError: 'epubcheck is not installed' });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('epubcheck is not installed')).toBeTruthy();
            });
        });

        it('publication 메타데이터를 표시한다', async () => {
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

    describe('프론트엔드 결과 표시', () => {
        it('이상 없을 때 PASS 배지와 이상 없음을 표시한다', async () => {
            setupMocks();
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('PASS')).toBeTruthy();
                expect(screen.getByText('이상 없음')).toBeTruthy();
            });
        });

        it('FATAL 에러 시 FAIL 배지와 심각도 그룹을 표시한다', async () => {
            setupMocks({ frontendData: MOCK_FRONTEND_DATA_WITH_ERRORS });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('FAIL')).toBeTruthy();
                expect(screen.getByText('FATAL')).toBeTruthy();
                expect(screen.getByText(/1건 — 렌더링 불가/)).toBeTruthy();
                expect(screen.getByText(/브라우저 DOMParser 파싱 실패/)).toBeTruthy();
            });
        });

        it('ERROR/WARNING 혼합 시 심각도별 그룹을 모두 표시한다', async () => {
            setupMocks({ frontendData: MOCK_FRONTEND_DATA_MIXED });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('ERROR')).toBeTruthy();
                expect(screen.getByText(/1건 — 스펙 위반/)).toBeTruthy();
                expect(screen.getByText(/spine.*ZIP에 없음/)).toBeTruthy();
                expect(screen.getByText('WARNING')).toBeTruthy();
                expect(screen.getByText(/1건 — 권장사항/)).toBeTruthy();
                expect(screen.getByText(/dc:language 없음/)).toBeTruthy();
            });
        });

        it('프론트엔드 진단 에러 시 에러 메시지를 표시한다', async () => {
            setupMocks({ frontendError: '서버 응답 오류: 500' });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('서버 응답 오류: 500')).toBeTruthy();
            });
        });
    });

    describe('bookId 변경', () => {
        it('bookId 변경 시 상태가 초기화된다', async () => {
            setupMocks();
            const { rerender } = render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            // 카드 열기 + 데이터 로드
            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });
            await waitFor(() => expect(screen.getByText('VALID')).toBeTruthy());

            // bookId 변경
            await act(async () => {
                rerender(<EpubDiagnoseView bookId={2} fileType="epub" />);
            });

            // 카드가 닫혀야 하고, 이전 데이터가 없어야 함
            expect(screen.queryByText('VALID')).toBeNull();
            expect(screen.queryByText(/Backend 진단/)).toBeNull();
        });
    });

    describe('fetch 취소', () => {
        it('bookId 변경 시 이전 fetch의 AbortController가 abort된다', async () => {
            setupMocks();

            // fetch를 지연시킨다
            let fetchResolve;
            globalThis.fetch = vi.fn(() => new Promise(resolve => { fetchResolve = resolve; }));

            const { rerender } = render(<EpubDiagnoseView bookId={1} fileType="epub" />);

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

    describe('프론트엔드 심각도 표시', () => {
        it('심각도 그룹에 섹션명이 location으로 표시된다', async () => {
            setupMocks({ frontendData: MOCK_FRONTEND_DATA_MIXED });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                // ERROR 그룹의 location에 Spine 파일 섹션명이 표시됨
                expect(screen.getByText('Spine 파일')).toBeTruthy();
                // WARNING 그룹의 location에 OPF 파싱 섹션명이 표시됨
                expect(screen.getByText('OPF 파싱')).toBeTruthy();
            });
        });

        it('FATAL만 있을 때 FAIL 배지와 FATAL 그룹이 표시된다', async () => {
            setupMocks({ frontendData: MOCK_FRONTEND_DATA_WITH_ERRORS });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('FAIL')).toBeTruthy();
                expect(screen.getByText('FATAL')).toBeTruthy();
                // ERROR/WARNING 그룹은 없음
                expect(screen.queryByText(/스펙 위반/)).toBeNull();
                expect(screen.queryByText(/권장사항 미준수/)).toBeNull();
            });
        });

        it('severity 없는 ok/info 항목은 심각도 그룹에 포함되지 않는다', async () => {
            const dataWithOkOnly = {
                sections: [
                    { name: 'ZIP 구조', results: [
                        { type: 'info', text: 'ZIP 파일 수: 30' },
                        { type: 'ok', text: 'mimetype: application/epub+zip' },
                    ] },
                ],
                summary: { fatal: 0, errors: 0, warnings: 0 },
            };
            setupMocks({ frontendData: dataWithOkOnly });
            render(<EpubDiagnoseView bookId={1} fileType="epub" />);

            await act(async () => {
                fireEvent.click(screen.getByText(/파일 정합성 진단/));
            });

            await waitFor(() => {
                expect(screen.getByText('PASS')).toBeTruthy();
                expect(screen.getByText('이상 없음')).toBeTruthy();
                // info/ok 텍스트는 심각도 테이블에 나오지 않음
                expect(screen.queryByText('ZIP 파일 수: 30')).toBeNull();
                expect(screen.queryByText('mimetype: application/epub+zip')).toBeNull();
            });
        });
    });
});
