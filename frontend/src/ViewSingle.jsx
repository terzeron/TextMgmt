import {useEffect, useState} from "react";
import {useParams, useSearchParams} from "react-router-dom";
import PropTypes from 'prop-types';

import './ViewSingle.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import ViewPDF from "./ViewPDF";
import ViewEPUB from "./ViewEPUB";
import ViewDOC from "./ViewDOC";
import ViewTXT from "./ViewTXT";
import ViewHTML from './ViewHTML';
import ViewRTF from './ViewRTF';
import ViewImage from "./ViewImage";
import {Button, Card} from "react-bootstrap";

export default function ViewSingle(props) {
    // URL 파라미터와 쿼리 파라미터에서 값 추출
    const { entryId, fileType: paramFileType } = useParams();
    const [searchParams] = useSearchParams();
    const paramFilePath = searchParams.get('path') || '';
    const paramApiPrefix = searchParams.get('api') || '';
    const standalone = Boolean(entryId && paramFileType);
    const [bookId, setBookId] = useState(0);
    const [filePath, setFilePath] = useState('');
    const [fileType, setFileType] = useState('');
    const [lineCount, setLineCount] = useState(0);
    const [pageCount, setPageCount] = useState(0);

    useEffect(() => {
        if (entryId && paramFileType && paramFilePath) {
            // standalone page via route
            setBookId(Number(entryId));
            setFileType(paramFileType);
            setFilePath(paramFilePath);
        } else if (props.bookId) {
            // nested component
            setBookId(props.bookId);
            setFileType(props.fileType);
            setFilePath(props.filePath);
            setLineCount(props.lineCount);
            setPageCount(props.pageCount);
        }
    }, [props.bookId, props.fileType, props.filePath, props.lineCount, props.pageCount, entryId, paramFileType, paramFilePath]);

    // standalone 모드에서 body/html 스크롤 잠금 (iOS 바운스 방지)
    useEffect(() => {
        if (!standalone) return;
        const html = document.documentElement;
        const body = document.body;
        const saved = {
            htmlOverflow: html.style.overflow,
            htmlHeight: html.style.height,
            bodyOverflow: body.style.overflow,
            bodyHeight: body.style.height,
            bodyPosition: body.style.position,
            bodyWidth: body.style.width,
        };
        html.style.overflow = 'hidden';
        html.style.height = '100%';
        body.style.overflow = 'hidden';
        body.style.height = '100%';
        body.style.position = 'fixed';
        body.style.width = '100%';
        return () => {
            html.style.overflow = saved.htmlOverflow;
            html.style.height = saved.htmlHeight;
            body.style.overflow = saved.bodyOverflow;
            body.style.height = saved.bodyHeight;
            body.style.position = saved.bodyPosition;
            body.style.width = saved.bodyWidth;
        };
    }, [standalone]);

    const preview = props.preview || false;

    const ap = standalone ? paramApiPrefix : (props.apiPrefix || '');
    const componentMap = {
        'pdf': <ViewPDF bookId={bookId} pageCount={pageCount} preview={preview} apiPrefix={ap} />,
        'epub': <ViewEPUB bookId={bookId} preview={preview} apiPrefix={ap} />,
        'doc': <ViewDOC bookId={bookId} fileType="doc" apiPrefix={ap} />,
        'docx': <ViewDOC bookId={bookId} fileType="docx" lineCount={lineCount} apiPrefix={ap} />,
        'hwp': <ViewDOC bookId={bookId} fileType="hwp" apiPrefix={ap} />,
        'txt': <ViewTXT bookId={bookId} lineCount={lineCount} apiPrefix={ap} />,
        'html': <ViewHTML bookId={bookId} apiPrefix={ap} />,
        'rtf': <ViewRTF bookId={bookId} apiPrefix={ap} />,
        'jpg': <ViewImage bookId={bookId} apiPrefix={ap} />,
        'gif': <ViewImage bookId={bookId} apiPrefix={ap} />,
        'png': <ViewImage bookId={bookId} apiPrefix={ap} />
    };
    const renderComponent = componentMap[fileType];

    return (
        <Card className={standalone ? 'standalone-viewer' : ''}>
            {!standalone && (
                <Card.Header>
                    책 보기
                    <span>
                        {props.viewUrl && (
                            <a href={props.viewUrl} target="_blank" rel="noreferrer">
                                <Button variant="outline-primary" disabled={!props.viewUrl} size="sm" className="float-end">
                                    전체 보기
                                </Button>
                            </a>
                        )}
                        {props.downloadUrl && (
                            <a href={props.downloadUrl} target="_blank" rel="noreferrer">
                                <Button variant="outline-primary" disabled={!props.downloadUrl} size="sm" className="float-end">
                                    다운로드
                                </Button>
                            </a>
                        )}
                        {props.onNextBook && (
                            <Button variant="outline-primary" size="sm" className="float-end" onClick={props.onNextBook} disabled={!props.hasNextBook}>
                                다음 책으로
                            </Button>
                        )}
                        {props.role === 'admin' && props.editUrl && (
                            <a href={props.editUrl}>
                                <Button variant="outline-secondary" size="sm" className="float-end">
                                    편집
                                </Button>
                            </a>
                        )}
                    </span>
                </Card.Header>
            )}
            <Card.Body>
                {bookId ? (
                    renderComponent
                ) : (
                    <div>책이 선택되지 않았습니다.</div>
                )}
            </Card.Body>
        </Card>
    )
}

ViewSingle.propTypes = {
    bookId: PropTypes.number,
    filePath: PropTypes.string,
    fileType: PropTypes.string,
    viewUrl: PropTypes.string,
    downloadUrl: PropTypes.string,
    lineCount: PropTypes.number,
    pageCount: PropTypes.number,
    preview: PropTypes.bool,
    apiPrefix: PropTypes.string,
    editUrl: PropTypes.string,
    onNextBook: PropTypes.func,
    hasNextBook: PropTypes.bool,
    role: PropTypes.string,
}