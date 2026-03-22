import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
    quasiRandomHue,
    vanDerCorput,
    getApiUrlPrefix,
    getRandomLightColor,
    getRandomMediumColor,
    handleFetchErrors,
    jsonGetReq,
    jsonPostReq,
    externalJsonGetReq
} from '../src/Common';

describe('Common Utilities', () => {
    beforeEach(() => {
        vi.stubGlobal('fetch', vi.fn());
        vi.stubGlobal('window', {
            location: { reload: vi.fn() },
            __ENV__: { VITE_API_URL_PREFIX: '/api' }
        });
    });

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it('quasiRandomHue returns values between 0 and 1', () => {
        const h1 = quasiRandomHue(1);
        const h2 = quasiRandomHue(2);
        expect(h1).toBeGreaterThanOrEqual(0);
        expect(h1).toBeLessThan(1);
        expect(h1).not.toBe(h2);
    });

    it('vanDerCorput returns consistent sequence', () => {
        expect(vanDerCorput(1, 2)).toBe(0.5);
        expect(vanDerCorput(2, 2)).toBe(0.25);
    });

    it('getApiUrlPrefix respects window.__ENV__', () => {
        expect(getApiUrlPrefix()).toBe('/api');
    });

    it('getRandomLightColor returns valid hsl string', () => {
        const color = getRandomLightColor('test-key');
        expect(color).toMatch(/^hsl\(\d+, 55%, 90%\)$/);
    });

    it('getRandomMediumColor returns valid hsl string', () => {
        const color = getRandomMediumColor('test-key');
        expect(color).toMatch(/^hsl\(\d+, 50%, 55%\)$/);
    });

    it('handleFetchErrors throws on non-ok non-401 response', () => {
        const response = { ok: false, status: 500, statusText: 'Internal Server Error' };
        expect(() => handleFetchErrors(response)).toThrow('Internal Server Error');
    });

    it('handleFetchErrors returns response on ok response', () => {
        const response = { ok: true, status: 200 };
        expect(handleFetchErrors(response)).toBe(response);
    });

    it('handleFetchErrors returns response on 401 response', () => {
        const response = { ok: false, status: 401 };
        expect(handleFetchErrors(response)).toBe(response);
    });

    it('jsonGetReq calls fetch with correct URL and options', async () => {
        const mockResult = { status: 'success', result: 'test-data' };
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => mockResult
        });

        const resolve = vi.fn();
        const reject = vi.fn();

        jsonGetReq('/test-endpoint', null, resolve, reject);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith('test-data');
        });

        expect(fetch).toHaveBeenCalledWith('/api/test-endpoint', expect.objectContaining({
            method: 'GET',
            credentials: 'include'
        }));
    });

    it('jsonPostReq calls fetch with payload', async () => {
        const mockResult = { status: 'success', result: { id: 1 } };
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => mockResult
        });

        const resolve = vi.fn();
        const payload = { name: 'test' };

        jsonPostReq('/test-endpoint', payload, resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith({ id: 1 });
        });

        expect(fetch).toHaveBeenCalledWith('/api/test-endpoint', expect.objectContaining({
            method: 'POST',
            body: JSON.stringify(payload),
            headers: expect.objectContaining({ 'Content-Type': 'application/json' })
        }));
    });

    it('apiReq handles error in response data', async () => {
        const mockResult = { status: 'error', error: 'Something went wrong' };
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => mockResult
        });

        const reject = vi.fn();

        jsonGetReq('/test-endpoint', null, null, reject);

        await vi.waitFor(() => {
            expect(reject).toHaveBeenCalledWith('Something went wrong');
        });
    });

    it('externalJsonGetReq calls fetch without prefix', async () => {
        const mockResult = { some: 'external-data' };
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => mockResult
        });

        const resolve = vi.fn();
        externalJsonGetReq('https://example.com/api', resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith(mockResult);
        });

        expect(fetch).toHaveBeenCalledWith('https://example.com/api');
    });

    it('apiReq handles 401 and attempts refresh', async () => {
        // 1st fetch returns 401
        fetch.mockResolvedValueOnce({
            ok: false,
            status: 401
        });
        // 2nd fetch (refresh) returns success
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200
        });
        // 3rd fetch (retry) returns success data
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => ({ status: 'success', result: 'refreshed-data' })
        });

        const resolve = vi.fn();
        jsonGetReq('/test-endpoint', null, resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith('refreshed-data');
        });

        expect(fetch).toHaveBeenCalledWith('/api/auth/refresh', expect.anything());
    });

    it('apiReq handles 401 and refresh failure', async () => {
        fetch.mockResolvedValueOnce({ ok: false, status: 401 }); // 1st fetch
        fetch.mockResolvedValueOnce({ ok: false, status: 401 }); // refresh attempt

        const reject = vi.fn();
        jsonGetReq('/test-endpoint', null, null, reject);

        await vi.waitFor(() => {
            expect(window.location.reload).toHaveBeenCalled();
        });
    });

    it('processData formats dates correctly', async () => {
        const mockResult = {
            status: 'success',
            result: {},
            last_modified_time: '2023-10-27T10:00:00Z',
            last_responded_time: '2023-10-27T11:00:00Z'
        };
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => mockResult
        });

        const resolve = vi.fn();
        jsonGetReq('/test-endpoint', null, resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith(expect.objectContaining({
                last_modified_time: expect.any(String),
                last_responded_time: expect.any(String)
            }));
        });
    });

    it('processData handles warnings', async () => {
        const mockResult = { status: 'success', result: 'some-data', warning: 'some-warning' };
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => mockResult
        });

        const resolve = vi.fn();
        jsonGetReq('/test-endpoint', null, resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith({ result: 'some-data', warning: 'some-warning' });
        });
    });

    it('textGetReq calls fetch and returns text', async () => {
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            text: async () => 'some-text'
        });

        const resolve = vi.fn();
        const { textGetReq } = await import('../src/Common');
        textGetReq('/test-text', null, resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith('some-text');
        });
    });

    it('blobGetReq calls fetch and returns blob', async () => {
        const mockBlob = new Blob(['test'], { type: 'text/plain' });
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            blob: async () => mockBlob
        });

        const resolve = vi.fn();
        const { blobGetReq } = await import('../src/Common');
        blobGetReq('/test-blob', null, resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith(mockBlob);
        });
    });

    it('rawJsonGetReq calls fetch and returns raw json', async () => {
        const mockResult = { some: 'raw-data' };
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => mockResult
        });

        const resolve = vi.fn();
        const { rawJsonGetReq } = await import('../src/Common');
        rawJsonGetReq('/test-raw', resolve);

        await vi.waitFor(() => {
            expect(resolve).toHaveBeenCalledWith(mockResult);
        });
    });
});
