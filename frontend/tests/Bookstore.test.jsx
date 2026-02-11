// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, act, waitFor } from '@testing-library/react';

vi.mock('../src/Common', () => ({
    rawJsonGetReq: vi.fn(),
    getApiUrlPrefix: () => 'http://localhost:8000',
    handleFetchErrors: (r) => r,
    getRandomMediumColor: () => '#000',
    ROOT_DIRECTORY: '$$rootdir$$',
}));

import { rawJsonGetReq } from '../src/Common';
import Bookstore, { getTwoLevelCategory, extractMultiPathCategories } from '../src/Bookstore';

describe('getTwoLevelCategory', () => {
    it('3단계 카테고리에서 하위 2단계를 추출한다', () => {
        expect(getTwoLevelCategory('소설/시/희곡 > SF > 한국SF')).toBe('SF 한국SF');
    });

    it('2단계 카테고리에서 두 단계 모두 추출한다', () => {
        expect(getTwoLevelCategory('소설/시/희곡 > 중국소설')).toBe('소설/시/희곡 중국소설');
    });

    it('1단계 카테고리는 그대로 반환한다', () => {
        expect(getTwoLevelCategory('한국SF')).toBe('한국SF');
    });

    it('빈 문자열이면 빈 문자열을 반환한다', () => {
        expect(getTwoLevelCategory('')).toBe('');
    });

    it('null이면 빈 문자열을 반환한다', () => {
        expect(getTwoLevelCategory(null)).toBe('');
    });

    it('undefined이면 빈 문자열을 반환한다', () => {
        expect(getTwoLevelCategory(undefined)).toBe('');
    });

    it('4단계 이상에서도 마지막 2단계만 추출한다', () => {
        expect(getTwoLevelCategory('A > B > C > D')).toBe('C D');
    });

    it('구분자 앞뒤 공백을 제거한다', () => {
        expect(getTwoLevelCategory('  A  >  B  ')).toBe('A B');
    });

    it('빈 세그먼트가 있으면 무시한다', () => {
        // "A > B > " → trim → ["A", "B", ""] → filter → ["A", "B"]
        expect(getTwoLevelCategory('A > B > ')).toBe('A B');
    });
});

describe('extractMultiPathCategories', () => {
    it('다중 경로에서 각 경로의 마지막 두 단계를 추출한다', () => {
        expect(extractMultiPathCategories('소설 > 한국소설 || 소설 > 추리/미스터리/스릴러'))
            .toEqual(['소설 한국소설', '소설 추리/미스터리/스릴러']);
    });

    it('단일 경로도 배열로 반환한다', () => {
        expect(extractMultiPathCategories('소설 > 한국소설'))
            .toEqual(['소설 한국소설']);
    });

    it('3개 이상의 경로도 처리한다', () => {
        expect(extractMultiPathCategories('A > B || C > D || E > F'))
            .toEqual(['A B', 'C D', 'E F']);
    });

    it('빈 문자열이면 빈 배열을 반환한다', () => {
        expect(extractMultiPathCategories('')).toEqual([]);
    });

    it('null이면 빈 배열을 반환한다', () => {
        expect(extractMultiPathCategories(null)).toEqual([]);
    });

    it('undefined이면 빈 배열을 반환한다', () => {
        expect(extractMultiPathCategories(undefined)).toEqual([]);
    });

    it('빈 경로는 필터링한다', () => {
        expect(extractMultiPathCategories('소설 > 한국소설 || || 소설 > SF'))
            .toEqual(['소설 한국소설', '소설 SF']);
    });
});

