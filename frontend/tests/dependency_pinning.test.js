/**
 * Dependency Pinning Test
 *
 * `frontend/src` 가 실제로 사용하는 모든 외부 모듈의 사용 방식을 박제(freeze)한다.
 * Dependabot 등으로 의존성 버전이 갱신되더라도 클라이언트 코드가 의존하는
 * export 이름, 호출 시그니처, 반환 형태가 그대로 유지되는지 검증한다.
 *
 * 본 파일이 깨지면 두 가지 중 하나다.
 *   1) 새 버전이 breaking change를 도입했다 → 코드 수정 또는 버전 고정 필요.
 *   2) src 의 사용 패턴이 바뀌었다 → 테스트도 함께 갱신.
 *
 * 본 파일은 외부 서비스(네트워크, 실제 PDF 렌더링)를 호출하지 않는다.
 * 동작 검증은 호환 가능한 최소 입력으로만 수행한다.
 */

// @vitest-environment jsdom
import { describe, it, expect } from "vitest";

// ===========================================================================
// 1. crc-32 — Common.js
//    `import { str } from "crc-32"`
//    str(key + "_saltstring") % (256 * 256 * 256) 형태로 결정적 색상 생성
// ===========================================================================
describe("crc-32", () => {
  it("named export `str` 가 string→int 함수다", async () => {
    const { str } = await import("crc-32");
    expect(typeof str).toBe("function");
    const hash = str("hello_saltstring");
    expect(Number.isInteger(hash)).toBe(true);
  });

  it("같은 입력에 같은 해시 (Common.js getColorFromKey 결정성)", async () => {
    const { str } = await import("crc-32");
    expect(str("abc_saltstring")).toBe(str("abc_saltstring"));
    expect(str("abc_saltstring")).not.toBe(str("xyz_saltstring"));
  });

  it("`str(key) % (256^3)` 패턴이 안전하게 동작한다", async () => {
    const { str } = await import("crc-32");
    let index = str("k_saltstring") % (256 * 256 * 256);
    if (index < 0) index = -index;
    expect(index).toBeGreaterThanOrEqual(0);
    expect(index).toBeLessThan(256 * 256 * 256);
  });
});

// ===========================================================================
// 2. luxon — Common.js, Edit.jsx
//    Common.js: DateTime.fromISO(iso).setZone("local").toFormat("MM-dd HH:mm")
//    Edit.jsx:  DateTime.now().toFormat("yyyy-MM-dd'T'HH:mm:ss.SSS")
// ===========================================================================
describe("luxon", () => {
  it("named export `DateTime` 가 존재한다", async () => {
    const { DateTime } = await import("luxon");
    expect(DateTime).toBeDefined();
  });

  it("Common.js: fromISO().setZone('local').toFormat('MM-dd HH:mm')", async () => {
    const { DateTime } = await import("luxon");
    const formatted = DateTime.fromISO("2024-06-15T14:30:00.000Z")
      .setZone("local")
      .toFormat("MM-dd HH:mm");
    expect(formatted).toMatch(/^\d{2}-\d{2} \d{2}:\d{2}$/);
  });

  it("Edit.jsx: now().toFormat(\"yyyy-MM-dd'T'HH:mm:ss.SSS\")", async () => {
    const { DateTime } = await import("luxon");
    const formatted = DateTime.now().toFormat("yyyy-MM-dd'T'HH:mm:ss.SSS");
    expect(formatted).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}$/);
  });

  it("fromISO 결과는 .isValid / .year 속성을 제공한다", async () => {
    const { DateTime } = await import("luxon");
    const dt = DateTime.fromISO("2024-06-15T14:30:00.000Z");
    expect(dt.isValid).toBe(true);
    expect(dt.year).toBe(2024);
  });
});

