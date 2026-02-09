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

    // ── 초기 로딩 ──

    it('preview=true이면 chapters=2로 첫 요청한다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=2',
            expect.objectContaining({ signal: expect.any(AbortSignal) })
        );
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalledWith(
                expect.objectContaining({ url: mockArrayBuffer })
            );
        });
    });

    it('preview=false(전체보기)이면 chapters=5로 첫 요청한다', async () => {
        render(<ViewEPUB bookId={42} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=5',
            expect.objectContaining({ signal: expect.any(AbortSignal) })
        );
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalledWith(
                expect.objectContaining({ url: mockArrayBuffer })
            );
        });
    });

    // ── 미리보기: 추가 로드 없음 ──

    it('미리보기 모드에서 2챕터 이후 추가 페칭하지 않는다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 20))
        );

        render(<ViewEPUB bookId={42} preview={true} />);

        // 초기 로드 완료 대기
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        // 추가 로드 없음 확인
        await act(async () => { await new Promise(r => setTimeout(r, 50)); });
        expect(globalThis.fetch).toHaveBeenCalledTimes(1);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=2',
            expect.any(Object)
        );
    });

    // ── 전체보기: 백그라운드 전체 페칭 ──

    it('전체보기 모드에서 초기 렌더 후 전체 챕터를 백그라운드 페칭한다', async () => {
        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            return Promise.resolve(createFetchResponse(
                callCount === 1 ? mockArrayBuffer : mockArrayBuffer2, 20
            ));
        });

        render(<ViewEPUB bookId={42} />);

        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/42?chapters=5',
            expect.any(Object)
        );

        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/42?chapters=20',
                expect.any(Object)
            );
        });
    });

    it('전체보기에서 전체 챕터가 초기 로드 이하이면 백그라운드 페칭하지 않는다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        await act(async () => { await new Promise(r => setTimeout(r, 50)); });
        // 초기 로드 1회만
        expect(globalThis.fetch).toHaveBeenCalledTimes(1);
    });

    // ── 페이지 넘기기 시 데이터 바꿔치기 ──

    it('백그라운드 데이터 준비 후 페이지 넘기면 전체 데이터로 바꿔치기한다', async () => {
        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            return Promise.resolve(createFetchResponse(
                callCount === 1 ? mockArrayBuffer : mockArrayBuffer2, 20
            ));
        });

        // autoLoad로 첫 locationChanged 자동 호출
        render(<ViewEPUB bookId={42} />);

        // 백그라운드 페칭 완료 대기
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/42?chapters=20',
                expect.any(Object)
            );
        });

        // 백그라운드 fetch 완료 후 잠시 대기
        await act(async () => { await new Promise(r => setTimeout(r, 50)); });

        // 페이지 넘기기 시뮬레이션: ReactReader의 locationChanged 호출
        const lastCall = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => {
            lastCall[0].locationChanged('epubcfi(/2)');
        });

        // 새 데이터(mockArrayBuffer2)로 ReactReader가 다시 렌더링됨
        await waitFor(() => {
            const calls = mockReactReader.mock.calls;
            const hasFullData = calls.some(c => c[0].url === mockArrayBuffer2);
            expect(hasFullData).toBe(true);
        });
    });

    // ── "더보기" 버튼 없음 ──

    it('"더 보기" 버튼이 표시되지 않는다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 20))
        );

        render(<ViewEPUB bookId={42} preview={true} />);

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
        expect(fetchUrl).toBe('http://localhost:8000/preview/42?chapters=5');
        expect(fetchUrl).not.toContain('%2F');
    });

    // ── bookId / preview 변경 ──

    it('bookId 변경 시 새로 fetch한다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/1?chapters=5',
                expect.any(Object)
            );
        });

        rerender(<ViewEPUB bookId={2} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/2?chapters=5',
                expect.any(Object)
            );
        });
    });

    it('preview 전환 시 새로 fetch한다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} preview={true} />);
        expect(globalThis.fetch).toHaveBeenCalledWith(
            'http://localhost:8000/preview/1?chapters=2',
            expect.any(Object)
        );

        globalThis.fetch.mockClear();
        rerender(<ViewEPUB bookId={1} preview={false} />);
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/1?chapters=5',
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

    it('추가 챕터 로드 시 타임아웃이 재발동되지 않는다', async () => {
        vi.useFakeTimers();
        autoLoad = true;

        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            return Promise.resolve(createFetchResponse(
                callCount === 1 ? mockArrayBuffer : mockArrayBuffer2, 20
            ));
        });

        render(<ViewEPUB bookId={42} />);

        // 초기 로딩 완료
        await act(async () => { vi.advanceTimersByTime(1); });

        // 백그라운드 페칭 완료 + 페이지 넘기기로 데이터 바꿔치기 후에도
        // 타임아웃이 발생하지 않아야 함
        await act(async () => { vi.advanceTimersByTime(20000); });
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

    // ── 백그라운드 fetch abort ──

    it('백그라운드 fetch 진행 중 언마운트 시 abort된다', async () => {
        let bgFetchStarted = false;
        globalThis.fetch = vi.fn((url) => {
            if (url.includes('chapters=20')) {
                bgFetchStarted = true;
                // 백그라운드 fetch를 지연시켜 진행 중 상태 유지
                return new Promise(() => {});
            }
            return Promise.resolve(createFetchResponse(mockArrayBuffer, 20));
        });

        const { unmount } = render(<ViewEPUB bookId={42} />);

        // 백그라운드 fetch 시작 대기
        await waitFor(() => { expect(bgFetchStarted).toBe(true); });

        const abortSpy = vi.spyOn(AbortController.prototype, 'abort');
        unmount();
        // 초기 + 백그라운드 controller 모두 abort
        expect(abortSpy).toHaveBeenCalled();
    });

    it('백그라운드 fetch 진행 중 bookId 변경 시 이전 fetch가 abort된다', async () => {
        let bgFetchCount = 0;
        globalThis.fetch = vi.fn((url) => {
            if (url.includes('chapters=20')) {
                bgFetchCount++;
                if (bgFetchCount === 1) {
                    return new Promise(() => {}); // 첫 번째 백그라운드는 지연
                }
            }
            return Promise.resolve(createFetchResponse(mockArrayBuffer, 20));
        });

        const { rerender } = render(<ViewEPUB bookId={1} />);

        // 백그라운드 fetch 시작 대기
        await waitFor(() => { expect(bgFetchCount).toBe(1); });

        const abortSpy = vi.spyOn(AbortController.prototype, 'abort');
        rerender(<ViewEPUB bookId={2} />);

        // bookId 변경으로 이전 fetch들이 abort됨
        expect(abortSpy).toHaveBeenCalled();
    });

    // ── 백그라운드 fetch 네트워크 에러 시 재시도 안 함 ──

    it('백그라운드 fetch 네트워크 실패 시 재시도하지 않는다', async () => {
        let bgFetchCount = 0;
        globalThis.fetch = vi.fn((url) => {
            if (url.includes('chapters=20')) {
                bgFetchCount++;
                return Promise.reject(new Error('Network error'));
            }
            return Promise.resolve(createFetchResponse(mockArrayBuffer, 20));
        });

        render(<ViewEPUB bookId={42} />);

        // 백그라운드 fetch 1회 시도 대기
        await waitFor(() => { expect(bgFetchCount).toBeGreaterThanOrEqual(1); });

        // 추가 대기 후에도 재시도 없음
        await act(async () => { await new Promise(r => setTimeout(r, 100)); });
        expect(bgFetchCount).toBe(1);
    });

    // ── 바꿔치기 후 두 번째 페이지 넘기기에서 중복 바꿔치기 없음 ──

    it('바꿔치기 완료 후 추가 페이지 넘기기에서 중복 바꿔치기가 없다', async () => {
        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            return Promise.resolve(createFetchResponse(
                callCount === 1 ? mockArrayBuffer : mockArrayBuffer2, 20
            ));
        });

        render(<ViewEPUB bookId={42} />);

        // 백그라운드 페칭 완료 대기
        await waitFor(() => {
            expect(globalThis.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/preview/42?chapters=20',
                expect.any(Object)
            );
        });

        await act(async () => { await new Promise(r => setTimeout(r, 50)); });

        // 첫 번째 페이지 넘기기: 바꿔치기 발생
        const call1 = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => { call1[0].locationChanged('epubcfi(/2)'); });

        await waitFor(() => {
            const calls = mockReactReader.mock.calls;
            expect(calls.some(c => c[0].url === mockArrayBuffer2)).toBe(true);
        });

        const callsAfterSwap = mockReactReader.mock.calls.length;

        // 두 번째 페이지 넘기기: 추가 바꿔치기 없음 (epubKey 변경 없음)
        const call2 = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => { call2[0].locationChanged('epubcfi(/3)'); });

        await act(async () => { await new Promise(r => setTimeout(r, 50)); });

        // 추가 fetch가 없어야 함 (초기 + 백그라운드 = 2회)
        const bgCalls = globalThis.fetch.mock.calls.filter(c => c[0].includes('chapters=20'));
        expect(bgCalls.length).toBe(1);
    });

    // ── 전체 챕터가 정확히 초기 로드와 같을 때 ──

    it('전체 챕터 수가 초기 로드와 정확히 같으면 백그라운드 페칭 안 한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 5))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        await act(async () => { await new Promise(r => setTimeout(r, 50)); });
        expect(globalThis.fetch).toHaveBeenCalledTimes(1);
    });
});
