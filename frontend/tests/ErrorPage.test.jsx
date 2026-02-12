// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockUseRouteError } = vi.hoisted(() => ({
    mockUseRouteError: vi.fn(),
}));

vi.mock('react-router-dom', () => ({
    useRouteError: mockUseRouteError,
}));

import ErrorPage from '../src/ErrorPage';

describe('ErrorPage', () => {
    it('에러의 statusText를 표시한다', () => {
        mockUseRouteError.mockReturnValue({ statusText: 'Not Found' });
        render(<ErrorPage />);
        expect(screen.getByText('Not Found')).toBeTruthy();
        expect(screen.getByText('Oops!')).toBeTruthy();
        expect(screen.getByText('예상치 못한 에러가 발생했습니다.')).toBeTruthy();
    });

    it('에러의 message를 표시한다 (statusText 없을 때)', () => {
        mockUseRouteError.mockReturnValue({ message: 'Something went wrong' });
        render(<ErrorPage />);
        expect(screen.getByText('Something went wrong')).toBeTruthy();
    });

    it('statusText가 message보다 우선한다', () => {
        mockUseRouteError.mockReturnValue({ statusText: 'Bad Request', message: 'fallback' });
        render(<ErrorPage />);
        expect(screen.getByText('Bad Request')).toBeTruthy();
    });
});