// ===========================================================================
// 3. clsx — Folder.jsx, CategoryAdmin.jsx
//    clsx("content", { "Mui-expanded": ..., "Mui-selected": ..., "Mui-focused": ..., "Mui-disabled": ... })
// ===========================================================================
describe("clsx", () => {
  it("default export 가 함수다", async () => {
    const { default: clsx } = await import("clsx");
    expect(typeof clsx).toBe("function");
  });

  it("Folder.jsx 패턴: 문자열 + 조건 객체", async () => {
    const { default: clsx } = await import("clsx");
    const result = clsx("content", {
      "Mui-expanded": true,
      "Mui-selected": false,
      "Mui-focused": true,
      "Mui-disabled": false,
    });
    expect(result.split(" ").sort()).toEqual(
      ["Mui-expanded", "Mui-focused", "content"].sort(),
    );
  });

  it("falsy 값을 무시한다", async () => {
    const { default: clsx } = await import("clsx");
    expect(clsx("base", null, undefined, false, 0, "")).toBe("base");
  });
});

// ===========================================================================
// 4. mammoth — ViewDOC.jsx
//    mammoth.convertToHtml({ arrayBuffer: buffer })
// ===========================================================================
describe("mammoth", () => {
  it("default import 에 convertToHtml 함수가 있다", async () => {
    const mammoth = (await import("mammoth")).default;
    expect(typeof mammoth.convertToHtml).toBe("function");
  });

  it("convertToHtml 호출이 thenable 을 반환한다 (ViewDOC.jsx 패턴)", async () => {
    const mammoth = (await import("mammoth")).default;
    // 실제 DOCX 디코딩 결과는 검증 대상 아님. Promise 반환 여부만 확인.
    // 내부 streaming reject 가 unhandled 로 노출되지 않게 즉시 catch.
    const p = mammoth.convertToHtml({ arrayBuffer: new ArrayBuffer(0) });
    expect(typeof p.then).toBe("function");
    await p.catch(() => {});
  });
});

// ===========================================================================
// 5. dompurify — ViewDOC.jsx, ViewRTF.jsx
//    DOMPurify.sanitize(html)
//    DOMPurify.addHook("afterSanitizeAttributes", fn)
//    DOMPurify.removeHook("afterSanitizeAttributes")
// ===========================================================================
describe("dompurify", () => {
  it("default export 가 sanitize/addHook/removeHook 메서드를 가진다", async () => {
    const { default: DOMPurify } = await import("dompurify");
    expect(typeof DOMPurify.sanitize).toBe("function");
    expect(typeof DOMPurify.addHook).toBe("function");
    expect(typeof DOMPurify.removeHook).toBe("function");
  });

  it("sanitize() 가 위험한 태그를 제거한다", async () => {
    const { default: DOMPurify } = await import("dompurify");
    const dirty = "<p>safe</p><script>alert(1)</script>";
    const clean = DOMPurify.sanitize(dirty);
    expect(clean).toContain("<p>safe</p>");
    expect(clean).not.toContain("<script>");
  });

  it("ViewRTF.jsx 패턴: addHook → sanitize → removeHook", async () => {
    const { default: DOMPurify } = await import("dompurify");
    const blockSvgDataUri = (node) => {
      if (
        node.nodeName === "IMG" &&
        node.getAttribute("src")?.startsWith("data:image/svg")
      ) {
        node.removeAttribute("src");
      }
    };
    DOMPurify.addHook("afterSanitizeAttributes", blockSvgDataUri);
    const out = DOMPurify.sanitize('<img src="data:image/svg+xml;base64,XX"/>');
    expect(out).not.toContain("data:image/svg");
    DOMPurify.removeHook("afterSanitizeAttributes");
  });
});

// ===========================================================================
// 6. pdfjs-dist — ViewPDF.jsx, PdfDiagnose.js
//    `import * as pdfjs from "pdfjs-dist"`
//    pdfjs.GlobalWorkerOptions.workerSrc = "..."
//    pdfjs.getDocument({ data: buffer }).promise
//    pdfDoc.numPages / pdfDoc.getPage(n) / pdfDoc.getMetadata() / pdfDoc.destroy()
//    page.getViewport({ scale }) / page.render({ canvas, canvasContext, viewport })
// ===========================================================================
describe("pdfjs-dist", () => {
  it("`getDocument` 가 함수다", async () => {
    const pdfjs = await import("pdfjs-dist");
    expect(typeof pdfjs.getDocument).toBe("function");
  });

  it("GlobalWorkerOptions.workerSrc 를 설정할 수 있다", async () => {
    const pdfjs = await import("pdfjs-dist");
    expect(pdfjs.GlobalWorkerOptions).toBeDefined();
    expect("workerSrc" in pdfjs.GlobalWorkerOptions).toBe(true);
    pdfjs.GlobalWorkerOptions.workerSrc = "test-worker.js";
    expect(pdfjs.GlobalWorkerOptions.workerSrc).toBe("test-worker.js");
  });

  // getDocument({ data }) 의 실제 호출은 pdfjs-dist v5+ 내부에서
  // Promise.withResolvers / OffscreenCanvas / unhandled rejection 등을 발생시켜
  // jsdom 환경에서는 안정적으로 검증할 수 없다.
  // 함수 존재 여부와 GlobalWorkerOptions 설정 가능 여부로 충분히 박제된다.
});

