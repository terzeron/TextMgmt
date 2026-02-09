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
        expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
    });

    it('bookId가 없으면 fetch를 호출하지 않는다', () => {
        render(<ViewEPUB bookId={0} />);
        expect(globalThis.fetch).not.toHaveBeenCalled();
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

    it('preview=false이면 /download/{bookId}만으로 fetch한다', async () => {
        render(<ViewEPUB bookId={42} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/download/42',
            expect.objectContaining({ signal: expect.any(AbortSignal) })
        );
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalledWith(
                expect.objectContaining({ url: mockArrayBuffer })
            );
        });
    });

    // ── %2F 회귀 방지 (버그 #1 핵심 회귀 테스트) ──

    it('download URL에 파일 경로나 %2F가 포함되지 않는다', () => {
        render(<ViewEPUB bookId={42} />);
        const fetchUrl = globalThis.fetch.mock.calls[0][0];
        expect(fetchUrl).toBe('http://localhost:8000/download/42');
        expect(fetchUrl).not.toContain('%2F');
        expect(fetchUrl).not.toMatch(/\/download\/\d+\/.+/);
    });

    it('한글·특수문자가 포함된 경로여도 URL에 인코딩 문제가 없다', () => {
        // 이전에는 filePath가 URL에 포함되어 %2F, 괄호 등이 프록시 400을 유발했음
        // 이제 bookId만 사용하므로 filePath에 무관하게 안전한 URL 생성
        render(<ViewEPUB bookId={381881} />);
        const fetchUrl = globalThis.fetch.mock.calls[0][0];
        expect(fetchUrl).toBe('http://localhost:8000/download/381881');
    });

    // ── bookId / preview 변경 ──

    it('bookId 변경 시 새로 fetch한다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/download/1',
                expect.any(Object)
            );
        });

        rerender(<ViewEPUB bookId={2} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/download/2',
                expect.any(Object)
            );
        });
    });

    it('preview 전환 시 URL 패턴이 변경된다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} preview={true} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/1?chapters=3',
            expect.any(Object)
        );

        globalThis.fetch.mockClear();
        rerender(<ViewEPUB bookId={1} preview={false} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/download/1',
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
        const { container } = render(<ViewEPUB bookId={1} />);
        const div = container.firstChild;
        expect(div.style.height).toBe('100vh');
    });

    // ── 로딩 상태 ──

    it('초기 로딩 시 스피너를 표시한다', () => {
        render(<ViewEPUB bookId={1} />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    // ── 로딩 타임아웃 ──

    it('30초 내 로딩 미완료 시 타임아웃 에러를 표시한다', async () => {
        vi.useFakeTimers();
        autoLoad = false;

        render(<ViewEPUB bookId={1} />);

        await act(async () => { vi.advanceTimersByTime(1); });
        await act(async () => { vi.advanceTimersByTime(30000); });

        expect(screen.getByText('미리보기 로딩 시간이 초과되었습니다.')).toBeTruthy();
        expect(screen.queryByText('로딩 중...')).toBeNull();

        vi.useRealTimers();
    });

    it('로딩 완료 후에는 타임아웃이 발생하지 않는다', async () => {
        vi.useFakeTimers();
        autoLoad = true;

        render(<ViewEPUB bookId={1} />);

        await act(async () => { vi.advanceTimersByTime(1); });

        act(() => { vi.advanceTimersByTime(30000); });
        expect(screen.queryByText('미리보기 로딩 시간이 초과되었습니다.')).toBeNull();

        vi.useRealTimers();
    });

    // ── fetch 에러 ──

    it('서버 에러(non-ok) 시 에러 메시지를 표시한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve({ ok: false, status: 500 })
        );
        render(<ViewEPUB bookId={1} />);
        await waitFor(() => {
            expect(screen.getByText(/EPUB 로딩 실패/)).toBeTruthy();
        });
    });

    it('네트워크 에러(TypeError: Failed to fetch) 시 에러 메시지를 표시한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.reject(new TypeError('Failed to fetch'))
        );
        render(<ViewEPUB bookId={1} />);
        await waitFor(() => {
            expect(screen.getByText(/EPUB 로딩 실패: Failed to fetch/)).toBeTruthy();
        });
    });

    it('AbortError는 에러 메시지로 표시하지 않는다', async () => {
        const abortError = new DOMException('The operation was aborted.', 'AbortError');
        globalThis.fetch = vi.fn(() => Promise.reject(abortError));
        render(<ViewEPUB bookId={1} />);

        // AbortError가 에러 메시지를 유발하지 않는 걸 확인하기 위해 짧은 대기
        await act(async () => { await new Promise(r => setTimeout(r, 50)); });
        expect(screen.queryByText(/EPUB 로딩 실패/)).toBeNull();
    });

    // ── 언마운트 시 cleanup ──

    it('언마운트 시 fetch가 abort된다', async () => {
        const abortSpy = vi.spyOn(AbortController.prototype, 'abort');
        const { unmount } = render(<ViewEPUB bookId={1} />);

        unmount();
        expect(abortSpy).toHaveBeenCalled();
    });
});
