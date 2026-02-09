// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';

vi.mock('../src/Common', () => ({
    getRandomMediumColor: () => '#000000',
    ROOT_DIRECTORY: '$$rootdir$$',
}));

vi.mock('../src/CategoryMapping', () => ({
    loadCategoryMappings: () => ({}),
    fetchCategoryMappings: () => Promise.resolve(),
    isCacheInitialized: () => false,
}));

import { generateNgrams, ngramSimilarity, calculateSimilarity } from '../src/Actions';

// ── generateNgrams ──

describe('generateNgrams', () => {
    it('bigram 집합을 생성한다', () => {
        const result = generateNgrams('hello', 2);
        expect(result).toEqual(new Set(['he', 'el', 'll', 'lo']));
    });

    it('문자열이 n보다 짧으면 빈 집합을 반환한다', () => {
        expect(generateNgrams('a', 2)).toEqual(new Set());
    });

    it('빈 문자열이면 빈 집합을 반환한다', () => {
        expect(generateNgrams('', 2)).toEqual(new Set());
    });

    it('문자열 길이가 n과 같으면 하나의 ngram을 반환한다', () => {
        expect(generateNgrams('ab', 2)).toEqual(new Set(['ab']));
    });

    it('trigram을 생성할 수 있다', () => {
        expect(generateNgrams('abcd', 3)).toEqual(new Set(['abc', 'bcd']));
    });
});

// ── ngramSimilarity ──

describe('ngramSimilarity', () => {
    it('동일한 문자열이면 1.0을 반환한다', () => {
        expect(ngramSimilarity('hello', 'hello')).toBe(1.0);
    });

    it('완전히 다른 문자열이면 0을 반환한다', () => {
        expect(ngramSimilarity('abc', 'xyz')).toBe(0);
    });

    it('부분적으로 겹치면 0과 1 사이 값을 반환한다', () => {
        const score = ngramSimilarity('abcd', 'abef');
        expect(score).toBeGreaterThan(0);
        expect(score).toBeLessThan(1);
    });

    it('n보다 짧은 동일 문자열이면 1.0을 반환한다', () => {
        expect(ngramSimilarity('a', 'a', 2)).toBe(1.0);
    });

    it('n보다 짧은 다른 문자열이면 0을 반환한다', () => {
        expect(ngramSimilarity('a', 'b', 2)).toBe(0);
    });

    it('Jaccard 유사도 공식이 정확하다', () => {
        // 'abcd' → {ab, bc, cd}, 'abef' → {ab, be, ef}
        // intersection: {ab} → 1, union: 3+3-1 = 5
        // Jaccard = 1/5 = 0.2
        expect(ngramSimilarity('abcd', 'abef')).toBeCloseTo(0.2);
    });
});

// ── calculateSimilarity ──

describe('calculateSimilarity', () => {
    it('완전 일치하면 1.0을 반환한다', () => {
        expect(calculateSimilarity('hello', 'hello')).toBe(1.0);
    });

    it('대소문자 무관하게 완전 일치하면 1.0을 반환한다', () => {
        expect(calculateSimilarity('Hello', 'HELLO')).toBe(1.0);
    });

    it('포함 관계이면 0.7~0.9 범위를 반환한다', () => {
        const score = calculateSimilarity('SF', 'SF소설');
        expect(score).toBeGreaterThanOrEqual(0.7);
        expect(score).toBeLessThanOrEqual(0.9);
    });

    it('한글 1글자도 포함 관계로 인정한다', () => {
        const score = calculateSimilarity('소', '소설');
        expect(score).toBeGreaterThanOrEqual(0.7);
    });

    it('비한글 1글자는 포함 관계로 인정하지 않는다', () => {
        const score = calculateSimilarity('S', 'SF');
        expect(score).toBeLessThan(0.7);
    });

    it('N-gram 유사도가 있으면 0.3~0.6 범위를 반환한다', () => {
        // '프로그래밍' vs '프로그래머': 공통 bigram 3/5 = 0.6
        // → 0.3 + 0.6*0.3 = 0.48
        const score = calculateSimilarity('프로그래밍', '프로그래머');
        expect(score).toBeGreaterThanOrEqual(0.3);
        expect(score).toBeLessThanOrEqual(0.6);
    });

    it('완전히 다른 문자열이면 0을 반환한다', () => {
        expect(calculateSimilarity('abc', 'xyz')).toBe(0);
    });

    it('빈 문자열이면 0을 반환한다', () => {
        expect(calculateSimilarity('', 'hello')).toBe(0);
        expect(calculateSimilarity('hello', '')).toBe(0);
    });

    it('null/undefined이면 0을 반환한다', () => {
        expect(calculateSimilarity(null, 'hello')).toBe(0);
        expect(calculateSimilarity('hello', undefined)).toBe(0);
    });
});
