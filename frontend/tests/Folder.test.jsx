// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';

afterEach(cleanup);

vi.mock('../src/Folder.css', () => ({}));

import { ThemeProvider, createTheme } from '@mui/material/styles';

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

    // ── 다양한 fileType 아이콘 렌더링 (getIconFromFileType 브랜치 커버) ──

    it('image 계열 fileType 항목을 렌더링한다', () => {
        const data = [
            {
                id: 'images',
                label: 'images',
                fileType: 'folder',
                children: [
                    { id: 'images/1', label: 'photo.jpg', fileType: 'jpg', children: [] },
                    { id: 'images/2', label: 'pic.png', fileType: 'png', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['images']} />
        );
        expect(screen.getByText('photo.jpg')).toBeTruthy();
        expect(screen.getByText('pic.png')).toBeTruthy();
    });

    it('doc/docx fileType 항목을 렌더링한다', () => {
        const data = [
            {
                id: 'docs',
                label: 'docs',
                fileType: 'folder',
                children: [
                    { id: 'docs/1', label: 'report.doc', fileType: 'doc', children: [] },
                    { id: 'docs/2', label: 'resume.docx', fileType: 'docx', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['docs']} />
        );
        expect(screen.getByText('report.doc')).toBeTruthy();
        expect(screen.getByText('resume.docx')).toBeTruthy();
    });

    it('rtf/html/txt fileType 항목을 렌더링한다', () => {
        const data = [
            {
                id: 'texts',
                label: 'texts',
                fileType: 'folder',
                children: [
                    { id: 'texts/1', label: 'note.rtf', fileType: 'rtf', children: [] },
                    { id: 'texts/2', label: 'page.html', fileType: 'html', children: [] },
                    { id: 'texts/3', label: 'readme.txt', fileType: 'txt', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['texts']} />
        );
        expect(screen.getByText('note.rtf')).toBeTruthy();
        expect(screen.getByText('page.html')).toBeTruthy();
        expect(screen.getByText('readme.txt')).toBeTruthy();
    });

    it('video fileType 항목을 렌더링한다', () => {
        const data = [
            {
                id: 'videos',
                label: 'videos',
                fileType: 'folder',
                children: [
                    { id: 'videos/1', label: 'clip.mp4', fileType: 'video', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['videos']} />
        );
        expect(screen.getByText('clip.mp4')).toBeTruthy();
    });

    it('pinned fileType 항목을 렌더링한다', () => {
        const data = [
            {
                id: 'pinned_folder',
                label: 'pinned_folder',
                fileType: 'folder',
                children: [
                    { id: 'pinned_folder/1', label: 'starred', fileType: 'pinned', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['pinned_folder']} />
        );
        expect(screen.getByText('starred')).toBeTruthy();
    });

    it('trash fileType 항목을 렌더링한다', () => {
        const data = [
            {
                id: 'trash_folder',
                label: 'trash_folder',
                fileType: 'folder',
                children: [
                    { id: 'trash_folder/1', label: 'deleted', fileType: 'trash', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['trash_folder']} />
        );
        expect(screen.getByText('deleted')).toBeTruthy();
    });

    it('여러 자식이 있는 폴더를 펼치면 모든 자식을 렌더링한다', () => {
        const data = [
            {
                id: 'multi',
                label: 'multi',
                fileType: 'folder',
                children: [
                    { id: 'multi/1', label: 'a.doc', fileType: 'doc', children: [] },
                    { id: 'multi/2', label: 'b.jpg', fileType: 'jpg', children: [] },
                    { id: 'multi/3', label: 'c.video', fileType: 'video', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['multi']} />
        );
        expect(screen.getByText('a.doc')).toBeTruthy();
        expect(screen.getByText('b.jpg')).toBeTruthy();
        expect(screen.getByText('c.video')).toBeTruthy();
    });

    it('알 수 없는 fileType은 기본 아이콘으로 렌더링한다', () => {
        const data = [
            {
                id: 'misc',
                label: 'misc',
                fileType: 'folder',
                children: [
                    { id: 'misc/1', label: 'data.xyz', fileType: 'xyz', children: [] },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['misc']} />
        );
        expect(screen.getByText('data.xyz')).toBeTruthy();
    });

    it('folder fileType의 리프 항목(children 없음)을 렌더링한다', () => {
        const data = [
            {
                id: 'parent',
                label: 'parent',
                fileType: 'folder',
                children: [
                    { id: 'parent/sub', label: 'subfolder', fileType: 'folder' },
                ],
            },
        ];
        render(
            <Folder {...defaultProps} folderData={data} expandedItems={['parent']} />
        );
        expect(screen.getByText('subfolder')).toBeTruthy();
    });
});

describe('Folder - 다크 테마 및 파일 타입 아이콘', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it('다크 테마에서도 트리를 렌더링한다', () => {
        render(
            <ThemeProvider theme={createTheme({ palette: { mode: 'dark' } })}>
                <Folder {...defaultProps} expandedItems={['1_fiction']} />
            </ThemeProvider>
        );
        expect(screen.getByText('1_fiction')).toBeTruthy();
        expect(screen.getByText('소설A.epub')).toBeTruthy();
    });

    it('fileType="image" 항목도 아이콘과 함께 렌더링한다', () => {
        const data = [
            {
                id: 'gallery',
                label: 'gallery',
                fileType: 'folder',
                count: 1,
                children: [
                    { id: 'gallery/1', label: '표지.image', fileType: 'image', children: [] },
                ],
            },
        ];
        render(<Folder {...defaultProps} folderData={data} expandedItems={['gallery']} />);
        expect(screen.getByText('표지.image')).toBeTruthy();
    });
});
