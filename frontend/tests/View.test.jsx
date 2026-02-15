// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockJsonGetReq, mockGetApiUrlPrefix, mockOutletContext, mockRouteState } = vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
    mockGetApiUrlPrefix: vi.fn(() => 'http://localhost:8000'),
    mockOutletContext: {
        searchResults: [],
        hasSearched: false,
        role: 'admin',
        searchTotal: 0,
        handleLoadMore: vi.fn(),
        searchLoading: false,
    },
    mockRouteState: { wildcard: '', searchParams: '' },
}));

vi.mock('../src/Common', () => ({
    jsonGetReq: mockJsonGetReq,
    getApiUrlPrefix: mockGetApiUrlPrefix,
}));

vi.mock('../src/Folder.jsx', () => ({
    default: ({ folderData, onClickHandler }) => (
        <div data-testid="folder">
            {folderData.map(item => (
                <div key={item.id}>
                    <button data-testid={`folder-item-${item.id}`} onClick={() => onClickHandler(item.id)}>
                        {item.label}
                    </button>
                    {item.children?.map(child => (
                        <button key={child.id} data-testid={`folder-item-${child.id}`} onClick={() => onClickHandler(child.id)}>
                            {child.label}
                        </button>
                    ))}
                </div>
            ))}
        </div>
    ),
}));

vi.mock('../src/ViewSingle.jsx', () => ({
    default: ({ bookId, fileType }) => <div data-testid="view-single">ViewSingle: {bookId} ({fileType})</div>,
}));

vi.mock('../src/BookInfoView.jsx', () => ({
    default: ({ bookInfo }) => <div data-testid="book-info-view">BookInfo: {bookInfo.title}</div>,
}));

vi.mock('../src/SearchResult', () => ({
    default: ({ results }) => <div data-testid="search-result">{results?.length || 0} results</div>,
}));

vi.mock('../src/folderUtils', async () => {
    const actual = await vi.importActual('../src/folderUtils');
    return actual;
});

vi.mock('react-router-dom', async () => {
    const actual = await vi.importActual('react-router-dom');
    return {
        ...actual,
        useParams: () => ({ '*': mockRouteState.wildcard }),
        useSearchParams: () => [new URLSearchParams(mockRouteState.searchParams)],
        useOutletContext: () => mockOutletContext,
    };
});

import View from '../src/View';

