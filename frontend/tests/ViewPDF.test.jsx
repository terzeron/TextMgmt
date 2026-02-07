// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

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

    it('bookId 변경 시 이전 로딩의 에러가 표시되지 않는다', async () => {
        const deferred1 = createDeferred();
        const deferred2 = createDeferred();
        const mockPdf2 = createMockPdf(1);

        mockGetDocument
            .mockReturnValueOnce({ promise: deferred1.promise, destroy: vi.fn() })
            .mockReturnValueOnce({ promise: deferred2.promise, destroy: vi.fn() });

        const { rerender } = render(<ViewPDF bookId={1} />);

        // bookId 변경 → 이전 로딩 cancelled=true
        rerender(<ViewPDF bookId={2} />);

        // 이전 로딩 실패 (cancelled 상태이므로 setError 호출 안 됨)
        deferred1.reject(new Error('First load failed'));
        // 새 로딩 성공
        deferred2.resolve(mockPdf2);

        await waitFor(() => {
            expect(screen.getByText('총 1쪽 표시')).toBeTruthy();
        });

        expect(screen.queryByText(/First load failed/)).toBeNull();
    });

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

    it('PDF 로드 성공 후 페이지를 렌더링한다', async () => {
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

    it('bookId가 없으면 에러를 표시한다', () => {
        render(<ViewPDF bookId={0} />);
        expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
    });
});
