// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/categoryMappingCache', () => ({
    loadCategoryMappings: () => ({}),
    fetchCategoryMappings: () => Promise.resolve(),
    isCacheInitialized: () => true,
}));

// getSimilarityDebugInfo를 제어 가능하게 mock
const mockGetSimilarityDebugInfo = vi.fn();
vi.mock('../src/Actions', () => ({
    getSimilarityDebugInfo: (...args) => mockGetSimilarityDebugInfo(...args),
}));

import SimilarityDebug from '../src/SimilarityDebug';

// ── 헬퍼: 디버그 데이터 생성 ──

const makeDebugInfo = (bookstoreKeywords, categoryDetails = []) => ({
    bookstoreKeywords,
    categoryDetails,
});

// ── 기본 렌더링 ──

describe('SimilarityDebug 기본 렌더링', () => {
    it('debugInfo가 null이면 아무것도 렌더링하지 않는다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(null);
        const { container } = render(
            <SimilarityDebug suggestedCategories={{}} categoryList={['소설']} />
        );
        expect(container.innerHTML).toBe('');
    });

    it('카드 헤더를 클릭하면 본문이 열린다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { yes24_0_0: { original: '소설', keywords: ['소설'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ yes24_0_0: '소설' }}
                categoryList={['소설']}
            />
        );
        // 헤더 존재 확인
        expect(screen.getByText('유사도 계산 디버그')).toBeTruthy();
        // 본문은 아직 안 보임
        expect(screen.queryByText('서점 카테고리에서 추출된 키워드:')).toBeNull();
        // 헤더 클릭
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        expect(screen.getByText('서점 카테고리에서 추출된 키워드:')).toBeTruthy();
    });
});

// ── 서점 뱃지 표시 (핵심 수정 부분) ──

describe('서점 뱃지 올바른 이름 표시', () => {
    it('yes24 키는 "yes24"로 표시한다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { yes24_0_0: { original: '소설 한국소설', keywords: ['한국소설'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ yes24_0_0: '소설 한국소설' }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        const badges = screen.getAllByText('yes24');
        expect(badges.length).toBeGreaterThanOrEqual(1);
    });

    it('aladin 키는 "aladin"으로 표시한다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { aladin_0_0: { original: '문학 한국문학', keywords: ['한국문학'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ aladin_0_0: '문학 한국문학' }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        const badges = screen.getAllByText('aladin');
        expect(badges.length).toBeGreaterThanOrEqual(1);
    });

    it('ridi 키는 "ridi"로 표시한다 (이전에는 aladin으로 잘못 표시됨)', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { ridi_0_0: { original: '경영 경영일반', keywords: ['경영일반'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ ridi_0_0: '경영 경영일반' }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        const ridiBadges = screen.getAllByText('ridi');
        expect(ridiBadges.length).toBeGreaterThanOrEqual(1);
        // "aladin"이 표시되지 않아야 함
        expect(screen.queryByText('aladin')).toBeNull();
    });

    it('세 서점이 혼합되면 각각 올바른 이름으로 표시된다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo({
            yes24_0_0: { original: '예술 대중문화론', keywords: ['대중문화론'] },
            aladin_0_0: { original: '예술/대중문화 예술/대중문화의 이해', keywords: ['대중문화의이해'] },
            ridi_0_0: { original: '경영/경제 경영일반', keywords: ['경영일반'] },
        }));
        render(
            <SimilarityDebug
                suggestedCategories={{
                    yes24_0_0: '예술 대중문화론',
                    aladin_0_0: '예술/대중문화 예술/대중문화의 이해',
                    ridi_0_0: '경영/경제 경영일반',
                }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        expect(screen.getAllByText('yes24').length).toBeGreaterThanOrEqual(1);
        expect(screen.getAllByText('aladin').length).toBeGreaterThanOrEqual(1);
        expect(screen.getAllByText('ridi').length).toBeGreaterThanOrEqual(1);
    });
});

// ── 뱃지 색상 ──

