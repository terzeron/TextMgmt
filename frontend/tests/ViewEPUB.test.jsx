// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, cleanup, waitFor, fireEvent } from '@testing-library/react';

afterEach(cleanup);

// ReactReader mock - autoLoad=true이면 즉시 locationChanged 호출
let autoLoad = true;
let capturedGetRendition = null;
const mockReactReader = vi.fn(({ url, locationChanged, getRendition, title }) => {
    if (getRendition && url) {
        capturedGetRendition = getRendition;
    }
    if (url && autoLoad) {
        setTimeout(() => locationChanged?.('epubcfi(/1)'), 0);
    }
    return <div data-testid="react-reader">{title && <span data-testid="book-title">{title}</span>}ReactReader</div>;
});

vi.mock('react-reader', () => ({
    ReactReader: (props) => mockReactReader(props),
}));

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

vi.mock('../src/ViewEPUB.css', () => ({}));

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

// localStorage mock
const localStorageMock = (() => {
    let store = {};
    return {
        getItem: vi.fn((key) => store[key] ?? null),
        setItem: vi.fn((key, value) => { store[key] = String(value); }),
        removeItem: vi.fn((key) => { delete store[key]; }),
        clear: vi.fn(() => { store = {}; }),
        _getStore: () => store,
    };
})();
Object.defineProperty(globalThis, 'localStorage', { value: localStorageMock, writable: true });

beforeEach(() => {
    mockReactReader.mockClear();
    capturedGetRendition = null;
    localStorageMock.clear();
    localStorageMock.getItem.mockClear();
    localStorageMock.setItem.mockClear();
    globalThis.fetch = vi.fn(() => Promise.resolve(createFetchResponse()));
});

afterEach(() => {
    vi.restoreAllMocks();
});

import ViewEPUB from '../src/ViewEPUB';

// rendition mock을 만들어주는 헬퍼
function createMockRendition({ destroyThrows = false, locationsTotal = 300 } = {}) {
    const handlers = {};
    const locations = {
        generate: vi.fn(() => {
            // epubjs: total = _locations.length - 1 (0-indexed 최대값)
            locations.total = locationsTotal - 1;
            return Promise.resolve(Array(locationsTotal));
        }),
        locationFromCfi: vi.fn(() => 44),
        total: 0,
    };
    return {
        destroy: destroyThrows
            ? vi.fn(() => { throw new Error('already destroyed'); })
            : vi.fn(),
        themes: {
            fontSize: vi.fn(),
            font: vi.fn(),
        },
        on: vi.fn((event, handler) => {
            handlers[event] = handler;
        }),
        book: {
            spine: {
                get: vi.fn((target) => target || null),
            },
            loaded: {
                metadata: Promise.resolve({ title: '테스트 책 제목' }),
            },
            ready: Promise.resolve(),
            locations,
        },
        _handlers: handlers,
        _emitRelocated: (location) => {
            if (handlers['relocated']) handlers['relocated'](location);
        },
    };
}

