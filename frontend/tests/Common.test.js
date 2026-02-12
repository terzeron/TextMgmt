// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { quasiRandomHue, vanDerCorput, getRandomLightColor, getRandomMediumColor, getApiUrlPrefix, handleFetchErrors, jsonGetReq, textGetReq, blobGetReq, rawJsonGetReq, externalJsonGetReq } from '../src/Common';

// ── quasiRandomHue ──

describe('quasiRandomHue', () => {
    it('결정적(deterministic) 출력을 반환한다', () => {
        expect(quasiRandomHue(0)).toBe(0);
        expect(quasiRandomHue(1)).toBeCloseTo(0.6180339887498949, 10);
        expect(quasiRandomHue(1)).toBe(quasiRandomHue(1));
    });

    it('반환값이 0~1 범위 내에 있다', () => {
        for (let i = 0; i < 1000; i++) {
            const val = quasiRandomHue(i);
            expect(val).toBeGreaterThanOrEqual(0);
            expect(val).toBeLessThan(1);
        }
    });

    it('연속 값이 균등하게 분포한다 (equidistribution)', () => {
        const buckets = new Array(10).fill(0);
        const N = 1000;
        for (let i = 0; i < N; i++) {
            const bucket = Math.floor(quasiRandomHue(i) * 10);
            buckets[bucket]++;
        }
        // 각 버킷에 최소 50개 이상 (기대값 100)
        for (const count of buckets) {
            expect(count).toBeGreaterThan(50);
        }
    });
});

// ── vanDerCorput ──

describe('vanDerCorput', () => {
    it('base 2에서 올바른 값을 반환한다', () => {
        expect(vanDerCorput(0)).toBe(0);
        expect(vanDerCorput(1)).toBe(0.5);
        expect(vanDerCorput(2)).toBe(0.25);
        expect(vanDerCorput(3)).toBe(0.75);
    });

    it('base 3에서 올바른 값을 반환한다', () => {
        expect(vanDerCorput(1, 3)).toBeCloseTo(1 / 3, 10);
        expect(vanDerCorput(2, 3)).toBeCloseTo(2 / 3, 10);
        expect(vanDerCorput(3, 3)).toBeCloseTo(1 / 9, 10);
    });

    it('반환값이 0~1 범위 내에 있다', () => {
        for (let i = 0; i < 500; i++) {
            const val = vanDerCorput(i);
            expect(val).toBeGreaterThanOrEqual(0);
            expect(val).toBeLessThan(1);
        }
    });
});

// ── getRandomLightColor ──

describe('getRandomLightColor', () => {
    it('HSL 형식의 문자열을 반환한다', () => {
        const color = getRandomLightColor('test');
        expect(color).toMatch(/^hsl\(\d+, 55%, 90%\)$/);
    });

    it('같은 입력에 대해 같은 색상을 반환한다', () => {
        expect(getRandomLightColor('myCategory')).toBe(getRandomLightColor('myCategory'));
    });

    it('다른 입력에 대해 다른 색상을 반환한다', () => {
        const colors = new Set();
        const prefixes = [
            '소설', '에세이', '시', '역사', '과학', '철학', '경제', '예술',
            '문학', '인문', '사회', '자연', '공학', '의학', '교육', '종교',
        ];
        for (const prefix of prefixes) {
            colors.add(getRandomLightColor(prefix));
        }
        // 16개 입력에 대해 최소 14개 이상 다른 색상
        expect(colors.size).toBeGreaterThanOrEqual(14);
    });

    it('다양한 키에서 Hue가 유효 범위(0~360)이다', () => {
        const keys = ['test', 'negative', '음수케이스', '123', '!@#$%'];
        for (const key of keys) {
            const color = getRandomLightColor(key);
            expect(color).toMatch(/^hsl\(\d+, 55%, 90%\)$/);
            const hue = parseInt(color.match(/\d+/)[0]);
            expect(hue).toBeGreaterThanOrEqual(0);
            expect(hue).toBeLessThanOrEqual(360);
        }
    });
});

// ── getRandomMediumColor ──

describe('getRandomMediumColor', () => {
    it('HSL 형식의 문자열을 반환한다', () => {
        const color = getRandomMediumColor('test');
        expect(color).toMatch(/^hsl\(\d+, 50%, 55%\)$/);
    });

    it('같은 입력에 대해 같은 색상을 반환한다', () => {
        expect(getRandomMediumColor('abc')).toBe(getRandomMediumColor('abc'));
    });
});

