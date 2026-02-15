// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, waitFor, cleanup, fireEvent } from '@testing-library/react';

afterEach(cleanup);

// mock 함수 호이스팅
const { mockJsonGetReq, mockJsonDeleteReq, mockJsonPostReq, mockJsonPutReq } = vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
    mockJsonDeleteReq: vi.fn(),
    mockJsonPostReq: vi.fn(),
    mockJsonPutReq: vi.fn(),
}));

vi.mock('../src/Common', () => ({
    jsonGetReq: mockJsonGetReq,
    jsonDeleteReq: mockJsonDeleteReq,
    jsonPostReq: mockJsonPostReq,
    jsonPutReq: mockJsonPutReq,
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

vi.mock('../src/categoryMappingCache', () => ({
    fetchCategoryMappings: vi.fn(() => Promise.resolve({})),
    updateCachedMappings: vi.fn(),
}));

import CategoryAdmin from '../src/CategoryAdmin';

// ── 헬퍼 ──

const CATEGORIES_RESPONSE = {
    '1_fiction': 10,
    '2_science': 8,
    '3_history': 5,
    '_root': 2,
};

const MISMATCH_RESPONSE_WITH_DATA = {
    mismatches: [{ category: '1_fiction', es_count: 10, fs_count: 8, diff: 2 }],
    es_only: [{ category: '2_science', es_count: 8 }],
    fs_only: [{ category: '4_fs_only_cat', fs_count: 9 }],
};

const MISMATCH_RESPONSE_EMPTY = {
    mismatches: [],
    es_only: [],
    fs_only: [],
};

const MAPPINGS_RESPONSE = {
    '1_fiction': ['소설', '문학'],
    '2_science': ['과학'],
};

const HIDDEN_RESPONSE = ['3_history'];

function setupMockResponses(categoriesResult, mismatchResult, {
    categoriesError, mismatchError, apiPrefix = '',
    mappingsResult = MAPPINGS_RESPONSE, hiddenResult = HIDDEN_RESPONSE,
} = {}) {
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
        if (url === apiPrefix + '/categories') {
            if (categoriesError) { reject(categoriesError); }
            else { resolve(categoriesResult); }
        } else if (url === apiPrefix + '/category-mismatches') {
            if (mismatchError) { reject(mismatchError); }
            else { resolve(mismatchResult); }
        } else if (url.startsWith('/category-mappings')) {
            resolve(mappingsResult);
        } else if (url.startsWith('/hidden-categories')) {
            resolve(hiddenResult);
        }
    });
}

describe('CategoryAdmin', () => {
    beforeEach(() => {
        mockJsonGetReq.mockReset();
        mockJsonDeleteReq.mockReset();
        mockJsonPostReq.mockReset();
        mockJsonPutReq.mockReset();
    });

    // ── 초기 렌더링 (접힌 상태) ──

    it('초기 상태에서 타이틀 헤더를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin title="책 카테고리 관리" />);
        await waitFor(() => {
            expect(screen.getByText('책 카테고리 관리')).toBeTruthy();
        });
    });

    // ── 펼치기/접기 ──

    it('헤더 클릭 시 카드가 펼쳐진다', async () => {
        setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryAdmin />);
        await waitFor(() => {
            expect(screen.getByText('카테고리 관리')).toBeTruthy();
        });
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText('카테고리 없음')).toBeTruthy();
        });
    });

    it('펼친 상태에서 헤더 클릭 시 접힌다', async () => {
        setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryAdmin />);
        await waitFor(() => {
            expect(screen.getByText('카테고리 관리')).toBeTruthy();
        });
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText('카테고리 없음')).toBeTruthy();
        });
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.queryByText('카테고리 없음')).toBeNull();
        });
    });

    it('카테고리가 있으면 트리 뷰로 펼쳐진다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        await waitFor(() => {
            expect(screen.getByText('카테고리 관리')).toBeTruthy();
        });
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });
    });

    // ── 데이터 로딩 ──

    it('4개 API를 모두 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryAdmin />);
        await waitFor(() => {
            expect(mockJsonGetReq).toHaveBeenCalledWith('/categories', null, expect.any(Function), expect.any(Function));
            expect(mockJsonGetReq).toHaveBeenCalledWith('/category-mismatches', null, expect.any(Function), expect.any(Function));
            expect(mockJsonGetReq).toHaveBeenCalledWith('/category-mappings?content_type=book', null, expect.any(Function), expect.any(Function));
            expect(mockJsonGetReq).toHaveBeenCalledWith('/hidden-categories?content_type=book', null, expect.any(Function), expect.any(Function));
        });
    });

    it('카테고리가 없으면 "카테고리 없음" 메시지를 표시한다', async () => {
        setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText('카테고리 없음')).toBeTruthy();
        });
    });

    it('트리에 모든 카테고리를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText('1_fiction')).toBeTruthy();
            expect(screen.getByText('2_science')).toBeTruthy();
            expect(screen.getByText('3_history')).toBeTruthy();
        });
    });

    it('fs_only 카테고리도 트리에 포함된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText('4_fs_only_cat')).toBeTruthy();
        });
    });

    it('_root 카테고리는 트리에서 제외된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });
        expect(screen.queryByText('_root')).toBeNull();
    });

    // ── 로딩 상태 ──

    it('API 응답 전에 펼치면 로딩 스피너를 표시한다', async () => {
        const resolvers = {};
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            resolvers[url] = resolve;
        });
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        expect(screen.getByText('로딩 중...')).toBeTruthy();

        // 4개 API 모두 resolve
        resolvers['/categories'](CATEGORIES_RESPONSE);
        resolvers['/category-mismatches'](MISMATCH_RESPONSE_EMPTY);
        resolvers['/category-mappings?content_type=book'](MAPPINGS_RESPONSE);
        resolvers['/hidden-categories?content_type=book'](HIDDEN_RESPONSE);

        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });
    });

    // ── 에러 처리 ──

    it('카테고리 API 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(null, MISMATCH_RESPONSE_EMPTY, { categoriesError: 'Network error' });
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText(/카테고리 목록을 불러올 수 없습니다/)).toBeTruthy();
        });
    });

    it('불일치 API 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, null, { mismatchError: 'Server error' });
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText(/불일치 데이터를 불러올 수 없습니다/)).toBeTruthy();
        });
    });

    it('에러 발생 시 로딩이 종료된다', async () => {
        setupMockResponses(null, MISMATCH_RESPONSE_EMPTY, { categoriesError: 'fail' });
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByText(/카테고리 목록을 불러올 수 없습니다/)).toBeTruthy();
        });
        expect(screen.queryByText('로딩 중...')).toBeNull();
    });

    // ── 폴더 클릭 시 불일치 항목 로딩 ──

    it('불일치가 있는 폴더 클릭 시 /category-mismatches/{id} API를 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        // 폴더 클릭용 mock 재설정
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [{ book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' }],
                    fs_only: [{ file_name: 'orphan.txt', file_path: '1_fiction/orphan.txt' }],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));

        await waitFor(() => {
            expect(mockJsonGetReq).toHaveBeenCalledWith(
                '/category-mismatches/1_fiction', null, expect.any(Function)
            );
        });

        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });
    });

    it('booksLoaded 플래그로 중복 API 호출을 방지한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        let categoryApiCallCount = 0;
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                categoryApiCallCount++;
                resolve({
                    es_only: [{ book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' }],
                    fs_only: [],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
        });
        expect(categoryApiCallCount).toBe(1);

        // 두 번째 클릭 — booksLoaded가 true이므로 API 호출 없음
        // 트리와 오른쪽 패널에 동일 텍스트가 있으므로 getAllByText 사용
        fireEvent.click(screen.getAllByText('1_fiction')[0]);
        await new Promise(r => setTimeout(r, 50));
        expect(categoryApiCallCount).toBe(1);
    });

    // ── ES-only 삭제 ──

    it('ES-only 항목 선택 시 삭제/편집/조회 버튼이 표시된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [{ book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' }],
                    fs_only: [],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('Book A.pdf'));
        await waitFor(() => {
            expect(screen.getByText('삭제')).toBeTruthy();
            expect(screen.getByText('편집')).toBeTruthy();
            expect(screen.getByText('조회')).toBeTruthy();
        });
    });

    it('삭제 버튼 클릭 시 DELETE /books/{bookId} API를 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [{ book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' }],
                    fs_only: [],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('Book A.pdf'));
        await waitFor(() => {
            expect(screen.getByText('삭제')).toBeTruthy();
        });

        mockJsonDeleteReq.mockImplementation((url, _payload, resolve) => {
            resolve('Ok');
        });

        fireEvent.click(screen.getByText('삭제'));
        await waitFor(() => {
            expect(mockJsonDeleteReq).toHaveBeenCalledWith(
                '/books/101', null, expect.any(Function), expect.any(Function)
            );
        });

        await waitFor(() => {
            expect(screen.getByText('책 정보가 삭제되었습니다.')).toBeTruthy();
        });
        expect(screen.queryByText('Book A.pdf')).toBeNull();
    });

    it('삭제 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [{ book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' }],
                    fs_only: [],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('Book A.pdf'));
        await waitFor(() => {
            expect(screen.getByText('삭제')).toBeTruthy();
        });

        mockJsonDeleteReq.mockImplementation((url, _payload, resolve, reject) => {
            reject('서버 오류');
        });

        fireEvent.click(screen.getByText('삭제'));
        await waitFor(() => {
            expect(screen.getByText('삭제 실패: 서버 오류')).toBeTruthy();
        });
        expect(screen.getAllByText('Book A.pdf').length).toBeGreaterThanOrEqual(1);
    });

    // ── FS-only 적재 ──

    it('FS-only 항목 선택 시 ES 적재/파일 삭제 버튼이 표시된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [],
                    fs_only: [{ file_name: 'orphan.txt', file_path: '1_fiction/orphan.txt' }],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('orphan.txt'));
        await waitFor(() => {
            expect(screen.getByText('ES 적재')).toBeTruthy();
            expect(screen.getByText('파일 삭제')).toBeTruthy();
        });
    });

    it('ES 적재 버튼 클릭 시 POST /category-mismatches/index-file API를 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [],
                    fs_only: [{ file_name: 'orphan.txt', file_path: '1_fiction/orphan.txt' }],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('orphan.txt'));
        await waitFor(() => {
            expect(screen.getByText('ES 적재')).toBeTruthy();
        });

        mockJsonPostReq.mockImplementation((url, payload, resolve) => {
            resolve({ book_id: 999 });
        });

        fireEvent.click(screen.getByText('ES 적재'));
        await waitFor(() => {
            expect(mockJsonPostReq).toHaveBeenCalledWith(
                '/category-mismatches/index-file',
                { file_path: '1_fiction/orphan.txt' },
                expect.any(Function),
                expect.any(Function)
            );
        });

        await waitFor(() => {
            expect(screen.getByText('ES에 적재되었습니다.')).toBeTruthy();
        });
        expect(screen.queryByText('orphan.txt')).toBeNull();
    });

    it('ES 적재 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [],
                    fs_only: [{ file_name: 'orphan.txt', file_path: '1_fiction/orphan.txt' }],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('orphan.txt'));
        await waitFor(() => {
            expect(screen.getByText('ES 적재')).toBeTruthy();
        });

        mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
            reject('적재 오류');
        });

        fireEvent.click(screen.getByText('ES 적재'));
        await waitFor(() => {
            expect(screen.getByText('ES 적재 실패: 적재 오류')).toBeTruthy();
        });
    });

    it('파일 삭제 버튼 클릭 시 POST /category-mismatches/delete-file API를 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [],
                    fs_only: [{ file_name: 'orphan.txt', file_path: '1_fiction/orphan.txt' }],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('orphan.txt'));
        await waitFor(() => {
            expect(screen.getByText('파일 삭제')).toBeTruthy();
        });

        mockJsonPostReq.mockImplementation((url, payload, resolve) => {
            resolve({ success: true });
        });

        fireEvent.click(screen.getByText('파일 삭제'));
        await waitFor(() => {
            expect(mockJsonPostReq).toHaveBeenCalledWith(
                '/category-mismatches/delete-file',
                { file_path: '1_fiction/orphan.txt' },
                expect.any(Function),
                expect.any(Function)
            );
        });

        await waitFor(() => {
            expect(screen.getByText('파일이 삭제되었습니다.')).toBeTruthy();
        });
        expect(screen.queryByText('orphan.txt')).toBeNull();
    });

    it('파일 삭제 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [],
                    fs_only: [{ file_name: 'orphan.txt', file_path: '1_fiction/orphan.txt' }],
                });
            } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
            else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
        });

        fireEvent.click(screen.getByText('1_fiction'));
        await waitFor(() => {
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('orphan.txt'));
        await waitFor(() => {
            expect(screen.getByText('파일 삭제')).toBeTruthy();
        });

        mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
            reject('삭제 실패');
        });

        fireEvent.click(screen.getByText('파일 삭제'));
        await waitFor(() => {
            expect(screen.getByText('파일 삭제 실패: 삭제 실패')).toBeTruthy();
        });
    });

    // ── 만화 contentType 테스트 ──

    describe('contentType="comic"', () => {
        it('만화 카테고리 관리 타이틀을 표시한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, { apiPrefix: '/comics' });
            render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
            await waitFor(() => {
                expect(screen.getByText('만화 카테고리 관리')).toBeTruthy();
            });
        });

        it('/comics prefix로 API를 호출한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, { apiPrefix: '/comics' });
            render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
            await waitFor(() => {
                expect(mockJsonGetReq).toHaveBeenCalledWith('/comics/categories', null, expect.any(Function), expect.any(Function));
                expect(mockJsonGetReq).toHaveBeenCalledWith('/comics/category-mismatches', null, expect.any(Function), expect.any(Function));
                expect(mockJsonGetReq).toHaveBeenCalledWith('/category-mappings?content_type=comic', null, expect.any(Function), expect.any(Function));
                expect(mockJsonGetReq).toHaveBeenCalledWith('/hidden-categories?content_type=comic', null, expect.any(Function), expect.any(Function));
            });
        });

        it('ES-only 항목 삭제 시 /comics/books/{id} API를 호출한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, { apiPrefix: '/comics' });
            render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
            fireEvent.click(screen.getByText('만화 카테고리 관리'));
            await waitFor(() => {
                expect(screen.getByRole('tree')).toBeTruthy();
            });

            mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
                if (url === '/comics/categories') resolve(CATEGORIES_RESPONSE);
                else if (url === '/comics/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
                else if (url.startsWith('/comics/category-mismatches/')) {
                    resolve({
                        es_only: [{ book_id: 201, title: 'Comic A', file_type: 'zip', file_path: '1_fiction/a.zip' }],
                        fs_only: [],
                    });
                } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
                else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
            });

            fireEvent.click(screen.getByText('1_fiction'));
            await waitFor(() => {
                expect(screen.getByText('Comic A.zip')).toBeTruthy();
            });

            fireEvent.click(screen.getByText('Comic A.zip'));
            await waitFor(() => {
                expect(screen.getByText('삭제')).toBeTruthy();
            });

            mockJsonDeleteReq.mockImplementation((url, _payload, resolve) => {
                resolve('Ok');
            });

            fireEvent.click(screen.getByText('삭제'));
            await waitFor(() => {
                expect(mockJsonDeleteReq).toHaveBeenCalledWith(
                    '/comics/books/201', null, expect.any(Function), expect.any(Function)
                );
            });

            await waitFor(() => {
                expect(screen.getByText('만화 정보가 삭제되었습니다.')).toBeTruthy();
            });
        });

        it('만화 정보 설명 텍스트를 올바르게 표시한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, { apiPrefix: '/comics' });
            render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
            fireEvent.click(screen.getByText('만화 카테고리 관리'));
            await waitFor(() => {
                expect(screen.getByRole('tree')).toBeTruthy();
            });

            mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
                if (url === '/comics/categories') resolve(CATEGORIES_RESPONSE);
                else if (url === '/comics/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
                else if (url.startsWith('/comics/category-mismatches/')) {
                    resolve({
                        es_only: [{ book_id: 201, title: 'Comic A', file_type: 'zip', file_path: '1_fiction/a.zip' }],
                        fs_only: [],
                    });
                } else if (url.startsWith('/category-mappings')) resolve(MAPPINGS_RESPONSE);
                else if (url.startsWith('/hidden-categories')) resolve(HIDDEN_RESPONSE);
            });

            fireEvent.click(screen.getByText('1_fiction'));
            await waitFor(() => {
                expect(screen.getByText('Comic A.zip')).toBeTruthy();
            });

            fireEvent.click(screen.getByText('Comic A.zip'));
            await waitFor(() => {
                expect(screen.getByText('만화 정보만 존재하고 파일시스템에는 존재하지 않습니다.')).toBeTruthy();
            });
        });
    });

    // ── 계층 구조 카테고리 ──

    it('슬래시가 포함된 계층 카테고리를 트리 구조로 표시한다', async () => {
        const categories = {
            'prefix/fiction': 10,
            'prefix/science': 8,
        };
        const mismatchData = {
            mismatches: [{ category: 'prefix/fiction', es_count: 10, fs_count: 8, diff: 2 }],
            es_only: [],
            fs_only: [],
        };
        setupMockResponses(categories, mismatchData);
        render(<CategoryAdmin />);
        fireEvent.click(screen.getByText('카테고리 관리'));
        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });
    });
});
