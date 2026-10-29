//! 뷰어가 지표를 막는지 정하는 정적 표. 포맷과 코드로 찾는다.
//!
//! - `Blocked`: 제품 뷰어가 이미 위험을 무력화한다(근거 줄을 주석으로 적는다).
//! - `NotBlocked`: 뷰어가 그대로 렌더링·실행하거나 막는 근거가 소스에 없다.
//!   근거가 모호하면 위험이 보이는 쪽인 `NotBlocked`로 둔다.
//! - `NotApplicable`: 뷰어 렌더링과 무관하다(결함·구조, 압축 해제·파일명, 실행 파일 내용).
//!   표에 없는 코드도 `NotApplicable`이다.
//!
//! 표가 낡지 않도록 `tests/rust/bookdoctor/viewer.rs`가 스캐너 소스의 코드 집합과 대조한다.

use scan_common::{FileReport, Format, ViewerBlocked};
use ViewerBlocked::{Blocked, NotApplicable, NotBlocked};

/// EPUB 보안 코드. 근거는 `frontend/src/ViewEPUB.jsx`다.
/// 본문은 `secureEpubDocument`(58~76행)에서 DOMPurify로 정제하고, CSP
/// `script-src 'none'; object-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'`(74행)를 넣는다.
/// `allowScriptedContent: true`(505행)라서 iframe sandbox는 스크립트를 막지 않는다. 방어는 정제와 CSP뿐이다.
/// CSP에 `default-src`·`img-src`·`style-src`·`connect-src`가 없으므로 원격 로드와 스타일은 막지 않는다.
pub const EPUB_CODES: &[(&str, ViewerBlocked)] = &[
    // --- 스크립트 실행: CSP script-src 'none'(74행)이 인라인 핸들러·javascript: URL·외부 스크립트·eval을 모두 막고,
    //     DOMPurify 기본 설정(63~69행)이 script 요소·on* 속성·javascript: URL을 먼저 걷어낸다.
    ("SCRIPT_TAG", Blocked),
    ("EVENT_HANDLER_ATTR", Blocked),
    ("JS_URL", Blocked),
    ("REMOTE_SCRIPT", Blocked),
    ("SCRIPT_MEDIA_TYPE", Blocked),
    // 스크립트 멤버는 script 요소로만 로드되고 그 요소는 위에서 막힌다.
    ("SCRIPT_FILE", Blocked),
    // 아래 JS 내용 코드는 스크립트가 실행되지 않으므로 효과가 없다(script-src 'none').
    ("DYNAMIC_CODE_EXEC", Blocked),
    ("DOCUMENT_WRITE", Blocked),
    ("NETWORK_API", Blocked),
    ("STORAGE_API", Blocked),
    ("HOST_API", Blocked),
    ("HOST_COMMAND_EXEC", Blocked),
    ("OBFUSCATED_LOADER", Blocked),
    ("OBFUSCATION", Blocked),
    ("DYNAMIC_IMPORT", Blocked),
    ("NAVIGATION", Blocked),
    // --- 요소: FORBID_TAGS iframe·object·embed·form(68행), CSP object-src·frame-src·form-action 'none'(74행).
    //     applet은 object-src, frame·frameset은 frame-src 범위다.
    ("ACTIVE_ELEMENT", Blocked),
    // form 요소는 제거되고 form-action 'none'이라 제출할 수 없다. input 등은 남지만 전송 경로가 없다.
    ("FORM_ELEMENT", Blocked),
    ("REMOTE_FORM_ACTION", Blocked),
    // 62행이 `meta[http-equiv]`를 정제 전에 모두 지운다.
    ("META_REFRESH", Blocked),
    // --- 막지 않는 항목.
    // ADD_TAGS에 base·link·meta가 있어(66행) base를 일부러 살려 둔다.
    ("BASE_HREF", NotBlocked),
    // 뷰어 소스에 foreignObject 처리가 없다. DOMPurify 기본값에 기대는 것은 근거로 삼지 않는다.
    ("SVG_FOREIGN_OBJECT", NotBlocked),
    ("SVG_ANIMATE_HREF", NotBlocked),
    // XML 파싱 단계의 위험이다. 뷰어는 epub.js가 파싱한 뒤 정제하므로 정제 전 단계를 막는 코드가 없다.
    ("XINCLUDE", NotBlocked),
    ("XXE_ENTITY", NotBlocked),
    ("ENTITY_DECLARATION", NotBlocked),
    ("EXTERNAL_DTD", NotBlocked),
    ("XML_STYLESHEET", NotBlocked),
    // CSP에 img-src·style-src·connect-src·default-src가 없어 원격 로드를 막지 않는다.
    ("REMOTE_RESOURCE", NotBlocked),
    ("REMOTE_CSS_IMPORT", NotBlocked),
    ("REMOTE_MANIFEST_ITEM", NotBlocked),
    // CSS는 정제하지 않고(DOMPurify 설정에 style 금지 없음), 뷰어 소스에 CSS 필터가 없다. 현대 브라우저는 실행하지 않지만 근거가 소스에 없다.
    ("CSS_EXPRESSION", NotBlocked),
    ("CSS_MOZ_BINDING", NotBlocked),
    ("CSS_BEHAVIOR", NotBlocked),
    // URL 스킴 필터는 뷰어 소스에 없다(DOMPurify 기본 허용 목록에 기대는 것은 근거로 삼지 않는다).
    ("DANGEROUS_URI_SCHEME", NotBlocked),
    ("UNKNOWN_URI_SCHEME", NotBlocked),
    ("UNC_PATH", NotBlocked),
    // 뷰어는 ZIP을 브라우저 메모리에서 풀며 압축비 상한을 두지 않는다(ViewEPUB.jsx에 검사 없음).
    ("HIGH_COMPRESSION_RATIO", NotBlocked),
    // --- 해당 없음: 렌더링 구조 문제(스캐너가 Render 분류로 보고한다).
    ("MISSING_MIMETYPE", NotApplicable),
    ("NONSTANDARD_MIMETYPE", NotApplicable),
    ("MISSING_CONTAINER_XML", NotApplicable),
    ("BAD_MIMETYPE", NotApplicable),
    // --- 해당 없음: 압축 파일 구조·파일명. 추출 단계의 위험이고 뷰어는 ZIP을 메모리에서만 읽는다.
    ("ZIP_PATH_TRAVERSAL", NotApplicable),
    ("ZIP_ABSOLUTE_PATH", NotApplicable),
    ("ZIP_BACKSLASH_PATH", NotApplicable),
    ("ZIP_CONTROL_CHAR_NAME", NotApplicable),
    ("ZIP_ALT_DATA_STREAM", NotApplicable),
    ("ZIP_NAME_TOO_LONG", NotApplicable),
    ("ZIP_SYMLINK", NotApplicable),
    ("ZIP_HEADER_NAME_MISMATCH", NotApplicable),
    ("BAD_LOCAL_HEADER", NotApplicable),
    ("TRUNCATED_LOCAL_HEADER", NotApplicable),
    ("WINDOWS_RESERVED_NAME", NotApplicable),
    ("WINDOWS_FILENAME_COLLISION", NotApplicable),
    ("MANIFEST_PATH_ESCAPE", NotApplicable),
    ("NESTED_ARCHIVE", NotApplicable),
    // --- 해당 없음: 실행 파일·스크립트 파일 내용. 뷰어는 멤버를 실행하지 않는다.
    ("PE_EXECUTABLE_CONTENT", NotApplicable),
    ("ELF_BINARY_CONTENT", NotApplicable),
    ("MACHO_BINARY_CONTENT", NotApplicable),
    ("JAVA_CLASS_CONTENT", NotApplicable),
    ("SCRIPT_SHEBANG", NotApplicable),
    ("EMBEDDED_EXECUTABLE", NotApplicable),
    // --- 해당 없음: 스캐너 검사 범위·메타데이터 불일치. 뷰어 렌더링 경로가 아니다.
    ("MALFORMED_MARKUP", NotApplicable),
    ("MARKUP_UNDECODABLE", NotApplicable),
    ("MEDIA_TYPE_MISMATCH", NotApplicable),
    ("UNDECLARED_SCRIPTED_CONTENT", NotApplicable),
    ("UNDECLARED_REMOTE_RESOURCES", NotApplicable),
];