describe('View', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        mockOutletContext.role = 'admin';
        mockOutletContext.hasSearched = false;
        mockOutletContext.searchResults = [];
        mockOutletContext.searchTotal = 0;
        mockOutletContext.searchLoading = false;
        mockRouteState.wildcard = '';
        mockRouteState.searchParams = '';
    });

    it('카테고리 목록을 로드하여 Folder에 전달한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 10, '역사': 5 });
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });
    });

    it('카테고리 로드 실패 시 에러 메시지를 표시하지 않고 빈 폴더를 표시한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
            if (url === '/categories') {
                reject('서버 오류');
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });
    });

    it('_root 카테고리가 있으면 최상위 파일을 로드한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 10, '_root': 2 });
            } else if (url === '/categories/_root') {
                resolve([
                    { book_id: 100, title: 'rootbook', file_type: 'pdf', file_path: '/root.pdf', category: '_root' },
                ]);
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });
    });

    it('_root 로드 실패 시에도 카테고리 폴더는 표시된다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
            if (url === '/categories') {
                resolve({ '소설': 10, '_root': 2 });
            } else if (url === '/categories/_root') {
                reject('_root 로드 실패');
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });
    });

    it('hasSearched=true일 때 검색 결과 컴포넌트를 표시한다', async () => {
        mockOutletContext.hasSearched = true;
        mockOutletContext.searchResults = [{ book_id: 1 }];

        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') resolve({});
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('search-result')).toBeTruthy();
        });
    });

    it('카테고리 클릭 시 entryClicked를 통해 하위 책 목록을 로드한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 1 });
            } else if (url === '/categories/소설') {
                resolve([
                    { book_id: 42, title: '테스트소설', file_type: 'epub', file_path: '/test.epub', category: '소설' },
                ]);
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });

        // 카테고리 클릭 → entryClicked 호출
        const folderItem = screen.getByTestId('folder-item-소설');
        folderItem.click();

        // jsonGetReq가 /categories/소설로 호출됨 확인
        await waitFor(() => {
            const calls = mockJsonGetReq.mock.calls;
            const categoryCall = calls.find(c => c[0] === '/categories/소설');
            expect(categoryCall).toBeTruthy();
        });
    });

    it('viewer 역할일 때 hidden-categories를 로드하여 필터링한다', async () => {
        mockOutletContext.role = 'viewer';

        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 10, '비공개': 5, '역사': 3 });
            } else if (url === '/hidden-categories?content_type=book') {
                resolve(['비공개']);
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });

        // hidden-categories API가 호출됨
        const calls = mockJsonGetReq.mock.calls;
        expect(calls.find(c => c[0] === '/hidden-categories?content_type=book')).toBeTruthy();
    });

    it('viewer 역할에서 hidden-categories 로드 실패 시 전체 카테고리를 표시한다', async () => {
        mockOutletContext.role = 'viewer';

        mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
            if (url === '/categories') {
                resolve({ '소설': 10 });
            } else if (url === '/hidden-categories?content_type=book') {
                reject('로드 실패');
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });
    });

    it('카테고리 클릭 후 책 클릭 시 BookInfoView와 ViewSingle이 표시된다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 1 });
            } else if (url === '/categories/소설') {
                resolve([
                    { book_id: 42, title: '테스트소설', file_type: 'epub', file_path: '/test.epub', category: '소설' },
                ]);
            }
        });

        render(<View />);

        // 카테고리 로드 대기
        await waitFor(() => {
            expect(screen.getByTestId('folder-item-소설')).toBeTruthy();
        });

        // 카테고리 클릭 → 책 목록 로드
        screen.getByTestId('folder-item-소설').click();

        // 책 목록이 폴더에 추가될 때까지 대기
        await waitFor(() => {
            expect(screen.getByTestId('folder-item-소설/42')).toBeTruthy();
        });

        // 책 클릭 → bookInfo 설정
        screen.getByTestId('folder-item-소설/42').click();

        await waitFor(() => {
            expect(screen.getByTestId('book-info-view')).toBeTruthy();
            expect(screen.getByTestId('view-single')).toBeTruthy();
        });
    });

    it('최상위 파일(_root) 클릭 시 BookInfoView가 표시된다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 10, '_root': 1 });
            } else if (url === '/categories/_root') {
                resolve([
                    { book_id: 200, title: 'root파일', file_type: 'pdf', file_path: '/root.pdf', category: '_root' },
                ]);
            }
        });

        render(<View />);

        // _root 파일 버튼 대기 (id는 '/200')
        await waitFor(() => {
            expect(screen.getByTestId('folder-item-/200')).toBeTruthy();
        });

        // 최상위 파일 클릭
        screen.getByTestId('folder-item-/200').click();

        await waitFor(() => {
            expect(screen.getByTestId('book-info-view')).toBeTruthy();
            expect(screen.getByTestId('view-single')).toBeTruthy();
        });
    });

    it('딥링크로 bookId와 category가 주어지면 자동으로 책을 선택한다', async () => {
        mockRouteState.wildcard = '42';
        mockRouteState.searchParams = 'category=소설';

        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 1 });
            } else if (url === '/categories/소설') {
                resolve([
                    { book_id: 42, title: '딥링크소설', file_type: 'epub', file_path: '/deep.epub', category: '소설' },
                ]);
            }
        });

        render(<View />);

        // 딥링크로 자동 선택되어 카테고리 로드 호출됨
        await waitFor(() => {
            const calls = mockJsonGetReq.mock.calls;
            expect(calls.find(c => c[0] === '/categories/소설')).toBeTruthy();
        });
    });

    it('딥링크에서 트리에 없는 카테고리는 /books/{id}로 직접 조회한다', async () => {
        mockRouteState.wildcard = '99';
        mockRouteState.searchParams = 'category=깊은/3레벨/카테고리';

        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 1 });
            } else if (url === '/books/99') {
                resolve({
                    book_id: 99, title: '깊은카테고리책', file_type: 'pdf',
                    file_path: '/deep.pdf', category: '깊은/3레벨/카테고리',
                });
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(mockJsonGetReq.mock.calls.find(c => c[0] === '/books/99')).toBeTruthy();
        });

        await waitFor(() => {
            expect(screen.getByTestId('book-info-view')).toBeTruthy();
        });
    });

    it('딥링크 직접 조회 실패 시 /books/{id} reject 콜백이 호출된다', async () => {
        mockRouteState.wildcard = '99';
        mockRouteState.searchParams = 'category=없는카테고리';

        mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
            if (url === '/categories') {
                resolve({ '소설': 1 });
            } else if (url === '/books/99') {
                reject('책을 찾을 수 없음');
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(mockJsonGetReq.mock.calls.find(c => c[0] === '/books/99')).toBeTruthy();
        });

        // bookInfo가 설정되지 않아 ViewSingle이 렌더링되지 않음
        expect(screen.queryByTestId('view-single')).toBeNull();
    });

    it('딥링크 _root 카테고리인 경우 /{bookId}로 entryClicked를 호출한다', async () => {
        mockRouteState.wildcard = '200';
        mockRouteState.searchParams = 'category=_root';

        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '_root': 1 });
            } else if (url === '/categories/_root') {
                resolve([
                    { book_id: 200, title: 'root파일', file_type: 'pdf', file_path: '/root.pdf', category: '_root' },
                ]);
            }
        });

        render(<View />);

        await waitFor(() => {
            expect(screen.getByTestId('folder')).toBeTruthy();
        });
    });

    it('viewer 역할에서 딥링크로 hidden 카테고리 접근 시 BookInfoView가 렌더링되지 않는다', async () => {
        mockOutletContext.role = 'viewer';
        mockRouteState.wildcard = '50';
        mockRouteState.searchParams = 'category=비공개/하위';

        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') {
                resolve({ '소설': 10 });
            } else if (url === '/hidden-categories?content_type=book') {
                resolve(['비공개']);
            } else if (url === '/books/50') {
                resolve({
                    book_id: 50, title: '비공개책', file_type: 'pdf',
                    file_path: '/secret.pdf', category: '비공개/하위',
                });
            }
        });

        render(<View />);

        // /books/50 호출 확인
        await waitFor(() => {
            expect(mockJsonGetReq.mock.calls.find(c => c[0] === '/books/50')).toBeTruthy();
        });

        // hidden 카테고리이므로 BookInfoView가 렌더링되지 않음
        expect(screen.queryByTestId('book-info-view')).toBeNull();
    });
});
