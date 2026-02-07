// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

vi.mock('react-router-dom', () => ({
    useParams: vi.fn(() => ({})),
    useSearchParams: vi.fn(() => [new URLSearchParams()]),
}));

vi.mock('../src/ViewPDF', () => ({
    default: ({ bookId }) => <div data-testid="view-pdf">PDF:{bookId}</div>,
}));
vi.mock('../src/ViewEPUB', () => ({
    default: ({ bookId }) => <div data-testid="view-epub">EPUB:{bookId}</div>,
}));
vi.mock('../src/ViewDOC', () => ({
    default: ({ bookId }) => <div data-testid="view-doc">DOC:{bookId}</div>,
}));
vi.mock('../src/ViewTXT', () => ({
    default: ({ bookId }) => <div data-testid="view-txt">TXT:{bookId}</div>,
}));
vi.mock('../src/ViewHTML', () => ({
    default: ({ bookId }) => <div data-testid="view-html">HTML:{bookId}</div>,
}));
vi.mock('../src/ViewRTF', () => ({
    default: ({ bookId }) => <div data-testid="view-rtf">RTF:{bookId}</div>,
}));
vi.mock('../src/ViewImage', () => ({
    default: ({ bookId }) => <div data-testid="view-image">IMG:{bookId}</div>,
}));

import ViewSingle from '../src/ViewSingle';

describe('ViewSingle', () => {
    it('props로 전달된 bookId에 해당하는 뷰어를 렌더링한다', async () => {
        render(<ViewSingle bookId={42} fileType="pdf" filePath="/test.pdf" />);

        await waitFor(() => {
            expect(screen.getByText('PDF:42')).toBeTruthy();
        });
    });

    it('부모 리렌더 시 props 값이 같으면 뷰어가 유지된다', async () => {
        const { rerender } = render(
            <ViewSingle bookId={42} fileType="pdf" filePath="/test.pdf" />
        );

        await waitFor(() => {
            expect(screen.getByText('PDF:42')).toBeTruthy();
        });

        // 새 props 객체지만 값은 동일 → effect 재실행 안 됨
        rerender(<ViewSingle bookId={42} fileType="pdf" filePath="/test.pdf" />);

        expect(screen.queryByText('책이 선택되지 않았습니다.')).toBeNull();
        expect(screen.getByText('PDF:42')).toBeTruthy();
    });

    it('bookId가 변경되면 새 뷰어를 렌더링한다', async () => {
        const { rerender } = render(
            <ViewSingle bookId={1} fileType="pdf" filePath="/a.pdf" />
        );

        await waitFor(() => {
            expect(screen.getByText('PDF:1')).toBeTruthy();
        });

        rerender(<ViewSingle bookId={2} fileType="pdf" filePath="/b.pdf" />);

        await waitFor(() => {
            expect(screen.getByText('PDF:2')).toBeTruthy();
        });
    });

    it('fileType에 따라 올바른 뷰어 컴포넌트를 렌더링한다', async () => {
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

    it('bookId가 없으면 안내 메시지를 표시한다', () => {
        render(<ViewSingle />);
        expect(screen.getByText('책이 선택되지 않았습니다.')).toBeTruthy();
    });
});
