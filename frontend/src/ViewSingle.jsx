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

    const preview = props.preview || false;

    const componentMap = {
        'pdf': <ViewPDF bookId={bookId} pageCount={pageCount} preview={preview} />,
        'epub': <ViewEPUB bookId={bookId} filePath={filePath} preview={preview} />,
        'doc': <ViewDOC bookId={bookId} />,
        'docx': <ViewDOC bookId={bookId} lineCount={lineCount} />,
        'txt': <ViewTXT bookId={bookId} lineCount={lineCount} />,
        'html': <ViewHTML bookId={bookId} />,
        'rtf': <ViewRTF bookId={bookId} />,
        'jpg': <ViewImage bookId={bookId} />,
        'gif': <ViewImage bookId={bookId} />,
        'png': <ViewImage bookId={bookId} />
    };
    const renderComponent = componentMap[fileType];

    return (
        <Card>
            {!standalone && (
                <Card.Header>
                    책 보기
                    <span>
                        {props.viewUrl && (
                            <a href={props.viewUrl} target="_blank" rel="noreferrer">
                                <Button variant="outline-primary" disabled={!props.viewUrl} className="btn-xs float-end">
                                    전체 보기
                                </Button>
                            </a>
                        )}
                        {props.downloadUrl && (
                            <a href={props.downloadUrl} target="_blank" rel="noreferrer">
                                <Button variant="outline-primary" disabled={!props.downloadUrl} className="btn-xs float-end">
                                    다운로드
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
}