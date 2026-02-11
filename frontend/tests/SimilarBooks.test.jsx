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

        // score >= 90이므로 자동 펼침
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

        // score >= 90인 책이 있으므로 자동 펼침
        await waitFor(() => {
            expect(screen.getByText('95')).toBeTruthy();
            expect(screen.getByText('72')).toBeTruthy();
        });
    });

    // ── 자동 펼침 ──

    it('90점 이상인 책이 있으면 자동으로 펼쳐진다', async () => {
        mockBooks([makeBook(1, 91), makeBook(2, 60)]);

        render(<SimilarBooks bookId={1} />);

        // 클릭 없이도 자동으로 펼쳐져 책 목록이 보여야 함
        await waitFor(() => {
            expect(screen.getByText('91')).toBeTruthy();
            expect(screen.getByText('60')).toBeTruthy();
        });
    });

    it('90점 미만이면 접힌 상태를 유지한다', async () => {
        mockBooks([makeBook(1, 89), makeBook(2, 50)]);

        render(<SimilarBooks bookId={1} />);

        // 접힌 상태이므로 책 목록이 보이지 않아야 함
        expect(screen.queryByText('편집')).toBeNull();
    });

    it('정확히 90점이면 자동으로 펼쳐진다 (경계값)', async () => {
        mockBooks([makeBook(1, 90)]);

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('90')).toBeTruthy();
        });
    });

    it('89.9점이면 자동으로 펼쳐지지 않는다 (경계값)', () => {
        mockBooks([makeBook(1, 89.9)]);

        render(<SimilarBooks bookId={1} />);

        expect(screen.queryByText('편집')).toBeNull();
    });

    it('bookId 변경 시 자동 펼침 상태가 초기화된다', async () => {
        let callCount = 0;
        mockRawJsonGetReq.mockImplementation((url, resolve) => {
            callCount++;
            if (callCount === 1) {
                // 첫 번째 책: 90점 이상 → 자동 펼침
                resolve({ status: 'success', result: [makeBook(1, 95)], total: 1 });
            } else {
                // 두 번째 책: 90점 미만 → 접힘
                resolve({ status: 'success', result: [makeBook(2, 50)], total: 1 });
            }
        });

        const { rerender } = render(<SimilarBooks bookId={1} />);

        // 첫 번째 책: 자동 펼침
        await waitFor(() => {
            expect(screen.getByText('95')).toBeTruthy();
        });

        // bookId 변경 → 접힌 상태로 초기화
        rerender(<SimilarBooks bookId={2} />);

        await waitFor(() => {
            expect(screen.queryByText('95')).toBeNull();
        });
        // 90점 미만이므로 접힌 상태
        expect(screen.queryByText('50')).toBeNull();
        expect(screen.queryByText('편집')).toBeNull();
    });

    it('자동 펼침 후 헤더 클릭으로 닫을 수 있다', async () => {
        mockBooks([makeBook(1, 95)]);

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('95')).toBeTruthy();
        });

        // 헤더 클릭으로 닫기
        fireEvent.click(screen.getByText('유사한 책 목록'));
        expect(screen.queryByText('95')).toBeNull();
    });

    // ── 90점 이상 하이라이트 ──

    it('90점 이상인 책 행에 highlight-secondary 클래스가 적용된다', async () => {
        mockBooks([makeBook(1, 95), makeBook(2, 70)]);

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('95')).toBeTruthy();
        });

        const row95 = screen.getByText('95').closest('div[class]');
        const row70 = screen.getByText('70').closest('div[class]');
        expect(row95.classList.contains('highlight-secondary')).toBe(true);
        expect(row70.classList.contains('highlight-secondary')).toBe(false);
    });

    it('정확히 90점이면 highlight-secondary가 적용된다', async () => {
        mockBooks([makeBook(1, 90)]);

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('90')).toBeTruthy();
        });

        const row = screen.getByText('90').closest('div[class]');
        expect(row.classList.contains('highlight-secondary')).toBe(true);
    });

    it('89점이면 highlight-secondary가 적용되지 않는다', async () => {
        mockBooks([makeBook(1, 89)]);

        render(<SimilarBooks bookId={1} />);
        fireEvent.click(screen.getByText('유사한 책 목록'));

        await waitFor(() => {
            expect(screen.getByText('89')).toBeTruthy();
        });

        const row = screen.getByText('89').closest('div[class]');
        expect(row.classList.contains('highlight-secondary')).toBe(false);
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

    // ── onSelect 콜백 ──

    it('책 항목 클릭 시 onSelect을 category/book_id로 호출한다', async () => {
        mockBooks([makeBook(42, 95)]);
        const onSelect = vi.fn();

        render(<SimilarBooks bookId={1} onSelect={onSelect} />);

        await waitFor(() => {
            expect(screen.getByText(/Book 42\.pdf/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/Book 42\.pdf/));
        expect(onSelect).toHaveBeenCalledWith('test_category/42');
    });

    it('onSelect이 없어도 책 항목 클릭 시 에러가 발생하지 않는다', async () => {
        mockBooks([makeBook(1, 95)]);

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText(/Book 1\.pdf/)).toBeTruthy();
        });

        expect(() => {
            fireEvent.click(screen.getByText(/Book 1\.pdf/));
        }).not.toThrow();
    });

    // ── 더 보기 ──

    it('total > 표시된 수일 때 "더 보기" 버튼을 표시한다', async () => {
        mockRawJsonGetReq.mockImplementation((url, resolve) => {
            resolve({ status: 'success', result: [makeBook(1, 95)], total: 5 });
        });

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('더 보기')).toBeTruthy();
        });
    });

    it('total === 표시된 수이면 "더 보기" 버튼을 표시하지 않는다', async () => {
        mockBooks([makeBook(1, 95)]);

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('95')).toBeTruthy();
        });

        expect(screen.queryByText('더 보기')).toBeNull();
    });

    it('"더 보기" 클릭 시 추가 데이터를 로드한다', async () => {
        let callCount = 0;
        mockRawJsonGetReq.mockImplementation((url, resolve) => {
            callCount++;
            if (callCount === 1) {
                resolve({ status: 'success', result: [makeBook(1, 95)], total: 2 });
            } else {
                resolve({ status: 'success', result: [makeBook(2, 80)], total: 2 });
            }
        });

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText('더 보기')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('더 보기'));

        await waitFor(() => {
            expect(screen.getByText('80')).toBeTruthy();
        });

        // 추가 로드 후 total과 같아지면 "더 보기" 사라짐
        expect(screen.queryByText('더 보기')).toBeNull();
    });

    // ── API 호출 ──

    it('올바른 URL로 API를 호출한다', () => {
        mockBooks([]);

        render(<SimilarBooks bookId={42} />);

        expect(mockRawJsonGetReq).toHaveBeenCalledWith(
            '/similar/42?offset=0&limit=10',
            expect.any(Function),
            expect.any(Function)
        );
    });

    it('bookId 변경 시 새 API를 호출한다', () => {
        mockBooks([]);

        const { rerender } = render(<SimilarBooks bookId={1} />);
        expect(mockRawJsonGetReq).toHaveBeenCalledWith(
            '/similar/1?offset=0&limit=10',
            expect.any(Function),
            expect.any(Function)
        );

        rerender(<SimilarBooks bookId={99} />);
        expect(mockRawJsonGetReq).toHaveBeenCalledWith(
            '/similar/99?offset=0&limit=10',
            expect.any(Function),
            expect.any(Function)
        );
    });

    it('API 에러 시 console.error를 호출한다', () => {
        const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
        mockRawJsonGetReq.mockImplementation((url, resolve, reject) => {
            reject(new Error('Network error'));
        });

        render(<SimilarBooks bookId={1} />);

        expect(consoleError).toHaveBeenCalled();
        consoleError.mockRestore();
    });

    // ── 파일명 표시 ──

    it('file_path에서 파일명만 추출하여 표시한다', async () => {
        mockBooks([{
            ...makeBook(1, 95),
            file_path: 'deep/nested/category/MyBook.epub',
        }]);

        render(<SimilarBooks bookId={1} />);

        await waitFor(() => {
            expect(screen.getByText(/MyBook\.epub/)).toBeTruthy();
        });

        // 전체 경로가 아닌 파일명만 표시
        expect(screen.queryByText(/deep\/nested/)).toBeNull();
    });
});
