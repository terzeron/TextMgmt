import { describe, it, expect } from 'vitest';
import { determineRole, isViewerAllowedPath } from '../src/auth.js';

const ADMIN_EMAIL = 'admin@example.com';
const ALLOWED_EMAILS = ['viewer1@example.com', 'viewer2@example.com'];

describe('determineRole', () => {
    it('admin 이메일이면 admin 반환', () => {
        expect(determineRole(ADMIN_EMAIL, ADMIN_EMAIL, ALLOWED_EMAILS)).toBe('admin');
    });

    it('허용 목록 이메일이면 viewer 반환', () => {
        expect(determineRole('viewer1@example.com', ADMIN_EMAIL, ALLOWED_EMAILS)).toBe('viewer');
        expect(determineRole('viewer2@example.com', ADMIN_EMAIL, ALLOWED_EMAILS)).toBe('viewer');
    });

    it('미등록 이메일이면 null 반환', () => {
        expect(determineRole('unknown@example.com', ADMIN_EMAIL, ALLOWED_EMAILS)).toBeNull();
    });

    it('admin 이메일이 allowedEmails에도 포함되면 admin 우선', () => {
        const emails = [ADMIN_EMAIL, 'viewer1@example.com'];
        expect(determineRole(ADMIN_EMAIL, ADMIN_EMAIL, emails)).toBe('admin');
    });

    it('allowedEmails가 빈 배열이면 admin만 접근 가능', () => {
        expect(determineRole(ADMIN_EMAIL, ADMIN_EMAIL, [])).toBe('admin');
        expect(determineRole('viewer1@example.com', ADMIN_EMAIL, [])).toBeNull();
    });

    it('빈 문자열 이메일은 null 반환', () => {
        expect(determineRole('', ADMIN_EMAIL, ALLOWED_EMAILS)).toBeNull();
    });
});

describe('isViewerAllowedPath', () => {
    it('루트 경로 허용', () => {
        expect(isViewerAllowedPath('/')).toBe(true);
    });

    it('/book-view 경로 허용', () => {
        expect(isViewerAllowedPath('/book-view')).toBe(true);
    });

    it('/book-view 하위 경로 허용', () => {
        expect(isViewerAllowedPath('/book-view/123')).toBe(true);
        expect(isViewerAllowedPath('/book-view/123?category=test')).toBe(true);
    });

    it('/book-latest 경로 허용', () => {
        expect(isViewerAllowedPath('/book-latest')).toBe(true);
    });

    it('/comics-latest 경로 허용', () => {
        expect(isViewerAllowedPath('/comics-latest')).toBe(true);
    });

    it('/comics-view 경로 허용', () => {
        expect(isViewerAllowedPath('/comics-view')).toBe(true);
    });

    it('/comics-view 하위 경로 허용', () => {
        expect(isViewerAllowedPath('/comics-view/456')).toBe(true);
    });

    it('/viewer 하위 경로 허용', () => {
        expect(isViewerAllowedPath('/viewer/epub/1')).toBe(true);
    });

    it('/book-edit 경로 차단', () => {
        expect(isViewerAllowedPath('/book-edit')).toBe(false);
        expect(isViewerAllowedPath('/book-edit/something')).toBe(false);
    });

    it('/comics-edit 경로 차단', () => {
        expect(isViewerAllowedPath('/comics-edit')).toBe(false);
        expect(isViewerAllowedPath('/comics-edit/something')).toBe(false);
    });

    it('/admin 경로 차단', () => {
        expect(isViewerAllowedPath('/admin')).toBe(false);
    });

    it('이전 경로 /view, /edit 차단', () => {
        expect(isViewerAllowedPath('/view')).toBe(false);
        expect(isViewerAllowedPath('/edit')).toBe(false);
    });

    it('/viewer 정확한 경로는 차단 (하위만 허용)', () => {
        expect(isViewerAllowedPath('/viewer')).toBe(false);
    });
});
