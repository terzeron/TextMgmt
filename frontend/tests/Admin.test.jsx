// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockUseOutletContext } = vi.hoisted(() => ({
    mockUseOutletContext: vi.fn(() => ({ searchResults: [], hasSearched: false })),
}));

vi.mock('react-router-dom', () => ({
    useOutletContext: mockUseOutletContext,
}));

vi.mock('../src/CategoryAdmin', () => ({
    default: ({ contentType, title }) => (
        <div data-testid={`category-admin-${contentType || 'book'}`}>
            {title || '카테고리 관리'}
        </div>
    ),
}));

import Admin from '../src/Admin';

describe('Admin', () => {
    it('CategoryAdmin 컴포넌트를 책/만화 각각 렌더링한다', () => {
        render(<Admin />);
        expect(screen.getByTestId('category-admin-book')).toBeTruthy();
        expect(screen.getByTestId('category-admin-comic')).toBeTruthy();
        expect(screen.getByText('책 카테고리 관리')).toBeTruthy();
        expect(screen.getByText('만화 카테고리 관리')).toBeTruthy();
    });
});
