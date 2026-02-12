// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/react';

afterEach(cleanup);

const { mockTextGetReq } = vi.hoisted(() => ({
    mockTextGetReq: vi.fn(),
}));

vi.mock('../src/Common', () => ({
    textGetReq: mockTextGetReq,
}));

const { mockRender, mockLoggingEnabled } = vi.hoisted(() => ({
    mockRender: vi.fn(),
    mockLoggingEnabled: vi.fn(),
}));

vi.mock('rtf.js', () => ({
    RTFJS: {
        Document: class {
            constructor() {}
            render() { return mockRender(); }
        },
        loggingEnabled: mockLoggingEnabled,
    },
}));

import ViewRTF from '../src/ViewRTF';

describe('ViewRTF', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it('로딩 중 상태를 표시한다', () => {
        mockTextGetReq.mockImplementation(() => {}); // 응답 안 함
        render(<ViewRTF bookId={1} />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    it('bookId가 없으면 에러 메시지를 표시한다', async () => {
        render(<ViewRTF bookId={0} />);
        await waitFor(() => {
            expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
        });
    });

    it('RTF 파일을 로드하고 렌더링한다', async () => {
        const mockElement = document.createElement('p');
        mockElement.textContent = 'RTF 내용';
        mockRender.mockResolvedValue([mockElement]);

        mockTextGetReq.mockImplementation((url, payload, resolve) => {
            resolve('rtf content');
        });

        render(<ViewRTF bookId={1} />);

        await waitFor(() => {
            expect(mockRender).toHaveBeenCalled();
        });
    });

    it('RTF 로딩이 비활성화된다', () => {
        mockTextGetReq.mockImplementation((url, payload, resolve) => {
            resolve('rtf content');
        });
        mockRender.mockResolvedValue([]);
        render(<ViewRTF bookId={1} />);
        expect(mockLoggingEnabled).toHaveBeenCalledWith(false);
    });

    it('textGetReq 에러 시 에러 메시지를 표시한다', async () => {
        mockTextGetReq.mockImplementation((url, payload, resolve, reject) => {
            reject('로드 실패');
        });
        render(<ViewRTF bookId={1} />);
        await waitFor(() => {
            expect(screen.getByText(/파일을 불러올 수 없습니다/)).toBeTruthy();
        });
    });

    it('RTF 렌더링 실패 시 에러 메시지를 표시한다', async () => {
        mockRender.mockRejectedValue(new Error('렌더링 오류'));
        mockTextGetReq.mockImplementation((url, payload, resolve) => {
            resolve('rtf content');
        });
        render(<ViewRTF bookId={1} />);
        await waitFor(() => {
            expect(screen.getByText(/RTF 렌더링에 실패했습니다/)).toBeTruthy();
        });
    });
});
