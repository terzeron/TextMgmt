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

import ViewTXT from '../src/ViewTXT';

describe('ViewTXT', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it('로딩 중 상태를 표시한다', () => {
        mockTextGetReq.mockImplementation(() => {}); // 응답 안 함
        render(<ViewTXT bookId={1} />);
        expect(screen.getByText('로딩 중...')).toBeTruthy();
    });

    it('텍스트 파일 내용을 렌더링한다', async () => {
        mockTextGetReq.mockImplementation((url, payload, resolve) => {
            resolve('첫 번째 줄\n두 번째 줄\n세 번째 줄');
        });
        render(<ViewTXT bookId={1} />);
        await waitFor(() => {
            expect(screen.getByText('첫 번째 줄')).toBeTruthy();
            expect(screen.getByText('두 번째 줄')).toBeTruthy();
            expect(screen.getByText('세 번째 줄')).toBeTruthy();
        });
    });

    it('lineCount로 줄 수를 제한한다', async () => {
        mockTextGetReq.mockImplementation((url, payload, resolve) => {
            resolve('줄1\n줄2\n줄3\n줄4\n줄5');
        });
        render(<ViewTXT bookId={1} lineCount={2} />);
        await waitFor(() => {
            expect(screen.getByText('줄1')).toBeTruthy();
            expect(screen.getByText('줄2')).toBeTruthy();
            expect(screen.queryByText('줄3')).toBeNull();
        });
    });

    it('에러 발생 시 에러 메시지를 표시한다', async () => {
        mockTextGetReq.mockImplementation((url, payload, resolve, reject) => {
            reject('네트워크 오류');
        });
        render(<ViewTXT bookId={1} />);
        await waitFor(() => {
            expect(screen.getByText(/파일을 불러올 수 없습니다/)).toBeTruthy();
        });
    });

    it('bookId가 없으면 에러 메시지를 표시한다', async () => {
        render(<ViewTXT bookId={0} />);
        await waitFor(() => {
            expect(screen.getByText(/유효한 bookId가 제공되지 않았습니다/)).toBeTruthy();
        });
    });

    it('lineCount가 0이면 모든 줄을 표시한다', async () => {
        mockTextGetReq.mockImplementation((url, payload, resolve) => {
            resolve('줄1\n줄2\n줄3');
        });
        render(<ViewTXT bookId={1} lineCount={0} />);
        await waitFor(() => {
            expect(screen.getByText('줄1')).toBeTruthy();
            expect(screen.getByText('줄3')).toBeTruthy();
        });
    });
});
