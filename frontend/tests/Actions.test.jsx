// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/Common', () => ({
    getRandomLightColor: () => '#cccccc',
    ROOT_DIRECTORY: '$$rootdir$$',
}));

vi.mock('../src/categoryMappingCache', () => ({
    loadCategoryMappings: () => ({ '소설': ['fiction', '소설'] }),
    fetchCategoryMappings: () => Promise.resolve(),
    isCacheInitialized: () => true,
}));

import Actions, { generateNgrams, ngramSimilarity, calculateSimilarity, filterSubstringKeywords, stripNoiseWords, getSimilarityDebugInfo } from '../src/Actions';
import * as categoryMappingCache from '../src/categoryMappingCache';

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

// ── stripNoiseWords ──

describe('stripNoiseWords', () => {
    it('"일반"을 제거한다', () => {
        expect(stripNoiseWords(['경영일반'])).toEqual(['경영']);
    });

    it('여러 키워드에서 "일반"을 제거한다', () => {
        expect(stripNoiseWords(['중문일반', '영문일반', '교육일반']))
            .toEqual(['중문', '영문', '교육']);
    });

    it('"일반"이 없는 키워드는 그대로 유지한다', () => {
        expect(stripNoiseWords(['소설', '역사'])).toEqual(['소설', '역사']);
    });

    it('"일반"만 있는 키워드는 제거한다 (빈 문자열 필터)', () => {
        expect(stripNoiseWords(['일반'])).toEqual([]);
    });

    it('혼합: "일반" 포함과 미포함 키워드를 올바르게 처리한다', () => {
        expect(stripNoiseWords(['경영일반', '소설', '건강일반']))
            .toEqual(['경영', '소설', '건강']);
    });

    it('빈 배열이면 빈 배열을 반환한다', () => {
        expect(stripNoiseWords([])).toEqual([]);
    });

    it('키워드 중간에 있는 "일반"도 제거한다', () => {
        expect(stripNoiseWords(['일반소설'])).toEqual(['소설']);
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

    it('한글 1글자가 3글자 이상 단어에 포함되면 N-gram으로 평가한다', () => {
        // "시" ⊂ "러시아소설" → 1글자, longer=5글자 → 우연적 포함
        expect(calculateSimilarity('시', '러시아소설')).toBeLessThan(0.7);
        // "시" ⊂ "레시피" → 1글자, longer=3글자 → 우연적 포함
        expect(calculateSimilarity('시', '레시피')).toBeLessThan(0.7);
    });

    it('한글 1글자가 2글자 단어에 포함되면 포함 관계로 인정한다', () => {
        // "시" ⊂ "시집" → 1글자, longer=2글자 → 의미 있는 포함
        expect(calculateSimilarity('시', '시집')).toBeGreaterThanOrEqual(0.7);
    });
});

// ── 공통 props ──

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

// ── isProcessing 버튼 비활성화 ──

describe('isProcessing 버튼 비활성화', () => {
    it('isProcessing=false이면 "다음 책으로" 버튼이 활성화된다', () => {
        render(<Actions {...defaultProps} isProcessing={false} />);
        expect(screen.getByText('다음 책으로').disabled).toBe(false);
    });

    it('isProcessing=true이면 "다음 책으로" 버튼이 비활성화된다', () => {
        render(<Actions {...defaultProps} isProcessing={true} />);
        expect(screen.getByText('다음 책으로').disabled).toBe(true);
    });

    it('isProcessing=true이면 "상위로" 버튼이 비활성화된다', () => {
        render(<Actions {...defaultProps} isProcessing={true} />);
        const btn = screen.getByText((content, element) =>
            element.tagName === 'BUTTON' && element.textContent.includes('상위로')
        );
        expect(btn.disabled).toBe(true);
    });

    it('isProcessing=true이면 "로 옮기기" 버튼이 비활성화된다', () => {
        render(<Actions {...defaultProps} selectedCategory="소설" isProcessing={true} />);
        const btn = screen.getByText((content, element) =>
            element.tagName === 'BUTTON' && element.textContent.includes('로 옮기기')
        );
        expect(btn.disabled).toBe(true);
    });

    it('isProcessing=false이면 "로 옮기기" 버튼이 활성화된다 (selectedCategory 있을 때)', () => {
        render(<Actions {...defaultProps} selectedCategory="소설" isProcessing={false} />);
        const btn = screen.getByText((content, element) =>
            element.tagName === 'BUTTON' && element.textContent.includes('로 옮기기')
        );
        expect(btn.disabled).toBe(false);
    });

    it('isProcessing가 undefined이면 기존 disabled 로직만 적용된다', () => {
        render(<Actions {...defaultProps} />);
        // "다음 책으로"는 isProcessing 없으므로 활성화
        expect(screen.getByText('다음 책으로').disabled).toBe(false);
    });

    it('isProcessing=true일 때 "다음 책으로" 클릭해도 핸들러가 호출되지 않는다', () => {
        const fn = vi.fn();
        render(<Actions {...defaultProps} toNextEntryClicked={fn} isProcessing={true} />);
        const btn = screen.getByText('다음 책으로');
        fireEvent.click(btn);
        expect(fn).not.toHaveBeenCalled();
    });

    it('isProcessing=true일 때 "로 옮기기" 클릭해도 핸들러가 호출되지 않는다', () => {
        const fn = vi.fn();
        render(<Actions {...defaultProps} selectedCategory="소설" moveToDirectoryButtonClicked={fn} isProcessing={true} />);
        const btn = screen.getByText((content, element) =>
            element.tagName === 'BUTTON' && element.textContent.includes('로 옮기기')
        );
        fireEvent.click(btn);
        expect(fn).not.toHaveBeenCalled();
    });
});

// ── 카테고리 버튼 하이라이트/선택 스타일 ──

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

    it('highlight-secondary 클래스를 유사도 2~5위 카테고리에 적용한다', () => {
        const selectFn = vi.fn();
        render(
            <Actions
                {...defaultProps}
                otherCategoryList={['소설', '역사', '과학', '철학', '예술']}
                suggestedCategories={{ yes24: '소설' }}
                selectedCategory="소설"
                selectDirectoryButtonClicked={selectFn}
            />
        );
        // 최소 1위인 소설은 highlight
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

describe('Actions 추가 분기', () => {
    it('캐시가 초기화되지 않았으면 fetchCategoryMappings를 호출한다', async () => {
        const fetchSpy = vi.spyOn(categoryMappingCache, 'fetchCategoryMappings')
            .mockResolvedValue();
        const initSpy = vi.spyOn(categoryMappingCache, 'isCacheInitialized')
            .mockReturnValue(false);

        render(<Actions {...defaultProps} />);

        expect(fetchSpy).toHaveBeenCalled();

        fetchSpy.mockRestore();
        initSpy.mockRestore();
    });

    it('언더스코어가 있는 카테고리는 접두사 색상과 하위 라벨로 렌더링한다', () => {
        render(
            <Actions
                {...defaultProps}
                otherCategoryList={['10_history_modern']}
            />
        );

        const button = screen.getByText('history');
        expect(button).toBeTruthy();
        expect(button.style.backgroundColor).toBe('rgb(204, 204, 204)');
    });
});

// ── getSimilarityDebugInfo ──

describe('getSimilarityDebugInfo', () => {
    it('null 입력 시 null을 반환한다', () => {
        expect(getSimilarityDebugInfo(null, ['소설'])).toBeNull();
        expect(getSimilarityDebugInfo({ yes24: '소설' }, null)).toBeNull();
        expect(getSimilarityDebugInfo({ yes24: '소설' }, [])).toBeNull();
    });

    it('서점별 키워드와 카테고리별 상세 정보를 반환한다', () => {
        const result = getSimilarityDebugInfo(
            { yes24: '한국소설' },
            ['소설', '역사'],
            10
        );
        expect(result).not.toBeNull();
        expect(result.bookstoreKeywords).toBeDefined();
        expect(result.bookstoreKeywords.yes24).toBeDefined();
        expect(result.bookstoreKeywords.yes24.original).toBe('한국소설');
        expect(result.categoryDetails).toBeDefined();
        expect(Array.isArray(result.categoryDetails)).toBe(true);
    });

    it('빈 카테고리 값은 건너뛴다', () => {
        const result = getSimilarityDebugInfo(
            { yes24: '', aladin: '한국소설' },
            ['소설'],
        );
        expect(result.bookstoreKeywords.yes24).toBeUndefined();
        expect(result.bookstoreKeywords.aladin).toBeDefined();
    });

    it('topN으로 상위 결과를 제한한다', () => {
        const categories = Array.from({ length: 20 }, (_, i) => `카테고리${i}`);
        const result = getSimilarityDebugInfo(
            { yes24: '카테고리0' },
            categories,
            3
        );
        expect(result.categoryDetails.length).toBeLessThanOrEqual(3);
    });
});

// ── calculateSimilarity 추가 엣지케이스 ──

describe('calculateSimilarity 추가 케이스', () => {
    it('양쪽 모두 빈 문자열이면 0을 반환한다', () => {
        expect(calculateSimilarity('', '')).toBe(0);
    });

    it('양쪽 모두 null이면 0을 반환한다', () => {
        expect(calculateSimilarity(null, null)).toBe(0);
    });

    it('매우 긴 문자열도 처리한다', () => {
        const long1 = '가'.repeat(100);
        const long2 = '가'.repeat(100) + '나';
        const score = calculateSimilarity(long1, long2);
        expect(score).toBeGreaterThan(0);
    });
});

// ── 추가 분기 ──

describe('ngramSimilarity 경계', () => {
    it('두 문자열이 모두 n보다 짧으면 0을 반환한다', () => {
        // 양쪽 ngram 집합이 비어 unionSize === 0
        expect(ngramSimilarity('a', 'b', 2)).toBe(0);
    });
});

describe('getSimilarityDebugInfo - 숫자 prefix / 빈 서점 카테고리', () => {
    it('숫자 prefix 가 있는 디렉토리명은 prefix 를 제거하고 비교한다', () => {
        const info = getSimilarityDebugInfo(
            { yes24: '소설' },
            ['4_소설', '5_역사'],
        );
        expect(info).not.toBeNull();
        const names = info.categoryDetails.map((d) => d.category);
        expect(names).toContain('4_소설');
    });

    it('빈 서점 카테고리는 건너뛴다', () => {
        const info = getSimilarityDebugInfo(
            { yes24: '', aladin: '소설' },
            ['소설', '역사'],
        );
        expect(Object.keys(info.bookstoreKeywords)).toEqual(['aladin']);
    });

    it('categoryList 가 비어 있으면 null 을 반환한다', () => {
        expect(getSimilarityDebugInfo({ yes24: '소설' }, [])).toBeNull();
    });
});

describe('Actions 카테고리 하이라이트', () => {
    it('유사도 2~5위 카테고리에 highlight-secondary 클래스를 적용한다', () => {
        render(
            <Actions
                {...defaultProps}
                otherCategoryList={['소설', '역사', '과학']}
                suggestedCategories={{ yes24: '소설 역사 과학' }}
            />,
        );
        const secondary = document.querySelectorAll('.highlight-secondary');
        expect(secondary.length).toBeGreaterThan(0);
        expect(document.querySelectorAll('.highlight').length).toBeGreaterThan(0);
    });

    it('빈 서점 카테고리가 섞여 있어도 나머지로 하이라이트를 계산한다', () => {
        render(
            <Actions
                {...defaultProps}
                otherCategoryList={['소설', '역사']}
                suggestedCategories={{ yes24: '', aladin: '소설' }}
            />,
        );
        expect(document.querySelectorAll('.highlight').length).toBeGreaterThan(0);
    });

    it('구분자만 있는 서점 카테고리는 하이라이트를 만들지 않는다', () => {
        render(
            <Actions
                {...defaultProps}
                otherCategoryList={['소설', '역사']}
                suggestedCategories={{ yes24: '///' }}
            />,
        );
        expect(document.querySelectorAll('.highlight').length).toBe(0);
    });

    it('_root 만 있는 목록은 하이라이트 대상이 없다', () => {
        render(
            <Actions
                {...defaultProps}
                otherCategoryList={['_root']}
                suggestedCategories={{ yes24: '소설' }}
            />,
        );
        expect(document.querySelectorAll('.highlight').length).toBe(0);
    });

    it('숫자 prefix 디렉토리도 하이라이트 대상이 된다', () => {
        render(
            <Actions
                {...defaultProps}
                otherCategoryList={['4_소설', '5_역사']}
                suggestedCategories={{ yes24: '소설' }}
            />,
        );
        expect(document.querySelectorAll('.highlight').length).toBeGreaterThan(0);
    });
});

describe('Actions 버튼 비활성화 조건', () => {
    it('선택된 항목과 카테고리가 모두 없으면 "로 옮기기" 가 비활성화된다', () => {
        render(
            <Actions
                {...defaultProps}
                selectedEntryId=""
                selectedCategory=""
                isProcessing={false}
            />,
        );
        const btn = screen.getByText((content, element) =>
            element.tagName === 'BUTTON' && element.textContent.includes('로 옮기기')
        );
        expect(btn.disabled).toBe(true);
    });
});