describe('서점 뱃지 색상', () => {
    it('yes24 뱃지는 보라색(#6A1B9A)이다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { yes24_0_0: { original: '소설', keywords: ['소설'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ yes24_0_0: '소설' }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        const badge = screen.getByText('yes24');
        expect(badge.style.backgroundColor).toBe('rgb(106, 27, 154)');
    });

    it('aladin 뱃지는 파란색(#0D47A1)이다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { aladin_0_0: { original: '소설', keywords: ['소설'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ aladin_0_0: '소설' }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        const badge = screen.getByText('aladin');
        expect(badge.style.backgroundColor).toBe('rgb(13, 71, 161)');
    });

    it('ridi 뱃지는 틸색(#00897B)이다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { ridi_0_0: { original: '소설', keywords: ['소설'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ ridi_0_0: '소설' }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        const badge = screen.getByText('ridi');
        expect(badge.style.backgroundColor).toBe('rgb(0, 137, 123)');
    });

    it('알 수 없는 서점은 회색(#455A64) 폴백을 사용한다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo(
            { unknown_0_0: { original: '소설', keywords: ['소설'] } }
        ));
        render(
            <SimilarityDebug
                suggestedCategories={{ unknown_0_0: '소설' }}
                categoryList={['소설']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        const badge = screen.getByText('unknown');
        expect(badge.style.backgroundColor).toBe('rgb(69, 90, 100)');
    });
});

// ── 매칭 상세 테이블의 서점 뱃지 ──

describe('매칭 상세 뱃지 표시', () => {
    it('categoryDetails의 matchDetails에서도 올바른 서점 이름을 표시한다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue({
            bookstoreKeywords: {
                ridi_0_0: { original: '경영 경영일반', keywords: ['경영일반'] },
            },
            categoryDetails: [{
                category: '4_경제',
                categoryName: '경제',
                dirKeywords: ['경제'],
                totalScore: 0.50,
                matchDetails: [
                    { store: 'ridi_0_0', bookstoreKeyword: '경영일반', dirKeyword: '경제', similarity: 0.35 },
                ],
            }],
        });
        render(
            <SimilarityDebug
                suggestedCategories={{ ridi_0_0: '경영 경영일반' }}
                categoryList={['4_경제']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        // 매칭 상세에서도 "ridi"로 표시 (이전에는 "aladin"으로 잘못 표시)
        const ridiBadges = screen.getAllByText('ridi');
        // 키워드 테이블 + 매칭 상세 = 최소 2개
        expect(ridiBadges.length).toBe(2);
    });

    it('여러 서점의 매칭 상세가 각각 올바른 이름으로 표시된다', () => {
        mockGetSimilarityDebugInfo.mockReturnValue({
            bookstoreKeywords: {
                yes24_0_0: { original: '예술 대중문화론', keywords: ['대중문화론'] },
                aladin_0_0: { original: '예술 대중문화', keywords: ['대중문화'] },
            },
            categoryDetails: [{
                category: '5_미술예술건축',
                categoryName: '미술예술건축',
                dirKeywords: ['미술', '예술', '건축'],
                totalScore: 0.43,
                matchDetails: [
                    { store: 'yes24_0_0', bookstoreKeyword: '대중문화론', dirKeyword: '미술예술건축', similarity: 0.36 },
                    { store: 'aladin_0_0', bookstoreKeyword: '대중문화', dirKeyword: '미술예술건축', similarity: 0.36 },
                ],
            }],
        });
        render(
            <SimilarityDebug
                suggestedCategories={{
                    yes24_0_0: '예술 대중문화론',
                    aladin_0_0: '예술 대중문화',
                }}
                categoryList={['5_미술예술건축']}
            />
        );
        fireEvent.click(screen.getByText('유사도 계산 디버그'));
        // 매칭 상세에서 yes24, aladin 각각 표시
        expect(screen.getAllByText('yes24').length).toBe(2); // 키워드 + 매칭
        expect(screen.getAllByText('aladin').length).toBe(2); // 키워드 + 매칭
    });
});
