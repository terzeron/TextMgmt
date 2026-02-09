// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, cleanup, waitFor, fireEvent } from '@testing-library/react';

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

// fetch mock: ArrayBuffer + X-Total-Chapters 헤더를 반환
const mockArrayBuffer = new ArrayBuffer(8);
const mockArrayBuffer2 = new ArrayBuffer(16);

function createFetchResponse(buf = mockArrayBuffer, totalChapters = 20) {
    return {
        ok: true,
        headers: {
            get: (name) => name === 'X-Total-Chapters' ? String(totalChapters) : null,
        },
        arrayBuffer: () => Promise.resolve(buf),
    };
}

beforeEach(() => {
    mockReactReader.mockClear();
    globalThis.fetch = vi.fn(() => Promise.resolve(createFetchResponse()));
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

    // ── 점진적 로딩: 초기 요청 ──

    it('preview=true이면 chapters=1로 첫 요청한다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=1',
            expect.objectContaining({ signal: expect.any(AbortSignal) })
        );
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalledWith(
                expect.objectContaining({ url: mockArrayBuffer })
            );
        });
    });

    it('preview=false(전체보기)이면 chapters=1로 첫 요청한다', async () => {
        render(<ViewEPUB bookId={42} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=1',
            expect.objectContaining({ signal: expect.any(AbortSignal) })
        );
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalledWith(
                expect.objectContaining({ url: mockArrayBuffer })
            );
        });
    });

    // ── 점진적 로딩: 자동 추가 로드 ──

    it('미리보기 모드에서 초기 로딩 후 자동으로 chapters=11 요청한다', async () => {
        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            return Promise.resolve(createFetchResponse(
                callCount === 1 ? mockArrayBuffer : mockArrayBuffer2, 20
            ));
        });

        render(<ViewEPUB bookId={42} preview={true} />);

        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=1',
            expect.any(Object)
        );

        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/42?chapters=11',
                expect.any(Object)
            );
        });
    });

    it('미리보기 모드에서 11챕터 이후 추가 자동 로드하지 않는다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 25))
        );

        render(<ViewEPUB bookId={42} preview={true} />);

        // chapters=11까지 자동 로드 대기
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/42?chapters=11',
                expect.any(Object)
            );
        });

        // 추가 로드 없음 확인
        await act(async () => { await new Promise(r => setTimeout(r, 50)); });
        expect(globalThis.fetch).not.toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=21',
            expect.any(Object)
        );
    });

    it('전체보기 모드에서 초기 로딩 후 자동으로 chapters=11 요청한다', async () => {
        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            return Promise.resolve(createFetchResponse(
                callCount === 1 ? mockArrayBuffer : mockArrayBuffer2, 20
            ));
        });

        render(<ViewEPUB bookId={42} />);

        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=1',
            expect.any(Object)
        );

        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/42?chapters=11',
                expect.any(Object)
            );
        });
    });

    it('전체보기 모드에서 연속 자동 로드한다 (1→11→21)', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 25))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/42?chapters=21',
                expect.any(Object)
            );
        });
    });

    // ── 챕터 로딩 버튼 ──

    it('자동 로드 진행 중 로딩 버튼이 표시된다', async () => {
        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            if (callCount === 1) {
                return Promise.resolve(createFetchResponse(mockArrayBuffer, 20));
            }
            // 자동 로드 요청은 지연 → 버튼이 보임
            return new Promise(() => {});
        });

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(screen.getByRole('button', { name: /1\/20/ })).toBeTruthy();
        });
    });

    it('자동 로드 실패 후 "더 보기" 클릭으로 재시도할 수 있다', async () => {
        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            if (callCount === 1) {
                return Promise.resolve(createFetchResponse(mockArrayBuffer, 20));
            }
            if (callCount === 2) {
                // 자동 로드 실패
                return Promise.reject(new Error('Network error'));
            }
            // 수동 재시도 성공
            return Promise.resolve(createFetchResponse(mockArrayBuffer2, 20));
        });

        render(<ViewEPUB bookId={42} />);

        // 자동 로드 실패 후 "더 보기" 버튼 표시 대기
        await waitFor(() => {
            expect(screen.getByText(/더 보기/)).toBeTruthy();
        });

        // "더 보기" 클릭으로 재시도
        fireEvent.click(screen.getByText(/더 보기/));

        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledTimes(3);
        });
    });

    it('모든 챕터 로드 완료 시 버튼이 숨겨진다', async () => {
        globalThis.fetch = vi.fn(() => Promise.resolve(createFetchResponse(mockArrayBuffer, 1)));

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        expect(screen.queryByText(/더 보기/)).toBeNull();
        expect(screen.queryByRole('button', { name: /챕터/ })).toBeNull();
    });

    // ── %2F 회귀 방지 ──

    it('preview URL에 파일 경로나 %2F가 포함되지 않는다', () => {
        render(<ViewEPUB bookId={42} />);
        const fetchUrl = globalThis.fetch.mock.calls[0][0];
        expect(fetchUrl).toBe('http://localhost:8000/preview/42?chapters=1');
        expect(fetchUrl).not.toContain('%2F');
    });

    // ── bookId / preview 변경 ──

    it('bookId 변경 시 새로 fetch한다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/1?chapters=1',
                expect.any(Object)
            );
        });

        rerender(<ViewEPUB bookId={2} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/2?chapters=1',
                expect.any(Object)
            );
        });
    });

    it('preview 전환 시 새로 fetch한다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} preview={true} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/1?chapters=1',
            expect.any(Object)
        );

        globalThis.fetch.mockClear();
        rerender(<ViewEPUB bookId={1} preview={false} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/1?chapters=1',
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

    it('15초 내 로딩 미완료 시 타임아웃 에러를 표시한다', async () => {
        vi.useFakeTimers();
        autoLoad = false;

        render(<ViewEPUB bookId={1} />);

        await act(async () => { vi.advanceTimersByTime(1); });
        await act(async () => { vi.advanceTimersByTime(15000); });

        expect(screen.getByText('미리보기 로딩 시간이 초과되었습니다.')).toBeTruthy();
        expect(screen.queryByText('로딩 중...')).toBeNull();

        vi.useRealTimers();
    });

    it('로딩 완료 후에는 타임아웃이 발생하지 않는다', async () => {
        vi.useFakeTimers();
        autoLoad = true;

        render(<ViewEPUB bookId={1} />);

        await act(async () => { vi.advanceTimersByTime(1); });

        act(() => { vi.advanceTimersByTime(15000); });
        expect(screen.queryByText('미리보기 로딩 시간이 초과되었습니다.')).toBeNull();

        vi.useRealTimers();
    });

    // ── fetch 에러 ──

    it('서버 에러(non-ok) 시 에러 메시지를 표시한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve({
                ok: false,
                status: 500,
                headers: { get: () => null },
            })
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
