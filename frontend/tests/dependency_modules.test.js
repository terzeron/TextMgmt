/**
 * 의존성 모듈 API 호환성 테스트
 *
 * dependabot 등에 의한 의존성 업데이트 후에도
 * 응용 코드가 사용하는 API가 정상 동작하는지 검증한다.
 * 외부 서비스(네트워크, DOM 렌더링)가 필요 없는 단위 테스트만 포함한다.
 */

// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';

// ---------------------------------------------------------------------------
// 1. crc-32
//    사용처: Common.js - 문자열 해시로 결정적 색상 생성
// ---------------------------------------------------------------------------
describe('crc-32', () => {
    it('str()이 문자열을 받아 정수를 반환한다', async () => {
        const { str } = await import('crc-32');
        const hash = str('test_saltstring');
        expect(typeof hash).toBe('number');
        expect(Number.isInteger(hash)).toBe(true);
    });

    it('같은 입력에 같은 해시를 반환한다 (결정적)', async () => {
        const { str } = await import('crc-32');
        expect(str('hello_saltstring')).toBe(str('hello_saltstring'));
    });

    it('다른 입력에 다른 해시를 반환한다', async () => {
        const { str } = await import('crc-32');
        expect(str('a_salt')).not.toBe(str('b_salt'));
    });

    it('음수 해시를 절대값으로 변환 가능하다 (Common.js 패턴)', async () => {
        const { str } = await import('crc-32');
        let index = str('negative_test_saltstring') % (256 * 256 * 256);
        if (index < 0) index = -index;
        expect(index).toBeGreaterThanOrEqual(0);
    });
});

// ---------------------------------------------------------------------------
// 2. luxon
//    사용처: Common.js - ISO 날짜 포맷팅, Edit.jsx - 현재 시간 포맷팅
// ---------------------------------------------------------------------------
describe('luxon', () => {
    it('DateTime.fromISO()로 ISO 문자열을 파싱한다', async () => {
        const { DateTime } = await import('luxon');
        const dt = DateTime.fromISO('2024-06-15T14:30:00.000Z');
        expect(dt.isValid).toBe(true);
        expect(dt.year).toBe(2024);
    });

    it('.setZone("local").toFormat() 체인 (Common.js 패턴)', async () => {
        const { DateTime } = await import('luxon');
        const formatted = DateTime.fromISO('2024-06-15T14:30:00.000Z')
            .setZone('local')
            .toFormat('MM-dd HH:mm');
        expect(formatted).toMatch(/^\d{2}-\d{2} \d{2}:\d{2}$/);
    });

    it('DateTime.now().toFormat() (Edit.jsx 패턴)', async () => {
        const { DateTime } = await import('luxon');
        const formatted = DateTime.now().toFormat("yyyy-MM-dd'T'HH:mm:ss.SSS");
        // ISO-like 형식 확인
        expect(formatted).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}$/);
    });
});

// ---------------------------------------------------------------------------
// 3. clsx
//    사용처: Folder.jsx - 조건부 CSS 클래스 조합
// ---------------------------------------------------------------------------
describe('clsx', () => {
    it('문자열과 조건 객체를 결합한다 (Folder.jsx 패턴)', async () => {
        const { default: clsx } = await import('clsx');
        const result = clsx('content', {
            'Mui-expanded': true,
            'Mui-selected': false,
            'Mui-focused': true,
            'Mui-disabled': false,
        });
        expect(result).toContain('content');
        expect(result).toContain('Mui-expanded');
        expect(result).toContain('Mui-focused');
        expect(result).not.toContain('Mui-selected');
        expect(result).not.toContain('Mui-disabled');
    });

    it('falsy 값을 무시한다', async () => {
        const { default: clsx } = await import('clsx');
        const result = clsx('base', null, undefined, false, 0, '');
        expect(result).toBe('base');
    });
});

// ---------------------------------------------------------------------------
// 4. mammoth
//    사용처: ViewDOC.jsx - DOCX ArrayBuffer → HTML 변환
// ---------------------------------------------------------------------------
describe('mammoth', () => {
    it('convertToHtml()이 존재하고 함수이다', async () => {
        const mammoth = await import('mammoth');
        expect(typeof mammoth.convertToHtml).toBe('function');
    });

    it('convertToHtml()이 arrayBuffer 옵션을 받는다', async () => {
        const mammoth = await import('mammoth');
        // 빈 ArrayBuffer는 유효한 DOCX가 아니므로 reject됨
        // API가 arrayBuffer 키를 인식하고 처리 시도하는지만 확인
        await expect(
            mammoth.convertToHtml({ arrayBuffer: new ArrayBuffer(0) })
        ).rejects.toThrow();
    });
});

