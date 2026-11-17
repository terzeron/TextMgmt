import { describe, it, expect } from 'vitest';
import { isViewerAllowedPath } from '../src/auth.js';

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
