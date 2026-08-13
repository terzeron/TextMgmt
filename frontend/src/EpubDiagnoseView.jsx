import {useState, useEffect, useCallback, useRef} from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Badge, Card, Spinner, Table} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faChevronDown, faChevronRight} from '@fortawesome/free-solid-svg-icons';

import {getApiUrlPrefix, jsonGetReq} from './Common';
import {diagnoseEpub} from './EpubDiagnose';
import {diagnosePdf} from './PdfDiagnose';

const SEVERITY_BADGE = {
    FATAL: 'danger',
    ERROR: 'warning',
    WARNING: 'secondary',
    USAGE: 'light',
    INFO: 'light',
};

const SEVERITY_TEXT_DARK = new Set(['ERROR', 'WARNING', 'USAGE', 'INFO']);

// epubcheck 심각도 순서 (높은 순)
const SEVERITY_ORDER = ['FATAL', 'ERROR', 'WARNING', 'USAGE', 'INFO'];

const SEVERITY_DESC = {
    FATAL: '렌더링 불가 — 파일 구조 결함',
    ERROR: '스펙 위반 — 대부분 렌더링에 영향 없음',
    WARNING: '권장사항 미준수',
    USAGE: '사용법 안내',
    INFO: '참고 정보',
};

function renderSeverityGroups(items, getLabel, getLocation) {
    const groups = {};
    for (const item of items) {
        const sev = getLabel(item);
        /* v8 ignore next -- diagnosis normalizers omit items without severity before rendering. */
        if (!sev) continue;
        if (!groups[sev]) groups[sev] = [];
        groups[sev].push(item);
    }

    return SEVERITY_ORDER
        .filter(sev => groups[sev]?.length > 0)
        .map(sev => {
            const msgs = groups[sev];
            /* v8 ignore next -- SEVERITY_ORDER contains only known badge keys. */
            const bg = SEVERITY_BADGE[sev] || 'secondary';
            const darkText = SEVERITY_TEXT_DARK.has(sev);
            return (
                <div key={sev} className="mt-1">
                    <span style={{fontSize: '0.8rem'}}>
                        <Badge bg={bg} text={darkText ? 'dark' : undefined} className="me-1">
                            {sev}
                        </Badge>
                        <span className="text-muted" style={{fontSize: '0.75rem'}}>
                            {msgs.length}건 — {SEVERITY_DESC[sev]}
                        </span>
                    </span>
                    <Table size="sm" bordered className="mt-1 mb-1" style={{tableLayout: 'fixed'}}>
                        <tbody>
                            {msgs.map((msg, idx) => (
                                <tr key={idx}>
                                    <td style={{width: '50%', fontSize: '0.75rem', wordBreak: 'break-word'}}>
                                        {msg._text || ''}
                                    </td>
                                    <td style={{width: '50%', fontSize: '0.7rem', wordBreak: 'break-word'}} className="text-muted">
                                        {getLocation(msg) || ''}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </Table>
                </div>
            );
        });
}

export default function EpubDiagnoseView({bookId, fileType, apiPrefix = ''}) {
    const [isOpen, setIsOpen] = useState(false);
    const [hasRun, setHasRun] = useState(false);
    const abortRef = useRef(null);

    // Backend validation (epubcheck)
    const [backendData, setBackendData] = useState(null);
    const [backendError, setBackendError] = useState(null);
    const [backendLoading, setBackendLoading] = useState(false);

    // Frontend diagnosis (browser DOMParser)
    const [frontendData, setFrontendData] = useState(null);
    const [frontendError, setFrontendError] = useState(null);
    const [frontendLoading, setFrontendLoading] = useState(false);

    const isEpub = fileType === 'epub';
    const isPdf = fileType === 'pdf';
    const isValidatable = isEpub || isPdf;

    const runDiagnosis = useCallback(() => {
        if (!bookId || !isValidatable) return;

        // 이전 요청 취소
        if (abortRef.current) abortRef.current.abort();
        const controller = new AbortController();
        abortRef.current = controller;

        setHasRun(true);

        // Backend 진단 호출
        setBackendLoading(true);
        setBackendData(null);
        setBackendError(null);
        jsonGetReq(
            apiPrefix + '/validate/' + bookId,
            null,
            (result) => {
                if (controller.signal.aborted) return;
                setBackendData(result);
                setBackendLoading(false);
            },
            (error) => {
                if (controller.signal.aborted) return;
                setBackendError(String(error));
                setBackendLoading(false);
            }
        );

        // Frontend: 브라우저 진단
        setFrontendLoading(true);
        setFrontendData(null);
        setFrontendError(null);
        const url = `${getApiUrlPrefix()}${apiPrefix}/download/${bookId}`;
        fetch(url, {signal: controller.signal, credentials: "include"})
            .then((res) => {
                if (!res.ok) throw new Error(`서버 응답 오류: ${res.status}`);
                return res.arrayBuffer();
            })
            .then((buf) => isEpub ? diagnoseEpub(buf) : diagnosePdf(buf))
            .then((result) => {
                if (controller.signal.aborted) return;
                setFrontendData(result);
                setFrontendLoading(false);
            })
            .catch((err) => {
                if (err.name === 'AbortError' || controller.signal.aborted) return;
                setFrontendError(err.message);
                setFrontendLoading(false);
            });
    }, [bookId, isEpub, isValidatable, apiPrefix]);

    // 카드 열릴 때 진단 실행
    useEffect(() => {
        if (isOpen && !hasRun) {
            runDiagnosis();
        }
    }, [isOpen, hasRun, runDiagnosis]);

    // bookId 변경 시 초기화 및 이전 요청 취소
    useEffect(() => {
        if (abortRef.current) abortRef.current.abort();
        setBackendData(null);
        setBackendError(null);
        setBackendLoading(false);
        setFrontendData(null);
        setFrontendError(null);
        setFrontendLoading(false);
        setHasRun(false);
        setIsOpen(false);
    }, [bookId]);

    // 언마운트 시 요청 취소
    useEffect(() => {
        return () => {
            if (abortRef.current) abortRef.current.abort();
        };
    }, []);

    if (!isValidatable) return null;

    return (
        <Card>
            <Card.Header
                onClick={() => setIsOpen(!isOpen)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={isOpen ? faChevronDown : faChevronRight} className="me-2"/>
                파일 정합성 진단
            </Card.Header>

            {isOpen && (
                <Card.Body className="p-2" style={{fontSize: '0.8rem'}}>

                    {/* Backend 진단 */}
                    <div className="mb-3">
                        <strong>Backend 진단 ({isEpub ? 'epubcheck' : 'pikepdf'})</strong>
                        {backendLoading && <Spinner animation="border" size="sm" className="ms-2"/>}

                        {backendError && (
                            <div className="text-danger mt-1">{backendError}</div>
                        )}

                        {backendData && (
                            <div className="mt-1">
                                <div className="mb-1">
                                    <Badge bg={backendData.valid ? 'success' : 'danger'}>
                                        {backendData.valid ? 'VALID' : 'INVALID'}
                                    </Badge>
                                    {backendData.file_path && (
                                        <span className="text-muted ms-2" style={{fontSize: '0.75rem'}}>
                                            {backendData.file_path}
                                        </span>
                                    )}
                                </div>

                                {backendData.publication && (
                                    <div className="text-muted mb-1" style={{fontSize: '0.75rem'}}>
                                        {[
                                            backendData.publication.title,
                                            backendData.publication.creator,
                                            backendData.publication.publisher,
                                            backendData.publication.producer,
                                            backendData.publication.page_count != null && `${backendData.publication.page_count}p`,
                                            backendData.publication.pdf_version && `PDF ${backendData.publication.pdf_version}`,
                                        ].filter(Boolean).join(' / ')}
                                    </div>
                                )}

                                {backendData.messages && backendData.messages.length > 0 &&
                                    renderSeverityGroups(
                                        backendData.messages.map(m => ({...m, _text: m.message})),
                                        (m) => m.severity || 'INFO',
                                        (m) => m.location?.path
                                            ? `${m.location.path}${m.location.line > 0 ? ':' + m.location.line : ''}`
                                            : null
                                    )}

                                {backendData.messages && backendData.messages.length === 0 && (
                                    <div className="text-success">메시지 없음</div>
                                )}
                            </div>
                        )}
                    </div>

                    {/* Frontend 진단 (브라우저) */}
                    <hr className="my-2"/>
                    <div>
                        <strong>Frontend 진단 ({isEpub ? '브라우저 DOMParser' : 'pdf.js'})</strong>
                        {frontendLoading && <Spinner animation="border" size="sm" className="ms-2"/>}

                        {frontendError && (
                            <div className="text-danger mt-1">{frontendError}</div>
                        )}

                        {frontendData && (() => {
                            const hasIssues = frontendData.summary.fatal > 0 || frontendData.summary.errors > 0;
                            const allItems = [];
                            for (const section of frontendData.sections) {
                                for (const item of section.results) {
                                    if (item.severity) {
                                        allItems.push({...item, _text: item.text, _section: section.name});
                                    }
                                }
                            }
                            return (
                                <div className="mt-1">
                                    <div className="mb-1">
                                        <Badge bg={hasIssues ? 'danger' : 'success'}>
                                            {hasIssues ? 'FAIL' : 'PASS'}
                                        </Badge>
                                    </div>
                                    {allItems.length > 0
                                        ? renderSeverityGroups(
                                            allItems,
                                            (m) => m.severity,
                                            (m) => m._section
                                        )
                                        : <div className="text-success">이상 없음</div>
                                    }
                                </div>
                            );
                        })()}
                    </div>
                </Card.Body>
            )}
        </Card>
    );
}

EpubDiagnoseView.propTypes = {
    bookId: PropTypes.number,
    fileType: PropTypes.string,
    apiPrefix: PropTypes.string,
};