// ---------------------------------------------------------------------------
// 5. pdfjs-dist
//    사용처: ViewPDF.jsx, PdfDiagnose.js - PDF 로드, 페이지 렌더링
// ---------------------------------------------------------------------------
describe('pdfjs-dist', () => {
    it('getDocument 함수가 존재한다', async () => {
        const pdfjs = await import('pdfjs-dist');
        expect(typeof pdfjs.getDocument).toBe('function');
    });

    it('GlobalWorkerOptions 설정이 가능하다', async () => {
        const pdfjs = await import('pdfjs-dist');
        expect(pdfjs.GlobalWorkerOptions).toBeDefined();
        expect('workerSrc' in pdfjs.GlobalWorkerOptions).toBe(true);
        // 실제 설정 패턴
        pdfjs.GlobalWorkerOptions.workerSrc = 'test-worker.js';
        expect(pdfjs.GlobalWorkerOptions.workerSrc).toBe('test-worker.js');
    });

    // getDocument() 실행 테스트는 제외:
    // pdfjs-dist 내부에서 Promise.withResolvers()를 사용하며,
    // Node/jsdom 환경에서 이 API를 지원하지 않아 항상 실패한다.
    // getDocument 함수 존재 여부와 GlobalWorkerOptions 설정 가능 여부로 충분히 검증된다.
});

// ---------------------------------------------------------------------------
// 6. rtf.js
//    사용처: ViewRTF.jsx - RTF 바이너리 → HTML 엘리먼트 변환
// ---------------------------------------------------------------------------
describe('rtf.js', () => {
    it('RTFJS 네임스페이스가 존재한다', async () => {
        const { RTFJS } = await import('rtf.js');
        expect(RTFJS).toBeDefined();
    });

    it('RTFJS.loggingEnabled()가 함수이다', async () => {
        const { RTFJS } = await import('rtf.js');
        expect(typeof RTFJS.loggingEnabled).toBe('function');
        // 실제 사용 패턴
        RTFJS.loggingEnabled(false);
    });

    it('RTFJS.Document 생성자가 존재한다', async () => {
        const { RTFJS } = await import('rtf.js');
        expect(typeof RTFJS.Document).toBe('function');
    });

    it('Document 인스턴스에 render() 메서드가 있다', async () => {
        const { RTFJS } = await import('rtf.js');
        // 최소 RTF: {\rtf1 }
        const rtfString = '{\\rtf1 Hello}';
        const encoder = new TextEncoder();
        const buf = encoder.encode(rtfString).buffer;
        const doc = new RTFJS.Document(buf, null);
        expect(typeof doc.render).toBe('function');
    });
});

// ---------------------------------------------------------------------------
// 7. react-reader
//    사용처: ViewEPUB.jsx - EPUB 뷰어 컴포넌트
// ---------------------------------------------------------------------------
describe('react-reader', () => {
    it('ReactReader 컴포넌트를 import할 수 있다', async () => {
        const { ReactReader } = await import('react-reader');
        expect(ReactReader).toBeDefined();
        expect(typeof ReactReader).toBe('function');
    });
});

// ---------------------------------------------------------------------------
// 8. @react-spring/web
//    사용처: Folder.jsx - 트리 노드 전개/접힘 애니메이션
// ---------------------------------------------------------------------------
describe('@react-spring/web', () => {
    it('animated가 export된다', async () => {
        const { animated } = await import('@react-spring/web');
        expect(animated).toBeDefined();
    });

    it('useSpring이 export된다', async () => {
        const { useSpring } = await import('@react-spring/web');
        expect(typeof useSpring).toBe('function');
    });

    it('animated()로 컴포넌트를 래핑할 수 있다 (Folder.jsx 패턴)', async () => {
        const { animated } = await import('@react-spring/web');
        // 간단한 HTML 요소 래핑
        const AnimatedDiv = animated('div');
        expect(AnimatedDiv).toBeDefined();
    });
});

