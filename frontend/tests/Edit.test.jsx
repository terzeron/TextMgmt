// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, cleanup, within } from '@testing-library/react';

afterEach(cleanup);

// ── mock 함수 ──

const { mockJsonGetReq, mockJsonPutReq, mockJsonDeleteReq } = vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
    mockJsonPutReq: vi.fn(),
    mockJsonDeleteReq: vi.fn(),
}));

vi.mock('../src/Common', () => ({
    jsonGetReq: mockJsonGetReq,
    jsonPutReq: mockJsonPutReq,
    jsonDeleteReq: mockJsonDeleteReq,
    getApiUrlPrefix: () => 'http://localhost:8000',
    ROOT_DIRECTORY: '_root',
}));

const mockOutletContext = {
    searchResults: [],
    hasSearched: false,
    searchTotal: 0,
    handleLoadMore: vi.fn(),
    searchLoading: false,
};

vi.mock('react-router-dom', () => ({
    useParams: vi.fn(() => ({ '*': '' })),
    useSearchParams: vi.fn(() => [new URLSearchParams()]),
    useOutletContext: vi.fn(() => mockOutletContext),
}));

vi.mock('../src/Folder', () => ({
    default: ({ folderData, isOpen, onToggle, onClickHandler }) => (
        <div data-testid={isOpen ? 'folder-open' : 'folder-closed'}>
            {folderData.map(f => (
                <div key={f.id} data-testid={`folder-item-${f.id}`}
                    onClick={() => onClickHandler(f.id)}>{f.label}</div>
            ))}
            {folderData.flatMap(f => (f.children || []).map(c => (
                <div key={c.id} data-testid={`folder-item-${c.id}`}
                    onClick={() => onClickHandler(c.id)}>{c.label}</div>
            )))}
            {!isOpen && <button onClick={() => onToggle(true)}>펼치기</button>}
        </div>
    ),
}));

vi.mock('../src/BookInfoView', () => ({
    default: ({ bookInfo, newFileName, newFileNameChanged, changeButtonClicked, deleteButtonClicked, onTitleChange, onAuthorChange, onExchangeButtonClick, onResetButtonClick, onCutTitleButtonClick, onCutAuthorButtonClick }) => (
        <div data-testid="book-info">
            <span data-testid="book-title">{bookInfo.title}</span>
            <span data-testid="book-author">{bookInfo.author}</span>
            <input data-testid="title-input" value={bookInfo.title || ''} onChange={onTitleChange} />
            <input data-testid="author-input" value={bookInfo.author || ''} onChange={onAuthorChange} />
            <input data-testid="filename-input" value={newFileName || ''} onChange={newFileNameChanged} />
            <button data-testid="change-btn" onClick={changeButtonClicked}>변경</button>
            <button data-testid="delete-btn" onClick={deleteButtonClicked}>삭제</button>
            <button data-testid="exchange-btn" onClick={onExchangeButtonClick}>교환</button>
            <button data-testid="reset-btn" onClick={onResetButtonClick}>초기화</button>
            <button data-testid="cut-title-btn" onClick={onCutTitleButtonClick}>제목자르기</button>
            <button data-testid="cut-author-btn" onClick={onCutAuthorButtonClick}>저자자르기</button>
        </div>
    ),
}));

vi.mock('../src/ViewSingle', () => ({
    default: ({ bookId }) => <div data-testid="view-single">ViewSingle:{bookId}</div>,
}));

vi.mock('../src/SimilarBooks', () => ({
    default: () => <div data-testid="similar-books">SimilarBooks</div>,
}));

vi.mock('../src/Bookstore', () => ({
    default: ({ onCategoriesFound }) => {
        // 간접 테스트를 위해 즉시 호출
        return <div data-testid="bookstore">Bookstore</div>;
    },
}));

vi.mock('../src/SimilarityDebug', () => ({
    default: () => <div data-testid="similarity-debug">SimilarityDebug</div>,
}));

vi.mock('../src/EpubDiagnoseView', () => ({
    default: () => <div data-testid="epub-diagnose">EpubDiagnose</div>,
}));

