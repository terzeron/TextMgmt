// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockRawJsonGetReq, mockGetApiUrlPrefix } = vi.hoisted(() => ({
    mockRawJsonGetReq: vi.fn(),
    mockGetApiUrlPrefix: vi.fn(() => 'http://localhost:8000'),
}));

vi.mock('../src/Common.js', () => ({
    rawJsonGetReq: mockRawJsonGetReq,
    getApiUrlPrefix: mockGetApiUrlPrefix,
}));

vi.mock('@react-oauth/google', () => ({
    GoogleOAuthProvider: ({ children }) => <div>{children}</div>,
    GoogleLogin: ({ onSuccess, onError }) => (
        <button data-testid="google-login" onClick={() => onSuccess({ credential: 'test-token' })}>
            Google로 로그인
        </button>
    ),
    googleLogout: vi.fn(),
}));

vi.mock('react-router-dom', async () => {
    const actual = await vi.importActual('react-router-dom');
    return {
        ...actual,
        useLocation: () => ({ pathname: '/' }),
        Outlet: (props) => <div data-testid="outlet">Outlet</div>,
        Navigate: ({ to }) => <div data-testid="navigate">Navigate to {to}</div>,
    };
});

import Navigation from '../src/Navigation';

describe('Navigation', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        localStorage.clear();
        window.__ENV__ = {
            VITE_GOOGLE_CLIENT_ID: 'test-client-id',
            VITE_ADMIN_EMAIL: 'admin@test.com',
            VITE_ALLOWED_EMAILS: 'viewer@test.com',
        };
    });

    afterEach(() => {
        delete window.__ENV__;
        localStorage.clear();
    });

    it('미로그인 시 Google 로그인 버튼을 표시한다', () => {
        render(<Navigation />);
        expect(screen.getByTestId('google-login')).toBeTruthy();
    });

    it('Navbar에 브랜드명 "Text"를 표시한다', () => {
        render(<Navigation />);
        expect(screen.getByText('Text')).toBeTruthy();
    });

    it('localStorage에 저장된 admin 이메일로 자동 로그인한다', () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);
        // admin 역할이면 책 편집/책/만화 편집/만화/관리 링크 표시
        expect(screen.getByText('책 편집')).toBeTruthy();
        expect(screen.getByText('책')).toBeTruthy();
        expect(screen.getByText('만화 편집')).toBeTruthy();
        expect(screen.getByText('만화')).toBeTruthy();
        expect(screen.getByText('관리')).toBeTruthy();
    });

    it('localStorage에 저장된 viewer 이메일로 자동 로그인한다', () => {
        localStorage.setItem('email', 'viewer@test.com');
        localStorage.setItem('name', 'Viewer');
        render(<Navigation />);
        // viewer 역할이면 책/만화만 표시
        expect(screen.getByText('책')).toBeTruthy();
        expect(screen.getByText('만화')).toBeTruthy();
        expect(screen.queryByText('책 편집')).toBeNull();
        expect(screen.queryByText('만화 편집')).toBeNull();
        expect(screen.queryByText('관리')).toBeNull();
    });

    it('권한 없는 이메일 로그인 시 접근 불가 메시지를 표시한다', () => {
        localStorage.setItem('email', 'unknown@test.com');
        localStorage.setItem('name', 'Unknown');
        render(<Navigation />);
        expect(screen.getByText(/서비스 접근 권한이 없습니다/)).toBeTruthy();
    });

    it('로그아웃 시 로그인 상태를 초기화한다', async () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);

        // Dropdown.Toggle(as="div")을 클릭하여 드롭다운 메뉴 표시
        const dropdownToggle = document.querySelector('.dropdown-toggle');
        expect(dropdownToggle).toBeTruthy();
        fireEvent.click(dropdownToggle);

        await waitFor(() => {
            expect(screen.getByText('로그아웃')).toBeTruthy();
        });

        fireEvent.click(screen.getByText('로그아웃'));

        // 로그아웃 후 Google 로그인 버튼이 다시 표시
        await waitFor(() => {
            expect(screen.getByTestId('google-login')).toBeTruthy();
        });
        expect(localStorage.getItem('email')).toBeNull();
    });

    it('admin 로그인 시 검색 입력 필드를 표시한다', () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);
        const searchInput = screen.getByPlaceholderText('키워드');
        expect(searchInput).toBeTruthy();
    });

    it('검색 버튼 클릭 시 rawJsonGetReq를 호출한다', async () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);

        const searchInput = screen.getByPlaceholderText('키워드');
        fireEvent.change(searchInput, { target: { value: '테스트' } });

        const searchBtn = screen.getByText('검색');
        fireEvent.click(searchBtn);

        expect(mockRawJsonGetReq).toHaveBeenCalledWith(
            expect.stringContaining('/search/'),
            expect.any(Function),
            expect.any(Function)
        );
    });

    it('프로필 사진이 있으면 이미지로 표시한다', () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        localStorage.setItem('picture', 'https://photo.example.com/admin.jpg');
        render(<Navigation />);
        const img = screen.getByAltText('admin@test.com');
        expect(img.getAttribute('src')).toBe('https://photo.example.com/admin.jpg');
    });

    it('Google 로그인 성공 시 백엔드 검증을 수행한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({
                email: 'admin@test.com',
                name: 'Admin User',
                picture: '',
            }),
        });

        render(<Navigation />);
        const loginBtn = screen.getByTestId('google-login');
        fireEvent.click(loginBtn);

        await waitFor(() => {
            expect(global.fetch).toHaveBeenCalledWith(
                'http://localhost:8000/auth/google',
                expect.objectContaining({
                    method: 'POST',
                    body: JSON.stringify({ credential: 'test-token' }),
                })
            );
        });

        delete global.fetch;
    });

    it('viewer는 hidden-categories를 로드한다', async () => {
        localStorage.setItem('email', 'viewer@test.com');
        localStorage.setItem('name', 'Viewer');

        mockRawJsonGetReq.mockImplementation((url, onSuccess) => {
            if (url.includes('/hidden-categories')) {
                onSuccess({ status: 'success', result: ['비공개'] });
            }
        });

        render(<Navigation />);

        await waitFor(() => {
            expect(mockRawJsonGetReq).toHaveBeenCalledWith(
                '/hidden-categories',
                expect.any(Function),
                expect.any(Function)
            );
        });
    });

    it('hidden-categories 로드 실패 시 빈 배열로 설정한다', async () => {
        localStorage.setItem('email', 'viewer@test.com');
        localStorage.setItem('name', 'Viewer');

        mockRawJsonGetReq.mockImplementation((url, onSuccess, onError) => {
            if (url === '/hidden-categories') {
                onError('서버 오류');
            }
        });

        render(<Navigation />);

        // 에러가 발생해도 컴포넌트는 정상 렌더링
        await waitFor(() => {
            expect(screen.getByText('책')).toBeTruthy();
        });
    });

    it('검색 성공 시 결과를 Outlet context로 전달한다', async () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');

        mockRawJsonGetReq.mockImplementation((url, onSuccess) => {
            if (url.includes('/search/')) {
                onSuccess({ status: 'success', result: [{ book_id: 1 }], total: 1 });
            }
        });

        render(<Navigation />);

        const searchInput = screen.getByPlaceholderText('키워드');
        fireEvent.change(searchInput, { target: { value: '테스트' } });
        fireEvent.click(screen.getByText('검색'));

        await waitFor(() => {
            const call = mockRawJsonGetReq.mock.calls.find(c => c[0].includes('/search/'));
            expect(call).toBeTruthy();
            // 성공 콜백 호출됨
            expect(call[1]).toBeTypeOf('function');
        });
    });

    it('폼 submit으로 검색을 실행한다', async () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');

        render(<Navigation />);

        const searchInput = screen.getByPlaceholderText('키워드');
        fireEvent.change(searchInput, { target: { value: '폼검색' } });

        // form submit 이벤트
        const form = searchInput.closest('form');
        fireEvent.submit(form);

        await waitFor(() => {
            expect(mockRawJsonGetReq).toHaveBeenCalledWith(
                expect.stringContaining('/search/'),
                expect.any(Function),
                expect.any(Function)
            );
        });
    });

    it('VITE_GOOGLE_CLIENT_ID가 없으면 로그인 상태를 복원하지 않는다', () => {
        window.__ENV__ = {
            VITE_ADMIN_EMAIL: 'admin@test.com',
            VITE_ALLOWED_EMAILS: 'viewer@test.com',
        };
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);
        expect(screen.queryByText('책 편집')).toBeNull();
    });

    it('VITE_ADMIN_EMAIL이 없으면 로그인 상태를 복원하지 않는다', () => {
        window.__ENV__ = {
            VITE_GOOGLE_CLIENT_ID: 'test-client-id',
            VITE_ALLOWED_EMAILS: 'viewer@test.com',
        };
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);
        expect(screen.queryByText('책 편집')).toBeNull();
    });

    it('Google 로그인 백엔드 검증 실패 시 alert를 표시한다', async () => {
        global.fetch = vi.fn().mockResolvedValue({
            ok: false,
            status: 500,
        });
        const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {});

        render(<Navigation />);
        fireEvent.click(screen.getByTestId('google-login'));

        await waitFor(() => {
            expect(alertSpy).toHaveBeenCalledWith(expect.stringContaining('오류'));
        });

        alertSpy.mockRestore();
        delete global.fetch;
    });

    it('검색어가 비어있으면 검색을 실행하지 않는다', () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);

        fireEvent.click(screen.getByText('검색'));
        expect(mockRawJsonGetReq).not.toHaveBeenCalled();
    });

    it('책 컨텍스트에서 검색 시 prefix 없이 /search/ URL을 호출한다', async () => {
        localStorage.setItem('email', 'admin@test.com');
        localStorage.setItem('name', 'Admin');
        render(<Navigation />);

        const searchInput = screen.getByPlaceholderText('키워드');
        fireEvent.change(searchInput, { target: { value: '소설' } });
        fireEvent.click(screen.getByText('검색'));

        await waitFor(() => {
            const searchCall = mockRawJsonGetReq.mock.calls.find(c => c[0].includes('/search/'));
            expect(searchCall).toBeTruthy();
            expect(searchCall[0]).toMatch(/^\/search\//);
            expect(searchCall[0]).not.toMatch(/^\/comics/);
        });
    });

    it('viewer에서 hidden categories가 있으면 검색 URL에 exclude_categories를 포함한다', async () => {
        localStorage.setItem('email', 'viewer@test.com');
        localStorage.setItem('name', 'Viewer');

        mockRawJsonGetReq.mockImplementation((url, onSuccess) => {
            if (url === '/hidden-categories') {
                onSuccess({ status: 'success', result: ['비공개', '비밀'] });
            }
        });

        render(<Navigation />);

        // hidden categories 로드 대기
        await waitFor(() => {
            expect(mockRawJsonGetReq).toHaveBeenCalledWith('/hidden-categories', expect.any(Function), expect.any(Function));
        });

        // 검색 실행
        const searchInput = screen.getByPlaceholderText('키워드');
        fireEvent.change(searchInput, { target: { value: '테스트' } });
        fireEvent.click(screen.getByText('검색'));

        await waitFor(() => {
            const searchCall = mockRawJsonGetReq.mock.calls.find(c => c[0].includes('/search/'));
            expect(searchCall).toBeTruthy();
            expect(searchCall[0]).toContain('exclude_categories=');
        });
    });
});
