/**
 * EPUB 브라우저 진단 모듈
 *
 * epub-diagnose.cjs (CLI)를 브라우저용으로 포팅.
 * 브라우저 네이티브 DOMParser를 사용하므로 epub.js와 동일한 파싱 환경에서 검증한다.
 */
import JSZip from "jszip";

const OPF_NS = "http://www.idpf.org/2007/opf";
const DC_NS = "http://purl.org/dc/elements/1.1/";
const NCX_NS = "http://www.daisy.org/z3986/2005/ncx/";

function browserParseXml(text) {
    const parser = new DOMParser();
    const doc = parser.parseFromString(text, "application/xml");
    const errors = doc.getElementsByTagName("parsererror");
    if (errors.length > 0) {
        return { ok: false, error: errors[0].textContent.trim() };
    }
    return { ok: true, doc };
}

function findUndeclaredPrefixes(xmlText) {
    const declared = new Set(["xml", "xmlns"]);
    const declRegex = /xmlns:(\w+)\s*=/g;
    let m;
    while ((m = declRegex.exec(xmlText)) !== null) {
        declared.add(m[1]);
    }
    const used = new Set();
    const useRegex = /[< ]\s*(\w+):\w+/g;
    while ((m = useRegex.exec(xmlText)) !== null) {
        if (!declared.has(m[1])) used.add(m[1]);
    }
    return [...used];
}

function posixNormalize(p) {
    const parts = p.split("/");
    const result = [];
    for (const part of parts) {
        if (part === "..") result.pop();
        else if (part !== "." && part !== "") result.push(part);
    }
    return result.join("/");
}

function posixJoin(dir, file) {
    if (!dir || dir === ".") return file;
    return posixNormalize(dir + "/" + file);
}

function posixDirname(p) {
    const idx = p.lastIndexOf("/");
    return idx < 0 ? "" : p.substring(0, idx);
}

// ─── 1. ZIP 구조 검사 ───

async function checkZipStructure(zip) {
    const results = [];
    const files = Object.keys(zip.files);
    results.push({ type: "info", text: `ZIP 파일 수: ${files.length}` });

    const mimetype = zip.file("mimetype");
    if (!mimetype) {
        results.push({ type: "error", severity: "FATAL", text: "mimetype 파일 없음" });
        return { results, ok: false };
    }
    const mt = (await mimetype.async("text")).trim();
    if (mt !== "application/epub+zip") {
        results.push({ type: "error", severity: "ERROR", text: `mimetype 내용 불일치: "${mt}"` });
    } else {
        results.push({ type: "ok", text: "mimetype: application/epub+zip" });
    }

    const container = zip.file("META-INF/container.xml");
    if (!container) {
        results.push({ type: "error", severity: "FATAL", text: "META-INF/container.xml 없음" });
        return { results, ok: false };
    }
    results.push({ type: "ok", text: "META-INF/container.xml 존재" });

    return { results, ok: true };
}

// ─── 2. OPF 파싱 검사 ───

