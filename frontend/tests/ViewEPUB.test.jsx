// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, cleanup, waitFor } from '@testing-library/react';

afterEach(cleanup);

// ReactReader mock - autoLoad=true이면 즉시 locationChanged 호출
let autoLoad = true;
const mockReactReader = vi.fn(({ url, locationChanged }) => {
    if (url && autoLoad) {
        setTimeout(() => locationChanged?.('epubcfi(/1)'), 0);
    }
    return <div data-testid="react-reader">ReactReader</div>;
});

vi.mock('react-reader', () => ({
    ReactReader: (props) => mockReactReader(props),
}));

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

// fetch mock: ArrayBuffer를 반환
const mockArrayBuffer = new ArrayBuffer(8);
beforeEach(() => {
    mockReactReader.mockClear();
    globalThis.fetch = vi.fn(() =>
        Promise.resolve({
            ok: true,
            arrayBuffer: () => Promise.resolve(mockArrayBuffer),
        })
    );
});

afterEach(() => {
    vi.restoreAllMocks();
});

import ViewEPUB from '../src/ViewEPUB';

describe('ViewEPUB', () => {
    beforeEach(() => {
        mockReactReader.mockClear();
        autoLoad = true;
    });

    // ── 유효성 검사 ──

    it('bookId가 없으면 에러를 표시한다', () => {
        render(<ViewEPUB bookId={0} />);
        expect(screen.getByText(/유효한 bookId 또는 filePath가 제공되지 않았습니다/)).toBeTruthy();
    });

    it('filePath 없이 preview=false이면 에러를 표시한다', () => {
        render(<ViewEPUB bookId={1} />);
        expect(screen.getByText(/유효한 bookId 또는 filePath가 제공되지 않았습니다/)).toBeTruthy();
    });

    // ── fetch URL 생성 ──

    it('preview=true이면 /preview/ URL로 fetch한다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=3',
            expect.objectContaining({ signal: expect.any(AbortSignal) })
        );
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalledWith(
                expect.objectContaining({ url: mockArrayBuffer })
            );
        });
    });

    it('preview=false이면 /download/ URL로 fetch한다', async () => {
        render(<ViewEPUB bookId={42} filePath="test/book.epub" />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/download/42/test%2Fbook.epub',
            expect.objectContaining({ signal: expect.any(AbortSignal) })
        );
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalledWith(
                expect.objectContaining({ url: mockArrayBuffer })
            );
        });
    });

    it('bookId 변경 시 새로 fetch한다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} filePath="a.epub" />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/download/1/a.epub',
                expect.any(Object)
            );
        });

        rerender(<ViewEPUB bookId={2} filePath="b.epub" />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/download/2/b.epub',
                expect.any(Object)
            );
        });
    });

    // ── 컨테이너 높이 ──

    it('preview=true이면 컨테이너 높이가 60vh이다', () => {
        const { container } = render(<ViewEPUB bookId={1} preview={true} />);
        const div = container.firstChild;
        expect(div.style.height).toBe('60vh');
    });

    it('preview=false이면 컨테이너 높이가 100vh이다', () => {
        const { container } = render(<ViewEPUB bookId={1} filePath="a.epub" />);
        const div = container.firstChild;
        expect(div.style.height).toBe('100vh');
    });

    // ── 로딩 상태 ──

    it('초기 로딩 시 스피너를 표시한다', () => {
        render(<ViewEPUB bookId={1} filePath="a.epub" />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    // ── 로딩 타임아웃 ──

    it('30초 내 로딩 미완료 시 타임아웃 에러를 표시한다', async () => {
        vi.useFakeTimers();
        autoLoad = false;

        render(<ViewEPUB bookId={1} filePath="a.epub" />);

        // fetch의 Promise를 resolve시키기 위해 타이머 진행
        await act(async () => { vi.advanceTimersByTime(1); });

        // epubData 설정 후 타임아웃 시작, 30초 경과
        await act(async () => { vi.advanceTimersByTime(30000); });

        expect(screen.getByText('미리보기 로딩 시간이 초과되었습니다.')).toBeTruthy();
        expect(screen.queryByText('로딩 중...')).toBeNull();

        vi.useRealTimers();
    });

    it('로딩 완료 후에는 타임아웃이 발생하지 않는다', async () => {
        vi.useFakeTimers();
        autoLoad = true;

        render(<ViewEPUB bookId={1} filePath="a.epub" />);

        // fetch resolve + locationChanged 콜백 실행
        await act(async () => { vi.advanceTimersByTime(1); });

        // 30초 경과해도 에러 없음
        act(() => { vi.advanceTimersByTime(30000); });
        expect(screen.queryByText('미리보기 로딩 시간이 초과되었습니다.')).toBeNull();

        vi.useRealTimers();
    });

    // ── fetch 에러 ──

    it('fetch 실패 시 에러 메시지를 표시한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve({ ok: false, status: 500 })
        );
        render(<ViewEPUB bookId={1} filePath="a.epub" />);
        await waitFor(() => {
            expect(screen.getByText(/EPUB 로딩 실패/)).toBeTruthy();
        });
    });
});
