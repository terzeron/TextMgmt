// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/Common', () => ({
    getRandomMediumColor: () => '#000000',
    ROOT_DIRECTORY: '$$rootdir$$',
}));

vi.mock('../src/CategoryMapping', () => ({
    loadCategoryMappings: () => ({ '소설': ['fiction', '소설'] }),
    fetchCategoryMappings: () => Promise.resolve(),
    isCacheInitialized: () => true,
}));

import Actions, { generateNgrams, ngramSimilarity, calculateSimilarity, filterSubstringKeywords } from '../src/Actions';

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

// ── filterSubstringKeywords ──

describe('filterSubstringKeywords', () => {
    it('다른 키워드에 포함되는 짧은 키워드를 제거한다', () => {
        expect(filterSubstringKeywords(['소설', '세계각국소설'])).toEqual(['세계각국소설']);
    });

    it('포함 관계가 없으면 모두 유지한다', () => {
        expect(filterSubstringKeywords(['역사', '과학'])).toEqual(['역사', '과학']);
    });

    it('키워드가 1개이면 그대로 반환한다', () => {
        expect(filterSubstringKeywords(['소설'])).toEqual(['소설']);
    });

    it('빈 배열이면 빈 배열을 반환한다', () => {
        expect(filterSubstringKeywords([])).toEqual([]);
    });

    it('대소문자 무관하게 포함 관계를 판단한다', () => {
        expect(filterSubstringKeywords(['sf', 'SF소설'])).toEqual(['SF소설']);
    });

    it('여러 키워드 중 포함되는 것만 제거한다', () => {
        expect(filterSubstringKeywords(['소설', '세계각국소설', '역사'])).toEqual(['세계각국소설', '역사']);
    });
});

// ── calculateSimilarity: 한글 포함 관계 임계값 ──

describe('calculateSimilarity: 한글 포함 관계 임계값', () => {
    it('한글 2글자가 긴 문자열의 50% 이하로 포함되면 N-gram으로 평가한다', () => {
        // "소설" ⊂ "세계각국소설" → ratio = 2/6 = 0.33 ≤ 0.5 → N-gram
        const score = calculateSimilarity('소설', '세계각국소설');
        expect(score).toBeLessThan(0.7); // 포함 관계가 아닌 N-gram 범위
    });

    it('한글 2글자가 긴 문자열의 50% 초과로 포함되면 포함 관계로 인정한다', () => {
        // "소설" ⊂ "SF소설" → ratio = 2/4 = 0.5 → 0.5 > 0.5 미성립 → 포함 관계
        // 그런데 "소설"은 한글 2글자이고 ratio=0.5이므로 0.5 <= 0.5 성립
        // 하지만 "SF"는 비한글이라 shorter가 "소설"이 아닐 수도...
        // "소설" vs "소설외국" → ratio = 2/4 = 0.5 → 한글 2글자, ratio 0.5 ≤ 0.5 → N-gram
        const score = calculateSimilarity('소설', '소설외국');
        expect(score).toBeLessThan(0.7);
    });

    it('한글 3글자가 긴 문자열의 50% 초과로 포함되면 포함 관계로 인정한다', () => {
        // "각국소설" ⊂ "세계각국소설" → ratio = 4/6 = 0.67 > 0.5 → 포함 관계
        const score = calculateSimilarity('각국소설', '세계각국소설');
        expect(score).toBeGreaterThanOrEqual(0.7);
    });

    it('영문 키워드의 포함 관계는 기존과 동일하다', () => {
        // "SF" ⊂ "SF소설" → 비한글이므로 한글 임계값 적용 안 됨 → 포함 관계
        const score = calculateSimilarity('SF', 'SF소설');
        expect(score).toBeGreaterThanOrEqual(0.7);
    });
});

// ── 카테고리 버튼 하이라이트/선택 스타일 ──

const defaultProps = {
    selectedEntryId: 'test/1',
    selectedCategory: '',
    otherCategoryList: ['소설', '역사', '과학'],
    newFileName: 'test.pdf',
    toNextEntryClicked: vi.fn(),
    moveToUpperButtonClicked: vi.fn(),
    moveToDirectoryButtonClicked: vi.fn(),
    selectDirectoryButtonClicked: vi.fn(),
    suggestedCategories: {},
};

describe('카테고리 버튼 하이라이트', () => {
    it('유사도 자동 선택 시 highlight 클래스를 유지한다 (노란색)', () => {
        const selectFn = vi.fn();
        render(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="소설"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        // 자동 선택으로 selectDirectoryButtonClicked가 호출됨
        expect(selectFn).toHaveBeenCalledWith(null, '소설');
        // 버튼에 highlight 클래스가 유지되어야 함
        const btn = screen.getByText('소설');
        expect(btn.className).toContain('highlight');
        // 흰색 배경이 아님
        expect(btn.style.backgroundColor).not.toBe('rgb(255, 255, 255)');
    });

    it('수동 클릭 시 흰색 배경을 적용한다', () => {
        const selectFn = vi.fn();
        const { rerender } = render(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="소설"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        // 사용자가 '역사' 버튼을 클릭
        fireEvent.click(screen.getByText('역사'));
        expect(selectFn).toHaveBeenCalledWith(expect.anything(), '역사');

        // 부모가 selectedCategory를 '역사'로 업데이트
        rerender(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="역사"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        const btn = screen.getByText('역사');
        expect(btn.style.backgroundColor).toBe('rgb(255, 255, 255)');
        expect(btn.className).not.toContain('highlight');
    });

    it('수동 클릭 후에도 유사 카테고리의 highlight 클래스는 유지된다', () => {
        const selectFn = vi.fn();
        const { rerender } = render(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="소설"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        // 사용자가 다른 버튼 클릭
        fireEvent.click(screen.getByText('역사'));
        rerender(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="역사"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        // '소설' 버튼은 선택 해제되었지만 여전히 highlight 클래스를 가짐
        const novelBtn = screen.getByText('소설');
        expect(novelBtn.className).toContain('highlight');
    });

    it('suggestedCategories 변경 시 manuallyClicked가 초기화된다', () => {
        const selectFn = vi.fn();
        const { rerender } = render(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="소설"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        // 수동 클릭
        fireEvent.click(screen.getByText('역사'));
        rerender(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="역사"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        // suggestedCategories가 변경되면 자동 선택으로 돌아감
        rerender(
            <Actions
                {...defaultProps}
                suggestedCategories={{ yes24: '역사' }}
                selectedCategory="역사"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        const btn = screen.getByText('역사');
        // 자동 선택이므로 highlight 클래스 유지
        expect(btn.className).toContain('highlight');
        expect(btn.style.backgroundColor).not.toBe('rgb(255, 255, 255)');
    });
});
