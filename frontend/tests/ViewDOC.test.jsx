// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/Common', () => ({
    getApiUrlPrefix: () => 'http://localhost:8000',
}));

const { mockMammoth } = vi.hoisted(() => ({
    mockMammoth: { convertToHtml: vi.fn() },
}));

vi.mock('mammoth', () => ({ default: mockMammoth }));

import ViewDOC from '../src/ViewDOC';

describe('ViewDOC', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        global.fetch = vi.fn();
    });

    afterEach(() => {
        delete global.fetch;
    });

    it('로딩 중 상태를 표시한다', () => {
        global.fetch.mockReturnValue(new Promise(() => {})); // 응답 안 함
        render(<ViewDOC bookId={1} fileType="docx" />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    it('bookId가 없으면 에러 메시지를 표시한다', async () => {
        render(<ViewDOC bookId={0} />);
        await waitFor(() => {
            expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
        });
    });

    it('doc/hwp 타입은 preview URL로 fetch한다', async () => {
        global.fetch.mockResolvedValue({
            text: () => Promise.resolve('<p>미리보기 HTML</p>'),
        });
        render(<ViewDOC bookId={1} fileType="doc" />);
        await waitFor(() => {
            expect(global.fetch).toHaveBeenCalledWith('http://localhost:8000/preview/1');
        });
    });

    it('doc 타입에서 HTML 콘텐츠를 렌더링한다', async () => {
        global.fetch.mockResolvedValue({
            text: () => Promise.resolve('<p>문서 내용</p>'),
        });
        render(<ViewDOC bookId={1} fileType="doc" />);
        await waitFor(() => {
            const content = document.querySelector('.doc-content');
            expect(content.innerHTML).toBe('<p>문서 내용</p>');
        });
    });

    it('docx 타입은 mammoth으로 변환한다', async () => {
        const buffer = new ArrayBuffer(8);
        global.fetch.mockResolvedValue({
            arrayBuffer: () => Promise.resolve(buffer),
        });
        mockMammoth.convertToHtml.mockResolvedValue({
            value: '<p>단락1</p><p>단락2</p><p>단락3</p>',
        });
        render(<ViewDOC bookId={1} fileType="docx" lineCount={2} />);
        await waitFor(() => {
            expect(mockMammoth.convertToHtml).toHaveBeenCalled();
            const content = document.querySelector('.doc-content');
            expect(content.innerHTML).toContain('단락1');
            expect(content.innerHTML).toContain('단락2');
        });
    });

    it('fetch 에러 시 에러 메시지를 표시한다', async () => {
        global.fetch.mockRejectedValue(new Error('네트워크 오류'));
        render(<ViewDOC bookId={1} fileType="doc" />);
        await waitFor(() => {
            expect(screen.getByText(/문서를 불러오는 중 오류가 발생했습니다/)).toBeTruthy();
        });
    });

    it('hwp 타입도 preview URL로 fetch한다', async () => {
        global.fetch.mockResolvedValue({
            text: () => Promise.resolve('<p>HWP 미리보기</p>'),
        });
        render(<ViewDOC bookId={5} fileType="hwp" />);
        await waitFor(() => {
            expect(global.fetch).toHaveBeenCalledWith('http://localhost:8000/preview/5');
        });
    });

    it('docx mammoth 변환 실패 시 에러 메시지를 표시한다', async () => {
        global.fetch.mockResolvedValue({
            arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)),
        });
        mockMammoth.convertToHtml.mockRejectedValue(new Error('변환 실패'));
        render(<ViewDOC bookId={1} fileType="docx" />);
        await waitFor(() => {
            expect(screen.getByText(/문서를 불러오는 중 오류가 발생했습니다/)).toBeTruthy();
        });
    });

    it('docx lineCount 미지정 시 전체 내용을 렌더링한다', async () => {
        global.fetch.mockResolvedValue({
            arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)),
        });
        mockMammoth.convertToHtml.mockResolvedValue({
            value: '<p>전체 내용</p>',
        });
        render(<ViewDOC bookId={1} fileType="docx" />);
        await waitFor(() => {
            const content = document.querySelector('.doc-content');
            expect(content.innerHTML).toContain('전체 내용');
        });
    });
});
