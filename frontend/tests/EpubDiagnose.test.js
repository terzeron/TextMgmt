// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import JSZip from 'jszip';
import { diagnoseEpub } from '../src/EpubDiagnose';

// ─── 헬퍼: 유효한 EPUB ZIP 생성 ───

const CONTAINER_XML = `<?xml version="1.0" encoding="UTF-8"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>`;

function makeOpf({ version = '2.0', items = [], spineRefs = [], tocId = null, hasNav = false, extraAttrs = '' } = {}) {
    const manifestItems = items.map(i =>
        `<item id="${i.id}" href="${i.href}" media-type="${i.mediaType || 'application/xhtml+xml'}"${i.properties ? ` properties="${i.properties}"` : ''}/>`
    ).join('\n      ');

    const spineItemrefs = spineRefs.map(id => `<itemref idref="${id}"/>`).join('\n      ');
    const tocAttr = tocId ? ` toc="${tocId}"` : '';

    return `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="${version}"${extraAttrs}>
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier>test-id</dc:identifier>
    <dc:title>Test Book</dc:title>
    <dc:language>ko</dc:language>
  </metadata>
  <manifest>
      ${manifestItems}
  </manifest>
  <spine${tocAttr}>
      ${spineItemrefs}
  </spine>
</package>`;
}

const VALID_XHTML = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter</title></head>
<body><p>Hello</p></body>
</html>`;

const INVALID_XHTML = `<html><body><p>unclosed tag<br></body></html>`;

function makeNcx(navPoints = []) {
    const nps = navPoints.map((np, i) =>
        `<navPoint id="np${i}"><navLabel><text>${np.label}</text></navLabel><content src="${np.src}"/></navPoint>`
    ).join('\n');
    return `<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="test"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>${nps}</navMap>