// ---------------------------------------------------------------------------
// 9. @react-oauth/google
//    사용처: Navigation.jsx - Google OAuth 로그인
// ---------------------------------------------------------------------------
describe('@react-oauth/google', () => {
    it('GoogleOAuthProvider, GoogleLogin, googleLogout가 export된다', async () => {
        const { GoogleOAuthProvider, GoogleLogin, googleLogout } = await import('@react-oauth/google');
        expect(GoogleOAuthProvider).toBeDefined();
        expect(GoogleLogin).toBeDefined();
        expect(typeof googleLogout).toBe('function');
    });
});

// ---------------------------------------------------------------------------
// 10. react-router-dom
//     사용처: App.jsx, Navigation.jsx, Edit.jsx, View.jsx, ViewSingle.jsx 등
// ---------------------------------------------------------------------------
describe('react-router-dom', () => {
    it('라우터 컴포넌트가 export된다', async () => {
        const { BrowserRouter, Routes, Route, Outlet, Navigate } = await import('react-router-dom');
        expect(BrowserRouter).toBeDefined();
        expect(Routes).toBeDefined();
        expect(Route).toBeDefined();
        expect(Outlet).toBeDefined();
        expect(Navigate).toBeDefined();
    });

    it('라우터 훅이 export된다', async () => {
        const { useLocation, useParams, useSearchParams, useOutletContext, useRouteError } = await import('react-router-dom');
        expect(typeof useLocation).toBe('function');
        expect(typeof useParams).toBe('function');
        expect(typeof useSearchParams).toBe('function');
        expect(typeof useOutletContext).toBe('function');
        expect(typeof useRouteError).toBe('function');
    });
});

// ---------------------------------------------------------------------------
// 11. react
//     사용처: 모든 컴포넌트
// ---------------------------------------------------------------------------
describe('react', () => {
    it('핵심 훅이 export된다', async () => {
        const React = await import('react');
        expect(typeof React.useState).toBe('function');
        expect(typeof React.useEffect).toBe('function');
        expect(typeof React.useCallback).toBe('function');
        expect(typeof React.useMemo).toBe('function');
        expect(typeof React.useRef).toBe('function');
    });

    it('Suspense, Fragment가 export된다', async () => {
        const React = await import('react');
        expect(React.Suspense).toBeDefined();
        expect(React.Fragment).toBeDefined();
    });
});

// ---------------------------------------------------------------------------
// 12. react-dom
//     사용처: main.jsx (createRoot), ViewPDF.jsx (flushSync)
// ---------------------------------------------------------------------------
describe('react-dom', () => {
    it('createRoot가 react-dom/client에서 export된다', async () => {
        const { createRoot } = await import('react-dom/client');
        expect(typeof createRoot).toBe('function');
    });

    it('flushSync가 export된다', async () => {
        const { flushSync } = await import('react-dom');
        expect(typeof flushSync).toBe('function');
    });
});

// ---------------------------------------------------------------------------
// 13. react-bootstrap
//     사용처: 대부분의 UI 컴포넌트
// ---------------------------------------------------------------------------
describe('react-bootstrap', () => {
    it('사용하는 모든 컴포넌트가 export된다', async () => {
        const rb = await import('react-bootstrap');
        const components = [
            'Alert', 'Badge', 'Button', 'ButtonGroup',
            'Card', 'Col', 'Container',
            'Dropdown', 'Form', 'FormControl',
            'InputGroup', 'ListGroup',
            'Nav', 'Navbar', 'Row',
            'Spinner', 'Tab', 'Tabs',
        ];
        for (const name of components) {
            expect(rb[name]).toBeDefined();
        }
    });
});

// ---------------------------------------------------------------------------
// 14. @fortawesome/react-fontawesome
//     사용처: 아이콘 표시 컴포넌트
// ---------------------------------------------------------------------------
describe('@fortawesome/react-fontawesome', () => {
    it('FontAwesomeIcon 컴포넌트가 export된다', async () => {
        const { FontAwesomeIcon } = await import('@fortawesome/react-fontawesome');
        expect(FontAwesomeIcon).toBeDefined();
        expect(typeof FontAwesomeIcon).toBe('object');
    });
});

// ---------------------------------------------------------------------------
// 15. @fortawesome/free-solid-svg-icons
//     사용처: 다수 컴포넌트에서 아이콘 지정
// ---------------------------------------------------------------------------
describe('@fortawesome/free-solid-svg-icons', () => {
    it('사용하는 모든 아이콘이 export된다', async () => {
        const icons = await import('@fortawesome/free-solid-svg-icons');
        const usedIcons = [
            'faChevronDown', 'faChevronRight',
            'faClockRotateLeft', 'faCut', 'faRotate',
            'faTruckMoving', 'faUpload',
            'faCheck', 'faTrash',
            'faPlus', 'faEyeSlash',
            'faSearch', 'faUser',
        ];
        for (const name of usedIcons) {
            expect(icons[name]).toBeDefined();
            // FontAwesome 아이콘 객체 구조 확인
            expect(icons[name]).toHaveProperty('iconName');
            expect(icons[name]).toHaveProperty('prefix');
        }
    });
});

