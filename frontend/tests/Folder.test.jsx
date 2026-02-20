// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/Folder.css', () => ({}));

import Folder from '../src/Folder';

const FOLDER_DATA = [
    {
        id: '1_fiction',
        label: '1_fiction',
        fileType: 'folder',
        count: 3,
        children: [
            { id: '1_fiction/101', label: '소설A.epub', fileType: 'epub', children: [] },
            { id: '1_fiction/102', label: '소설B.pdf', fileType: 'pdf', children: [] },
        ],
    },
    {
        id: '2_science',
        label: '2_science',
        fileType: 'folder',
        count: 1,
        children: [
            { id: '2_science/201', label: '과학A.txt', fileType: 'txt', children: [] },
        ],
    },
];

const defaultProps = {
    folderData: FOLDER_DATA,
    expandedItems: [],
    onExpandedItemsChange: vi.fn(),
    onClickHandler: vi.fn(),
    isOpen: true,
    onToggle: vi.fn(),
};

describe('Folder', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    // ── 접힌 상태 ──

    it('isOpen=false일 때 접힌 카드 헤더만 표시한다', () => {
        render(<Folder {...defaultProps} isOpen={false} />);
        expect(screen.getByText('디렉토리')).toBeTruthy();
        expect(screen.queryByRole('tree')).toBeNull();
    });

    it('접힌 상태에서 헤더 클릭 시 onToggle(true)을 호출한다', () => {
        const onToggle = vi.fn();
        render(<Folder {...defaultProps} isOpen={false} onToggle={onToggle} />);
        fireEvent.click(screen.getByText('디렉토리'));
        expect(onToggle).toHaveBeenCalledWith(true);
    });

    // ── 펼친 상태 ──

    it('isOpen=true일 때 트리를 표시한다', () => {
        render(<Folder {...defaultProps} />);
        expect(screen.getByRole('tree')).toBeTruthy();
        expect(screen.getByText('디렉토리')).toBeTruthy();
    });

    it('펼친 상태에서 헤더 클릭 시 onToggle(false)을 호출한다', () => {
        const onToggle = vi.fn();
        render(<Folder {...defaultProps} onToggle={onToggle} />);
        fireEvent.click(screen.getByText('디렉토리'));
        expect(onToggle).toHaveBeenCalledWith(false);
    });

    it('폴더 데이터의 항목을 렌더링한다', () => {
        render(<Folder {...defaultProps} />);
        expect(screen.getByText('1_fiction')).toBeTruthy();
        expect(screen.getByText('2_science')).toBeTruthy();
    });

    // ── 트리 아이템 클릭 ──

    it('폴더 클릭 시 onClickHandler를 호출하고 onToggle(false)을 호출하지 않는다', () => {
        const onClickHandler = vi.fn();
        const onToggle = vi.fn();
        render(<Folder {...defaultProps} onClickHandler={onClickHandler} onToggle={onToggle} />);

        // 헤더가 아닌 트리 내 폴더 텍스트 클릭
        const folderItems = screen.getAllByText('1_fiction');
        // 첫 번째는 트리 아이템
        fireEvent.click(folderItems[0]);
        expect(onClickHandler).toHaveBeenCalledWith('1_fiction');
        // 폴더 클릭이므로 onToggle(false) 호출 안 됨 (헤더 클릭과 별개)
    });

    it('폴더가 아닌 항목 클릭 시 onToggle(false)을 호출한다', () => {
        const onClickHandler = vi.fn();
        const onToggle = vi.fn();
        // 1_fiction을 펼쳐서 하위 항목이 보이게
        render(
            <Folder {...defaultProps}
                expandedItems={['1_fiction']}
                onClickHandler={onClickHandler}
                onToggle={onToggle}
            />
        );

        fireEvent.click(screen.getByText('소설A.epub'));
        expect(onClickHandler).toHaveBeenCalledWith('1_fiction/101');
        expect(onToggle).toHaveBeenCalledWith(false);
    });

    // ── expandedItems 제어 ──

    it('접힌 폴더 클릭 시 onExpandedItemsChange로 항목을 추가한다', () => {
        const onExpandedItemsChange = vi.fn();
        render(<Folder {...defaultProps} expandedItems={[]} onExpandedItemsChange={onExpandedItemsChange} />);

        fireEvent.click(screen.getByText('1_fiction'));

        // 함수 업데이터가 호출됨
        expect(onExpandedItemsChange).toHaveBeenCalled();
        const updater = onExpandedItemsChange.mock.calls[0][0];
        expect(updater([])).toEqual(['1_fiction']);
    });

    it('펼쳐진 폴더 클릭 시 onExpandedItemsChange로 항목을 제거한다', () => {
        const onExpandedItemsChange = vi.fn();
        render(<Folder {...defaultProps} expandedItems={['1_fiction']} onExpandedItemsChange={onExpandedItemsChange} />);

        fireEvent.click(screen.getByText('1_fiction'));

        expect(onExpandedItemsChange).toHaveBeenCalled();
        const updater = onExpandedItemsChange.mock.calls[0][0];
        expect(updater(['1_fiction'])).toEqual([]);
    });

    // ── 빈 데이터 ──

    it('folderData가 빈 배열이면 트리만 렌더링한다', () => {
        render(<Folder {...defaultProps} folderData={[]} />);
        expect(screen.getByText('디렉토리')).toBeTruthy();
    });

    // ── selectedItems ──

    it('selectedItems prop을 전달할 수 있다', () => {
        render(<Folder {...defaultProps} selectedItems={['1_fiction']} />);
        expect(screen.getByRole('tree')).toBeTruthy();
    });
});
