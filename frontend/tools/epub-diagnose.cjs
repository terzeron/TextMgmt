#!/usr/bin/env node
/**
 * EPUB 진단 도구
 *
 * EPUB 파일의 구조를 검사하고 epub.js 로딩 가능 여부를 진단한다.
 * 브라우저 DOMParser(jsdom)와 xmldom 양쪽으로 XML을 파싱하여
 * 브라우저에서만 발생하는 XML 파싱 오류를 사전에 잡아낸다.
 *
 * 사용법:
 *   node frontend/tools/epub-diagnose.cjs <epub_path> [epub_path2 ...]
 *
 * 예시:
 *   node frontend/tools/epub-diagnose.cjs /mnt/data/text/.preview_cache/2063509_ch5.epub
 *   node frontend/tools/epub-diagnose.cjs book1.epub book2.epub
 */

const fs = require("fs");
const path = require("path");
const JSZip = require("jszip");

// ─── 유틸리티 ───

let errorCount = 0;
let warnCount = 0;

function ok(msg) { console.log(`  \x1b[32m✓\x1b[0m ${msg}`); }
function fail(msg) { errorCount++; console.log(`  \x1b[31m✗\x1b[0m ${msg}`); }
function warn(msg) { warnCount++; console.log(`  \x1b[33m!\x1b[0m ${msg}`); }
function info(msg) { console.log(`  \x1b[90m  ${msg}\x1b[0m`); }

const OPF_NS = "http://www.idpf.org/2007/opf";
const DC_NS = "http://purl.org/dc/elements/1.1/";
const NCX_NS = "http://www.daisy.org/z3986/2005/ncx/";

function getXmldomParser() {
    const { DOMParser } = require("@xmldom/xmldom");
    return new DOMParser();
}

function getBrowserParser() {
    try {
        const { JSDOM } = require("jsdom");
        return new (new JSDOM().window.DOMParser)();
    } catch {
        return null;
    }
}

/** 브라우저 DOMParser로 XML을 파싱하고 에러 여부를 반환 */
function browserParseXml(text) {
    const parser = getBrowserParser();
    if (!parser) return { available: false };
    const doc = parser.parseFromString(text, "application/xml");
    const errors = doc.getElementsByTagName("parsererror");
    if (errors.length > 0) {
        return { available: true, ok: false, error: errors[0].textContent.trim() };
    }
    return { available: true, ok: true, doc };
}

/** XML 텍스트에서 선언되지 않은 네임스페이스 프리픽스를 찾는다 */
function findUndeclaredPrefixes(xmlText) {
    // 선언된 프리픽스 수집: xmlns:prefix="..."
    const declared = new Set(["xml", "xmlns"]);
    const declRegex = /xmlns:(\w+)\s*=/g;
    let m;
    while ((m = declRegex.exec(xmlText)) !== null) {
        declared.add(m[1]);
    }
    // 사용된 프리픽스 수집: <prefix:tag 또는 prefix:attr="
    const used = new Set();
    const useRegex = /[< ]\s*(\w+):\w+/g;
    while ((m = useRegex.exec(xmlText)) !== null) {
        if (!declared.has(m[1])) used.add(m[1]);
    }
    return [...used];
}

// ─── 1. ZIP 구조 검사 ───

async function checkZipStructure(zipData, zip) {
    const files = Object.keys(zip.files);
    info(`ZIP 파일 수: ${files.length}`);

    // mimetype 존재 및 내용
    const mimetype = zip.file("mimetype");
    if (!mimetype) {
        fail("mimetype 파일 없음");
        return null;
    }
    const mt = (await mimetype.async("text")).trim();
    if (mt !== "application/epub+zip") {
        fail(`mimetype 내용 불일치: "${mt}"`);
    } else {
        ok("mimetype: application/epub+zip");
    }

    // mimetype이 ZIP의 첫번째 엔트리인지 + 비압축인지 (raw ZIP 헤더 검사)
    // Local file header: PK\x03\x04 ... offset 26: filename length(2) + extra length(2)
    // offset 8: compression method (2 bytes, 0=STORED)
    // offset 30: filename
    if (zipData.length > 38) {
        const sig = zipData.readUInt32LE(0);
        if (sig === 0x04034b50) { // PK\x03\x04
            const compression = zipData.readUInt16LE(8);
            const fnLen = zipData.readUInt16LE(26);
            const fn = zipData.slice(30, 30 + fnLen).toString("ascii");
            if (fn !== "mimetype") {
                fail(`mimetype이 ZIP 첫번째 엔트리가 아님 (첫번째: "${fn}")`);
            } else if (compression !== 0) {
                fail("mimetype이 압축되어 있음 (ZIP_STORED여야 함)");
            } else {
                ok("mimetype: ZIP 첫번째 엔트리, 비압축");
            }
        }
    }

    // container.xml
    const container = zip.file("META-INF/container.xml");
    if (!container) {
        fail("META-INF/container.xml 없음");
        return null;
    }
    ok("META-INF/container.xml 존재");

    return files;
}