async function checkOpf(zip, containerText) {
    const results = [];

    const opfMatch = containerText.match(/full-path="([^"]+)"/);
    if (!opfMatch) {
        results.push({ type: "error", severity: "FATAL", text: "container.xml에서 OPF 경로를 찾을 수 없음" });
        return { results, opfResult: null };
    }
    const opfPath = opfMatch[1];
    results.push({ type: "info", text: `OPF 경로: ${opfPath}` });

    const opfFile = zip.file(opfPath);
    if (!opfFile) {
        results.push({ type: "error", severity: "FATAL", text: `OPF 파일이 ZIP에 없음: ${opfPath}` });
        return { results, opfResult: null };
    }
    const opfText = await opfFile.async("text");

    // 브라우저 DOMParser 파싱 (핵심 — epub.js와 동일 환경)
    const parseResult = browserParseXml(opfText);
    if (!parseResult.ok) {
        results.push({ type: "error", severity: "FATAL", text: "브라우저 DOMParser 파싱 실패 → epub.js 로딩 불가!" });
        results.push({ type: "info", text: parseResult.error.slice(0, 200) });
    } else {
        results.push({ type: "ok", text: "브라우저 DOMParser 파싱: 정상" });
    }

    // 미선언 네임스페이스 프리픽스
    const undeclared = findUndeclaredPrefixes(opfText);
    if (undeclared.length > 0) {
        results.push({ type: "error", severity: "ERROR", text: `미선언 네임스페이스 프리픽스: ${undeclared.join(", ")}` });
    }

    // OPF 메타 정보
    const doc = parseResult.ok ? parseResult.doc : null;
    let opfInfo = null;
    if (doc) {
        opfInfo = inspectOpfDoc(doc);
        results.push({ type: "info", text: `version: ${opfInfo.version}` });
        results.push({ type: "info", text: `manifest items: ${opfInfo.manifestCount}` });
        results.push({ type: "info", text: `spine itemrefs: ${opfInfo.spineCount}` });
        if (opfInfo.title) results.push({ type: "info", text: `title: ${opfInfo.title}` });

        if (opfInfo.manifestCount === 0) results.push({ type: "error", severity: "FATAL", text: "manifest에 item이 없음" });
        if (opfInfo.spineCount === 0) results.push({ type: "error", severity: "FATAL", text: "spine에 itemref가 없음" });

        // 필수 메타데이터
        const identifiers = doc.getElementsByTagNameNS(DC_NS, "identifier");
        const languages = doc.getElementsByTagNameNS(DC_NS, "language");
        if (identifiers.length === 0) results.push({ type: "warn", severity: "WARNING", text: "dc:identifier 없음" });
        if (!opfInfo.title) results.push({ type: "warn", severity: "WARNING", text: "dc:title 없음" });
        if (languages.length === 0) results.push({ type: "warn", severity: "WARNING", text: "dc:language 없음" });

        // EPUB3 + nav
        if (opfInfo.version && opfInfo.version.startsWith("3")) {
            if (opfInfo.hasNav) {
                results.push({ type: "ok", text: "EPUB3: nav 문서 있음" });
            } else {
                results.push({ type: "warn", severity: "WARNING", text: "EPUB3이지만 nav 문서 없음 (NCX 폴백 사용)" });
            }
        }
    }

    return { results, opfResult: doc ? { opfPath, opfText, opfInfo, doc } : null };
}

function inspectOpfDoc(doc) {
    const items = doc.getElementsByTagNameNS(OPF_NS, "item");
    const itemrefs = doc.getElementsByTagNameNS(OPF_NS, "itemref");
    const titles = doc.getElementsByTagNameNS(DC_NS, "title");
    const pkg = doc.getElementsByTagNameNS(OPF_NS, "package");
    const version = pkg.length > 0 ? pkg[0].getAttribute("version") : "?";

    let hasNav = false;
    for (let i = 0; i < items.length; i++) {
        const props = items[i].getAttribute("properties") || "";
        if (props.includes("nav")) { hasNav = true; break; }
    }

    return {
        version,
        manifestCount: items.length,
        spineCount: itemrefs.length,
        title: titles.length > 0 ? titles[0].textContent : null,
        hasNav,
    };
}

// ─── 3. Spine 파일 존재 검사 ───

function checkSpineFiles(zip, doc, opfPath) {
    const results = [];
    const opfDir = posixDirname(opfPath);

    const manifest = {};
    const items = doc.getElementsByTagNameNS(OPF_NS, "item");
    for (let i = 0; i < items.length; i++) {
        manifest[items[i].getAttribute("id")] = items[i].getAttribute("href");
    }

    const itemrefs = doc.getElementsByTagNameNS(OPF_NS, "itemref");
    let missing = 0;
    for (let i = 0; i < itemrefs.length; i++) {
        const idref = itemrefs[i].getAttribute("idref");
        const href = manifest[idref];
        if (!href) {
            results.push({ type: "error", severity: "ERROR", text: `spine idref="${idref}" → manifest에 없음` });
            missing++;
            continue;
        }
        const zp = posixJoin(opfDir, href);
        if (!zip.file(zp)) {
            results.push({ type: "error", severity: "ERROR", text: `spine "${idref}" → ${zp} ZIP에 없음` });
            missing++;
        }
    }
    if (missing === 0) {
        results.push({ type: "ok", text: `spine ${itemrefs.length}개 항목 모두 ZIP에 존재` });
    }
    return results;
}

// ─── 4. NCX 검사 ───