describe('ViewEPUB', () => {
    beforeEach(() => {
        mockReactReader.mockClear();
        capturedGetRendition = null;
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

        expect(screen.getByText('EPUB 로딩 시간이 초과되었습니다.')).toBeTruthy();
        expect(screen.queryByText('로딩 중...')).toBeNull();

        vi.useRealTimers();
    });

    it('로딩 완료 후에는 타임아웃이 발생하지 않는다', async () => {
        vi.useFakeTimers();
        autoLoad = true;

        render(<ViewEPUB bookId={1} />);

        await act(async () => { vi.advanceTimersByTime(1); });

        act(() => { vi.advanceTimersByTime(15000); });
        expect(screen.queryByText('EPUB 로딩 시간이 초과되었습니다.')).toBeNull();

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
        expect(screen.queryByText('EPUB 로딩 시간이 초과되었습니다.')).toBeNull();

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

    // ══════════════════════════════════════════════
    // ── 전체보기 기능 테스트 ──
    // ══════════════════════════════════════════════

    // ── 책 제목 표시 ──

    it('전체보기 모드에서 getRendition으로 책 제목을 추출한다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // metadata Promise 해결 대기
        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // title prop이 ReactReader에 전달되는지 확인
        const calls = mockReactReader.mock.calls;
        const hasTitle = calls.some(c => c[0].title === '테스트 책 제목');
        expect(hasTitle).toBe(true);
    });

    it('미리보기 모드에서는 책 제목을 title prop에 전달하지 않는다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        // preview 모드의 모든 호출에서 title이 undefined
        const calls = mockReactReader.mock.calls;
        calls.forEach((c) => {
            expect(c[0].title).toBeUndefined();
        });
    });

    // ── 글자 크기 조절 ──

    it('전체보기에서 A+/A- 버튼이 표시된다', async () => {
        render(<ViewEPUB bookId={42} />);

        // 로딩 완료 대기
        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });

        expect(screen.getByLabelText('글자 크기 늘리기')).toBeTruthy();
        expect(screen.getByLabelText('글자 크기 줄이기')).toBeTruthy();
    });

    it('미리보기에서는 툴바가 표시되지 않는다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        await act(async () => { await new Promise(r => setTimeout(r, 50)); });
        expect(screen.queryByTestId('epub-toolbar')).toBeNull();
    });

    it('A+ 클릭 시 rendition.themes.fontSize가 호출된다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // 로딩 완료 대기
        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });

        const plusBtn = screen.getByLabelText('글자 크기 늘리기');
        await act(async () => {
            fireEvent.click(plusBtn);
        });

        expect(mockRendition.themes.fontSize).toHaveBeenCalledWith('120%');
        expect(localStorageMock.setItem).toHaveBeenCalledWith('epub_fontSize', '120');
    });

    it('A- 클릭 시 글자 크기가 줄어든다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });

        const minusBtn = screen.getByLabelText('글자 크기 줄이기');
        await act(async () => {
            fireEvent.click(minusBtn);
        });

        expect(mockRendition.themes.fontSize).toHaveBeenCalledWith('80%');
        expect(localStorageMock.setItem).toHaveBeenCalledWith('epub_fontSize', '80');
    });

    it('글자 크기가 최소(80%)이면 A- 버튼이 비활성화된다', async () => {
        // localStorage에 최소값 설정
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_fontSize') return '80';
            return null;
        });

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });

        const minusBtn = screen.getByLabelText('글자 크기 줄이기');
        expect(minusBtn.disabled).toBe(true);
    });

    it('글자 크기가 최대(160%)이면 A+ 버튼이 비활성화된다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_fontSize') return '160';
            return null;
        });

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });

        const plusBtn = screen.getByLabelText('글자 크기 늘리기');
        expect(plusBtn.disabled).toBe(true);
    });

    it('getRendition에서 저장된 글자 크기를 적용한다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_fontSize') return '140';
            return null;
        });

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        expect(mockRendition.themes.fontSize).toHaveBeenCalledWith('140%');
    });

    // ── 글꼴 변경 ──

    it('전체보기에서 글꼴 선택 드롭다운이 표시된다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });

        expect(screen.getByLabelText('글꼴 선택')).toBeTruthy();
    });

    it('글꼴 변경 시 rendition.themes.font가 호출된다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });

        const select = screen.getByLabelText('글꼴 선택');
        await act(async () => {
            fireEvent.change(select, { target: { value: 'serif' } });
        });

        expect(mockRendition.themes.font).toHaveBeenCalledWith('serif');
        expect(localStorageMock.setItem).toHaveBeenCalledWith('epub_fontFamily', 'serif');
    });

    it('getRendition에서 저장된 글꼴을 적용한다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_fontFamily') return 'serif';
            return null;
        });

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        expect(mockRendition.themes.font).toHaveBeenCalledWith('serif');
    });

    // ── 페이지 번호 ──

    it('전체보기에서 relocated 이벤트로 페이지 정보를 표시한다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // relocated 이벤트 시뮬레이션
        await act(async () => {
            mockRendition._emitRelocated({
                start: { displayed: { page: 3, total: 12 } },
            });
        });

        expect(screen.getByTestId('epub-page-info').textContent).toBe('3 / 12');
    });

    it('미리보기에서는 페이지 정보가 표시되지 않는다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        await act(async () => { await new Promise(r => setTimeout(r, 50)); });
        expect(screen.queryByTestId('epub-page-info')).toBeNull();
    });

    // ── 읽기 위치 저장/복원 ──

    it('전체보기에서 페이지 변경 시 위치를 localStorage에 저장한다', async () => {
        render(<ViewEPUB bookId={42} />);

        // 초기 로드 완료 대기
        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // 페이지 넘기기 시뮬레이션 (초기 로드 이후)
        const lastCall = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => {
            lastCall[0].locationChanged('epubcfi(/6/4)');
        });

        expect(localStorageMock.setItem).toHaveBeenCalledWith('epub_location_42', 'epubcfi(/6/4)');
    });

    it('미리보기에서는 읽기 위치를 저장하지 않는다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // 이전 테스트의 stale autoLoad timeout 호출 기록을 정리
        localStorageMock.setItem.mockClear();

        const lastCall = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => {
            lastCall[0].locationChanged('epubcfi(/6/4)');
        });

        const locationSaves = localStorageMock.setItem.mock.calls.filter(
            c => c[0].startsWith('epub_location_')
        );
        expect(locationSaves.length).toBe(0);
    });

    it('전체보기 초기 부분 로드에서는 저장된 위치를 적용하지 않는다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_location_42') return 'epubcfi(/6/4)';
            return null;
        });

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        // 초기 부분 로드(5챕터)에서는 저장된 위치가 전달되지 않음
        const firstCall = mockReactReader.mock.calls[0];
        expect(firstCall[0].location).toBeUndefined();
    });

    it('전체보기에서 전체 데이터 바꿔치기 시 저장된 위치를 복원한다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_location_42') return 'epubcfi(/6/4)';
            return null;
        });

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

        // 페이지 넘기기 → 데이터 바꿔치기 트리거
        const lastCall = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => {
            lastCall[0].locationChanged('epubcfi(/2)');
        });

        // 전체 데이터 바꿔치기 후 저장된 위치가 복원됨
        await waitFor(() => {
            const calls = mockReactReader.mock.calls;
            const hasRestoredLocation = calls.some(c =>
                c[0].url === mockArrayBuffer2 && c[0].location === 'epubcfi(/6/4)'
            );
            expect(hasRestoredLocation).toBe(true);
        });
    });

    it('전체 챕터가 초기 로드로 충분하면 저장된 위치를 즉시 복원한다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_location_42') return 'epubcfi(/6/4)';
            return null;
        });

        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        // initialLoadDone 후 즉시 복원 (epubKey 증가로 재렌더)
        await waitFor(() => {
            const calls = mockReactReader.mock.calls;
            const hasRestoredLocation = calls.some(c => c[0].location === 'epubcfi(/6/4)');
            expect(hasRestoredLocation).toBe(true);
        });
    });

    it('저장된 위치가 없으면 전체보기에서 처음부터 시작한다', async () => {
        // localStorage에 저장된 위치 없음
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        // 모든 호출에서 location이 undefined (처음부터 시작)
        const firstCall = mockReactReader.mock.calls[0];
        expect(firstCall[0].location).toBeUndefined();
    });

    it('저장된 위치가 있어도 초기 로드에서 타임아웃이 발생하지 않는다', async () => {
        vi.useFakeTimers();

        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_location_42') return 'epubcfi(/6/100)';
            return null;
        });

        autoLoad = true;
        render(<ViewEPUB bookId={42} />);

        // 초기 로딩 완료 (autoLoad가 locationChanged 호출)
        await act(async () => { vi.advanceTimersByTime(1); });

        // 15초 후에도 타임아웃 에러 없음
        await act(async () => { vi.advanceTimersByTime(15000); });
        expect(screen.queryByText('EPUB 로딩 시간이 초과되었습니다.')).toBeNull();

        vi.useRealTimers();
    });

    it('바꿔치기 전 여러 번 페이지를 넘겨도 저장된 위치가 보존된다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_location_42') return 'epubcfi(/6/4)';
            return null;
        });

        let callCount = 0;
        globalThis.fetch = vi.fn(() => {
            callCount++;
            if (callCount === 1) {
                return Promise.resolve(createFetchResponse(mockArrayBuffer, 20));
            }
            // 백그라운드 fetch를 지연시켜 아직 준비 안 된 상태 유지
            return new Promise(() => {});
        });

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // 백그라운드 데이터가 아직 없는 상태에서 페이지를 여러 번 넘김
        const getLastCall = () => mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => { getLastCall()[0].locationChanged('epubcfi(/2)'); });
        await act(async () => { getLastCall()[0].locationChanged('epubcfi(/3)'); });
        await act(async () => { getLastCall()[0].locationChanged('epubcfi(/4)'); });

        // 아직 바꿔치기가 안 됐으므로 저장된 위치 epubcfi(/6/4)가 어떤 호출에도 나타나지 않음
        const calls = mockReactReader.mock.calls;
        const hasRestoredLocation = calls.some(c => c[0].location === 'epubcfi(/6/4)');
        expect(hasRestoredLocation).toBe(false);

        // localStorage에는 페이지 넘기기 위치가 저장됨 (현재 위치)
        expect(localStorageMock.setItem).toHaveBeenCalledWith('epub_location_42', 'epubcfi(/4)');
    });

    it('바꿔치기 후에는 저장된 위치가 소비되어 재사용되지 않는다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_location_42') return 'epubcfi(/6/4)';
            return null;
        });

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

        // 첫 페이지 넘기기 → 바꿔치기 + 위치 복원
        const call1 = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => { call1[0].locationChanged('epubcfi(/2)'); });

        await waitFor(() => {
            const calls = mockReactReader.mock.calls;
            expect(calls.some(c => c[0].location === 'epubcfi(/6/4)')).toBe(true);
        });

        // 두 번째 페이지 넘기기 후 → 저장된 위치가 다시 사용되지 않음
        const call2 = mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
        await act(async () => { call2[0].locationChanged('epubcfi(/7)'); });

        // localStorage에 최신 위치가 저장됨 (복원된 위치가 아닌 현재 위치)
        expect(localStorageMock.setItem).toHaveBeenCalledWith('epub_location_42', 'epubcfi(/7)');
    });

    it('미리보기에서는 저장된 읽기 위치를 무시한다', async () => {
        localStorageMock.getItem.mockImplementation((key) => {
            if (key === 'epub_location_42') return 'epubcfi(/6/4)';
            return null;
        });

        render(<ViewEPUB bookId={42} preview={true} />);

        await waitFor(() => {
            expect(mockReactReader).toHaveBeenCalled();
        });

        // preview 모드에서는 어떤 호출에서도 저장된 위치가 전달되지 않음
        const calls = mockReactReader.mock.calls;
        const hasStoredLocation = calls.some(c => c[0].location === 'epubcfi(/6/4)');
        expect(hasStoredLocation).toBe(false);
    });

    // ── getRendition에서 relocated 리스너 등록 ──

    it('전체보기에서 getRendition이 relocated 리스너를 등록한다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        expect(mockRendition.on).toHaveBeenCalledWith('relocated', expect.any(Function));
    });

    it('미리보기에서는 getRendition이 relocated 리스너를 등록하지 않는다', async () => {
        render(<ViewEPUB bookId={42} preview={true} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        expect(mockRendition.on).not.toHaveBeenCalled();
    });

    // ══════════════════════════════════════════════
    // ── 전역 페이지 번호 (book.locations) 테스트 ──
    // ══════════════════════════════════════════════

    it('전체 챕터 로드 완료 시 book.locations.generate가 호출된다', async () => {
        // totalChapters=3 ≤ 5 → allChaptersLoadedRef = true
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        expect(mockRendition.book.locations.generate).toHaveBeenCalledWith(1024);
    });

    it('부분 로드 시에는 book.locations.generate가 호출되지 않는다', async () => {
        // totalChapters=20, initialChapters=5 → allChaptersLoaded = false
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        expect(mockRendition.book.locations.generate).not.toHaveBeenCalled();
    });

    it('locations 준비 후 relocated에서 전역 페이지 번호를 표시한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition({ locationsTotal: 300 });
        mockRendition.book.locations.locationFromCfi.mockReturnValue(44);
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // locations.generate 완료 대기
        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // relocated 이벤트 (cfi 포함)
        await act(async () => {
            mockRendition._emitRelocated({
                start: {
                    cfi: 'epubcfi(/6/4)',
                    displayed: { page: 3, total: 12 },
                },
            });
        });

        // 전역 페이지: 44 + 1 = 45 / 300
        expect(screen.getByTestId('epub-page-info').textContent).toBe('45 / 300');
    });

    it('locations.generate 완료 시 현재 위치의 전역 페이지로 즉시 업데이트된다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        // autoLoad가 locationRef.current = 'epubcfi(/1)' 설정
        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });
        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        const mockRendition = createMockRendition({ locationsTotal: 300 });
        mockRendition.book.locations.locationFromCfi.mockReturnValue(0);
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // generate 완료 → locationRef.current로 즉시 pageInfo 업데이트
        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        expect(screen.getByTestId('epub-page-info').textContent).toBe('1 / 300');
    });

    it('locations.generate 실패 시 챕터 내 페이지로 폴백한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        mockRendition.book.locations.generate.mockRejectedValue(new Error('generate failed'));
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // generate 실패 대기
        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // relocated 이벤트 → 챕터 내 페이지로 폴백
        await act(async () => {
            mockRendition._emitRelocated({
                start: {
                    cfi: 'epubcfi(/6/4)',
                    displayed: { page: 3, total: 12 },
                },
            });
        });

        expect(screen.getByTestId('epub-page-info').textContent).toBe('3 / 12');
    });

    it('미리보기에서는 locations.generate가 호출되지 않는다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 2))
        );

        render(<ViewEPUB bookId={42} preview={true} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        expect(mockRendition.book.locations.generate).not.toHaveBeenCalled();
    });

    it('마지막 페이지에서 relocated 시 page가 total을 초과하지 않는다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition({ locationsTotal: 300 });
        // locationFromCfi가 최대 인덱스(= locations.total = 299)를 반환
        mockRendition.book.locations.locationFromCfi.mockReturnValue(299);
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        await act(async () => {
            mockRendition._emitRelocated({
                start: {
                    cfi: 'epubcfi(/6/last)',
                    displayed: { page: 15, total: 15 },
                },
            });
        });

        // page(300) == total(300), 초과하지 않음
        const info = screen.getByTestId('epub-page-info').textContent;
        const [page, total] = info.split(' / ').map(Number);
        expect(page).toBe(300);
        expect(total).toBe(300);
        expect(page).toBeLessThanOrEqual(total);
    });

    it('마지막 페이지에서 generate 완료 시에도 page가 total을 초과하지 않는다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });
        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        const mockRendition = createMockRendition({ locationsTotal: 300 });
        // locationRef.current('epubcfi(/1)')에 대해 최대 인덱스 반환
        mockRendition.book.locations.locationFromCfi.mockReturnValue(299);
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // generate 완료 → 즉시 업데이트에서도 보정 확인
        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        const info = screen.getByTestId('epub-page-info').textContent;
        const [page, total] = info.split(' / ').map(Number);
        expect(page).toBe(300);
        expect(total).toBe(300);
        expect(page).toBeLessThanOrEqual(total);
    });

    it('locationFromCfi가 음수를 반환하면 챕터 내 페이지로 폴백한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition({ locationsTotal: 300 });
        mockRendition.book.locations.locationFromCfi.mockReturnValue(-1);
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // relocated에서 locationFromCfi가 -1 → 전역 분기 스킵 → 챕터 내 페이지
        await act(async () => {
            mockRendition._emitRelocated({
                start: {
                    cfi: 'epubcfi(/unknown)',
                    displayed: { page: 3, total: 12 },
                },
            });
        });

        expect(screen.getByTestId('epub-page-info').textContent).toBe('3 / 12');
    });

    it('locationFromCfi가 예외를 던지면 챕터 내 페이지로 폴백한다', async () => {
        globalThis.fetch = vi.fn(() =>
            Promise.resolve(createFetchResponse(mockArrayBuffer, 3))
        );

        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition({ locationsTotal: 300 });
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        await act(async () => { await new Promise(r => setTimeout(r, 10)); });

        // generate 완료 후 locationFromCfi가 예외를 던지도록 변경
        mockRendition.book.locations.locationFromCfi.mockImplementation(() => {
            throw new Error('invalid cfi');
        });

        await act(async () => {
            mockRendition._emitRelocated({
                start: {
                    cfi: 'epubcfi(/bad)',
                    displayed: { page: 7, total: 20 },
                },
            });
        });

        expect(screen.getByTestId('epub-page-info').textContent).toBe('7 / 20');
    });

    // ══════════════════════════════════════════════
    // ── rendition 리소스 해제 테스트 ──
    // ══════════════════════════════════════════════

    it('언마운트 시 rendition.destroy()가 호출된다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // 언마운트
        cleanup();

        expect(mockRendition.destroy).toHaveBeenCalledTimes(1);
    });

    it('bookId 변경 시 이전 rendition.destroy()가 호출된다', async () => {
        const { rerender } = render(<ViewEPUB bookId={1} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // bookId 변경 → cleanup → destroy
        rerender(<ViewEPUB bookId={2} />);

        expect(mockRendition.destroy).toHaveBeenCalledTimes(1);
    });

    it('getRendition 재호출 시 이전 rendition.destroy()가 호출된다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const oldRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(oldRendition);
        });

        // 새 rendition이 전달되면 이전 것을 destroy
        const newRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(newRendition);
        });

        expect(oldRendition.destroy).toHaveBeenCalledTimes(1);
        expect(newRendition.destroy).not.toHaveBeenCalled();
    });

    it('rendition.destroy()가 에러를 던져도 크래시하지 않는다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const badRendition = createMockRendition({ destroyThrows: true });
        await act(async () => {
            capturedGetRendition(badRendition);
        });

        // destroy()가 throw해도 언마운트가 정상 동작해야 함
        expect(() => cleanup()).not.toThrow();
        expect(badRendition.destroy).toHaveBeenCalled();
    });

    it('getRendition에서 같은 rendition이 다시 전달되면 destroy하지 않는다', async () => {
        render(<ViewEPUB bookId={42} />);

        await waitFor(() => {
            expect(capturedGetRendition).toBeTruthy();
        });

        const mockRendition = createMockRendition();
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        // 동일 rendition 재전달
        await act(async () => {
            capturedGetRendition(mockRendition);
        });

        expect(mockRendition.destroy).not.toHaveBeenCalled();
    });

    it('rendition이 없는 상태에서 언마운트해도 에러가 발생하지 않는다', async () => {
        const { unmount } = render(<ViewEPUB bookId={42} />);

        // getRendition 호출 전에 바로 언마운트
        expect(() => unmount()).not.toThrow();
    });
});