vi.mock('../src/Actions', () => ({
    default: ({ toNextEntryClicked, toPrevEntryClicked, moveToUpperButtonClicked, moveToDirectoryButtonClicked }) => (
        <div data-testid="actions">
            <button data-testid="next-entry" onClick={toNextEntryClicked}>다음</button>
            <button data-testid="prev-entry" onClick={toPrevEntryClicked}>이전</button>
            <button data-testid="move-upper" onClick={moveToUpperButtonClicked}>최상위이동</button>
            <button data-testid="move-dir" onClick={moveToDirectoryButtonClicked}>디렉토리이동</button>
        </div>
    ),
}));

vi.mock('../src/SearchResult', () => ({
    default: () => <div data-testid="search-result">SearchResult</div>,
}));

vi.mock('../src/Edit.css', () => ({}));

import Edit from '../src/Edit';

// ── 헬퍼 ──

const CATEGORIES = {
    '1_fiction': 3,
    '2_science': 2,
};

const BOOKS_IN_FICTION = [
    { book_id: 101, title: '[작가A] 소설1', author: '', file_type: 'epub', file_path: '1_fiction/[작가A] 소설1.epub', category: '1_fiction' },
    { book_id: 102, title: '[작가B] 소설2', author: '', file_type: 'pdf', file_path: '1_fiction/[작가B] 소설2.pdf', category: '1_fiction' },
];

function setupMockCategories(categories = CATEGORIES, books = BOOKS_IN_FICTION) {
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
        if (url === '/categories') {
            resolve(categories);
        } else if (url.startsWith('/categories/')) {
            resolve(books);
        } else if (url.startsWith('/books/')) {
            const bookId = parseInt(url.split('/').pop());
            const found = books.find(b => b.book_id === bookId);
            if (found) resolve(found);
            else if (reject) reject('Not found');
        } else {
            resolve({});
        }
    });
}