// ── getApiUrlPrefix ──

describe('getApiUrlPrefix', () => {
    afterEach(() => {
        delete window.__ENV__;
    });

    it('window.__ENV__에서 URL prefix를 가져온다', () => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://test:8000' };
        expect(getApiUrlPrefix()).toBe('http://test:8000');
    });

    it('환경 변수가 없으면 undefined를 반환한다', () => {
        delete window.__ENV__;
        // import.meta.env는 테스트 환경에서 설정 안 됨
        const result = getApiUrlPrefix();
        expect(result === undefined || result === null || result === '').toBe(true);
    });
});

// ── handleFetchErrors ──

describe('handleFetchErrors', () => {
    it('정상 응답은 그대로 반환한다', () => {
        const response = { ok: true, statusText: 'OK' };
        expect(handleFetchErrors(response)).toBe(response);
    });

    it('에러 응답은 예외를 던진다', () => {
        const response = { ok: false, statusText: 'Not Found' };
        expect(() => handleFetchErrors(response)).toThrow('Not Found');
    });
});

// ── apiReq (jsonGetReq / textGetReq) ──

describe('apiReq via jsonGetReq', () => {
    beforeEach(() => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
    });

    afterEach(() => {
        delete window.__ENV__;
        vi.restoreAllMocks();
    });

    it('JSON success 응답의 result를 resolve한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ status: 'success', result: { data: 42 } }),
        });
        const resolve = vi.fn();
        jsonGetReq('/test', null, resolve, vi.fn());
        await vi.waitFor(() => expect(resolve).toHaveBeenCalledWith({ data: 42 }));
        delete global.fetch;
    });

    it('JSON error 응답의 error를 reject한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ status: 'error', error: '서버 오류' }),
        });
        const reject = vi.fn();
        jsonGetReq('/test', null, vi.fn(), reject);
        await vi.waitFor(() => expect(reject).toHaveBeenCalledWith('서버 오류'));
        delete global.fetch;
    });

    it('warning 필드가 있으면 result와 함께 전달한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ status: 'success', result: { x: 1 }, warning: '주의' }),
        });
        const resolve = vi.fn();
        jsonGetReq('/test', null, resolve, vi.fn());
        await vi.waitFor(() => expect(resolve).toHaveBeenCalledWith({ result: { x: 1 }, warning: '주의' }));
        delete global.fetch;
    });

    it('fetch 실패 시 reject를 호출한다', async () => {
        global.fetch = vi.fn().mockRejectedValue(new Error('network'));
        const reject = vi.fn();
        jsonGetReq('/test', null, vi.fn(), reject);
        await vi.waitFor(() => expect(reject).toHaveBeenCalled());
        delete global.fetch;
    });

    it('final 콜백이 항상 호출된다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ status: 'success', result: 'ok' }),
        });
        const final = vi.fn();
        jsonGetReq('/test', null, vi.fn(), vi.fn(), final);
        await vi.waitFor(() => expect(final).toHaveBeenCalled());
        delete global.fetch;
    });

    it('payload가 있으면 POST body에 JSON을 전송한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ status: 'success', result: 'ok' }),
        });
        jsonGetReq('/test', { key: 'val' }, vi.fn(), vi.fn());
        await vi.waitFor(() => {
            expect(global.fetch).toHaveBeenCalledWith('http://api/test', expect.objectContaining({
                method: 'GET',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ key: 'val' }),
            }));
        });
        delete global.fetch;
    });
});

describe('textGetReq', () => {
    afterEach(() => {
        delete window.__ENV__;
        vi.restoreAllMocks();
    });

    it('TEXT 타입은 response.text()의 결과를 resolve한다', async () => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            text: () => Promise.resolve('텍스트 내용'),
        });
        const resolve = vi.fn();
        textGetReq('/test', null, resolve, vi.fn());
        await vi.waitFor(() => expect(resolve).toHaveBeenCalledWith('텍스트 내용'));
        delete global.fetch;
    });
});

// ── rawJsonGetReq ──