// ===========================================================================
// 7. rtf.js — ViewRTF.jsx
//    RTFJS.loggingEnabled(false)
//    new RTFJS.Document(arrayBuffer, null).render() → Promise<Element[]>
// ===========================================================================
describe("rtf.js", () => {
  it("named export `RTFJS` 가 존재한다", async () => {
    const { RTFJS } = await import("rtf.js");
    expect(RTFJS).toBeDefined();
  });

  it("RTFJS.loggingEnabled 가 함수다 (false 호출 가능)", async () => {
    const { RTFJS } = await import("rtf.js");
    expect(typeof RTFJS.loggingEnabled).toBe("function");
    RTFJS.loggingEnabled(false);
  });

  it("new RTFJS.Document(buf, null).render() 패턴이 가능하다", async () => {
    const { RTFJS } = await import("rtf.js");
    expect(typeof RTFJS.Document).toBe("function");
    const buf = new TextEncoder().encode("{\\rtf1 Hello}").buffer;
    const doc = new RTFJS.Document(buf, null);
    expect(typeof doc.render).toBe("function");
  });
});

// ===========================================================================
// 8. jszip — EpubDiagnose.js
//    JSZip.loadAsync(arrayBuffer) → zip
//    zip.files, zip.file(path), file.async("text")
//    (transitive dep via react-reader/epubjs, package.json 에는 미선언)
// ===========================================================================
describe("jszip", () => {
  it("default export 가 클래스이며 정적 loadAsync 를 가진다", async () => {
    const JSZip = (await import("jszip")).default;
    expect(typeof JSZip).toBe("function");
    expect(typeof JSZip.loadAsync).toBe("function");
  });

  it("EpubDiagnose.js 패턴: loadAsync → files / file().async('text')", async () => {
    const JSZip = (await import("jszip")).default;
    const z = new JSZip();
    z.file("mimetype", "application/epub+zip");
    const blob = await z.generateAsync({ type: "arraybuffer" });
    const loaded = await JSZip.loadAsync(blob);
    expect(loaded.files).toBeDefined();
    expect(Object.keys(loaded.files)).toContain("mimetype");
    const entry = loaded.file("mimetype");
    expect(entry).not.toBeNull();
    const text = await entry.async("text");
    expect(text).toBe("application/epub+zip");
  });
});

// ===========================================================================
// 9. react-reader — ViewEPUB.jsx
//    `import { ReactReader } from "react-reader"`
// ===========================================================================
describe("react-reader", () => {
  it("ReactReader 가 React 컴포넌트(함수)다", async () => {
    const { ReactReader } = await import("react-reader");
    expect(ReactReader).toBeDefined();
    expect(typeof ReactReader).toBe("function");
  });
});

// ===========================================================================
// 10. @react-spring/web — Folder.jsx, CategoryAdmin.jsx
//     animated(Component), useSpring({...})
//     실제: `const AnimatedCollapse = animated(Collapse)`
// ===========================================================================
describe("@react-spring/web", () => {
  it("animated 와 useSpring 이 export 된다", async () => {
    const { animated, useSpring } = await import("@react-spring/web");
    expect(animated).toBeDefined();
    expect(typeof useSpring).toBe("function");
  });

  it("animated(string) 으로 HTML 요소 래핑이 가능하다", async () => {
    const { animated } = await import("@react-spring/web");
    const AnimatedDiv = animated("div");
    expect(AnimatedDiv).toBeDefined();
  });

  it("animated(Component) 로 임의 컴포넌트 래핑이 가능하다 (Folder.jsx 패턴)", async () => {
    const { animated } = await import("@react-spring/web");
    const Dummy = () => null;
    const Wrapped = animated(Dummy);
    expect(Wrapped).toBeDefined();
  });
});

