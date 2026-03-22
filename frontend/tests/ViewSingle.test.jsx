// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, cleanup } from '@testing-library/react';
import React from 'react';
import * as Common from '../src/Common';

afterEach(cleanup);

const { mockUseParams, mockUseSearchParams } = vi.hoisted(() => ({
    mockUseParams: vi.fn(() => ({})),
    mockUseSearchParams: vi.fn(() => [new URLSearchParams()]),
}));

vi.mock('react-router-dom', () => ({
    useParams: mockUseParams,
    useSearchParams: mockUseSearchParams,
}));

vi.mock('../src/Common', async () => {
    const actual = await vi.importActual('../src/Common');
    return {
        ...actual,
        jsonGetReq: vi.fn(),
        getApiUrlPrefix: vi.fn(() => '/api'),
    };
});

vi.mock('../src/ViewPDF', () => ({
    default: ({ bookId, pageCount, apiPrefix }) => <div data-testid="view-pdf">PDF:{bookId}:pc={pageCount}:ap={apiPrefix || ''}</div>,
}));
vi.mock('../src/ViewEPUB', () => ({
    default: ({ bookId, preview, apiPrefix }) => <div data-testid="view-epub">EPUB:{bookId}:preview={String(!!preview)}:ap={apiPrefix || ''}</div>,
}));
vi.mock('../src/ViewDOC', () => ({
    default: ({ bookId, fileType, lineCount, apiPrefix }) => <div data-testid="view-doc">DOC:{bookId}:ft={fileType}:lc={lineCount}:ap={apiPrefix || ''}</div>,
}));
vi.mock('../src/ViewTXT', () => ({
    default: ({ bookId, lineCount, apiPrefix }) => <div data-testid="view-txt">TXT:{bookId}:lc={lineCount}:ap={apiPrefix || ''}</div>,
}));
vi.mock('../src/ViewHTML', () => ({
    default: ({ bookId, apiPrefix }) => <div data-testid="view-html">HTML:{bookId}:ap={apiPrefix || ''}</div>,
}));
vi.mock('../src/ViewRTF', () => ({
    default: ({ bookId, apiPrefix }) => <div data-testid="view-rtf">RTF:{bookId}:ap={apiPrefix || ''}</div>,
}));
vi.mock('../src/ViewImage', () => ({
    default: ({ bookId, apiPrefix }) => <div data-testid="view-image">IMG:{bookId}:ap={apiPrefix || ''}</div>,
}));

import ViewSingle from '../src/ViewSingle';

describe('ViewSingle', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        mockUseParams.mockReturnValue({});
        mockUseSearchParams.mockReturnValue([new URLSearchParams()]);
        vi.stubGlobal('location', { href: '' });
    });

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    // ── 기본 렌더링 ──

    it('bookId가 없으면 안내 메시지를 표시한다', () => {
        render(<ViewSingle />);
        expect(screen.getByText('책이 선택되지 않았습니다.')).toBeTruthy();
    });

    it('props로 전달된 bookId에 해당하는 뷰어를 렌더링한다', async () => {
        render(<ViewSingle bookId={42} fileType="pdf" filePath="/test.pdf" />);
        await waitFor(() => {
            expect(screen.getByTestId('view-pdf')).toBeTruthy();
        });
    });

    // ── fileType별 뷰어 매핑 ──

    it.each([
        ['pdf', 'view-pdf'],
        ['epub', 'view-epub'],
        ['doc', 'view-doc'],
        ['docx', 'view-doc'],
        ['hwp', 'view-doc'],
        ['txt', 'view-txt'],
        ['html', 'view-html'],
        ['rtf', 'view-rtf'],
        ['jpg', 'view-image'],
        ['gif', 'view-image'],
        ['png', 'view-image'],
    ])('fileType="%s"이면 %s 뷰어를 렌더링한다', async (fileType, testId) => {
        render(<ViewSingle bookId={1} fileType={fileType} filePath={`/test.${fileType}`} />);
        await waitFor(() => {
            expect(screen.getByTestId(testId)).toBeTruthy();
        });
    });

    // ── standalone 모드 ──

    it('standalone 모드에서 body 스타일을 잠근다', async () => {
        mockUseParams.mockReturnValue({ entryId: '10', fileType: 'pdf' });
        mockUseSearchParams.mockReturnValue([new URLSearchParams('path=/test.pdf')]);

        const { unmount } = render(<ViewSingle />);
        expect(document.documentElement.style.overflow).toBe('hidden');
        expect(document.body.style.position).toBe('fixed');

        unmount();
        expect(document.documentElement.style.overflow).toBe('');
    });

    it('standalone 모드에서 카테고리 정보를 로드하고 네비게이션을 처리한다', async () => {
        mockUseParams.mockReturnValue({ entryId: '101', fileType: 'epub' });
        mockUseSearchParams.mockReturnValue([new URLSearchParams('path=1_fiction/a.epub&category=1_fiction')]);

        Common.jsonGetReq.mockImplementation((url, payload, resolve) => {
            resolve([
                { book_id: 100, title: 'Book 0', file_type: 'epub', file_path: '1_fiction/0.epub' },
                { book_id: 101, title: 'Book 1', file_type: 'epub', file_path: '1_fiction/a.epub' },
                { book_id: 102, title: 'Book 2', file_type: 'epub', file_path: '1_fiction/b.epub' },
            ]);
        });

        render(<ViewSingle />);

        await waitFor(() => {
            expect(Common.jsonGetReq).toHaveBeenCalledWith(
                expect.stringContaining('/categories/1_fiction'),
                null,
                expect.any(Function)
            );
        });

        await waitFor(() => {
            expect(screen.getByText(/이전 책으로/)).toBeTruthy();
            expect(screen.getByText(/다음 책으로/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/다음 책으로/));
        expect(window.location.href).toContain('102');
    });

    // ── 기타 버튼 및 엣지 케이스 ──

    it('viewUrl과 downloadUrl 버튼을 올바르게 렌더링한다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                viewUrl="http://view" downloadUrl="http://dl" />
        );
        await waitFor(() => {
            expect(screen.getByText('전체 보기')).toBeTruthy();
            expect(screen.getByText('다운로드')).toBeTruthy();
        });
    });

    it('admin 역할일 때 편집 버튼을 표시한다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                role="admin" editUrl="/edit/1" />
        );
        await waitFor(() => {
            expect(screen.getByText('편집')).toBeTruthy();
        });
    });
});
