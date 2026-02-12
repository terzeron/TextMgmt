// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest';

const { mockGetDocument, mockGetMetadata, mockGetPage, mockGetTextContent, mockDestroy } = vi.hoisted(() => ({
    mockGetDocument: vi.fn(),
    mockGetMetadata: vi.fn(),
    mockGetPage: vi.fn(),
    mockGetTextContent: vi.fn(),
    mockDestroy: vi.fn(),
}));

vi.mock('pdfjs-dist', () => ({
    getDocument: mockGetDocument,
    GlobalWorkerOptions: { workerSrc: '' },
}));

import { diagnosePdf } from '../src/PdfDiagnose';

const createMockPdfDoc = (overrides = {}) => ({
    numPages: overrides.numPages ?? 3,
    getMetadata: overrides.getMetadata ?? mockGetMetadata,
    getPage: overrides.getPage ?? mockGetPage,
    destroy: mockDestroy,
});

describe('diagnosePdf', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it('정상 PDF를 진단한다', async () => {
        const pdfDoc = createMockPdfDoc();
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({
            info: { Title: '테스트 PDF', Author: '저자' },
        });
        mockGetPage.mockResolvedValue({
            view: [0, 0, 595, 842],
            getTextContent: () => Promise.resolve({
                items: [{ str: '본문 텍스트입니다' }],
            }),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        expect(result.sections).toHaveLength(4);
        expect(result.sections[0].name).toBe('PDF 파싱');
        expect(result.sections[0].results[0].type).toBe('ok');
        expect(result.sections[1].name).toBe('메타데이터');
        expect(result.sections[2].name).toBe('페이지 구조');
        expect(result.sections[3].name).toBe('텍스트 추출');
        expect(result.summary.fatal).toBe(0);
        expect(result.summary.errors).toBe(0);
        expect(mockDestroy).toHaveBeenCalled();
    });

    it('파싱 실패 시 FATAL 에러를 반환한다', async () => {
        mockGetDocument.mockReturnValue({
            promise: Promise.reject(new Error('Invalid PDF')),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        expect(result.sections).toHaveLength(1);
        expect(result.sections[0].results[0].type).toBe('error');
        expect(result.sections[0].results[0].severity).toBe('FATAL');
        expect(result.summary.fatal).toBe(1);
    });

    it('암호화된 PDF를 감지한다', async () => {
        mockGetDocument.mockReturnValue({
            promise: Promise.reject(new Error('password required')),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        expect(result.sections[0].results[0].severity).toBe('ERROR');
        expect(result.sections[0].results[0].text).toContain('암호화된 PDF');
    });

    it('메타데이터가 없으면 경고를 반환한다', async () => {
        const pdfDoc = createMockPdfDoc();
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({ info: {} });
        mockGetPage.mockResolvedValue({
            view: [0, 0, 595, 842],
            getTextContent: () => Promise.resolve({ items: [{ str: 'text' }] }),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        const metaSection = result.sections.find(s => s.name === '메타데이터');
        expect(metaSection.results.some(r => r.type === 'warn')).toBe(true);
        expect(result.summary.warnings).toBeGreaterThanOrEqual(1);
    });

    it('AcroForm 포함 시 info를 반환한다', async () => {
        const pdfDoc = createMockPdfDoc();
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({
            info: { Title: 'Form', IsAcroFormPresent: true },
        });
        mockGetPage.mockResolvedValue({
            view: [0, 0, 595, 842],
            getTextContent: () => Promise.resolve({ items: [{ str: 'text' }] }),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));
        const metaSection = result.sections.find(s => s.name === '메타데이터');
        expect(metaSection.results.some(r => r.text.includes('AcroForm'))).toBe(true);
    });

    it('페이지가 0개이면 FATAL을 반환한다', async () => {
        const pdfDoc = createMockPdfDoc({ numPages: 0 });
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({ info: { Title: 'test' } });

        const result = await diagnosePdf(new ArrayBuffer(8));

        const pageSection = result.sections.find(s => s.name === '페이지 구조');
        expect(pageSection.results.some(r => r.severity === 'FATAL')).toBe(true);
    });

    it('5페이지 이하에서 모든 페이지를 샘플링한다', async () => {
        const pdfDoc = createMockPdfDoc({ numPages: 3 });
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({ info: { Title: 'test' } });
        mockGetPage.mockResolvedValue({
            view: [0, 0, 595, 842],
            getTextContent: () => Promise.resolve({ items: [{ str: 'text' }] }),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        const pageSection = result.sections.find(s => s.name === '페이지 구조');
        expect(pageSection.results.some(r => r.text.includes('샘플 3개'))).toBe(true);
    });

    it('6페이지 이상에서 5개 샘플 페이지를 검사한다', async () => {
        const pdfDoc = createMockPdfDoc({ numPages: 20 });
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({ info: { Title: 'test' } });
        mockGetPage.mockResolvedValue({
            view: [0, 0, 595, 842],
            getTextContent: () => Promise.resolve({ items: [{ str: 'text' }] }),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        const pageSection = result.sections.find(s => s.name === '페이지 구조');
        expect(pageSection.results.some(r => r.text.includes('샘플 5개'))).toBe(true);
    });

    it('텍스트 추출 불가 시 경고를 반환한다', async () => {
        const pdfDoc = createMockPdfDoc();
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({ info: { Title: 'test' } });
        mockGetPage.mockResolvedValue({
            view: [0, 0, 595, 842],
            getTextContent: () => Promise.resolve({ items: [] }),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        const textSection = result.sections.find(s => s.name === '텍스트 추출');
        expect(textSection.results.some(r => r.type === 'warn')).toBe(true);
    });

    it('메타데이터 추출 실패 시 에러를 반환한다', async () => {
        const pdfDoc = createMockPdfDoc({
            getMetadata: () => Promise.reject(new Error('metadata error')),
        });
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetPage.mockResolvedValue({
            view: [0, 0, 595, 842],
            getTextContent: () => Promise.resolve({ items: [{ str: 'text' }] }),
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        const metaSection = result.sections.find(s => s.name === '메타데이터');
        expect(metaSection.results.some(r => r.type === 'error')).toBe(true);
    });

    it('페이지 로드 실패 시 에러를 반환한다', async () => {
        const pdfDoc = createMockPdfDoc({
            numPages: 2,
            getPage: vi.fn()
                .mockResolvedValueOnce({ view: [0, 0, 595, 842] }) // 첫 페이지 크기 확인
                .mockResolvedValueOnce({ view: [0, 0, 595, 842] }) // 샘플 1번째
                .mockRejectedValueOnce(new Error('page error')),   // 샘플 2번째 실패
        });
        mockGetDocument.mockReturnValue({ promise: Promise.resolve(pdfDoc) });
        mockGetMetadata.mockResolvedValue({ info: { Title: 'test' } });
        // 텍스트 추출용: 첫 페이지 성공
        pdfDoc.getPage.mockImplementation((pageNum) => {
            if (pageNum === 1) {
                return Promise.resolve({
                    view: [0, 0, 595, 842],
                    getTextContent: () => Promise.resolve({ items: [{ str: 'text' }] }),
                });
            }
            return Promise.reject(new Error('page error'));
        });

        const result = await diagnosePdf(new ArrayBuffer(8));

        const pageSection = result.sections.find(s => s.name === '페이지 구조');
        expect(pageSection.results.some(r => r.text.includes('페이지 로드 실패') || r.text.includes('page error'))).toBe(true);
    });
});
