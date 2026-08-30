// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/react';

afterEach(cleanup);

import SearchResult from '../src/SearchResult';

const sampleResults = [
    { book_id: 1, category: '소설', file_path: '/books/novel1.epub', file_type: 'epub' },
    { book_id: 2, category: '역사', file_path: '/books/history/ancient.pdf', file_type: 'pdf' },
];

describe('SearchResult', () => {
    it('검색 결과를 렌더링한다', () => {
        render(<SearchResult results={sampleResults} />);
        expect(screen.getByText('소설/novel1.epub')).toBeTruthy();
        expect(screen.getByText('역사/ancient.pdf')).toBeTruthy();
    });

    it('_root 카테고리인 경우 접두어를 제외하고 파일명만 표시한다', () => {
        const rootResults = [
            { book_id: 3, category: '_root', file_path: '/books/root_novel.epub', file_type: 'epub' },
            { book_id: 4, category: '', file_path: '/books/empty_cat.epub', file_type: 'epub' }
        ];
        render(<SearchResult results={rootResults} />);
        expect(screen.getByText('root_novel.epub')).toBeTruthy();
        expect(screen.getByText('empty_cat.epub')).toBeTruthy();
    });

    it('결과가 없으면 "검색 결과가 없습니다" 메시지를 표시한다', () => {
        render(<SearchResult results={[]} />);
        expect(screen.getByText('검색 결과가 없습니다.')).toBeTruthy();
    });

    it('헤더와 빈 메시지를 커스터마이즈할 수 있다', () => {
        render(<SearchResult results={[]} title="최신 책" emptyMessage="최신 책이 없습니다." />);
        expect(screen.getByText('최신 책')).toBeTruthy();
        expect(screen.getByText('최신 책이 없습니다.')).toBeTruthy();
    });

    it('헤더 클릭으로 결과를 접을 수 있다', () => {
        render(<SearchResult results={sampleResults} />);
        expect(screen.getByText('소설/novel1.epub')).toBeTruthy();

        // 헤더 클릭하여 접기
        fireEvent.click(screen.getByText('검색 결과'));
        expect(screen.queryByText('소설/novel1.epub')).toBeNull();

        // 다시 클릭하여 펼치기
        fireEvent.click(screen.getByText('검색 결과'));
        expect(screen.getByText('소설/novel1.epub')).toBeTruthy();
    });

    it('showEditButton=true일 때 편집 버튼을 표시한다', () => {
        render(<SearchResult results={sampleResults} showEditButton={true} />);
        const editButtons = screen.getAllByText('편집');
        expect(editButtons.length).toBe(2);
    });

    it('role="admin"일 때 편집 버튼을 표시한다', () => {
        render(<SearchResult results={sampleResults} role="admin" />);
        const editButtons = screen.getAllByText('편집');
        expect(editButtons.length).toBe(2);
    });

    it('role="viewer"일 때 편집 버튼을 숨긴다', () => {
        render(<SearchResult results={sampleResults} role="viewer" />);
        expect(screen.queryByText('편집')).toBeNull();
        const viewAllButtons = screen.getAllByText('전체보기');
        expect(viewAllButtons.length).toBe(2);
    });

    it('showEditButton=false일 때 편집 버튼을 숨기고 전체보기 버튼을 표시한다', () => {
        render(<SearchResult results={sampleResults} showEditButton={false} />);
        expect(screen.queryByText('편집')).toBeNull();
        const viewAllButtons = screen.getAllByText('전체보기');
        expect(viewAllButtons.length).toBe(2);
    });

    it('조회 버튼을 항상 표시한다', () => {
        render(<SearchResult results={sampleResults} />);
        const viewButtons = screen.getAllByText('조회');
        expect(viewButtons.length).toBe(2);
    });

    it('hasMore=true일 때 "더 보기" 버튼을 표시한다', () => {
        const onLoadMore = vi.fn();
        render(<SearchResult results={sampleResults} hasMore={true} onLoadMore={onLoadMore} />);
        const loadMoreBtn = screen.getByText('더 보기');
        expect(loadMoreBtn).toBeTruthy();
        fireEvent.click(loadMoreBtn);
        expect(onLoadMore).toHaveBeenCalledOnce();
    });

    it('hasMore=false일 때 "더 보기" 버튼을 숨긴다', () => {
        render(<SearchResult results={sampleResults} hasMore={false} />);
        expect(screen.queryByText('더 보기')).toBeNull();
    });

    it('loading=true일 때 "로딩 중..." 텍스트를 표시하고 클릭을 무시한다', () => {
        const onLoadMore = vi.fn();
        render(<SearchResult results={sampleResults} hasMore={true} loading={true} onLoadMore={onLoadMore} />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
        fireEvent.click(screen.getByText('로딩 중...'));
        expect(onLoadMore).not.toHaveBeenCalled();
    });

    it('results가 undefined이면 빈 결과로 처리한다', () => {
        render(<SearchResult />);
        expect(screen.getByText('검색 결과가 없습니다.')).toBeTruthy();
    });

    it('편집 버튼 클릭 시 window.open을 호출한다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} showEditButton={true} />);
        const editButtons = screen.getAllByText('편집');
        fireEvent.click(editButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            `/book-edit/1?category=${encodeURIComponent('소설')}`,
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });

    it('조회 버튼 클릭 시 window.open을 호출한다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} showEditButton={true} />);
        const viewButtons = screen.getAllByText('조회');
        fireEvent.click(viewButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            `/book-view/1?category=${encodeURIComponent('소설')}`,
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });

    it('전체보기 버튼 클릭 시 window.open을 호출한다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} showEditButton={false} />);
        const viewAllButtons = screen.getAllByText('전체보기');
        fireEvent.click(viewAllButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            `/viewer/epub/1?path=${encodeURIComponent('/books/novel1.epub')}`,
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });

    it('더 보기에서 Enter 키로 onLoadMore를 호출한다', () => {
        const onLoadMore = vi.fn();
        render(<SearchResult results={sampleResults} hasMore={true} onLoadMore={onLoadMore} />);
        const loadMore = screen.getByText('더 보기').closest('[role="button"]');
        fireEvent.keyDown(loadMore, { key: 'Enter' });
        expect(onLoadMore).toHaveBeenCalledOnce();
    });

    it('더 보기에서 Space 키로 onLoadMore를 호출한다', () => {
        const onLoadMore = vi.fn();
        render(<SearchResult results={sampleResults} hasMore={true} onLoadMore={onLoadMore} />);
        const loadMore = screen.getByText('더 보기').closest('[role="button"]');
        fireEvent.keyDown(loadMore, { key: ' ' });
        expect(onLoadMore).toHaveBeenCalledOnce();
    });

    it('loading=true이면 키보드 이벤트를 무시한다', () => {
        const onLoadMore = vi.fn();
        render(<SearchResult results={sampleResults} hasMore={true} loading={true} onLoadMore={onLoadMore} />);
        const loadMore = screen.getByText('로딩 중...').closest('[role="button"]');
        fireEvent.keyDown(loadMore, { key: 'Enter' });
        expect(onLoadMore).not.toHaveBeenCalled();
    });

    it('새 결과가 들어오면 자동으로 펼친다', () => {
        const { rerender } = render(<SearchResult results={sampleResults} />);

        // 접기
        fireEvent.click(screen.getByText('검색 결과'));
        expect(screen.queryByText('소설/novel1.epub')).toBeNull();

        // 새 결과로 rerender → 자동 펼침
        const newResults = [
            { book_id: 3, category: '과학', file_path: '/books/science.pdf', file_type: 'pdf' },
        ];
        rerender(<SearchResult results={newResults} />);
        expect(screen.getByText('과학/science.pdf')).toBeTruthy();
    });

    // ── 만화 컨텍스트 (basePath="/comics-edit") ──

    it('basePath="/comics-edit" 편집 버튼 클릭 시 comics-edit URL로 열린다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} showEditButton={true} basePath="/comics-edit" />);
        const editButtons = screen.getAllByText('편집');
        fireEvent.click(editButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            `/comics-edit/1?category=${encodeURIComponent('소설')}`,
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });

    it('basePath="/comics-edit" 조회 버튼 클릭 시 comics-view URL로 열린다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} showEditButton={true} basePath="/comics-edit" />);
        const viewButtons = screen.getAllByText('조회');
        fireEvent.click(viewButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            `/comics-view/1?category=${encodeURIComponent('소설')}`,
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });

    it('basePath="/comics-view" 전체보기 버튼 클릭 시 api 파라미터가 포함된다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} showEditButton={false} basePath="/comics-view" />);
        const viewAllButtons = screen.getAllByText('전체보기');
        fireEvent.click(viewAllButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            expect.stringContaining('&api=%2Fcomics'),
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });

    it('basePath="/book-view" 전체보기 버튼 클릭 시 api 파라미터가 포함되지 않는다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} showEditButton={false} basePath="/book-view" />);
        const viewAllButtons = screen.getAllByText('전체보기');
        fireEvent.click(viewAllButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            expect.not.stringContaining('&api='),
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });

    it('basePath="/book-view"에서 role="admin"일 때 편집 버튼 클릭 시 book-edit URL로 열린다', () => {
        const mockOpen = vi.fn();
        vi.stubGlobal('open', mockOpen);
        render(<SearchResult results={sampleResults} role="admin" basePath="/book-view" />);
        const editButtons = screen.getAllByText('편집');
        fireEvent.click(editButtons[0]);
        expect(mockOpen).toHaveBeenCalledWith(
            `/book-edit/1?category=${encodeURIComponent('소설')}`,
            '_blank',
            'noopener'
        );
        vi.unstubAllGlobals();
    });
});
