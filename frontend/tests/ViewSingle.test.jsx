// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockUseParams, mockUseSearchParams } = vi.hoisted(() => ({
    mockUseParams: vi.fn(() => ({})),
    mockUseSearchParams: vi.fn(() => [new URLSearchParams()]),
}));

vi.mock('react-router-dom', () => ({
    useParams: mockUseParams,
    useSearchParams: mockUseSearchParams,
}));

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
        mockUseParams.mockReturnValue({});
        mockUseSearchParams.mockReturnValue([new URLSearchParams()]);
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

    it('지원하지 않는 fileType이면 뷰어를 렌더링하지 않는다', async () => {
        render(<ViewSingle bookId={1} fileType="xyz" filePath="/test.xyz" />);

        await waitFor(() => {
            // 뷰어가 undefined → Card.Body에 아무것도 표시 안 됨
            expect(screen.queryByTestId('view-pdf')).toBeNull();
            expect(screen.queryByTestId('view-doc')).toBeNull();
        });
    });

    // ── props 전달 ──

    it('ViewPDF에 pageCount를 전달한다', async () => {
        render(<ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf" pageCount={5} />);

        await waitFor(() => {
            expect(screen.getByText(/pc=5/)).toBeTruthy();
        });
    });

    it('ViewDOC(doc)에 fileType="doc"을 전달한다', async () => {
        render(<ViewSingle bookId={1} fileType="doc" filePath="/test.doc" />);

        await waitFor(() => {
            expect(screen.getByText(/ft=doc/)).toBeTruthy();
        });
    });

    it('ViewDOC(hwp)에 fileType="hwp"을 전달한다', async () => {
        render(<ViewSingle bookId={1} fileType="hwp" filePath="/test.hwp" />);

        await waitFor(() => {
            expect(screen.getByText(/ft=hwp/)).toBeTruthy();
        });
    });

    it('ViewDOC(docx)에 fileType="docx"와 lineCount를 전달한다', async () => {
        render(<ViewSingle bookId={1} fileType="docx" filePath="/test.docx" lineCount={20} />);

        await waitFor(() => {
            expect(screen.getByText(/ft=docx/)).toBeTruthy();
            expect(screen.getByText(/lc=20/)).toBeTruthy();
        });
    });

    it('ViewTXT에 lineCount를 전달한다', async () => {
        render(<ViewSingle bookId={1} fileType="txt" filePath="/test.txt" lineCount={30} />);

        await waitFor(() => {
            expect(screen.getByText(/lc=30/)).toBeTruthy();
        });
    });

    it('ViewEPUB에 preview를 전달한다', async () => {
        render(<ViewSingle bookId={1} fileType="epub" preview={true} />);

        await waitFor(() => {
            expect(screen.getByText(/preview=true/)).toBeTruthy();
        });
    });

    // ── Header / 버튼 (nested 모드) ──

    it('nested 모드에서 "책 보기" 헤더를 표시한다', async () => {
        render(<ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf" />);

        await waitFor(() => {
            expect(screen.getByText('책 보기')).toBeTruthy();
        });
    });

    it('viewUrl이 있으면 "전체 보기" 버튼을 표시한다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                viewUrl="http://example.com/view/1/pdf" />
        );

        await waitFor(() => {
            const btn = screen.getByText('전체 보기');
            expect(btn).toBeTruthy();
            // 링크의 href 확인
            const link = btn.closest('a');
            expect(link.getAttribute('href')).toBe('http://example.com/view/1/pdf');
            expect(link.getAttribute('target')).toBe('_blank');
        });
    });

    it('downloadUrl이 있으면 "다운로드" 버튼을 표시한다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                downloadUrl="http://example.com/download/1" />
        );

        await waitFor(() => {
            const btn = screen.getByText('다운로드');
            expect(btn).toBeTruthy();
            const link = btn.closest('a');
            expect(link.getAttribute('href')).toBe('http://example.com/download/1');
            expect(link.getAttribute('target')).toBe('_blank');
        });
    });

    it('viewUrl과 downloadUrl이 없으면 버튼을 표시하지 않는다', async () => {
        render(<ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf" />);

        await waitFor(() => {
            expect(screen.getByTestId('view-pdf')).toBeTruthy();
        });

        expect(screen.queryByText('전체 보기')).toBeNull();
        expect(screen.queryByText('다운로드')).toBeNull();
    });

    it('viewUrl과 downloadUrl을 동시에 표시할 수 있다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                viewUrl="http://example.com/view" downloadUrl="http://example.com/dl" />
        );

        await waitFor(() => {
            expect(screen.getByText('전체 보기')).toBeTruthy();
            expect(screen.getByText('다운로드')).toBeTruthy();
        });
    });

    // ── standalone 모드 (URL 파라미터) ──

    it('standalone 모드에서는 Card.Header를 숨긴다', async () => {
        mockUseParams.mockReturnValue({ entryId: '10', fileType: 'pdf' });
        mockUseSearchParams.mockReturnValue([new URLSearchParams('path=/test.pdf')]);

        render(<ViewSingle />);

        await waitFor(() => {
            expect(screen.getByTestId('view-pdf')).toBeTruthy();
        });

        expect(screen.queryByText('책 보기')).toBeNull();
    });

    it('standalone 모드에서 URL 파라미터로 뷰어를 렌더링한다', async () => {
        mockUseParams.mockReturnValue({ entryId: '7', fileType: 'txt' });
        mockUseSearchParams.mockReturnValue([new URLSearchParams('path=/docs/readme.txt')]);

        render(<ViewSingle />);

        await waitFor(() => {
            expect(screen.getByTestId('view-txt')).toBeTruthy();
            expect(screen.getByText(/TXT:7/)).toBeTruthy();
        });
    });

    // ── 리렌더 안정성 ──

    it('부모 리렌더 시 props 값이 같으면 뷰어가 유지된다', async () => {
        const { rerender } = render(
            <ViewSingle bookId={42} fileType="pdf" filePath="/test.pdf" />
        );

        await waitFor(() => {
            expect(screen.getByTestId('view-pdf')).toBeTruthy();
        });

        rerender(<ViewSingle bookId={42} fileType="pdf" filePath="/test.pdf" />);

        expect(screen.queryByText('책이 선택되지 않았습니다.')).toBeNull();
        expect(screen.getByTestId('view-pdf')).toBeTruthy();
    });

    it('bookId가 변경되면 새 뷰어를 렌더링한다', async () => {
        const { rerender } = render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/a.pdf" />
        );

        await waitFor(() => {
            expect(screen.getByText(/PDF:1/)).toBeTruthy();
        });

        rerender(<ViewSingle bookId={2} fileType="pdf" filePath="/b.pdf" />);

        await waitFor(() => {
            expect(screen.getByText(/PDF:2/)).toBeTruthy();
        });
    });

    it('fileType이 변경되면 다른 뷰어를 렌더링한다', async () => {
        const { rerender } = render(
            <ViewSingle bookId={1} fileType="txt" filePath="/test.txt" />
        );

        await waitFor(() => {
            expect(screen.getByTestId('view-txt')).toBeTruthy();
        });

        rerender(<ViewSingle bookId={1} fileType="epub" filePath="/test.epub" />);

        await waitFor(() => {
            expect(screen.getByTestId('view-epub')).toBeTruthy();
        });
    });

    // ── apiPrefix 전달 ──

    it('apiPrefix prop이 자식 뷰어에 전달된다', async () => {
        render(<ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf" apiPrefix="/comics" />);

        await waitFor(() => {
            expect(screen.getByText(/ap=\/comics/)).toBeTruthy();
        });
    });

    it('apiPrefix가 없으면 빈 문자열이 자식 뷰어에 전달된다', async () => {
        render(<ViewSingle bookId={1} fileType="epub" filePath="/test.epub" />);

        await waitFor(() => {
            expect(screen.getByText(/ap=$/m)).toBeTruthy();
        });
    });

    it('standalone 모드에서 ?api= 쿼리 파라미터가 apiPrefix로 사용된다', async () => {
        mockUseParams.mockReturnValue({ entryId: '10', fileType: 'pdf' });
        mockUseSearchParams.mockReturnValue([new URLSearchParams('path=/test.pdf&api=/comics')]);

        render(<ViewSingle />);

        await waitFor(() => {
            expect(screen.getByTestId('view-pdf')).toBeTruthy();
            expect(screen.getByText(/ap=\/comics/)).toBeTruthy();
        });
    });

    it('standalone 모드에서 ?api= 파라미터가 없으면 빈 apiPrefix를 사용한다', async () => {
        mockUseParams.mockReturnValue({ entryId: '10', fileType: 'epub' });
        mockUseSearchParams.mockReturnValue([new URLSearchParams('path=/test.epub')]);

        render(<ViewSingle />);

        await waitFor(() => {
            expect(screen.getByTestId('view-epub')).toBeTruthy();
            // ap= 뒤에 아무 값도 없어야 함
            expect(screen.getByText(/EPUB:10:preview=false:ap=$/)).toBeTruthy();
        });
    });

    // ── 편집 버튼 ──

    it('role=admin이고 editUrl이 있으면 "편집" 버튼을 표시한다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                role="admin" editUrl="/book-edit/1?category=fiction" />
        );

        await waitFor(() => {
            const btn = screen.getByText('편집');
            expect(btn).toBeTruthy();
            const link = btn.closest('a');
            expect(link.getAttribute('href')).toBe('/book-edit/1?category=fiction');
        });
    });

    it('role=admin이 아니면 "편집" 버튼을 표시하지 않는다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                role="user" editUrl="/book-edit/1" />
        );

        await waitFor(() => {
            expect(screen.getByTestId('view-pdf')).toBeTruthy();
        });
        expect(screen.queryByText('편집')).toBeNull();
    });

    it('editUrl이 없으면 "편집" 버튼을 표시하지 않는다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf" role="admin" />
        );

        await waitFor(() => {
            expect(screen.getByTestId('view-pdf')).toBeTruthy();
        });
        expect(screen.queryByText('편집')).toBeNull();
    });

    // ── 이전/다음 책 네비게이션 (nested 모드) ──

    it('onPrevBook/onNextBook이 있으면 네비게이션 버튼을 표시한다', async () => {
        const onPrev = vi.fn();
        const onNext = vi.fn();
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                onPrevBook={onPrev} onNextBook={onNext}
                hasPrevBook={true} hasNextBook={true} />
        );

        await waitFor(() => {
            expect(screen.getByText(/이전 책으로/)).toBeTruthy();
            expect(screen.getByText(/다음 책으로/)).toBeTruthy();
        });
    });

    it('이전 책 버튼 클릭 시 onPrevBook을 호출한다', async () => {
        const onPrev = vi.fn();
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                onPrevBook={onPrev} hasPrevBook={true} />
        );

        await waitFor(() => {
            expect(screen.getByText(/이전 책으로/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/이전 책으로/));
        expect(onPrev).toHaveBeenCalled();
    });

    it('다음 책 버튼 클릭 시 onNextBook을 호출한다', async () => {
        const onNext = vi.fn();
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                onNextBook={onNext} hasNextBook={true} />
        );

        await waitFor(() => {
            expect(screen.getByText(/다음 책으로/)).toBeTruthy();
        });

        fireEvent.click(screen.getByText(/다음 책으로/));
        expect(onNext).toHaveBeenCalled();
    });

    it('hasPrevBook=false이면 이전 버튼이 비활성화된다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                onPrevBook={vi.fn()} onNextBook={vi.fn()}
                hasPrevBook={false} hasNextBook={true} />
        );

        await waitFor(() => {
            const prevBtn = screen.getByText(/이전 책으로/);
            expect(prevBtn.disabled).toBe(true);
        });
    });

    it('hasNextBook=false이면 다음 버튼이 비활성화된다', async () => {
        render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/test.pdf"
                onPrevBook={vi.fn()} onNextBook={vi.fn()}
                hasPrevBook={true} hasNextBook={false} />
        );

        await waitFor(() => {
            const nextBtn = screen.getByText(/다음 책으로/);
            expect(nextBtn.disabled).toBe(true);
        });
    });
});
