// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockJsonGetReq } = vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
}));

const { mockUseOutletContext } = vi.hoisted(() => ({
    mockUseOutletContext: vi.fn(() => ({ searchResults: [], hasSearched: false })),
}));

vi.mock('../src/Common', () => ({
    jsonGetReq: mockJsonGetReq,
}));

vi.mock('react-router-dom', () => ({
    useOutletContext: mockUseOutletContext,
}));

vi.mock('../src/CategoryMapping', () => ({
    default: ({ categoryList }) => <div data-testid="category-mapping">카테고리: {categoryList.join(', ')}</div>,
}));

vi.mock('../src/CategoryMismatch', () => ({
    default: () => <div data-testid="category-mismatch">불일치</div>,
}));

import Admin from '../src/Admin';

describe('Admin', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it('카테고리 목록을 로드하여 하위 컴포넌트에 전달한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            resolve({ '소설': 10, '역사': 5, '_root': 3, '소설/SF': 2 });
        });
        render(<Admin />);
        await waitFor(() => {
            // _root와 슬래시 포함 카테고리 제외, 정렬
            expect(screen.getByText('카테고리: 소설, 역사')).toBeTruthy();
        });
    });

    it('에러 시 에러 메시지를 표시한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
            reject('서버 오류');
        });
        render(<Admin />);
        await waitFor(() => {
            expect(screen.getByText(/카테고리 목록을 불러올 수 없습니다/)).toBeTruthy();
        });
    });

    it('CategoryMapping과 CategoryMismatch를 렌더링한다', async () => {
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
            resolve({ '소설': 10 });
        });
        render(<Admin />);
        await waitFor(() => {
            expect(screen.getByTestId('category-mapping')).toBeTruthy();
            expect(screen.getByTestId('category-mismatch')).toBeTruthy();
        });
    });
});
