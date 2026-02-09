// @vitest-environment jsdom
//
// epub.js의 Book.prototype.determineType()를 직접 호출하여
// 입력 타입별 반환값을 검증한다.
//
// 프로덕션 버그 #1 재현:
//   Blob URL("blob:http://…")은 확장자가 없어 "directory"로 판정됨
//   → epub.js가 META-INF/container.xml을 개별 HTTP 요청으로 시도하여 실패
//
import { describe, it, expect } from 'vitest';
import Book from 'epubjs/src/book';

const determineType = Book.prototype.determineType;
const ctx = { settings: {} };

describe('epub.js determineType — 입력 타입 감지', () => {

    it('ArrayBuffer → "binary"', () => {
        const input = new ArrayBuffer(8);
        expect(determineType.call(ctx, input)).toBe('binary');
    });

    it('Uint8Array → "binary"', () => {
        const input = new Uint8Array([0x50, 0x4B, 0x03, 0x04]);
        expect(determineType.call(ctx, input)).toBe('binary');
    });

    it('blob: URL → "directory" (버그 #1 재현 — Blob URL 사용 불가)', () => {
        const input = 'blob:http://localhost:3000/abc-def-123';
        expect(determineType.call(ctx, input)).toBe('directory');
    });

    it('.epub 확장자 URL → "epub"', () => {
        const input = 'https://example.com/books/sample.epub';
        expect(determineType.call(ctx, input)).toBe('epub');
    });

    it('확장자 없는 URL → "directory" (preview URL 직접 전달 불가)', () => {
        const input = 'https://example.com/preview/123';
        expect(determineType.call(ctx, input)).toBe('directory');
    });

    it('base64 encoding 설정 시 → "base64"', () => {
        const base64Ctx = { settings: { encoding: 'base64' } };
        expect(determineType.call(base64Ctx, 'anything')).toBe('base64');
    });
});
