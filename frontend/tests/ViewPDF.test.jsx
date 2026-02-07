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

describe('ViewPDF', () => {
    beforeEach(() => {
        mockGetDocument.mockReset();
        HTMLCanvasElement.prototype.getContext = vi.fn(() => ({}));
    });

    // ── 초기 상태 / 유효성 검사 ──

    it('bookId가 0이면 에러를 표시한다', () => {
        render(<ViewPDF bookId={0} />);
        expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
    });

    it('로딩 중 스피너와 "PDF 로딩 중..." 메시지를 표시한다', () => {
        const deferred = createDeferred();
        mockGetDocument.mockReturnValue({
            promise: deferred.promise,
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        expect(screen.getByText('PDF 로딩 중...')).toBeTruthy();
        expect(document.querySelector('.spinner')).toBeTruthy();
    });

    // ── URL 및 getDocument 호출 ──

    it('올바른 URL로 getDocument를 호출한다', () => {
        const deferred = createDeferred();
        mockGetDocument.mockReturnValue({
            promise: deferred.promise,
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={42} />);

        expect(mockGetDocument).toHaveBeenCalledWith(expect.objectContaining({
            url: 'http://localhost:8000/download/42',
        }));
    });

    it('bookId 변경 시 새 URL로 getDocument를 호출한다', async () => {
        const deferred1 = createDeferred();
        const deferred2 = createDeferred();

        mockGetDocument
            .mockReturnValueOnce({ promise: deferred1.promise, destroy: vi.fn() })
            .mockReturnValueOnce({ promise: deferred2.promise, destroy: vi.fn() });

        const { rerender } = render(<ViewPDF bookId={1} />);
        expect(mockGetDocument).toHaveBeenCalledWith(expect.objectContaining({
            url: 'http://localhost:8000/download/1',
        }));

        rerender(<ViewPDF bookId={99} />);
        expect(mockGetDocument).toHaveBeenCalledWith(expect.objectContaining({
            url: 'http://localhost:8000/download/99',
        }));
    });

    it('Range 요청 옵션이 올바르게 설정된다', () => {
        const deferred = createDeferred();
        mockGetDocument.mockReturnValue({
            promise: deferred.promise,
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        expect(mockGetDocument).toHaveBeenCalledWith(expect.objectContaining({
            rangeChunkSize: 65536,
            disableAutoFetch: true,
            disableRange: false,
        }));
    });

    // ── 정상 렌더링 ──

    it('PDF 로드 성공 후 모든 페이지를 렌더링한다', async () => {
        const mockPdf = createMockPdf(3);
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 3쪽 표시')).toBeTruthy();
        });
    });

    it('각 페이지에 페이지 번호를 표시한다', async () => {
        const mockPdf = createMockPdf(3);
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('총 3쪽 표시')).toBeTruthy();
        });

        expect(screen.getByText('1쪽')).toBeTruthy();
        expect(screen.getByText('2쪽')).toBeTruthy();
        expect(screen.getByText('3쪽')).toBeTruthy();
    });

    it('각 페이지마다 canvas 요소를 생성한다', async () => {
        const mockPdf = createMockPdf(2);
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

    it('에러 메시지에 실제 에러 내용을 포함한다', async () => {
        mockGetDocument.mockReturnValue({
            promise: Promise.reject(new Error('Network request failed')),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText(/Network request failed/)).toBeTruthy();
        });
    });

    it('err.message가 없으면 fallback 에러 메시지를 표시한다', async () => {
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
        mockGetDocument.mockReturnValue({
            promise: Promise.resolve(mockPdf),
            destroy: vi.fn(),
        });

        render(<ViewPDF bookId={1} />);

        await waitFor(() => {
            expect(document.querySelector('.pdf-pages')).toBeTruthy();
        });

        // 에러 메시지가 화면에 표시되지 않음
        expect(screen.queryByText(/렌더링 실패/)).toBeNull();
    });

    // ── cleanup / 취소 ──

    it('cleanup 시 진행 중인 loadingTask.destroy()를 호출한다', () => {
        const deferred = createDeferred();
        const destroyFn = vi.fn();
        mockGetDocument.mockReturnValue({
            promise: deferred.promise,
            destroy: destroyFn,
        });

        const { unmount } = render(<ViewPDF bookId={1} />);
        unmount();

        expect(destroyFn).toHaveBeenCalledTimes(1);
    });

    it('로딩 완료 후 cleanup에서 loadingTask.destroy()를 호출하지 않는다', async () => {
        const mockPdf = createMockPdf(1);
        const loadingTaskDestroy = vi.fn();
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

    it('bookId 변경 시 이전 로딩의 에러가 표시되지 않는다', async () => {
        const deferred1 = createDeferred();
        const deferred2 = createDeferred();
        const mockPdf2 = createMockPdf(1);

        mockGetDocument
            .mockReturnValueOnce({ promise: deferred1.promise, destroy: vi.fn() })
            .mockReturnValueOnce({ promise: deferred2.promise, destroy: vi.fn() });

        const { rerender } = render(<ViewPDF bookId={1} />);

        rerender(<ViewPDF bookId={2} />);

        deferred1.reject(new Error('First load failed'));
        deferred2.resolve(mockPdf2);

        await waitFor(() => {
            expect(screen.getByText('총 1쪽 표시')).toBeTruthy();
        });

        expect(screen.queryByText(/First load failed/)).toBeNull();
    });
});
