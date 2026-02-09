// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockRawJsonGetReq } = vi.hoisted(() => ({
    mockRawJsonGetReq: vi.fn(),
}));

vi.mock('../src/Common', () => ({
    rawJsonGetReq: mockRawJsonGetReq,
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

import SimilarBooks from '../src/SimilarBooks';

const makeBook = (id, score = 0) => ({
    book_id: id,
    category: 'test_category',
    title: `Book ${id}`,
    author: `Author ${id}`,
    file_path: `test_category/Book ${id}.pdf`,
    file_type: 'pdf',
    file_size: 1000,
    score,
    updated_time: '2025-01-01T00:00:00.000000',
});

const mockBooks = (books) => {
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
        resolve({ status: 'success', result: books, total: books.length });
    });
};

describe('SimilarBooks', () => {
    beforeEach(() => {
        mockRawJsonGetReq.mockReset();
    });

    // ── 점수 배지 표시 ──

    it('score > 0일 때 점수 배지를 표시한다', async () => {
        mockBooks([makeBook(1, 87.4)]);

        render(<SimilarBooks bookId={1} />);
        fireEvent.click(screen.getByText('유사한 책 목록'));

        await waitFor(() => {
            expect(screen.getByText('87')).toBeTruthy();
        });
    });

    it('score가 소수일 때 반올림하여 표시한다', async () => {
        mockBooks([makeBook(1, 92.6)]);

        render(<SimilarBooks bookId={1} />);
        fireEvent.click(screen.getByText('유사한 책 목록'));

        await waitFor(() => {
            expect(screen.getByText('93')).toBeTruthy();
        });
    });

    it('score === 0이면 점수 배지를 표시하지 않는다', async () => {
        mockBooks([makeBook(1, 0)]);

        render(<SimilarBooks bookId={1} />);
        fireEvent.click(screen.getByText('유사한 책 목록'));

        await waitFor(() => {
            // 책 항목은 렌더링됨
            expect(screen.getByText('편집')).toBeTruthy();
        });

        // 점수 0은 배지로 표시되지 않아야 함
        expect(screen.queryByText('0')).toBeNull();
    });

    it('여러 책의 점수 배지가 각각 표시된다', async () => {
        mockBooks([makeBook(1, 95), makeBook(2, 72), makeBook(3, 0)]);

        render(<SimilarBooks bookId={1} />);
        fireEvent.click(screen.getByText('유사한 책 목록'));

        await waitFor(() => {
            expect(screen.getByText('95')).toBeTruthy();
            expect(screen.getByText('72')).toBeTruthy();
        });
    });

    // ── 기본 동작 ──

    it('bookId가 없으면 API를 호출하지 않는다', () => {
        render(<SimilarBooks />);
        expect(mockRawJsonGetReq).not.toHaveBeenCalled();
    });

    it('헤더 클릭으로 목록을 열고 닫을 수 있다', async () => {
        mockBooks([makeBook(1, 50)]);

        render(<SimilarBooks bookId={1} />);

        // 닫힌 상태에서는 책 목록이 보이지 않음
        expect(screen.queryByText('편집')).toBeNull();

        // 열기
        fireEvent.click(screen.getByText('유사한 책 목록'));
        await waitFor(() => {
            expect(screen.getByText('편집')).toBeTruthy();
        });

        // 닫기
        fireEvent.click(screen.getByText('유사한 책 목록'));
        expect(screen.queryByText('편집')).toBeNull();
    });

    it('유사한 책이 없으면 안내 메시지를 표시한다', async () => {
        mockBooks([]);

        render(<SimilarBooks bookId={1} />);
        fireEvent.click(screen.getByText('유사한 책 목록'));

        await waitFor(() => {
            expect(screen.getByText('유사한 책이 없습니다.')).toBeTruthy();
        });
    });
});
