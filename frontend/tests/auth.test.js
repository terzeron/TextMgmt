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

    it('/view 경로 허용', () => {
        expect(isViewerAllowedPath('/view')).toBe(true);
    });

    it('/view 하위 경로 허용', () => {
        expect(isViewerAllowedPath('/view/book')).toBe(true);
        expect(isViewerAllowedPath('/view/book/123')).toBe(true);
    });

    it('/edit 경로 차단', () => {
        expect(isViewerAllowedPath('/edit')).toBe(false);
        expect(isViewerAllowedPath('/edit/something')).toBe(false);
    });

    it('/admin 경로 차단', () => {
        expect(isViewerAllowedPath('/admin')).toBe(false);
    });

    it('/viewer 등 유사 경로 차단 (정확한 매칭)', () => {
        expect(isViewerAllowedPath('/viewer')).toBe(false);
        expect(isViewerAllowedPath('/viewall')).toBe(false);
    });
});
