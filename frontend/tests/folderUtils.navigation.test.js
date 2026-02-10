// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { determineNextEntryId, determinePrevEntryId } from '../src/folderUtils';

// ── 테스트 데이터 ──

// root files (폴더가 아닌 최상위 파일들)
const rootFiles = [
    { id: '/100', label: 'book1.pdf', fileType: 'pdf', children: [], book: {} },
    { id: '/200', label: 'book2.epub', fileType: 'epub', children: [], book: {} },
    { id: '/300', label: 'book3.pdf', fileType: 'pdf', children: [], book: {} },
];

// 폴더 + root files 혼합
const mixedFolderData = [
    {
        id: 'Fiction',
        label: 'Fiction',
        fileType: 'folder',
        booksLoaded: true,
        children: [
            { id: 'Fiction/10', label: 'novel1.epub', fileType: 'epub', children: [], book: {} },
            { id: 'Fiction/20', label: 'novel2.epub', fileType: 'epub', children: [], book: {} },
            { id: 'Fiction/30', label: 'novel3.epub', fileType: 'epub', children: [], book: {} },
        ],
    },
    {
        id: 'Science',
        label: 'Science',
        fileType: 'folder',
        booksLoaded: true,
        children: [
            { id: 'Science/40', label: 'physics.pdf', fileType: 'pdf', children: [], book: {} },
        ],
    },
    ...rootFiles,
];

// 하위 폴더가 섞인 경우 (폴더와 책이 같은 children에 있을 때)
const folderWithSubfolders = [
    {
        id: 'Literature',
        label: 'Literature',
        fileType: 'folder',
        booksLoaded: true,
        children: [
            { id: 'Literature/Korean', label: 'Korean', fileType: 'folder', children: [] },
            { id: 'Literature/50', label: 'classic1.pdf', fileType: 'pdf', children: [], book: {} },
            { id: 'Literature/60', label: 'classic2.pdf', fileType: 'pdf', children: [], book: {} },
        ],
    },
];

// ── determineNextEntryId ──

describe('determineNextEntryId', () => {
    describe('root files', () => {
        it('중간 root file에서 다음 파일 ID를 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, '/100')).toBe('/200');
            expect(determineNextEntryId(mixedFolderData, '/200')).toBe('/300');
        });

        it('마지막 root file에서 null을 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, '/300')).toBeNull();
        });

        it('존재하지 않는 root file에서 null을 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, '/999')).toBeNull();
        });

        it('root file이 하나뿐이면 null을 반환한다', () => {
            const singleRoot = [{ id: '/100', label: 'only.pdf', fileType: 'pdf', children: [], book: {} }];
            expect(determineNextEntryId(singleRoot, '/100')).toBeNull();
        });
    });

    describe('폴더 내 파일', () => {
        it('중간 책에서 다음 책 ID를 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, 'Fiction/10')).toBe('Fiction/20');
            expect(determineNextEntryId(mixedFolderData, 'Fiction/20')).toBe('Fiction/30');
        });

        it('마지막 책에서 null을 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, 'Fiction/30')).toBeNull();
        });

        it('폴더에 책이 하나뿐이면 null을 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, 'Science/40')).toBeNull();
        });

        it('존재하지 않는 entry에서 null을 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, 'Fiction/999')).toBeNull();
        });

        it('존재하지 않는 카테고리에서 null을 반환한다', () => {
            expect(determineNextEntryId(mixedFolderData, 'Unknown/10')).toBeNull();
        });
    });

    describe('하위 폴더가 섞인 경우', () => {
        it('하위 폴더를 건너뛰고 책만 순회한다', () => {
            expect(determineNextEntryId(folderWithSubfolders, 'Literature/50')).toBe('Literature/60');
        });

        it('마지막 책에서 null을 반환한다 (하위 폴더 무시)', () => {
            expect(determineNextEntryId(folderWithSubfolders, 'Literature/60')).toBeNull();
        });
    });
});

// ── determinePrevEntryId ──

describe('determinePrevEntryId', () => {
    describe('root files', () => {
        it('중간 root file에서 이전 파일 ID를 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, '/200')).toBe('/100');
            expect(determinePrevEntryId(mixedFolderData, '/300')).toBe('/200');
        });

        it('첫 번째 root file에서 null을 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, '/100')).toBeNull();
        });

        it('존재하지 않는 root file에서 null을 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, '/999')).toBeNull();
        });

        it('root file이 하나뿐이면 null을 반환한다', () => {
            const singleRoot = [{ id: '/100', label: 'only.pdf', fileType: 'pdf', children: [], book: {} }];
            expect(determinePrevEntryId(singleRoot, '/100')).toBeNull();
        });
    });

    describe('폴더 내 파일', () => {
        it('중간 책에서 이전 책 ID를 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, 'Fiction/20')).toBe('Fiction/10');
            expect(determinePrevEntryId(mixedFolderData, 'Fiction/30')).toBe('Fiction/20');
        });

        it('첫 번째 책에서 null을 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, 'Fiction/10')).toBeNull();
        });

        it('폴더에 책이 하나뿐이면 null을 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, 'Science/40')).toBeNull();
        });

        it('존재하지 않는 entry에서 null을 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, 'Fiction/999')).toBeNull();
        });

        it('존재하지 않는 카테고리에서 null을 반환한다', () => {
            expect(determinePrevEntryId(mixedFolderData, 'Unknown/10')).toBeNull();
        });
    });

    describe('하위 폴더가 섞인 경우', () => {
        it('하위 폴더를 건너뛰고 책만 순회한다', () => {
            expect(determinePrevEntryId(folderWithSubfolders, 'Literature/60')).toBe('Literature/50');
        });

        it('첫 번째 책에서 null을 반환한다 (하위 폴더 무시)', () => {
            expect(determinePrevEntryId(folderWithSubfolders, 'Literature/50')).toBeNull();
        });
    });
});

// ── next/prev 대칭성 ──

describe('next/prev 대칭성', () => {
    it('next로 이동한 뒤 prev로 돌아오면 원래 위치이다', () => {
        const nextId = determineNextEntryId(mixedFolderData, 'Fiction/10');
        expect(nextId).toBe('Fiction/20');
        const backId = determinePrevEntryId(mixedFolderData, nextId);
        expect(backId).toBe('Fiction/10');
    });

    it('prev로 이동한 뒤 next로 돌아오면 원래 위치이다', () => {
        const prevId = determinePrevEntryId(mixedFolderData, '/300');
        expect(prevId).toBe('/200');
        const forwardId = determineNextEntryId(mixedFolderData, prevId);
        expect(forwardId).toBe('/300');
    });

    it('첫 번째 항목은 prev=null, next!=null이다', () => {
        expect(determinePrevEntryId(mixedFolderData, 'Fiction/10')).toBeNull();
        expect(determineNextEntryId(mixedFolderData, 'Fiction/10')).not.toBeNull();
    });

    it('마지막 항목은 next=null, prev!=null이다', () => {
        expect(determineNextEntryId(mixedFolderData, 'Fiction/30')).toBeNull();
        expect(determinePrevEntryId(mixedFolderData, 'Fiction/30')).not.toBeNull();
    });

    it('유일한 항목은 next=null, prev=null이다', () => {
        expect(determineNextEntryId(mixedFolderData, 'Science/40')).toBeNull();
        expect(determinePrevEntryId(mixedFolderData, 'Science/40')).toBeNull();
    });
});