// ─── 2. OPF 파싱 검사 ───

async function checkOpf(zip, containerText) {
    const opfMatch = containerText.match(/full-path="([^"]+)"/);
    if (!opfMatch) {
        fail("container.xml에서 OPF 경로를 찾을 수 없음");
        return null;
    }
    const opfPath = opfMatch[1];
    info(`OPF 경로: ${opfPath}`);

    const opfFile = zip.file(opfPath);
    if (!opfFile) {
        fail(`OPF 파일이 ZIP에 없음: ${opfPath}`);
        return null;
    }
    const opfText = await opfFile.async("text");

    // xmldom 파싱 (관대)
    const xmldomErrors = [];
    const { DOMParser } = require("@xmldom/xmldom");
    const xmldomParser = new DOMParser({
        errorHandler: {
            warning: (msg) => xmldomErrors.push({ level: "warn", msg }),
            error: (msg) => xmldomErrors.push({ level: "error", msg }),
            fatalError: (msg) => xmldomErrors.push({ level: "fatal", msg }),
        },
    });
    const xmldomDoc = xmldomParser.parseFromString(opfText, "application/xml");
    if (xmldomErrors.length > 0) {
        warn(`xmldom 파싱 경고/에러 ${xmldomErrors.length}건:`);
        for (const e of xmldomErrors.slice(0, 5)) {
            info(`[${e.level}] ${e.msg.slice(0, 120)}`);
        }
    } else {
        ok("xmldom 파싱: 정상");
    }

    // 브라우저 DOMParser 파싱 (엄격 — 핵심!)
    const browserResult = browserParseXml(opfText);
    if (!browserResult.available) {
        warn("브라우저 DOMParser 테스트 불가 (jsdom 미설치: npm i -D jsdom)");
    } else if (!browserResult.ok) {
        fail("브라우저 DOMParser 파싱 실패 → epub.js 로딩 불가!");
        info(browserResult.error.slice(0, 200));
    } else {
        ok("브라우저 DOMParser 파싱: 정상");
    }

    // 미선언 네임스페이스 프리픽스 (범용)
    const undeclared = findUndeclaredPrefixes(opfText);
    if (undeclared.length > 0) {
        fail(`미선언 네임스페이스 프리픽스: ${undeclared.join(", ")} → 브라우저 파싱 실패`);
    }

    // OPF 메타 정보
    const opfInfo = inspectOpfDoc(xmldomDoc);
    info(`version: ${opfInfo.version}`);
    info(`manifest items: ${opfInfo.manifestCount}`);
    info(`spine itemrefs: ${opfInfo.spineCount}`);
    if (opfInfo.title) info(`title: ${opfInfo.title}`);

    if (opfInfo.manifestCount === 0) fail("manifest에 item이 없음");
    if (opfInfo.spineCount === 0) fail("spine에 itemref가 없음");

    // 필수 메타데이터
    const identifiers = xmldomDoc.getElementsByTagNameNS(DC_NS, "identifier");
    const languages = xmldomDoc.getElementsByTagNameNS(DC_NS, "language");
    if (identifiers.length === 0) warn("dc:identifier 없음");
    if (opfInfo.title === null) warn("dc:title 없음");
    if (languages.length === 0) warn("dc:language 없음");

    // EPUB3 + nav
    if (opfInfo.version && opfInfo.version.startsWith("3")) {
        if (opfInfo.hasNav) {
            ok('EPUB3: nav 문서 있음 (properties="nav")');
        } else {
            warn("EPUB3이지만 nav 문서 없음 (NCX 폴백 사용)");
        }
    }

    return { opfPath, opfText, opfInfo, xmldomDoc };
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

