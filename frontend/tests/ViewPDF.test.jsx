// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

// IntersectionObserver mock (jsdom에 없으므로 직접 구현)
globalThis.IntersectionObserver = class MockIntersectionObserver {
    constructor(callback) {
        this.callback = callback;
        this.elements = new Set();
    }
    observe(element) {
        this.elements.add(element);
        // 즉시 visible 처리 → 테스트에서 모든 페이지가 바로 렌더링됨
        this.callback([{isIntersecting: true, target: element}], this);
    }
    unobserve(element) {
        this.elements.delete(element);
    }
    disconnect() {
        this.elements.clear();
    }
};

const { mockGetDocument } = vi.hoisted(() => ({
    mockGetDocument: vi.fn(),
}));

vi.mock('pdfjs-dist', () => ({
    getDocument: mockGetDocument,
    GlobalWorkerOptions: { workerSrc: '' },
}));

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

import ViewPDF from '../src/ViewPDF';

function createDeferred() {
    let resolve, reject;
    const promise = new Promise((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

function createMockPdf(numPages = 2) {
    return {
        numPages,
        getPage: vi.fn(() => Promise.resolve({
            getViewport: () => ({ width: 800, height: 600 }),
            render: () => ({ promise: Promise.resolve() }),
        })),
        destroy: vi.fn(),
    };
}

// pdfjs loadingTask mock 헬퍼
function createMockLoadingTask(promiseOrPdf) {
    const promise = promiseOrPdf.then ? promiseOrPdf : Promise.resolve(promiseOrPdf);
    return {
        promise,
        onProgress: null,
        destroy: vi.fn(),
    };
}

describe('ViewPDF', () => {
    beforeEach(() => {
        mockGetDocument.mockReset();
        HTMLCanvasElement.prototype.getContext = vi.fn(() => ({}));
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    // ── 초기 상태 / 유효성 검사 ──

    it('bookId가 0이면 에러를 표시한다', () => {
        render(<ViewPDF bookId={0} />);
        expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
    });

    it('로딩 중 스피너와 "PDF 로딩 중..." 메시지를 표시한다', () => {
        mockGetDocument.mockReturnValue(createMockLoadingTask(new Promise(() => {})));

        render(<ViewPDF bookId={1} />);

        expect(screen.getByText('PDF 로딩 중...')).toBeTruthy();
        expect(document.querySelector('.spinner')).toBeTruthy();
    });

    // ── URL 및 getDocument 호출 ──

    it('올바른 URL로 getDocument를 호출한다', () => {
        mockGetDocument.mockReturnValue(createMockLoadingTask(new Promise(() => {})));

        render(<ViewPDF bookId={42} />);

        expect(mockGetDocument).toHaveBeenCalledWith({url: 'http://localhost:8000/download/42'});
    });

    it('bookId 변경 시 새 URL로 getDocument를 호출한다', async () => {
        mockGetDocument.mockReturnValue(createMockLoadingTask(new Promise(() => {})));

        const { rerender } = render(<ViewPDF bookId={1} />);
        expect(mockGetDocument).toHaveBeenCalledWith({url: 'http://localhost:8000/download/1'});

        rerender(<ViewPDF bookId={99} />);
        expect(mockGetDocument).toHaveBeenCalledWith({url: 'http://localhost:8000/download/99'});
    });

    it('preview=true이면 preview URL로 getDocument를 호출한다', () => {
        mockGetDocument.mockReturnValue(createMockLoadingTask(new Promise(() => {})));

        render(<ViewPDF bookId={42} preview={true} pageCount={3} />);

        expect(mockGetDocument).toHaveBeenCalledWith({url: 'http://localhost:8000/preview/42?pages=3'});
    });

    // ── 정상 렌더링 ──

    it('PDF 로드 성공 후 모든 페이지를 렌더링한다', async () => {
        const mockPdf = createMockPdf(3);
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 3쪽 표시')).toBeTruthy();
        });
    });

    it('각 페이지마다 canvas 요소를 생성한다', async () => {
        const mockPdf = createMockPdf(2);
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 2쪽 표시')).toBeTruthy();
        });

        const canvases = document.querySelectorAll('canvas');
        expect(canvases.length).toBe(2);
    });

    it('렌더링 진행 중 "렌더링 중... X/Y쪽" 상태를 표시한다', async () => {
        // getPage는 즉시 resolve하되, render의 promise는 영원히 pending
        const mockPdf = {
            numPages: 2,
            getPage: vi.fn(() => Promise.resolve({
                getViewport: () => ({ width: 800, height: 600 }),
                render: () => ({ promise: new Promise(() => {}) }),
            })),
            destroy: vi.fn(),
        };
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('렌더링 중... 0/2쪽')).toBeTruthy();
        });
    });

    // ── pageCount 제한 ──

    it('pageCount로 렌더링 페이지 수를 제한한다', async () => {
        const mockPdf = createMockPdf(10);
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} pageCount={3} />);

        await waitFor(() => {
            expect(screen.getByText('총 3쪽 표시')).toBeTruthy();
        });
    });

    it('pageCount가 numPages보다 크면 numPages만큼 렌더링한다', async () => {
        const mockPdf = createMockPdf(2);
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} pageCount={100} />);

        await waitFor(() => {
            expect(screen.getByText('총 2쪽 표시')).toBeTruthy();
        });
    });

    it('pageCount=0이면 모든 페이지를 렌더링한다', async () => {
        const mockPdf = createMockPdf(5);
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} pageCount={0} />);

        await waitFor(() => {
            expect(screen.getByText('총 5쪽 표시')).toBeTruthy();
        });
    });

    // ── 에러 처리 ──

    it('getDocument 실패 시 에러 메시지를 표시한다', async () => {
        mockGetDocument.mockReturnValue(createMockLoadingTask(Promise.reject(new Error('Invalid PDF structure'))));

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText(/Invalid PDF structure/)).toBeTruthy();
        });
    });

    it('err.message가 없으면 fallback 에러 메시지를 표시한다', async () => {
        mockGetDocument.mockReturnValue(createMockLoadingTask(Promise.reject({ name: 'UnknownError' })));

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText(/파일이 존재하지 않거나 올바르지 않은 형식/)).toBeTruthy();
        });
    });

    it('개별 페이지 렌더링 실패 시 전체 에러 상태가 되지 않는다', async () => {
        const mockPdf = {
            numPages: 3,
            getPage: vi.fn((pageNum) => {
                if (pageNum === 2) {
                    return Promise.reject(new Error('page 2 corrupt'));
                }
                return Promise.resolve({
                    getViewport: () => ({ width: 800, height: 600 }),
                    render: () => ({ promise: Promise.resolve() }),
                });
            }),
            destroy: vi.fn(),
        };
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            const canvases = document.querySelectorAll('canvas');
            expect(canvases.length).toBe(3);
        });

        // 에러 메시지가 화면에 표시되지 않음
        expect(screen.queryByText(/렌더링 실패/)).toBeNull();
    });

    // ── cleanup / 취소 ──

    it('cleanup 시 진행 중인 loadingTask.destroy()를 호출한다', async () => {
        const deferred = createDeferred();
        const loadingTask = createMockLoadingTask(deferred.promise);
        mockGetDocument.mockReturnValue(loadingTask);

        const { unmount } = render(<ViewPDF bookId={1} />);
        await waitFor(() => {
            expect(mockGetDocument).toHaveBeenCalled();
        });
        unmount();

        expect(loadingTask.destroy).toHaveBeenCalledTimes(1);
    });

    it('로딩 완료 후 cleanup에서 loadingTask.destroy()를 호출하지 않는다', async () => {
        const mockPdf = createMockPdf(1);
        const loadingTask = createMockLoadingTask(mockPdf);
        mockGetDocument.mockReturnValue(loadingTask);

        const { unmount } = render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 1쪽 표시')).toBeTruthy();
        });

        unmount();

        expect(loadingTask.destroy).not.toHaveBeenCalled();
        expect(mockPdf.destroy).toHaveBeenCalled();
    });

    it('bookId 변경 시 이전 PDF를 destroy한다', async () => {
        const mockPdf1 = createMockPdf(1);
        const mockPdf2 = createMockPdf(1);

        mockGetDocument
            .mockReturnValueOnce(createMockLoadingTask(mockPdf1))
            .mockReturnValueOnce(createMockLoadingTask(mockPdf2));

        const { rerender } = render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 1쪽 표시')).toBeTruthy();
        });

        rerender(<ViewPDF bookId={2} />);

        await waitFor(() => {
            expect(mockPdf1.destroy).toHaveBeenCalled();
        });
    });

    it('bookId 변경 시 이전 로딩의 에러가 표시되지 않는다', async () => {
        const deferred1 = createDeferred();
        const deferred2 = createDeferred();
        const mockPdf2 = createMockPdf(1);

        mockGetDocument
            .mockReturnValueOnce(createMockLoadingTask(deferred1.promise))
            .mockReturnValueOnce(createMockLoadingTask(deferred2.promise));

        const { rerender } = render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(mockGetDocument).toHaveBeenCalledTimes(1);
        });

        rerender(<ViewPDF bookId={2} />);

        await waitFor(() => {
            expect(mockGetDocument).toHaveBeenCalledTimes(2);
        });

        deferred1.reject(new Error('First load failed'));
        deferred2.resolve(mockPdf2);

        await waitFor(() => {
            expect(screen.getByText('총 1쪽 표시')).toBeTruthy();
        });

        expect(screen.queryByText(/First load failed/)).toBeNull();
    });

    // ── onProgress ──

    it('onProgress 콜백으로 다운로드 진행률을 업데이트한다', async () => {
        const loadingTask = createMockLoadingTask(new Promise(() => {}));
        mockGetDocument.mockReturnValue(loadingTask);

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(loadingTask.onProgress).toBeTypeOf('function');
        });

        // onProgress 콜백 호출 시뮬레이션
        loadingTask.onProgress({ loaded: 50, total: 100 });

        await waitFor(() => {
            expect(screen.getByText(/50%/)).toBeTruthy();
        });
    });

    it('onProgress의 total이 0이면 진행률을 표시하지 않는다', async () => {
        const loadingTask = createMockLoadingTask(new Promise(() => {}));
        mockGetDocument.mockReturnValue(loadingTask);

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(loadingTask.onProgress).toBeTypeOf('function');
        });

        loadingTask.onProgress({ loaded: 100, total: 0 });

        // total이 0이면 진행률 표시 없이 "PDF 로딩 중..." 유지
        expect(screen.getByText('PDF 로딩 중...')).toBeTruthy();
    });

    // ── preview 모드 렌더링 ──

    it('preview=true이면 PDF의 모든 페이지를 렌더링한다', async () => {
        const mockPdf = createMockPdf(5);
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} preview={true} />);

        await waitFor(() => {
            expect(screen.getByText('총 5쪽 표시')).toBeTruthy();
        });
    });

    it('preview=false이고 pageCount가 있으면 pageCount만큼만 렌더링한다', async () => {
        const mockPdf = createMockPdf(10);
        mockGetDocument.mockReturnValue(createMockLoadingTask(mockPdf));

        render(<ViewPDF bookId={1} preview={false} pageCount={3} />);

        await waitFor(() => {
            expect(screen.getByText('총 3쪽 표시')).toBeTruthy();
        });
    });
});