async function checkNcx(zip, doc, opfText, opfPath) {
    const results = [];
    const opfDir = posixDirname(opfPath);

    // spine의 toc 속성에서 NCX id를 찾는다
    const spineEls = doc.getElementsByTagNameNS(OPF_NS, "spine");
    const tocId = spineEls.length > 0 ? spineEls[0].getAttribute("toc") : null;
    if (!tocId) {
        results.push({ type: "info", text: "spine에 toc 속성 없음 (NCX 미참조)" });
        return results;
    }

    // DOM 기반으로 manifest에서 해당 item을 찾는다 (속성 순서 무관)
    const items = doc.getElementsByTagNameNS(OPF_NS, "item");
    let ncxHref = null;
    for (let i = 0; i < items.length; i++) {
        if (items[i].getAttribute("id") === tocId) {
            ncxHref = items[i].getAttribute("href");
            break;
        }
    }
    if (!ncxHref) {
        results.push({ type: "error", severity: "ERROR", text: `toc="${tocId}" 참조하지만 manifest에 해당 item 없음` });
        return results;
    }
    const ncxZp = posixJoin(opfDir, ncxHref);

    const ncxFile = zip.file(ncxZp);
    if (!ncxFile) {
        results.push({ type: "error", severity: "ERROR", text: `NCX 파일이 ZIP에 없음: ${ncxZp}` });
        return results;
    }

    const ncxText = await ncxFile.async("text");

    // NCX 브라우저 파싱 검사
    const parseResult = browserParseXml(ncxText);
    if (!parseResult.ok) {
        results.push({ type: "error", severity: "ERROR", text: "NCX 브라우저 DOMParser 파싱 실패" });
        results.push({ type: "info", text: parseResult.error.slice(0, 200) });
    }

    const ncxDoc = parseResult.ok ? parseResult.doc : null;
    if (ncxDoc) {
        const navPoints = ncxDoc.getElementsByTagNameNS(NCX_NS, "navPoint");
        results.push({ type: "info", text: `NCX navPoint 수: ${navPoints.length}` });

        let missing = 0;
        const missingFiles = [];
        for (let i = 0; i < navPoints.length; i++) {
            const contentEls = navPoints[i].getElementsByTagNameNS(NCX_NS, "content");
            if (contentEls.length === 0) continue;
            const src = contentEls[0].getAttribute("src") || "";
            const srcFile = src.split("#")[0];
            if (!srcFile) continue;
            const srcZp = posixJoin(opfDir, srcFile);
            if (!zip.file(srcZp)) {
                missing++;
                if (missing <= 3) missingFiles.push(srcZp);
            }
        }
        if (missing > 0) {
            for (const f of missingFiles) {
                results.push({ type: "info", text: `navPoint 참조 파일 없음: ${f}` });
            }
            if (missing > 3) results.push({ type: "info", text: `... 외 ${missing - 3}건` });
            results.push({ type: "error", severity: "ERROR", text: `NCX navPoint 중 ${missing}건이 존재하지 않는 파일 참조` });
        } else {
            results.push({ type: "ok", text: "NCX navPoint 참조 파일 모두 존재" });
        }
    }
    return results;
}

// ─── 5. Guide 참조 검사 ───

function checkGuide(zip, opfText, opfPath) {
    const results = [];
    const opfDir = posixDirname(opfPath);

    const guideMatch = opfText.match(/<guide>([\s\S]*?)<\/guide>/);
    if (!guideMatch) {
        results.push({ type: "info", text: "guide 섹션 없음" });
        return results;
    }
    const refRegex = /href="([^"]+)"/g;
    let m;
    let missing = 0;
    const missingFiles = [];
    while ((m = refRegex.exec(guideMatch[1])) !== null) {
        const href = m[1].split("#")[0];
        const zp = posixJoin(opfDir, href);
        if (!zip.file(zp)) {
            missing++;
            if (missing <= 3) missingFiles.push(zp);
        }
    }
    if (missing > 0) {
        for (const f of missingFiles) {
            results.push({ type: "info", text: `guide 참조 파일 없음: ${f}` });
        }
        if (missing > 3) results.push({ type: "info", text: `... 외 ${missing - 3}건` });
        results.push({ type: "warn", severity: "WARNING", text: `guide 참조 중 ${missing}건이 존재하지 않는 파일 참조` });
    } else {
        results.push({ type: "ok", text: "guide 참조 파일 모두 존재" });
    }
    return results;
}

// ─── 6. 콘텐츠 문서 XML 유효성 검사 ───