// ===========================================================================
// 11. @react-oauth/google — Navigation.jsx
//     GoogleOAuthProvider, GoogleLogin, googleLogout
// ===========================================================================
describe("@react-oauth/google", () => {
  it("GoogleOAuthProvider / GoogleLogin / googleLogout 가 export 된다", async () => {
    const m = await import("@react-oauth/google");
    expect(m.GoogleOAuthProvider).toBeDefined();
    expect(m.GoogleLogin).toBeDefined();
    expect(typeof m.googleLogout).toBe("function");
  });
});

// ===========================================================================
// 12. react-router-dom — App.jsx, main.jsx, Navigation.jsx, View/Edit/...
//     컴포넌트: BrowserRouter, Routes, Route, Outlet, Navigate
//     훅: useLocation, useNavigate, useParams, useSearchParams,
//         useOutletContext, useRouteError
// ===========================================================================
describe("react-router-dom", () => {
  it("라우터 컴포넌트가 export 된다", async () => {
    const m = await import("react-router-dom");
    for (const k of [
      "BrowserRouter",
      "Routes",
      "Route",
      "Outlet",
      "Navigate",
    ]) {
      expect(m[k], `missing export: ${k}`).toBeDefined();
    }
  });

  it("라우터 훅이 모두 함수다", async () => {
    const m = await import("react-router-dom");
    for (const k of [
      "useLocation",
      "useNavigate",
      "useParams",
      "useSearchParams",
      "useOutletContext",
      "useRouteError",
    ]) {
      expect(typeof m[k], `not a function: ${k}`).toBe("function");
    }
  });
});

// ===========================================================================
// 13. react — 모든 컴포넌트, main.jsx (React.StrictMode)
// ===========================================================================
describe("react", () => {
  it("핵심 훅이 export 된다", async () => {
    const React = await import("react");
    for (const k of [
      "useState",
      "useEffect",
      "useCallback",
      "useMemo",
      "useRef",
      "lazy",
    ]) {
      expect(typeof React[k], `not a function: ${k}`).toBe("function");
    }
  });

  it("Fragment / Suspense / StrictMode 가 export 된다", async () => {
    const React = await import("react");
    expect(React.Fragment).toBeDefined();
    expect(React.Suspense).toBeDefined();
    expect(React.StrictMode).toBeDefined();
  });

  it("default import 가 createElement 를 제공한다 (main.jsx React.* 패턴)", async () => {
    const ReactDefault = (await import("react")).default;
    expect(typeof ReactDefault.createElement).toBe("function");
    expect(ReactDefault.StrictMode).toBeDefined();
  });
});

// ===========================================================================
// 14. react-dom / react-dom/client — main.jsx, ViewPDF.jsx
//     main.jsx:    ReactDOM.createRoot(el).render(<...>)
//     ViewPDF.jsx: flushSync(() => setState(...))
// ===========================================================================
describe("react-dom", () => {
  it("react-dom/client default 의 createRoot 가 함수다 (main.jsx 패턴)", async () => {
    const ReactDOM = (await import("react-dom/client")).default;
    expect(typeof ReactDOM.createRoot).toBe("function");
  });

  it("react-dom/client named export createRoot 도 함수다", async () => {
    const { createRoot } = await import("react-dom/client");
    expect(typeof createRoot).toBe("function");
  });

  it("flushSync 가 함수다 (ViewPDF.jsx 패턴)", async () => {
    const { flushSync } = await import("react-dom");
    expect(typeof flushSync).toBe("function");
    let ran = false;
    flushSync(() => {
      ran = true;
    });
    expect(ran).toBe(true);
  });
});

