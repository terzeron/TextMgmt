// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest';

const { mockJsonGetReq } = vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
}));

vi.mock('../src/Common', () => ({
    jsonGetReq: mockJsonGetReq,
}));

describe('categoryMappingCache', () => {
    let loadCategoryMappings, fetchCategoryMappings, isCacheInitialized, updateCachedMappings;

    beforeEach(async () => {
        vi.clearAllMocks();
        vi.resetModules();
        const mod = await import('../src/categoryMappingCache');
        loadCategoryMappings = mod.loadCategoryMappings;
        fetchCategoryMappings = mod.fetchCategoryMappings;
        isCacheInitialized = mod.isCacheInitialized;
        updateCachedMappings = mod.updateCachedMappings;
    });

    // ── loadCategoryMappings ──

    it('초기 상태에서 빈 객체를 반환한다', () => {
        expect(loadCategoryMappings()).toEqual({});
        expect(loadCategoryMappings('book')).toEqual({});
        expect(loadCategoryMappings('comic')).toEqual({});
    });

    it('알 수 없는 contentType은 빈 객체를 반환한다', () => {
        expect(loadCategoryMappings('unknown')).toEqual({});
    });

    // ── isCacheInitialized ──

    it('초기 상태에서 false를 반환한다', () => {
        expect(isCacheInitialized()).toBe(false);
        expect(isCacheInitialized('book')).toBe(false);
        expect(isCacheInitialized('comic')).toBe(false);
    });

    // ── fetchCategoryMappings ──

    it('올바른 URL로 jsonGetReq를 호출한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            resolve({ '1_fiction': ['소설'] });
        });

        await fetchCategoryMappings('book');

        expect(mockJsonGetReq).toHaveBeenCalledWith(
            '/category-mappings?content_type=book', null,
            expect.any(Function), expect.any(Function)
        );
    });

    it('성공 시 캐시를 갱신하고 초기화 플래그를 설정한다', async () => {
        const data = { '1_fiction': ['소설', '문학'] };
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            resolve(data);
        });

        const result = await fetchCategoryMappings('book');

        expect(result).toEqual(data);
        expect(loadCategoryMappings('book')).toEqual(data);
        expect(isCacheInitialized('book')).toBe(true);
    });

    it('성공 시 null 결과를 빈 객체로 변환한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            resolve(null);
        });

        const result = await fetchCategoryMappings('book');

        expect(result).toEqual({});
    });

    it('실패 시에도 초기화 완료로 표시하고 기존 캐시를 반환한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
            reject('네트워크 오류');
        });

        const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
        const result = await fetchCategoryMappings('book');

        expect(result).toEqual({});
        expect(isCacheInitialized('book')).toBe(true);
        expect(consoleSpy).toHaveBeenCalledWith('Failed to fetch category mappings:', '네트워크 오류');
        consoleSpy.mockRestore();
    });

    // ── updateCachedMappings ──

    it('캐시를 직접 갱신한다', () => {
        const data = { '2_science': ['과학'] };
        updateCachedMappings('book', data);

        expect(loadCategoryMappings('book')).toEqual(data);
        expect(isCacheInitialized('book')).toBe(true);
    });

    // ── contentType 독립성 ──

    it('book과 comic 캐시가 독립적이다', async () => {
        const bookData = { '1_fiction': ['소설'] };
        const comicData = { '2_manhwa': ['만화'] };

        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url.includes('book')) resolve(bookData);
            else resolve(comicData);
        });

        await fetchCategoryMappings('book');
        await fetchCategoryMappings('comic');

        expect(loadCategoryMappings('book')).toEqual(bookData);
        expect(loadCategoryMappings('comic')).toEqual(comicData);
        expect(isCacheInitialized('book')).toBe(true);
        expect(isCacheInitialized('comic')).toBe(true);
    });
});
