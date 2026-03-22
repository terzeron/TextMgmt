// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import Navigation from '../src/Navigation';
import React from 'react';
import * as Common from '../src/Common';

// Mock @react-oauth/google
vi.mock('@react-oauth/google', () => ({
    GoogleOAuthProvider: ({ children }) => <div data-testid="google-oauth-provider">{children}</div>,
    GoogleLogin: ({ onSuccess }) => (
        <button data-testid="google-login" onClick={() => onSuccess({ credential: 'test-token' })}>
            Google Login Mock
        </button>
    ),
    googleLogout: vi.fn(),
}));

// Mock Common.js
vi.mock('../src/Common', async () => {
    const actual = await vi.importActual('../src/Common');
    return {
        ...actual,
        rawJsonGetReq: vi.fn(),
        getApiUrlPrefix: vi.fn(() => '/api'),
    };
});

// Mock window.matchMedia
if (!window.matchMedia) {
    Object.defineProperty(window, 'matchMedia', {
        writable: true,
        value: vi.fn().mockImplementation(query => ({
            matches: false,
            media: query,
            onchange: null,
            addListener: vi.fn(),
            removeListener: vi.fn(),
            addEventListener: vi.fn(),
            removeEventListener: vi.fn(),
            dispatchEvent: vi.fn(),
        })),
    });
}

describe('Navigation Component', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        window.__ENV__ = {
            VITE_API_URL_PREFIX: '/api',
            VITE_GOOGLE_CLIENT_ID: 'test-client-id'
        };
        vi.stubGlobal('fetch', vi.fn());
        vi.stubGlobal('alert', vi.fn());
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('loads session on mount', async () => {
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => ({
                status: 'success',
                result: { role: 'admin', name: 'Test User', email: 'test@example.com' }
            })
        });

        render(
            <MemoryRouter>
                <Navigation />
            </MemoryRouter>
        );

        await waitFor(() => {
            expect(screen.getByText('책 편집')).toBeDefined();
        });
        expect(fetch).toHaveBeenCalledWith('/api/auth/me', expect.anything());
    });

    it('handles login success', async () => {
        // Initial session check fails
        fetch.mockResolvedValueOnce({ ok: false });
        
        // Google auth verification success
        fetch.mockResolvedValueOnce({
            ok: true,
            status: 200,
            json: async () => ({ role: 'viewer', name: 'Viewer User', email: 'viewer@example.com' })
        });

        render(
            <MemoryRouter>
                <Navigation />
            </MemoryRouter>
        );

        const loginButton = await screen.findByTestId('google-login');
        fireEvent.click(loginButton);

        await waitFor(() => {
            expect(screen.queryByText('책 편집')).toBeNull();
            expect(screen.getByText('책')).toBeDefined();
        });
    });

    it('handles logout', async () => {
        // Initial session check success
        fetch.mockResolvedValueOnce({
            ok: true,
            json: async () => ({ status: 'success', result: { role: 'admin' } })
        });
        // Logout API success
        fetch.mockResolvedValueOnce({ ok: true });

        const { container } = render(
            <MemoryRouter>
                <Navigation />
            </MemoryRouter>
        );

        // Wait for admin menu to appear
        await screen.findByText('관리');
        
        // Find user dropdown toggle. It might not have a button role because it's a div
        const userDropdown = container.querySelector('.dropdown-toggle');
        fireEvent.click(userDropdown);

        const logoutButton = screen.getByText('로그아웃');
        fireEvent.click(logoutButton);

        await waitFor(() => {
            expect(fetch).toHaveBeenCalledWith('/api/auth/logout', expect.anything());
            expect(screen.getByTestId('google-login')).toBeDefined();
        });
    });

    it('performs search when keyword is entered and button clicked', async () => {
        fetch.mockResolvedValueOnce({
            ok: true,
            json: async () => ({ status: 'success', result: { role: 'admin' } })
        });

        Common.rawJsonGetReq.mockImplementation((url, resolve) => {
            if (url.includes('search')) {
                resolve({ status: 'success', result: [{ id: 1, title: 'Found Book' }], total: 1 });
            }
        });

        render(
            <MemoryRouter>
                <Navigation />
            </MemoryRouter>
        );

        const input = await screen.findByPlaceholderText(/키워드/i);
        fireEvent.change(input, { target: { value: 'python' } });
        
        const searchButton = screen.getByRole('button', { name: /검색/i });
        fireEvent.click(searchButton);

        await waitFor(() => {
            expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
                expect.stringContaining('search/python'),
                expect.any(Function),
                expect.any(Function)
            );
        });
    });

    it('loads hidden categories for viewer', async () => {
        fetch.mockResolvedValueOnce({
            ok: true,
            json: async () => ({ status: 'success', result: { role: 'viewer' } })
        });

        Common.rawJsonGetReq.mockImplementation((url, resolve) => {
            if (url.includes('hidden-categories')) {
                resolve({ status: 'success', result: ['cat1', 'cat2'] });
            }
        });

        render(
            <MemoryRouter>
                <Navigation />
            </MemoryRouter>
        );

        await waitFor(() => {
            expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
                expect.stringContaining('hidden-categories'),
                expect.any(Function),
                expect.any(Function)
            );
        });
    });

    it('navigates to book-view when searching from home', async () => {
        fetch.mockResolvedValueOnce({
            ok: true,
            json: async () => ({ status: 'success', result: { role: 'admin' } })
        });
        
        Common.rawJsonGetReq.mockImplementation((url, resolve) => {
            if (url.includes('search')) {
                resolve({ status: 'success', result: [] });
            }
        });

        render(
            <MemoryRouter initialEntries={['/']}>
                <Routes>
                    <Route path="/" element={<Navigation />} />
                    <Route path="/book-view" element={<div>Book View Page</div>} />
                </Routes>
            </MemoryRouter>
        );

        const input = await screen.findByPlaceholderText(/키워드/i);
        fireEvent.change(input, { target: { value: 'test' } });
        fireEvent.click(screen.getByRole('button', { name: /검색/i }));

        await waitFor(() => {
            expect(screen.getByText('Book View Page')).toBeDefined();
        });
    });
});
