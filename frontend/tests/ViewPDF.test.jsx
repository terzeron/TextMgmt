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

// fetch mock 헬퍼: ReadableStream을 반환하는 Response를 생성
function createMockFetchResponse(ok = true, status = 200) {
    const pdfBytes = new Uint8Array([0x25, 0x50, 0x44, 0x46]); // %PDF
    const stream = new ReadableStream({
        start(controller) {
            controller.enqueue(pdfBytes);
            controller.close();
        }
    });
    return {
        ok,
        status,
        statusText: ok ? 'OK' : 'Internal Server Error',
        headers: new Headers({'Content-Length': String(pdfBytes.length)}),
        body: { getReader: () => stream.getReader() },
    };
}

// fetch가 resolve되지 않는 pending 상태 Response
function createPendingFetchResponse() {
    const stream = new ReadableStream({
        start() { /* never closes */ }
    });
    return {
        ok: true,
        status: 200,
        statusText: 'OK',
        headers: new Headers({'Content-Length': '0'}),
        body: { getReader: () => stream.getReader() },
    };
}

describe('ViewPDF', () => {
    beforeEach(() => {
        mockGetDocument.mockReset();
        HTMLCanvasElement.prototype.getContext = vi.fn(() => ({}));
        vi.stubGlobal('fetch', vi.fn());
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
        fetch.mockReturnValue(new Promise(() => {})); // pending

        render(<ViewPDF bookId={1} />);

        expect(screen.getByText('PDF 로딩 중...')).toBeTruthy();
        expect(document.querySelector('.spinner')).toBeTruthy();
    });

    // ── URL 및 fetch 호출 ──

    it('올바른 URL로 fetch를 호출한다', () => {
        fetch.mockReturnValue(new Promise(() => {}));

        render(<ViewPDF bookId={42} />);

        expect(fetch).toHaveBeenCalledWith('http://localhost:8000/download/42', expect.objectContaining({signal: expect.any(AbortSignal)}));
    });

    it('bookId 변경 시 새 URL로 fetch를 호출한다', async () => {
        fetch.mockReturnValue(new Promise(() => {}));

        const { rerender } = render(<ViewPDF bookId={1} />);
        expect(fetch).toHaveBeenCalledWith('http://localhost:8000/download/1', expect.objectContaining({signal: expect.any(AbortSignal)}));

        rerender(<ViewPDF bookId={99} />);
        expect(fetch).toHaveBeenCalledWith('http://localhost:8000/download/99', expect.objectContaining({signal: expect.any(AbortSignal)}));
    });

    it('preview=true이면 preview URL로 fetch를 호출한다', () => {
        fetch.mockReturnValue(new Promise(() => {}));

        render(<ViewPDF bookId={42} preview={true} pageCount={3} />);

        expect(fetch).toHaveBeenCalledWith('http://localhost:8000/preview/42?pages=3', expect.objectContaining({signal: expect.any(AbortSignal)}));
    });

    // ── 정상 렌더링 ──

    it('PDF 로드 성공 후 모든 페이지를 렌더링한다', async () => {
        const mockPdf = createMockPdf(3);
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 3쪽 표시')).toBeTruthy();
        });
    });

    it('각 페이지마다 canvas 요소를 생성한다', async () => {
        const mockPdf = createMockPdf(2);
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

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
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('렌더링 중... 0/2쪽')).toBeTruthy();
        });
    });

    // ── pageCount 제한 ──

    it('pageCount로 렌더링 페이지 수를 제한한다', async () => {
        const mockPdf = createMockPdf(10);
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} pageCount={3} />);

        await waitFor(() => {
            expect(screen.getByText('총 3쪽 표시')).toBeTruthy();
        });
    });

    it('pageCount가 numPages보다 크면 numPages만큼 렌더링한다', async () => {
        const mockPdf = createMockPdf(2);
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} pageCount={100} />);

        await waitFor(() => {
            expect(screen.getByText('총 2쪽 표시')).toBeTruthy();
        });
    });

    it('pageCount=0이면 모든 페이지를 렌더링한다', async () => {
        const mockPdf = createMockPdf(5);
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} pageCount={0} />);

        await waitFor(() => {
            expect(screen.getByText('총 5쪽 표시')).toBeTruthy();
        });
    });

    // ── 에러 처리 ──

    it('fetch 실패 시 에러 메시지를 표시한다', async () => {
        fetch.mockResolvedValue(createMockFetchResponse(false, 500));

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText(/HTTP 500/)).toBeTruthy();
        });
    });

    it('getDocument 실패 시 에러 메시지를 표시한다', async () => {
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.reject(new Error('Invalid PDF structure')),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText(/Invalid PDF structure/)).toBeTruthy();
        });
    });

    it('err.message가 없으면 fallback 에러 메시지를 표시한다', async () => {
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.reject({ name: 'UnknownError' }),
            destroy: vi.fn(),
        });

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
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

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
        const destroyFn = vi.fn();
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: deferred.promise,
            destroy: destroyFn,
        });

        const { unmount } = render(<ViewPDF bookId={1} />);
        // fetch가 완료되고 getDocument가 호출될 때까지 대기
        await waitFor(() => {
            expect(mockGetDocument).toHaveBeenCalled();
        });
        unmount();

        expect(destroyFn).toHaveBeenCalledTimes(1);
    });

    it('로딩 완료 후 cleanup에서 loadingTask.destroy()를 호출하지 않는다', async () => {
        const mockPdf = createMockPdf(1);
        const loadingTaskDestroy = vi.fn();
        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: loadingTaskDestroy,
        });

        const { unmount } = render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 1쪽 표시')).toBeTruthy();
        });

        unmount();

        expect(loadingTaskDestroy).not.toHaveBeenCalled();
        expect(mockPdf.destroy).toHaveBeenCalled();
    });

    it('bookId 변경 시 이전 PDF를 destroy한다', async () => {
        const mockPdf1 = createMockPdf(1);
        const mockPdf2 = createMockPdf(1);

        fetch.mockResolvedValue(createMockFetchResponse());
        mockGetDocument
            .mockReturnValueOnce({ promise: Promise.resolve(mockPdf1), destroy: vi.fn() })
            .mockReturnValueOnce({ promise: Promise.resolve(mockPdf2), destroy: vi.fn() });

        const { rerender } = render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 1쪽 표시')).toBeTruthy();
        });

        rerender(<ViewPDF bookId={2} />);

        await waitFor(() => {
            expect(mockPdf1.destroy).toHaveBeenCalled();
        });
    });

    it('fetch abort 시 "PDF 로드 실패" 에러를 로깅하지 않는다', async () => {
        const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

        // fetch가 AbortSignal을 받으면 AbortError를 throw하도록 설정
        fetch.mockImplementation((url, opts) => {
            return new Promise((resolve, reject) => {
                if (opts?.signal) {
                    opts.signal.addEventListener('abort', () => {
                        reject(new DOMException('The operation was aborted.', 'AbortError'));
                    });
                }
            });
        });

        const { unmount } = render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(fetch).toHaveBeenCalled();
        });

        unmount();

        // AbortError rejection이 처리될 시간 대기
        await new Promise(r => setTimeout(r, 50));

        // "PDF 로드 실패" 메시지가 console.error로 출력되지 않아야 함
        const pdfLoadErrors = consoleError.mock.calls.filter(
            args => typeof args[0] === 'string' && args[0].includes('PDF 로드 실패')
        );
        expect(pdfLoadErrors).toHaveLength(0);

        consoleError.mockRestore();
    });

    it('bookId 변경 시 이전 로딩의 에러가 표시되지 않는다', async () => {
        const deferred1 = createDeferred();
        const deferred2 = createDeferred();
        const mockPdf2 = createMockPdf(1);

        // 각 호출마다 새 ReadableStream 생성
        fetch.mockImplementation(() => Promise.resolve(createMockFetchResponse()));
        mockGetDocument
            .mockReturnValueOnce({ promise: deferred1.promise, destroy: vi.fn() })
            .mockReturnValueOnce({ promise: deferred2.promise, destroy: vi.fn() });

        const { rerender } = render(<ViewPDF bookId={1} />);

        // fetch 완료 후 getDocument 호출 대기
        await waitFor(() => {
            expect(mockGetDocument).toHaveBeenCalledTimes(1);
        });

        rerender(<ViewPDF bookId={2} />);

        // 두 번째 getDocument 호출 대기
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
});