describe('Edit', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        vi.spyOn(window, 'confirm').mockReturnValue(true);
        vi.spyOn(console, 'log').mockImplementation(() => {});
    });

    afterEach(() => {
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    // ── 초기 렌더링 ──

    it('카테고리 API를 호출하고 폴더 데이터를 로드한다', async () => {
        setupMockCategories();
        render(<Edit />);

        await waitFor(() => {
            expect(mockJsonGetReq).toHaveBeenCalledWith(
                '/categories', null, expect.any(Function), expect.any(Function)
            );
        });

        await waitFor(() => {
            expect(screen.getByTestId('folder-open')).toBeTruthy();
        });
    });

    it('카테고리 로드 실패 시 폴더 데이터가 비어있다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
            if (url === '/categories' && reject) reject('서버 오류');
        });
        render(<Edit />);

        await waitFor(() => {
            expect(screen.getByTestId('folder-open')).toBeTruthy();
            expect(screen.queryByTestId('folder-item-1_fiction')).toBeNull();
        });
    });

    // ── 폴더 항목 클릭 ──

    it('폴더 클릭 시 카테고리 내 책 목록을 로드한다', async () => {
        setupMockCategories();
        render(<Edit />);

        await waitFor(() => {
            expect(screen.getByTestId('folder-open')).toBeTruthy();
        });

        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));

        await waitFor(() => {
            const calls = mockJsonGetReq.mock.calls;
            const hasCategoryCall = calls.some(c => c[0] === '/categories/1_fiction');
            expect(hasCategoryCall).toBe(true);
        });
    });

    it('책 항목 클릭 시 책 정보를 표시한다', async () => {
        setupMockCategories();
        render(<Edit />);

        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        await waitFor(() => { expect(screen.getByText('[작가A] 소설1.epub')).toBeTruthy(); });

        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        await waitFor(() => {
            expect(screen.getByTestId('book-info')).toBeTruthy();
        });
    });

    // ── decomposeTitle ──

    it('[저자] 제목 패턴의 파일명을 분해한다', async () => {
        setupMockCategories();
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        await waitFor(() => { expect(screen.getByText('[작가A] 소설1.epub')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));

        await waitFor(() => {
            expect(screen.getByTestId('book-title').textContent).toBe('소설1');
            expect(screen.getByTestId('book-author').textContent).toBe('작가A');
        });
    });

    it('저자 필드가 있으면 파일명 패턴 분석을 건너뛴다', async () => {
        const books = [{ book_id: 201, title: '제목만', author: '저자만', file_type: 'pdf', file_path: '1_fiction/제목만.pdf', category: '1_fiction' }];
        setupMockCategories(CATEGORIES, books);
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        await waitFor(() => { expect(screen.getByText('제목만.pdf')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/201'));
        await waitFor(() => {
            expect(screen.getByTestId('book-title').textContent).toBe('제목만');
            expect(screen.getByTestId('book-author').textContent).toBe('저자만');
        });
    });

    // ── 제목/저자 편집 ──

    it('제목 변경 시 bookInfo가 업데이트된다', async () => {
        setupMockCategories();
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        await waitFor(() => { expect(screen.getByTestId('book-info')).toBeTruthy(); });

        fireEvent.change(screen.getByTestId('title-input'), { target: { value: '새제목' } });
        await waitFor(() => {
            expect(screen.getByTestId('book-title').textContent).toBe('새제목');
        });
    });

    it('저자 변경 시 bookInfo가 업데이트된다', async () => {
        setupMockCategories();
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        await waitFor(() => { expect(screen.getByTestId('book-info')).toBeTruthy(); });

        fireEvent.change(screen.getByTestId('author-input'), { target: { value: '새저자' } });
        await waitFor(() => {
            expect(screen.getByTestId('book-author').textContent).toBe('새저자');
        });
    });

    it('교환 버튼 클릭 시 제목과 저자가 교환된다', async () => {
        setupMockCategories();
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        fireEvent.click(screen.getByTestId('exchange-btn'));
        await waitFor(() => {
            expect(screen.getByTestId('book-title').textContent).toBe('작가A');
            expect(screen.getByTestId('book-author').textContent).toBe('소설1');
        });
    });

    it('삭제 버튼 클릭 시 delete API를 호출한다', async () => {
        setupMockCategories();
        mockJsonDeleteReq.mockImplementation((url, payload, resolve) => resolve({}));
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        fireEvent.click(screen.getByTestId('delete-btn'));

        await waitFor(() => {
            expect(mockJsonDeleteReq).toHaveBeenCalledWith(
                '/books/101', null, expect.any(Function), expect.any(Function)
            );
        });
    });

    it('변경 버튼 클릭 시 PUT API를 호출한다', async () => {
        setupMockCategories();
        mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        fireEvent.click(screen.getByTestId('change-btn'));

        await waitFor(() => {
            expect(mockJsonPutReq).toHaveBeenCalledWith(
                '/books/101', expect.any(Object), expect.any(Function), expect.any(Function)
            );
        });
    });

    it('apiPrefix가 전달되면 API URL에 포함된다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/comics/categories') resolve(CATEGORIES);
            else if (url.startsWith('/comics/categories/')) resolve(BOOKS_IN_FICTION);
            else resolve({});
        });
        render(<Edit apiPrefix="/comics" />);
        await waitFor(() => {
            expect(mockJsonGetReq).toHaveBeenCalledWith(
                '/comics/categories', null, expect.any(Function), expect.any(Function)
            );
        });
    });

    it('_root 카테고리가 있으면 최상위 파일을 로드한다', async () => {
        const categories = { '1_fiction': 3, '_root': 1 };
        const rootBooks = [{ book_id: 999, title: '최상위파일', author: '', file_type: 'txt', file_path: '최상위파일.txt', category: '_root' }];
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            if (url === '/categories') resolve(categories);
            else if (url === '/categories/_root') resolve(rootBooks);
            else if (url.startsWith('/categories/')) resolve(BOOKS_IN_FICTION);
            else resolve({});
        });
        render(<Edit />);
        await waitFor(() => {
            expect(mockJsonGetReq).toHaveBeenCalledWith(
                '/categories/_root', null, expect.any(Function), expect.any(Function)
            );
        });
    });

    it('모바일 화면 크기일 때 적절한 클래스를 사용한다', async () => {
        setupMockCategories();
        vi.stubGlobal('innerWidth', 500);
        const { container } = render(<Edit />);
        await waitFor(() => {
            expect(container.querySelector('.section.directory-menu')).toBeNull();
        });
    });

    it('다음 버튼 클릭 시 다음 책으로 이동한다', async () => {
        setupMockCategories();
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        await waitFor(() => { expect(screen.getByText('[작가A] 소설1.epub')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        await waitFor(() => { expect(screen.getByTestId('book-title').textContent).toBe('소설1'); });

        fireEvent.click(screen.getByTestId('next-entry'));
        await waitFor(() => {
            expect(screen.getByTestId('book-title').textContent).toBe('소설2');
        });
    });

    it('최상위로 이동 버튼 클릭 시 PUT API를 호출한다', async () => {
        setupMockCategories();
        mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        fireEvent.click(screen.getByTestId('move-upper'));
        await waitFor(() => {
            expect(mockJsonPutReq).toHaveBeenCalledWith(
                '/books/101', expect.objectContaining({ category: '_root' }), expect.any(Function), expect.any(Function)
            );
        });
    });

    it('(저자) 제목 패턴의 파일명을 분해한다', async () => {
        const books = [{ book_id: 301, title: '(작가C) 소설3', author: '', file_type: 'epub', file_path: '1_fiction/(작가C) 소설3.epub', category: '1_fiction' }];
        setupMockCategories(CATEGORIES, books);
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/301'));
        await waitFor(() => {
            expect(screen.getByTestId('book-title').textContent).toBe('소설3');
            expect(screen.getByTestId('book-author').textContent).toBe('작가C');
        });
    });

    it('제목 @ 저자 패턴의 파일명을 분해한다', async () => {
        const books = [{ book_id: 302, title: '소설4 @ 작가D', author: '', file_type: 'epub', file_path: '1_fiction/소설4 @ 작가D.epub', category: '1_fiction' }];
        setupMockCategories(CATEGORIES, books);
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/302'));
        await waitFor(() => {
            expect(screen.getByTestId('book-title').textContent).toBe('소설4');
            expect(screen.getByTestId('book-author').textContent).toBe('작가D');
        });
    });

    it('제목자르기 버튼 클릭 시 공백을 기준으로 분해한다', async () => {
        setupMockCategories();
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        
        fireEvent.change(screen.getByTestId('title-input'), { target: { value: '저자명 제목' } });
        fireEvent.click(screen.getByTestId('cut-title-btn'));
        await waitFor(() => {
            expect(screen.getByTestId('book-author').textContent).toBe('저자명');
            expect(screen.getByTestId('book-title').textContent).toBe('제목');
        });
    });

    it('삭제 시 warning이 있으면 메시지에 포함한다', async () => {
        setupMockCategories();
        mockJsonDeleteReq.mockImplementation((url, payload, resolve) => {
            resolve({ warning: '파일은 지워졌으나 DB 연동 오류' });
        });
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        fireEvent.click(screen.getByTestId('delete-btn'));

        await waitFor(() => {
            expect(screen.getByText(/경고: 파일은 지워졌으나 DB 연동 오류/)).toBeTruthy();
        });
    });

    it('마지막 책 삭제 시 "마지막 책이었습니다" 메시지를 표시한다', async () => {
        const singleBook = [{ book_id: 999, title: '막책', author: '', file_type: 'txt', file_path: '1_fiction/last.txt', category: '1_fiction' }];
        setupMockCategories(CATEGORIES, singleBook);
        mockJsonDeleteReq.mockImplementation((url, payload, resolve) => resolve({}));
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/999'));
        fireEvent.click(screen.getByTestId('delete-btn'));

        await waitFor(() => {
            expect(screen.getByText(/마지막 책이었습니다/)).toBeTruthy();
        });
    });

    it('다음 책이 없을 때 에러 메시지를 표시한다', async () => {
        const singleBook = [{ book_id: 999, title: '막책', author: '', file_type: 'txt', file_path: '1_fiction/last.txt', category: '1_fiction' }];
        setupMockCategories(CATEGORIES, singleBook);
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/999'));
        
        fireEvent.click(screen.getByTestId('next-entry'));
        await waitFor(() => {
            expect(screen.getByText('마지막 책입니다.')).toBeTruthy();
        });
    });

    it('이전 책이 없을 때 에러 메시지를 표시한다', async () => {
        setupMockCategories();
        render(<Edit />);
        await waitFor(() => { expect(screen.getByTestId('folder-open')).toBeTruthy(); });
        fireEvent.click(screen.getByTestId('folder-item-1_fiction'));
        fireEvent.click(screen.getByTestId('folder-item-1_fiction/101'));
        
        fireEvent.click(screen.getByTestId('prev-entry'));
        await waitFor(() => {
            expect(screen.getByText('첫 번째 책입니다.')).toBeTruthy();
        });
    });
});
