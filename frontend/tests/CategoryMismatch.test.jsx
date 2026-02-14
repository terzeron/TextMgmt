// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, waitFor, cleanup, fireEvent } from '@testing-library/react';

afterEach(cleanup);

// mock 함수 호이스팅
const { mockJsonGetReq, mockJsonDeleteReq, mockJsonPostReq } = vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
    mockJsonDeleteReq: vi.fn(),
    mockJsonPostReq: vi.fn(),
}));

vi.mock('../src/Common', () => ({
    jsonGetReq: mockJsonGetReq,
    jsonDeleteReq: mockJsonDeleteReq,
    jsonPostReq: mockJsonPostReq,
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

// Folder의 CustomTreeItem은 MUI TreeView 내부 훅에 의존하므로 간단히 mock
vi.mock('../src/Folder', () => ({
    CustomTreeItem: undefined,
}));

import CategoryMismatch from '../src/CategoryMismatch';

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

function setupMockResponses(categoriesResult, mismatchResult, { categoriesError, mismatchError, apiPrefix = '' } = {}) {
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
        if (url === apiPrefix + '/categories') {
            if (categoriesError) {
                reject(categoriesError);
            } else {
                resolve(categoriesResult);
            }
        } else if (url === apiPrefix + '/category-mismatches') {
            if (mismatchError) {
                reject(mismatchError);
            } else {
                resolve(mismatchResult);
            }
        }
    });
}

describe('CategoryMismatch', () => {
    beforeEach(() => {
        mockJsonGetReq.mockReset();
        mockJsonDeleteReq.mockReset();
        mockJsonPostReq.mockReset();
    });

    // ── 초기 렌더링 (접힌 상태) ──

    it('초기 상태에서 "불일치 관리" 헤더를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });
    });

    it('헤더에 총 건수를 표시하지 않는다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });
        expect(screen.queryByText(/건\)/)).toBeNull();
    });

    // ── 펼치기/접기 ──

    it('헤더 클릭 시 카드가 펼쳐진다', async () => {
        // 카테고리가 비어있어야 folderData가 빈 배열이 되어 "불일치 없음" 표시
        setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText('불일치 없음')).toBeTruthy();
        });
    });

    it('펼친 상태에서 헤더 클릭 시 접힌다', async () => {
        setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        // 펼치기
        fireEvent.click(screen.getByText(/불일치 관리/));
        await waitFor(() => {
            expect(screen.getByText('불일치 없음')).toBeTruthy();
        });

        // 접기
        fireEvent.click(screen.getByText(/불일치 관리/));
        await waitFor(() => {
            expect(screen.queryByText('불일치 없음')).toBeNull();
        });
    });

    it('헤더 클릭 시 불일치 카테고리가 있으면 트리 뷰로 펼쳐진다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });
    });

    // ── 데이터 로딩 ──

    it('두 API를 모두 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(mockJsonGetReq).toHaveBeenCalledWith('/categories', null, expect.any(Function), expect.any(Function));
            expect(mockJsonGetReq).toHaveBeenCalledWith('/category-mismatches', null, expect.any(Function), expect.any(Function));
        });
    });

    it('불일치가 없으면 펼친 상태에서 "불일치 없음" 메시지를 표시한다', async () => {
        // 카테고리도 비어있으면 folderData가 빈 배열
        setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText('불일치 없음')).toBeTruthy();
        });
    });

    it('불일치 데이터가 있으면 트리 뷰를 렌더링한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });
    });

    // ── fs_only 카테고리 병합 ──

    it('fs_only 카테고리를 ES 카테고리와 병합하여 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            // fs_only 카테고리 '4_fs_only_cat'가 트리에 포함되어야 함
            expect(screen.getByText('4_fs_only_cat')).toBeTruthy();
        });
    });

    it('불일치가 없는 ES 카테고리는 트리에서 제외된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        // 3_history는 불일치가 없으므로 트리에서 제외
        expect(screen.queryByText('3_history')).toBeNull();
    });

    it('_root 카테고리는 트리에서 제외된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.queryByText('_root')).toBeNull();
        });
    });

    // ── 로딩 상태 ──

    it('API 응답 전에 펼치면 로딩 스피너를 표시한다', async () => {
        // resolve를 지연시켜 로딩 상태를 확인
        const resolvers = {};
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            resolvers[url] = resolve;
        });

        render(<CategoryMismatch />);

        // 헤더 클릭으로 펼치기
        fireEvent.click(screen.getByText(/불일치 관리/));

        // 로딩 중 텍스트 확인
        expect(screen.getByText('로딩 중...')).toBeTruthy();

        // resolve하여 로딩 완료
        resolvers['/categories'](CATEGORIES_RESPONSE);
        resolvers['/category-mismatches'](MISMATCH_RESPONSE_EMPTY);

        await waitFor(() => {
            expect(screen.queryByText('로딩 중...')).toBeNull();
        });
    });

    // ── 에러 처리 ──

    it('카테고리 API 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(null, MISMATCH_RESPONSE_EMPTY, { categoriesError: 'Network error' });
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText(/카테고리 목록을 불러올 수 없습니다/)).toBeTruthy();
        });
    });

    it('불일치 API 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, null, { mismatchError: 'Server error' });
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText(/불일치 데이터를 불러올 수 없습니다/)).toBeTruthy();
        });
    });

    it('양쪽 API 모두 실패 시 먼저 실패한 에러 메시지를 표시한다', async () => {
        setupMockResponses(null, null, {
            categoriesError: 'Network error',
            mismatchError: 'Server error',
        });
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            // 두 API 중 하나의 에러 메시지가 표시됨
            const hasError = screen.queryByText(/카테고리 목록을 불러올 수 없습니다/)
                || screen.queryByText(/불일치 데이터를 불러올 수 없습니다/);
            expect(hasError).toBeTruthy();
        });
    });

    it('에러 발생 시 로딩이 종료된다', async () => {
        setupMockResponses(null, MISMATCH_RESPONSE_EMPTY, { categoriesError: 'fail' });
        render(<CategoryMismatch />);

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText(/카테고리 목록을 불러올 수 없습니다/)).toBeTruthy();
        });
        // 스피너가 사라졌는지 확인
        expect(screen.queryByText('로딩 중...')).toBeNull();
    });

    // ── 비동기 응답 순서 ──

    it('카테고리 API가 먼저 응답하고 불일치 API가 나중에 응답해도 정상 동작한다', async () => {
        const resolvers = {};
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            resolvers[url] = resolve;
        });

        render(<CategoryMismatch />);

        // 카테고리 먼저 응답
        resolvers['/categories'](CATEGORIES_RESPONSE);

        // 불일치 나중에 응답
        resolvers['/category-mismatches'](MISMATCH_RESPONSE_WITH_DATA);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });
        // 건수는 표시하지 않음
        expect(screen.queryByText(/건\)/)).toBeNull();
    });

    it('불일치 API가 먼저 응답하고 카테고리 API가 나중에 응답해도 정상 동작한다', async () => {
        const resolvers = {};
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            resolvers[url] = resolve;
        });

        render(<CategoryMismatch />);

        // 불일치 먼저 응답
        resolvers['/category-mismatches'](MISMATCH_RESPONSE_WITH_DATA);

        // 카테고리 나중에 응답
        resolvers['/categories'](CATEGORIES_RESPONSE);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });
        // 건수는 표시하지 않음
        expect(screen.queryByText(/건\)/)).toBeNull();
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
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            // 부모 폴더 'prefix'가 표시되어야 함
            expect(screen.getByRole('tree')).toBeTruthy();
        });
    });

    // ── buildMismatchCounts 로직 검증 (통합) ──

    it('같은 카테고리가 여러 배열에 중복 등장해도 트리에 한 번만 표시한다', async () => {
        const mismatchData = {
            mismatches: [{ category: '1_fiction', es_count: 10, fs_count: 8, diff: 2 }],
            es_only: [{ category: '1_fiction', es_count: 10 }], // 중복
            fs_only: [],
        };

        setupMockResponses(CATEGORIES_RESPONSE, mismatchData);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
            expect(screen.getByText('1_fiction')).toBeTruthy();
        });
    });

    it('mismatches만 있고 es_only, fs_only가 빈 경우 해당 카테고리만 트리에 표시한다', async () => {
        const mismatchData = {
            mismatches: [
                { category: '1_fiction', es_count: 10, fs_count: 8, diff: 2 },
                { category: '2_science', es_count: 5, fs_count: 7, diff: -2 },
            ],
            es_only: [],
            fs_only: [],
        };

        setupMockResponses(CATEGORIES_RESPONSE, mismatchData);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText('1_fiction')).toBeTruthy();
            expect(screen.getByText('2_science')).toBeTruthy();
        });
    });

    it('mismatches, es_only, fs_only 각각의 카테고리를 트리에 표시한다', async () => {
        const mismatchData = {
            mismatches: [
                { category: '1_fiction', es_count: 10, fs_count: 8, diff: 2 },
                { category: '2_science', es_count: 5, fs_count: 7, diff: -2 },
            ],
            es_only: [{ category: '5_es_cat', es_count: 12 }],
            fs_only: [{ category: '6_fs_cat', fs_count: 9 }],
        };

        const categories = {
            '1_fiction': 10,
            '2_science': 5,
            '3_history': 3,
            '5_es_cat': 12,
        };

        setupMockResponses(categories, mismatchData);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText('1_fiction')).toBeTruthy();
            expect(screen.getByText('2_science')).toBeTruthy();
            expect(screen.getByText('5_es_cat')).toBeTruthy();
            expect(screen.getByText('6_fs_cat')).toBeTruthy();
        });
    });

    it('mismatch 응답에 일부 필드가 없어도(undefined) 오류 없이 동작한다', async () => {
        const mismatchData = {
            mismatches: [{ category: '1_fiction', es_count: 10, fs_count: 8, diff: 2 }],
            // es_only, fs_only 키 없음
        };

        setupMockResponses(CATEGORIES_RESPONSE, mismatchData);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByText('1_fiction')).toBeTruthy();
        });
    });

    // ── 트리 뷰 접근성 ──

    // ── 폴더 클릭 시 책 로딩 ──

    it('폴더 클릭 시 /categories/{categoryId} API를 호출하여 책 목록을 로딩한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        // 폴더 클릭을 위해 mock 재설정 — /category-mismatches/{id} 추가
        mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
            if (url === '/categories') {
                resolve(CATEGORIES_RESPONSE);
            } else if (url === '/category-mismatches') {
                resolve(MISMATCH_RESPONSE_WITH_DATA);
            } else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [
                        { book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' },
                    ],
                    fs_only: [
                        { file_name: 'orphan.txt', file_path: '1_fiction/orphan.txt' },
                    ],
                });
            }
        });

        // 트리에서 폴더 항목 클릭 (1_fiction은 불일치가 있는 카테고리)
        fireEvent.click(screen.getByText('1_fiction'));

        await waitFor(() => {
            expect(mockJsonGetReq).toHaveBeenCalledWith(
                '/category-mismatches/1_fiction', null, expect.any(Function)
            );
        });

        // 불일치 항목이 트리에 추가되었는지 확인
        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });
    });

    it('booksLoaded 플래그로 중복 API 호출을 방지한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        let categoryApiCallCount = 0;
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') {
                resolve(CATEGORIES_RESPONSE);
            } else if (url === '/category-mismatches') {
                resolve(MISMATCH_RESPONSE_WITH_DATA);
            } else if (url.startsWith('/category-mismatches/')) {
                categoryApiCallCount++;
                resolve({
                    es_only: [{ book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' }],
                    fs_only: [],
                });
            }
        });

        // 첫 번째 클릭
        fireEvent.click(screen.getByText('1_fiction'));

        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
        });

        expect(categoryApiCallCount).toBe(1);

        // 두 번째 클릭 — booksLoaded가 true이므로 API 호출 없음
        fireEvent.click(screen.getByText('1_fiction'));

        // 약간의 대기 후에도 카운트가 1인지 확인
        await new Promise(r => setTimeout(r, 50));
        expect(categoryApiCallCount).toBe(1);
    });

    it('가상 부모(isVirtualParent) 클릭 시 API를 호출하지 않는다', async () => {
        // 가상 부모가 생성되려면: 공통접두사가 없는 상태에서 A/fiction, A/science, B_other
        // 모두 불일치가 있어야 트리에 포함되어 가상 부모 생성됨
        const categories = {
            'A/fiction': 10,
            'A/science': 8,
            'B_other': 5,
        };
        const mismatchData = {
            mismatches: [
                { category: 'A/fiction', es_count: 10, fs_count: 8, diff: 2 },
                { category: 'A/science', es_count: 8, fs_count: 6, diff: 2 },
                { category: 'B_other', es_count: 5, fs_count: 3, diff: 2 },
            ],
            es_only: [],
            fs_only: [],
        };

        setupMockResponses(categories, mismatchData);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        // mock 재설정
        mockJsonGetReq.mockClear();
        mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
            if (url.startsWith('/categories/')) {
                resolve([]);
            }
        });

        // 가상 부모 'A' 클릭 (id: __virtual__A)
        const virtualParentItem = screen.getByText('A');
        fireEvent.click(virtualParentItem);

        // 약간의 대기 후 API 호출이 없는지 확인
        await new Promise(r => setTimeout(r, 50));
        expect(mockJsonGetReq).not.toHaveBeenCalled();
    });

    it('불일치가 없는 카테고리는 트리에 표시되지 않는다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        // 불일치가 있는 카테고리는 표시됨
        expect(screen.getByText('1_fiction')).toBeTruthy();
        expect(screen.getByText('2_science')).toBeTruthy();
        expect(screen.getByText('4_fs_only_cat')).toBeTruthy();

        // 불일치가 없는 카테고리는 표시되지 않음
        expect(screen.queryByText('3_history')).toBeNull();
    });

    it('트리 뷰에 "category mismatches" aria-label이 설정된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByLabelText('category mismatches')).toBeTruthy();
        });
    });

    // ── ES-only 삭제 ──

    it('ES-only 항목 선택 시 삭제 버튼이 표시된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            expect(screen.getByRole('tree')).toBeTruthy();
        });

        // 폴더 클릭 후 불일치 항목 로딩
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            if (url === '/categories') resolve(CATEGORIES_RESPONSE);
            else if (url === '/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
            else if (url.startsWith('/category-mismatches/')) {
                resolve({
                    es_only: [{ book_id: 101, title: 'Book A', file_type: 'pdf', file_path: '1_fiction/a.pdf' }],
                    fs_only: [],
                });
            }
        });

        fireEvent.click(screen.getByText('1_fiction'));

        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
        });

        // ES-only 항목 클릭
        fireEvent.click(screen.getByText('Book A.pdf'));

        await waitFor(() => {
            expect(screen.getByText('삭제')).toBeTruthy();
            expect(screen.getByText('편집')).toBeTruthy();
            expect(screen.getByText('조회')).toBeTruthy();
        });
    });

    it('삭제 버튼 클릭 시 DELETE /books/{bookId} API를 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

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
            }
        });

        fireEvent.click(screen.getByText('1_fiction'));

        await waitFor(() => {
            expect(screen.getByText('Book A.pdf')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('Book A.pdf'));

        await waitFor(() => {
            expect(screen.getByText('삭제')).toBeTruthy();
        });

        // 삭제 버튼 클릭
        mockJsonDeleteReq.mockImplementation((url, _payload, resolve) => {
            resolve('Ok');
        });

        fireEvent.click(screen.getByText('삭제'));

        await waitFor(() => {
            expect(mockJsonDeleteReq).toHaveBeenCalledWith(
                '/books/101', null, expect.any(Function), expect.any(Function)
            );
        });

        // 성공 메시지 표시
        await waitFor(() => {
            expect(screen.getByText('책 정보가 삭제되었습니다.')).toBeTruthy();
        });

        // 삭제된 항목이 트리에서 제거됨
        expect(screen.queryByText('Book A.pdf')).toBeNull();
    });

    // ── FS-only 적재 ──

    it('FS-only 항목 선택 시 ES 적재 버튼이 표시된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

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
            }
        });

        fireEvent.click(screen.getByText('1_fiction'));

        await waitFor(() => {
            expect(screen.getByText('orphan.txt')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('orphan.txt'));

        await waitFor(() => {
            expect(screen.getByText('ES 적재')).toBeTruthy();
        });
    });

    it('ES 적재 버튼 클릭 시 POST /category-mismatches/index-file API를 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

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
            }
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

    it('삭제 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

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
            }
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

        // 항목은 여전히 존재 (트리 + 선택 패널에 두 번 표시)
        expect(screen.getAllByText('Book A.pdf').length).toBeGreaterThanOrEqual(1);
    });

    it('ES 적재 실패 시 에러 메시지를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

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
            }
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

    it('FS-only 항목에서 파일 삭제 버튼 클릭 시 POST /category-mismatches/delete-file API를 호출한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

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
            }
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
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

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
            }
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
        it('만화 불일치 관리 타이틀을 표시한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, { apiPrefix: '/comics' });
            render(<CategoryMismatch contentType="comic" title="만화 불일치 관리" apiPrefix="/comics" />);

            await waitFor(() => {
                expect(screen.getByText('만화 불일치 관리')).toBeTruthy();
            });
        });

        it('/comics prefix로 API를 호출한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, { apiPrefix: '/comics' });
            render(<CategoryMismatch contentType="comic" title="만화 불일치 관리" apiPrefix="/comics" />);

            await waitFor(() => {
                expect(mockJsonGetReq).toHaveBeenCalledWith('/comics/categories', null, expect.any(Function), expect.any(Function));
                expect(mockJsonGetReq).toHaveBeenCalledWith('/comics/category-mismatches', null, expect.any(Function), expect.any(Function));
            });
        });

        it('ES-only 항목 삭제 시 /comics/books/{id} API를 호출한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, { apiPrefix: '/comics' });
            render(<CategoryMismatch contentType="comic" title="만화 불일치 관리" apiPrefix="/comics" />);

            await waitFor(() => {
                expect(screen.getByText('만화 불일치 관리')).toBeTruthy();
            });

            fireEvent.click(screen.getByText('만화 불일치 관리'));

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
                }
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

        it('FS-only 항목 ES 적재 시 /comics/category-mismatches/index-file API를 호출한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, { apiPrefix: '/comics' });
            render(<CategoryMismatch contentType="comic" title="만화 불일치 관리" apiPrefix="/comics" />);

            await waitFor(() => {
                expect(screen.getByText('만화 불일치 관리')).toBeTruthy();
            });

            fireEvent.click(screen.getByText('만화 불일치 관리'));

            await waitFor(() => {
                expect(screen.getByRole('tree')).toBeTruthy();
            });

            mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
                if (url === '/comics/categories') resolve(CATEGORIES_RESPONSE);
                else if (url === '/comics/category-mismatches') resolve(MISMATCH_RESPONSE_WITH_DATA);
                else if (url.startsWith('/comics/category-mismatches/')) {
                    resolve({
                        es_only: [],
                        fs_only: [{ file_name: 'orphan.zip', file_path: '1_fiction/orphan.zip' }],
                    });
                }
            });

            fireEvent.click(screen.getByText('1_fiction'));

            await waitFor(() => {
                expect(screen.getByText('orphan.zip')).toBeTruthy();
            });

            fireEvent.click(screen.getByText('orphan.zip'));

            await waitFor(() => {
                expect(screen.getByText('ES 적재')).toBeTruthy();
            });

            mockJsonPostReq.mockImplementation((url, payload, resolve) => {
                resolve({ book_id: 999 });
            });

            fireEvent.click(screen.getByText('ES 적재'));

            await waitFor(() => {
                expect(mockJsonPostReq).toHaveBeenCalledWith(
                    '/comics/category-mismatches/index-file',
                    { file_path: '1_fiction/orphan.zip' },
                    expect.any(Function),
                    expect.any(Function)
                );
            });
        });

        it('만화 정보 설명 텍스트를 올바르게 표시한다', async () => {
            setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, { apiPrefix: '/comics' });
            render(<CategoryMismatch contentType="comic" title="만화 불일치 관리" apiPrefix="/comics" />);

            await waitFor(() => {
                expect(screen.getByText('만화 불일치 관리')).toBeTruthy();
            });

            fireEvent.click(screen.getByText('만화 불일치 관리'));

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
                }
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
});
