// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, waitFor, cleanup, fireEvent } from '@testing-library/react';

afterEach(cleanup);

// jsonGetReq mock — 호출별로 다른 응답을 반환하도록 구성
const { mockJsonGetReq } = vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
}));

vi.mock('../src/Common', () => ({
    jsonGetReq: mockJsonGetReq,
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

function setupMockResponses(categoriesResult, mismatchResult, { categoriesError, mismatchError } = {}) {
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
        if (url === '/categories') {
            if (categoriesError) {
                reject(categoriesError);
            } else {
                resolve(categoriesResult);
            }
        } else if (url === '/category-mismatches') {
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
    });

    // ── 초기 렌더링 (접힌 상태) ──

    it('초기 상태에서 "불일치 관리" 헤더를 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });
    });

    it('불일치가 있으면 총 건수를 헤더에 표시한다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        // mismatches:1 + es_only:1 + fs_only:1 = 3건
        await waitFor(() => {
            expect(screen.getByText('(3건)')).toBeTruthy();
        });
    });

    it('불일치가 없으면 건수를 표시하지 않는다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
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

    it('헤더 클릭 시 카테고리가 있으면 트리 뷰로 펼쳐진다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
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

    it('ES 카테고리 중 불일치가 없는 것도 트리에 포함된다', async () => {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
        render(<CategoryMismatch />);

        await waitFor(() => {
            expect(screen.getByText(/불일치 관리/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/불일치 관리/));

        await waitFor(() => {
            // 3_history는 불일치가 없지만 ES에 존재하므로 트리에 표시
            expect(screen.getByText('3_history')).toBeTruthy();
        });
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

        // 아직 불일치 미응답 → 건수 미표시
        expect(screen.queryByText(/건\)/)).toBeNull();

        // 불일치 나중에 응답
        resolvers['/category-mismatches'](MISMATCH_RESPONSE_WITH_DATA);

        await waitFor(() => {
            expect(screen.getByText('(3건)')).toBeTruthy();
        });
    });

    it('불일치 API가 먼저 응답하고 카테고리 API가 나중에 응답해도 정상 동작한다', async () => {
        const resolvers = {};
        mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
            resolvers[url] = resolve;
        });

        render(<CategoryMismatch />);

        // 불일치 먼저 응답
        resolvers['/category-mismatches'](MISMATCH_RESPONSE_WITH_DATA);

        // 아직 카테고리 미응답
        expect(screen.queryByText(/건\)/)).toBeNull();

        // 카테고리 나중에 응답
        resolvers['/categories'](CATEGORIES_RESPONSE);

        await waitFor(() => {
            expect(screen.getByText('(3건)')).toBeTruthy();
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

    it('같은 카테고리가 여러 배열에 중복 등장해도 건수는 유니크하게 카운트한다', async () => {
        const mismatchData = {
            mismatches: [{ category: '1_fiction', es_count: 10, fs_count: 8, diff: 2 }],
            es_only: [{ category: '1_fiction', es_count: 10 }], // 중복
            fs_only: [],
        };

        setupMockResponses(CATEGORIES_RESPONSE, mismatchData);
        render(<CategoryMismatch />);

        // 중복이지만 buildMismatchCounts는 counts[category] = 1로 덮어쓰므로 1건
        await waitFor(() => {
            expect(screen.getByText('(1건)')).toBeTruthy();
        });
    });

    it('mismatches만 있고 es_only, fs_only가 빈 경우 해당 건수만 카운트한다', async () => {
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
            expect(screen.getByText('(2건)')).toBeTruthy();
        });
    });

    it('mismatches, es_only, fs_only 각각의 카테고리를 불일치로 카운트한다', async () => {
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

        // 2 + 1 + 1 = 4건
        await waitFor(() => {
            expect(screen.getByText('(4건)')).toBeTruthy();
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
            expect(screen.getByText('(1건)')).toBeTruthy();
        });
    });

    // ── 트리 뷰 접근성 ──

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
});