describe('Bookstore 카테고리 수집', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    // rawJsonGetReq mock helper: URL 패턴별 응답 매핑
    const mockSearchResponses = (responses) => {
        rawJsonGetReq.mockImplementation((url, onSuccess) => {
            for (const [pattern, data] of Object.entries(responses)) {
                if (url.includes(pattern)) {
                    setTimeout(() => onSuccess(data), 0);
                    return;
                }
            }
            // 기본: 빈 결과
            setTimeout(() => onSuccess({ status: 'not_found', result: [] }), 0);
        });
    };

    it('자동 검색 시 yes24, aladin, naver 세 서점의 카테고리를 수집한다', async () => {
        const onCategoriesFound = vi.fn();

        mockSearchResponses({
            '/search/bookstore/yes24': {
                status: 'success',
                result: [{ title: 'T', author: 'A', category: '소설 > 한국소설', book_url: 'u1' }]
            },
            '/search/bookstore/aladin': {
                status: 'success',
                result: [{ title: 'T', author: 'A', category: '문학 > 한국문학', book_url: 'u2' }]
            },
            '/search/bookstore/naver': {
                status: 'success',
                result: [{ title: 'T', author: 'A', category: '도서 > 소설 > 추리/미스터리', book_url: 'u3' }]
            },
        });

        await act(async () => {
            render(<Bookstore
                bookInfo={{ title: '테스트', author: '저자', isbn: '' }}
                searchTrigger={1}
                onCategoriesFound={onCategoriesFound}
            />);
        });

        // 비동기 검색 완료 대기
        await waitFor(() => {
            const calls = onCategoriesFound.mock.calls;
            // 마지막 호출이 카테고리를 포함해야 함 (첫 호출은 빈 {} 초기화)
            const lastCall = calls[calls.length - 1]?.[0];
            expect(lastCall).toBeDefined();
            expect(Object.keys(lastCall).length).toBeGreaterThan(0);
        });

        const lastCall = onCategoriesFound.mock.calls[onCategoriesFound.mock.calls.length - 1][0];

        // yes24 카테고리
        expect(Object.values(lastCall)).toContain('소설 한국소설');
        // aladin 카테고리
        expect(Object.values(lastCall)).toContain('문학 한국문학');
        // naver 카테고리
        expect(Object.values(lastCall)).toContain('소설 추리/미스터리');
    });

    it('네이버쇼핑 다중 경로가 개별 키로 분리되어 수집된다', async () => {
        const onCategoriesFound = vi.fn();

        mockSearchResponses({
            '/search/bookstore/naver': {
                status: 'success',
                result: [{ title: 'T', author: 'A', category: '소설 > 한국소설 || 소설 > SF', book_url: 'u' }]
            },
        });

        await act(async () => {
            render(<Bookstore
                bookInfo={{ title: '테스트', author: '저자', isbn: '' }}
                searchTrigger={1}
                onCategoriesFound={onCategoriesFound}
            />);
        });

        await waitFor(() => {
            const calls = onCategoriesFound.mock.calls;
            const lastCall = calls[calls.length - 1]?.[0];
            expect(lastCall).toBeDefined();
            const naverKeys = Object.keys(lastCall).filter(k => k.startsWith('naver_'));
            expect(naverKeys.length).toBeGreaterThanOrEqual(2);
        });

        const lastCall = onCategoriesFound.mock.calls[onCategoriesFound.mock.calls.length - 1][0];

        // 하나의 검색 결과에서 두 경로가 별도 키로 수집됨
        const naverKeys = Object.keys(lastCall).filter(k => k.startsWith('naver_'));
        const naverValues = naverKeys.map(k => lastCall[k]);
        expect(naverValues).toContain('소설 한국소설');
        expect(naverValues).toContain('소설 SF');
    });

    it('검색 결과가 없는 서점은 카테고리에 포함되지 않는다', async () => {
        const onCategoriesFound = vi.fn();

        mockSearchResponses({
            '/search/bookstore/yes24': {
                status: 'success',
                result: [{ title: 'T', author: 'A', category: '소설 > 판타지', book_url: 'u' }]
            },
            // aladin, ridi는 기본값(not_found) 사용
        });

        await act(async () => {
            render(<Bookstore
                bookInfo={{ title: '테스트', author: '저자', isbn: '' }}
                searchTrigger={1}
                onCategoriesFound={onCategoriesFound}
            />);
        });

        await waitFor(() => {
            const calls = onCategoriesFound.mock.calls;
            const lastCall = calls[calls.length - 1]?.[0];
            expect(lastCall).toBeDefined();
            expect(Object.keys(lastCall).some(k => k.startsWith('yes24_'))).toBe(true);
        });

        const lastCall = onCategoriesFound.mock.calls[onCategoriesFound.mock.calls.length - 1][0];

        expect(Object.keys(lastCall).filter(k => k.startsWith('yes24_'))).toHaveLength(1);
        expect(Object.keys(lastCall).filter(k => k.startsWith('aladin_'))).toHaveLength(0);
        expect(Object.keys(lastCall).filter(k => k.startsWith('ridi_'))).toHaveLength(0);
    });
});
