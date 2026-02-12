// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { determineNextEntryId, determinePrevEntryId, findCommonPrefix, buildFolderHierarchy, parseEntryId, findFolderInTree, updateFolderInTree, updateFolderChildren } from '../src/folderUtils';

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

// ── findCommonPrefix ──

describe('findCommonPrefix', () => {
    it('빈 배열이면 빈 문자열을 반환한다', () => {
        expect(findCommonPrefix([])).toBe('');
    });

    it('null이면 빈 문자열을 반환한다', () => {
        expect(findCommonPrefix(null)).toBe('');
    });

    it('단일 항목에서 부모 prefix를 추출한다', () => {
        expect(findCommonPrefix(['소설/SF/한국SF'])).toBe('소설/SF/');
    });

    it('단일 항목에 슬래시가 없으면 빈 문자열을 반환한다', () => {
        expect(findCommonPrefix(['소설'])).toBe('');
    });

    it('공통 prefix를 찾는다', () => {
        expect(findCommonPrefix(['문학/소설', '문학/시'])).toBe('문학/');
    });

    it('공통 prefix가 없으면 빈 문자열을 반환한다', () => {
        expect(findCommonPrefix(['소설/SF', '역사/한국사'])).toBe('');
    });

    it('다단계 공통 prefix를 찾는다', () => {
        expect(findCommonPrefix(['A/B/C/D', 'A/B/C/E', 'A/B/C/F'])).toBe('A/B/C/');
    });

    it('완전히 동일한 문자열이면 마지막 세그먼트를 제외한 prefix를 반환한다', () => {
        expect(findCommonPrefix(['소설/SF', '소설/SF'])).toBe('소설/');
    });
});

// ── parseEntryId ──

describe('parseEntryId', () => {
    it('null이면 null을 반환한다', () => {
        expect(parseEntryId(null)).toBeNull();
    });

    it('빈 문자열이면 null을 반환한다', () => {
        expect(parseEntryId('')).toBeNull();
    });

    it('root file ID를 파싱한다', () => {
        expect(parseEntryId('/12345')).toEqual({ category: '_root', bookId: '12345' });
    });

    it('카테고리/bookId를 분리한다', () => {
        expect(parseEntryId('Fiction/100')).toEqual({ category: 'Fiction', bookId: '100' });
    });

    it('다단계 카테고리를 파싱한다', () => {
        expect(parseEntryId('Fiction/Novels/12345')).toEqual({ category: 'Fiction/Novels', bookId: '12345' });
    });

    it('bookId가 숫자가 아니면 null을 반환한다', () => {
        expect(parseEntryId('Fiction/subfolder')).toBeNull();
    });

    it('슬래시가 없는 문자열이면 null을 반환한다', () => {
        expect(parseEntryId('noSlash')).toBeNull();
    });
});

// ── buildFolderHierarchy ──

describe('buildFolderHierarchy', () => {
    it('1 depth 카테고리를 폴더로 생성한다', () => {
        const result = buildFolderHierarchy(['소설', '역사'], '');
        expect(result).toHaveLength(2);
        expect(result[0].label).toBe('소설');
        expect(result[0].fileType).toBe('folder');
    });

    it('2 depth 카테고리에서 부모-자식 관계를 생성한다', () => {
        const result = buildFolderHierarchy(['문학/소설', '문학/시'], '');
        expect(result).toHaveLength(1);
        expect(result[0].label).toBe('문학');
        expect(result[0].isVirtualParent).toBe(true);
        expect(result[0].children).toHaveLength(2);
    });

    it('부모 카테고리가 있으면 isVirtualParent=false로 생성한다', () => {
        const result = buildFolderHierarchy(['문학', '문학/소설', '문학/시'], '');
        expect(result).toHaveLength(1);
        expect(result[0].isVirtualParent).toBe(false);
        expect(result[0].children).toHaveLength(2);
    });

    it('prefix를 제거하고 계층 구조를 생성한다', () => {
        const result = buildFolderHierarchy(['Library/소설', 'Library/역사'], 'Library/');
        expect(result).toHaveLength(2);
        expect(result[0].label).toBe('소설');
    });

    it('categoryCounts를 반영한다', () => {
        const counts = { '소설': 10, '역사': 5 };
        const result = buildFolderHierarchy(['소설', '역사'], '', counts);
        expect(result[0].count).toBe(10);
        expect(result[1].count).toBe(5);
    });
});

// ── findFolderInTree ──

describe('findFolderInTree', () => {
    const tree = [
        { id: '소설', label: '소설', children: [{ id: '소설/SF', label: 'SF' }] },
        { id: '역사', label: '역사' },
    ];

    it('1단계에서 폴더를 찾는다', () => {
        expect(findFolderInTree(tree, '소설').label).toBe('소설');
    });

    it('2단계 children에서 폴더를 찾는다', () => {
        expect(findFolderInTree(tree, '소설/SF').label).toBe('SF');
    });

    it('존재하지 않는 ID는 null을 반환한다', () => {
        expect(findFolderInTree(tree, '과학')).toBeNull();
    });
});

// ── updateFolderInTree ──

describe('updateFolderInTree', () => {
    const tree = [
        { id: '소설', label: '소설', children: [{ id: '소설/SF', label: 'SF', count: 0 }] },
        { id: '역사', label: '역사' },
    ];

    it('1단계 항목을 업데이트한다', () => {
        const result = updateFolderInTree(tree, '역사', item => ({ ...item, count: 5 }));
        expect(result.find(i => i.id === '역사').count).toBe(5);
    });

    it('2단계 children 항목을 업데이트한다', () => {
        const result = updateFolderInTree(tree, '소설/SF', item => ({ ...item, count: 3 }));
        const child = result.find(i => i.id === '소설').children.find(c => c.id === '소설/SF');
        expect(child.count).toBe(3);
    });

    it('존재하지 않는 ID는 원본을 유지한다', () => {
        const result = updateFolderInTree(tree, '과학', item => ({ ...item, count: 1 }));
        expect(result).toEqual(tree);
    });
});

// ── updateFolderChildren ──

describe('updateFolderChildren', () => {
    it('children 배열을 업데이트한다', () => {
        const tree = [{ id: '소설', children: [{ id: 'a' }, { id: 'b' }] }];
        const result = updateFolderChildren(tree, '소설', children => [...children, { id: 'c' }]);
        expect(result[0].children).toHaveLength(3);
    });

    it('children이 없는 항목도 빈 배열로 처리한다', () => {
        const tree = [{ id: '역사' }];
        const result = updateFolderChildren(tree, '역사', children => [...children, { id: 'x' }]);
        expect(result[0].children).toHaveLength(1);
    });
});