async function checkContentDocuments(zip, doc, opfPath) {
    const results = [];
    const opfDir = posixDirname(opfPath);

    const manifest = {};
    const items = doc.getElementsByTagNameNS(OPF_NS, "item");
    for (let i = 0; i < items.length; i++) {
        manifest[items[i].getAttribute("id")] = {
            href: items[i].getAttribute("href"),
            mediaType: items[i].getAttribute("media-type"),
        };
    }

    const itemrefs = doc.getElementsByTagNameNS(OPF_NS, "itemref");
    let checked = 0;
    let invalid = 0;
    const invalidFiles = [];
    for (let i = 0; i < itemrefs.length; i++) {
        const idref = itemrefs[i].getAttribute("idref");
        const item = manifest[idref];
        if (!item) continue;
        const zp = posixJoin(opfDir, item.href);
        const file = zip.file(zp);
        if (!file) continue;

        checked++;
        const text = await file.async("text");
        const result = browserParseXml(text);
        if (!result.ok) {
            invalid++;
            if (invalid <= 3) {
                invalidFiles.push({ href: item.href, error: result.error.slice(0, 150) });
            }
        }
    }
    if (invalid > 0) {
        for (const f of invalidFiles) {
            results.push({ type: "error", severity: "ERROR", text: `XHTML 파싱 실패: ${f.href}` });
            results.push({ type: "info", text: f.error });
        }
        if (invalid > 3) results.push({ type: "info", text: `... 외 ${invalid - 3}건` });
    } else {
        results.push({ type: "ok", text: `XHTML ${checked}개 파일 브라우저 DOMParser 파싱 모두 정상` });
    }
    return results;
}

// ─── 메인 진단 함수 ───

/**
 * ArrayBuffer를 받아 EPUB 진단을 수행하고 구조화된 결과를 반환한다.
 * @param {ArrayBuffer} arrayBuffer - EPUB 파일 데이터
 * @returns {Promise<{sections: Array<{name: string, results: Array}>, summary: {fatal: number, errors: number, warnings: number}}>}
 */
export async function diagnoseEpub(arrayBuffer) {
    const sections = [];
    let fatal = 0;
    let errors = 0;
    let warnings = 0;

    // fatal과 errors는 겹치지 않게 분리 집계 (백엔드 epubcheck summary와 동일 패턴)
    const countResults = (results) => {
        for (const r of results) {
            if (r.severity === "FATAL") fatal++;
            else if (r.type === "error") errors++;
            if (r.type === "warn") warnings++;
        }
    };

    let zip;
    try {
        zip = await JSZip.loadAsync(arrayBuffer);
    } catch (e) {
        return {
            sections: [{ name: "ZIP", results: [{ type: "error", severity: "FATAL", text: `ZIP 열기 실패: ${e.message}` }] }],
            summary: { fatal: 1, errors: 0, warnings: 0 },
        };
    }

    // 1. ZIP 구조
    const zipCheck = await checkZipStructure(zip);
    countResults(zipCheck.results);
    sections.push({ name: "ZIP 구조", results: zipCheck.results });
    if (!zipCheck.ok) return { sections, summary: { fatal, errors, warnings } };

    // 2. OPF 파싱
    const containerText = await zip.file("META-INF/container.xml").async("text");
    const { results: opfResults, opfResult } = await checkOpf(zip, containerText);
    countResults(opfResults);
    sections.push({ name: "OPF 파싱", results: opfResults });
    if (!opfResult) return { sections, summary: { fatal, errors, warnings } };

    // 3. Spine 파일
    const spineResults = checkSpineFiles(zip, opfResult.doc, opfResult.opfPath);
    countResults(spineResults);
    sections.push({ name: "Spine 파일", results: spineResults });

    // 4. NCX
    const ncxResults = await checkNcx(zip, opfResult.doc, opfResult.opfText, opfResult.opfPath);
    countResults(ncxResults);
    sections.push({ name: "NCX", results: ncxResults });

    // 5. Guide
    const guideResults = checkGuide(zip, opfResult.opfText, opfResult.opfPath);
    countResults(guideResults);
    sections.push({ name: "Guide", results: guideResults });

    // 6. 콘텐츠 문서
    const contentResults = await checkContentDocuments(zip, opfResult.doc, opfResult.opfPath);
    countResults(contentResults);
    sections.push({ name: "콘텐츠 문서 (XHTML)", results: contentResults });

    return { sections, summary: { fatal, errors, warnings } };
}