/// PDF 보안 지표 종류. 근거는 `frontend/src/pdfjsEnv.js`의 `PDF_DOCUMENT_OPTIONS`다.
/// 옵션은 `cMapUrl`·`cMapPacked`·`standardFontDataUrl`·`wasmUrl`뿐이고 `enableScripting`·`isEvalSupported`·`enableXfa`는
/// 어느 `getDocument(` 호출에도 없다(ViewPDF.jsx 260·559행, DirectoryPagePreview.jsx 63행, PdfDiagnose.js 16행).
/// 막는다는 근거가 소스에 없으므로 모두 `NotBlocked`다.
pub const PDF_CODES: &[(&str, ViewerBlocked)] = &[
    ("JavaScript action", NotBlocked),
    ("JavaScript code", NotBlocked),
    ("JavaScript name tree", NotBlocked),
    ("JavaScript stream not fully inspected", NotBlocked),
    ("exploit JavaScript", NotBlocked),
    ("Launch action", NotBlocked),
    ("platform launch parameters", NotBlocked),
    ("RichMediaExecute action", NotBlocked),
    ("RichMedia content", NotBlocked),
    ("XFA form", NotBlocked),
    ("3D content", NotBlocked),
    ("screen annotation", NotBlocked),
    ("OpenAction (runs on open)", NotBlocked),
    ("additional-actions trigger", NotBlocked),
    ("external/media action", NotBlocked),
    ("GoToR action", NotBlocked),
    ("GoToE action", NotBlocked),
    ("SubmitForm action", NotBlocked),
    ("ImportData action", NotBlocked),
    ("URI action", NotBlocked),
    ("embedded file", NotBlocked),
    ("file attachment annotation", NotBlocked),
    ("executable-looking file name", NotBlocked),
    ("embedded executable content", NotBlocked),
    ("hex-obfuscated action name", NotBlocked),
    ("unresolvable action type", NotBlocked),
    ("nesting limit exceeded", NotBlocked),
    ("long filter chain", NotBlocked),
    ("many incremental updates", NotBlocked),
];

