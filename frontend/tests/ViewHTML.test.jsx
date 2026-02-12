// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

import ViewHTML from '../src/ViewHTML';

describe('ViewHTML', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it('로딩 중 상태를 표시한다', () => {
        render(<ViewHTML bookId={1} />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    it('iframe에 올바른 src를 설정한다', () => {
        render(<ViewHTML bookId={42} />);
        const iframe = screen.getByTitle('html viewer');
        expect(iframe.getAttribute('src')).toBe('http://localhost:8000/download/42');
    });

    it('iframe 로드 완료 시 로딩 상태를 숨긴다', () => {
        render(<ViewHTML bookId={1} />);
        const iframe = screen.getByTitle('html viewer');
        fireEvent.load(iframe);
        expect(screen.queryByText('로딩 중...')).toBeNull();
    });

    it('bookId가 없으면 에러 메시지를 표시한다', () => {
        render(<ViewHTML bookId={0} />);
        expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
    });

    it('에러 상태에서 iframe을 숨긴다', () => {
        render(<ViewHTML bookId={0} />);
        const iframe = screen.getByTitle('html viewer');
        expect(iframe.style.display).toBe('none');
    });
});