function checkSpineFiles(zip, xmldomDoc, opfPath) {
    const opfDir = path.posix.dirname(opfPath);

    const manifest = {};
    const items = xmldomDoc.getElementsByTagNameNS(OPF_NS, "item");
    for (let i = 0; i < items.length; i++) {
        manifest[items[i].getAttribute("id")] = items[i].getAttribute("href");
    }

    const itemrefs = xmldomDoc.getElementsByTagNameNS(OPF_NS, "itemref");
    let missing = 0;
    for (let i = 0; i < itemrefs.length; i++) {
        const idref = itemrefs[i].getAttribute("idref");
        const href = manifest[idref];
        if (!href) {
            fail(`spine idref="${idref}" → manifest에 없음`);
            missing++;
            continue;
        }
        const zp = opfDir ? path.posix.join(opfDir, href) : href;
        if (!zip.file(zp)) {
            fail(`spine "${idref}" → ${zp} ZIP에 없음`);
            missing++;
        }
    }
    if (missing === 0) {
        ok(`spine ${itemrefs.length}개 항목 모두 ZIP에 존재`);
    }
}

// ─── 4. NCX 검사 ───

async function checkNcx(zip, opfText, opfPath) {
    const opfDir = path.posix.dirname(opfPath);

    const tocMatch = opfText.match(/<spine[^>]*toc="([^"]+)"/);
    if (!tocMatch) {
        info("spine에 toc 속성 없음 (NCX 미참조)");
        return;
    }
    const tocId = tocMatch[1];

    const itemRegex = new RegExp(`<item[^>]*id="${tocId}"[^>]*href="([^"]+)"`, "i");
    const itemMatch = opfText.match(itemRegex);
    if (!itemMatch) {
        fail(`toc="${tocId}" 참조하지만 manifest에 해당 item 없음`);
        return;
    }
    const ncxHref = itemMatch[1];
    const ncxZp = opfDir ? path.posix.join(opfDir, ncxHref) : ncxHref;

    const ncxFile = zip.file(ncxZp);
    if (!ncxFile) {
        fail(`NCX 파일이 ZIP에 없음: ${ncxZp}`);
        return;
    }

    const ncxText = await ncxFile.async("text");

    // NCX 브라우저 파싱 검사
    const browserResult = browserParseXml(ncxText);
    if (browserResult.available && !browserResult.ok) {
        fail("NCX 브라우저 DOMParser 파싱 실패");
        info(browserResult.error.slice(0, 200));
    }

    const ncxDoc = getXmldomParser().parseFromString(ncxText, "application/xml");
    const navPoints = ncxDoc.getElementsByTagNameNS(NCX_NS, "navPoint");
    info(`NCX navPoint 수: ${navPoints.length}`);

    let missing = 0;
    for (let i = 0; i < navPoints.length; i++) {
        const contentEls = navPoints[i].getElementsByTagNameNS(NCX_NS, "content");
        if (contentEls.length === 0) continue;
        const src = contentEls[0].getAttribute("src") || "";
        const srcFile = src.split("#")[0];
        if (!srcFile) continue;
        const srcZp = opfDir
            ? path.posix.normalize(path.posix.join(opfDir, srcFile))
            : srcFile;
        if (!zip.file(srcZp)) {
            missing++;
            if (missing <= 3) info(`navPoint 참조 파일 없음: ${srcZp}`);
        }
    }
    if (missing > 3) info(`... 외 ${missing - 3}건`);
    if (missing > 0) {
        fail(`NCX navPoint 중 ${missing}건이 존재하지 않는 파일 참조 → epub.js hang 가능`);
    } else {
        ok("NCX navPoint 참조 파일 모두 존재");
    }
}

// ─── 5. Guide 참조 검사 ───

function checkGuide(zip, opfText, opfPath) {
    const opfDir = path.posix.dirname(opfPath);
    const guideMatch = opfText.match(/<guide>([\s\S]*?)<\/guide>/);
    if (!guideMatch) {
        info("guide 섹션 없음");
        return;
    }
    const refRegex = /href="([^"]+)"/g;
    let m;
    let missing = 0;
    while ((m = refRegex.exec(guideMatch[1])) !== null) {
        const href = m[1].split("#")[0];
        const zp = opfDir ? path.posix.join(opfDir, href) : href;
        if (!zip.file(zp)) {
            missing++;
            if (missing <= 3) info(`guide 참조 파일 없음: ${zp}`);
        }
    }
    if (missing > 3) info(`... 외 ${missing - 3}건`);
    if (missing > 0) {
        warn(`guide 참조 중 ${missing}건이 존재하지 않는 파일 참조`);
    } else {
        ok("guide 참조 파일 모두 존재");
    }
}

// ─── 6. 콘텐츠 문서 XML 유효성 검사 ───