</ncx>`;
}

async function buildEpub(setup) {
    const zip = new JSZip();
    zip.file('mimetype', 'application/epub+zip');
    zip.file('META-INF/container.xml', CONTAINER_XML);
    if (setup) await setup(zip);
    return zip.generateAsync({ type: 'arraybuffer' });
}

// ─── 테스트 ───

describe('diagnoseEpub', () => {

    // === ZIP 구조 검사 ===

    describe('ZIP 구조', () => {
        it('잘못된 데이터에서 ZIP 열기 실패 반환', async () => {
            const result = await diagnoseEpub(new ArrayBuffer(4));
            expect(result.summary.fatal).toBe(1);
            expect(result.sections[0].results[0].type).toBe('error');
            expect(result.sections[0].results[0].severity).toBe('FATAL');
            expect(result.sections[0].results[0].text).toMatch(/ZIP 열기 실패/);
        });

        it('mimetype 없는 ZIP에서 에러 반환', async () => {
            const zip = new JSZip();
            zip.file('META-INF/container.xml', CONTAINER_XML);
            const buf = await zip.generateAsync({ type: 'arraybuffer' });

            const result = await diagnoseEpub(buf);
            expect(result.summary.fatal).toBeGreaterThan(0);
            const zipSection = result.sections.find(s => s.name === 'ZIP 구조');
            expect(zipSection.results.some(r => r.type === 'error' && r.severity === 'FATAL' && r.text.includes('mimetype'))).toBe(true);
        });

        it('잘못된 mimetype 내용에서 에러 반환', async () => {
            const zip = new JSZip();
            zip.file('mimetype', 'text/plain');
            zip.file('META-INF/container.xml', CONTAINER_XML);
            const buf = await zip.generateAsync({ type: 'arraybuffer' });

            const result = await diagnoseEpub(buf);
            const zipSection = result.sections.find(s => s.name === 'ZIP 구조');
            expect(zipSection.results.some(r => r.type === 'error' && r.text.includes('불일치'))).toBe(true);
        });

        it('container.xml 없으면 에러 반환 후 조기 종료', async () => {
            const zip = new JSZip();
            zip.file('mimetype', 'application/epub+zip');
            const buf = await zip.generateAsync({ type: 'arraybuffer' });

            const result = await diagnoseEpub(buf);
            const zipSection = result.sections.find(s => s.name === 'ZIP 구조');
            expect(zipSection.results.some(r => r.type === 'error' && r.text.includes('container.xml'))).toBe(true);
            // OPF 파싱 섹션은 없어야 함 (조기 종료)
            expect(result.sections.find(s => s.name === 'OPF 파싱')).toBeUndefined();
        });
    });

    // === OPF 파싱 검사 ===

    describe('OPF 파싱', () => {
        it('유효한 OPF에서 정상 통과', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const opfSection = result.sections.find(s => s.name === 'OPF 파싱');
            expect(opfSection.results.some(r => r.type === 'ok' && r.text.includes('DOMParser'))).toBe(true);
        });

        it('OPF 파일이 ZIP에 없으면 에러', async () => {
            const buf = await buildEpub(() => {
                // OPF 파일을 생성하지 않음
            });

            const result = await diagnoseEpub(buf);
            const opfSection = result.sections.find(s => s.name === 'OPF 파싱');
            expect(opfSection.results.some(r => r.type === 'error' && r.text.includes('ZIP에 없음'))).toBe(true);
        });

        it('미선언 네임스페이스 프리픽스 감지', async () => {
            const badOpf = `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test</dc:title>
    <calibre:series>MyBook</calibre:series>
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>`;

            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', badOpf);
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const opfSection = result.sections.find(s => s.name === 'OPF 파싱');
            expect(opfSection.results.some(r => r.type === 'error' && r.text.includes('calibre'))).toBe(true);
        });

        it('manifest/spine이 비어있으면 에러', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({ items: [], spineRefs: [] }));
            });

            const result = await diagnoseEpub(buf);
            const opfSection = result.sections.find(s => s.name === 'OPF 파싱');
            expect(opfSection.results.some(r => r.type === 'error' && r.text.includes('manifest'))).toBe(true);
            expect(opfSection.results.some(r => r.type === 'error' && r.text.includes('spine'))).toBe(true);
        });

        it('필수 메타데이터 누락 시 경고', async () => {
            const noMetaOpf = `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
  </metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>`;

            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', noMetaOpf);
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const opfSection = result.sections.find(s => s.name === 'OPF 파싱');
            expect(opfSection.results.some(r => r.type === 'warn' && r.text.includes('dc:identifier'))).toBe(true);
            expect(opfSection.results.some(r => r.type === 'warn' && r.text.includes('dc:title'))).toBe(true);
            expect(opfSection.results.some(r => r.type === 'warn' && r.text.includes('dc:language'))).toBe(true);
        });

        it('EPUB3 + nav 문서 감지', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    version: '3.0',
                    items: [
                        { id: 'nav', href: 'nav.xhtml', properties: 'nav' },
                        { id: 'ch1', href: 'ch1.xhtml' },
                    ],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
                zip.file('OEBPS/nav.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const opfSection = result.sections.find(s => s.name === 'OPF 파싱');
            expect(opfSection.results.some(r => r.type === 'ok' && r.text.includes('nav'))).toBe(true);
        });

        it('EPUB3이지만 nav 없으면 경고', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    version: '3.0',
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const opfSection = result.sections.find(s => s.name === 'OPF 파싱');
            expect(opfSection.results.some(r => r.type === 'warn' && r.text.includes('nav 문서 없음'))).toBe(true);
        });
    });

    // === Spine 파일 검사 ===

    describe('Spine 파일', () => {
        it('모든 spine 파일이 ZIP에 존재하면 통과', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [
                        { id: 'ch1', href: 'ch1.xhtml' },
                        { id: 'ch2', href: 'ch2.xhtml' },
                    ],
                    spineRefs: ['ch1', 'ch2'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
                zip.file('OEBPS/ch2.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const spineSection = result.sections.find(s => s.name === 'Spine 파일');
            expect(spineSection.results.some(r => r.type === 'ok' && r.text.includes('2개'))).toBe(true);
        });

        it('spine idref가 manifest에 없으면 에러', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1', 'missing_id'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const spineSection = result.sections.find(s => s.name === 'Spine 파일');
            expect(spineSection.results.some(r => r.type === 'error' && r.text.includes('missing_id'))).toBe(true);
        });

        it('spine 참조 파일이 ZIP에 없으면 에러', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                // ch1.xhtml을 ZIP에 추가하지 않음
            });

            const result = await diagnoseEpub(buf);
            const spineSection = result.sections.find(s => s.name === 'Spine 파일');
            expect(spineSection.results.some(r => r.type === 'error' && r.text.includes('ZIP에 없음'))).toBe(true);
        });
    });

    // === NCX 검사 ===

    describe('NCX', () => {
        it('toc 속성 없으면 info 메시지', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const ncxSection = result.sections.find(s => s.name === 'NCX');
            expect(ncxSection.results.some(r => r.type === 'info' && r.text.includes('toc 속성 없음'))).toBe(true);
        });

        it('유효한 NCX 참조가 모두 존재하면 통과', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [
                        { id: 'ch1', href: 'ch1.xhtml' },
                        { id: 'ncx', href: 'toc.ncx', mediaType: 'application/x-dtbncx+xml' },
                    ],
                    spineRefs: ['ch1'],
                    tocId: 'ncx',
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
                zip.file('OEBPS/toc.ncx', makeNcx([{ label: 'Chapter 1', src: 'ch1.xhtml' }]));
            });

            const result = await diagnoseEpub(buf);
            const ncxSection = result.sections.find(s => s.name === 'NCX');
            expect(ncxSection.results.some(r => r.type === 'ok' && r.text.includes('navPoint 참조 파일 모두 존재'))).toBe(true);
        });

        it('NCX navPoint가 존재하지 않는 파일을 참조하면 에러', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [
                        { id: 'ch1', href: 'ch1.xhtml' },
                        { id: 'ncx', href: 'toc.ncx', mediaType: 'application/x-dtbncx+xml' },
                    ],
                    spineRefs: ['ch1'],
                    tocId: 'ncx',
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
                zip.file('OEBPS/toc.ncx', makeNcx([
                    { label: 'Chapter 1', src: 'ch1.xhtml' },
                    { label: 'Missing', src: 'missing.xhtml' },
                ]));
            });

            const result = await diagnoseEpub(buf);
            const ncxSection = result.sections.find(s => s.name === 'NCX');
            expect(ncxSection.results.some(r => r.type === 'error' && r.text.includes('1건'))).toBe(true);
        });

        it('NCX manifest item의 href가 id 앞에 와도 정상 동작 (DOM 기반)', async () => {
            // id/href 속성 순서가 반대인 OPF
            const opfWithReversedAttrs = `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier>test</dc:identifier><dc:title>Test</dc:title><dc:language>ko</dc:language>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item href="toc.ncx" id="ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>`;

            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', opfWithReversedAttrs);
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
                zip.file('OEBPS/toc.ncx', makeNcx([{ label: 'Chapter 1', src: 'ch1.xhtml' }]));
            });

            const result = await diagnoseEpub(buf);
            const ncxSection = result.sections.find(s => s.name === 'NCX');
            // regex 기반이었다면 실패했을 케이스
            expect(ncxSection.results.some(r => r.type === 'ok' && r.text.includes('navPoint 참조 파일 모두 존재'))).toBe(true);
            expect(result.summary.errors).toBe(0);
        });
    });

    // === Guide 검사 ===

    describe('Guide', () => {
        it('guide 섹션 없으면 info', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const guideSection = result.sections.find(s => s.name === 'Guide');
            expect(guideSection.results.some(r => r.type === 'info' && r.text.includes('guide 섹션 없음'))).toBe(true);
        });
    });

    // === 콘텐츠 문서 (XHTML) 검사 ===

    describe('콘텐츠 문서', () => {
        it('유효한 XHTML이면 모두 통과', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [
                        { id: 'ch1', href: 'ch1.xhtml' },
                        { id: 'ch2', href: 'ch2.xhtml' },
                    ],
                    spineRefs: ['ch1', 'ch2'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
                zip.file('OEBPS/ch2.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const contentSection = result.sections.find(s => s.name === '콘텐츠 문서 (XHTML)');
            expect(contentSection.results.some(r => r.type === 'ok' && r.text.includes('2개'))).toBe(true);
        });

        it('잘못된 XHTML에서 파싱 에러', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', INVALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            const contentSection = result.sections.find(s => s.name === '콘텐츠 문서 (XHTML)');
            expect(contentSection.results.some(r => r.type === 'error' && r.text.includes('XHTML 파싱 실패'))).toBe(true);
            expect(result.summary.errors).toBeGreaterThan(0);
        });
    });

    // === 경로 정규화 ===

    describe('경로 정규화', () => {
        it('../ 상대 경로가 포함된 NCX 참조를 올바르게 처리', async () => {
            // OPF가 OEBPS/sub/ 하위에 있고, NCX navPoint가 ../ch1.xhtml 참조
            const containerXml = `<?xml version="1.0" encoding="UTF-8"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="OEBPS/sub/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>`;

            const zip = new JSZip();
            zip.file('mimetype', 'application/epub+zip');
            zip.file('META-INF/container.xml', containerXml);
            zip.file('OEBPS/sub/content.opf', makeOpf({
                items: [
                    { id: 'ch1', href: '../ch1.xhtml' },
                    { id: 'ncx', href: '../toc.ncx', mediaType: 'application/x-dtbncx+xml' },
                ],
                spineRefs: ['ch1'],
                tocId: 'ncx',
            }));
            zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            zip.file('OEBPS/toc.ncx', makeNcx([{ label: 'Chapter 1', src: 'ch1.xhtml' }]));

            const buf = await zip.generateAsync({ type: 'arraybuffer' });
            const result = await diagnoseEpub(buf);

            const spineSection = result.sections.find(s => s.name === 'Spine 파일');
            expect(spineSection.results.some(r => r.type === 'ok')).toBe(true);

            const ncxSection = result.sections.find(s => s.name === 'NCX');
            // ../toc.ncx → OEBPS/toc.ncx로 정규화되어야 함
            expect(ncxSection.results.some(r => r.type === 'error' && r.text.includes('ZIP에 없음'))).toBe(false);
        });
    });

    // === 종합 ===

    describe('종합', () => {
        it('완전히 유효한 EPUB에서 fatal/에러/경고 모두 0', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [
                        { id: 'ch1', href: 'ch1.xhtml' },
                        { id: 'ch2', href: 'ch2.xhtml' },
                    ],
                    spineRefs: ['ch1', 'ch2'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
                zip.file('OEBPS/ch2.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            expect(result.summary.fatal).toBe(0);
            expect(result.summary.errors).toBe(0);
            expect(result.summary.warnings).toBe(0);
            expect(result.sections.length).toBe(6);
        });

        it('결과 구조가 올바른 형식', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            expect(result).toHaveProperty('sections');
            expect(result).toHaveProperty('summary');
            expect(result.summary).toHaveProperty('fatal');
            expect(result.summary).toHaveProperty('errors');
            expect(result.summary).toHaveProperty('warnings');

            for (const section of result.sections) {
                expect(section).toHaveProperty('name');
                expect(section).toHaveProperty('results');
                for (const r of section.results) {
                    expect(r).toHaveProperty('type');
                    expect(r).toHaveProperty('text');
                    expect(['ok', 'error', 'warn', 'info']).toContain(r.type);
                    if (r.type === 'error') {
                        expect(r).toHaveProperty('severity');
                        expect(['FATAL', 'ERROR']).toContain(r.severity);
                    }
                    if (r.type === 'warn') {
                        expect(r.severity).toBe('WARNING');
                    }
                }
            }
        });

        it('ok/info 항목은 severity를 가지지 않는다', async () => {
            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', makeOpf({
                    items: [{ id: 'ch1', href: 'ch1.xhtml' }],
                    spineRefs: ['ch1'],
                }));
                zip.file('OEBPS/ch1.xhtml', VALID_XHTML);
            });

            const result = await diagnoseEpub(buf);
            for (const section of result.sections) {
                for (const r of section.results) {
                    if (r.type === 'ok' || r.type === 'info') {
                        expect(r.severity).toBeUndefined();
                    }
                }
            }
        });

        it('summary.fatal은 FATAL severity 항목 수와 일치한다', async () => {
            const result = await diagnoseEpub(new ArrayBuffer(4));
            let actualFatal = 0;
            for (const section of result.sections) {
                for (const r of section.results) {
                    if (r.severity === 'FATAL') actualFatal++;
                }
            }
            expect(result.summary.fatal).toBe(actualFatal);
            expect(actualFatal).toBeGreaterThan(0);
        });

        it('fatal과 errors는 겹치지 않게 분리 집계된다', async () => {
            // mimetype 없는 ZIP → FATAL 1건 (mimetype 파일 없음), ERROR 0건
            const zip = new JSZip();
            zip.file('META-INF/container.xml', CONTAINER_XML);
            const buf = await zip.generateAsync({ type: 'arraybuffer' });

            const result = await diagnoseEpub(buf);
            expect(result.summary.fatal).toBeGreaterThan(0);
            expect(result.summary.errors).toBe(0);
        });

        it('FATAL과 ERROR가 동시에 존재할 때 각각 정확히 집계된다', async () => {
            // manifest 비어있음(FATAL) + spine 비어있음(FATAL) + NCX 참조 오류(ERROR)
            // spine에 toc="ncx" 설정하되 manifest가 비어있으므로 NCX item을 찾지 못함
            const badOpf = `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier>test</dc:identifier><dc:title>T</dc:title><dc:language>ko</dc:language>
  </metadata>
  <manifest></manifest>
  <spine toc="ncx"></spine>
</package>`;

            const buf = await buildEpub((zip) => {
                zip.file('OEBPS/content.opf', badOpf);
            });

            const result = await diagnoseEpub(buf);
            // manifest 비어있음 + spine 비어있음 → FATAL 2건
            // toc="ncx" 참조하지만 manifest에 해당 item 없음 → ERROR 1건
            expect(result.summary.fatal).toBe(2);
            expect(result.summary.errors).toBe(1);
        });
    });
});