// ===========================================================================
// 15. react-bootstrap — 다수 컴포넌트
//     실제 src 에서 import 되는 모든 named export
// ===========================================================================
describe("react-bootstrap", () => {
  const usedExports = [
    "Alert",
    "Badge",
    "Button",
    "ButtonGroup",
    "Card",
    "Col",
    "Container",
    "Form",
    // src 에서는 `Form.Control` 도 함께 사용 → FormControl 별도 import 는 없으나 호환 검증
    "InputGroup",
    "Row",
    "Spinner",
    "Tab",
    "Table",
    "Tabs",
  ];

  it("src 에서 import 하는 모든 컴포넌트가 export 된다", async () => {
    const rb = await import("react-bootstrap");
    for (const name of usedExports) {
      expect(rb[name], `missing export: ${name}`).toBeDefined();
    }
  });

  it("Form.Control 패턴이 가능하다", async () => {
    const rb = await import("react-bootstrap");
    expect(rb.Form.Control).toBeDefined();
  });
});

// ===========================================================================
// 16. @fortawesome/react-fontawesome — 다수 컴포넌트
//     `<FontAwesomeIcon icon={faX} />`
// ===========================================================================
describe("@fortawesome/react-fontawesome", () => {
  it("FontAwesomeIcon 이 React 컴포넌트로 export 된다", async () => {
    const { FontAwesomeIcon } = await import("@fortawesome/react-fontawesome");
    expect(FontAwesomeIcon).toBeDefined();
    // memo() 결과는 객체이므로 'object' 또는 'function' 모두 허용
    expect(["object", "function"]).toContain(typeof FontAwesomeIcon);
  });
});

// ===========================================================================
// 17. @fortawesome/free-solid-svg-icons — src 에서 import 되는 아이콘 전수
// ===========================================================================
describe("@fortawesome/free-solid-svg-icons", () => {
  const used = [
    "faCheck",
    "faChevronDown",
    "faChevronRight",
    "faClockRotateLeft",
    "faCut",
    "faEdit",
    "faEyeSlash",
    "faPlus",
    "faRotate",
    "faSearch",
    "faSpinner",
    "faTrash",
    "faTruckMoving",
    "faUpload",
    "faUser",
  ];

  it("src 에서 사용하는 모든 솔리드 아이콘이 export 된다", async () => {
    const icons = await import("@fortawesome/free-solid-svg-icons");
    for (const name of used) {
      expect(icons[name], `missing icon: ${name}`).toBeDefined();
      expect(icons[name]).toHaveProperty("iconName");
      expect(icons[name]).toHaveProperty("prefix");
    }
  });
});

// ===========================================================================
// 18. @mui/material — Folder.jsx, CategoryAdmin.jsx
//     서브패스 import:
//       "@mui/material/Box", "@mui/material/Collapse", "@mui/material/Typography"
//       "@mui/material/styles" → { styled, alpha }
// ===========================================================================
describe("@mui/material", () => {
  it("서브패스에서 Box/Collapse/Typography 가 default export 된다", async () => {
    const Box = (await import("@mui/material/Box")).default;
    const Collapse = (await import("@mui/material/Collapse")).default;
    const Typography = (await import("@mui/material/Typography")).default;
    expect(Box).toBeDefined();
    expect(Collapse).toBeDefined();
    expect(Typography).toBeDefined();
  });

  it("@mui/material/styles 의 styled / alpha 가 함수다", async () => {
    const { styled, alpha } = await import("@mui/material/styles");
    expect(typeof styled).toBe("function");
    expect(typeof alpha).toBe("function");
  });

  it("alpha('#rrggbb', n) 가 문자열을 반환한다", async () => {
    const { alpha } = await import("@mui/material/styles");
    const v = alpha("#ff0000", 0.5);
    expect(typeof v).toBe("string");
  });

  it("styled(Component)(styleFn) 패턴이 동작한다", async () => {
    const { styled } = await import("@mui/material/styles");
    const Box = (await import("@mui/material/Box")).default;
    const Styled = styled(Box)(() => ({ color: "red" }));
    expect(Styled).toBeDefined();
  });
});