async function checkContentDocuments(zip, xmldomDoc, opfPath) {
    const opfDir = path.posix.dirname(opfPath);

    // spine에 포함된 XHTML 파일들의 XML 유효성을 브라우저 DOMParser로 검사
    const manifest = {};
    const items = xmldomDoc.getElementsByTagNameNS(OPF_NS, "item");
    for (let i = 0; i < items.length; i++) {
        manifest[items[i].getAttribute("id")] = {
            href: items[i].getAttribute("href"),
            mediaType: items[i].getAttribute("media-type"),
        };
    }

    const itemrefs = xmldomDoc.getElementsByTagNameNS(OPF_NS, "itemref");
    let checked = 0;
    let invalid = 0;
    for (let i = 0; i < itemrefs.length; i++) {
        const idref = itemrefs[i].getAttribute("idref");
        const item = manifest[idref];
        if (!item) continue;
        const zp = opfDir ? path.posix.join(opfDir, item.href) : item.href;
        const file = zip.file(zp);
        if (!file) continue;

        checked++;
        const text = await file.async("text");
        const result = browserParseXml(text);
        if (result.available && !result.ok) {
            invalid++;
            if (invalid <= 3) {
                fail(`XHTML 파싱 실패: ${item.href}`);
                info(result.error.slice(0, 150));
            }
        }
    }
    if (invalid > 3) info(`... 외 ${invalid - 3}건`);
    if (invalid === 0) {
        ok(`XHTML ${checked}개 파일 브라우저 DOMParser 파싱 모두 정상`);
    }
}

// ─── 메인 ───

async function diagnose(epubPath) {
    errorCount = 0;
    warnCount = 0;

    console.log(`\n${"=".repeat(60)}`);
    console.log(`EPUB 진단: ${epubPath}`);
    console.log(`${"=".repeat(60)}`);

    if (!fs.existsSync(epubPath)) {
        fail(`파일 없음: ${epubPath}`);
        return;
    }

    const data = fs.readFileSync(epubPath);
    info(`파일 크기: ${(data.length / 1024).toFixed(1)} KB`);

    let zip;
    try {
        zip = await JSZip.loadAsync(data);
    } catch (e) {
        fail(`ZIP 열기 실패: ${e.message}`);
        return;
    }

    // 1. ZIP 구조
    console.log("\n[ZIP 구조]");
    const files = await checkZipStructure(data, zip);
    if (!files) return;

    // 2. OPF 파싱
    console.log("\n[OPF 파싱]");
    const containerText = await zip.file("META-INF/container.xml").async("text");
    const opfResult = await checkOpf(zip, containerText);
    if (!opfResult) return;

    // 3. Spine 파일
    console.log("\n[Spine 파일]");
    checkSpineFiles(zip, opfResult.xmldomDoc, opfResult.opfPath);

    // 4. NCX
    console.log("\n[NCX 검사]");
    await checkNcx(zip, opfResult.opfText, opfResult.opfPath);

    // 5. Guide
    console.log("\n[Guide 검사]");
    checkGuide(zip, opfResult.opfText, opfResult.opfPath);

    // 6. 콘텐츠 문서
    console.log("\n[콘텐츠 문서 (XHTML)]");
    if (getBrowserParser()) {
        await checkContentDocuments(zip, opfResult.xmldomDoc, opfResult.opfPath);
    } else {
        warn("jsdom 미설치 → XHTML 파싱 검사 스킵 (npm i -D jsdom)");
    }

    // 요약
    console.log(`\n${"─".repeat(40)}`);
    if (errorCount === 0 && warnCount === 0) {
        console.log(`  \x1b[32m결과: 문제 없음\x1b[0m`);
    } else {
        const parts = [];
        if (errorCount > 0) parts.push(`\x1b[31m${errorCount}건 에러\x1b[0m`);
        if (warnCount > 0) parts.push(`\x1b[33m${warnCount}건 경고\x1b[0m`);
        console.log(`  결과: ${parts.join(" / ")}`);
    }
    console.log("");
}

async function main() {
    const args = process.argv.slice(2);
    if (args.length === 0) {
        console.log("EPUB 진단 도구 — epub.js 렌더링 가능 여부를 사전 검증\n");
        console.log("사용법: node epub-diagnose.cjs <epub_path> [epub_path2 ...]");
        console.log("예시:   node epub-diagnose.cjs /mnt/data/text/.preview_cache/2063509_ch5.epub");
        process.exit(1);
    }

    for (const epubPath of args) {
        await diagnose(path.resolve(epubPath));
    }
}

main().catch((e) => {
    console.error("Fatal:", e.message);
    process.exit(1);
});
