// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

import ViewImage from '../src/ViewImage';

describe('ViewImage', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it('로딩 중 상태를 표시한다', () => {
        render(<ViewImage bookId={1} />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    it('이미지에 올바른 src를 설정한다', () => {
        render(<ViewImage bookId={42} />);
        const img = screen.getByAltText('book cover');
        expect(img.getAttribute('src')).toBe('http://localhost:8000/download/42');
    });

    it('이미지 로드 완료 시 로딩 상태를 숨긴다', () => {
        render(<ViewImage bookId={1} />);
        const img = screen.getByAltText('book cover');
        fireEvent.load(img);
        expect(screen.queryByText('로딩 중...')).toBeNull();
    });

    it('이미지 로드 실패 시 에러 메시지를 표시한다', () => {
        render(<ViewImage bookId={1} />);
        const img = screen.getByAltText('book cover');
        fireEvent.error(img);
        expect(screen.getByText(/이미지 파일을 불러올 수 없습니다/)).toBeTruthy();
    });

    it('bookId가 없으면 에러 메시지를 표시한다', () => {
        render(<ViewImage bookId={0} />);
        expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
    });

    it('에러 상태에서 이미지를 숨긴다', () => {
        render(<ViewImage bookId={0} />);
        const img = screen.getByAltText('book cover');
        expect(img.style.display).toBe('none');
    });
});