// ===========================================================================
// 19. @mui/icons-material — Folder.jsx, CategoryAdmin.jsx
//     `import X from "@mui/icons-material/X"` (v7: /esm/ deep import 제거됨)
// ===========================================================================
describe("@mui/icons-material", () => {
  const used = [
    "Article",
    "Delete",
    "FolderOpen",
    "FolderRounded",
    "Image",
    "PictureAsPdf",
    "VideoCameraBack",
  ];

  it("`<Name>` 경로로 default import 가 가능하다", async () => {
    for (const name of used) {
      const mod = await import(`@mui/icons-material/${name}`);
      expect(mod.default, `missing default export at ${name}`).toBeDefined();
    }
  }, 30000);
});

// ===========================================================================
// 20. @mui/x-tree-view — Folder.jsx, CategoryAdmin.jsx
//     RichTreeView, TreeItem (treeItemClasses), TreeItem{Content,IconContainer,Label,Root},
//     TreeItemIcon, TreeItemProvider, useTreeItem
// ===========================================================================
describe("@mui/x-tree-view", () => {
  it("RichTreeView 가 export 된다", async () => {
    const { RichTreeView } = await import("@mui/x-tree-view/RichTreeView");
    expect(RichTreeView).toBeDefined();
  });

  it("treeItemClasses 가 객체로 export 되며 스타일 키를 포함한다", async () => {
    const { treeItemClasses } = await import("@mui/x-tree-view/TreeItem");
    expect(treeItemClasses).toBeDefined();
    expect(typeof treeItemClasses).toBe("object");
    expect(treeItemClasses.groupTransition).toBeDefined();
    expect(treeItemClasses.iconContainer).toBeDefined();
  });

  it("TreeItem 서브 컴포넌트가 모두 export 된다", async () => {
    const m = await import("@mui/x-tree-view/TreeItem");
    for (const k of [
      "TreeItemContent",
      "TreeItemIconContainer",
      "TreeItemLabel",
      "TreeItemRoot",
    ]) {
      expect(m[k], `missing export: ${k}`).toBeDefined();
    }
  });

  it("TreeItemIcon / TreeItemProvider 가 export 된다", async () => {
    const { TreeItemIcon } = await import("@mui/x-tree-view/TreeItemIcon");
    const { TreeItemProvider } =
      await import("@mui/x-tree-view/TreeItemProvider");
    expect(TreeItemIcon).toBeDefined();
    expect(TreeItemProvider).toBeDefined();
  });

  it("useTreeItem 의 useTreeItem 가 함수다", async () => {
    const { useTreeItem } = await import("@mui/x-tree-view/useTreeItem");
    expect(typeof useTreeItem).toBe("function");
  });
});

// ===========================================================================
// 23. prop-types — 다수 컴포넌트
//     PropTypes.{string,number,bool,func,object,array,node,elementType,shape,oneOfType}
//     PropTypes.<type>.isRequired
// ===========================================================================
describe("prop-types", () => {
  const used = [
    "string",
    "number",
    "bool",
    "func",
    "object",
    "array",
    "node",
    "elementType",
  ];

  it("기본 검증자가 모두 함수이고 .isRequired 를 가진다", async () => {
    const PropTypes = (await import("prop-types")).default;
    for (const name of used) {
      expect(typeof PropTypes[name], `not a function: ${name}`).toBe(
        "function",
      );
      expect(
        PropTypes[name].isRequired,
        `${name}.isRequired missing`,
      ).toBeDefined();
    }
  });

  it("복합 검증자 shape / oneOfType 이 함수다", async () => {
    const PropTypes = (await import("prop-types")).default;
    expect(typeof PropTypes.shape).toBe("function");
    expect(typeof PropTypes.oneOfType).toBe("function");
    // 실제 사용 패턴: PropTypes.oneOfType([PropTypes.number, PropTypes.string])
    const validator = PropTypes.oneOfType([PropTypes.number, PropTypes.string]);
    expect(typeof validator).toBe("function");
  });
});

// ===========================================================================
// 24. bootstrap — SearchResult.jsx (CSS import only)
//     `import "bootstrap/dist/css/bootstrap.min.css"`
//     CSS 자체는 jsdom 에서 평가되지 않으므로 패키지 존재만 확인.
// ===========================================================================
describe("bootstrap", () => {
  it("패키지가 import 가능하다", async () => {
    const m = await import("bootstrap");
    expect(m).toBeDefined();
  });
});