/// TXT 보안 코드. 근거는 `frontend/src/ViewTXT.jsx`(59·61·63행)다. 본문을 `<p>{block.text}</p>`로만 그린다.
pub const TXT_CODES: &[(&str, ViewerBlocked)] = &[
    // React가 텍스트 자식을 이스케이프하므로 HTML은 마크업이 되지 않는다.
    ("HTML_IN_TEXT", Blocked),
    // 이스케이프는 표시 순서 조작(U+202E 등)과 보이지 않는 문자를 막지 못한다. 화면에 그대로 반영된다.
    ("BIDI_CONTROL", NotBlocked),
    ("ZERO_WIDTH", NotBlocked),
];

/// 포맷과 코드로 뷰어 차단 여부를 찾는다. 표에 없으면 `NotApplicable`이다.
pub fn viewer_blocked(format: Format, code: &str) -> ViewerBlocked {
    let table = match format {
        Format::Epub => EPUB_CODES,
        Format::Pdf => PDF_CODES,
        Format::Txt => TXT_CODES,
    };
    table.iter().find(|(c, _)| *c == code).map_or(NotApplicable, |(_, v)| *v)
}

/// 보고서의 모든 지표에 `viewer_blocked`를 채운다. 포맷을 모르면 모든 지표가 `NotApplicable`로 남는다.
pub fn annotate(report: &mut FileReport) {
    let Some(format) = report.format else { return };
    for f in &mut report.findings {
        f.viewer_blocked = viewer_blocked(format, &f.code);
    }
}
