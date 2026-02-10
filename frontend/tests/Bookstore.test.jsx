// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';

vi.mock('../src/Common', () => ({
    rawJsonGetReq: vi.fn(),
    getApiUrlPrefix: () => 'http://localhost:8000',
    handleFetchErrors: (r) => r,
    getRandomMediumColor: () => '#000',
    ROOT_DIRECTORY: '$$rootdir$$',
}));

import { getTwoLevelCategory, extractMultiPathCategories } from '../src/Bookstore';

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
