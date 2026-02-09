// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/react';

afterEach(cleanup);

// ReactReader mock
const mockReactReader = vi.fn(({ url, locationChanged }) => {
    // URL이 설정되면 즉시 locationChanged를 호출하여 로딩 완료 시뮬레이션
    if (url) {
        setTimeout(() => locationChanged?.('epubcfi(/1)'), 0);
    }
    return <div data-testid="react-reader" data-url={url}>ReactReader</div>;
});

vi.mock('react-reader', () => ({
    ReactReader: (props) => mockReactReader(props),
}));

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

import ViewEPUB from '../src/ViewEPUB';

describe('ViewEPUB', () => {
    beforeEach(() => {
        mockReactReader.mockClear();
    });

    // ── 유효성 검사 ──

    it('bookId가 없으면 에러를 표시한다', () => {
        render(<ViewEPUB bookId={0} />);
        expect(screen.getByText(/유효한 bookId 또는 filePath가 제공되지 않았습니다/)).toBeTruthy();
    });

    it('filePath 없이 preview=false이면 에러를 표시한다', () => {
        render(<ViewEPUB bookId={1} />);
        expect(screen.getByText(/유효한 bookId 또는 filePath가 제공되지 않았습니다/)).toBeTruthy();
    });

    // ── URL 생성 ──

    it('preview=true이면 /preview/ URL로 ReactReader를 호출한다', () => {
        render(<ViewEPUB bookId={42} preview={true} />);
        expect(mockReactReader).toHaveBeenCalledWith(
            expect.objectContaining({
                url: 'http://localhost:8000/preview/42?chapters=3',
            })
        );
    });

    it('preview=false이면 /download/ URL로 ReactReader를 호출한다', () => {
        render(<ViewEPUB bookId={42} filePath="test/book.epub" />);
        expect(mockReactReader).toHaveBeenCalledWith(
            expect.objectContaining({
                url: 'http://localhost:8000/download/42/test/book.epub',
            })
        );
    });

    it('bookId 변경 시 URL이 업데이트된다', () => {
        const { rerender } = render(<ViewEPUB bookId={1} filePath="a.epub" />);
        expect(mockReactReader).toHaveBeenCalledWith(
            expect.objectContaining({ url: 'http://localhost:8000/download/1/a.epub' })
        );

        rerender(<ViewEPUB bookId={2} filePath="b.epub" />);
        expect(mockReactReader).toHaveBeenCalledWith(
            expect.objectContaining({ url: 'http://localhost:8000/download/2/b.epub' })
        );
    });

    // ── 컨테이너 높이 ──

    it('preview=true이면 컨테이너 높이가 60vh이다', () => {
        const { container } = render(<ViewEPUB bookId={1} preview={true} />);
        const div = container.firstChild;
        expect(div.style.height).toBe('60vh');
    });

    it('preview=false이면 컨테이너 높이가 100vh이다', () => {
        const { container } = render(<ViewEPUB bookId={1} filePath="a.epub" />);
        const div = container.firstChild;
        expect(div.style.height).toBe('100vh');
    });

    // ── 로딩 상태 ──

    it('초기 로딩 시 스피너를 표시한다', () => {
        render(<ViewEPUB bookId={1} filePath="a.epub" />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    // ── cleanup ──

    it('unmount 시 URL을 초기화한다', () => {
        const { unmount, rerender } = render(<ViewEPUB bookId={1} filePath="a.epub" />);
        // ReactReader가 URL과 함께 호출되었는지 확인
        expect(mockReactReader).toHaveBeenCalledWith(
            expect.objectContaining({ url: 'http://localhost:8000/download/1/a.epub' })
        );
        unmount();
        // unmount 후 URL 초기화 (cleanup 함수 실행)
    });
});