describe('rawJsonGetReq', () => {
    afterEach(() => {
        delete window.__ENV__;
        vi.restoreAllMocks();
    });

    it('원본 JSON 응답을 그대로 resolve한다', async () => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ status: 'success', result: [1, 2, 3] }),
        });
        const resolve = vi.fn();
        rawJsonGetReq('/test', resolve, vi.fn());
        await vi.waitFor(() => expect(resolve).toHaveBeenCalledWith({ status: 'success', result: [1, 2, 3] }));
        delete global.fetch;
    });

    it('에러 시 reject를 호출한다', async () => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
        global.fetch = vi.fn().mockRejectedValue(new Error('fail'));
        const reject = vi.fn();
        rawJsonGetReq('/test', vi.fn(), reject);
        await vi.waitFor(() => expect(reject).toHaveBeenCalled());
        delete global.fetch;
    });

    it('final 콜백이 항상 호출된다', async () => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({}),
        });
        const final = vi.fn();
        rawJsonGetReq('/test', vi.fn(), vi.fn(), final);
        await vi.waitFor(() => expect(final).toHaveBeenCalled());
        delete global.fetch;
    });
});

// ── externalJsonGetReq ──

describe('externalJsonGetReq', () => {
    afterEach(() => {
        vi.restoreAllMocks();
    });

    it('prefix 없이 외부 URL로 fetch한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ data: 'external' }),
        });
        const resolve = vi.fn();
        externalJsonGetReq('https://external.api/data', resolve, vi.fn());
        await vi.waitFor(() => {
            expect(global.fetch).toHaveBeenCalledWith('https://external.api/data');
            expect(resolve).toHaveBeenCalledWith({ data: 'external' });
        });
        delete global.fetch;
    });

    it('에러 시 reject를 호출한다', async () => {
        global.fetch = vi.fn().mockRejectedValue(new Error('fail'));
        const reject = vi.fn();
        externalJsonGetReq('https://external.api/data', vi.fn(), reject);
        await vi.waitFor(() => expect(reject).toHaveBeenCalled());
        delete global.fetch;
    });
});

// ── blobGetReq ──

describe('blobGetReq', () => {
    afterEach(() => {
        delete window.__ENV__;
        vi.restoreAllMocks();
    });

    it('BLOB 타입은 response.blob()의 결과를 resolve한다', async () => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
        const mockBlob = new Blob(['data']);
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            blob: () => Promise.resolve(mockBlob),
        });
        const resolve = vi.fn();
        blobGetReq('/test', null, resolve, vi.fn());
        await vi.waitFor(() => expect(resolve).toHaveBeenCalledWith(mockBlob));
        delete global.fetch;
    });
});

// ── jsonGetReq: last_modified_time / last_responded_time ──

describe('jsonGetReq timestamp 처리', () => {
    beforeEach(() => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
    });

    afterEach(() => {
        delete window.__ENV__;
        vi.restoreAllMocks();
    });

    it('last_modified_time을 result 객체에 포맷팅하여 추가한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({
                status: 'success',
                result: { data: 1 },
                last_modified_time: '2025-01-15T10:30:00Z',
            }),
        });
        const resolve = vi.fn();
        jsonGetReq('/test', null, resolve, vi.fn());
        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalled();
            const arg = resolve.mock.calls[0][0];
            expect(arg.last_modified_time).toBeDefined();
            expect(arg.last_modified_time).toMatch(/\d{2}-\d{2} \d{2}:\d{2}/);
        });
        delete global.fetch;
    });

    it('last_responded_time을 result 객체에 포맷팅하여 추가한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({
                status: 'success',
                result: { data: 2 },
                last_responded_time: '2025-06-20T14:00:00Z',
            }),
        });
        const resolve = vi.fn();
        jsonGetReq('/test', null, resolve, vi.fn());
        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalled();
            const arg = resolve.mock.calls[0][0];
            expect(arg.last_responded_time).toBeDefined();
            expect(arg.last_responded_time).toMatch(/\d{2}-\d{2} \d{2}:\d{2}/);
        });
        delete global.fetch;
    });
});

// ── apiReq: reject가 없는 경우 ──

describe('apiReq catch without reject', () => {
    afterEach(() => {
        delete window.__ENV__;
        vi.restoreAllMocks();
    });

    it('reject 콜백 없이 fetch 실패해도 에러를 던지지 않는다', async () => {
        window.__ENV__ = { VITE_API_URL_PREFIX: 'http://api' };
        global.fetch = vi.fn().mockRejectedValue(new Error('network'));
        // reject를 전달하지 않음
        jsonGetReq('/test', null, vi.fn(), undefined);
        // 에러 없이 완료되어야 함
        await vi.waitFor(() => expect(global.fetch).toHaveBeenCalled());
        delete global.fetch;
    });
});