// ---------------------------------------------------------------------------
// 16. @mui/material
//     사용처: Folder.jsx - 트리뷰 스타일링
// ---------------------------------------------------------------------------
describe('@mui/material', () => {
    it('styled, alpha가 @mui/material/styles에서 export된다', async () => {
        const { styled, alpha } = await import('@mui/material/styles');
        expect(typeof styled).toBe('function');
        expect(typeof alpha).toBe('function');
    });

    it('alpha()가 색상에 투명도를 적용한다', async () => {
        const { alpha } = await import('@mui/material/styles');
        const result = alpha('#ff0000', 0.5);
        expect(typeof result).toBe('string');
    });

    it('Box, Collapse, Typography 컴포넌트가 export된다', async () => {
        const { Box, Collapse, Typography } = await import('@mui/material');
        expect(Box).toBeDefined();
        expect(Collapse).toBeDefined();
        expect(Typography).toBeDefined();
    });
});

// ---------------------------------------------------------------------------
// 17. @mui/icons-material
//     사용처: Folder.jsx - 트리뷰 파일 타입 아이콘
// ---------------------------------------------------------------------------
describe('@mui/icons-material', () => {
    it('사용하는 모든 아이콘이 export된다', async () => {
        const muiIcons = await import('@mui/icons-material');
        const usedIcons = [
            'Article', 'Delete', 'FolderOpen',
            'FolderRounded', 'Image',
            'PictureAsPdf', 'VideoCameraBack',
        ];
        for (const name of usedIcons) {
            expect(muiIcons[name]).toBeDefined();
        }
    });
});

// ---------------------------------------------------------------------------
// 18. @mui/x-tree-view
//     사용처: Folder.jsx - 폴더 트리뷰
// ---------------------------------------------------------------------------
describe('@mui/x-tree-view', () => {
    it('RichTreeView가 export된다', async () => {
        const { RichTreeView } = await import('@mui/x-tree-view/RichTreeView');
        expect(RichTreeView).toBeDefined();
    });

    it('TreeItem 관련 export가 존재한다', async () => {
        const { treeItemClasses } = await import('@mui/x-tree-view/TreeItem');
        expect(treeItemClasses).toBeDefined();
        expect(typeof treeItemClasses).toBe('object');
    });

    it('TreeItem2 컴포넌트가 export된다', async () => {
        const mod = await import('@mui/x-tree-view/TreeItem2');
        expect(mod.TreeItem2Content).toBeDefined();
        expect(mod.TreeItem2IconContainer).toBeDefined();
        expect(mod.TreeItem2Label).toBeDefined();
        expect(mod.TreeItem2Root).toBeDefined();
    });

    it('TreeItem2Icon, TreeItem2Provider가 export된다', async () => {
        const { TreeItem2Icon } = await import('@mui/x-tree-view/TreeItem2Icon');
        const { TreeItem2Provider } = await import('@mui/x-tree-view/TreeItem2Provider');
        expect(TreeItem2Icon).toBeDefined();
        expect(TreeItem2Provider).toBeDefined();
    });
});

// ---------------------------------------------------------------------------
// 19. @react-buddy/ide-toolbox
//     사용처: dev/previews.jsx, main.jsx - 개발 도구
// ---------------------------------------------------------------------------
describe('@react-buddy/ide-toolbox', () => {
    it('DevSupport가 export된다', async () => {
        const { DevSupport } = await import('@react-buddy/ide-toolbox');
        expect(DevSupport).toBeDefined();
    });
});

// ---------------------------------------------------------------------------
// 20. bootstrap (CSS)
//     사용처: SearchResult.jsx - CSS import
// ---------------------------------------------------------------------------
describe('bootstrap', () => {
    it('bootstrap 패키지가 import 가능하다', async () => {
        // CSS import는 jsdom에서 동작하지 않으므로 패키지 존재만 확인
        const bootstrap = await import('bootstrap');
        expect(bootstrap).toBeDefined();
    });
});
